//! Rust port of `com.madgag.gif.fmsware` (`AnimatedGifEncoder`, `GifDecoder`,
//! `LZWEncoder`, `NeuQuant`). See ../../PORTING.md for status.

pub mod decoder;
pub mod encoder;
pub mod lzw;
pub mod neuquant;
pub mod sprite_sheet;

pub use decoder::GifDecoder;
pub use encoder::AnimatedGifEncoder;
pub use neuquant::NeuQuant;
pub use sprite_sheet::VrcSpriteSheet;
