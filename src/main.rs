#![no_std]
#![no_main]

mod board;
#[cfg(feature = "pdm-capture")]
mod level_led;
#[cfg(feature = "noise-suppression")]
mod noise;
#[cfg(feature = "pdm-capture")]
mod pdm;
mod perf;
#[cfg(feature = "live-mic")]
mod pipe;
mod stats;

use core::sync::atomic::Ordering::Relaxed;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
#[cfg(feature = "live-mic")]
use embassy_stm32::interrupt;
#[cfg(feature = "live-mic")]
use embassy_stm32::interrupt::{InterruptExt, Priority};
use embassy_stm32::{bind_interrupts, peripherals, usb};
use embassy_usb::class::uac::device::source::{
    AudioSource, Config as SourceConfig, ControlMonitor, FeatureUnitControls, State, Stream,
};
use embassy_usb::class::uac::device::{Channel, SampleWidth};
use embassy_usb::driver::EndpointError;
use panic_probe as _;
use static_cell::StaticCell;
use stm32f4_usb_mic::SAMPLE_RATE_HZ;
#[cfg(not(feature = "live-mic"))]
use stm32f4_usb_mic::{SAMPLES_PER_PACKET, SILENCE_PACKET, SINE_PACKET};

type UsbDriver = usb::Driver<'static, peripherals::USB_OTG_FS>;

// UART4 is unused by this application; its vector is software-pended only.
#[cfg(feature = "live-mic")]
static USB_EXECUTOR: embassy_executor::InterruptExecutor =
    embassy_executor::InterruptExecutor::new();
#[cfg(feature = "live-mic")]
#[interrupt]
unsafe fn UART4() {
    unsafe { USB_EXECUTOR.on_interrupt() }
}

bind_interrupts!(struct Irqs {
    OTG_FS => usb::InterruptHandler<peripherals::USB_OTG_FS>;
    #[cfg(feature = "pdm-capture")]
    DMA1_STREAM3 => pdm::MicDmaIrq, embassy_stm32::dma::InterruptHandler<peripherals::DMA1_CH3>;
});

#[embassy_executor::task]
async fn usb_task(mut device: embassy_usb::UsbDevice<'static, UsbDriver>) {
    device.run().await;
}

#[embassy_executor::task]
#[cfg(not(feature = "live-mic"))]
async fn audio_task(mut stream: Stream<'static, UsbDriver>, monitor: ControlMonitor<'static>) {
    loop {
        stream.wait_connection().await;
        // No timer: IN transfers are paced by the host's 1 ms USB frames.
        // At this stage there is no independently clocked capture producer/ring.
        let packet = if monitor.muted() {
            &SILENCE_PACKET
        } else {
            &SINE_PACKET
        };
        stats::WATER_SAMPLES.store(SAMPLES_PER_PACKET as u32, Relaxed);
        stats::WATER_HIGH.fetch_max(SAMPLES_PER_PACKET as u32, Relaxed);
        let result = stream.write_packet(packet).await;
        stats::WATER_SAMPLES.store(0, Relaxed);
        match result {
            Ok(()) => {
                stats::PACKETS.fetch_add(1, Relaxed);
            }
            Err(EndpointError::Disabled) => {
                // Normal on alt=0, reset or unplug: wait for a new enabled stream.
                stats::DISABLED.fetch_add(1, Relaxed);
                stats::DROPPED_SAMPLES.fetch_add(SAMPLES_PER_PACKET as u32, Relaxed);
            }
            Err(EndpointError::BufferOverflow) => {
                stats::USB_ERRORS.fetch_add(1, Relaxed);
                stats::DROPPED_SAMPLES.fetch_add(SAMPLES_PER_PACKET as u32, Relaxed);
                defmt::error!("USB packet exceeds endpoint capacity");
                // A configuration fault must not busy-loop on the executor.
                embassy_time::Timer::after_millis(100).await;
            }
        }
    }
}

