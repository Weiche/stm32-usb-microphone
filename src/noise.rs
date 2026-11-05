//! Firmware controls and telemetry around the portable Wiener DSP.
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use embassy_stm32::gpio::{Input, Output};
use embassy_time::{Instant, Timer};
use stm32f4_usb_mic::denoiser::{BLOCK, Denoiser};

static ENABLED: AtomicBool = AtomicBool::new(true);
static RECALIBRATE: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
static CALIBRATION: AtomicU32 = AtomicU32::new(0);
static HOPS: AtomicU32 = AtomicU32::new(0);
static MAX_US: AtomicU32 = AtomicU32::new(0);

pub fn process(state: &mut Denoiser, input: &[i16; BLOCK], output: &mut [f32; BLOCK]) {
    let work = crate::perf::begin();
    let start = Instant::now();
    if RECALIBRATE.swap(false, Relaxed) {
        state.recalibrate();
    }
    state.set_enabled(ENABLED.load(Relaxed));
    if state.process_f32(input, output) {
        HOPS.fetch_add(1, Relaxed);
        MAX_US.fetch_max(start.elapsed().as_micros() as u32, Relaxed);
    }
    READY.store(state.ready(), Relaxed);
    CALIBRATION.store(state.calibration_frames(), Relaxed);
    crate::perf::end_noise(work);
}

pub fn log_stats() {
    defmt::info!(
        "noise enabled={} ready={} calibration_frames={} hops={} hop_max_us={} cap_db=12 delay_samples=256",
        ENABLED.load(Relaxed),
        READY.load(Relaxed),
        CALIBRATION.load(Relaxed),
        HOPS.load(Relaxed),
        MAX_US.load(Relaxed)
    );
}

/// Short press: gain. Hold 1-3 s: bypass. Hold >=3 s: relearn on release.
/// Red LED is calibration; orange LED is ready + suppression enabled.
#[embassy_executor::task]
pub async fn controls_task(
    button: Input<'static>,
    mut active_led: Output<'static>,
    mut learn_led: Output<'static>,
) {
    let mut pressed_at = None;
    loop {
        Timer::after_millis(20).await;
        let ready = READY.load(Relaxed);
        active_led.set_level((ready && ENABLED.load(Relaxed)).into());
        learn_led.set_level((!ready).into());
        if button.is_high() {
            if pressed_at.is_none() {
                pressed_at = Some(Instant::now());
            }
        } else if let Some(started) = pressed_at.take() {
            let held = started.elapsed().as_millis();
            if held >= 3000 {
                READY.store(false, Relaxed);
                RECALIBRATE.store(true, Relaxed);
                defmt::info!("noise recalibration requested: keep quiet for 5 seconds");
            } else if held >= 1000 {
                let enabled = !ENABLED.load(Relaxed);
                ENABLED.store(enabled, Relaxed);
                defmt::info!("noise enabled={}", enabled);
            } else if held >= 20 {
                crate::pipe::cycle_gain();
            }
        }
    }
}
