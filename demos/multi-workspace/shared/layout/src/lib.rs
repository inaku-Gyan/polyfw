#![no_std]

/// A line of text positioned in screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextLine {
    pub text: &'static str,
    pub x: u16,
    pub y: u16,
}
