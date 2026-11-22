#![no_std]

pub struct Dashboard {
    pub title: layout::TextLine,
    pub refresh_interval: timing::Millis,
}

pub fn dashboard() -> Dashboard {
    Dashboard {
        title: layout::TextLine {
            text: "System ready",
            x: 8,
            y: 8,
        },
        refresh_interval: timing::Millis(100),
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
