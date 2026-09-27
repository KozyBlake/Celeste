//! Port of `com.madgag.gif.fmsware.NeuQuant`.
//!
//! NeuQuant Neural-Net Quantization Algorithm, Copyright (c) 1994 Anthony
//! Dekker. See "Kohonen neural networks for optimal colour quantization" in
//! "Network: Computation in Neural Systems" Vol. 5 (1994) pp 351-367.
//!
//! Ported line-for-line from the Java (itself ported from the original C by
//! K Weiner, 12/00) rather than rewritten, since this is a numerically
//! delicate fixed-point algorithm where "obviously equivalent" refactors are
//! exactly how subtle quantization-quality regressions creep in. A couple of
//! `clippy::needless_range_loop` lints are silenced for the same reason:
//! index-based loops here mirror the Java source directly, which matters
//! more for auditability than idiom.
#![allow(clippy::needless_range_loop)]

const NETSIZE: usize = 256; // number of colours used

// four primes near 500 - assume no image has a length so large that it is
// divisible by all four primes
const PRIME1: i32 = 499;
const PRIME2: i32 = 491;
const PRIME3: i32 = 487;
const PRIME4: i32 = 503;

const MINPICTUREBYTES: i32 = 3 * PRIME4; // minimum size for input image

const MAXNETPOS: i32 = (NETSIZE - 1) as i32;
const NETBIASSHIFT: i32 = 4; // bias for colour values
const NCYCLES: i32 = 100; // no. of learning cycles

// defs for freq and bias
const INTBIASSHIFT: i32 = 16; // bias for fractions
const INTBIAS: i32 = 1 << INTBIASSHIFT;
const GAMMASHIFT: i32 = 10; // gamma = 1024
const BETASHIFT: i32 = 10;
const BETA: i32 = INTBIAS >> BETASHIFT; // beta = 1/1024
const BETAGAMMA: i32 = INTBIAS << (GAMMASHIFT - BETASHIFT);

// defs for decreasing radius factor
const INITRAD: i32 = (NETSIZE >> 3) as i32; // for 256 cols, radius starts
const RADIUSBIASSHIFT: i32 = 6; // at 32.0 biased by 6 bits
const RADIUSBIAS: i32 = 1 << RADIUSBIASSHIFT;
const INITRADIUS: i32 = INITRAD * RADIUSBIAS; // and decreases by a
const RADIUSDEC: i32 = 30; // factor of 1/30 each cycle

// defs for decreasing alpha factor
const ALPHABIASSHIFT: i32 = 10; // alpha starts at 1.0
const INITALPHA: i32 = 1 << ALPHABIASSHIFT;

// radbias and alpharadbias used for radpower calculation
const RADBIASSHIFT: i32 = 8;
const RADBIAS: i32 = 1 << RADBIASSHIFT;
const ALPHARADBSHIFT: i32 = ALPHABIASSHIFT + RADBIASSHIFT;
const ALPHARADBIAS: i32 = 1 << ALPHARADBSHIFT;

pub struct NeuQuant<'a> {
    thepicture: &'a [u8],
    lengthcount: i32,
    samplefac: i32,
    alphadec: i32,

    /// the network itself - `[netsize][4]` (b, g, r, colour-index)
    network: Vec<[i32; 4]>,
    /// for network lookup - really 256
    netindex: [i32; 256],
    /// bias and freq arrays for learning
    bias: [i32; NETSIZE],
    freq: [i32; NETSIZE],
    /// radpower for precomputation
    radpower: Vec<i32>,
}

impl<'a> NeuQuant<'a> {
    /// Initialise network in range (0,0,0) to (255,255,255) and set parameters.
    pub fn new(thepic: &'a [u8], len: i32, sample: i32) -> Self {
        let mut network = Vec::with_capacity(NETSIZE);
        let mut freq = [0i32; NETSIZE];
        for i in 0..NETSIZE {
            let v = ((i as i32) << (NETBIASSHIFT + 8)) / NETSIZE as i32;
            network.push([v, v, v, 0]);
            freq[i] = INTBIAS / NETSIZE as i32; // 1/netsize
        }
        NeuQuant {
            thepicture: thepic,
            lengthcount: len,
            samplefac: sample,
            alphadec: 0,
            network,
            netindex: [0; 256],
            bias: [0; NETSIZE],
            freq,
            radpower: vec![0; INITRAD as usize],
        }
    }

