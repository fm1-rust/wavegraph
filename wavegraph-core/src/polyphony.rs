//! Four-voice keyboard modes shared by the organ and wavetable synths.

use crate::midi::Note;

const CHORD_COUNT: u8 = 16;
const CHORDS: [(&str, &[u8]); CHORD_COUNT as usize] = [
    ("Maj", &[0, 4, 7]),
    ("Min", &[0, 3, 7]),
    ("Maj7", &[0, 4, 7, 11]),
    ("m7", &[0, 3, 7, 10]),
    ("7", &[0, 4, 7, 10]),
    ("Sus2", &[0, 2, 7]),
    ("Sus4", &[0, 5, 7]),
    ("Dim", &[0, 3, 6]),
    ("Aug", &[0, 4, 8]),
    ("6", &[0, 4, 7, 9]),
    ("m6", &[0, 3, 7, 9]),
    ("Dim7", &[0, 3, 6, 9]),
    ("m7b5", &[0, 3, 6, 10]),
    ("mMaj7", &[0, 3, 7, 11]),
    ("Add9", &[0, 4, 7, 14]),
    ("mAdd9", &[0, 3, 7, 14]),
];

/// A root-position chord of up to four notes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Chord(u8);

impl Chord {
    /// Major triad.
    pub const MAJOR: Self = Self(0);

    /// Short chord symbol for panel displays.
    #[must_use]
    pub fn name(self) -> &'static str {
        CHORDS[usize::from(self.0)].0
    }

    /// Move through the chord catalog, wrapping at either end.
    #[must_use]
    pub fn turn(self, delta: i8) -> Self {
        let i = i16::from(self.0).saturating_add(i16::from(delta));
        Self(i.rem_euclid(i16::from(CHORD_COUNT)).to_le_bytes()[0])
    }
}

/// Assignment of four oscillators to held notes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VoiceMode {
    /// Each key takes one voice with the instrument's usual stealing policy.
    #[default]
    Poly,
    /// Play a chord from the most recent held key.
    Chord(Chord),
    /// Stack four detuned oscillators on the most recent held key.
    Unison,
}

/// Four most recently held roots, in priority order, with their strike velocities.
#[derive(Clone, Copy, Debug)]
pub struct Keyboard {
    /// Current voice allocation mode.
    pub mode: VoiceMode,
    held: [Option<Note>; 4],
}

impl Keyboard {
    /// Silent polyphonic keyboard.
    pub const EMPTY: Self = Self {
        mode: VoiceMode::Poly,
        held: [None; 4],
    };

    /// Remember a playable nonzero-velocity strike, moving restrikes to the front.
    pub fn press(&mut self, note: Note) {
        let index = self
            .held
            .iter()
            .position(|n| n.is_some_and(|n| n.number == note.number))
            .unwrap_or(3);
        self.held[..=index].rotate_right(1);
        self.held[0] = Some(note);
    }

    /// Forget a root. Returns true if removing it changes the active chord or unison root.
    pub fn release(&mut self, note: u8) -> bool {
        let Some(index) = self
            .held
            .iter()
            .position(|n| n.is_some_and(|n| n.number == note))
        else {
            return false;
        };
        self.held[index..].rotate_left(1);
        self.held[3] = None;
        index == 0
    }

    /// Drop all key ownership while retaining the selected mode.
    pub fn clear(&mut self) {
        self.held.fill(None);
    }

    /// Sounding pitch and velocity in a slot when rebuilding the four voices.
    #[must_use]
    pub fn voice(&self, slot: usize) -> Option<Note> {
        let root = self.held[0]?;
        match self.mode {
            VoiceMode::Poly => self.held.get(slot).copied().flatten(),
            VoiceMode::Unison => (slot < 4).then_some(root),
            VoiceMode::Chord(chord) => {
                let offset = *CHORDS[usize::from(chord.0)].1.get(slot)?;
                let number = root.number.checked_add(offset)?;
                (number < 128).then_some(Note { number, ..root })
            }
        }
    }

    /// Detune the four unison slots approximately −7, −2, +2 and +7 cents.
    /// Return zero for increments reaching Nyquist.
    #[must_use]
    pub fn step(&self, slot: usize, base: u32) -> u32 {
        if self.mode != VoiceMode::Unison {
            return base;
        }
        let step = match slot {
            0 => base.saturating_sub(base >> 8),
            1 => base.saturating_sub(base >> 10),
            2 => base.saturating_add(base >> 10),
            _ => base.saturating_add(base >> 8),
        };
        if step < (1 << 31) { step } else { 0 }
    }
}
