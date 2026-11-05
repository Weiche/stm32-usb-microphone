//! Relative audio level display, not a calibrated acoustic SPL measurement.
//! 20 ms RMS, approximately logarithmic amplitude mapping and visual smoothing.

pub const WINDOW_SAMPLES: u32 = 320;
pub const FLOOR_LSB: u32 = 8;
pub const FULL_LSB: u32 = 2048;

#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub rms_lsb: u32,
    pub brightness_permille: u32,
    pub pwm_permille: u32,
}

pub struct LevelMeter {
    power: f32,
    samples: u32,
    brightness: u32,
}

impl Default for LevelMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl LevelMeter {
    pub const fn new() -> Self {
        Self {
            power: 0.0,
            samples: 0,
            brightness: 0,
        }
    }

    /// PCM is in pre-gain LSB units; apply the current Q8 gain to window energy.
    /// A gain change within a window takes effect at the next 20 ms reading.
    pub fn update(
        &mut self,
        input: impl IntoIterator<Item = f32>,
        gain_q8: u32,
    ) -> Option<Reading> {
        let mut reading = None;
        for sample in input {
            self.power += sample * sample;
            self.samples += 1;
            if self.samples == WINDOW_SAMPLES {
                let gain = gain_q8.min(4096) as f32 / 256.0;
                let mean = (self.power / WINDOW_SAMPLES as f32 * gain * gain)
                    .clamp(0.0, 32768.0 * 32768.0);
                let rms = (mean as u64).isqrt() as u32;
                let target = brightness_target(rms);
                if target > self.brightness {
                    self.brightness += (target - self.brightness).div_ceil(2);
                } else {
                    self.brightness -= (self.brightness - target).div_ceil(10);
                }
                reading = Some(Reading {
                    rms_lsb: rms,
                    brightness_permille: self.brightness,
                    // Gamma ~2: smooth apparent brightness at the dim end.
                    pwm_permille: self.brightness * self.brightness / 1000,
                });
                self.power = 0.0;
                self.samples = 0;
            }
        }
        reading
    }
}

fn brightness_target(rms: u32) -> u32 {
    if rms <= FLOOR_LSB {
        return 0;
    }
    if rms >= FULL_LSB {
        return 1000;
    }
    let mut base = FLOOR_LSB;
    let mut band = 0;
    while rms >= base * 2 {
        base *= 2;
        band += 1;
    }
    band * 125 + (rms - base) * 125 / base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amplitude_mapping_is_monotonic_and_bounded() {
        let mut last = 0;
        for rms in 0..=32768 {
            let current = brightness_target(rms);
            assert!((last..=1000).contains(&current));
            last = current;
        }
        assert_eq!(brightness_target(8), 0);
        assert_eq!(brightness_target(128), 500);
        assert_eq!(brightness_target(2048), 1000);
    }

    #[test]
    fn window_rms_and_gain_track_signal_without_losing_short_blocks() {
        let mut meter = LevelMeter::new();
        for _ in 0..19 {
            assert!(
                meter
                    .update([3.0, -3.0].into_iter().cycle().take(16), 1024)
                    .is_none()
            );
        }
        let value = meter.update([3.0; 16], 1024).unwrap();
        assert_eq!(value.rms_lsb, 12);
        assert!(value.brightness_permille > 0);
        let settled = meter.update([3.0; 320], 1024).unwrap();
        assert!(settled.pwm_permille > 0);
    }

    #[test]
    fn fast_attack_slow_release_eventually_reaches_dark() {
        let mut meter = LevelMeter::new();
        let loud = meter.update([FULL_LSB as f32; 320], 256).unwrap();
        assert_eq!(loud.brightness_permille, 500);
        let quiet = meter.update([0.0; 320], 256).unwrap();
        assert_eq!(quiet.brightness_permille, 450);
        for _ in 0..80 {
            meter.update([0.0; 320], 256);
        }
        let dark = meter.update([0.0; 320], 256).unwrap();
        assert_eq!(dark.pwm_permille, 0);
        assert_eq!(dark.brightness_permille, 0);
    }
}
