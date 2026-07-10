//! Offline proof: Rust calling NVIDIA's Maxine denoiser on a WAV file.
//!   cargo run --release --bin denoise -- in.wav out.wav [version=2]

use anyhow::Result;
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use nv_maxine::Denoiser;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: denoise <in.wav> <out.wav> [version]");
        std::process::exit(1);
    }
    let version: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);
    let home = std::env::var("HOME")?;
    let model = std::env::var("NVAFX_MODEL").unwrap_or_else(|_| {
        let f = if version == 2 { "denoiser_v2_48k_4480" } else { "denoiser_48k_6656" };
        format!("{home}/maxine-dl/sdk/Audio_Effects_SDK/features/denoiser/models/sm_89/{f}.trtpkg")
    });

    let mut den = Denoiser::new(&model, version)?;
    println!("Maxine denoiser loaded from Rust (v{version}), frame = {}", den.frame);

    // decode -> mono f32 [-1,1]
    let mut rd = WavReader::open(&args[1])?;
    let spec = rd.spec();
    let ch = spec.channels as usize;
    let inter: Vec<f32> = match spec.sample_format {
        SampleFormat::Float => rd.samples::<f32>().map(|s| s.unwrap()).collect(),
        SampleFormat::Int => {
            let sc = (1i64 << (spec.bits_per_sample - 1)) as f32;
            rd.samples::<i32>().map(|s| s.unwrap() as f32 / sc).collect()
        }
    };
    let mono: Vec<f32> = if ch <= 1 {
        inter
    } else {
        inter.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect()
    };

    let f = den.frame;
    let mut inbuf = vec![0f32; f];
    let mut outbuf = vec![0f32; f];
    let mut out = Vec::with_capacity(mono.len() + f);
    let mut i = 0;
    while i < mono.len() {
        for k in 0..f {
            inbuf[k] = mono.get(i + k).copied().unwrap_or(0.0);
        }
        den.process(&inbuf, &mut outbuf)?;
        out.extend_from_slice(&outbuf);
        i += f;
    }
    out.truncate(mono.len());

    let os = WavSpec { channels: 1, sample_rate: 48_000, bits_per_sample: 32, sample_format: SampleFormat::Float };
    let mut wr = WavWriter::create(&args[2], os)?;
    for s in &out {
        wr.write_sample(*s)?;
    }
    wr.finalize()?;

    let irms = (mono.iter().map(|v| v * v).sum::<f32>() / mono.len() as f32).sqrt();
    let orms = (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt();
    println!("denoised {} -> {}  | in rms={irms:.5}  out rms={orms:.5}", args[1], args[2]);
    Ok(())
}
