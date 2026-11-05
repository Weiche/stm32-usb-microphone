//! MP45DT02: PB10 I2S2 clock, PC3 data; PB12 is unused WS at the header.
use core::cell::RefCell;
use core::sync::atomic::{
    AtomicBool, AtomicU32,
    Ordering::{Acquire, Relaxed, Release},
};
use critical_section::Mutex;
use embassy_stm32::interrupt::typelevel::Handler;
use embassy_stm32::time::Hertz;
use embassy_stm32::{Peri, dma, i2s, interrupt, pac, peripherals};
use static_cell::StaticCell;
use stm32f4_usb_mic::decoder::Decoder;

pub const DMA_WORDS: usize = 512;
pub const BLOCK_WORDS: usize = 64; // 1024 PDM bits, 1 ms at 1.024 MHz.
const SNAPSHOT_WORDS: usize = 4096;
static SNAPSHOT: Mutex<RefCell<[u16; SNAPSHOT_WORDS]>> =
    Mutex::new(RefCell::new([0; SNAPSHOT_WORDS]));
static SNAPSHOT_READY: AtomicBool = AtomicBool::new(false);
static PCM_SNAPSHOT: Mutex<RefCell<[i16; SNAPSHOT_WORDS / 4]>> =
    Mutex::new(RefCell::new([0; SNAPSHOT_WORDS / 4]));
static DECODE_MAX_US: AtomicU32 = AtomicU32::new(0);
#[derive(Clone, Copy)]
struct PcmMetrics {
    samples: u32,
    sum: i32,
    sumsq: u64,
    peak: u32,
}
impl PcmMetrics {
    const EMPTY: Self = Self {
        samples: 0,
        sum: 0,
        sumsq: 0,
        peak: 0,
    };
}
static PCM_METRICS: Mutex<RefCell<PcmMetrics>> = Mutex::new(RefCell::new(PcmMetrics::EMPTY));
pub static WORDS: AtomicU32 = AtomicU32::new(0);
pub static OVERRUNS: AtomicU32 = AtomicU32::new(0);
pub static DMA_ERRORS: AtomicU32 = AtomicU32::new(0);
static FAULT_LATCHED: AtomicBool = AtomicBool::new(false);
static PENDING: AtomicU32 = AtomicU32::new(0);
static HIGH: AtomicU32 = AtomicU32::new(0);
static LOW: AtomicU32 = AtomicU32::new(u32::MAX);
static ONES: AtomicU32 = AtomicU32::new(0);
static BITS: AtomicU32 = AtomicU32::new(0);

/// Count hardware faults BEFORE Embassy's handler (which panics on TEIF).
pub struct MicDmaIrq;
impl Handler<interrupt::typelevel::DMA1_STREAM3> for MicDmaIrq {
    unsafe fn on_interrupt() {
        let flags = pac::DMA1.isr(0).read();
        if (flags.teif(3) || flags.dmeif(3) || flags.feif(3)) && !FAULT_LATCHED.swap(true, Relaxed)
        {
            DMA_ERRORS.fetch_add(1, Relaxed);
            defmt::error!("pdm DMA hardware error flags={=u32:x}", flags.0);
        }
    }
}

