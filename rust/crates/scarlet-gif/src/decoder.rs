//! Port of `com.madgag.gif.fmsware.GifDecoder` (LZW decoder adapted from
//! John Cristy's ImageMagick, ported to Java by Kevin Weiner).
//!
//! Ported line-for-line like `neuquant`/`lzw`, for the same reason: this is
//! a bit-level LZW decoder where a "cleaner" rewrite is exactly how a
//! subtle decode bug would sneak in. A `clippy::needless_range_loop` lint is
//! silenced for the same reason as in `neuquant`.
#![allow(clippy::needless_range_loop)]
//!
//! ## Representing frames without `BufferedImage`
//!
//! Java decodes into `TYPE_INT_ARGB_PRE` (premultiplied ARGB) pixel
//! buffers. This port uses a plain packed-ARGB `Vec<u32>`
//! (`0xAARRGGBB`) with no premultiplication step, which is exactly
//! equivalent here: GIF transparency is binary (a pixel is either the
//! transparent index, given color `0` — zero alpha, zero RGB — or fully
//! opaque with `0xff` alpha), and premultiplying is a no-op at both those
//! extremes.
//!
//! Java's `act` (active color table) is a *reference* to either `gct` or
//! `lct`, temporarily mutated in place to zero out the transparent index
//! and restored before `readImage` returns. This port instead clones the
//! relevant table into a local, owned buffer at the start of decoding a
//! frame and mutates that — since nothing observes the table between the
//! mutate and the (in Java) restore, an owned local copy is behaviorally
//! identical and avoids needing to alias two struct fields.

use std::io::Read;

pub const STATUS_OK: i32 = 0;
pub const STATUS_FORMAT_ERROR: i32 = 1;
pub const STATUS_OPEN_ERROR: i32 = 2;

const MAX_STACK_SIZE: usize = 4096;

pub struct GifFrame {
    /// Packed ARGB (`0xAARRGGBB`), `width * height` pixels, row-major.
    pub image: Vec<u32>,
    /// Display duration in milliseconds.
    pub delay: i32,
}

pub struct GifDecoder<R: Read> {
    reader: R,
    status: i32,

    width: i32,
    height: i32,
    gct_flag: bool,
    gct_size: i32,
    loop_count: i32,

    gct: Option<Vec<u32>>,
    lct: Option<Vec<u32>>,

    bg_index: i32,
    bg_color: u32,
    last_bg_color: u32,
    #[allow(dead_code)]
    pixel_aspect: i32,

    lct_flag: bool,
    interlace: bool,
    lct_size: i32,

    ix: i32,
    iy: i32,
    iw: i32,
    ih: i32,
    last_rect: Option<(i32, i32, i32, i32)>,
    image: Vec<u32>,
    last_image: Option<Vec<u32>>,

    block: [u8; 256],
    block_size: i32,

    dispose: i32,
    last_dispose: i32,
    transparency: bool,
    delay: i32,
    trans_index: i32,

    prefix: Vec<i16>,
    suffix: Vec<u8>,
    pixel_stack: Vec<u8>,
    pixels: Vec<u8>,

    frames: Vec<GifFrame>,
    frame_count: i32,
}

impl<R: Read> GifDecoder<R> {
    pub fn new(reader: R) -> Self {
        GifDecoder {
            reader,
            status: STATUS_OK,
            width: 0,
            height: 0,
            gct_flag: false,
            gct_size: 0,
            loop_count: 1,
            gct: None,
            lct: None,
            bg_index: 0,
            bg_color: 0,
            last_bg_color: 0,
            pixel_aspect: 0,
            lct_flag: false,
            interlace: false,
            lct_size: 0,
            ix: 0,
            iy: 0,
            iw: 0,
            ih: 0,
            last_rect: None,
            image: Vec::new(),
            last_image: None,
            block: [0u8; 256],
            block_size: 0,
            dispose: 0,
            last_dispose: 0,
            transparency: false,
            delay: 0,
            trans_index: 0,
            prefix: Vec::new(),
            suffix: Vec::new(),
            pixel_stack: Vec::new(),
            pixels: Vec::new(),
            frames: Vec::new(),
            frame_count: 0,
        }
    }

    pub fn frame_count(&self) -> i32 {
        self.frame_count
    }
    pub fn loop_count(&self) -> i32 {
        self.loop_count
    }
    pub fn frame_size(&self) -> (i32, i32) {
        (self.width, self.height)
    }
    pub fn delay(&self, n: usize) -> i32 {
        self.frames.get(n).map(|f| f.delay).unwrap_or(-1)
    }
    pub fn frame(&self, n: usize) -> Option<&GifFrame> {
        self.frames.get(n)
    }
    pub fn frames(&self) -> &[GifFrame] {
        &self.frames
    }

