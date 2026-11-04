//! Decode a u16-LE PDM capture with the same no_std library used by the MCU.
use std::{env, fs, io, path::Path};
use stm32f4_usb_mic::decoder::Decoder;

fn main() -> io::Result<()> {
    let arguments: Vec<_> = env::args().collect();
    if arguments.len() != 3 {
        return Err(io::Error::other("usage: decode_pdm INPUT.u16le OUTPUT.wav"));
    }
    let input = fs::read(&arguments[1])?;
    if input.len() % 2 != 0 {
        return Err(io::Error::other(
            "PDM file must contain complete little-endian u16 words",
        ));
    }
    let mut decoder = Decoder::new();
    let mut pcm = Vec::new();
    for bytes in input.as_chunks::<2>().0 {
        if let Some(sample) = decoder.process_word(u16::from_le_bytes(*bytes)) {
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
    }
    let size = pcm.len() as u32;
    let mut wave = Vec::new();
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&(36 + size).to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16u32.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&16000u32.to_le_bytes());
    wave.extend_from_slice(&32000u32.to_le_bytes());
    wave.extend_from_slice(&2u16.to_le_bytes());
    wave.extend_from_slice(&16u16.to_le_bytes());
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&size.to_le_bytes());
    wave.extend_from_slice(&pcm);
    if let Some(parent) = Path::new(&arguments[2]).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&arguments[2], wave)?;
    println!(
        "Decoded {} PDM bits to {} PCM samples at 16000 Hz",
        input.len() * 8,
        size / 2
    );
    Ok(())
}