pub fn new(
    spi: Peri<'static, peripherals::SPI2>,
    sd: Peri<'static, peripherals::PC3>,
    ws: Peri<'static, peripherals::PB12>,
    ck: Peri<'static, peripherals::PB10>,
    dma: Peri<'static, peripherals::DMA1_CH3>,
    irq: impl interrupt::typelevel::Binding<
        interrupt::typelevel::DMA1_STREAM3,
        dma::InterruptHandler<peripherals::DMA1_CH3>,
    > + 'static,
) -> i2s::I2S<'static, u16> {
    static BUFFER: StaticCell<[u16; DMA_WORDS]> = StaticCell::new();
    let buffer = BUFFER.init([0; DMA_WORDS]);
    let address = buffer.as_ptr() as usize;
    assert!(address >= 0x20000000 && address + core::mem::size_of_val(buffer) <= 0x20020000);
    assert_eq!(address & 1, 0);
    defmt::info!(
        "pdm DMA buffer={=usize:x} words={} SRAM=true",
        address,
        DMA_WORDS
    );
    let mut config = i2s::Config::default();
    config.frequency = Hertz(32_000); // I2S stereo frame rate; NOT PCM rate.
    config.mode = i2s::Mode::Master;
    config.standard = i2s::Standard::LsbFirst; // right justified, as ST BSP; wire is MSB first.
    config.format = i2s::Format::Data16Channel16;
    config.clock_polarity = i2s::ClockPolarity::IdleHigh;
    config.master_clock = false;
    let driver = i2s::I2S::new_rxonly_nomck(spi, sd, ws, ck, dma, irq, buffer, config);
    defmt::info!(
        "pdm configured: kernel=51200000 bitclock=1024000 I2SPR={=u32:x} I2SCFGR={=u32:x}",
        pac::SPI2.i2spr().read().0,
        pac::SPI2.i2scfgr().read().0
    );
    driver
}

#[embassy_executor::task]
pub async fn capture_task(mut driver: i2s::I2S<'static, u16>) {
    let mut block = [0u16; BLOCK_WORDS];
    let mut copied = 0;
    let mut snapshot_valid = true;
    let mut decoder = Decoder::new();
    let mut pcm = [0i16; BLOCK_WORDS / 4];
    #[cfg(feature = "noise-suppression")]
    let denoiser = {
        static STATE: StaticCell<stm32f4_usb_mic::denoiser::Denoiser> = StaticCell::new();
        STATE.init(stm32f4_usb_mic::denoiser::Denoiser::new())
    };
    #[cfg(feature = "noise-suppression")]
    let mut processed = [0.0f32; BLOCK_WORDS / 4];
    driver.start();
    loop {
        match driver.read(&mut block).await {
            Ok(()) => {
                let work = crate::perf::begin();
                let decode_start = embassy_time::Instant::now();
                let mut output = 0;
                let mut metrics = PcmMetrics::EMPTY;
                for &word in &block {
                    if let Some(sample) = decoder.process_word(word) {
                        pcm[output] = sample;
                        output += 1;
                        metrics.samples += 1;
                        metrics.sum += i32::from(sample);
                        metrics.sumsq += (i64::from(sample) * i64::from(sample)) as u64;
                        metrics.peak = metrics.peak.max(i32::from(sample).unsigned_abs());
                    }
                }
                assert_eq!(output, pcm.len());
                DECODE_MAX_US.fetch_max(decode_start.elapsed().as_micros() as u32, Relaxed);
                #[cfg(feature = "noise-suppression")]
                {
                    crate::noise::process(denoiser, &pcm, &mut processed);
                    crate::pipe::publish(&processed);
                }
                #[cfg(all(feature = "live-mic", not(feature = "noise-suppression")))]
                crate::pipe::publish(&pcm);
                critical_section::with(|cs| {
                    let mut accumulated = PCM_METRICS.borrow(cs).borrow_mut();
                    accumulated.samples += metrics.samples;
                    accumulated.sum += metrics.sum;
                    accumulated.sumsq += metrics.sumsq;
                    accumulated.peak = accumulated.peak.max(metrics.peak);
                });
                let total = WORDS
                    .fetch_add(BLOCK_WORDS as u32, Relaxed)
                    .wrapping_add(BLOCK_WORDS as u32);
                let ones: u32 = block.iter().map(|word| word.count_ones()).sum();
                ONES.fetch_add(ones, Relaxed);
                BITS.fetch_add((BLOCK_WORDS * 16) as u32, Relaxed);
                if let Ok(pending) = driver.rx_len() {
                    PENDING.store(pending as u32, Relaxed);
                    HIGH.fetch_max((pending + BLOCK_WORDS) as u32, Relaxed);
                    LOW.fetch_min(pending as u32, Relaxed);
                }
                // One finite snapshot after microphone startup; logging is in another task.
                if total >= 512_000 && copied < SNAPSHOT_WORDS && snapshot_valid {
                    critical_section::with(|cs| {
                        SNAPSHOT.borrow(cs).borrow_mut()[copied..copied + BLOCK_WORDS]
                            .copy_from_slice(&block);
                        PCM_SNAPSHOT.borrow(cs).borrow_mut()[copied / 4..copied / 4 + pcm.len()]
                            .copy_from_slice(&pcm);
                    });
                    copied += BLOCK_WORDS;
                    if copied == SNAPSHOT_WORDS {
                        SNAPSHOT_READY.store(true, Release);
                    }
                }
                crate::perf::end_dsp(work);
                // DMA wakes in bursts. Let USB/control tasks run between bounded DSP blocks.
                embassy_futures::yield_now().await;
            }
            Err(i2s::Error::Overrun) => {
                OVERRUNS.fetch_add(1, Relaxed);
                // The exact number overwritten is unknown; never invent a dropped-sample total.
                if copied > 0 && copied < SNAPSHOT_WORDS {
                    snapshot_valid = false;
                }
                driver.clear();
                decoder.reset();
                #[cfg(feature = "noise-suppression")]
                denoiser.reset_signal();
                defmt::warn!("pdm ring overrun: unknown input loss, realigning");
            }
            Err(error) => {
                defmt::panic!("pdm unexpected configuration error: {:?}", error);
            }
        }
    }
}

