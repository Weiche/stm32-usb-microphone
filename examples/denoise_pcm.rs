//! Apply the exact firmware DSP to s16le PCM, using a supplied f32le noise PSD.
use std::{env, fmt::Write, fs};
use stm32f4_usb_mic::denoiser::{BINS, BLOCK, DELAY_SAMPLES, Denoiser};

fn main() {
    let args: Vec<_> = env::args().collect();
    assert!(
        args.len() >= 4,
        "Usage: denoise_pcm input.s16le output.s16le noise.f32le [--adaptive] [--trace trace.csv]"
    );
    let mut adaptive = false;
    let mut trace_path = None;
    let mut options = args[4..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--adaptive" => adaptive = true,
            "--trace" => trace_path = Some(options.next().expect("--trace needs a path")),
            _ => panic!("Unknown option: {option}"),
        }
    }
    let bytes = fs::read(&args[1]).unwrap();
    assert_eq!(bytes.len() % 2, 0);
    let samples: Vec<_> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|x| i16::from_le_bytes(*x))
        .collect();
    let bytes = fs::read(&args[3]).unwrap();
    assert_eq!(bytes.len(), BINS * 4);
    let mut profile = [0.0; BINS];
    for (value, bytes) in profile.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *value = f32::from_le_bytes(*bytes);
    }
    let mut state = Denoiser::with_noise_profile(&profile);
    state.set_adaptive_enabled(adaptive);
    let mut trace = String::from("input_samples,speech_guard,updating,updates,power_permille\n");
    let mut result = Vec::new();
    for start in (0..samples.len() + DELAY_SAMPLES).step_by(BLOCK) {
        let mut input = [0; BLOCK];
        for (offset, value) in input.iter_mut().enumerate() {
            *value = samples.get(start + offset).copied().unwrap_or(0);
        }
        let mut output = [0; BLOCK];
        if state.process(&input, &mut output) && trace_path.is_some() {
            let status = state.tracking_status();
            writeln!(
                trace,
                "{},{},{},{},{}",
                start + BLOCK,
                u8::from(status.speech_detected),
                u8::from(status.updating),
                status.updates,
                status.power_permille
            )
            .unwrap();
        }
        result.extend(output);
    }
    let bytes: Vec<_> = result[DELAY_SAMPLES..DELAY_SAMPLES + samples.len()]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    fs::write(&args[2], bytes).unwrap();
    if let Some(path) = trace_path {
        fs::write(path, trace).unwrap();
    }
}
