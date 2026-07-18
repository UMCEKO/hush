//! The audio engine: real mic -> Maxine denoiser -> hum notches -> "HUSH".
//! `run` blocks (PipeWire mainloop); call it on a thread. Controlled live via
//! the shared `Controls` (intensity, notch mask) and it fills `spectrum`.

use std::f32::consts::PI;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use pipewire as pw;
use pw::{properties::properties, spa};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use spa::param::audio::{AudioFormat, AudioInfoRaw, MAX_CHANNELS};
use spa::pod::serialize::PodSerializer;
use spa::pod::{Object, Pod, Value};

use crate::Denoiser;
use hush_core::ipc::MicInfo;
use hush_core::{Controls, NotchParam, SPECTRUM_BINS};

const RATE: u32 = 48_000;
const HOP: usize = 480;
const SAMPLE: usize = std::mem::size_of::<f32>();
const SINK: &str = "HUSH";
const FFT_SIZE: usize = 4096;

/// RBJ notch biquad (transposed direct-form II).
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}
impl Biquad {
    /// RBJ peaking EQ. `gain_db < 0` cuts; very negative ≈ a full notch.
    fn peaking(f0: f32, q: f32, gain_db: f32, sr: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * f0 / sr;
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cw) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cw) / a0,
            a2: (1.0 - alpha / a) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

fn build_notches(notches: &[NotchParam], sr: f32) -> Vec<Biquad> {
    notches
        .iter()
        .filter(|n| n.enabled && n.freq > 0.0 && n.freq < sr * 0.5 && n.gain < -0.1)
        .map(|n| Biquad::peaking(n.freq, n.q.clamp(0.3, 40.0), n.gain.clamp(-60.0, 0.0), sr))
        .collect()
}

fn audio_format_param() -> Vec<u8> {
    let mut info = AudioInfoRaw::new();
    info.set_format(AudioFormat::F32LE);
    info.set_rate(RATE);
    info.set_channels(1);
    let mut pos = [0u32; MAX_CHANNELS];
    pos[0] = pw::spa::sys::SPA_AUDIO_CHANNEL_MONO;
    info.set_position(pos);
    let obj = Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(obj))
        .unwrap()
        .0
        .into_inner()
}

fn pactl(args: &[&str]) -> Result<String> {
    let out = Command::new("pactl").args(args).output()?;
    if !out.status.success() {
        bail!("pactl {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Unload any leftover HUSH virtual-mic null-sink modules (shutdown/restart).
pub fn unload_virtual_mic() {
    clean_stale_modules();
}

/// Capture-capable sources: real mics + third-party virtual sources, minus
/// sink monitors and HUSH's own output.
pub fn list_mics() -> Vec<MicInfo> {
    let Ok(out) = pactl(&["--format=json", "list", "sources"]) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&out) else {
        return Vec::new();
    };
    json.as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| {
            let name = s.get("name")?.as_str()?;
            let is_monitor = s
                .get("monitor_source")
                .and_then(|m| m.as_str())
                .is_some_and(|m| !m.is_empty());
            if name == SINK || is_monitor {
                return None;
            }
            Some(MicInfo {
                name: name.to_string(),
                desc: s
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or(name)
                    .to_string(),
            })
        })
        .collect()
}

/// Select the capture source at runtime: persist the choice, then live-move the
/// `nv-maxine-cap` stream to it (`None` = whatever the default is right now).
/// No engine restart — the move is a session-manager reroute, audio keeps flowing.
pub fn apply_mic(controls: &Controls, name: Option<&str>) {
    hush_core::save_mic_pref(name);
    controls.set_mic(name.map(str::to_string));
    let target = match name {
        Some(n) => n.to_string(),
        None => match pactl(&["get-default-source"]) {
            Ok(d) => d,
            Err(_) => return,
        },
    };
    let Ok(out) = pactl(&["--format=json", "list", "source-outputs"]) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&out) else {
        return;
    };
    for so in json.as_array().map(|a| a.as_slice()).unwrap_or_default() {
        if so.pointer("/properties/node.name").and_then(|n| n.as_str()) != Some("nv-maxine-cap") {
            continue;
        }
        if let Some(idx) = so.get("index").and_then(|i| i.as_u64()) {
            let _ = pactl(&["move-source-output", &idx.to_string(), &target]);
        }
    }
}

fn clean_stale_modules() {
    if let Ok(list) = pactl(&["list", "short", "modules"]) {
        for line in list.lines() {
            if line.contains(SINK)
                && let Some(id) = line.split('\t').next()
            {
                let _ = Command::new("pactl").args(["unload-module", id]).status();
            }
        }
    }
}