    /// Port of `read(InputStream)`.
    pub fn read_gif(&mut self) -> i32 {
        self.read_header();
        if !self.err() {
            self.read_contents();
            if self.frame_count < 0 {
                self.status = STATUS_FORMAT_ERROR;
            }
        }
        self.status
    }

    fn err(&self) -> bool {
        self.status != STATUS_OK
    }

    fn read_byte(&mut self) -> i32 {
        let mut buf = [0u8; 1];
        match self.reader.read(&mut buf) {
            Ok(1) => buf[0] as i32,
            Ok(_) => -1, // EOF, matching InputStream.read() returning -1
            Err(_) => {
                self.status = STATUS_FORMAT_ERROR;
                0
            }
        }
    }

    fn read_short(&mut self) -> i32 {
        let a = self.read_byte();
        let b = self.read_byte();
        a | (b << 8)
    }

    fn read_block(&mut self) -> i32 {
        self.block_size = self.read_byte();
        let mut n = 0i32;
        if self.block_size > 0 {
            let mut buf = vec![0u8; self.block_size as usize];
            match self.reader.read_exact(&mut buf) {
                Ok(()) => {
                    self.block[..self.block_size as usize].copy_from_slice(&buf);
                    n = self.block_size;
                }
                Err(_) => {
                    // Best-effort partial read, matching the Java loop that
                    // tolerates a short read and flags STATUS_FORMAT_ERROR
                    // only if fewer than blockSize bytes came back.
                    n = 0;
                }
            }
            if n < self.block_size {
                self.status = STATUS_FORMAT_ERROR;
            }
        }
        n
    }

    fn read_color_table(&mut self, ncolors: i32) -> Option<Vec<u32>> {
        let nbytes = 3 * ncolors as usize;
        let mut c = vec![0u8; nbytes];
        let n = self.reader.read(&mut c).unwrap_or(0);
        if n < nbytes {
            self.status = STATUS_FORMAT_ERROR;
            return None;
        }
        let mut tab = vec![0u32; 256]; // max size, matching Java's fixed-256 table
        let mut j = 0;
        for i in 0..ncolors as usize {
            let r = c[j] as u32;
            let g = c[j + 1] as u32;
            let b = c[j + 2] as u32;
            j += 3;
            tab[i] = 0xff000000 | (r << 16) | (g << 8) | b;
        }
        Some(tab)
    }

    fn read_header(&mut self) {
        let mut id = String::new();
        for _ in 0..6 {
            id.push(self.read_byte() as u8 as char);
        }
        if !id.starts_with("GIF") {
            self.status = STATUS_FORMAT_ERROR;
            return;
        }
        self.read_lsd();
        if self.gct_flag && !self.err() {
            self.gct = self.read_color_table(self.gct_size);
            if let Some(gct) = &self.gct {
                self.bg_color = gct[self.bg_index as usize];
            }
        }
    }

    fn read_lsd(&mut self) {
        self.width = self.read_short();
        self.height = self.read_short();
        let packed = self.read_byte();
        self.gct_flag = (packed & 0x80) != 0;
        self.gct_size = 2 << (packed & 7);
        self.bg_index = self.read_byte();
        self.pixel_aspect = self.read_byte();
    }

    fn read_contents(&mut self) {
        let mut done = false;
        while !(done || self.err()) {
            let code = self.read_byte();
            match code {
                0x2C => self.read_image(),
                0x21 => {
                    let code = self.read_byte();
                    match code {
                        0xf9 => self.read_graphic_control_ext(),
                        0xff => {
                            self.read_block();
                            let app: String =
                                self.block[..11.min(self.block.len())].iter().map(|&b| b as char).collect();
                            if app == "NETSCAPE2.0" {
                                self.read_netscape_ext();
                            } else {
                                self.skip();
                            }
                        }
                        _ => self.skip(),
                    }
                }
                0x3b => done = true,
                0x00 => {}
                _ => self.status = STATUS_FORMAT_ERROR,
            }
        }
    }

