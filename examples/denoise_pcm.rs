//! Apply the exact firmware DSP to s16le PCM, using a supplied f32le noise PSD.
use std::{env, fs};
use stm32f4_usb_mic::denoiser::{BINS, BLOCK, DELAY_SAMPLES, Denoiser};

fn main() {
    let args: Vec<_> = env::args().collect();
    assert_eq!(
        args.len(),
        4,
        "Usage: denoise_pcm input.s16le output.s16le noise.f32le"
    );
    let bytes = fs::read(&args[1]).unwrap();
    assert_eq!(bytes.len() % 2, 0);
    let samples: Vec<_> = bytes
        .chunks_exact(2)
        .map(|x| i16::from_le_bytes(x.try_into().unwrap()))
        .collect();
    let bytes = fs::read(&args[3]).unwrap();
    assert_eq!(bytes.len(), BINS * 4);
    let mut profile = [0.0; BINS];
    for (value, bytes) in profile.iter_mut().zip(bytes.chunks_exact(4)) {
        *value = f32::from_le_bytes(bytes.try_into().unwrap());
    }
    let mut state = Denoiser::with_noise_profile(&profile);
    let mut result = Vec::new();
    for start in (0..samples.len() + DELAY_SAMPLES).step_by(BLOCK) {
        let mut input = [0; BLOCK];
        for (offset, value) in input.iter_mut().enumerate() {
            *value = samples.get(start + offset).copied().unwrap_or(0);
        }
        let mut output = [0; BLOCK];
        state.process(&input, &mut output);
        result.extend(output);
    }
    let bytes: Vec<_> = result[DELAY_SAMPLES..DELAY_SAMPLES + samples.len()]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    fs::write(&args[2], bytes).unwrap();
}
