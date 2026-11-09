//! Allocation-free, 12 dB calibrated Wiener suppression, 16 kHz mono.
//! Both active and bypass paths have a fixed 256-sample pipeline delay.
use crate::calibration::Calibration;
pub use crate::calibration::CalibrationStatus;
use crate::denoiser_tables::{TWIDDLE, WINDOW};
use crate::noise_tracker::NoiseTracker;
pub use crate::noise_tracker::TrackingStatus;

pub const FFT_SIZE: usize = 256;
pub const HOP: usize = 128;
pub const BINS: usize = FFT_SIZE / 2 + 1;
pub const BLOCK: usize = 16;
pub const DELAY_SAMPLES: usize = FFT_SIZE;
pub const CALIBRATION_FRAMES: u32 = 500; // 4 seconds, after 504 ms warmup.
const FLOOR: f32 = 0.251_188_64; // 10^(-12/20), amplitude gain floor.

#[derive(Clone, Copy)]
struct Complex {
    re: f32,
    im: f32,
}
impl Complex {
    const ZERO: Self = Self { re: 0.0, im: 0.0 };
}

#[inline(always)]
fn stage<const WIDTH: usize, const INVERSE: bool>(data: &mut [Complex; FFT_SIZE]) {
    for chunk in data.as_chunks_mut::<WIDTH>().0 {
        let (left, right) = chunk.split_at_mut(WIDTH / 2);
        for (offset, (a, b)) in left.iter_mut().zip(right).enumerate() {
            let (wr, mut wi) = TWIDDLE[offset * (FFT_SIZE / WIDTH)];
            if INVERSE {
                wi = -wi;
            }
            let re = wr * b.re - wi * b.im;
            let im = wr * b.im + wi * b.re;
            *b = Complex {
                re: a.re - re,
                im: a.im - im,
            };
            *a = Complex {
                re: a.re + re,
                im: a.im + im,
            };
        }
    }
}

fn fft<const INVERSE: bool>(data: &mut [Complex; FFT_SIZE]) {
    let mut reversed = 0;
    for index in 1..FFT_SIZE {
        let mut bit = FFT_SIZE / 2;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit >>= 1;
        }
        reversed ^= bit;
        if index < reversed {
            data.swap(index, reversed);
        }
    }
    stage::<2, INVERSE>(data);
    stage::<4, INVERSE>(data);
    stage::<8, INVERSE>(data);
    stage::<16, INVERSE>(data);
    stage::<32, INVERSE>(data);
    stage::<64, INVERSE>(data);
    stage::<128, INVERSE>(data);
    stage::<256, INVERSE>(data);
    if INVERSE {
        for sample in data {
            sample.re /= FFT_SIZE as f32;
            sample.im /= FFT_SIZE as f32;
        }
    }
}

pub struct Denoiser {
    input: [f32; FFT_SIZE],
    output: [f32; HOP],
    overlap: [f32; HOP],
    work: [Complex; FFT_SIZE],
    noise: [f32; BINS],
    previous_gain: [f32; BINS],
    previous_posterior: [f32; BINS],
    target: [f32; BINS],
    position: usize,
    calibration: Calibration,
    enabled: bool,
    ready: bool,
    tracker: NoiseTracker,
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}

impl Denoiser {
    pub const fn new() -> Self {
        Self {
            input: [0.0; FFT_SIZE],
            output: [0.0; HOP],
            overlap: [0.0; HOP],
            work: [Complex::ZERO; FFT_SIZE],
            noise: [0.0; BINS],
            previous_gain: [1.0; BINS],
            previous_posterior: [0.0; BINS],
            target: [1.0; BINS],
            position: 0,
            calibration: Calibration::new(),
            enabled: true,
            ready: false,
            tracker: NoiseTracker::new(),
        }
    }

    /// For reproducible host tests using the exact offline noise power profile.
    pub fn with_noise_profile(profile: &[f32; BINS]) -> Self {
        assert!(profile.iter().all(|x| x.is_finite() && *x >= 0.0));
        let mut state = Self::new();
        for (noise, &value) in state.noise.iter_mut().zip(profile) {
            *noise = value.max(1e-20);
        }
        state.ready = true;
        state.previous_gain.fill(FLOOR);
        state.tracker.initialize(&state.noise);
        state
    }