    fn read_graphic_control_ext(&mut self) {
        self.read_byte(); // block size
        let packed = self.read_byte();
        self.dispose = (packed & 0x1c) >> 2;
        if self.dispose == 0 {
            self.dispose = 1;
        }
        self.transparency = (packed & 1) != 0;
        self.delay = self.read_short() * 10;
        self.trans_index = self.read_byte();
        self.read_byte(); // block terminator
    }

    fn read_netscape_ext(&mut self) {
        loop {
            self.read_block();
            if self.block[0] == 1 {
                let b1 = self.block[1] as i32;
                let b2 = self.block[2] as i32;
                self.loop_count = (b2 << 8) | b1;
            }
            if self.block_size <= 0 || self.err() {
                break;
            }
        }
    }

    fn skip(&mut self) {
        loop {
            self.read_block();
            if self.block_size <= 0 || self.err() {
                break;
            }
        }
    }

    fn read_image(&mut self) {
        self.ix = self.read_short();
        self.iy = self.read_short();
        self.iw = self.read_short();
        self.ih = self.read_short();

        let packed = self.read_byte();
        self.lct_flag = (packed & 0x80) != 0;
        self.interlace = (packed & 0x40) != 0;
        self.lct_size = 2 << (packed & 7);

        let mut act = if self.lct_flag {
            self.lct = self.read_color_table(self.lct_size);
            self.lct.clone()
        } else {
            if self.bg_index == self.trans_index {
                self.bg_color = 0;
            }
            self.gct.clone()
        };

        if let Some(act) = &mut act {
            if self.transparency {
                act[self.trans_index as usize] = 0;
            }
        }

        if act.is_none() {
            self.status = STATUS_FORMAT_ERROR;
        }
        if self.err() {
            return;
        }
        let act = act.unwrap();

        self.decode_image_data();
        self.skip();
        if self.err() {
            return;
        }

        self.frame_count += 1;
        self.image = vec![0u32; (self.width * self.height) as usize];
        self.set_pixels(&act);
        self.frames.push(GifFrame { image: self.image.clone(), delay: self.delay });

        self.reset_frame();
    }

    /// Port of `decodeImageData` (see module docs — this is the
    /// ImageMagick-derived LZW decoder, ported exactly).
    fn decode_image_data(&mut self) {
        const NULL_CODE: i32 = -1;
        let npix = (self.iw * self.ih) as usize;

        if self.pixels.len() < npix {
            self.pixels = vec![0u8; npix];
        }
        if self.prefix.is_empty() {
            self.prefix = vec![0i16; MAX_STACK_SIZE];
        }
        if self.suffix.is_empty() {
            self.suffix = vec![0u8; MAX_STACK_SIZE];
        }
        if self.pixel_stack.is_empty() {
            self.pixel_stack = vec![0u8; MAX_STACK_SIZE + 1];
        }

        let data_size = self.read_byte();
        let clear = 1i32 << data_size;
        let end_of_information = clear + 1;
        let mut available = clear + 2;
        let mut old_code = NULL_CODE;
        let mut code_size = data_size + 1;
        let mut code_mask = (1i32 << code_size) - 1;
        for code in 0..clear {
            self.prefix[code as usize] = 0;
            self.suffix[code as usize] = code as u8;
        }

        let (mut datum, mut bits, mut count, mut first, mut top, mut pi, mut bi) = (0i32, 0i32, 0i32, 0i32, 0i32, 0usize, 0usize);

        let mut i = 0usize;
        while i < npix {
            if top == 0 {
                if bits < code_size {
                    if count == 0 {
                        count = self.read_block();
                        if count <= 0 {
                            break;
                        }
                        bi = 0;
                    }
                    datum += ((self.block[bi] as i32) & 0xff) << bits;
                    bits += 8;
                    bi += 1;
                    count -= 1;
                    continue;
                }

                let mut code = datum & code_mask;
                datum >>= code_size;
                bits -= code_size;

                if code > available || code == end_of_information {
                    break;
                }
                if code == clear {
                    code_size = data_size + 1;
                    code_mask = (1 << code_size) - 1;
                    available = clear + 2;
                    old_code = NULL_CODE;
                    continue;
                }
                if old_code == NULL_CODE {
                    self.pixel_stack[top as usize] = self.suffix[code as usize];
                    top += 1;
                    old_code = code;
                    first = code;
                    continue;
                }
                let in_code = code;
                if code == available {
                    self.pixel_stack[top as usize] = first as u8;
                    top += 1;
                    code = old_code;
                }
                while code > clear {
                    self.pixel_stack[top as usize] = self.suffix[code as usize];
                    top += 1;
                    code = self.prefix[code as usize] as i32;
                }
                first = (self.suffix[code as usize] as i32) & 0xff;

                if available >= MAX_STACK_SIZE as i32 {
                    self.pixel_stack[top as usize] = first as u8;
                    top += 1;
                    continue;
                }
                self.pixel_stack[top as usize] = first as u8;
                top += 1;
                self.prefix[available as usize] = old_code as i16;
                self.suffix[available as usize] = first as u8;
                available += 1;
                if (available & code_mask) == 0 && available < MAX_STACK_SIZE as i32 {
                    code_size += 1;
                    code_mask += available;
                }
                old_code = in_code;
            }

            top -= 1;
            self.pixels[pi] = self.pixel_stack[top as usize];
            pi += 1;
            i += 1;
        }

        for p in self.pixels.iter_mut().take(npix).skip(pi) {
            *p = 0;
        }
    }

