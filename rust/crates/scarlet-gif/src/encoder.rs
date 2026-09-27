//! Port of `com.madgag.gif.fmsware.AnimatedGifEncoder`.
//!
//! Differences from the Java version, both because Rust has no AWT/Swing:
//!
//! - Frames are supplied as raw interleaved RGB byte buffers
//!   (`width * height * 3` bytes, row-major) instead of `BufferedImage`.
//!   Callers already holding image data via the `image` crate (or similar)
//!   can pass `.to_rgb8().into_raw()`.
//! - `getImagePixels`'s automatic resize-with-background-fill and
//!   `scaleToFit` (via `Graphics2D`) are **not ported** — there's no
//!   lightweight Rust equivalent of AWT's `Graphics2D` in this crate's
//!   dependency set, and silently approximating image resampling would be
//!   worse than being explicit about it. [`AnimatedGifEncoder::add_frame`]
//!   requires the frame to already match the encoder's configured size and
//!   returns [`GifError::FrameSizeMismatch`] otherwise; scale-to-fit is
//!   tracked as a follow-up in ../../PORTING.md.
//! - Output is any `std::io::Write`, matching the Java `OutputStream`
//!   overload of `start`; the `start(String file)` convenience is left to
//!   the caller (`std::fs::File::create(..)`), since Rust's `File` is
//!   already a `Write`.

use std::io::{self, Write};

use crate::lzw::LzwEncoder;
use crate::neuquant::NeuQuant;

#[derive(Debug, thiserror::Error)]
pub enum GifError {
    #[error("encoder not started (call start() first)")]
    NotStarted,
    #[error(
        "frame is {frame_w}x{frame_h} but the encoder is configured for {enc_w}x{enc_h}; \
         resizing frames (Java's scaleToFit) is not ported, see PORTING.md"
    )]
    FrameSizeMismatch {
        frame_w: i32,
        frame_h: i32,
        enc_w: i32,
        enc_h: i32,
    },
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub struct AnimatedGifEncoder<W: Write> {
    width: i32,
    height: i32,
    /// transparent color if given, as RGB
    transparent: Option<[u8; 3]>,
    transparent_exact_match: bool,
    trans_index: i32,
    /// -1 = no repeat
    repeat: i32,
    /// frame delay (hundredths of a second)
    delay: i32,
    started: bool,
    out: Option<W>,
    /// BGR byte buffer for the current frame
    pixels: Option<Vec<u8>>,
    /// converted frame indexed to palette
    indexed_pixels: Option<Vec<u8>>,
    /// number of bit planes
    color_depth: i32,
    /// RGB palette
    color_tab: Option<Vec<u8>>,
    /// active palette entries
    used_entry: [bool; 256],
    /// color table size (bits-1)
    pal_size: i32,
    /// disposal code (-1 = use default)
    dispose: i32,
    first_frame: bool,
    /// if false, get size from first frame
    size_set: bool,
    /// default sample interval for quantizer
    sample: i32,
}

impl<W: Write> Default for AnimatedGifEncoder<W> {
    fn default() -> Self {
        AnimatedGifEncoder {
            width: 0,
            height: 0,
            transparent: None,
            transparent_exact_match: false,
            trans_index: 0,
            repeat: -1,
            delay: 0,
            started: false,
            out: None,
            pixels: None,
            indexed_pixels: None,
            color_depth: 0,
            color_tab: None,
            used_entry: [false; 256],
            pal_size: 7,
            dispose: -1,
            first_frame: true,
            size_set: false,
            sample: 10,
        }
    }
}

