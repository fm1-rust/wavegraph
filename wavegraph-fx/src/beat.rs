//! Musical time shared by tempo-synchronized effects.

/// A duration in quarter-note beats. The ordered detents are shared by Beat FX.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BeatDivision(u8);

// The common denominator covers straight, dotted and triplet values exactly.
const DIVISIONS: [(&str, u8); 13] = [
    ("1/16", 3),
    ("1/8", 6),
    ("1/6", 8),
    ("1/4", 12),
    ("1/3", 16),
    ("1/2", 24),
    ("2/3", 32),
    ("3/4", 36),
    ("1", 48),
    ("3/2", 72),
    ("2", 96),
    ("3", 144),
    ("4", 192),
];

impl BeatDivision {
    /// Select an exact supported beat ratio. Equivalent fractions compare equally.
    #[must_use]
    #[allow(clippy::arithmetic_side_effects, reason = "Two u8 factors fit u16")]
    pub fn from_ratio(numerator: u8, denominator: u8) -> Option<Self> {
        if denominator == 0 {
            return None;
        }
        DIVISIONS
            .iter()
            .position(|&(_, ticks)| {
                u16::from(ticks) * u16::from(denominator) == 48 * u16::from(numerator)
            })
            .and_then(|i| u8::try_from(i).ok())
            .map(Self)
    }

    /// A dotted eighth note, three quarters of a beat.
    pub const DOTTED_EIGHTH: Self = Self(7);
    /// One quarter-note beat.
    pub const ONE: Self = Self(8);

    /// Turn the shared BEATS control. Both ends of the range saturate.
    #[must_use]
    pub fn turn(self, detents: i8) -> Self {
        Self(self.0.saturating_add_signed(detents).min(12))
    }

    /// Text for the BEATS readout, independent of tempo and effect.
    #[must_use]
    pub fn label(self) -> &'static str {
        DIVISIONS[usize::from(self.0)].0
    }

    /// Repeat interval in audio frames, rounded once. Zero BPM or rate has no interval.
    #[must_use]
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "At 48 ticks per beat, the products fit u64 and the divisor is checked"
    )]
    pub fn frames(self, bpm: u16, rate: (u32, u32)) -> Option<u64> {
        if bpm == 0 || rate.0 == 0 || rate.1 == 0 {
            return None;
        }
        let ticks = u64::from(DIVISIONS[usize::from(self.0)].1);
        let denominator = 48 * u64::from(bpm) * u64::from(rate.1);
        Some((u64::from(rate.0) * 60 * ticks + denominator / 2) / denominator)
    }
}

/// Sixteen on/off cells. Bit zero is the first step of the repeating pattern.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepPattern(u16);

impl StepPattern {
    /// Enable every step for a continuous effect.
    pub const ALL: Self = Self(u16::MAX);

    /// Pass the first half of the cycle and rest for the second half.
    pub const FIRST_HALF: Self = Self(0x00ff);

    /// Build a pattern from a pass/rest bit mask, least significant bit first.
    #[must_use]
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// Whether the effect is active at this step of the pattern.
    #[must_use]
    pub fn active(self, step: u8) -> bool {
        self.0 & (1 << (step & 15)) != 0
    }

    /// Toggle one cell of the pattern.
    pub fn toggle(&mut self, step: u8) {
        self.0 ^= 1 << (step & 15);
    }
}

/// Master tempo and the single musical duration selected for Beat FX.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BeatClock {
    /// Number of quarter-note beats in one effect cycle.
    pub beats: BeatDivision,
    /// Tempo in quarter notes per minute.
    pub bpm: u16,
}

impl BeatClock {
    /// Phase of the effect cycle at a transport position. The whole quarter-note
    /// count may wrap at any multiple of twelve beats for the supported divisions.
    #[must_use]
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "Twelve Q32 beats times 48 fit u64; the result wraps at one turn"
    )]
    pub fn phase(self, whole_beats: u32, fraction: u32) -> u32 {
        let position = (u64::from(whole_beats % 12) << 32) | u64::from(fraction);
        (position * 48 / u64::from(DIVISIONS[usize::from(self.beats.0)].1)) as u32
    }

    /// Build a control message for a sample-clocked LFO. A missing phase keeps
    /// the current sweep running, including when the sequencer is stopped.
    #[must_use]
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A u16 BPM times 48 in Q32 fits u64 and the divisor is nonzero"
    )]
    pub fn lfo(self, rate_hz: u32, phase: Option<u32>) -> wavegraph_lfo::PhaseControl {
        let divisor =
            60 * u64::from(rate_hz.max(1)) * u64::from(DIVISIONS[usize::from(self.beats.0)].1);
        let increment = (((u64::from(self.bpm) * 48) << 32) + divisor / 2) / divisor;
        wavegraph_lfo::PhaseControl {
            increment: u32::try_from(increment).unwrap_or(u32::MAX),
            phase,
        }
    }
}

/// A processor that can follow the shared Beat FX clock. Supply phase at the
/// first audio frame of a span for a phase-locked sweep. Echo uses duration alone.
pub trait BeatFx: wavegraph_core::Process<crate::Message, wavegraph_sample::I24> {
    /// Apply transport timing without clearing the processor's signal history.
    fn sync(&mut self, clock: BeatClock, phase: Option<u32>);
}
