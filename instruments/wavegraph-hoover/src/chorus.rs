//! Two opposing fractional taps on the same band-limited mono input.
//! The short delay memory is shared because both BBD paths receive the same mix.
#[derive(Debug)]
pub(crate) struct Chorus {
    ring: [i16; 256],
    cursor: usize,
    phase: u32,
    filtered: f32,
    center: f32,
    pole: f32,
}
impl Chorus {
    pub(crate) fn new(period: f32) -> Self {
        Self {
            ring: [0; 256],
            cursor: 0,
            phase: 0,
            filtered: 0.0,
            center: (0.0035 / period).clamp(1.0, 185.0),
            pole: (28_000.0 * period).min(1.0),
        }
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
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn sample(&mut self, input: f32, step: u32, enabled: bool) -> [f32; 2] {
        self.filtered += self.pole * (input - self.filtered);
        self.ring[self.cursor] = (self.filtered * 32767.0) as i16;
        let sweep = super::triangle(self.phase) * 0.34;
        let out = if enabled {
            [
                self.tap(self.center * (1.0 + sweep)),
                self.tap(self.center * (1.0 - sweep)),
            ]
            .map(|wet| 0.63 * input + 0.37 * wet)
        } else {
            [input; 2]
        };
        self.phase = self.phase.wrapping_add(step);
        self.cursor = self.cursor.wrapping_add(1) & 255;
        out
    }
}
