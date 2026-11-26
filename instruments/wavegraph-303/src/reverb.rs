use crate::{
    LengthMismatch, StereoFrame,
    dsp::{fast_exp2, fast_sin, tau_to_coef},
};
use core::{f32::consts::PI, mem};

// Delay lengths in samples at the Dattorro reference rate.
const RATE: f32 = 29_761.0;
const INPUT: [f32; 4] = [142.0, 107.0, 379.0, 277.0];
// Modulated allpass, first delay, tank allpass and second delay in each half.
const TANK: [[f32; 4]; 2] = [
    [672.0, 4453.0, 1800.0, 3720.0],
    [908.0, 4217.0, 2656.0, 3163.0],
];

/// Dattorro plate reverb. Its delay lines borrow disjoint regions of caller-owned storage.
#[derive(Debug)]
pub struct Reverb<'a> {
    rate: f32,
    scale: f32,
    output_taps: [[usize; 7]; 2],
    increments: [f32; 2],
    size: f32,
    max_size: f32,
    decay: f32,
    damp: f32,
    bandwidth: f32,
    bw1: f32,
    bw2: f32,
    pre: Line<'a>,
    input: [Line<'a>; 4],
    tank: [Half<'a>; 2],
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects,
    reason = "Line lengths truncate fractional frames as in CHOMPI; construction bounds their sum to 65536."
)]
impl<'a> Reverb<'a> {
    /// Size of the original reverb's mono scratch buffer, in samples.
    pub const FRAMES: usize = 65_536;

    /// Initialize the tank for this sample rate. Lower rates can supply a shorter
    /// buffer. Returns `None` if the lines exceed the storage or the original
    /// maximum of 65,536 samples.
    pub fn new(rate: f32, storage: &'a mut [f32]) -> Option<Self> {
        Self::with_max_size(rate, storage, 1.0)
    }

    /// Reserve only the delay lengths needed up to `max_size` (0..=1).
    /// The layout and taps at a supported size sound identical to the full tank.
    /// Subsequent size edits are clamped to this limit.
    pub fn with_max_size(rate: f32, storage: &'a mut [f32], max_size: f32) -> Option<Self> {
        if !rate.is_finite() || rate <= 0.0 || !(0.0..=1.0).contains(&max_size) {
            return None;
        }
        let scale = rate / RATE;
        let pre_base = 0.04 * RATE / 1.25 + 1.0;
        let max_scale = 0.75 + 0.5 * max_size;
        let line_size = |base: f32| ((base * scale * max_scale) as usize).saturating_add(64);
        let used = core::iter::once(pre_base)
            .chain(INPUT)
            .chain(TANK.into_iter().flatten())
            .try_fold(0usize, |used, base| used.checked_add(line_size(base)))?;
        if used > Self::FRAMES {
            return None;
        }
        let mut rest = storage.get_mut(..used)?;
        rest.fill(0.0);
        let mut take = |base| {
            let (buf, tail) = mem::take(&mut rest).split_at_mut(line_size(base));
            rest = tail;
            Line {
                buf,
                len: 1,
                pos: 0,
            }
        };
        let pre = take(pre_base);
        let input = INPUT.map(&mut take);
        let [first, second] = TANK.map(|bases| {
            let [mod_ap, d1, ap, d2] = bases.map(&mut take);
            Half {
                mod_ap,
                d1,
                ap,
                d2,
                lp: 0.0,
                phase: 0.0,
            }
        });
        let mut rv = Self {
            rate,
            scale,
            output_taps: [[0; 7]; 2],
            increments: [0.6 / rate, 0.83 / rate],
            size: -1.0,
            max_size,
            decay: 0.5,
            damp: 0.5,
            bandwidth: 0.5,
            bw1: 0.0,
            bw2: 0.0,
            pre,
            input,
            tank: [first, second],
        };
        rv.tank[1].phase = 0.25;
        rv.set_size(0.5);
        Some(rv)
    }

