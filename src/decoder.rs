//! Allocation-free PDM64 -> PCM16k. I2S halfwords are consumed bit15 first.
//! CIC integrators/comb use explicit modulo-2^32 arithmetic; wrap is intentional.
use crate::fir_coeffs::FIR;

pub const DECIMATION: usize = 64;
const CIC_GAIN: f32 = 1_048_576.0; // 16^5; reject folded PDM quantization noise.
const HP_POLE: f32 = 0.992_176_8; // exp(-2*pi*20/16000), 20 Hz DC blocker

pub struct Decoder {
    integrators: [i32; 5],
    comb_delays: [i32; 5],
    // Mirrored ring makes the FIR window contiguous: no modulo/bounds branch per tap.
    history: [f32; FIR.len() * 2],
    cursor: usize,
    phase: u8,
    previous_input: f32,
    previous_output: f32,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub const fn new() -> Self {
        Self {
            integrators: [0; 5],
            comb_delays: [0; 5],
            history: [0.0; FIR.len() * 2],
            cursor: 0,
            phase: 0,
            previous_input: 0.0,
            previous_output: 0.0,
        }
    }

    /// Reset after a known acquisition gap; never carry stale filter state across a gap.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// One u16 is 16 consecutive PDM clocks. Exactly one PCM sample per four words.
    /// LSB-justified I2S still transmits MSB first; do not reverse bytes implicitly.
    pub fn process_word(&mut self, word: u16) -> Option<i16> {
        for bit in (0..16).rev() {
            let value = if word & (1 << bit) == 0 { -1 } else { 1 };
            self.integrators[0] = self.integrators[0].wrapping_add(value);
            self.integrators[1] = self.integrators[1].wrapping_add(self.integrators[0]);
            self.integrators[2] = self.integrators[2].wrapping_add(self.integrators[1]);
            self.integrators[3] = self.integrators[3].wrapping_add(self.integrators[2]);
            self.integrators[4] = self.integrators[4].wrapping_add(self.integrators[3]);
        }
        let mut value = self.integrators[4];
        for delay in &mut self.comb_delays {
            let next = value.wrapping_sub(*delay);
            *delay = value;
            value = next;
        }
        self.history[self.cursor] = value as f32 / CIC_GAIN;
        self.history[self.cursor + FIR.len()] = value as f32 / CIC_GAIN;
        self.cursor += 1;
        if self.cursor == FIR.len() {
            self.cursor = 0;
        }
        self.phase += 1;
        if self.phase != 4 {
            return None;
        }
        self.phase = 0;
        let mut filtered = 0.0;
        // FIR coefficients are symmetric, so oldest-to-newest accumulation is equivalent.
        for (&coefficient, &sample) in FIR
            .iter()
            .zip(&self.history[self.cursor..self.cursor + FIR.len()])
        {
            filtered += coefficient * sample;
        }
        let highpass = filtered - self.previous_input + HP_POLE * self.previous_output;
        self.previous_input = filtered;
        self.previous_output = highpass;
        Some((highpass * 32768.0).clamp(-32768.0, 32767.0) as i16)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{f64::consts::PI, vec::Vec};

    // Independent first-order sigma-delta test source, with continuous-time phase.
    fn modulated_tone(frequency: f64, amplitude: f64, dc: f64, seconds: f64) -> Vec<i16> {
        let mut decoder = Decoder::new();
        let mut error = 0.0;
        let mut rng = 0x1234_5678_u32;
        let mut word = 0u16;
        let mut out = Vec::new();
        for n in 0..(1_024_000.0 * seconds) as usize {
            let desired = dc + amplitude * (2.0 * PI * frequency * n as f64 / 1_024_000.0).sin();
            error += desired;
            // Fixed-seed quantizer dither avoids deterministic idle tones in the test
            // source itself (e.g. a 6 kHz component for a 10 kHz input).
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let dither = (f64::from(rng) / f64::from(u32::MAX) - 0.5) * 0.5;
            let bit = error + dither >= 0.0;
            error -= if bit { 1.0 } else { -1.0 };
            word = (word << 1) | u16::from(bit);
            if n % 16 == 15
                && let Some(sample) = decoder.process_word(word)
            {
                out.push(sample);
            }
        }
        out
    }

    fn sinusoid_metrics(samples: &[i16], frequency: f64) -> (f64, f64, f64) {
        let n = samples.len() as f64;
        let dc = samples.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
        let (mut re, mut im, mut energy) = (0.0, 0.0, 0.0);
        for (i, &x) in samples.iter().enumerate() {
            let x = f64::from(x) - dc;
            let phase = 2.0 * PI * frequency * i as f64 / 16000.0;
            re += x * phase.cos();
            im += x * phase.sin();
            energy += x * x;
        }
        let amplitude = 2.0 * re.hypot(im) / n;
        let signal = amplitude * amplitude / 2.0;
        let noise = (energy / n - signal).max(1e-12);
        (amplitude, dc, 10.0 * (signal / noise).log10())
    }

    #[test]
    fn decimation_is_exact_and_independent_of_call_grouping() {
        let mut decoder = Decoder::new();
        for n in 0..100_000 {
            assert_eq!(decoder.process_word(0xaaaa).is_some(), n % 4 == 3);
        }
    }

    #[test]
    fn zero_density_offset_and_integrator_wrap_do_not_create_dc() {
        let mut decoder = Decoder::new();
        for i in 0..200_000 {
            if let Some(x) = decoder.process_word(0xaaaa)
                && i > 10000
            {
                assert!(x.abs() <= 1);
            }
        }
        // Ten ones in each 16-bit word represents a deterministic +0.25 DC input.
        let mut dc_decoder = Decoder::new();
        for i in 0..32_000 {
            if let Some(sample) = dc_decoder.process_word(0xe739)
                && i > 25_600
            {
                assert!(sample.abs() <= 2);
            }
        }
    }

    #[test]
    fn tone_1khz_amplitude_dc_and_noise() {
        let pcm = modulated_tone(1000.0, 0.25, 0.03, 0.5);
        assert_eq!(pcm.len(), 8000);
        let (amplitude, dc, snr) = sinusoid_metrics(&pcm[1600..], 1000.0);
        std::println!(
            "synthetic 1kHz: amplitude_lsb={amplitude:.3} dc_lsb={dc:.3} SNR_db={snr:.3}"
        );
        assert!(
            (amplitude / 8192.0 - 1.0).abs() < 0.02,
            "amplitude={amplitude}"
        );
        assert!(dc.abs() < 3.0, "DC={dc}");
        assert!(snr > 40.0, "SNR={snr}");
    }

    #[test]
    fn upper_voice_band_survives_and_aliasing_tone_is_rejected() {
        let pass = modulated_tone(6000.0, 0.25, 0.0, 0.5);
        let (amplitude, _, _) = sinusoid_metrics(&pass[1600..], 6000.0);
        assert!(
            amplitude / 8192.0 > 0.90 && amplitude / 8192.0 < 1.01,
            "6kHz amplitude={amplitude}"
        );
        let stop = modulated_tone(10_000.0, 0.25, 0.0, 0.5);
        let (alias, _, _) = sinusoid_metrics(&stop[1600..], 6000.0);
        std::println!(
            "synthetic 6kHz: amplitude_lsb={amplitude:.3}; 10kHz input alias_at_6kHz_lsb={alias:.3}"
        );
        assert!(alias < 8.2, "10kHz alias={alias}"); // >60 dB suppression
    }

    #[test]
    fn fir_has_unity_gain_and_70db_stopband() {
        let gain: f64 = FIR.iter().map(|&x| f64::from(x)).sum();
        assert!((gain - 1.0).abs() < 1e-6);
        for frequency in (8000..=32000).step_by(100) {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, &x) in FIR.iter().enumerate() {
                let phase = 2.0 * PI * frequency as f64 * i as f64 / 64000.0;
                re += f64::from(x) * phase.cos();
                im += f64::from(x) * phase.sin();
            }
            assert!(re.hypot(im) < 0.0003163, "stopband frequency={frequency}");
        }
    }

    #[test]
    fn reset_after_gap_matches_a_fresh_decoder() {
        let mut decoder = Decoder::new();
        for _ in 0..1000 {
            decoder.process_word(0xffff);
        }
        decoder.reset();
        let mut fresh = Decoder::new();
        for i in 0..4096 {
            let word = (i as u16).wrapping_mul(1171);
            assert_eq!(decoder.process_word(word), fresh.process_word(word));
        }
    }
}
