//! Conservative background tracking; this heuristic is not a trained speech VAD.
use crate::denoiser::BINS;

const FIRST_BIN: usize = 4; // 250 Hz, with 62.5 Hz FFT bins.
const BAND_WIDTH: usize = 4;
const BANDS: usize = 15; // 250..4000 Hz, excluding 4000 Hz itself.
const QUIET_HOPS: u32 = 63; // 504 ms after a veto or signal discontinuity.
const UPDATE_RATE: f32 = 0.0016; // ~5 s time constant, counting accepted hops only.

#[derive(Clone, Copy, Debug)]
pub struct TrackingStatus {
    pub enabled: bool,
    /// Excess energy relative to startup calibration; not a speech probability.
    pub speech_detected: bool,
    pub updating: bool,
    pub updates: u32,
    /// Estimated total noise power / initial calibrated power, times 1000.
    pub power_permille: u32,
}

pub(crate) struct NoiseTracker {
    reference: [f32; BINS],
    smoothed_bands: [f32; BANDS],
    hold: u32,
    status: TrackingStatus,
}

impl NoiseTracker {
    pub const fn new() -> Self {
        Self {
            reference: [1e-20; BINS],
            smoothed_bands: [0.0; BANDS],
            hold: QUIET_HOPS,
            status: TrackingStatus {
                enabled: false,
                speech_detected: false,
                updating: false,
                updates: 0,
                power_permille: 1000,
            },
        }
    }

    pub fn initialize(&mut self, noise: &[f32; BINS]) {
        self.reference.copy_from_slice(noise);
        self.status.updates = 0;
        self.status.power_permille = 1000;
        self.pause();
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.status.enabled != enabled {
            self.status.enabled = enabled;
            self.pause();
        }
    }

    /// Freeze while bypassed; require a fresh quiet interval before resuming.
    pub fn pause(&mut self) {
        self.hold = QUIET_HOPS;
        self.status.speech_detected = false;
        self.status.updating = false;
        for (band, smooth) in self.smoothed_bands.iter_mut().enumerate() {
            let start = FIRST_BIN + band * BAND_WIDTH;
            *smooth = self.reference[start..start + BAND_WIDTH].iter().sum();
        }
    }

    pub fn status(&self) -> TrackingStatus {
        self.status
    }