    /// Set the room size from zero to one. Small changes below 0.0001 are ignored.
    pub fn set_size(&mut self, size: f32) {
        let size = size.clamp(0.0, self.max_size);
        if (size - self.size).abs() < 1e-4 {
            return;
        }
        self.size = size;
        let sc = 0.75 + 0.5 * size;
        let rt60 = 0.4 * fast_exp2(3.9 * size);
        for (line, base) in self.input.iter_mut().zip(INPUT) {
            line.len = (base * self.scale) as usize;
        }
        let mut loop_length = 0.0;
        for (half, bases) in self.tank.iter_mut().zip(TANK) {
            let [mod_ap, d1, ap, d2] = bases.map(|base| (base * self.scale * sc) as usize);
            half.mod_ap.len = mod_ap;
            half.d1.len = d1;
            half.ap.len = ap;
            half.d2.len = d2;
            loop_length += (mod_ap + d1 + ap + d2) as f32;
        }
        let exponent = -3.0 * loop_length / (4.0 * rt60 * self.rate);
        self.decay = crate::dsp::decay_exp(exponent * core::f32::consts::LN_10).min(0.97);
        self.pre.len = ((0.01 + 0.03 * size) * self.rate) as usize;
        self.damp = tau_to_coef(1.0 / (2.0 * PI * (6000.0 - 3500.0 * size)), self.rate);
        self.bandwidth = tau_to_coef(1.0 / (2.0 * PI * (8000.0 - 3000.0 * size)), self.rate);
        let k = self.scale * sc;
        self.output_taps = [
            [266.0, 2974.0, 1913.0, 1996.0, 1990.0, 187.0, 1066.0],
            [353.0, 3627.0, 1228.0, 2673.0, 2111.0, 335.0, 121.0],
        ]
        .map(|row| row.map(|tap| (tap * k) as usize));
    }

    /// Add the wet return to an existing stereo block.
    ///
    /// # Errors
    /// Returns [`LengthMismatch`] before processing if the block lengths differ.
    // SAFETY: the FM-1 startup copies this hot loop into executable SRAM.
    #[cfg_attr(
        all(target_os = "none", feature = "fm1-ram-code"),
        allow(unsafe_code),
        unsafe(link_section = ".data.wavegraph_audio"),
        inline(never)
    )]
    pub fn process_add(
        &mut self,
        input: &[f32],
        output: &mut [StereoFrame],
    ) -> Result<(), LengthMismatch> {
        if input.len() != output.len() {
            return Err(LengthMismatch);
        }
        for (&sample, out) in input.iter().zip(output) {
            let delayed = self.pre.read(self.pre.len);
            self.pre.write(sample);
            self.bw1 += (delayed - self.bw1) * self.bandwidth;
            self.bw2 += (self.bw1 - self.bw2) * self.bandwidth;
            let mut x = self.bw2;
            for (line, gain) in self.input.iter_mut().zip([0.75, 0.75, 0.625, 0.625]) {
                x = line.allpass(x, gain);
            }

            // Read both tank ends before advancing either feedback path.
            let [a, b] = &self.tank;
            let ends = [b.d2.read(b.d2.len), a.d2.read(a.d2.len)];
            for ((t, end), inc) in self.tank.iter_mut().zip(ends).zip(self.increments) {
                t.phase += inc;
                if t.phase >= 1.0 {
                    t.phase -= 1.0;
                }
                let m = t.mod_ap.len as f32 + 16.0 * self.scale * fast_sin(t.phase);
                let d = t.mod_ap.read_frac(m);
                let v = (x + self.decay * end) + 0.7 * d;
                t.mod_ap.write(v);
                t.d1.write(d - 0.7 * v);
                t.lp += (t.d1.read(t.d1.len) - t.lp) * self.damp;
                t.d2.write(t.ap.allpass(t.lp * self.decay, -0.5));
            }

            let [a, b] = &self.tank;
            let [l, r] = self.output_taps;
            let l = b.d1.read(l[0]) + b.d1.read(l[1]) - b.ap.read(l[2]) + b.d2.read(l[3])
                - a.d1.read(l[4])
                - a.ap.read(l[5])
                - a.d2.read(l[6]);
            let r = a.d1.read(r[0]) + a.d1.read(r[1]) - a.ap.read(r[2]) + a.d2.read(r[3])
                - b.d1.read(r[4])
                - b.ap.read(r[5])
                - b.d2.read(r[6]);
            out.left += 0.5 * l;
            out.right += 0.5 * r;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Half<'a> {
    mod_ap: Line<'a>,
    d1: Line<'a>,
    ap: Line<'a>,
    d2: Line<'a>,
    lp: f32,
    phase: f32,
}

#[derive(Debug)]
struct Line<'a> {
    buf: &'a mut [f32],
    len: usize,
    pos: usize,
}
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects,
    reason = "Each line has at least 64 frames and uses nonnegative tank delays bounded by the validated layout."
)]
impl Line<'_> {
    fn read(&self, delay: usize) -> f32 {
        let size = self.buf.len();
        let delay = if delay > size { delay % size } else { delay };
        let index = if self.pos >= delay {
            self.pos - delay
        } else {
            size - (delay - self.pos)
        };
        self.buf[index]
    }
    fn read_frac(&self, delay: f32) -> f32 {
        let whole = delay as usize;
        let f = delay - whole as f32;
        let a = self.read(whole);
        a + (self.read(whole + 1) - a) * f
    }
    fn write(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
    }
    fn allpass(&mut self, x: f32, gain: f32) -> f32 {
        let d = self.read(self.len);
        let v = x + gain * d;
        self.write(v);
        d - gain * v
    }
}
