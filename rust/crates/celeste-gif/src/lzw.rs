//! Port of `com.madgag.gif.fmsware.LZWEncoder` (GIFCOMPR.C-derived GIF LZW
//! compressor; adapted from Jef Poskanzer's Java port by way of J. M. G.
//! Elliott, K Weiner 12/00).

use std::io::{self, Write};

const EOF: i32 = -1;

const BITS: i32 = 12;
const HSIZE: usize = 5003; // 80% occupancy

const MASKS: [i32; 17] = [
    0x0000, 0x0001, 0x0003, 0x0007, 0x000F, 0x001F, 0x003F, 0x007F, 0x00FF, 0x01FF, 0x03FF, 0x07FF,
    0x0FFF, 0x1FFF, 0x3FFF, 0x7FFF, 0xFFFF,
];

pub struct LzwEncoder<'a> {
    img_w: i32,
    img_h: i32,
    pix_ary: &'a [u8],
    init_code_size: i32,
    remaining: i32,
    cur_pixel: i32,

    n_bits: i32,
    maxbits: i32,
    maxcode: i32,
    maxmaxcode: i32,

    htab: Vec<i32>,
    codetab: Vec<i32>,
    hsize: i32,

    free_ent: i32,
    clear_flg: bool,

    g_init_bits: i32,
    clear_code: i32,
    eof_code: i32,

    cur_accum: i32,
    cur_bits: i32,

    a_count: i32,
    accum: [u8; 256],
}

impl<'a> LzwEncoder<'a> {
    pub fn new(width: i32, height: i32, pixels: &'a [u8], color_depth: i32) -> Self {
        LzwEncoder {
            img_w: width,
            img_h: height,
            pix_ary: pixels,
            init_code_size: color_depth.max(2),
            remaining: 0,
            cur_pixel: 0,
            n_bits: 0,
            maxbits: BITS,
            maxcode: 0,
            maxmaxcode: 1 << BITS,
            htab: vec![0; HSIZE],
            codetab: vec![0; HSIZE],
            hsize: HSIZE as i32,
            free_ent: 0,
            clear_flg: false,
            g_init_bits: 0,
            clear_code: 0,
            eof_code: 0,
            cur_accum: 0,
            cur_bits: 0,
            a_count: 0,
            accum: [0; 256],
        }
    }

    /// Add a character to the end of the current packet, and if it is 254
    /// characters, flush the packet.
    fn char_out<W: Write>(&mut self, c: u8, outs: &mut W) -> io::Result<()> {
        self.accum[self.a_count as usize] = c;
        self.a_count += 1;
        if self.a_count >= 254 {
            self.flush_char(outs)?;
        }
        Ok(())
    }

    /// Table clear for block compress.
    fn cl_block<W: Write>(&mut self, outs: &mut W) -> io::Result<()> {
        self.cl_hash(self.hsize);
        self.free_ent = self.clear_code + 2;
        self.clear_flg = true;
        self.output(self.clear_code, outs)
    }

    /// Reset code table.
    fn cl_hash(&mut self, hsize: i32) {
        for i in 0..hsize {
            self.htab[i as usize] = -1;
        }
    }

    fn compress<W: Write>(&mut self, init_bits: i32, outs: &mut W) -> io::Result<()> {
        self.g_init_bits = init_bits;

        self.clear_flg = false;
        self.n_bits = self.g_init_bits;
        self.maxcode = Self::maxcode_for(self.n_bits);

        self.clear_code = 1 << (init_bits - 1);
        self.eof_code = self.clear_code + 1;
        self.free_ent = self.clear_code + 2;

        self.a_count = 0; // clear packet

        let mut ent = self.next_pixel();

        let mut hshift = 0;
        let mut fcode_probe = self.hsize;
        while fcode_probe < 65536 {
            hshift += 1;
            fcode_probe *= 2;
        }
        hshift = 8 - hshift; // set hash code range bound

        let hsize_reg = self.hsize;
        self.cl_hash(hsize_reg); // clear hash table

        self.output(self.clear_code, outs)?;

        'outer: loop {
            let c = self.next_pixel();
            if c == EOF {
                break 'outer;
            }
            let fcode = (c << self.maxbits) + ent;
            let mut i = (c << hshift) ^ ent; // xor hashing

            if self.htab[i as usize] == fcode {
                ent = self.codetab[i as usize];
                continue 'outer;
            } else if self.htab[i as usize] >= 0 {
                // non-empty slot
                let mut disp = hsize_reg - i; // secondary hash (after G. Knott)
                if i == 0 {
                    disp = 1;
                }
                loop {
                    i -= disp;
                    if i < 0 {
                        i += hsize_reg;
                    }
                    if self.htab[i as usize] == fcode {
                        ent = self.codetab[i as usize];
                        continue 'outer;
                    }
                    if self.htab[i as usize] < 0 {
                        break;
                    }
                }
            }
            self.output(ent, outs)?;
            ent = c;
            if self.free_ent < self.maxmaxcode {
                self.codetab[i as usize] = self.free_ent; // code -> hashtable
                self.free_ent += 1;
                self.htab[i as usize] = fcode;
            } else {
                self.cl_block(outs)?;
            }
        }

