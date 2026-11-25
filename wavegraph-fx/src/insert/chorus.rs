//! Two opposing fractional taps on the same band-limited mono input.
//! The short delay memory is shared because both BBD paths receive the same mix.
use wavegraph_sample::I24;

/// Panel controls for the MKS-50-inspired parallel delay paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChorusControls {
    /// Triangle LFO rate, 0..=127, in steps of 0.012 Hz above 0.15 Hz.
    pub rate: u8,
    /// Sweep depth, 0..=128. The Hoover setting is 64.
    pub depth: u8,
    /// Wet-path brightness, 0..=128. The dry path is unaffected.
    pub tone: u8,
    /// Wet signal as a percentage of the output, 0..=100.
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
}

/// Two band-limited, oppositely swept delay taps on one mono input.
#[derive(Clone, Copy, Debug)]
pub struct Chorus {
    ring: [i16; 256],
    cursor: usize,
    phase: u32,
    filtered: f32,
    center: f32,
    pole: f32,
    period: f32,
    step: u32,
    depth: f32,
    wet: f32,
    enabled: bool,
}
impl Chorus {
    /// Construct from the sample period in seconds.
    #[must_use]
    pub fn new(period: f32) -> Self {
        let mut chorus = Self {
            ring: [0; 256],
            cursor: 0,
            phase: 0,
            filtered: 0.0,
            center: (0.0035 / period).clamp(1.0, 185.0),
            period,
            pole: 0.0,
            step: 0,
            depth: 0.0,
            wet: 0.0,
            enabled: true,
        };
        chorus.set_controls(ChorusControls::INITIAL);
        chorus
    }

    /// Change parameters without clearing the delay or resetting the LFO.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn set_controls(&mut self, controls: ChorusControls) {
        let hz = 0.15 + f32::from(controls.rate.min(127)) * 0.012;
        self.step = (hz * self.period * 4294967296.0) as u32;
        // Keep both fractional taps within the 256-frame history at high sample rates.
        self.depth =
            (0.34 * (f32::from(controls.depth.min(128)) / 64.0)).min(254.0 / self.center - 1.0);
        self.pole = ((2_400.0 + 200.0 * f32::from(controls.tone.min(128))) * self.period).min(1.0);
        self.wet = f32::from(controls.mix.min(100)) / 100.0;
        self.enabled = controls.enabled;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn tap(&self, delay: f32) -> f32 {
        let whole = delay as usize;
        let i = self.cursor.wrapping_sub(whole) & 255;
        let a = f32::from(self.ring[i]);
        (a + (delay - whole as f32) * (f32::from(self.ring[i.wrapping_sub(1) & 255]) - a)) / 32768.0
    }
    /// Process one normalized mono sample and return left and right.
    #[allow(clippy::cast_possible_truncation)]
    pub fn stereo_frame(&mut self, input: f32) -> [f32; 2] {
        self.filtered += self.pole * (input - self.filtered);
        self.ring[self.cursor] = (self.filtered * 32767.0) as i16;
        let hi = (self.phase >> 16) as u16;
        let folded = if hi < 32768 { hi } else { !hi };
        let sweep = (f32::from(folded) / 16384.0 - 1.0) * self.depth;
        let out = if self.enabled && self.wet != 0.0 {
            [
                self.tap(self.center * (1.0 + sweep)),
                self.tap(self.center * (1.0 - sweep)),
            ]
            .map(|wet| (1.0 - self.wet) * input + self.wet * wet)
        } else {
            [input; 2]
        };
        self.phase = self.phase.wrapping_add(self.step);
        self.cursor = self.cursor.wrapping_add(1) & 255;
        out
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
