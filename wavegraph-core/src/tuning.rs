/// Equal-tempered phase increments and sample period for a device's clock.
/// One table can be shared by all voices running at the same rate.
#[derive(Debug)]
pub struct Tuning {
    steps: [u32; 128],
    sample_period: f32,
}

impl Tuning {
    /// Construct a table for `numerator / denominator` frames per second,
    /// with MIDI note 69 at 440 Hz. Returns `None` for a zero clock component.
    /// Pitches at or above Nyquist are silent.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The phase increment is positive and at most 2^31. The nonzero u32 clock ratio fits in f32."
    )]
    pub const fn new(numerator: u32, denominator: u32) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }
        let mut steps = [0; 128];
        // MIDI note 0, in cycles of a 32-bit phase accumulator per frame.
        let mut step =
            8.175_798_915_643_707 * 4_294_967_296.0 * denominator as f64 / numerator as f64;
        let mut note: usize = 0;
        while note < steps.len() {
            if step < 2_147_483_648.0 {
                steps[note] = (step + 0.5) as u32;
            }
            step *= 1.059_463_094_359_295_3;
            note = note.wrapping_add(1);
        }
        Some(Self {
            steps,
            sample_period: (denominator as f64 / numerator as f64) as f32,
        })
    }

    /// Phase advance per output frame in units of 2⁻³² turns. Returns zero
    /// for notes outside the MIDI range or at or above the output Nyquist limit.
    #[must_use]
    pub const fn phase_step(&self, note: u8) -> u32 {
        if (note as usize) < self.steps.len() {
            self.steps[note as usize]
        } else {
            0
        }
    }

    /// Duration of one output frame in seconds.
    #[must_use]
    pub const fn sample_period(&self) -> f32 {
        self.sample_period
    }
}