#[cfg(feature = "live-mic")]
#[embassy_executor::task]
async fn audio_task(mut stream: Stream<'static, UsbDriver>, monitor: ControlMonitor<'static>) {
    let mut bytes = [0u8; 32];
    loop {
        stream.wait_connection().await;
        let mut state = stm32f4_usb_mic::audio_output::PacketState::new();
        loop {
            let started = embassy_time::Instant::now();
            let work = perf::begin();
            pipe::packet(&mut state, monitor.muted(), &mut bytes);
            pipe::PREPARE_MAX_US.fetch_max(started.elapsed().as_micros() as u32, Relaxed);
            perf::end_audio(work);
            match stream.write_packet(&bytes).await {
                Ok(()) => {
                    stats::PACKETS.fetch_add(1, Relaxed);
                }
                Err(error) => {
                    stats::DROPPED_SAMPLES.fetch_add(16, Relaxed);
                    match error {
                        EndpointError::Disabled => {
                            stats::DISABLED.fetch_add(1, Relaxed);
                        }
                        EndpointError::BufferOverflow => {
                            stats::USB_ERRORS.fetch_add(1, Relaxed);
                            embassy_time::Timer::after_millis(10).await;
                        }
                    }
                    pipe::clear();
                    break;
                }
            }
        }
    }
}

