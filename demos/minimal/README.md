# NUCLEO-F401RE blinky

The `blinky` firmware toggles the board's green user LED (LD2 on PA5) every 500 ms using
SysTick. [ST Nucleo-64 user manual](https://www.st.com/resource/en/user_manual/dm00105823.pdf)

The Cargo package contains one `no_std` binary. The `nucleo-f401re` platform uses probe-rs chip
`STM32F401RETx`, with 512 KiB of flash and 96 KiB of SRAM in the linker script.
[Firmware source](src/main.rs) · [Project configuration](polyfw.toml) · [Linker memory map](memory.x)

## Build and flash

Use the development CLI from this checkout for binary-only firmware support. Run these commands
from the repository root.

```sh
just run --project-root demos/minimal info
just run --project-root demos/minimal verify blinky
just run --project-root demos/minimal build blinky --release
```

The binary sets `test = false` because it uses the board's peripherals. `verify` checks formatting
and runs Clippy for the MCU.

Install probe-rs and connect the board's ST-LINK USB port to flash the release build.
[Flash and monitor](../../docs/usage.md#flash-and-monitor)

```sh
just run --project-root demos/minimal flash blinky --release
```
