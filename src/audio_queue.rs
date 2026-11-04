//! Bounded PCM queue, capture/USB clock control, and saturating gain.
pub struct PcmRing<const N: usize> {
    data: [i16; N],
    read: usize,
    len: usize,
}
impl<const N: usize> Default for PcmRing<N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<const N: usize> PcmRing<N> {
    pub const fn new() -> Self {
        assert!(N > 0);
        Self {
            data: [0; N],
            read: 0,
            len: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn clear(&mut self) -> usize {
        let count = self.len;
        self.read = 0;
        self.len = 0;
        count
    }
    /// Preserve the newest data if full; return the exact number of old samples lost.
    pub fn push(&mut self, samples: &[i16]) -> usize {
        let mut discarded = 0;
        for &sample in samples {
            if self.len == N {
                self.pop();
                discarded += 1;
            }
            let mut write = self.read + self.len;
            if write >= N {
                write -= N;
            }
            self.data[write] = sample;
            self.len += 1;
        }
        discarded
    }
    pub fn pop(&mut self) -> Option<i16> {
        if self.len == 0 {
            return None;
        }
        let sample = self.data[self.read];
        self.read += 1;
        if self.read == N {
            self.read = 0;
        }
        self.len -= 1;
        Some(sample)
    }
}

/// Smooth DMA bursts, then correct drift with continuous fractional resampling.
/// USB packets always contain 16 samples; input consumption varies by +/-0.5%.
pub struct PacketRate {
    smoothed_q8: i32,
}
impl Default for PacketRate {
    fn default() -> Self {
        Self::new()
    }
}
impl PacketRate {
    pub const TARGET: usize = 192;
    pub const fn new() -> Self {
        Self {
            smoothed_q8: (Self::TARGET * 256) as i32,
        }
    }
    pub fn step_q16(&mut self, level: usize) -> u32 {
        self.smoothed_q8 += (level as i32 * 256 - self.smoothed_q8) / 32;
        let correction = ((self.smoothed_q8 - (Self::TARGET * 256) as i32) / 64).clamp(-328, 328);
        (65536 + correction) as u32
    }
}

/// Q8 gain, bounded to 0..16x. Clipping is reported explicitly.
pub fn gain(sample: i16, q8: u32) -> (i16, bool) {
    let scaled = i32::from(sample) * q8.min(4096) as i32 / 256;
    (
        scaled.clamp(-32768, 32767) as i16,
        !(-32768..=32767).contains(&scaled),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_keeps_fresh_data_and_reports_exact_loss() {
        let mut ring = PcmRing::<4>::new();
        assert_eq!(ring.push(&[1, 2, 3]), 0);
        assert_eq!(ring.pop(), Some(1));
        assert_eq!(ring.push(&[4, 5, 6]), 1);
        for x in [3, 4, 5, 6] {
            assert_eq!(ring.pop(), Some(x));
        }
        assert!(ring.is_empty());
        assert_eq!(ring.pop(), None);
    }
    #[test]
    fn reconnect_clear_removes_all_old_audio() {
        let mut ring = PcmRing::<16>::new();
        ring.push(&[100, 200, 300]);
        assert_eq!(ring.clear(), 3);
        assert_eq!(ring.pop(), None);
        ring.push(&[-1, -2]);
        assert_eq!(ring.pop(), Some(-1));
    }
    #[test]
    fn gain_is_bounded_and_reports_clipping() {
        assert_eq!(gain(1234, 256), (1234, false));
        assert_eq!(gain(-1234, 1024), (-4936, false));
        assert_eq!(gain(30000, 4096), (32767, true));
        assert_eq!(gain(-30000, 4096), (-32768, true));
        assert_eq!(gain(-32768, 256), (-32768, false));
        assert_eq!(gain(1234, 0), (0, false));
    }
    #[test]
    fn rate_control_bounds_both_clock_directions_under_dma_bursts() {
        for ppm in [-5000_i64, -1000, 0, 1000, 5000] {
            let mut ring = PcmRing::<1024>::new();
            ring.push(&[0; 256]);
            let mut controller = PacketRate::new();
            let mut phase = 0_i64;
            let mut adjusted = 0;
            let mut source_phase = 65536_u32;
            for frame in 0..120_000 {
                phase += 16 * (1_000_000 + ppm);
                if frame % 4 == 3 {
                    let count = (phase / 1_000_000) as usize;
                    phase %= 1_000_000;
                    assert_eq!(ring.push(&[0; 68][..count]), 0, "overflow ppm={ppm}");
                }
                let step = controller.step_q16(ring.len());
                if step != 65536 {
                    adjusted += 1;
                }
                // Exactly 16 USB samples. Fixed-point model of consumed source samples.
                for _ in 0..16 {
                    for _ in 0..source_phase >> 16 {
                        assert!(ring.pop().is_some(), "underrun ppm={ppm}");
                    }
                    source_phase = (source_phase & 65535) + step;
                }
                if frame > 1000 {
                    assert!(
                        (64..512).contains(&ring.len()),
                        "level={} ppm={ppm}",
                        ring.len()
                    );
                }
            }
            if ppm != 0 {
                assert!(adjusted > 100);
            }
        }
    }
}
