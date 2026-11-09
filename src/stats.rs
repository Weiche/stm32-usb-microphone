use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use embassy_usb::{Handler, types::InterfaceNumber};

pub static CONFIGURED: AtomicBool = AtomicBool::new(false);
pub static SUSPENDED: AtomicBool = AtomicBool::new(false);
pub static STREAMING: AtomicBool = AtomicBool::new(false);
pub static RESETS: AtomicU32 = AtomicU32::new(0);
pub static PACKETS: AtomicU32 = AtomicU32::new(0);
pub static USB_ERRORS: AtomicU32 = AtomicU32::new(0);
pub static DISABLED: AtomicU32 = AtomicU32::new(0);
pub static DROPPED_SAMPLES: AtomicU32 = AtomicU32::new(0);
pub static WATER_SAMPLES: AtomicU32 = AtomicU32::new(0);
pub static WATER_HIGH: AtomicU32 = AtomicU32::new(0);

pub struct UsbEvents;

impl Handler for UsbEvents {
    fn enabled(&mut self, enabled: bool) {
        if !enabled {
            clear_stream();
        }
        defmt::info!("usb enabled={}", enabled);
    }
    fn reset(&mut self) {
        RESETS.fetch_add(1, Relaxed);
        clear_stream();
        defmt::info!("usb reset");
    }
    fn configured(&mut self, configured: bool) {
        CONFIGURED.store(configured, Relaxed);
        if !configured {
            STREAMING.store(false, Relaxed);
            #[cfg(feature = "live-mic")]
            crate::pipe::clear();
        }
        defmt::info!("usb configured={}", configured);
    }
    fn suspended(&mut self, suspended: bool) {
        SUSPENDED.store(suspended, Relaxed);
        #[cfg(feature = "live-mic")]
        if suspended {
            crate::pipe::clear();
        }
        defmt::info!("usb suspended={}", suspended);
    }
    fn set_alternate_setting(&mut self, iface: InterfaceNumber, alt: u8) {
        // Sole function: AudioControl interface 0, AudioStreaming interface 1.
        if u8::from(iface) == 1 {
            STREAMING.store(alt == 1, Relaxed);
            #[cfg(feature = "live-mic")]
            crate::pipe::clear();
        }
        defmt::info!("usb interface={} alt={}", u8::from(iface), alt);
    }
}

fn clear_stream() {
    CONFIGURED.store(false, Relaxed);
    SUSPENDED.store(false, Relaxed);
    STREAMING.store(false, Relaxed);
    #[cfg(feature = "live-mic")]
    crate::pipe::clear();
}
