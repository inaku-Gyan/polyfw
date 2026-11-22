#![no_std]
#![no_main]

#[cortex_m_rt::entry]
fn main() -> ! {
    loop {
        core::hint::black_box(screen::dashboard());
    }
}
