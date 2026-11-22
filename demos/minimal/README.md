# Minimal example

A single Cargo package containing a library and one firmware binary.

## Layout

```text
minimal/
├── Cargo.toml
├── polyfw.toml
├── .cargo/config.toml
├── memory.x
├── build.rs
└── src/
    ├── lib.rs
    └── main.rs
```

## Platform and firmware

[polyfw.toml](polyfw.toml) defines the `dm-mc02` platform for DM-MC02
(`cortex-m7`, `STM32H723VGTx`). The sole firmware, `hello`, uses the `hello` package and binary.