#[embassy_executor::task]
async fn diagnostics_task(mut led: Output<'static>, vbus: Input<'static>) {
    let mut work = perf::Reporter::new();
    loop {
        embassy_time::Timer::after_secs(1).await;
        led.toggle();
        defmt::info!(
            "stats vbus={} cfg={} suspend={} stream={} packets={} usb_errors={} disabled={} discarded_samples={} pending={} high={} dma_active={}",
            vbus.is_high(),
            stats::CONFIGURED.load(Relaxed),
            stats::SUSPENDED.load(Relaxed),
            stats::STREAMING.load(Relaxed),
            stats::PACKETS.load(Relaxed),
            stats::USB_ERRORS.load(Relaxed),
            stats::DISABLED.load(Relaxed),
            stats::DROPPED_SAMPLES.load(Relaxed),
            stats::WATER_SAMPLES.load(Relaxed),
            stats::WATER_HIGH.load(Relaxed),
            cfg!(feature = "pdm-capture"),
        );
        #[cfg(feature = "pdm-capture")]
        pdm::log_stats();
        #[cfg(feature = "pdm-capture")]
        level_led::log_stats();
        #[cfg(feature = "live-mic")]
        pipe::log_stats();
        #[cfg(feature = "noise-suppression")]
        noise::log_stats();
        work.log();
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    perf::init();
    defmt::info!(
        "usb-mic boot: MB997B STM32F407VG UAC1 16000Hz s16le mono pdm_capture={} live_mic={}",
        cfg!(feature = "pdm-capture"),
        cfg!(feature = "live-mic")
    );
    let p = embassy_stm32::init(board::config());
    #[cfg(feature = "audio-gap-test")]
    defmt::warn!("audio-gap-test: diagnostic firmware, three deliberate capture pauses enabled");
    board::enable_flash_acceleration();
    defmt::info!("clocks ready: SYSCLK=168MHz USB=48MHz HSE=8MHz crystal");
    #[cfg(feature = "pdm-capture")]
    {
        let microphone = pdm::new(p.SPI2, p.PC3, p.PB12, p.PB10, p.DMA1_CH3, Irqs);
        spawner.spawn(pdm::capture_task(microphone).unwrap());
        spawner.spawn(pdm::snapshot_task().unwrap());
        let pwm = embassy_stm32::timer::simple_pwm::SimplePwm::new(
            p.TIM4,
            None,
            None,
            None,
            Some(embassy_stm32::timer::simple_pwm::PwmPin::new(
                p.PD15,
                embassy_stm32::gpio::OutputType::PushPull,
            )),
            embassy_stm32::time::Hertz(2000),
            Default::default(),
        );
        spawner.spawn(level_led::task(pwm).unwrap());
    }

    // MB997 B-02 sheet 5: U6 STMPS2141 has active-low EN on PC0.
    // HIGH disables the host VBUS power switch in USB device mode.
    // Keep the GPIO objects alive for the lifetime of main's static task frame.
    let _vbus_power_off = Output::new(p.PC0, Level::High, Speed::Low);
    let vbus_sense = Input::new(p.PA9, Pull::None);
    let led = Output::new(p.PD12, Level::Low, Speed::Low);
    #[cfg(all(feature = "live-mic", not(feature = "noise-suppression")))]
    spawner.spawn(pipe::gain_button_task(Input::new(p.PA0, Pull::None)).unwrap());
    #[cfg(feature = "noise-suppression")]
    {
        spawner.spawn(
            noise::controls_task(
                Input::new(p.PA0, Pull::None),
                Output::new(p.PD13, Level::Low, Speed::Low),
                Output::new(p.PD14, Level::High, Speed::Low),
            )
            .unwrap(),
        );
        defmt::info!(
            "noise: 12dB cap; startup calibration, keep quiet for 5 seconds; USER hold 1-3s=bypass, >=3s=relearn"
        );
        #[cfg(feature = "adaptive-noise")]
        defmt::info!("noise: experimental guarded background tracking enabled");
    }

    static EP_OUT: StaticCell<[u8; 256]> = StaticCell::new();
    let mut driver_config = usb::Config::default();
    driver_config.vbus_detection = true;
    let driver = usb::Driver::new_fs(
        p.USB_OTG_FS,
        p.PA12,
        p.PA11,
        Irqs,
        EP_OUT.init([0; 256]),
        driver_config,
    );

    // Embassy example IDs, for private lab testing only; not an assigned VID/PID.
    let mut config = embassy_usb::Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("STM32 Rust Lab");
    config.product = Some("STM32F4 USB Microphone");
    config.serial_number = Some("MB997B-STAGE1-0001");
    config.bcd_usb = embassy_usb::UsbVersion::Two;
    config.self_powered = true; // CN1 ST-LINK supplies the board independently of CN5.
    config.max_power = 0; // CN5 is data/VBUS sensing; board power comes from CN1.

    static CONFIG: StaticCell<[u8; 512]> = StaticCell::new();
    static BOS: StaticCell<[u8; 64]> = StaticCell::new();
    static CONTROL: StaticCell<[u8; 64]> = StaticCell::new();
    static AUDIO: StaticCell<State<'static>> = StaticCell::new();
    static EVENTS: StaticCell<stats::UsbEvents> = StaticCell::new();
    let mut builder = embassy_usb::Builder::new(
        driver,
        config,
        CONFIG.init([0; 512]),
        BOS.init([0; 64]),
        &mut [],
        CONTROL.init([0; 64]),
    );
    builder.handler(EVENTS.init(stats::UsbEvents));
    let mut source_config = SourceConfig::new(
        &[SAMPLE_RATE_HZ],
        SampleWidth::Width2Byte,
        &[Channel::LeftFront],
    );
    // Advertise only controls we apply; optional volume/gain belongs to stage 4.
    source_config.feature_unit = [FeatureUnitControls::MUTE, FeatureUnitControls::empty()].into();
    let AudioSource {
        stream,
        control_monitor,
    } = AudioSource::new(&mut builder, AUDIO.init(State::new()), source_config);
    let device = builder.build();
    defmt::info!(
        "descriptors ready: config_bytes={} packet=32 endpoint_max=34",
        device.buffer_usage().config_descriptor_used
    );
    #[cfg(feature = "live-mic")]
    {
        interrupt::OTG_FS.set_priority(Priority::P2);
        interrupt::DMA1_STREAM3.set_priority(Priority::P3);
        interrupt::UART4.set_priority(Priority::P6);
        let usb_spawner = USB_EXECUTOR.start(interrupt::UART4);
        usb_spawner.spawn(audio_task(stream, control_monitor).unwrap());
    }
    #[cfg(not(feature = "live-mic"))]
    {
        spawner.spawn(audio_task(stream, control_monitor).unwrap());
    }
    spawner.spawn(usb_task(device).unwrap());
    spawner.spawn(diagnostics_task(led, vbus_sense).unwrap());
    core::future::pending::<()>().await;
}