        // Put out the final code.
        self.output(ent, outs)?;
        self.output(self.eof_code, outs)?;
        Ok(())
    }

    pub fn encode<W: Write>(&mut self, os: &mut W) -> io::Result<()> {
        os.write_all(&[self.init_code_size as u8])?; // "initial code size" byte

        self.remaining = self.img_w * self.img_h; // reset navigation variables
        self.cur_pixel = 0;

        self.compress(self.init_code_size + 1, os)?; // compress and write the pixel data

        os.write_all(&[0]) // block terminator
    }

    /// Flush the packet, and reset the accumulator.
    fn flush_char<W: Write>(&mut self, outs: &mut W) -> io::Result<()> {
        if self.a_count > 0 {
            outs.write_all(&[self.a_count as u8])?;
            outs.write_all(&self.accum[0..self.a_count as usize])?;
            self.a_count = 0;
        }
        Ok(())
    }

    fn maxcode_for(n_bits: i32) -> i32 {
        (1 << n_bits) - 1
    }

    /// Return the next pixel from the image.
    fn next_pixel(&mut self) -> i32 {
        if self.remaining == 0 {
            return EOF;
        }
        self.remaining -= 1;
        let pix = self.pix_ary[self.cur_pixel as usize];
        self.cur_pixel += 1;
        pix as i32 & 0xff
    }

    fn output<W: Write>(&mut self, code: i32, outs: &mut W) -> io::Result<()> {
        self.cur_accum &= MASKS[self.cur_bits as usize];

        if self.cur_bits > 0 {
            self.cur_accum |= code << self.cur_bits;
        } else {
            self.cur_accum = code;
        }

        self.cur_bits += self.n_bits;

        while self.cur_bits >= 8 {
            self.char_out((self.cur_accum & 0xff) as u8, outs)?;
            self.cur_accum >>= 8;
            self.cur_bits -= 8;
        }

        // If the next entry is going to be too big for the code size,
        // then increase it, if possible.
        if self.free_ent > self.maxcode || self.clear_flg {
            if self.clear_flg {
                self.n_bits = self.g_init_bits;
                self.maxcode = Self::maxcode_for(self.n_bits);
                self.clear_flg = false;
            } else {
                self.n_bits += 1;
                if self.n_bits == self.maxbits {
                    self.maxcode = self.maxmaxcode;
                } else {
                    self.maxcode = Self::maxcode_for(self.n_bits);
                }
            }
        }

        if code == self.eof_code {
            // At EOF, write the rest of the buffer.
            while self.cur_bits > 0 {
                self.char_out((self.cur_accum & 0xff) as u8, outs)?;
                self.cur_accum >>= 8;
                self.cur_bits -= 8;
            }
            self.flush_char(outs)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_without_error_and_starts_with_code_size_byte() {
        let width = 4;
        let height = 4;
        let pixels = vec![0u8; (width * height) as usize];
        let mut enc = LzwEncoder::new(width, height, &pixels, 2);
        let mut out = Vec::new();
        enc.encode(&mut out).unwrap();
        assert_eq!(out[0], 2); // init code size byte
        assert_eq!(*out.last().unwrap(), 0); // block terminator
        assert!(out.len() > 2);
    }

    #[test]
    fn encodes_varied_pixel_data() {
        let width = 10;
        let height = 10;
        let pixels: Vec<u8> = (0..(width * height) as usize)
            .map(|i| (i % 7) as u8)
            .collect();
        let mut enc = LzwEncoder::new(width, height, &pixels, 7);
        let mut out = Vec::new();
        enc.encode(&mut out).unwrap();
        assert_eq!(out[0], 7);
        assert_eq!(*out.last().unwrap(), 0);
    }

    /// Golden-byte regression test cross-checked against the original Java
    /// `com.madgag.gif.fmsware.LZWEncoder` (OpenJDK 21): same 80-byte pixel
    /// buffer, same `color_depth`, byte-for-byte identical `encode()` output.
    #[test]
    fn matches_java_reference_output_byte_for_byte() {
        let pixels: [u8; 80] = std::array::from_fn(|i| ((i * 13 + 3) % 37) as u8);
        let mut enc = LzwEncoder::new(10, 8, &pixels, 6);
        let mut out = Vec::new();
        enc.encode(&mut out).unwrap();

        let expected: [u8; 57] = [
            6, 54, 192, 1, 164, 83, 144, 124, 14, 148, 80, 194, 50, 90, 96, 0, 13, 141, 224, 193,
            33, 68, 60, 134, 9, 8, 81, 17, 41, 46, 36, 70, 38, 224, 216, 8, 137, 70, 164, 146, 233,
            132, 74, 169, 86, 172, 150, 235, 5, 139, 201, 230, 97, 241, 19, 4, 0,
        ];
        assert_eq!(&out[..], &expected[..]);
    }
}
