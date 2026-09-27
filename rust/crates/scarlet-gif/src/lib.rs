//! Rust port of `com.madgag.gif.fmsware` (`AnimatedGifEncoder`, `GifDecoder`,
//! `LZWEncoder`, `NeuQuant`). See ../../PORTING.md for status.

pub mod lzw;
pub mod neuquant;
pub mod encoder;

pub use encoder::AnimatedGifEncoder;
pub use neuquant::NeuQuant;
