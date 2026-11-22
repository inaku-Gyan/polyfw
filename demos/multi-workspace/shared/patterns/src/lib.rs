#![no_std]

use timing::Millis;

/// Reusable on/off durations for a buzzer or LED.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PulsePattern {
    pub on: Millis,
    pub off: Millis,
}

pub const FAST: PulsePattern = PulsePattern {
    on: Millis(100),
    off: Millis(100),
};

pub const SLOW: PulsePattern = PulsePattern {
    on: Millis(500),
    off: Millis(500),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_patterns_have_nonzero_on_and_off_durations() {
        for pattern in [FAST, SLOW] {
            assert!(pattern.on.0 > 0);
            assert!(pattern.off.0 > 0);
        }
    }

    #[test]
    fn fast_pulses_have_a_shorter_cycle_than_slow_pulses() {
        let fast_cycle = FAST.on.0 + FAST.off.0;
        let slow_cycle = SLOW.on.0 + SLOW.off.0;
        assert!(fast_cycle < slow_cycle);
    }
}