    pub fn color_map(&self) -> Vec<u8> {
        let mut map = vec![0u8; 3 * NETSIZE];
        let mut index = [0usize; NETSIZE];
        for i in 0..NETSIZE {
            index[self.network[i][3] as usize] = i;
        }
        let mut k = 0;
        for i in 0..NETSIZE {
            let j = index[i];
            map[k] = self.network[j][0] as u8;
            map[k + 1] = self.network[j][1] as u8;
            map[k + 2] = self.network[j][2] as u8;
            k += 3;
        }
        map
    }

    /// Insertion sort of network and building of netindex\[0..255\] (to do after unbias).
    pub fn inxbuild(&mut self) {
        let mut previouscol = 0i32;
        let mut startpos = 0i32;
        for i in 0..NETSIZE {
            let mut smallpos = i;
            let mut smallval = self.network[i][1]; // index on g
            // find smallest in i..netsize-1
            for j in (i + 1)..NETSIZE {
                if self.network[j][1] < smallval {
                    smallpos = j;
                    smallval = self.network[j][1];
                }
            }
            // swap i and smallpos entries
            if i != smallpos {
                self.network.swap(i, smallpos);
            }
            // smallval entry is now in position i
            if smallval != previouscol {
                self.netindex[previouscol as usize] = (startpos + i as i32) >> 1;
                for j in (previouscol + 1)..smallval {
                    self.netindex[j as usize] = i as i32;
                }
                previouscol = smallval;
                startpos = i as i32;
            }
        }
        self.netindex[previouscol as usize] = (startpos + MAXNETPOS) >> 1;
        for j in (previouscol + 1)..256 {
            self.netindex[j as usize] = MAXNETPOS; // really 256
        }
    }

    /// Main learning loop.
    pub fn learn(&mut self) {
        if self.lengthcount < MINPICTUREBYTES {
            self.samplefac = 1;
        }
        self.alphadec = 30 + ((self.samplefac - 1) / 3);
        let p = self.thepicture;
        let mut pix: i32 = 0;
        let lim = self.lengthcount;
        let samplepixels = self.lengthcount / (3 * self.samplefac);
        let mut delta = samplepixels / NCYCLES;
        let mut alpha = INITALPHA;
        let mut radius = INITRADIUS;

        let mut rad = radius >> RADIUSBIASSHIFT;
        if rad <= 1 {
            rad = 0;
        }
        for i in 0..rad {
            self.radpower[i as usize] =
                alpha * (((rad * rad - i * i) * RADBIAS) / (rad * rad));
        }

        let step = if self.lengthcount < MINPICTUREBYTES {
            3
        } else if (self.lengthcount % PRIME1) != 0 {
            3 * PRIME1
        } else if (self.lengthcount % PRIME2) != 0 {
            3 * PRIME2
        } else if (self.lengthcount % PRIME3) != 0 {
            3 * PRIME3
        } else {
            3 * PRIME4
        };

        let mut i = 0;
        while i < samplepixels {
            let b = (p[pix as usize] as i32 & 0xff) << NETBIASSHIFT;
            let g = (p[(pix + 1) as usize] as i32 & 0xff) << NETBIASSHIFT;
            let r = (p[(pix + 2) as usize] as i32 & 0xff) << NETBIASSHIFT;
            let j = self.contest(b, g, r);

            self.altersingle(alpha, j, b, g, r);
            if rad != 0 {
                self.alterneigh(rad, j, b, g, r); // alter neighbours
            }

            pix += step;
            if pix >= lim {
                pix -= self.lengthcount;
            }

            i += 1;
            if delta == 0 {
                delta = 1;
            }
            if i % delta == 0 {
                alpha -= alpha / self.alphadec;
                radius -= radius / RADIUSDEC;
                rad = radius >> RADIUSBIASSHIFT;
                if rad <= 1 {
                    rad = 0;
                }
                for j in 0..rad {
                    self.radpower[j as usize] =
                        alpha * (((rad * rad - j * j) * RADBIAS) / (rad * rad));
                }
            }
        }
    }

