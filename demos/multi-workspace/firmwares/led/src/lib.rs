#![no_std]

pub enum BlinkRate {
    Slow,
    Fast,
}

pub fn blink_pattern(rate: BlinkRate) -> patterns::PulsePattern {
    match rate {
        BlinkRate::Slow => patterns::SLOW,
        BlinkRate::Fast => patterns::FAST,
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_blinks_have_shorter_on_and_off_intervals_than_slow_blinks() {
        let fast = blink_pattern(BlinkRate::Fast);
        let slow = blink_pattern(BlinkRate::Slow);
        assert!(fast.on.0 < slow.on.0);
        assert!(fast.off.0 < slow.off.0);
    }
}
