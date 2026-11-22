#![no_std]

/// A time interval expressed in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Millis(pub u32);
