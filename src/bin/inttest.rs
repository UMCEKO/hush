//! Does intensity change take effect live (after Load)? 0.0 should ~= passthrough.
use anyhow::Result;
use hound::{SampleFormat, WavReader};
use nv_maxine::Denoiser;

fn rms(b: &[f32]) -> f64 {
    (b.iter().map(|x| (x * x) as f64).sum::<f64>() / b.len() as f64).sqrt()
}

fn main() -> Result<()> {
    let home = std::env::var("HOME")?;
    let model = format!(
        "{home}/maxine-dl/sdk/Audio_Effects_SDK/features/denoiser/models/sm_89/denoiser_v2_48k_4480.trtpkg"
    );
    let mut den = Denoiser::new(&model, 2)?;
    let f = den.frame;

    let mut rd = WavReader::open("/tmp/test_noisy.wav")?;
    let spec = rd.spec();
    let ch = spec.channels as usize;
    let inter: Vec<f32> = match spec.sample_format {
        SampleFormat::Float => rd.samples::<f32>().map(|s| s.unwrap()).collect(),
        SampleFormat::Int => {
            let sc = (1i64 << (spec.bits_per_sample - 1)) as f32;
            rd.samples::<i32>().map(|s| s.unwrap() as f32 / sc).collect()
        }
    };
    let mono: Vec<f32> = if ch <= 1 { inter } else { inter.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect() };

    let mut run = |den: &mut Denoiser, inten: f32| {
        den.set_intensity(inten).unwrap();
        let (mut ai, mut ao, mut n) = (0f64, 0f64, 0u32);
        let mut inb = vec![0f32; f];
        let mut outb = vec![0f32; f];
        let mut i = 0;
        while i + f <= mono.len() {
            inb.copy_from_slice(&mono[i..i + f]);
            den.process(&inb, &mut outb).unwrap();
            ai += rms(&inb);
            ao += rms(&outb);
            n += 1;
            i += f;
        }
        (ai / n as f64, ao / n as f64)
    };

    for inten in [1.0f32, 0.0, 0.5, 1.0] {
        let (i, o) = run(&mut den, inten);
        println!("intensity={inten:.1}: in_rms={i:.5} out_rms={o:.5}  out/in={:.2}", o / i);
    }
    Ok(())
}
