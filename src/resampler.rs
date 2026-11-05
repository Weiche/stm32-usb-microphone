//! Continuous fractional-delay conversion between independent capture and USB clocks.
use crate::resampler_coeffs::PHASES;

pub struct Resampler {
    history: [f32; 96],
    cursor: usize,
    phase_q16: u32,
}
impl Default for Resampler {
    fn default() -> Self {
        Self::new()
    }
}
impl Resampler {
    pub const fn new() -> Self {
        Self {
            history: [0.0; 96],
            cursor: 0,
            phase_q16: 65536,
        }
    }
    /// Returns None on missing input. Caller must re-prime and reset after starvation.
    pub fn next(&mut self, step_q16: u32, mut input: impl FnMut() -> Option<i16>) -> Option<i16> {
        self.next_f32(step_q16, || input().map(f32::from))
            .map(|sample| sample.clamp(-32768.0, 32767.0) as i16)
    }

    /// PCM in LSB units, retaining fractional precision until USB digital gain.
    pub fn next_f32(
        &mut self,
        step_q16: u32,
        mut input: impl FnMut() -> Option<f32>,
    ) -> Option<f32> {
        assert!((65208..=65864).contains(&step_q16));
        for _ in 0..self.phase_q16 >> 16 {
            let sample = input()?;
            self.history[self.cursor] = sample;
            self.history[self.cursor + 48] = sample;
            self.cursor += 1;
            if self.cursor == 48 {
                self.cursor = 0;
            }
        }
        self.phase_q16 &= 65535;
        let phase = (self.phase_q16 >> 11) as usize;
        let blend = (self.phase_q16 & 2047) as f32 / 2048.0;
        let window = &self.history[self.cursor..self.cursor + 48];
        let mut a = 0.0;
        let mut b = 0.0;
        for ((sample, c0), c1) in window.iter().zip(&PHASES[phase]).zip(&PHASES[phase + 1]) {
            a += sample * c0;
            b += sample * c1;
        }
        self.phase_q16 += step_q16;
        Some(a + (b - a) * blend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    #[test]
    fn fractional_pcm_is_not_quantized_before_gain() {
        let mut state = Resampler::new();
        for i in 0..100 {
            let sample = state.next_f32(65536, || Some(0.24)).unwrap();
            if i > 64 {
                assert!((sample - 0.24).abs() < 1e-6);
            }
        }
    }
    #[test]
    fn every_fraction_preserves_dc_and_voice_band() {
        for taps in PHASES {
            let dc: f64 = taps.iter().map(|x| f64::from(*x)).sum();
            assert!((dc - 1.0).abs() < 1e-6);
            for frequency in [1000.0, 6000.0] {
                let mut re = 0.0;
                let mut im = 0.0;
                for (i, tap) in taps.iter().enumerate() {
                    let angle = 2.0 * core::f64::consts::PI * frequency * i as f64 / 16000.0;
                    re += f64::from(*tap) * angle.cos();
                    im += f64::from(*tap) * angle.sin();
                }
                assert!((re.hypot(im) - 1.0).abs() < 0.015);
            }
        }
    }
    #[test]
    fn fractional_clock_conversion_preserves_tone_and_handles_missing_input() {
        for step in [65208, 65536, 65864] {
            for frequency in [1000.0, 6000.0] {
                let mut resampler = Resampler::new();
                let mut index = 0_u32;
                let mut energy = 0.0;
                let mut re = 0.0;
                let mut im = 0.0;
                for output in 0..32000 {
                    let sample = resampler
                        .next(step, || {
                            let value = (8192.0
                                * (2.0 * core::f64::consts::PI * frequency * f64::from(index)
                                    / 16000.0)
                                    .sin()) as i16;
                            index += 1;
                            Some(value)
                        })
                        .unwrap();
                    if output >= 1000 {
                        energy += f64::from(sample).powi(2);
                        let angle = 2.0
                            * core::f64::consts::PI
                            * frequency
                            * f64::from(output)
                            * f64::from(step)
                            / (16000.0 * 65536.0);
                        re += f64::from(sample) * angle.cos();
                        im += f64::from(sample) * angle.sin();
                    }
                }
                let rms = (energy / 31000.0).sqrt();
                assert_eq!(u64::from(index), 1 + 31999 * u64::from(step) / 65536);
                let amplitude = 2.0 * re.hypot(im) / 31000.0;
                assert!(
                    (amplitude - 8192.0).abs() < 120.0,
                    "wrong output frequency or amplitude: step={step} f={frequency} amp={amplitude}"
                );
                assert!(
                    (rms - 8192.0 / 2.0_f64.sqrt()).abs() < 90.0,
                    "step={step} f={frequency} rms={rms}"
                );
                assert!(resampler.next(step, || None).is_none() || step < 65536);
            }
        }
    }
}
