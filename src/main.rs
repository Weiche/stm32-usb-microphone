#![no_std]
#![no_main]

mod board;
mod stats;

use core::sync::atomic::Ordering::Relaxed;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::{bind_interrupts, peripherals, usb};
use embassy_usb::class::uac::device::source::{
    AudioSource, Config as SourceConfig, ControlMonitor, FeatureUnitControls, State, Stream,
};
use embassy_usb::class::uac::device::{Channel, SampleWidth};
use embassy_usb::driver::EndpointError;
use panic_probe as _;
use static_cell::StaticCell;
use stm32f4_usb_mic::{SAMPLE_RATE_HZ, SAMPLES_PER_PACKET, SILENCE_PACKET, SINE_PACKET};

type UsbDriver = usb::Driver<'static, peripherals::USB_OTG_FS>;

bind_interrupts!(struct Irqs {
    OTG_FS => usb::InterruptHandler<peripherals::USB_OTG_FS>;
});

#[embassy_executor::task]
async fn usb_task(mut device: embassy_usb::UsbDevice<'static, UsbDriver>) {
    device.run().await;
}

#[embassy_executor::task]
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

#[embassy_executor::task]
async fn diagnostics_task(mut led: Output<'static>, vbus: Input<'static>) {
    loop {
        embassy_time::Timer::after_secs(1).await;
        led.toggle();
        defmt::info!(
            "stats vbus={} cfg={} suspend={} stream={} packets={} usb_errors={} disabled={} discarded_samples={} pending={} high={} dma_active=false",
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
        );
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    defmt::info!("stage1 boot: MB997B STM32F407VG UAC1 16000Hz s16le mono sine=1000Hz");
    let p = embassy_stm32::init(board::config());
    defmt::info!("clocks ready: SYSCLK=168MHz USB=48MHz HSE=8MHz crystal");

    // MB997 B-02 sheet 5: U6 STMPS2141 has active-low EN on PC0.
    // HIGH disables the host VBUS power switch in USB device mode.
    // Keep the GPIO objects alive for the lifetime of main's static task frame.
    let _vbus_power_off = Output::new(p.PC0, Level::High, Speed::Low);
    let vbus_sense = Input::new(p.PA9, Pull::None);
    let led = Output::new(p.PD12, Level::Low, Speed::Low);

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
    spawner.spawn(usb_task(device).unwrap());
    spawner.spawn(audio_task(stream, control_monitor).unwrap());
    spawner.spawn(diagnostics_task(led, vbus_sense).unwrap());
    core::future::pending::<()>().await;
}
