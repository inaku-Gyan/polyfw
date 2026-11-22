#![no_std]
#![no_main]

#[cortex_m_rt::entry]
fn main() -> ! {
    loop {
        core::hint::black_box(led::blink_pattern(led::BlinkRate::Fast));
    }
}