#[embassy_executor::task]
pub async fn snapshot_task() {
    while !SNAPSHOT_READY.load(Acquire) {
        embassy_time::Timer::after_millis(100).await;
    }
    defmt::info!(
        "pdm_snapshot begin words={} encoding=u16le rate_bits=1024000",
        SNAPSHOT_WORDS
    );
    defmt::info!(
        "pcm_snapshot begin samples={} rate_hz=16000",
        SNAPSHOT_WORDS / 4
    );
    for index in (0..SNAPSHOT_WORDS).step_by(64) {
        let mut chunk = [0u16; 64];
        critical_section::with(|cs| {
            chunk.copy_from_slice(&SNAPSHOT.borrow(cs).borrow()[index..index + 64])
        });
        defmt::info!("pdm_raw index={} data={:?}", index, &chunk[..]);
        let mut pcm_chunk = [0i16; 16];
        critical_section::with(|cs| {
            pcm_chunk.copy_from_slice(&PCM_SNAPSHOT.borrow(cs).borrow()[index / 4..index / 4 + 16])
        });
        defmt::info!("pcm_raw index={} data={:?}", index / 4, &pcm_chunk[..]);
        embassy_time::Timer::after_millis(100).await;
    }
    defmt::info!("pdm_snapshot end words={}", SNAPSHOT_WORDS);
    defmt::info!("pcm_snapshot end samples={}", SNAPSHOT_WORDS / 4);
}

pub fn log_stats() {
    let pcm = critical_section::with(|cs| PCM_METRICS.borrow(cs).replace(PcmMetrics::EMPTY));
    defmt::info!(
        "pcm samples={} sum={} sumsq={} peak={} decode_max_us={}",
        pcm.samples,
        pcm.sum,
        pcm.sumsq,
        pcm.peak,
        DECODE_MAX_US.load(Relaxed)
    );
    defmt::info!(
        "pdm words={} ones={} bits={} pending_words={} low={} high={} ring_overruns={} dma_errors={} spi_sr={=u32:x}",
        WORDS.load(Relaxed),
        ONES.swap(0, Relaxed),
        BITS.swap(0, Relaxed),
        PENDING.load(Relaxed),
        LOW.load(Relaxed),
        HIGH.load(Relaxed),
        OVERRUNS.load(Relaxed),
        DMA_ERRORS.load(Relaxed),
        pac::SPI2.sr().read().0
    );
}
