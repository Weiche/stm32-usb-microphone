//! Shared static PCM queue. Short critical sections; no lock is held over await.
use crate::stats;
use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
use critical_section::Mutex;
#[cfg(not(feature = "noise-suppression"))]
use embassy_stm32::gpio::Input;
#[cfg(not(feature = "noise-suppression"))]
use stm32f4_usb_mic::audio_queue::gain;
#[cfg(feature = "noise-suppression")]
use stm32f4_usb_mic::audio_queue::gain_f32;
use stm32f4_usb_mic::audio_queue::{PacketRate, PcmRing};
use stm32f4_usb_mic::resampler::Resampler;

const CAPACITY: usize = 1024;
const PREFILL: usize = 256;
#[cfg(feature = "noise-suppression")]
type Sample = f32;
#[cfg(not(feature = "noise-suppression"))]
type Sample = i16;
#[cfg(feature = "noise-suppression")]
static RING: Mutex<RefCell<PcmRing<CAPACITY, Sample>>> =
    Mutex::new(RefCell::new(PcmRing::with_zero(0.0)));
#[cfg(not(feature = "noise-suppression"))]
static RING: Mutex<RefCell<PcmRing<CAPACITY, Sample>>> = Mutex::new(RefCell::new(PcmRing::new()));
pub static GAIN_Q8: AtomicU32 = AtomicU32::new(1024);
static LOW: AtomicU32 = AtomicU32::new(u32::MAX);
static HIGH: AtomicU32 = AtomicU32::new(0);
static OVERFLOWS: AtomicU32 = AtomicU32::new(0);
static LOST: AtomicU32 = AtomicU32::new(0);
static UNDERRUNS: AtomicU32 = AtomicU32::new(0);
static MISSING: AtomicU32 = AtomicU32::new(0);
static IDLE_DISCARDED: AtomicU32 = AtomicU32::new(0);
static RESET_DISCARDED: AtomicU32 = AtomicU32::new(0);
static PRIMING: AtomicU32 = AtomicU32::new(0);
static UP: AtomicU32 = AtomicU32::new(0);
static DOWN: AtomicU32 = AtomicU32::new(0);
static RATE_Q16: AtomicU32 = AtomicU32::new(65536);
pub static PREPARE_MAX_US: AtomicU32 = AtomicU32::new(0);
static CLIPPED: AtomicU32 = AtomicU32::new(0);

pub fn active() -> bool {
    stats::CONFIGURED.load(Relaxed)
        && stats::STREAMING.load(Relaxed)
        && !stats::SUSPENDED.load(Relaxed)
}
pub fn clear() {
    let removed = critical_section::with(|cs| RING.borrow(cs).borrow_mut().clear());
    RESET_DISCARDED.fetch_add(removed as u32, Relaxed);
}
pub fn publish(samples: &[Sample]) {
    if !active() {
        clear();
        IDLE_DISCARDED.fetch_add(samples.len() as u32, Relaxed);
        return;
    }
    let (lost, level) = critical_section::with(|cs| {
        let mut ring = RING.borrow(cs).borrow_mut();
        let lost = ring.push(samples);
        (lost, ring.len())
    });
    HIGH.fetch_max(level as u32, Relaxed);
    if lost > 0 {
        OVERFLOWS.fetch_add(1, Relaxed);
        LOST.fetch_add(lost as u32, Relaxed);
    }
}

/// Consume even when muted, so mute never backs up capture or changes its clock.
pub fn packet(
    rate: &mut PacketRate,
    resampler: &mut Resampler,
    priming: &mut bool,
    muted: bool,
    bytes: &mut [u8; 32],
) {
    bytes.fill(0);
    if !active() {
        *priming = true;
        *rate = PacketRate::new();
        *resampler = Resampler::new();
        return;
    }
    let level = critical_section::with(|cs| RING.borrow(cs).borrow().len());
    if *priming {
        if level < PREFILL {
            PRIMING.fetch_add(1, Relaxed);
            return;
        }
        *priming = false;
        *rate = PacketRate::new();
        *resampler = Resampler::new();
    }
    let step = rate.step_q16(level);
    RATE_Q16.store(step, Relaxed);
    if step > 65536 {
        UP.fetch_add(1, Relaxed);
    } else if step < 65536 {
        DOWN.fetch_add(1, Relaxed);
    }
    let amplification = GAIN_Q8.load(Relaxed);
    // FIR work runs with interrupts enabled. Lock only one source sample at a time.
    for i in 0..16 {
        #[cfg(not(feature = "noise-suppression"))]
        let sample = resampler.next(step, || {
            critical_section::with(|cs| RING.borrow(cs).borrow_mut().pop())
        });
        #[cfg(feature = "noise-suppression")]
        let sample = resampler.next_f32(step, || {
            critical_section::with(|cs| RING.borrow(cs).borrow_mut().pop())
        });
        let Some(sample) = sample else {
            UNDERRUNS.fetch_add(1, Relaxed);
            MISSING.fetch_add((16 - i) as u32, Relaxed);
            *priming = true;
            return;
        };
        #[cfg(not(feature = "noise-suppression"))]
        let (sample, clipped) = gain(sample, amplification);
        #[cfg(feature = "noise-suppression")]
        let (sample, clipped) = gain_f32(sample, amplification);
        if clipped {
            CLIPPED.fetch_add(1, Relaxed);
        }
        if !muted {
            bytes[2 * i..2 * i + 2].copy_from_slice(&sample.to_le_bytes());
        }
    }
    let level = critical_section::with(|cs| RING.borrow(cs).borrow().len());
    LOW.fetch_min(level as u32, Relaxed);
}

pub fn log_stats() {
    let level = critical_section::with(|cs| RING.borrow(cs).borrow().len());
    let low = LOW.load(Relaxed);
    defmt::info!(
        "pipe level={} low={} high={} overflow_events={} lost_samples={} underruns={} missing_samples={} priming_packets={} rate_up_frames={} rate_down_frames={} rate_q16={} gain_q8={} clipped={} idle_discarded={} reset_discarded={} prepare_max_us={}",
        level,
        if low == u32::MAX { 0 } else { low },
        HIGH.load(Relaxed),
        OVERFLOWS.load(Relaxed),
        LOST.load(Relaxed),
        UNDERRUNS.load(Relaxed),
        MISSING.load(Relaxed),
        PRIMING.load(Relaxed),
        UP.load(Relaxed),
        DOWN.load(Relaxed),
        RATE_Q16.load(Relaxed),
        GAIN_Q8.load(Relaxed),
        CLIPPED.load(Relaxed),
        IDLE_DISCARDED.load(Relaxed),
        RESET_DISCARDED.load(Relaxed),
        PREPARE_MAX_US.load(Relaxed)
    );
}

pub fn cycle_gain() {
    let gain = match GAIN_Q8.load(Relaxed) {
        256 => 1024,
        1024 => 4096,
        _ => 256,
    };
    GAIN_Q8.store(gain, Relaxed);
    defmt::info!("microphone gain_q8={}", gain);
}

#[cfg(not(feature = "noise-suppression"))]
#[embassy_executor::task]
pub async fn gain_button_task(button: Input<'static>) {
    let mut was_pressed = false;
    loop {
        embassy_time::Timer::after_millis(20).await;
        let pressed = button.is_high();
        if pressed && !was_pressed {
            cycle_gain();
            embassy_time::Timer::after_millis(150).await;
        }
        was_pressed = pressed;
    }
}
