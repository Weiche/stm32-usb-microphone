//! PD15 / blue LD6, hardware TIM4_CH4 PWM at 2 kHz; no PWM IRQ or DMA.
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
use embassy_stm32::{peripherals, timer::simple_pwm::SimplePwm};
use embassy_time::{Duration, Ticker};
use stm32f4_usb_mic::level_meter::{LevelMeter, Reading};

static RMS: AtomicU32 = AtomicU32::new(0);
static BRIGHTNESS: AtomicU32 = AtomicU32::new(0);
static DUTY: AtomicU32 = AtomicU32::new(0);
static HIGH_RMS: AtomicU32 = AtomicU32::new(0);
static HIGH_DUTY: AtomicU32 = AtomicU32::new(0);
static UPDATES: AtomicU32 = AtomicU32::new(0);

pub fn observe(meter: &mut LevelMeter, samples: impl IntoIterator<Item = f32>) {
    #[cfg(feature = "live-mic")]
    let gain = crate::pipe::GAIN_Q8.load(Relaxed);
    #[cfg(not(feature = "live-mic"))]
    let gain = 256;
    if let Some(Reading {
        rms_lsb,
        brightness_permille,
        pwm_permille,
    }) = meter.update(samples, gain)
    {
        RMS.store(rms_lsb, Relaxed);
        BRIGHTNESS.store(brightness_permille, Relaxed);
        DUTY.store(pwm_permille, Relaxed);
        HIGH_RMS.fetch_max(rms_lsb, Relaxed);
        HIGH_DUTY.fetch_max(pwm_permille, Relaxed);
        UPDATES.fetch_add(1, Relaxed);
    }
}

pub fn log_stats() {
    defmt::info!(
        "level rms_lsb={} brightness_permille={} pwm_permille={} rms_high={} pwm_high={} updates={}",
        RMS.load(Relaxed),
        BRIGHTNESS.load(Relaxed),
        DUTY.load(Relaxed),
        HIGH_RMS.swap(0, Relaxed),
        HIGH_DUTY.swap(0, Relaxed),
        UPDATES.load(Relaxed)
    );
}

#[embassy_executor::task]
pub async fn task(mut pwm: SimplePwm<'static, peripherals::TIM4>) {
    let mut channel = pwm.ch4();
    channel.set_duty_cycle_fully_off();
    channel.enable();
    defmt::info!(
        "level LED: blue PD15 TIM4_CH4 PWM=2000Hz max_duty={} RMS=20ms",
        channel.max_duty_cycle()
    );
    let mut ticker = Ticker::every(Duration::from_millis(20));
    loop {
        ticker.next().await;
        channel.set_duty_cycle_fraction(DUTY.load(Relaxed).min(1000), 1000);
    }
}
