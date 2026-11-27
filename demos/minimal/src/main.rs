#![no_std]
#![no_main]

use cortex_m::{Peripherals, delay::Delay};
use cortex_m_rt::entry;
use stm32_metapac::{GPIOA, RCC, gpio::vals::Moder};

const LD2_PIN: usize = 5;
const HSI_HZ: u32 = 16_000_000;
const HALF_PERIOD_MS: u32 = 500;

#[entry]
fn main() -> ! {
    // SysTick uses the internal oscillator selected at reset.
    let mut delay = Delay::new(Peripherals::take().unwrap().SYST, HSI_HZ);

    RCC.ahb1enr().modify(|r| r.set_gpioaen(true));
    // Read back the clock enable before accessing GPIOA.
    let _ = RCC.ahb1enr().read();
    GPIOA
        .moder()
        .modify(|r| r.set_moder(LD2_PIN, Moder::OUTPUT));

    loop {
        GPIOA.bsrr().write(|r| r.set_bs(LD2_PIN, true));
        delay.delay_ms(HALF_PERIOD_MS);
        GPIOA.bsrr().write(|r| r.set_br(LD2_PIN, true));
        delay.delay_ms(HALF_PERIOD_MS);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        cortex_m::asm::wfi();
    }
}
