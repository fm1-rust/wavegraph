//! Two opposing fractional taps on the same band-limited mono input.
//! The short delay memory is shared because both BBD paths receive the same mix.
mod mks50;
use mks50::WetPath;

const PCM_MAX: f32 = f32::from_bits(1.0_f32.to_bits() - 1);
use wavegraph_sample::{FromSample, I24};

/// Panel controls for the MKS-50-inspired parallel delay paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChorusControls {
    /// LFO panel value, 0..=127. The MKS-50 uses a nonlinear rate curve.
    pub rate: u8,
    /// Sweep depth, 0..=128. The Hoover setting is 64.
    pub depth: u8,
    /// Wet-path brightness, 0..=128. The dry path is unaffected.
    pub tone: u8,
    /// Dry/wet balance, 0..=100. The MKS-50 has unity dry plus its wet return
    /// at 50. Below 50 the wet level fades, and above 50 the dry level fades.
    pub mix: u8,
    /// Enable the delayed returns.
    pub enabled: bool,
}

impl ChorusControls {
    /// Fast Hoover's chorus settings.
    pub const INITIAL: Self = Self {
        rate: 92,
        depth: 64,
        tone: 128,
        mix: 37,
        enabled: true,
    };

    /// MKS-50 nominal equal dry and wet routing. The wet chain also has insertion loss.
    pub const MKS50: Self = Self {
        mix: 50,
        ..Self::INITIAL
    };
}

/// Two band-limited, oppositely swept delay taps on one mono input.
#[derive(Clone, Copy, Debug)]
pub struct Chorus {
    ring: [i16; 256],
    cursor: u8,
    phase: u32,
    filtered: f32,
    center: f32,
    pole: f32,
    period: f32,
    step: u32,
    sweep_scale: f32,
    dry: f32,
    wet: f32,
    enabled: bool,
    // Zero input leaves the filter unchanged and the entire delay ring is zero.
    silent: bool,
    mks50: bool,
    color: WetPath,
}
impl Chorus {
    /// Construct from the sample period in seconds.
    #[must_use]
    pub fn new(period: f32) -> Self {
        Self::with_model(period, false)
    }

    /// Construct the MKS-50 ensemble with filtered, oppositely clocked BBD lines.
    #[must_use]
    pub fn mks50(period: f32) -> Self {
        Self::with_model(period, true)
    }

    fn with_model(period: f32, mks50: bool) -> Self {
        let mut chorus = Self {
            ring: [0; 256],
            cursor: 0,
            phase: 0,
            filtered: 0.0,
            center: (0.0035 / period).clamp(1.0, 185.0),
            period,
            pole: 0.0,
            step: 0,
            sweep_scale: 0.0,
            dry: 0.0,
            wet: 0.0,
            enabled: true,
            silent: false,
            mks50,
            color: if mks50 {
                WetPath::new(period)
            } else {
                WetPath::default()
            },
        };
        chorus.set_controls(if mks50 {
            ChorusControls::MKS50
        } else {
            ChorusControls::INITIAL
        });
        chorus
    }

