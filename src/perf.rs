//! DWT accounting for DSP and audio preparation only; not total CPU utilization.
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
#[cfg(feature = "pdm-capture")]
use cortex_m::peripheral::DWT;

static DSP: AtomicU32 = AtomicU32::new(0);
static AUDIO: AtomicU32 = AtomicU32::new(0);
static NOISE: AtomicU32 = AtomicU32::new(0);

pub fn init() {
    let mut core = cortex_m::Peripherals::take().unwrap();
    core.DCB.enable_trace();
    core.DWT.set_cycle_count(0);
    core.DWT.enable_cycle_counter();
}

#[cfg(feature = "pdm-capture")]
pub struct Span {
    start: u32,
    audio: u32,
}
#[cfg(feature = "pdm-capture")]
pub fn begin() -> Span {
    Span {
        start: DWT::cycle_count(),
        audio: AUDIO.load(Relaxed),
    }
}
#[cfg(feature = "pdm-capture")]
fn exclusive(span: Span) -> u32 {
    let elapsed = DWT::cycle_count().wrapping_sub(span.start);
    elapsed.saturating_sub(AUDIO.load(Relaxed).wrapping_sub(span.audio))
}
#[cfg(feature = "pdm-capture")]
pub fn end_dsp(span: Span) {
    DSP.fetch_add(exclusive(span), Relaxed);
}
#[cfg(feature = "noise-suppression")]
pub fn end_noise(span: Span) {
    NOISE.fetch_add(exclusive(span), Relaxed);
}
#[cfg(feature = "live-mic")]
pub fn end_audio(span: Span) {
    AUDIO.fetch_add(DWT::cycle_count().wrapping_sub(span.start), Relaxed);
}

pub struct Reporter {
    last: [u32; 3],
    at: embassy_time::Instant,
}
impl Reporter {
    pub fn new() -> Self {
        Self {
            last: [DSP.load(Relaxed), AUDIO.load(Relaxed), NOISE.load(Relaxed)],
            at: embassy_time::Instant::now(),
        }
    }
    pub fn log(&mut self) {
        let now = embassy_time::Instant::now();
        let elapsed = (now - self.at).as_micros().max(1);
        let current = [DSP.load(Relaxed), AUDIO.load(Relaxed), NOISE.load(Relaxed)];
        let mut load = [0u64; 3];
        for i in 0..3 {
            load[i] = u64::from(current[i].wrapping_sub(self.last[i])) * 1000 / (elapsed * 168);
        }
        self.last = current;
        self.at = now;
        defmt::info!(
            "work dsp_permille={} audio_permille={} noise_subset_permille={} accounted_permille={} interval_us={} scope=instrumented_work_not_total_cpu",
            load[0],
            load[1],
            load[2],
            load[0] + load[1],
            elapsed
        );
    }
}
