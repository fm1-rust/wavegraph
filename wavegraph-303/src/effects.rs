use crate::{
    Drive,
    dsp::{BitCrusher, fast_sin, fast_tanh, tau_to_coef},
};
use core::f32::consts::PI;

/// A pair of left and right audio samples.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StereoFrame {
    /// Left channel.
    pub left: f32,
    /// Right channel.
    pub right: f32,
}

/// A mono input, optional send, and stereo output must have the same frame count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LengthMismatch;

impl core::fmt::Display for LengthMismatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("audio buffers have different frame counts")
    }
}
impl core::error::Error for LengthMismatch {}

/// Controls for the bass effects rack. Amounts range from zero to one.
#[derive(Clone, Copy, Debug)]
pub struct EffectsSettings {
    /// Post-VCA drive.
    pub drive: f32,
    /// Bit-depth reduction.
    pub crush_bits: f32,
    /// Sample-rate reduction.
    pub crush_rate: f32,
    /// Chorus in the lower half, flanger in the upper half.
    pub modulation: f32,
    /// Stereo modulation width.
    pub mod_width: f32,
    /// Delay mix. The lower half increases the send, the upper half fades the dry signal.
    pub delay_mix: f32,
    /// Bass level in the shared delay input.
    pub delay_send: f32,
    /// Tempo-synchronized delay division, indexed from 0 to 8.
    pub delay_division: usize,
    /// Select free delay time in milliseconds instead of tempo synchronization.
    pub delay_free: bool,
    /// Free delay time in milliseconds.
    pub delay_ms: f32,
    /// Delay feedback.
    pub delay_feedback: f32,
    /// Tape tone, dark to bright.
    pub delay_tone: f32,
    /// Tempo in beats per minute.
    pub bpm: f32,
}

impl Default for EffectsSettings {
    fn default() -> Self {
        Self {
            drive: 0.0,
            crush_bits: 0.0,
            crush_rate: 0.0,
            modulation: 0.0,
            mod_width: 0.5,
            delay_mix: 0.0,
            delay_send: 1.0,
            delay_division: 5,
            delay_free: false,
            delay_ms: 375.0,
            delay_feedback: 0.35,
            delay_tone: 0.4,
            bpm: 120.0,
        }
    }
}

/// Drive, bit crusher, chorus/flanger and ping-pong tape delay.
/// The delay borrows caller-owned storage for the lifetime of the rack.
#[derive(Debug)]
pub struct Effects<'a> {
    rate: f32,
    settings: EffectsSettings,
    drive: Drive,
    crusher: BitCrusher,
    modulation: Modulation,
    delay: Delay<'a>,
    bad: u32,
}

impl<'a> Effects<'a> {
    /// Shortest storage that fits the 64-frame tap and upstream's 400-frame margin.
    pub const MIN_DELAY_FRAMES: usize = 464;

    /// Initialize the rack and clear its delay. Returns `None` for an invalid
    /// sample rate or a buffer shorter than [`Self::MIN_DELAY_FRAMES`].
    pub fn new(rate: f32, storage: &'a mut [StereoFrame]) -> Option<Self> {
        if storage.len() < Self::MIN_DELAY_FRAMES || !rate.is_finite() || rate <= 0.0 {
            return None;
        }
        storage.fill(StereoFrame::default());
        Some(Self {
            rate,
            settings: EffectsSettings::default(),
            drive: Drive::new(rate),
            crusher: BitCrusher::default(),
            modulation: Modulation {
                buf: [0.0; 2048],
                pos: 0,
                phase: 0.0,
            },
            delay: Delay {
                buf: storage,
                pos: 0,
                target: 12000.0,
                a: 12000.0,
                b: 12000.0,
                fade: 0.0,
                fading: false,
                heads_set: false,
                tail: 0,
                wow: 0.0,
                flutter: 0.0,
                lp: StereoFrame::default(),
                hp: StereoFrame::default(),
            },
            bad: 0,
        })
    }

