//! PCM discontinuity epochs and USB playback state, shared with host tests.
use crate::audio_queue::{PacketRate, PcmRing};
use crate::resampler::Resampler;

pub const PREFILL: usize = 256;

#[derive(Clone, Copy)]
pub struct QueueSnapshot {
    pub level: usize,
    pub generation: u32,
}

/// Call through the same lock for queue mutations and epoch changes.
pub struct AudioQueue<const N: usize, T: Copy> {
    ring: PcmRing<N, T>,
    generation: u32,
}

impl<const N: usize, T: Copy> AudioQueue<N, T> {
    pub const fn new(zero: T) -> Self {
        Self {
            ring: PcmRing::with_zero(zero),
            generation: 0,
        }
    }

    /// Notify even when empty: the consumer may still hold old FIR history.
    pub fn clear(&mut self) -> usize {
        let discarded = self.ring.clear();
        self.generation = self.generation.wrapping_add(1);
        discarded
    }

    /// Preserve the newest samples, marking any overwritten input as a gap.
    pub fn push(&mut self, samples: &[T]) -> usize {
        let lost = self.ring.push(samples);
        if lost > 0 {
            self.generation = self.generation.wrapping_add(1);
        }
        lost
    }

    pub fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            level: self.ring.len(),
            generation: self.generation,
        }
    }

    /// An old packet must never consume samples from a new epoch.
    pub fn pop(&mut self, generation: u32) -> Option<T> {
        if generation != self.generation {
            return None;
        }
        self.ring.pop()
    }
}

pub struct PacketState {
    generation: Option<u32>,
    priming: bool,
    rate: PacketRate,
    resampler: Resampler,
}

impl Default for PacketState {
    fn default() -> Self {
        Self::new()
    }
}

impl PacketState {
    pub const fn new() -> Self {
        Self {
            generation: None,
            priming: true,
            rate: PacketRate::new(),
            resampler: Resampler::new(),
        }
    }

    pub fn reset(&mut self) {
        self.priming = true;
        self.rate = PacketRate::new();
        self.resampler = Resampler::new();
    }

    /// Check at packet boundaries, and again before committing a prepared packet.
    /// Equality works across u32 wrap, assuming fewer than 2^32 unseen changes.
    fn synchronize(&mut self, generation: u32) -> bool {
        if self.generation == Some(generation) {
            return false;
        }
        self.reset();
        self.generation = Some(generation);
        true
    }

    /// None means emit a zero packet without consuming queued input.
    pub fn begin_packet(&mut self, snapshot: QueueSnapshot) -> Option<u32> {
        self.synchronize(snapshot.generation);
        if self.priming {
            if snapshot.level < PREFILL {
                return None;
            }
            self.priming = false;
        }
        Some(self.rate.step_q16(snapshot.level))
    }

    /// Silence a partially or fully prepared packet if its source epoch changed.
    pub fn discard_if_interrupted(
        &mut self,
        generation: u32,
        bytes: &mut [u8; crate::PACKET_BYTES],
    ) -> bool {
        if self.synchronize(generation) {
            bytes.fill(0);
            return true;
        }
        false
    }

    pub fn next(&mut self, step: u32, input: impl FnMut() -> Option<i16>) -> Option<i16> {
        self.resampler.next(step, input)
    }

