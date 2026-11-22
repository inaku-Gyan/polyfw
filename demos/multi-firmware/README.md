# Multiple firmware example

One Cargo workspace with two firmware packages sharing the `common` library.

## Layout

```text
multi-firmware/
├── Cargo.toml
├── polyfw.toml
├── .cargo/config.toml
├── memory.x
├── common/
└── firmwares/
    ├── led-demo/
    └── buzzer-demo/
```

## Platform and firmwares

[polyfw.toml](polyfw.toml) defines the `dm-mc02` platform for DM-MC02
(`cortex-m7`, `STM32H723VGTx`). Both `led-demo` and `buzzer-demo` use this platform;
their firmware IDs match their package and binary names.