    pub fn observe(&mut self, power: &[f32; BINS], noise: &mut [f32; BINS]) {
        if !self.status.enabled {
            return;
        }
        // Always compare with the original calibration. Comparing with the
        // adapting estimate itself could gradually absorb a sustained voice.
        let mut reference_total = 0.0;
        let mut instant_total = 0.0;
        let mut smoothed_total = 0.0;
        let mut speech = false;
        for (band, smooth) in self.smoothed_bands.iter_mut().enumerate() {
            let start = FIRST_BIN + band * BAND_WIDTH;
            let reference: f32 = self.reference[start..start + BAND_WIDTH].iter().sum();
            let current: f32 = power[start..start + BAND_WIDTH].iter().sum();
            *smooth = 0.9 * *smooth + 0.1 * current;
            reference_total += reference;
            instant_total += current;
            smoothed_total += *smooth;
            speech |= *smooth > 5.0 * reference || current > 20.0 * reference;
        }
        speech |= smoothed_total > 3.0 * reference_total || instant_total > 8.0 * reference_total;
        self.status.speech_detected = speech;
        self.status.updating = false;
        if speech {
            self.hold = QUIET_HOPS;
        } else if self.hold > 0 {
            self.hold -= 1;
        } else {
            for ((noise, &current), &reference) in noise.iter_mut().zip(power).zip(&self.reference)
            {
                // A hard +/-6 dB power envelope also bounds mistakes made by
                // the heuristic. Larger environmental changes need recalibration.
                *noise = (*noise + UPDATE_RATE * (current - *noise))
                    .max((0.25 * reference).max(1e-20))
                    .min(4.0 * reference);
            }
            self.status.updates = self.status.updates.wrapping_add(1);
            self.status.updating = true;
        }
        let reference: f32 = self.reference.iter().sum();
        self.status.power_permille = (noise.iter().sum::<f32>() / reference * 1000.0) as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> (NoiseTracker, [f32; BINS]) {
        let mut tracker = NoiseTracker::new();
        let noise = [1.0; BINS];
        tracker.initialize(&noise);
        tracker.set_enabled(true);
        (tracker, noise)
    }

    #[test]
    fn moderate_background_changes_track_in_both_directions() {
        let (mut tracker, mut noise) = tracker();
        for _ in 0..2500 {
            tracker.observe(&[2.0; BINS], &mut noise);
        }
        assert!(noise.iter().all(|&x| (1.95..2.0).contains(&x)));
        assert!(!tracker.status().speech_detected);
        for _ in 0..3500 {
            tracker.observe(&[0.5; BINS], &mut noise);
        }
        assert!(noise.iter().all(|&x| (0.5..0.51).contains(&x)));
    }

    #[test]
    fn speech_onsets_and_sustained_harmonics_freeze_learning() {
        let (mut tracker, mut noise) = tracker();
        for _ in 0..100 {
            tracker.observe(&[1.0; BINS], &mut noise);
        }
        let before = noise;
        let updates = tracker.status().updates;
        let mut voice = [1.0; BINS];
        for bin in (4..60).step_by(4) {
            voice[bin] = 100.0;
        }
        for _ in 0..2500 {
            tracker.observe(&voice, &mut noise);
            assert!(tracker.status().speech_detected);
        }
        assert_eq!(noise, before);
        assert_eq!(tracker.status().updates, updates);
        // A quiet frame immediately after a word must not be learned either.
        for _ in 0..QUIET_HOPS {
            tracker.observe(&[1.0; BINS], &mut noise);
            assert!(!tracker.status().updating);
        }
        for _ in 0..200 {
            tracker.observe(&[1.0; BINS], &mut noise);
        }
        assert!(tracker.status().updates > updates);
    }

    #[test]
    fn anchor_prevents_runaway_and_large_changes_require_recalibration() {
        let (mut tracker, mut noise) = tracker();
        for _ in 0..6000 {
            tracker.observe(&[0.0; BINS], &mut noise);
        }
        assert!(noise.iter().all(|&x| x >= 0.25));
        let before = noise;
        for _ in 0..1000 {
            tracker.observe(&[20.0; BINS], &mut noise);
        }
        assert_eq!(noise, before);
        assert!(tracker.status().speech_detected);
        // Even an unguarded high-frequency source is bounded by the anchor.
        let mut outside_voice_band = [1.0; BINS];
        outside_voice_band[100] = 100.0;
        for _ in 0..6000 {
            tracker.observe(&outside_voice_band, &mut noise);
        }
        assert_eq!(noise[100], 4.0);
    }

    #[test]
    fn bypass_and_signal_gap_restart_the_quiet_guard() {
        let (mut tracker, mut noise) = tracker();
        for _ in 0..100 {
            tracker.observe(&[2.0; BINS], &mut noise);
        }
        assert!(tracker.status().updates > 0);
        let before = noise;
        let updates = tracker.status().updates;
        tracker.set_enabled(false);
        for _ in 0..100 {
            tracker.observe(&[2.0; BINS], &mut noise);
        }
        assert_eq!(noise, before);
        tracker.set_enabled(true);
        for _ in 0..QUIET_HOPS {
            tracker.observe(&[2.0; BINS], &mut noise);
        }
        assert_eq!(noise, before);
        tracker.pause();
        assert!(!tracker.status().updating);
        assert_eq!(tracker.status().updates, updates);
        tracker.initialize(&[2.0; BINS]);
        assert!(tracker.status().enabled);
        assert_eq!(tracker.status().updates, 0);
    }
}