    /// Port of `setPixels`.
    fn set_pixels(&mut self, act: &[u32]) {
        let (width, height) = (self.width, self.height);
        let dest = &mut self.image;

        if self.last_dispose > 0 {
            if self.last_dispose == 3 {
                let n = self.frame_count - 2;
                self.last_image = if n > 0 { self.frames.get((n - 1) as usize).map(|f| f.image.clone()) } else { None };
            }
            if let Some(prev) = &self.last_image {
                let len = (width * height) as usize;
                dest[..len].copy_from_slice(&prev[..len]);

                if self.last_dispose == 2 {
                    let fill = if self.transparency { 0u32 } else { self.last_bg_color };
                    if let Some((rx, ry, rw, rh)) = self.last_rect {
                        for y in ry.max(0)..(ry + rh).min(height) {
                            for x in rx.max(0)..(rx + rw).min(width) {
                                dest[(y * width + x) as usize] = fill;
                            }
                        }
                    }
                }
            }
        }

        let (ix, iy, iw, ih) = (self.ix, self.iy, self.iw, self.ih);
        let mut pass = 1;
        let mut inc = 8;
        let mut iline = 0;
        for i in 0..ih {
            let mut line = i;
            if self.interlace {
                if iline >= ih {
                    pass += 1;
                    match pass {
                        2 => iline = 4,
                        3 => {
                            iline = 2;
                            inc = 4;
                        }
                        4 => {
                            iline = 1;
                            inc = 2;
                        }
                        _ => {}
                    }
                }
                line = iline;
                iline += inc;
            }
            line += iy;
            if line < height {
                let k = line * width;
                let dx0 = k + ix;
                let mut dlim = dx0 + iw;
                if k + width < dlim {
                    dlim = k + width;
                }
                let mut dx = dx0;
                let mut sx = (i * iw) as usize;
                while dx < dlim {
                    let index = self.pixels[sx] as usize & 0xff;
                    sx += 1;
                    let c = act[index];
                    if c != 0 {
                        dest[dx as usize] = c;
                    }
                    dx += 1;
                }
            }
        }
    }

    fn reset_frame(&mut self) {
        self.last_dispose = self.dispose;
        self.last_rect = Some((self.ix, self.iy, self.iw, self.ih));
        self.last_image = Some(self.image.clone());
        self.last_bg_color = self.bg_color;
        self.lct = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::AnimatedGifEncoder;

    fn encode_test_gif(width: i32, height: i32, frames: &[[u8; 3]]) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();
        let mut enc = AnimatedGifEncoder::new();
        enc.start(&mut buf).unwrap();
        enc.set_repeat(0);
        enc.set_delay(100);
        for &color in frames {
            let mut rgb = Vec::with_capacity((width * height * 3) as usize);
            for _ in 0..(width * height) {
                rgb.extend_from_slice(&color);
            }
            enc.add_frame(width, height, &rgb).unwrap();
        }
        enc.finish().unwrap();
        buf
    }

    #[test]
    fn decodes_a_single_frame_gif_round_trip() {
        let gif_bytes = encode_test_gif(4, 4, &[[10, 20, 30]]);
        let mut decoder = GifDecoder::new(std::io::Cursor::new(gif_bytes));
        let status = decoder.read_gif();
        assert_eq!(status, STATUS_OK);
        assert_eq!(decoder.frame_count(), 1);
        assert_eq!(decoder.frame_size(), (4, 4));

        let frame = decoder.frame(0).unwrap();
        assert_eq!(frame.image.len(), 16);
        // Every pixel should be opaque and close to the encoded RGB
        // (NeuQuant-quantized, so allow a little slack).
        for &pixel in &frame.image {
            let a = (pixel >> 24) & 0xff;
            let r = (pixel >> 16) & 0xff;
            let g = (pixel >> 8) & 0xff;
            let b = pixel & 0xff;
            assert_eq!(a, 0xff);
            assert!(r.abs_diff(10) <= 20, "r={r}");
            assert!(g.abs_diff(20) <= 20, "g={g}");
            assert!(b.abs_diff(30) <= 20, "b={b}");
        }
    }