impl<W: Write> AnimatedGifEncoder<W> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the delay time between each frame, or changes it for subsequent
    /// frames (applies to last frame added).
    pub fn set_delay(&mut self, ms: i32) {
        self.delay = (ms as f32 / 10.0f32).round() as i32;
    }

    /// Sets the GIF frame disposal code for the last added frame and any
    /// subsequent frames. Default is 0 if no transparent color has been
    /// set, otherwise 2.
    pub fn set_dispose(&mut self, code: i32) {
        if code >= 0 {
            self.dispose = code;
        }
    }

    /// Sets the number of times the set of GIF frames should be played.
    /// Default is 1; 0 means play indefinitely. Must be invoked before the
    /// first image is added.
    pub fn set_repeat(&mut self, iter: i32) {
        if iter >= 0 {
            self.repeat = iter;
        }
    }

    /// Sets the transparent color for the last added frame and any
    /// subsequent frames (closest-match by default; see
    /// [`set_transparent_exact`](Self::set_transparent_exact)).
    pub fn set_transparent(&mut self, c: Option<[u8; 3]>) {
        self.set_transparent_exact(c, false);
    }

    pub fn set_transparent_exact(&mut self, c: Option<[u8; 3]>, exact_match: bool) {
        self.transparent = c;
        self.transparent_exact_match = exact_match;
    }

    /// Adds next GIF frame, given as an interleaved RGB byte buffer
    /// (`width * height * 3` bytes). The frame is not written immediately,
    /// but is actually deferred until the next frame is received so that
    /// timing data can be inserted. Invoking [`finish`](Self::finish)
    /// flushes all frames. If [`set_size`](Self::set_size) was not invoked,
    /// the size of the first frame is used for all subsequent frames.
    pub fn add_frame(&mut self, width: i32, height: i32, rgb: &[u8]) -> Result<bool, GifError> {
        if !self.started {
            return Ok(false);
        }
        if !self.size_set {
            self.set_size(width, height);
        }
        if width != self.width || height != self.height {
            return Err(GifError::FrameSizeMismatch {
                frame_w: width,
                frame_h: height,
                enc_w: self.width,
                enc_h: self.height,
            });
        }

        self.get_image_pixels(rgb);
        self.analyze_pixels(); // build color table & map pixels
        if self.first_frame {
            self.write_lsd()?; // logical screen descriptor
            self.write_palette()?; // global color table
            if self.repeat >= 0 {
                self.write_netscape_ext()?; // NS app extension to indicate reps
            }
        }
        self.write_graphic_ctrl_ext()?;
        self.write_image_desc()?;
        if !self.first_frame {
            self.write_palette()?; // local color table
        }
        self.write_pixels()?; // encode and write pixel data
        self.first_frame = false;

        Ok(true)
    }

    /// Flushes any pending data. If writing to something other than a file
    /// this opened itself, the stream is not closed — same as the Java
    /// `OutputStream` overload of `start`; drop the returned encoder (or its
    /// writer) to close it.
    pub fn finish(&mut self) -> io::Result<bool> {
        if !self.started {
            return Ok(false);
        }
        self.started = false;
        let result = (|| -> io::Result<()> {
            let out = self.out.as_mut().expect("started implies out is set");
            out.write_all(&[0x3b])?; // gif trailer
            out.flush()
        })();

        // reset for subsequent use, mirroring the Java `finish()`
        self.trans_index = 0;
        self.out = None;
        self.pixels = None;
        self.indexed_pixels = None;
        self.color_tab = None;
        self.first_frame = true;

        Ok(result.is_ok())
    }

    /// Sets frame rate in frames per second. Equivalent to
    /// `set_delay(1000/fps)`.
    pub fn set_frame_rate(&mut self, fps: f32) {
        if fps != 0.0 {
            self.delay = (100.0 / fps).round() as i32;
        }
    }

    /// Sets quality of color quantization (conversion of images to the
    /// maximum 256 colors allowed by the GIF specification). Lower values
    /// (minimum = 1) produce better colors, but slow processing
    /// significantly. 10 is the default.
    pub fn set_quality(&mut self, quality: i32) {
        self.sample = quality.max(1);
    }

    /// Sets the GIF frame size. The default size is the size of the first
    /// frame added if this method is not invoked.
    pub fn set_size(&mut self, w: i32, h: i32) {
        if self.started && !self.first_frame {
            return;
        }
        self.width = if w < 1 { 320 } else { w };
        self.height = if h < 1 { 240 } else { h };
        self.size_set = true;
    }

    /// Initiates GIF file creation on the given writer. The writer is not
    /// closed automatically.
    pub fn start(&mut self, mut os: W) -> io::Result<bool> {
        let ok = os.write_all(b"GIF89a").is_ok(); // header
        self.out = Some(os);
        self.started = ok;
        Ok(ok)
    }

    pub fn is_started(&self) -> bool {
        self.started
    }

    /// Analyzes image colors and creates color map.
    fn analyze_pixels(&mut self) {
        let pixels = self.pixels.take().expect("get_image_pixels sets pixels");
        let len = pixels.len() as i32;
        let n_pix = pixels.len() / 3;
        let mut indexed_pixels = vec![0u8; n_pix];
        let mut nq = NeuQuant::new(&pixels, len, self.sample);
        let mut color_tab = nq.process(); // create reduced palette (BGR)
        // convert map from BGR to RGB
        let mut i = 0;
        while i < color_tab.len() {
            color_tab.swap(i, i + 2);
            self.used_entry[i / 3] = false;
            i += 3;
        }
        // map image pixels to new palette
        let mut k = 0;
        for indexed in indexed_pixels.iter_mut() {
            let b = pixels[k] as i32 & 0xff;
            let g = pixels[k + 1] as i32 & 0xff;
            let r = pixels[k + 2] as i32 & 0xff;
            k += 3;
            let index = nq.map(b, g, r);
            self.used_entry[index as usize] = true;
            *indexed = index as u8;
        }
        self.indexed_pixels = Some(indexed_pixels);
        self.color_depth = 8;
        self.pal_size = 7;
        self.color_tab = Some(color_tab);
        // get closest match to transparent color if specified
        if let Some(transparent) = self.transparent {
            self.trans_index = if self.transparent_exact_match {
                self.find_exact(transparent)
            } else {
                self.find_closest(transparent)
            };
        }
    }

    /// Returns index of palette color closest to `c` (RGB).
    fn find_closest(&self, c: [u8; 3]) -> i32 {
        let Some(color_tab) = &self.color_tab else {
            return -1;
        };
        let (r, g, b) = (c[0] as i32, c[1] as i32, c[2] as i32);
        let mut minpos = 0i32;
        let mut dmin = 256 * 256 * 256;
        let len = color_tab.len();
        let mut i = 0;
        while i < len {
            let dr = r - (color_tab[i] as i32 & 0xff);
            let dg = g - (color_tab[i + 1] as i32 & 0xff);
            let db = b - (color_tab[i + 2] as i32 & 0xff);
            let d = dr * dr + dg * dg + db * db;
            let index = i / 3;
            if self.used_entry[index] && d < dmin {
                dmin = d;
                minpos = index as i32;
            }
            i += 3;
        }
        minpos
    }

    /// Returns index of palette color exactly matching `c` (RGB), or -1 if
    /// there is no exact match. Must be called before [`finish`](Self::finish),
    /// since after that the palette is gone.
    fn find_exact(&self, c: [u8; 3]) -> i32 {
        let Some(color_tab) = &self.color_tab else {
            return -1;
        };
        let (r, g, b) = (c[0] as i32, c[1] as i32, c[2] as i32);
        let len = color_tab.len() / 3;
        for index in 0..len {
            let i = index * 3;
            if self.used_entry[index]
                && r == color_tab[i] as i32 & 0xff
                && g == color_tab[i + 1] as i32 & 0xff
                && b == color_tab[i + 2] as i32 & 0xff
            {
                return index as i32;
            }
        }
        -1
    }

    /// Returns true if the exact matching color exists and is used in the
    /// color palette; must be called before `finish()`.
    pub fn is_color_used(&self, c: [u8; 3]) -> bool {
        self.find_exact(c) != -1
    }

    /// Converts an RGB frame buffer into the BGR `pixels` buffer the
    /// quantizer expects. See the module docs for why resizing
    /// (`scaleToFit`/background compositing) isn't ported here.
    fn get_image_pixels(&mut self, rgb: &[u8]) {
        let mut bgr = Vec::with_capacity(rgb.len());
        for chunk in rgb.chunks_exact(3) {
            bgr.push(chunk[2]);
            bgr.push(chunk[1]);
            bgr.push(chunk[0]);
        }
        self.pixels = Some(bgr);
    }

    /// Writes Graphic Control Extension.
    fn write_graphic_ctrl_ext(&mut self) -> io::Result<()> {
        let out = self.out.as_mut().expect("started");
        out.write_all(&[0x21])?; // extension introducer
        out.write_all(&[0xf9])?; // GCE label
        out.write_all(&[4])?; // data block size
        let (transp, mut disp) = if self.transparent.is_none() {
            (0, 0) // dispose = no action
        } else {
            (1, 2) // force clear if using transparent color
        };
        if self.dispose >= 0 {
            disp = self.dispose & 7; // user override
        }
        disp <<= 2;

        out.write_all(&[(disp | transp) as u8])?; // packed fields
        write_short(out, self.delay)?; // delay x 1/100 sec
        out.write_all(&[self.trans_index as u8])?; // transparent color index
        out.write_all(&[0]) // block terminator
    }

    /// Writes Image Descriptor.
    fn write_image_desc(&mut self) -> io::Result<()> {
        let first_frame = self.first_frame;
        let (width, height, pal_size) = (self.width, self.height, self.pal_size);
        let out = self.out.as_mut().expect("started");
        out.write_all(&[0x2c])?; // image separator
        write_short(out, 0)?; // image position x,y = 0,0
        write_short(out, 0)?;
        write_short(out, width)?; // image size
        write_short(out, height)?;
        if first_frame {
            // no LCT - GCT is used for first (or only) frame
            out.write_all(&[0])
        } else {
            // specify normal LCT
            out.write_all(&[(0x80 | pal_size) as u8])
        }
    }

    /// Writes Logical Screen Descriptor.
    fn write_lsd(&mut self) -> io::Result<()> {
        let (width, height, pal_size) = (self.width, self.height, self.pal_size);
        let out = self.out.as_mut().expect("started");
        write_short(out, width)?; // logical screen size
        write_short(out, height)?;
        out.write_all(&[(0x80 | 0x70 | pal_size) as u8])?; // gct flag=1, color res=7, gct size
        out.write_all(&[0])?; // background color index
        out.write_all(&[0]) // pixel aspect ratio - assume 1:1
    }

    /// Writes Netscape application extension to define repeat count.
    fn write_netscape_ext(&mut self) -> io::Result<()> {
        let repeat = self.repeat;
        let out = self.out.as_mut().expect("started");
        out.write_all(&[0x21])?; // extension introducer
        out.write_all(&[0xff])?; // app extension label
        out.write_all(&[11])?; // block size
        out.write_all(b"NETSCAPE2.0")?; // app id + auth code
        out.write_all(&[3])?; // sub-block size
        out.write_all(&[1])?; // loop sub-block id
        write_short(out, repeat)?; // loop count (extra iterations, 0=repeat forever)
        out.write_all(&[0]) // block terminator
    }

    /// Writes color table.
    fn write_palette(&mut self) -> io::Result<()> {
        let color_tab = self.color_tab.clone().unwrap_or_default();
        let out = self.out.as_mut().expect("started");
        out.write_all(&color_tab)?;
        let n = (3 * 256) - color_tab.len();
        out.write_all(&vec![0u8; n])
    }

    /// Encodes and writes pixel data.
    fn write_pixels(&mut self) -> io::Result<()> {
        let indexed_pixels = self.indexed_pixels.take().expect("analyze_pixels sets this");
        let (width, height, color_depth) = (self.width, self.height, self.color_depth);
        let out = self.out.as_mut().expect("started");
        let mut encoder = LzwEncoder::new(width, height, &indexed_pixels, color_depth);
        encoder.encode(out)
    }
}

