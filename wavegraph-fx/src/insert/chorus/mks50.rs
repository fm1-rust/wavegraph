//! Wet-path approximation of the two MN3009 circuits on page 11 of the MKS-50 service notes.
//! The input network has a dominant 872 Hz pole (R54/C43 plus loading). The two
//! biquads approximate the remaining input and reconstruction networks at the FM-1 clock.

#[derive(Clone, Copy, Debug, Default)]
struct LowPass {
    b0: f32,
    a1: f32,
    a2: f32,
}

impl LowPass {
    fn new(hz: f32, q: f32, period: f32) -> Self {
        let k = libm::tanf(core::f32::consts::PI * (hz * period).clamp(0.00001, 0.45));
        let k2 = k * k;
        let norm = 1.0 / (1.0 + k / q + k2);
        Self {
            b0: k2 * norm,
            a1: 2.0 * (k2 - 1.0) * norm,
            a2: (1.0 - k / q + k2) * norm,
        }
    }

    #[inline]
    fn sample(self, memory: &mut [f32; 2], x: f32) -> f32 {
        let feed = self.b0 * x;
        let y = feed + memory[0];
        *memory = [2.0 * feed - self.a1 * y + memory[1], feed - self.a2 * y];
        // A silent analog return has a finite noise floor. Avoid subnormal IIR
        // limit cycles far below one Q23 LSB, including in scalar rendering.
        if x == 0.0 && memory[0].abs() < 1e-17 && memory[1].abs() < 1e-17 {
            *memory = [0.0; 2];
        }
        y
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct WetPath {
    input: LowPass,
    output: LowPass,
    pre: [f32; 2],
    post: [[f32; 2]; 2],
}

impl WetPath {
    pub(super) fn new(period: f32) -> Self {
        Self {
            input: LowPass::new(9950.0, 0.565, period),
            output: LowPass::new(9000.0, 1.865, period),
            ..Self::default()
        }
    }

    #[inline]
    pub(super) fn write(&mut self, sample: f32) -> f32 {
        self.input.sample(&mut self.pre, sample)
    }

    #[inline]
    pub(super) fn reconstruct(&mut self, left: f32, right: f32) -> [f32; 2] {
        [
            self.output.sample(&mut self.post[0], left),
            self.output.sample(&mut self.post[1], right),
        ]
    }

    pub(super) fn silent(&self) -> bool {
        self.pre == [0.0; 2] && self.post == [[0.0; 2]; 2]
    }
}

// Measured Alpha-Juno cycle rates (millihertz), Eokuwwy, June 2021.
// Intermediate panel positions interpolate between the measured anchors.
pub(super) fn rate(value: u8) -> f32 {
    const RATES: [(u8, u16); 14] = [
        (0, 8),
        (16, 19),
        (32, 53),
        (48, 128),
        (54, 169),
        (64, 315),
        (72, 481),
        (80, 781),
        (88, 1183),
        (92, 1449),
        (96, 1923),
        (104, 2857),
        (112, 4878),
        (120, 7353),
    ];
    let mut last = RATES[0];
    for next in RATES.into_iter().skip(1).chain([(127, 10101)]) {
        if value <= next.0 {
            let part =
                f32::from(value.saturating_sub(last.0)) / f32::from(next.0.saturating_sub(last.0));
            return (f32::from(last.1) + part * f32::from(next.1.saturating_sub(last.1))) / 1000.0;
        }
        last = next;
    }
    10.101
}