    /// Change parameters without clearing the delay or resetting the LFO.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn set_controls(&mut self, controls: ChorusControls) {
        let hz = if self.mks50 {
            mks50::rate(controls.rate)
        } else {
            0.15 + f32::from(controls.rate.min(127)) * 0.012
        };
        self.step = (hz * self.period * 4294967296.0) as u32;
        // Keep both fractional taps within the 256-frame history at high sample rates.
        let limit = if self.mks50 {
            1.0 - self.center / 252.0
        } else {
            254.0 / self.center - 1.0
        };
        let depth = (0.34 * (f32::from(controls.depth.min(128)) / 64.0)).min(limit);
        self.sweep_scale = depth / 16384.0;
        let tone = f32::from(controls.tone.min(128));
        self.pole = if self.mks50 {
            let hz = 300.0 + 572.0 * (tone / 128.0);
            -libm::expm1f(-core::f32::consts::TAU * hz * self.period)
        } else {
            ((2_400.0 + 200.0 * tone) * self.period).min(1.0)
        };
        let wet = f32::from(controls.mix.min(100)) / 100.0;
        // The MKS-50 switch only gates the wet return. At nominal balance the
        // dry path retains unity gain; resistor ratios give ~0.914 wet/dry at DC.
        (self.dry, self.wet) = if self.mks50 {
            (
                (2.0 * (1.0 - wet)).min(1.0),
                (2.0 * wet).min(1.0) * (0.914 / 32768.0),
            )
        } else {
            (1.0 - wet, wet / 32768.0)
        };
        self.enabled = controls.enabled;
        self.silent = false;
    }
    #[inline]
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "The delay fits in the 256-frame ring and the difference of two i16 samples fits in f32"
    )]
    fn tap(&self, cursor: u8, delay: f32) -> f32 {
        let whole = delay as u8;
        let i = cursor.wrapping_sub(whole);
        let a = self.ring[usize::from(i)];
        let b = self.ring[usize::from(i.wrapping_sub(1))];
        let delta = i32::from(b) - i32::from(a);
        f32::from(a) + (delay - f32::from(whole)) * delta as f32
    }

    #[inline]
    #[allow(clippy::cast_possible_truncation)]
    fn write<const MKS: bool>(&mut self, input: f32) {
        self.filtered += self.pole * (input - self.filtered);
        let sample = if MKS {
            if input == 0.0 && self.filtered.abs() < 1e-17 {
                self.filtered = 0.0;
            }
            self.color.write(self.filtered)
        } else {
            self.filtered
        };
        self.ring[usize::from(self.cursor)] = (sample * 32767.0) as i16;
        self.cursor = self.cursor.wrapping_add(1);
        self.phase = self.phase.wrapping_add(self.step);
    }

    #[inline]
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_precision_loss,
        reason = "Signed triangle is in -16384..16384"
    )]
    fn wet_frame<const MKS: bool>(&mut self, input: f32) -> [f32; 2] {
        let cursor = self.cursor;
        let hi = (self.phase >> 16) as u16;
        let folded = if hi < 32768 { hi } else { !hi };
        let sweep = (i32::from(folded) - 16384) as f32 * self.sweep_scale;
        self.write::<MKS>(input);
        let wet = if MKS {
            // The two clocks sweep in opposite directions. A fixed number of BBD
            // stages means delay varies inversely with clock frequency.
            let common = self.center / (1.0 - sweep * sweep);
            let left = self.tap(cursor, common * (1.0 + sweep));
            let right = self.tap(cursor, common * (1.0 - sweep));
            self.color.reconstruct(left, right)
        } else {
            [
                self.tap(cursor, self.center * (1.0 + sweep)),
                self.tap(cursor, self.center * (1.0 - sweep)),
            ]
        };
        let dry = self.dry * input;
        wet.map(|sample| dry + self.wet * sample)
    }

    /// Process one normalized mono sample and return left and right.
    pub fn stereo_frame(&mut self, input: f32) -> [f32; 2] {
        self.silent = false;
        if self.mks50 {
            let wet = self.wet_frame::<true>(input);
            (if self.enabled && self.wet != 0.0 {
                wet
            } else {
                [input; 2]
            })
            .map(pcm_range)
        } else if self.enabled && self.wet != 0.0 {
            self.wet_frame::<false>(input)
        } else {
            self.write::<false>(input);
            [input; 2]
        }
    }

    /// Fold the stereo chorus into a mono output for the common length of both slices.
    pub fn mono_block<S: FromSample<f32>>(&mut self, input: &[f32], output: &mut [S]) {
        self.silent = false;
        self.frames(input.iter().copied().zip(output), mono);
    }

    /// Render the stereo chorus as (L + R)/2 and (L − R)/2 to the common slice length.
    pub fn mid_side_block<S: FromSample<f32>>(
        &mut self,
        input: &[f32],
        mid: &mut [S],
        side: &mut [S],
    ) {
        self.silent = false;
        self.frames(
            input.iter().copied().zip(mid.iter_mut().zip(side)),
            mid_side,
        );
    }

    /// Render the chorus tail after the dry input has fallen silent.
    pub fn mono_silence<S: FromSample<f32>>(&mut self, output: &mut [S]) {
        if self.skip_silence(output.len()) {
            output.fill_with(|| S::from_sample_(0.0));
        } else {
            self.frames(core::iter::repeat(0.0).zip(output), mono);
        }
    }

    /// Render both mid and side tails after the dry input has fallen silent.
    pub fn mid_side_silence<S: FromSample<f32>>(&mut self, mid: &mut [S], side: &mut [S]) {
        let len = mid.len().min(side.len());
        if self.skip_silence(len) {
            mid[..len].fill_with(|| S::from_sample_(0.0));
            side[..len].fill_with(|| S::from_sample_(0.0));
        } else {
            self.frames(
                core::iter::repeat(0.0).zip(mid.iter_mut().zip(side)),
                mid_side,
            );
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "Both clocks wrap at their native word size"
    )]
    fn skip_silence(&mut self, len: usize) -> bool {
        if len == 0 {
            return true;
        }
        if !self.silent {
            // A legacy one-pole can settle on a nonzero subnormal. Preserve it.
            let next = self.filtered + self.pole * (0.0 - self.filtered);
            self.silent = self.filtered.abs() < 1.0 / 32768.0
                && next == self.filtered
                && self.center.is_finite()
                && (!self.mks50 || (self.filtered == 0.0 && self.color.silent()))
                && self.ring.iter().all(|&sample| sample == 0);
            if self.silent {
                self.filtered = next;
            }
        }
        if self.silent {
            self.cursor = self.cursor.wrapping_add(len as u8);
            self.phase = self.phase.wrapping_add(self.step.wrapping_mul(len as u32));
        }
        self.silent
    }

    #[inline]
    fn frames<T>(
        &mut self,
        frames: impl Iterator<Item = (f32, T)>,
        mut emit: impl FnMut(T, [f32; 2]),
    ) {
        let wet = self.enabled && self.wet != 0.0;
        if self.mks50 {
            for (input, out) in frames {
                let lr = self.wet_frame::<true>(input);
                emit(out, (if wet { lr } else { [input; 2] }).map(pcm_range));
            }
        } else if wet {
            for (input, out) in frames {
                emit(out, self.wet_frame::<false>(input));
            }
        } else {
            for (input, out) in frames {
                self.write::<false>(input);
                emit(out, [input; 2]);
            }
        }
    }

    pub(super) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Fold the stereo returns to one signed Q23 channel strip sample.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "Q23 samples fit exactly in f32 and the mixed result is clamped"
    )]
    pub fn tick(&mut self, input: I24) -> I24 {
        let [left, right] = self.stereo_frame(input.inner() as f32 / 8_388_608.0);
        super::pcm(((left + right) * 4_194_304.0) as i32)
    }
}

#[inline]
fn pcm_range(x: f32) -> f32 {
    x.clamp(-1.0, PCM_MAX)
}

#[inline]
fn mono<S: FromSample<f32>>(out: &mut S, [l, r]: [f32; 2]) {
    *out = S::from_sample_((l + r) * 0.5);
}

#[inline]
fn mid_side<S: FromSample<f32>>((mid, side): (&mut S, &mut S), [l, r]: [f32; 2]) {
    *mid = S::from_sample_((l + r) * 0.5);
    *side = S::from_sample_(pcm_range((l - r) * 0.5));
}
