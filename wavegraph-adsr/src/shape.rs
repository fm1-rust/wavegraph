//! Compact patch storage for the shared envelope generator.

use super::{Adsr, TIMES_US};

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

    /// Move all four envelope controls by signed encoder detents.
    pub fn turn(&mut self, [attack, decay, sustain, release]: [i8; 4]) {
        self.attack = self.attack.turn(attack);
        self.decay = self.decay.turn(decay);
        self.sustain_percent = self.sustain_percent.saturating_add_signed(sustain).min(100);
        self.release = self.release.turn(release);
    }
}
