//! Rust port of `com.madgag.gif.fmsware` (`AnimatedGifEncoder`, `GifDecoder`,
//! `LZWEncoder`, `NeuQuant`). See ../../PORTING.md for status.

pub mod decoder;
pub mod lzw;
pub mod neuquant;
pub mod encoder;

pub use decoder::GifDecoder;
pub use encoder::AnimatedGifEncoder;
pub use neuquant::NeuQuant;