/// Run the engine on `model`. Returns `Err` only for PipeWire/audio setup
/// failures (a systemd restart may help). A denoiser *load* failure instead lands
/// in `engine_error` and the audio thread stops quietly — the daemon keeps serving
/// the control socket so the GUI can surface it, rather than crash-looping.
pub fn run(
    version: u32,
    model: PathBuf,
    controls: Arc<Controls>,
    engine_error: Arc<Mutex<Option<String>>>,
) -> Result<()> {
    clean_stale_modules();
    pw::init();

    // Capture target: the persisted preference if that source still exists,
    // else the system default.
    let real_mic = match hush_core::load_mic_pref() {
        Some(pref) if list_mics().iter().any(|m| m.name == pref) => {
            controls.set_mic(Some(pref.clone()));
            pref
        }
        _ => pactl(&["get-default-source"])?,
    };
    let module = pactl(&[
        "load-module",
        "module-null-sink",
        // Must stay "Audio/Source/Virtual": only the Virtual class pre-configures
        // the adapter's ports (input_MONO/capture_MONO) — with plain
        // "Audio/Source" the node comes up portless and the pw-link below fails.
        // Note quickshell-based shells (DankMaterialShell) hide Virtual sources
        // from their input pickers (exact media.class match); that needs a
        // quickshell-side fix, not a class change here.
        "media.class=Audio/Source/Virtual",
        &format!("sink_name={SINK}"),
        "channel_map=mono",
        "sink_properties=device.description=HUSH",
    ])?;
    {
        let id = module.clone();
        ctrlc::set_handler(move || {
            let _ = Command::new("pactl").args(["unload-module", &id]).status();
            std::process::exit(0);
        })
        .ok();
    }

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    let cap_n = 4 * HOP;
    let (prod_raw, mut cons_raw) = HeapRb::<f32>::new(cap_n).split();
    let (mut prod_clean, cons_clean) = HeapRb::<f32>::new(cap_n).split();

    let model = model.to_string_lossy().into_owned();

    // ---- worker: Maxine denoise -> notches -> clean ring; also fills the spectrum ----
    {
        let controls = controls.clone();
        std::thread::spawn(move || {
            let mut den = match Denoiser::new(&model, version) {
                Ok(d) => d,
                Err(e) => {
                    // Don't panic (that would leave a zombie engine) — report it and stop.
                    if let Ok(mut g) = engine_error.lock() {
                        *g = Some(format!("failed to load denoiser model: {e}"));
                    }
                    eprintln!("hushd: {e}");
                    return;
                }
            };
            let frame = den.frame;
            let mut inbuf = vec![0f32; frame];
            let mut outbuf = vec![0f32; frame];
            let mut last_int = u32::MAX;
            let mut last_gen = u64::MAX;
            let mut notches: Vec<Biquad> = Vec::new();

            // FFT spectrum
            let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
            let hann: Vec<f32> = (0..FFT_SIZE)
                .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / FFT_SIZE as f32).cos())
                .collect();
            let mut ring_in = vec![0f32; FFT_SIZE]; // raw mic (original)
            let mut ring_out = vec![0f32; FFT_SIZE]; // denoised + notched (adjusted)
            let mut rpos = 0usize;
            let mut buf = vec![Complex::new(0.0f32, 0.0); FFT_SIZE];
            let mut since = 0usize;
            let mut smoothed_in = vec![0f32; SPECTRUM_BINS];
            let mut smoothed_out = vec![0f32; SPECTRUM_BINS];

            loop {
                let cur = controls.intensity().to_bits();
                if cur != last_int {
                    den.set_intensity(f32::from_bits(cur)).ok();
                    last_int = cur;
                }
                let generation = controls.notch_gen();
                if generation != last_gen {
                    notches = build_notches(&controls.notches_snapshot(), RATE as f32);
                    last_gen = generation;
                }

                if cons_raw.occupied_len() >= frame {
                    cons_raw.pop_slice(&mut inbuf);
                    if den.process(&inbuf, &mut outbuf).is_ok() {
                        for s in outbuf.iter_mut() {
                            let denoised = *s;
                            let mut v = denoised;
                            for b in notches.iter_mut() {
                                v = b.process(v);
                            }
                            *s = v;
                            ring_in[rpos] = denoised; // before the bands (denoised, pre-notch)
                            ring_out[rpos] = v; // after the bands (denoised + notch)
                            rpos = (rpos + 1) % FFT_SIZE;
                        }
                        prod_clean.push_slice(&outbuf);

                        since += frame;
                        if since >= 2400 {
                            // ~50ms: refresh both spectra. Pre-band + post-band magnitudes
                            // share ONE normalizer (the pre-band peak) so the post-band trace
                            // sits visibly lower exactly where the user's notches cut.
                            since = 0;
                            let mut mag_in = [0f32; SPECTRUM_BINS];
                            let mut mag_out = [0f32; SPECTRUM_BINS];
                            for i in 0..FFT_SIZE {
                                buf[i] =
                                    Complex::new(ring_in[(rpos + i) % FFT_SIZE] * hann[i], 0.0);
                            }
                            fft.process(&mut buf);
                            let mut fmax = 1e-9f32;
                            for i in 0..SPECTRUM_BINS {
                                mag_in[i] = buf[i + 1].norm();
                                if mag_in[i] > fmax {
                                    fmax = mag_in[i];
                                }
                            }
                            for i in 0..FFT_SIZE {
                                buf[i] =
                                    Complex::new(ring_out[(rpos + i) % FFT_SIZE] * hann[i], 0.0);
                            }
                            fft.process(&mut buf);
                            for i in 0..SPECTRUM_BINS {
                                mag_out[i] = buf[i + 1].norm();
                            }
                            // sqrt expands the low end for visibility; both scaled by fmax.
                            for i in 0..SPECTRUM_BINS {
                                let vi = (mag_in[i] / fmax).clamp(0.0, 1.0).sqrt();
                                let vo = (mag_out[i] / fmax).clamp(0.0, 1.0).sqrt();
                                smoothed_in[i] = smoothed_in[i] * 0.6 + vi * 0.4;
                                smoothed_out[i] = smoothed_out[i] * 0.6 + vo * 0.4;
                            }
                            if let Ok(mut g) = controls.spectrum.lock() {
                                g.clone_from(&smoothed_out);
                            }
                            if let Ok(mut g) = controls.spectrum_in.lock() {
                                g.clone_from(&smoothed_in);
                            }
                        }
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        });
    }

    // ---- capture: real mic -> raw ring ----
    let cap = pw::stream::StreamBox::new(
        &core,
        "nv-maxine-cap",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Communication",
            *pw::keys::NODE_NAME => "nv-maxine-cap",
            "target.object" => real_mic.as_str(),
        },
    )?;
    let _capl = cap
        .add_local_listener_with_user_data(prod_raw)
        .process(|stream, prod: &mut HeapProd<f32>| {
            let Some(mut b) = stream.dequeue_buffer() else {
                return;
            };
            let datas = b.datas_mut();
            if datas.is_empty() {
                return;
            }
            let d = &mut datas[0];
            let n = d.chunk().size() as usize / SAMPLE;
            if let Some(slice) = d.data() {
                for i in 0..n {
                    let s = i * SAMPLE;
                    let v = f32::from_le_bytes(slice[s..s + SAMPLE].try_into().unwrap());
                    let _ = prod.try_push(v);
                }
            }
        })
        .register()?;

    // ---- playback: clean ring -> virtual source ----
    let play = pw::stream::StreamBox::new(
        &core,
        "nv-maxine-out",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::MEDIA_ROLE => "Communication",
            *pw::keys::NODE_NAME => "nv-maxine-out",
            *pw::keys::NODE_AUTOCONNECT => "false",
        },
    )?;
    let play_data = (cons_clean, vec![0.0f32; 8192]);
    let _playl = play
        .add_local_listener_with_user_data(play_data)
        .process(|stream, (cons, scratch): &mut (HeapCons<f32>, Vec<f32>)| {
            let Some(mut b) = stream.dequeue_buffer() else {
                return;
            };
            let req = b.requested() as usize;
            let datas = b.datas_mut();
            if datas.is_empty() {
                return;
            }
            let d = &mut datas[0];
            let nf = if let Some(slice) = d.data() {
                let cap = slice.len() / SAMPLE;
                let nf = if req > 0 { req.min(cap) } else { cap };
                if scratch.len() < nf {
                    scratch.resize(nf, 0.0);
                }
                for v in scratch[..nf].iter_mut() {
                    *v = 0.0;
                }
                let _ = cons.pop_slice(&mut scratch[..nf]);
                for (i, &val) in scratch[..nf].iter().enumerate() {
                    let s = i * SAMPLE;
                    slice[s..s + SAMPLE].copy_from_slice(&val.to_le_bytes());
                }
                nf
            } else {
                0
            };
            let chunk = d.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = SAMPLE as _;
            *chunk.size_mut() = (SAMPLE * nf) as _;
        })
        .register()?;

    let cap_fmt = audio_format_param();
    let mut cp = [Pod::from_bytes(&cap_fmt).unwrap()];
    cap.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut cp,
    )?;
    let play_fmt = audio_format_param();
    let mut pp = [Pod::from_bytes(&play_fmt).unwrap()];
    play.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
        &mut pp,
    )?;

    std::thread::spawn(|| {
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(250));
            if Command::new("pw-link")
                .args(["nv-maxine-out:output_MONO", "HUSH:input_MONO"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return;
            }
        }
    });

    let _ = Command::new("pw-metadata")
        .args(["-n", "settings", "0", "clock.force-rate", "48000"])
        .status();
    let _ = Command::new("pw-metadata")
        .args(["-n", "settings", "0", "clock.force-quantum", "0"])
        .status();

    mainloop.run();
    Ok(())
}
