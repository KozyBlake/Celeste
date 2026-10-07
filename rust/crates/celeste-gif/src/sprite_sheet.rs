//! VRChat animated-emoji 1024×1024 spritesheet generation.

use std::io::Cursor;

use image::imageops::FilterType;
use image::{ImageFormat, Rgba, RgbaImage};

use crate::decoder::{GifDecoder, STATUS_OK};

pub const MAX_GIF_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_GIF_FRAMES: usize = 256;
pub const MAX_GIF_DIMENSION: u32 = 2048;
pub const MAX_GIF_PIXELS: u64 = 2048 * 2048;

#[derive(Debug, thiserror::Error)]
pub enum SpriteSheetError {
    #[error("GIF exceeds maximum allowed size")]
    TooLarge,
    #[error("GIF decode failed with status {0}")]
    Decode(i32),
    #[error("GIF contains no frames")]
    NoFrames,
    #[error("GIF has too many frames")]
    TooManyFrames,
    #[error("GIF has invalid dimensions")]
    InvalidDimensions,
    #[error("GIF dimensions exceed maximum allowed size")]
    DimensionsTooLarge,
    #[error("PNG encoding failed: {0}")]
    Png(#[from] image::ImageError),
}

pub struct VrcSpriteSheet {
    pub name: String,
    pub png: Vec<u8>,
    pub original_frames: usize,
    pub rendered_frames: usize,
    pub fps: f32,
    pub stride: u32,
    pub rendered_frame_bounds: (u32, u32, u32, u32),
}

impl VrcSpriteSheet {
    pub fn from_gif(name: &str, bytes: &[u8]) -> Result<Self, SpriteSheetError> {
        if bytes.len() > MAX_GIF_BYTES {
            return Err(SpriteSheetError::TooLarge);
        }
        let mut decoder = GifDecoder::new(Cursor::new(bytes));
        let status = decoder.read_gif();
        if status != STATUS_OK {
            return Err(SpriteSheetError::Decode(status));
        }
        let count = usize::try_from(decoder.frame_count()).unwrap_or(0);
        let (width, height) = decoder.frame_size();
        if count == 0 {
            return Err(SpriteSheetError::NoFrames);
        }
        if count > MAX_GIF_FRAMES {
            return Err(SpriteSheetError::TooManyFrames);
        }
        if width <= 0 || height <= 0 {
            return Err(SpriteSheetError::InvalidDimensions);
        }
        let (width, height) = (width as u32, height as u32);
        if width > MAX_GIF_DIMENSION
            || height > MAX_GIF_DIMENSION
            || u64::from(width) * u64::from(height) > MAX_GIF_PIXELS
        {
            return Err(SpriteSheetError::DimensionsTooLarge);
        }

        let scrunch = count > 64;
        let cells = if count > 16 {
            8
        } else if count > 4 {
            4
        } else if count > 1 {
            2
        } else {
            1
        };
        let stride = 1024 / cells;
        let (render_width, render_height) = if width > height {
            (
                stride,
                ((height as f32 / width as f32) * stride as f32).round() as u32,
            )
        } else if width < height {
            (
                ((width as f32 / height as f32) * stride as f32).round() as u32,
                stride,
            )
        } else {
            (stride, stride)
        };
        let offset_x = (stride - render_width) / 2;
        let offset_y = (stride - render_height) / 2;
        let rendered_frames = if scrunch { 64 } else { count };
        let total_time: i32 = decoder.frames().iter().map(|frame| frame.delay).sum();
        let fps = 1000.0 * rendered_frames as f32 / total_time as f32;
        let mut sheet = RgbaImage::new(1024, 1024);

        for output_index in 0..rendered_frames {
            let input_index = if scrunch {
                ((output_index as f32 / 63.0) * (count - 1) as f32).round() as usize
            } else {
                output_index
            };
            let frame = &decoder.frames()[input_index];
            let source = RgbaImage::from_fn(width, height, |x, y| {
                let argb = frame.image[(y * width + x) as usize];
                Rgba([
                    ((argb >> 16) & 0xff) as u8,
                    ((argb >> 8) & 0xff) as u8,
                    (argb & 0xff) as u8,
                    ((argb >> 24) & 0xff) as u8,
                ])
            });
            let resized =
                image::imageops::resize(&source, render_width, render_height, FilterType::Triangle);
            let output_x = (output_index as u32 % cells) * stride + offset_x;
            let output_y = (output_index as u32 / cells) * stride + offset_y;
            image::imageops::overlay(&mut sheet, &resized, output_x.into(), output_y.into());
        }

        let mut png = Cursor::new(Vec::new());
        sheet.write_to(&mut png, ImageFormat::Png)?;
        let file_name = name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("animated-emoji")
            .trim();
        let base_name = if file_name.to_ascii_lowercase().ends_with(".gif") {
            file_name[..file_name.len() - 4].to_string()
        } else {
            file_name.to_string()
        };
        Ok(Self {
            name: if base_name.is_empty() {
                "animated-emoji".into()
            } else {
                base_name
            },
            png: png.into_inner(),
            original_frames: count,
            rendered_frames,
            fps,
            stride,
            rendered_frame_bounds: (offset_x, offset_y, render_width, render_height),
        })
    }

    pub fn info(&self) -> String {
        format!(
            "{} ({}/{} frames, {:.1} fps)",
            self.name, self.rendered_frames, self.original_frames, self.fps
        )
    }

    pub fn file_name(&self) -> String {
        format!("{}.png", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AnimatedGifEncoder;

    #[test]
    fn creates_a_png_sprite_sheet_from_a_gif() {
        let mut gif = Vec::new();
        {
            let mut encoder = AnimatedGifEncoder::new();
            encoder.set_size(2, 1);
            encoder.set_delay(100);
            encoder.start(&mut gif).unwrap();
            encoder.add_frame(2, 1, &[255, 0, 0, 0, 255, 0]).unwrap();
            encoder
                .add_frame(2, 1, &[0, 0, 255, 255, 255, 255])
                .unwrap();
            encoder.finish().unwrap();
        }
        let sheet = VrcSpriteSheet::from_gif("test.gif", &gif).unwrap();
        assert_eq!(sheet.original_frames, 2);
        assert_eq!(sheet.rendered_frames, 2);
        assert_eq!(sheet.file_name(), "test.png");
        assert!(sheet.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(image::load_from_memory(&sheet.png).unwrap().width(), 1024);
    }
}
