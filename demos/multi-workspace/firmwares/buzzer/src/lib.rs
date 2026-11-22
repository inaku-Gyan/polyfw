#![no_std]

pub fn alert_pattern() -> patterns::PulsePattern {
    patterns::FAST
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
