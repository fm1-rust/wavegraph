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
    tap_scale: f32,
    size: f32,
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

    /// Initialize the original fixed-size tank. Returns `None` for an invalid
    /// rate, short storage, or a sample rate whose delay lengths exceed the tank.
    pub fn new(rate: f32, storage: &'a mut [f32]) -> Option<Self> {
        if !rate.is_finite() || rate <= 0.0 {
            return None;
        }
        let mut rest = storage.get_mut(..Self::FRAMES)?;
        let scale = rate / RATE;
        let pre_base = 0.04 * RATE / 1.25 + 1.0;
        let line_size = |base: f32| ((base * scale * 1.25) as usize).saturating_add(64);
        let used = core::iter::once(pre_base)
            .chain(INPUT)
            .chain(TANK.into_iter().flatten())
            .try_fold(0usize, |used, base| used.checked_add(line_size(base)))?;
        if used > rest.len() {
            return None;
        }
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
            tap_scale: scale,
            size: -1.0,
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
        let size = size.clamp(0.0, 1.0);
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
        self.tap_scale = self.scale * sc;
    }

    /// Add the wet return to an existing stereo block.
    ///
    /// # Errors
    /// Returns [`LengthMismatch`] before processing if the block lengths differ.
    pub fn process_add(
        &mut self,
        input: &[f32],
        output: &mut [StereoFrame],
    ) -> Result<(), LengthMismatch> {
        if input.len() != output.len() {
            return Err(LengthMismatch);
        }
        let increments = [0.6 / self.rate, 0.83 / self.rate];
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
            for ((t, end), inc) in self.tank.iter_mut().zip(ends).zip(increments) {
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
            let k = self.tap_scale;
            let l = b.d1.tap(266.0 * k) + b.d1.tap(2974.0 * k) - b.ap.tap(1913.0 * k)
                + b.d2.tap(1996.0 * k)
                - a.d1.tap(1990.0 * k)
                - a.ap.tap(187.0 * k)
                - a.d2.tap(1066.0 * k);
            let r = a.d1.tap(353.0 * k) + a.d1.tap(3627.0 * k) - a.ap.tap(1228.0 * k)
                + a.d2.tap(2673.0 * k)
                - b.d1.tap(2111.0 * k)
                - b.ap.tap(335.0 * k)
                - b.d2.tap(121.0 * k);
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
    fn tap(&self, delay: f32) -> f32 {
        self.read(delay as usize)
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
