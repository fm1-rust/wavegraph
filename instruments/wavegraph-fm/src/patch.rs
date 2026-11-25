use wavegraph_adsr::{Adsr, TIMES_US};

/// Frequency of the modulator relative to the pitched carrier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ratio {
    /// One octave below the carrier.
    Half,
    /// Same frequency as the carrier.
    One,
    /// Inharmonic bell ratio, approximately √2.
    RootTwo,
    /// One octave above.
    Two,
    /// Third harmonic.
    Three,
    /// Two octaves above.
    Four,
    /// Seventh harmonic.
    Seven,
}

impl Ratio {
    /// Panel order, from lowest to highest.
    pub const ALL: [Self; 7] = [
        Self::Half,
        Self::One,
        Self::RootTwo,
        Self::Two,
        Self::Three,
        Self::Four,
        Self::Seven,
    ];

    /// Short decimal ratio for small displays.
    #[must_use]
    pub const fn label(self) -> &'static str {
        ["0.50", "1.00", "1.41", "2.00", "3.00", "4.00", "7.00"][self as usize]
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A Q12 ratio up to seven times a u32 fits in u64."
    )]
    pub(super) fn step(self, carrier: u32) -> u32 {
        let ratio: u16 = [2048, 4096, 5793, 8192, 12288, 16384, 28672][self as usize];
        let step = (u64::from(carrier) * u64::from(ratio)) >> 12;
        u32::try_from(step)
            .ok()
            .filter(|&s| s < 1 << 31)
            .unwrap_or(0)
    }
}

/// One of WaveGraph's envelope times, from zero to ten seconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Time(u8);

impl Time {
    const MAX: Self = Self::from_us(u32::MAX);

    /// Choose the first supported time at least as long as requested, clamping to ten seconds.
    #[must_use]
    pub const fn from_us(us: u32) -> Self {
        let mut i = 0u8;
        while (i as usize) < TIMES_US.len().saturating_sub(1) && TIMES_US[i as usize] < us {
            i = i.saturating_add(1);
        }
        Self(i)
    }

    /// Segment duration in microseconds.
    #[must_use]
    pub const fn micros(self) -> u32 {
        TIMES_US[self.0 as usize]
    }

    /// Move by encoder detents, stopping at the endpoints.
    #[must_use]
    pub fn turn(self, delta: i8) -> Self {
        Self(self.0.saturating_add_signed(delta).min(Self::MAX.0))
    }
}

/// Compact ADSR shared by the keys in a patch. Each sounding note runs its own copy of the contour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Shape {
    /// Time to the peak after key-on.
    pub attack: Time,
    /// Time from the peak to the sustain level.
    pub decay: Time,
    /// Held level, clamped to 100 percent.
    pub sustain_percent: u8,
    /// Time from key-off to zero.
    pub release: Time,
}

impl Shape {
    /// Build a shape from microsecond durations and a sustain percentage.
    #[must_use]
    pub const fn new(attack_us: u32, decay_us: u32, sustain_percent: u8, release_us: u32) -> Self {
        Self {
            attack: Time::from_us(attack_us),
            decay: Time::from_us(decay_us),
            sustain_percent,
            release: Time::from_us(release_us),
        }
    }

    /// Expand the time steps for the sample-clocked envelope generator.
    #[must_use]
    pub const fn adsr(self) -> Adsr {
        Adsr {
            attack_us: self.attack.micros(),
            decay_us: self.decay.micros(),
            sustain_percent: self.sustain_percent,
            release_us: self.release.micros(),
        }
    }
}

/// Sound parameters shared by the four voices. Parameter edits retain oscillator phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Patch {
    /// Frequency ratio of operator 2 to operator 1.
    pub ratio: Ratio,
    /// Peak modulation index, 0–127 (zero to five radians at full velocity).
    pub index: u8,
    /// One-sample feedback around the modulator, 0–127.
    pub feedback: u8,
    /// Operator 1 output level, which controls loudness.
    pub carrier: Shape,
    /// Operator 2 output level, which controls modulation depth.
    pub modulator: Shape,
    /// Pitch contour common to both operators, triggered independently by each note.
    pub pitch: Shape,
    /// Pitch at the contour peak relative to the played key, clamped to ±1200 cents.
    pub pitch_cents: i16,
}

impl Patch {
    /// Velocity-sensitive electric piano with an octave-up tine.
    pub const TINE: Self = Self {
        ratio: Ratio::Two,
        index: 62,
        feedback: 10,
        carrier: Shape::new(3_000, 1_600_000, 24, 200_000),
        modulator: Shape::new(0, 500_000, 8, 160_000),
        pitch: Shape::new(0, 32_000, 0, 20_000),
        pitch_cents: 10,
    };

    /// Inharmonic struck bell with a long release.
    pub const BELL: Self = Self {
        ratio: Ratio::RootTwo,
        index: 106,
        feedback: 6,
        carrier: Shape::new(2_000, 2_500_000, 0, 1_250_000),
        modulator: Shape::new(0, 1_600_000, 8, 800_000),
        pitch: Shape::new(0, 80_000, 0, 80_000),
        pitch_cents: 0,
    };

    /// Short bass with a unison modulator, feedback and a two-semitone pitch drop.
    pub const BASS: Self = Self {
        ratio: Ratio::One,
        index: 85,
        feedback: 78,
        carrier: Shape::new(2_000, 320_000, 62, 64_000),
        modulator: Shape::new(0, 160_000, 14, 50_000),
        pitch: Shape::new(0, 40_000, 0, 20_000),
        pitch_cents: 200,
    };

    /// Original factory sounds in program order.
    pub const PRESETS: [(&'static str, Self); 3] = [
        ("TINE EP", Self::TINE),
        ("GLASS BELL", Self::BELL),
        ("WIRE BASS", Self::BASS),
    ];
}

// Peak frequency ratio minus one. Interpolating the semitone table is within one cent.
pub(super) fn pitch_span(cents: i16) -> f32 {
    const RATIOS: [f32; 13] = [
        1.0,
        1.059463,
        1.122462,
        1.189207,
        1.259921,
        1.334_84,
        core::f32::consts::SQRT_2,
        1.498307,
        1.587401,
        1.681793,
        1.781797,
        1.887749,
        2.0,
    ];
    let n = cents.unsigned_abs().min(1200);
    let i = usize::from(n / 100);
    let lower = RATIOS[i];
    let upper = RATIOS[i.saturating_add(1).min(12)];
    let ratio = lower + (upper - lower) * f32::from(n % 100) * 0.01;
    if cents < 0 {
        1.0 / ratio - 1.0
    } else {
        ratio - 1.0
    }
}
