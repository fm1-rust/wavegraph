//! MIDI 1.0 voice messages for use as frame-timed [patch events](crate::Event).
//! Routing (a mixer channel, USB cable or graph port) belongs to the sender.

/// A seven-bit note number and strike velocity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Note {
    /// MIDI note number (60 is middle C).
    pub number: u8,
    /// Strike velocity in 1..=127.
    pub velocity: u8,
}

/// The channel voice messages used by WaveGraph instruments and pattern clips.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MidiMessage {
    /// Strike a note.
    On(Note),
    /// Release a note.
    Off(u8),
    /// Set a controller to a seven-bit value.
    Control(u8, u8),
}

impl MidiMessage {
    /// Decode a three-byte note or controller message, normalizing zero-velocity note-ons.
    #[must_use]
    pub const fn decode([status, key, value]: [u8; 3]) -> Option<Self> {
        if key >= 128 || value >= 128 {
            return None;
        }
        match (status >> 4, value) {
            (8, _) | (9, 0) => Some(Self::Off(key)),
            (9, velocity) => Some(Self::On(Note {
                number: key,
                velocity,
            })),
            (11, _) => Some(Self::Control(key, value)),
            _ => None,
        }
    }

    /// Encode on a zero-based MIDI channel. Invalid data or channels return `None`.
    #[must_use]
    pub const fn encode(self, channel: u8) -> Option<[u8; 3]> {
        let (status, key, value) = match self {
            Self::On(Note { velocity: 0, .. }) => return None,
            Self::On(Note { number, velocity }) => (0x90, number, velocity),
            Self::Off(note) => (0x80, note, 64),
            Self::Control(number, value) => (0xb0, number, value),
        };
        if channel < 16 && key < 128 && value < 128 {
            Some([status | channel, key, value])
        } else {
            None
        }
    }
}