/// Writes a 16-bit value LSB first, matching `AnimatedGifEncoder.writeShort`.
fn write_short<W: Write>(out: &mut W, value: i32) -> io::Result<()> {
    out.write_all(&[(value & 0xff) as u8, ((value >> 8) & 0xff) as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_well_formed_single_frame_gif() {
        let mut buf: Vec<u8> = Vec::new();
        let mut enc = AnimatedGifEncoder::new();
        assert!(enc.start(&mut buf).unwrap());
        enc.set_repeat(0);
        enc.set_delay(100);

        let width = 8;
        let height = 8;
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                rgb.extend_from_slice(&[(x * 30) as u8, (y * 30) as u8, 128]);
            }
        }
        assert!(enc.add_frame(width, height, &rgb).unwrap());
        assert!(enc.finish().unwrap());

        assert_eq!(&buf[0..6], b"GIF89a");
        assert_eq!(*buf.last().unwrap(), 0x3b); // gif trailer
    }

    #[test]
    fn rejects_frames_of_the_wrong_size() {
        let mut buf: Vec<u8> = Vec::new();
        let mut enc = AnimatedGifEncoder::new();
        enc.start(&mut buf).unwrap();
        enc.set_size(8, 8);
        let wrong = vec![0u8; 4 * 4 * 3];
        let err = enc.add_frame(4, 4, &wrong).unwrap_err();
        assert!(matches!(err, GifError::FrameSizeMismatch { .. }));
    }

    #[test]
    fn add_frame_without_start_is_a_noop_false() {
        let mut buf: Vec<u8> = Vec::new();
        let mut enc: AnimatedGifEncoder<&mut Vec<u8>> = AnimatedGifEncoder::new();
        let rgb = vec![0u8; 4 * 4 * 3];
        // Not started yet: matches Java's `!started -> return false`.
        assert!(!enc.add_frame(4, 4, &rgb).unwrap());
        let _ = &mut buf;
    }
}
