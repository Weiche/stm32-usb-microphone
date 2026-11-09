#![no_std]

pub mod audio_output;
pub mod audio_queue;
mod calibration;
pub mod decoder;
pub mod denoiser;
mod denoiser_tables;
mod fir_coeffs;
pub mod level_meter;
mod noise_tracker;
pub mod resampler;
mod resampler_coeffs;

pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const SAMPLES_PER_PACKET: usize = 16;
pub const PACKET_BYTES: usize = SAMPLES_PER_PACKET * 2;

// round(8192 * sin(2*pi*n/16)): 1 kHz at 16 kHz; peak -12.04 dBFS.
// One packet is exactly one complete period, so repeating has no phase jump.
pub const SINE_SAMPLES: [i16; SAMPLES_PER_PACKET] = [
    0, 3135, 5793, 7568, 8192, 7568, 5793, 3135, 0, -3135, -5793, -7568, -8192, -7568, -5793, -3135,
];

pub const fn encode_packet(samples: &[i16; SAMPLES_PER_PACKET]) -> [u8; PACKET_BYTES] {
    let mut packet = [0; PACKET_BYTES];
    let mut i = 0;
    while i < SAMPLES_PER_PACKET {
        let bytes = samples[i].to_le_bytes();
        packet[2 * i] = bytes[0];
        packet[2 * i + 1] = bytes[1];
        i += 1;
    }
    packet
}

pub static SINE_PACKET: [u8; PACKET_BYTES] = encode_packet(&SINE_SAMPLES);
pub static SILENCE_PACKET: [u8; PACKET_BYTES] = [0; PACKET_BYTES];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_frame_has_exactly_one_ms_of_mono_pcm_little_endian() {
        assert_eq!(SAMPLES_PER_PACKET as u32 * 1000, SAMPLE_RATE_HZ);
        assert_eq!(SINE_PACKET.len(), 32);
        for (bytes, sample) in SINE_PACKET.as_chunks::<2>().0.iter().zip(SINE_SAMPLES) {
            assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), sample);
        }
    }

    #[test]
    fn tone_has_zero_dc_correct_peak_and_continuous_wrap() {
        assert_eq!(SINE_SAMPLES.iter().map(|&x| i32::from(x)).sum::<i32>(), 0);
        assert_eq!(SINE_SAMPLES.iter().copied().max(), Some(8192));
        assert_eq!(SINE_SAMPLES.iter().copied().min(), Some(-8192));
        for i in 0..16 {
            let step = (i32::from(SINE_SAMPLES[(i + 1) % 16]) - i32::from(SINE_SAMPLES[i])).abs();
            assert!(step <= 3135);
        }
    }

    #[test]
    fn tone_energy_is_in_one_khz_dft_bin() {
        // DFT fundamental using the quantized sine/cosine basis, no host math crate.
        let energy: i64 = SINE_SAMPLES.iter().map(|&x| i64::from(x).pow(2)).sum();
        let sin: i64 = SINE_SAMPLES.iter().map(|&x| i64::from(x).pow(2)).sum();
        let cos: i64 = (0..16)
            .map(|i| i64::from(SINE_SAMPLES[i]) * i64::from(SINE_SAMPLES[(i + 4) % 16]))
            .sum();
        assert_eq!(cos, 0);
        let fundamental_energy = (sin * sin + cos * cos) / (8 * 8192_i64.pow(2));
        assert!((energy - fundamental_energy).abs() < energy / 10_000);
    }
}
