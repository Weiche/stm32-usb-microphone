use embassy_stm32::rcc::*;
use embassy_stm32::time::Hertz;

/// STM32F407 ART Flash read acceleration. Call after RCC initialization, which
/// writes FLASH_ACR latency and clears the cache/prefetch enables in this HAL.
/// These are Flash-side caches, not a DMA-visible SRAM data cache.
pub fn enable_flash_acceleration() {
    let flash = embassy_stm32::pac::FLASH;
    let before = flash.acr().read().0;
    flash.acr().modify(|w| {
        w.set_icen(true);
        w.set_dcen(true);
        w.set_prften(true);
    });
    cortex_m::asm::dsb();
    cortex_m::asm::isb();
    defmt::info!(
        "flash ACR before={=u32:x} after={=u32:x}",
        before,
        flash.acr().read().0
    );
}

pub fn config() -> embassy_stm32::Config {
    let mut config = embassy_stm32::Config::default();
    // MB997 B-02 schematic sheet 3: X2, 8 MHz on PH0/PH1.
    // This is a crystal, NOT a Nucleo ST-LINK MCO/bypass clock.
    config.rcc.hse = Some(Hse {
        freq: Hertz(8_000_000),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll_src = PllSource::Hse;
    config.rcc.pll = Some(Pll {
        prediv: PllPreDiv::Div8,
        mul: PllMul::Mul336,
        divp: Some(PllPDiv::Div2), // 8 / 8 * 336 / 2 = 168 MHz
        divq: Some(PllQDiv::Div7), // 8 / 8 * 336 / 7 = 48 MHz USB
        divr: None,
    });
    config.rcc.sys = Sysclk::Pll1P;
    config.rcc.ahb_pre = AHBPrescaler::Div1;
    config.rcc.apb1_pre = APBPrescaler::Div4; // 42 MHz
    config.rcc.apb2_pre = APBPrescaler::Div2; // 84 MHz
    config.rcc.mux.clk48sel = mux::Clk48sel::Pll1Q;
    #[cfg(feature = "pdm-capture")]
    {
        // Shared PLLM=8. 8/8*256/5 = 51.2 MHz I2S kernel.
        // I2SDIV=25, ODD=0: bit clock = 51.2 MHz/50 = 1.024 MHz.
        config.rcc.plli2s = Some(Pll {
            prediv: PllPreDiv::Div8,
            mul: PllMul::Mul256,
            divp: None,
            divq: None,
            divr: Some(PllRDiv::Div5),
        });
    }
    config
}