    pub fn next_f32(&mut self, step: u32, input: impl FnMut() -> Option<f32>) -> Option<f32> {
        self.resampler.next_f32(step, input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warm(state: &mut PacketState, queue: &mut AudioQueue<1024, f32>) {
        queue.push(&[1000.0; 512]);
        for _ in 0..8 {
            let snapshot = queue.snapshot();
            let step = state.begin_packet(snapshot).unwrap();
            for _ in 0..16 {
                state
                    .next_f32(step, || queue.pop(snapshot.generation))
                    .unwrap();
            }
        }
    }

    #[test]
    fn gap_discards_old_audio_waits_for_prefill_and_clears_filter_history() {
        let mut queue = AudioQueue::<1024, f32>::new(0.0);
        let mut state = PacketState::new();
        warm(&mut state, &mut queue);
        assert!(queue.clear() > 0);
        for _ in 0..15 {
            queue.push(&[0.0; 16]);
            let before = queue.snapshot();
            assert!(state.begin_packet(before).is_none());
            assert_eq!(queue.snapshot().level, before.level);
        }
        queue.push(&[0.0; 16]);
        let snapshot = queue.snapshot();
        let step = state.begin_packet(snapshot).unwrap();
        // Drift control must restart too, even after a high-water previous epoch.
        let expected = PacketRate::new().step_q16(snapshot.level);
        assert_eq!(step, expected);
        for _ in 0..128 {
            assert_eq!(
                state.next_f32(step, || queue.pop(snapshot.generation)),
                Some(0.0)
            );
        }
    }

    #[test]
    fn gap_during_packet_rejects_new_samples_and_is_seen_at_commit() {
        let mut queue = AudioQueue::<1024, f32>::new(0.0);
        let mut state = PacketState::new();
        warm(&mut state, &mut queue);
        let old = queue.snapshot();
        let step = state.begin_packet(old).unwrap();
        let mut bytes = [0; crate::PACKET_BYTES];
        for output in bytes[..16].as_chunks_mut::<2>().0 {
            let sample = state.next_f32(step, || queue.pop(old.generation)).unwrap();
            output.copy_from_slice(&(sample as i16).to_le_bytes());
        }
        assert!(bytes.iter().any(|&byte| byte != 0));
        queue.clear();
        queue.push(&[0.0; PREFILL]);
        assert!(queue.pop(old.generation).is_none());
        assert_eq!(queue.snapshot().level, PREFILL);
        assert!(state.discard_if_interrupted(queue.snapshot().generation, &mut bytes));
        assert_eq!(bytes, [0; crate::PACKET_BYTES]);
        assert!(!state.discard_if_interrupted(queue.snapshot().generation, &mut bytes));
        let snapshot = queue.snapshot();
        let step = state.begin_packet(snapshot).unwrap();
        assert_eq!(
            state.next_f32(step, || queue.pop(snapshot.generation)),
            Some(0.0)
        );
    }

    #[test]
    fn overflow_notifies_without_losing_the_newest_queue_contents() {
        let mut queue = AudioQueue::<4, i16>::new(0);
        assert_eq!(queue.push(&[1, 2, 3, 4]), 0);
        let old = queue.snapshot();
        assert_eq!(queue.push(&[5, 6]), 2);
        let fresh = queue.snapshot();
        assert_ne!(old.generation, fresh.generation);
        assert!(queue.pop(old.generation).is_none());
        for value in [3, 4, 5, 6] {
            assert_eq!(queue.pop(fresh.generation), Some(value));
        }
    }

    #[test]
    fn repeated_empty_gaps_and_generation_wrap_are_observed() {
        let mut queue = AudioQueue::<1024, i16>::new(0);
        queue.generation = u32::MAX - 1;
        let mut state = PacketState::new();
        assert!(state.synchronize(queue.snapshot().generation));
        assert_eq!(queue.clear(), 0);
        assert_eq!(queue.clear(), 0);
        assert_eq!(queue.snapshot().generation, 0);
        assert!(state.synchronize(queue.snapshot().generation));
        assert!(state.begin_packet(queue.snapshot()).is_none());
    }

    #[test]
    fn reset_and_raw_pcm_path_restart_like_a_fresh_output() {
        let mut state = PacketState::new();
        let snapshot = QueueSnapshot {
            level: PREFILL,
            generation: 0,
        };
        let step = state.begin_packet(snapshot).unwrap();
        for _ in 0..128 {
            state.next(step, || Some(1000)).unwrap();
        }
        // Covers starvation/inactive resets even without a new producer epoch.
        state.reset();
        assert!(
            state
                .begin_packet(QueueSnapshot {
                    level: 16,
                    ..snapshot
                })
                .is_none()
        );
        let step = state.begin_packet(snapshot).unwrap();
        for _ in 0..64 {
            assert_eq!(state.next(step, || Some(0)), Some(0));
        }
    }

    #[test]
    fn empty_queue_clear_still_invalidates_old_resampler_history() {
        let mut queue = AudioQueue::<1024, f32>::new(0.0);
        let mut state = PacketState::new();
        warm(&mut state, &mut queue);
        let generation = queue.snapshot().generation;
        while queue.pop(generation).is_some() {}
        assert_eq!(queue.clear(), 0);
        queue.push(&[0.0; PREFILL]);
        let snapshot = queue.snapshot();
        let step = state.begin_packet(snapshot).unwrap();
        for _ in 0..64 {
            assert_eq!(
                state.next_f32(step, || queue.pop(snapshot.generation)),
                Some(0.0)
            );
        }
    }

    #[test]
    fn resumed_nonzero_audio_matches_a_fresh_output() {
        let mut queue = AudioQueue::<1024, f32>::new(0.0);
        let mut state = PacketState::new();
        warm(&mut state, &mut queue);
        queue.clear();
        let mut fresh_queue = AudioQueue::<1024, f32>::new(0.0);
        let mut fresh_state = PacketState::new();
        let input = core::array::from_fn::<_, 512, _>(|i| (i as f32 * 0.3).sin() * 500.0);
        queue.push(&input);
        fresh_queue.push(&input);
        let mut nonzero = false;
        for _ in 0..16 {
            let a = queue.snapshot();
            let b = fresh_queue.snapshot();
            let step = state.begin_packet(a).unwrap();
            assert_eq!(step, fresh_state.begin_packet(b).unwrap());
            for _ in 0..16 {
                let sample = state.next_f32(step, || queue.pop(a.generation)).unwrap();
                assert_eq!(
                    Some(sample),
                    fresh_state.next_f32(step, || fresh_queue.pop(b.generation))
                );
                nonzero |= sample.abs() > 1.0;
            }
        }
        assert!(nonzero);
    }

    #[test]
    fn uninterrupted_packets_keep_the_original_rate_and_resampler_behavior() {
        let mut queue = AudioQueue::<1024, i16>::new(0);
        let mut state = PacketState::new();
        let mut rate = PacketRate::new();
        let mut resampler = Resampler::new();
        let input = core::array::from_fn::<_, 512, _>(|i| ((i * 137) % 4000) as i16 - 2000);
        queue.push(&input);
        let mut cursor = 0;
        for _ in 0..16 {
            let snapshot = queue.snapshot();
            let step = state.begin_packet(snapshot).unwrap();
            assert_eq!(step, rate.step_q16(snapshot.level));
            let mut bytes = [0x55; crate::PACKET_BYTES];
            for _ in 0..16 {
                let expected = resampler.next(step, || {
                    let value = input[cursor];
                    cursor += 1;
                    Some(value)
                });
                assert_eq!(
                    state.next(step, || queue.pop(snapshot.generation)),
                    expected
                );
            }
            assert!(!state.discard_if_interrupted(snapshot.generation, &mut bytes));
            assert_eq!(bytes, [0x55; crate::PACKET_BYTES]);
        }
        assert_eq!(queue.snapshot().level, input.len() - cursor);
    }
}