    /// Apply panel settings. The first setting positions both delay heads.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_precision_loss,
        reason = "Construction reserves the 400-frame margin; CHOMPI computes tap positions in f32."
    )]
    pub fn set(&mut self, settings: &EffectsSettings) {
        const BEATS: [f32; 9] = [
            1.0 / 6.0,
            0.25,
            1.0 / 3.0,
            0.5,
            2.0 / 3.0,
            0.75,
            1.0,
            1.5,
            2.0,
        ];
        self.settings = *settings;
        let seconds = if settings.delay_free {
            settings.delay_ms.clamp(30.0, 1900.0) * 0.001
        } else {
            BEATS[settings.delay_division.min(8)] * 60.0 / settings.bpm
        };
        self.delay.target = (seconds * self.rate).clamp(64.0, (self.delay.buf.len() - 400) as f32);
        if !self.delay.heads_set {
            self.delay.a = self.delay.target;
            self.delay.b = self.delay.target;
            self.delay.heads_set = true;
        }
    }

    /// Count of invalid values caught by the delay or block-end recovery.
    #[must_use]
    pub fn bad_count(&self) -> u32 {
        self.bad
    }

    /// Write stereo output from the bass and an optional mono drum delay send.
    ///
    /// # Errors
    /// Returns [`LengthMismatch`] before processing if the block lengths differ.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The finite positive rate gives a nonnegative delay-tail frame count."
    )]
    pub fn process(
        &mut self,
        bass: &[f32],
        send: Option<&[f32]>,
        output: &mut [StereoFrame],
    ) -> Result<(), LengthMismatch> {
        if bass.len() != output.len() || send.is_some_and(|s| s.len() != bass.len()) {
            return Err(LengthMismatch);
        }
        let s = &self.settings;
        let drive = self.drive.coefficients(s.drive);
        let crush = BitCrusher::settings(s.crush_bits, s.crush_rate);
        let modulation = (s.modulation > 0.005).then(|| ModSettings::new(s, self.rate));
        let sending = s.delay_mix > 0.005 || send.is_some();
        let delay = (sending || self.delay.tail > 0).then(|| DelaySettings::new(s, self.rate));
        for (i, (&sample, frame)) in bass.iter().zip(output).enumerate() {
            let x = drive
                .as_ref()
                .map_or(sample, |c| self.drive.sample(sample, c));
            let x = self.crusher.sample(x, &crush);
            *frame = modulation
                .as_ref()
                .map_or(StereoFrame { left: x, right: x }, |c| {
                    self.modulation.sample(x, c)
                });
            if let Some(c) = &delay {
                self.delay
                    .sample(frame, send.map_or(0.0, |s| s[i]), c, &mut self.bad);
                if sending {
                    self.delay.tail = (self.rate * 6.0) as u32;
                } else {
                    self.delay.tail = self.delay.tail.saturating_sub(1);
                }
            }
        }
        let level = self.delay.lp.left.abs()
            + self.delay.lp.right.abs()
            + self.delay.hp.left.abs()
            + self.delay.hp.right.abs();
        let level = self.drive.state_level(level)
            + self.crusher.held.abs()
            + self.modulation.buf[self.modulation.pos].abs();
        if level >= 1e6 || level.is_nan() {
            self.delay.lp = StereoFrame::default();
            self.delay.hp = StereoFrame::default();
            self.drive.clear();
            self.crusher.held = 0.0;
            self.modulation.buf.fill(0.0);
            self.bad = self.bad.wrapping_add(1);
        }
        Ok(())
    }
}

struct ModSettings {
    base: f32,
    depth: f32,
    inc: f32,
    phase_r: f32,
    feedback: f32,
    wet: f32,
}
impl ModSettings {
    fn new(s: &EffectsSettings, sr: f32) -> Self {
        let w = s.mod_width;
        let (base, depth, hz, feedback, wet) = if s.modulation < 0.5 {
            (
                8.0,
                2.0 + 3.0 * w,
                0.6,
                0.0,
                0.4 + 0.3 * (s.modulation * 2.0),
            )
        } else {
            (
                1.5,
                1.0 + w,
                0.18,
                0.3 + 0.5 * ((s.modulation - 0.5) * 2.0),
                0.7,
            )
        };
        Self {
            base: base * 0.001 * sr,
            depth: depth * 0.001 * sr,
            inc: hz / sr,
            phase_r: 0.5 * w,
            feedback,
            wet,
        }
    }
}

#[derive(Debug)]
struct Modulation {
    buf: [f32; 2048],
    pos: usize,
    phase: f32,
}
impl Modulation {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The write index is always inside the fixed 2048-frame ring."
    )]
    fn sample(&mut self, x: f32, c: &ModSettings) -> StereoFrame {
        self.phase += c.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let mut right = self.phase + c.phase_r;
        if right >= 1.0 {
            right -= 1.0;
        }
        let tap = |phase| {
            let delay = c.base + c.depth * (0.5 + 0.5 * fast_sin(phase));
            let (a, b, f) = tap_indices(self.pos, delay, self.buf.len());
            self.buf[a] + (self.buf[b] - self.buf[a]) * f
        };
        let (l, r) = (tap(self.phase), tap(right));
        self.buf[self.pos] = x + c.feedback * 0.5 * (l + r);
        self.pos = (self.pos + 1) % self.buf.len();
        let gain = 1.0 / (1.0 + 0.5 * c.wet);
        StereoFrame {
            left: (x + c.wet * l) * gain,
            right: (x + c.wet * r) * gain,
        }
    }
}