    /// Search for BGR values 0..255 (after net is unbiased) and return colour index.
    pub fn map(&self, b: i32, g: i32, r: i32) -> i32 {
        let mut bestd = 1000i32; // biggest possible dist is 256*3
        let mut best = -1i32;
        let mut i = self.netindex[g as usize]; // index on g
        let mut j = i - 1; // start at netindex[g] and work outwards

        while i < NETSIZE as i32 || j >= 0 {
            if i < NETSIZE as i32 {
                let p = &self.network[i as usize];
                let mut dist = p[1] - g; // inx key
                if dist >= bestd {
                    i = NETSIZE as i32; // stop iter
                } else {
                    i += 1;
                    if dist < 0 {
                        dist = -dist;
                    }
                    let mut a = p[0] - b;
                    if a < 0 {
                        a = -a;
                    }
                    dist += a;
                    if dist < bestd {
                        a = p[2] - r;
                        if a < 0 {
                            a = -a;
                        }
                        dist += a;
                        if dist < bestd {
                            bestd = dist;
                            best = p[3];
                        }
                    }
                }
            }
            if j >= 0 {
                let p = &self.network[j as usize];
                let mut dist = g - p[1]; // inx key - reverse dif
                if dist >= bestd {
                    j = -1; // stop iter
                } else {
                    j -= 1;
                    if dist < 0 {
                        dist = -dist;
                    }
                    let mut a = p[0] - b;
                    if a < 0 {
                        a = -a;
                    }
                    dist += a;
                    if dist < bestd {
                        a = p[2] - r;
                        if a < 0 {
                            a = -a;
                        }
                        dist += a;
                        if dist < bestd {
                            bestd = dist;
                            best = p[3];
                        }
                    }
                }
            }
        }
        best
    }

    pub fn process(&mut self) -> Vec<u8> {
        self.learn();
        self.unbiasnet();
        self.inxbuild();
        self.color_map()
    }

    /// Unbias network to give byte values 0..255 and record position i to
    /// prepare for sort.
    pub fn unbiasnet(&mut self) {
        for i in 0..NETSIZE {
            self.network[i][0] >>= NETBIASSHIFT;
            self.network[i][1] >>= NETBIASSHIFT;
            self.network[i][2] >>= NETBIASSHIFT;
            self.network[i][3] = i as i32; // record colour no
        }
    }

    /// Move adjacent neurons by precomputed `alpha*(1-((i-j)^2/[r]^2))` in
    /// `radpower[|i-j|]`.
    fn alterneigh(&mut self, rad: i32, i: i32, b: i32, g: i32, r: i32) {
        let mut lo = i - rad;
        if lo < -1 {
            lo = -1;
        }
        let mut hi = i + rad;
        if hi > NETSIZE as i32 {
            hi = NETSIZE as i32;
        }

        let mut j = i + 1;
        let mut k = i - 1;
        let mut m = 1;
        while j < hi || k > lo {
            let a = self.radpower[m as usize];
            m += 1;
            if j < hi {
                let p = &mut self.network[j as usize];
                p[0] -= (a * (p[0] - b)) / ALPHARADBIAS;
                p[1] -= (a * (p[1] - g)) / ALPHARADBIAS;
                p[2] -= (a * (p[2] - r)) / ALPHARADBIAS;
                j += 1;
            }
            if k > lo {
                let p = &mut self.network[k as usize];
                p[0] -= (a * (p[0] - b)) / ALPHARADBIAS;
                p[1] -= (a * (p[1] - g)) / ALPHARADBIAS;
                p[2] -= (a * (p[2] - r)) / ALPHARADBIAS;
                k -= 1;
            }
        }
    }

    /// Move neuron i towards biased (b,g,r) by factor alpha.
    fn altersingle(&mut self, alpha: i32, i: i32, b: i32, g: i32, r: i32) {
        let n = &mut self.network[i as usize];
        n[0] -= (alpha * (n[0] - b)) / INITALPHA;
        n[1] -= (alpha * (n[1] - g)) / INITALPHA;
        n[2] -= (alpha * (n[2] - r)) / INITALPHA;
    }

