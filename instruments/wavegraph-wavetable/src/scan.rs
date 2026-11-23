/// How a note selects frames over its lifetime. Every mode interpolates
/// samples inside each oscillator cycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// Select the nearest frame to the start position.
    Stepped,
    /// Interpolate between adjacent frames at the start position.
    Smooth,
    /// Travel from start to end once, then hold the last position.
    OneShot,
    /// Travel from start to end and back for as long as the voice sounds.
    PingPong,
}

/// Frame positions are on the MIDI 0–127 scale, spanning the entire bank.
/// An end below the start scans backward. A fresh note starts its own clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Scan {
    /// Frame-selection or motion law.
    pub mode: Mode,
    /// Manual position or beginning of a sweep, clamped to 127.
    pub start: u8,
    /// End of a sweep, clamped to 127.
    pub end: u8,
    /// Milliseconds for a single traversal. Zero takes one audio frame.
    pub travel_ms: u16,
}

impl Scan {
    /// Resonant saw with a directly playable, stepped wave index.
    pub const RESONANT: Self = Self {
        mode: Mode::Stepped,
        start: 36,
        end: 127,
        travel_ms: 1_200,
    };
    /// A slow, smoothly reversing traversal of the vocal recordings.
    pub const VOCAL: Self = Self {
        mode: Mode::PingPong,
        start: 0,
        end: 127,
        travel_ms: 1_600,
    };
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Travel {
    mode: Mode,
    frames: u32,
    start: u16,
    end: u16,
    slope: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Cursor {
    elapsed: u32,
    reverse: bool,
}

impl Travel {
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Normalized positions fit Q8.8; the positive duration rounds and saturates to 1..=u32::MAX."
    )]
    pub(super) fn new(scan: Scan, last: u8, period: f32) -> Self {
        let coord =
            |value: u8| ((u32::from(value.min(127)) * u32::from(last) * 256 + 63) / 127) as u16;
        let start = coord(scan.start);
        let end = coord(scan.end);
        let frames = ((f32::from(scan.travel_ms) * 0.001 / period + 0.5) as u32).max(1);
        Self {
            mode: scan.mode,
            frames,
            start,
            end,
            slope: (f32::from(end) - f32::from(start)) / frames as f32,
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The elapsed clock stays within the endpoints; both positions fit u16."
    )]
    pub(super) fn position(self, cursor: Cursor) -> u16 {
        match self.mode {
            Mode::Stepped => self.start.saturating_add(128) & 0xff00,
            Mode::Smooth => self.start,
            _ if cursor.elapsed >= self.frames => self.end,
            _ => (f32::from(self.start) + self.slope * cursor.elapsed as f32) as u16,
        }
    }

    pub(super) fn advance(self, cursor: &mut Cursor) {
        match self.mode {
            Mode::Stepped | Mode::Smooth => (),
            Mode::OneShot => cursor.elapsed = cursor.elapsed.saturating_add(1).min(self.frames),
            Mode::PingPong => {
                if cursor.elapsed == self.frames {
                    cursor.reverse = true;
                } else if cursor.elapsed == 0 {
                    cursor.reverse = false;
                }
                cursor.elapsed = if cursor.reverse {
                    cursor.elapsed.saturating_sub(1)
                } else {
                    cursor.elapsed.saturating_add(1)
                };
            }
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The old duration is nonzero and the u32 product fits u64."
    )]
    pub(super) fn retime(self, cursor: &mut Cursor, old: Self) {
        if self.frames != old.frames {
            // Both factors are at most u32::MAX and old.frames is nonzero.
            let elapsed =
                u64::from(cursor.elapsed) * u64::from(self.frames) / u64::from(old.frames);
            cursor.elapsed = u32::try_from(elapsed).unwrap_or(self.frames);
        }
        if matches!(self.mode, Mode::Stepped | Mode::Smooth) {
            *cursor = Cursor::default();
        }
    }
}
