//! Buffered quiet calibration. Rejected activity never enters the committed PSD.
use crate::denoiser::{BINS, CALIBRATION_FRAMES};

const WARMUP: u32 = 63;
const QUIET_HOLD: u32 = 63;
const CHUNK: u32 = 32; // 256 ms of tentative data before committing.

#[derive(Clone, Copy, Debug)]
pub struct CalibrationStatus {
    pub accepted_frames: u32,
    pub rejected_frames: u32,
    pub hold_frames: u32,
    pub blocked: bool,
}

pub(crate) struct Calibration {
    pending: [f32; BINS],
    smooth: [f32; BINS],
    pending_frames: u32,
    warmup: u32,
    observed: u32,
    min_energy: f32,
    max_energy: f32,
    status: CalibrationStatus,
}

/// Two persistent, separated spectral peaks are treated as untrusted activity.
/// This is a conservative spectral heuristic, not speech recognition.
fn tonal_activity(power: &[f32; BINS]) -> bool {
    let total = power[2..64].iter().sum::<f32>().max(1e-20);
    let mut peaks = 0;
    let mut peak_power = 0.0;
    for bin in 3..61 {
        let value = power[bin];
        let neighbors = (power[bin - 3] + power[bin - 2] + power[bin + 2] + power[bin + 3]) * 0.25;
        if value > power[bin - 1] && value >= power[bin + 1] && value > 3.0 * neighbors.max(1e-20) {
            peaks += 1;
            peak_power += value;
        }
    }
    peaks >= 2 && peak_power > 0.2 * total
}

impl Calibration {
    pub const fn new() -> Self {
        Self {
            pending: [0.0; BINS],
            smooth: [0.0; BINS],
            pending_frames: 0,
            warmup: WARMUP,
            observed: 0,
            min_energy: f32::MAX,
            max_energy: 0.0,
            status: CalibrationStatus {
                accepted_frames: 0,
                rejected_frames: 0,
                hold_frames: 0,
                blocked: false,
            },
        }
    }

    pub fn status(&self) -> CalibrationStatus {
        self.status
    }

    fn discard(&mut self) {
        self.status.rejected_frames = self
            .status
            .rejected_frames
            .saturating_add(self.pending_frames);
        self.pending.fill(0.0);
        self.pending_frames = 0;
        self.min_energy = f32::MAX;
        self.max_energy = 0.0;
        self.status.hold_frames = QUIET_HOLD;
        self.status.blocked = true;
    }

    /// Returns true only after 500 accepted frames. No wall-clock timeout forces readiness.
    pub fn observe(&mut self, power: &[f32; BINS], accumulated: &mut [f32; BINS]) -> bool {
        if self.warmup > 0 {
            self.warmup -= 1;
            return false;
        }
        self.observed = self.observed.saturating_add(1);
        let energy = power[2..64].iter().sum::<f32>();
        let old_energy = self.smooth[2..64].iter().sum::<f32>();
        for (smooth, &current) in self.smooth.iter_mut().zip(power) {
            *smooth += 0.08 * (current - *smooth);
        }
        let activity = self.observed >= 16
            && (tonal_activity(&self.smooth) || energy > 3.0 * old_energy.max(1e-18));
        if activity {
            self.status.rejected_frames = self.status.rejected_frames.saturating_add(1);
            self.discard();
            return false;
        }
        if self.status.hold_frames > 0 {
            self.status.hold_frames -= 1;
            self.status.blocked = true;
            return false;
        }
        self.status.blocked = false;
        self.pending_frames += 1;
        for (sum, &current) in self.pending.iter_mut().zip(power) {
            *sum += current;
        }
        self.min_energy = self.min_energy.min(energy);
        self.max_energy = self.max_energy.max(energy);
        // Room noise is not constant: a 256 ms chunk can span several-fold
        // energy variation even without speech. Reject a full 10 dB swing;
        // faster onsets and voiced structure have their own veto above.
        if self.max_energy > 10.0 * self.min_energy.max(1e-18) {
            self.discard();
            return false;
        }
        let chunk = CHUNK.min(CALIBRATION_FRAMES - self.status.accepted_frames);
        if self.pending_frames == chunk {
            for (sum, &current) in accumulated.iter_mut().zip(&self.pending) {
                *sum += current;
            }
            self.status.accepted_frames += chunk;
            self.pending.fill(0.0);
            self.pending_frames = 0;
            self.min_energy = f32::MAX;
            self.max_energy = 0.0;
        }
        self.status.accepted_frames == CALIBRATION_FRAMES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_frames_finish_without_biasing_the_profile() {
        let mut state = Calibration::new();
        let mut sum = [0.0; BINS];
        for frame in 0..WARMUP + CALIBRATION_FRAMES {
            assert_eq!(
                state.observe(&[2.0; BINS], &mut sum),
                frame == WARMUP + CALIBRATION_FRAMES - 1
            );
        }
        assert_eq!(sum, [1000.0; BINS]);
        assert_eq!(state.status().rejected_frames, 0);
    }

    #[test]
    fn speech_from_start_is_rejected_then_quiet_calibration_can_complete() {
        let mut state = Calibration::new();
        let mut sum = [0.0; BINS];
        let mut voice = [1.0; BINS];
        for bin in [3, 6, 9, 12, 15] {
            voice[bin] = 100.0;
        }
        for _ in 0..1000 {
            assert!(!state.observe(&voice, &mut sum));
        }
        assert_eq!(state.status().accepted_frames, 0);
        assert_eq!(sum, [0.0; BINS]);
        let mut complete = false;
        for _ in 0..1000 {
            if state.observe(&[1.0; BINS], &mut sum) {
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert_eq!(sum, [500.0; BINS]);
        assert!(state.status().rejected_frames > 0);
    }

    #[test]
    fn an_onset_discards_the_entire_uncommitted_chunk() {
        let mut state = Calibration::new();
        let mut sum = [0.0; BINS];
        for _ in 0..WARMUP + 40 {
            state.observe(&[1.0; BINS], &mut sum);
        }
        assert_eq!(state.status().accepted_frames, 32);
        state.observe(&[100.0; BINS], &mut sum);
        assert_eq!(sum, [32.0; BINS]);
        assert_eq!(state.pending_frames, 0);
        assert!(state.status().blocked);
    }

    #[test]
    fn moderate_room_noise_variation_does_not_prevent_calibration() {
        let mut state = Calibration::new();
        let mut sum = [0.0; BINS];
        for frame in 0..WARMUP + CALIBRATION_FRAMES {
            let level = [0.5, 1.0, 2.0][frame as usize % 3];
            let ready = state.observe(&[level; BINS], &mut sum);
            assert_eq!(ready, frame == WARMUP + CALIBRATION_FRAMES - 1);
        }
        assert_eq!(state.status().rejected_frames, 0);
    }
}