    /// Search for biased BGR values.
    ///
    /// Finds closest neuron (min dist) and updates freq; finds best neuron
    /// (min dist-bias) and returns position. For frequently chosen neurons,
    /// `freq[i]` is high and `bias[i]` is negative:
    /// `bias[i] = gamma*((1/netsize)-freq[i])`.
    fn contest(&mut self, b: i32, g: i32, r: i32) -> i32 {
        let start_bestd = !(1i32 << 31); // i32::MAX
        let mut bestd = start_bestd;
        let mut bestbiasd = bestd;
        let mut bestpos = -1i32;
        let mut bestbiaspos = bestpos;

        for i in 0..NETSIZE {
            let n = &self.network[i];
            let mut dist = n[0] - b;
            if dist < 0 {
                dist = -dist;
            }
            let mut a = n[1] - g;
            if a < 0 {
                a = -a;
            }
            dist += a;
            a = n[2] - r;
            if a < 0 {
                a = -a;
            }
            dist += a;
            if dist < bestd {
                bestd = dist;
                bestpos = i as i32;
            }
            let biasdist = dist - (self.bias[i] >> (INTBIASSHIFT - NETBIASSHIFT));
            if biasdist < bestbiasd {
                bestbiasd = biasdist;
                bestbiaspos = i as i32;
            }
            let betafreq = self.freq[i] >> BETASHIFT;
            self.freq[i] -= betafreq;
            self.bias[i] += betafreq << GAMMASHIFT;
        }
        self.freq[bestpos as usize] += BETA;
        self.bias[bestpos as usize] -= BETAGAMMA;
        bestbiaspos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_color_image(width: usize, height: usize, bgr: [u8; 3]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(width * height * 3);
        for _ in 0..(width * height) {
            buf.extend_from_slice(&bgr);
        }
        buf
    }

    #[test]
    fn map_returns_a_valid_palette_index() {
        let pic = solid_color_image(8, 8, [100, 150, 200]);
        let len = pic.len() as i32;
        let mut nq = NeuQuant::new(&pic, len, 10);
        nq.process();
        let idx = nq.map(100, 150, 200);
        assert!((0..NETSIZE as i32).contains(&idx));
    }

    /// Golden-byte regression test cross-checked against the original Java
    /// `com.madgag.gif.fmsware.NeuQuant` (OpenJDK 21): same 90-byte input,
    /// same `samplefac`, byte-for-byte identical `process()` output and
    /// `map()` lookups. This is a fixed-point, order-of-operations-sensitive
    /// algorithm, so this test pins the port against silent numeric drift
    /// (e.g. an accidentally reordered shift/divide) that unit tests on
    /// "roughly the right colour" can't catch.
    #[test]
    fn matches_java_reference_output_byte_for_byte() {
        let input: [u8; 90] = std::array::from_fn(|i| ((i * 37 + 11) % 256) as u8);
        let mut nq = NeuQuant::new(&input, input.len() as i32, 10);
        let map = nq.process();

        let expected_map: [u8; 768] = [
            77, 4, 16, 98, 5, 21, 116, 6, 25, 138, 8, 30, 156, 9, 34, 171, 10, 37, 183, 11, 40,
            192, 11, 42, 202, 12, 44, 210, 13, 46, 216, 14, 48, 220, 14, 49, 224, 15, 50, 227, 16,
            51, 229, 16, 52, 231, 17, 53, 233, 18, 54, 234, 18, 55, 236, 19, 56, 237, 20, 57, 238,
            21, 58, 239, 22, 59, 241, 23, 60, 242, 24, 61, 243, 26, 63, 244, 27, 64, 244, 27, 64,
            244, 27, 64, 243, 27, 64, 243, 27, 64, 242, 27, 63, 241, 26, 63, 240, 26, 63, 238, 26,
            63, 235, 26, 62, 233, 26, 62, 229, 25, 62, 221, 26, 63, 212, 27, 63, 201, 28, 64, 176,
            30, 66, 151, 32, 68, 127, 34, 71, 102, 36, 73, 85, 38, 75, 69, 40, 77, 54, 42, 79, 40,
            44, 81, 27, 45, 82, 23, 46, 83, 19, 46, 83, 16, 47, 84, 13, 47, 84, 11, 48, 85, 11, 48,
            85, 12, 48, 85, 12, 49, 86, 13, 50, 86, 14, 50, 87, 15, 51, 88, 16, 52, 89, 17, 53, 90,
            18, 54, 91, 19, 55, 92, 21, 57, 93, 22, 58, 94, 24, 60, 96, 26, 62, 98, 27, 63, 99, 29,
            64, 100, 30, 65, 101, 31, 66, 102, 31, 67, 103, 32, 68, 104, 33, 69, 104, 34, 69, 105,
            34, 70, 106, 35, 71, 106, 36, 72, 107, 37, 73, 109, 39, 75, 110, 40, 76, 112, 41, 77,
            113, 42, 78, 114, 43, 79, 115, 43, 80, 116, 44, 81, 117, 45, 81, 117, 46, 82, 118, 46,
            83, 119, 47, 84, 120, 49, 85, 121, 50, 86, 122, 52, 88, 124, 54, 90, 126, 56, 91, 127,
            58, 93, 129, 60, 95, 131, 61, 97, 132, 62, 98, 133, 64, 99, 134, 66, 101, 136, 67, 102,
            137, 68, 103, 137, 69, 103, 137, 69, 103, 137, 70, 104, 137, 71, 105, 138, 72, 106,
            139, 74, 107, 140, 75, 108, 141, 76, 109, 142, 77, 110, 143, 78, 111, 143, 79, 112,
            144, 80, 113, 145, 82, 114, 145, 84, 115, 146, 86, 117, 147, 87, 118, 148, 87, 118,
            149, 88, 119, 150, 88, 120, 151, 89, 120, 151, 90, 121, 152, 91, 122, 153, 91, 123,
            155, 91, 124, 156, 92, 125, 158, 92, 125, 159, 93, 127, 161, 93, 128, 162, 94, 129,
            164, 94, 129, 164, 94, 130, 165, 94, 130, 166, 94, 130, 166, 95, 131, 167, 96, 132,
            169, 97, 133, 169, 97, 134, 170, 98, 135, 171, 99, 135, 172, 99, 136, 172, 100, 136,
            173, 100, 137, 174, 101, 138, 175, 102, 139, 176, 104, 141, 177, 105, 142, 178, 106,
            143, 179, 106, 143, 180, 107, 144, 181, 108, 145, 182, 109, 146, 183, 110, 147, 184,
            111, 148, 185, 112, 149, 186, 112, 149, 186, 114, 151, 188, 116, 153, 190, 118, 155,
            192, 120, 157, 194, 121, 158, 195, 123, 160, 197, 125, 162, 199, 127, 164, 201, 129,
            166, 203, 131, 168, 205, 133, 170, 207, 135, 172, 209, 138, 175, 211, 140, 177, 214,
            142, 179, 215, 143, 179, 214, 143, 179, 213, 144, 181, 205, 146, 183, 197, 148, 184,
            189, 150, 187, 179, 153, 189, 168, 158, 194, 147, 163, 199, 127, 167, 203, 109, 171,
            207, 92, 176, 212, 75, 180, 216, 58, 184, 220, 44, 188, 224, 30, 189, 226, 25, 191,
            227, 21, 192, 229, 17, 193, 230, 16, 194, 230, 15, 194, 231, 15, 195, 232, 14, 195,
            232, 15, 196, 233, 15, 197, 234, 16, 198, 235, 17, 199, 236, 19, 200, 237, 21, 202,
            238, 24, 203, 240, 28, 205, 241, 33, 206, 242, 38, 207, 243, 43, 208, 244, 49, 207,
            243, 54, 205, 241, 64, 200, 236, 86, 195, 231, 106, 191, 227, 125, 187, 223, 142, 183,
            219, 158, 180, 216, 174, 177, 213, 188, 174, 210, 202, 171, 207, 216, 170, 206, 221,
            169, 205, 227, 168, 204, 231, 168, 203, 236, 168, 204, 237, 168, 204, 238, 169, 204,
            239, 169, 204, 240, 170, 205, 240, 170, 205, 241, 171, 206, 241, 172, 207, 242, 173,
            208, 242, 175, 209, 243, 176, 210, 243, 177, 210, 243, 178, 211, 243, 179, 211, 243,
            181, 212, 243, 182, 213, 243, 184, 213, 243, 187, 215, 242, 190, 216, 242, 193, 218,
            242, 197, 220, 243, 201, 222, 243, 207, 225, 243, 213, 228, 244, 220, 233, 244, 228,
            237, 245, 234, 240, 247, 239, 244, 248, 245, 248, 250, 252, 252, 252, 253, 253, 253,
            254, 254, 254, 255, 255, 255,
        ];
        assert_eq!(&map[..], &expected_map[..]);

        let expected_lookups = [54i32, 224, 101, 205, 160];
        for (idx, &expected) in expected_lookups.iter().enumerate() {
            let i = idx * 18;
            let b = input[i] as i32;
            let g = input[i + 1] as i32;
            let r = input[i + 2] as i32;
            assert_eq!(nq.map(b, g, r), expected, "lookup #{idx}");
        }
    }
}
