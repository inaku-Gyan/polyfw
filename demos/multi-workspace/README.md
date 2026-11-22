# Multiple workspaces example

Four firmware entries for two illustrative boards, board-A and board-B, organized into three
Cargo workspaces. The example demonstrates shared dependencies across packages and shared library
code within a package. Its entry points consume pattern and layout data in loops without driving
peripherals.

## Layout

```text
multi-workspace/
├── Cargo.toml                 # root workspace: shared/*
├── polyfw.toml
├── rust-toolchain.toml        # shared by all three workspaces
├── shared/
│   ├── timing/                # millisecond intervals
│   ├── patterns/              # on/off patterns; depends on timing
│   └── layout/                # positioned screen text
├── bsp/
│   ├── board-a/               # workspace: buzzer, led
│   └── board-b/               # workspace: screen
└── firmwares/
    ├── buzzer/
    ├── screen/
    └── led/
        └── src/
            ├── lib.rs         # LED logic shared by both binaries
            └── bin/
                ├── led-1.rs   # slow blink pattern
                └── led-2.rs   # fast blink pattern
```

Each firmware package sets `package.workspace` to its owning BSP workspace. The shared libraries
belong to the root workspace. All three workspaces use the root `rust-toolchain.toml`; each
workspace has its own lockfile. The two BSP workspaces also carry their own Cargo target
configuration and memory layout.

## Platforms and firmwares

[polyfw.toml](polyfw.toml) defines two platforms. Configuration IDs and directory names use
lowercase `board-a` and `board-b`; their descriptions supply the display names:

| Platform | Board | CPU | Chip | Workspace |
| --- | --- | --- | --- | --- |
| `board-a` | board-A | `cortex-m7` | `STM32H723VGTx` | `bsp/board-a` |
| `board-b` | board-B | `cortex-m4` | `STM32F407IG` | `bsp/board-b` |

Firmware IDs match their binary names. The `led` package provides two independently selectable
binaries, both using its `src/lib.rs` library:

| Firmware | Package | Platform |
| --- | --- | --- |
| `buzzer` | `buzzer` | `board-a` |
| `screen` | `screen` | `board-b` |
| `led-1` | `led` | `board-a` |
| `led-2` | `led` | `board-a` |

The platforms keep final images under `target/firmware/board-a` and `target/firmware/board-b`.
The explicit `bin` registrations let a build of `led-1` leave `led-2` unbuilt; after building both,
their ELF files coexist in the board-A output directory.

## Shared libraries and check scopes

The root workspace contains small `no_std` libraries with reusable data types:

| Library | Responsibility | Used by |
| --- | --- | --- |
| `timing` | Millisecond intervals | `screen` directly; `buzzer` and `led` through `patterns` |
| `patterns` | Fast and slow on/off durations | `buzzer` and the `led` library |
| `layout` | Text and its position on a screen | `screen` |

The `led` library selects a shared pulse pattern for its `BlinkRate`. Its two binaries select
different rates through that common interface. The buzzer uses a pulse pattern for an alert;
the screen combines text layout with a refresh interval.

Host tests in `patterns` check that both pulse patterns have nonzero on/off durations and that
the fast pattern has a shorter cycle. The `led` library tests that selecting a fast blink shortens
both intervals. These tests run through polyfw's host test and verification commands.

`config.local-dependencies = ["shared/*"]` selects reachable shared packages for checks:

| Firmware | Shared packages checked |
| --- | --- |
| `buzzer`, `led-1`, `led-2` | `patterns`, `timing` |
| `screen` | `layout`, `timing` |

The firmware package itself is always checked. The indirect `timing` dependency is included for
the buzzer and LED programs, while the unrelated `layout` package is excluded. Shared libraries
are checked on the host through their owning root workspace. See
[dependency selection](../../docs/configuration.md#select-local-dependencies-for-checks) for the
configuration rules and [common commands](../README.md#common-commands) to run this example.