    pub fn ready(&self) -> bool {
        self.ready
    }
    pub fn calibration_frames(&self) -> u32 {
        self.calibration.status().accepted_frames
    }

    pub fn calibration_status(&self) -> CalibrationStatus {
        self.calibration.status()
    }

    /// Inspect the calibrated pre-gain PSD in host validation tools.
    pub fn noise_profile(&self) -> Option<&[f32; BINS]> {
        self.ready.then_some(&self.noise)
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.tracker.pause();
        }
        self.enabled = enabled;
    }

    /// Opt-in experimental tracking; fixed-profile behavior remains the default.
    pub fn set_adaptive_enabled(&mut self, enabled: bool) {
        self.tracker.set_enabled(enabled);
    }

    pub fn tracking_status(&self) -> TrackingStatus {
        self.tracker.status()
    }

    /// Relearn in a quiet room. Audio continues in delayed bypass during learning.
    pub fn recalibrate(&mut self) {
        self.noise.fill(0.0);
        self.calibration = Calibration::new();
        self.ready = false;
        self.previous_gain.fill(1.0);
        self.previous_posterior.fill(0.0);
        self.tracker.initialize(&[1e-20; BINS]);
    }

    /// Reset temporal history after an input discontinuity, retaining calibration.
    pub fn reset_signal(&mut self) {
        self.tracker.pause();
        self.input.fill(0.0);
        self.output.fill(0.0);
        self.overlap.fill(0.0);
        self.position = 0;
        self.previous_gain.fill(if self.ready && self.enabled {
            FLOOR
        } else {
            1.0
        });
        self.previous_posterior.fill(0.0);
        if !self.ready {
            self.recalibrate();
        }
    }

    /// Consumes and emits exactly 16 samples, with unchanged clock and delay.
    /// Returns true when this call performs a 128-sample hop of FFT work.
    pub fn process(&mut self, input: &[i16; BLOCK], output: &mut [i16; BLOCK]) -> bool {
        let mut precise = [0.0; BLOCK];
        let hop = self.process_f32(input, &mut precise);
        for (output, value) in output.iter_mut().zip(precise) {
            let rounded = if value >= 0.0 {
                value + 0.5
            } else {
                value - 0.5
            };
            *output = rounded.clamp(-32768.0, 32767.0) as i16;
        }
        hop
    }

    /// Output is in unnormalized PCM LSB units, retaining fractional samples
    /// through the queue and resampler until final USB gain/quantization.
    pub fn process_f32(&mut self, input: &[i16; BLOCK], output: &mut [f32; BLOCK]) -> bool {
        for i in 0..BLOCK {
            output[i] = self.output[self.position + i] * 32768.0;
            self.input[HOP + self.position + i] = f32::from(input[i]) / 32768.0;
        }
        self.position += BLOCK;
        if self.position != HOP {
            return false;
        }
        self.process_hop();
        self.input.copy_within(HOP..FFT_SIZE, 0);
        self.position = 0;
        true
    }

    fn process_hop(&mut self) {
        for ((work, &sample), &window) in self.work.iter_mut().zip(&self.input).zip(&WINDOW) {
            *work = Complex {
                re: sample * window,
                im: 0.0,
            };
        }
        fft::<false>(&mut self.work);
        // Reuse the gain scratch array for pre-gain PSD during calibration/tracking.
        for (power, sample) in self.target.iter_mut().zip(&self.work) {
            *power = sample.re * sample.re + sample.im * sample.im;
        }
        let learning = !self.ready;
        if learning && self.calibration.observe(&self.target, &mut self.noise) {
            for noise in &mut self.noise {
                *noise = (*noise / CALIBRATION_FRAMES as f32).max(1e-20);
            }
            self.ready = true;
            self.tracker.initialize(&self.noise);
        }
        if !learning && self.enabled {
            if self.tracker.status().enabled {
                self.tracker.observe(&self.target, &mut self.noise);
            }
            for bin in 0..BINS {
                let sample = self.work[bin];
                let posterior =
                    ((sample.re * sample.re + sample.im * sample.im) / self.noise[bin]).min(1e12);
                let prior = 0.96
                    * self.previous_gain[bin]
                    * self.previous_gain[bin]
                    * self.previous_posterior[bin]
                    + 0.04 * (posterior - 1.0).max(0.0);
                self.target[bin] = (prior / (1.0 + prior)).clamp(FLOOR, 1.0);
                self.previous_posterior[bin] = posterior;
            }
            for bin in 0..BINS {
                let target = 0.25 * self.target[bin.saturating_sub(1)]
                    + 0.5 * self.target[bin]
                    + 0.25 * self.target[(bin + 1).min(BINS - 1)];
                let old = self.previous_gain[bin];
                let smoothing = if target > old { 0.2 } else { 0.8 };
                let gain = (smoothing * old + (1.0 - smoothing) * target).clamp(FLOOR, 1.0);
                self.previous_gain[bin] = gain;
                self.work[bin].re *= gain;
                self.work[bin].im *= gain;
                if bin > 0 && bin < FFT_SIZE / 2 {
                    self.work[FFT_SIZE - bin].re *= gain;
                    self.work[FFT_SIZE - bin].im *= gain;
                }
            }
        } else {
            self.previous_gain.fill(1.0);
            self.previous_posterior.fill(0.0);
        }
        fft::<true>(&mut self.work);
        for i in 0..HOP {
            // The two periodic Hann windows sum to one at a 128-sample hop.
            let norm = WINDOW[i] * WINDOW[i] + WINDOW[HOP + i] * WINDOW[HOP + i];
            self.output[i] = (self.work[i].re * WINDOW[i] + self.overlap[i]) / norm;
            self.overlap[i] = self.work[HOP + i].re * WINDOW[HOP + i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_roundtrip_preserves_complex_input() {
        let mut original = [Complex::ZERO; FFT_SIZE];
        for (i, x) in original.iter_mut().enumerate() {
            *x = Complex {
                re: (i as f32 * 0.71).sin(),
                im: (i as f32 * 0.13).cos(),
            };
        }
        let mut actual = original;
        fft::<false>(&mut actual);
        fft::<true>(&mut actual);
        for (a, b) in actual.iter().zip(original) {
            assert!((a.re - b.re).abs() < 1e-5 && (a.im - b.im).abs() < 1e-5);
        }
    }

    #[test]
    fn bypass_has_exact_delay_and_keeps_sample_values() {
        let mut state = Denoiser::with_noise_profile(&[1.0; BINS]);
        state.set_adaptive_enabled(true);
        state.set_enabled(false);
        let mut history = [0i16; 2048];
        for (i, x) in history[..1792].iter_mut().enumerate() {
            *x = (((i * 7919) % 60001) as i32 - 30000) as i16;
        }
        let mut actual = [0i16; 2048];
        for (input, output) in history
            .as_chunks::<BLOCK>()
            .0
            .iter()
            .zip(actual.as_chunks_mut::<BLOCK>().0)
        {
            state.process(input, output);
        }
        assert!(actual[..DELAY_SAMPLES].iter().all(|&x| x == 0));
        assert_eq!(&actual[DELAY_SAMPLES..], &history[..1792]);
        assert_eq!(state.tracking_status().updates, 0);
    }

    #[test]
    fn calibration_completes_and_signal_reset_preserves_it() {
        let mut state = Denoiser::new();
        state.set_adaptive_enabled(true);
        let mut output = [0; BLOCK];
        for _ in 0..(63 + CALIBRATION_FRAMES) * (HOP / BLOCK) as u32 {
            state.process(&[0; BLOCK], &mut output);
        }
        assert!(state.ready());
        assert_eq!(state.calibration_frames(), CALIBRATION_FRAMES);
        assert!(state.tracking_status().enabled);
        // Learning must start only after the calibrated reference exists.
        assert_eq!(state.tracking_status().updates, 0);
        for _ in 0..100 * HOP / BLOCK {
            state.process(&[0; BLOCK], &mut output);
        }
        let updates = state.tracking_status().updates;
        assert!(updates > 0);
        let noise = state.noise;
        state.reset_signal();
        assert!(state.ready());
        assert_eq!(state.noise, noise);
        assert_eq!(state.tracking_status().updates, updates);
        for _ in 0..63 * HOP / BLOCK {
            state.process(&[0; BLOCK], &mut output);
        }
        assert_eq!(state.tracking_status().updates, updates);
        state.recalibrate();
        assert!(!state.ready());
        assert_eq!(state.calibration_frames(), 0);
        assert!(state.tracking_status().enabled);
        assert_eq!(state.tracking_status().updates, 0);
    }
}