    #[test]
    fn decodes_multiple_frames_with_delay() {
        let gif_bytes = encode_test_gif(3, 3, &[[255, 0, 0], [0, 255, 0], [0, 0, 255]]);
        let mut decoder = GifDecoder::new(std::io::Cursor::new(gif_bytes));
        assert_eq!(decoder.read_gif(), STATUS_OK);
        assert_eq!(decoder.frame_count(), 3);
        for i in 0..3 {
            assert_eq!(decoder.delay(i), 100);
        }
    }

    #[test]
    fn rejects_non_gif_input() {
        let mut decoder = GifDecoder::new(std::io::Cursor::new(b"not a gif".to_vec()));
        assert_eq!(decoder.read_gif(), STATUS_FORMAT_ERROR);
    }

    #[test]
    fn loop_count_defaults_to_the_encoded_repeat_value() {
        let gif_bytes = encode_test_gif(2, 2, &[[1, 2, 3]]);
        let mut decoder = GifDecoder::new(std::io::Cursor::new(gif_bytes));
        decoder.read_gif();
        assert_eq!(decoder.loop_count(), 0); // set_repeat(0) => loop forever
    }

    /// Golden-byte regression test cross-checked against the original Java
    /// `com.madgag.gif.fmsware.GifDecoder` (OpenJDK 21): this exact 1,639-byte
    /// GIF (produced by this crate's own `AnimatedGifEncoder`, base64'd below
    /// only to keep it out of a 1,600-entry literal array) was decoded with
    /// the real Java class and dumped frame-by-frame as raw ARGB bytes via
    /// `DataBufferInt` -- both frames below are byte-for-byte identical to
    /// that dump, not just "close" (this test doesn't use the same
    /// AnimatedGifEncoder-round-trip trick as the other decoder tests, so it
    /// exercises the decoder against fixed, externally-verified bytes rather
    /// than whatever this crate's own encoder happens to currently produce).
    #[test]
    fn matches_java_reference_output_byte_for_byte() {
        use base64::Engine;
        let gif_b64 = "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";
        let gif_bytes = base64::engine::general_purpose::STANDARD.decode(gif_b64).unwrap();

        let mut decoder = GifDecoder::new(std::io::Cursor::new(gif_bytes));
        assert_eq!(decoder.read_gif(), STATUS_OK);
        assert_eq!(decoder.frame_count(), 2);
        assert_eq!(decoder.loop_count(), 0);
        assert_eq!(decoder.frame_size(), (4, 3));
        assert_eq!(decoder.delay(0), 80);
        assert_eq!(decoder.delay(1), 80);

        let expected_frame0: [u8; 48] = [
            255, 4, 86, 5, 255, 38, 93, 2, 255, 82, 95, 3, 255, 111, 89, 4, 255, 4, 86, 5, 255, 38, 93, 2, 255, 82,
            95, 3, 255, 111, 89, 4, 255, 8, 89, 3, 255, 42, 94, 2, 255, 82, 95, 3, 255, 110, 95, 4,
        ];
        let expected_frame1: [u8; 48] = [
            255, 13, 87, 10, 255, 48, 93, 7, 255, 89, 94, 8, 255, 122, 80, 9, 255, 13, 87, 10, 255, 48, 93, 7, 255,
            89, 94, 8, 255, 122, 80, 9, 255, 15, 88, 9, 255, 52, 93, 7, 255, 89, 94, 8, 255, 121, 94, 9,
        ];

        for (idx, expected) in [(0, expected_frame0), (1, expected_frame1)] {
            let frame = decoder.frame(idx).unwrap();
            let mut actual = Vec::with_capacity(frame.image.len() * 4);
            for &p in &frame.image {
                actual.push(((p >> 24) & 0xff) as u8);
                actual.push(((p >> 16) & 0xff) as u8);
                actual.push(((p >> 8) & 0xff) as u8);
                actual.push((p & 0xff) as u8);
            }
            assert_eq!(actual, expected, "frame {idx}");
        }
    }
}