// Subtract the fractional delay from the write position before truncating.
// Reversing the interpolation changes rounding at ring boundaries.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects,
    reason = "Fractional taps use f32 frame positions; both ring buffers are nonempty slices."
)]
fn tap_indices(cursor: usize, delay: f32, len: usize) -> (usize, usize, f32) {
    let len_f = len as f32;
    let mut pos = cursor as f32 - delay;
    if pos < 0.0 {
        pos += len_f;
        if pos < 0.0 {
            pos -= crate::dsp::floor(pos / len_f) * len_f;
        }
    }
    let i = pos as usize;
    let a = i % len;
    (a, (a + 1) % len, pos - i as f32)
}

#[derive(Debug)]
struct Delay<'a> {
    buf: &'a mut [StereoFrame],
    pos: usize,
    target: f32,
    a: f32,
    b: f32,
    fade: f32,
    fading: bool,
    heads_set: bool,
    tail: u32,
    wow: f32,
    flutter: f32,
    lp: StereoFrame,
    hp: StereoFrame,
}

struct DelaySettings {
    rate: f32,
    dry: f32,
    wet: f32,
    send: f32,
    feedback: f32,
    lp: f32,
    hp: f32,
    glide: f32,
    fade_in: f32,
    free: bool,
}
impl DelaySettings {
    fn new(s: &EffectsSettings, rate: f32) -> Self {
        let mix = s.delay_mix.clamp(0.0, 1.0);
        Self {
            rate,
            dry: if mix > 0.5 { 2.0 * (1.0 - mix) } else { 1.0 },
            wet: if mix < 0.5 { 2.0 * mix } else { 1.0 },
            send: s.delay_send,
            feedback: 1.05 * s.delay_feedback,
            lp: tau_to_coef(
                1.0 / (2.0 * PI * (900.0 + 6000.0 * s.delay_tone * s.delay_tone)),
                rate,
            ),
            hp: tau_to_coef(1.0 / (2.0 * PI * 90.0), rate),
            glide: tau_to_coef(if s.delay_free { 0.15 } else { 0.05 }, rate),
            fade_in: 1.0 / (0.08 * rate),
            free: s.delay_free,
        }
    }
}

impl Delay<'_> {
    fn tap(&self, delay: f32) -> StereoFrame {
        let (a, b, f) = tap_indices(self.pos, delay, self.buf.len());
        let (a, b) = (self.buf[a], self.buf[b]);
        StereoFrame {
            left: a.left + (b.left - a.left) * f,
            right: a.right + (b.right - a.right) * f,
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The nonempty slice bounds the write index before it advances."
    )]
    fn sample(&mut self, frame: &mut StereoFrame, send: f32, c: &DelaySettings, bad: &mut u32) {
        if self.fading {
            self.fade += c.fade_in;
            if self.fade >= 1.0 {
                self.a = self.b;
                self.fade = 0.0;
                self.fading = false;
            }
        } else if (self.target - self.a).abs() > 0.03 * self.a && !c.free {
            self.b = self.target;
            self.fade = 0.0;
            self.fading = true;
        } else {
            self.a += ((self.target - self.a) * c.glide).clamp(-0.33, 0.33);
        }
        self.wow += 0.55 / c.rate;
        if self.wow >= 1.0 {
            self.wow -= 1.0;
        }
        self.flutter += 6.5 / c.rate;
        if self.flutter >= 1.0 {
            self.flutter -= 1.0;
        }
        let wobble = c.rate * (0.0012 * fast_sin(self.wow) + 0.00015 * fast_sin(self.flutter));
        let mut d = self.tap(self.a + wobble);
        let level = d.left.abs() + d.right.abs();
        if level >= 100.0 || level.is_nan() {
            d = StereoFrame::default();
            *bad = bad.wrapping_add(1);
        }
        if self.fading {
            let b = self.tap(self.b + wobble);
            let gb = fast_sin(0.25 * self.fade);
            let ga = fast_sin(0.25 - 0.25 * self.fade);
            d.left = d.left * ga + b.left * gb;
            d.right = d.right * ga + b.right * gb;
        }

        let input = c.send * c.wet * (frame.left + frame.right) * 0.5 + send;
        self.lp.left += (input + c.feedback * d.right - self.lp.left) * c.lp;
        self.lp.right += (c.feedback * d.left - self.lp.right) * c.lp;
        self.hp.left += (self.lp.left - self.hp.left) * c.hp;
        self.hp.right += (self.lp.right - self.hp.right) * c.hp;
        self.buf[self.pos] = StereoFrame {
            left: fast_tanh(self.lp.left - self.hp.left),
            right: fast_tanh(self.lp.right - self.hp.right),
        };
        self.pos = (self.pos + 1) % self.buf.len();
        frame.left = frame.left * c.dry + d.left;
        frame.right = frame.right * c.dry + d.right;
    }
}
