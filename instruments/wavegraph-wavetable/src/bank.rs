/// Maximum harmonic and table geometry for one pitch range. Every frame packs
/// these levels in order. Even the highest notes have 32 points per cycle.
#[derive(Clone, Copy, Debug)]
pub(super) struct Level {
    offset: usize,
    bits: u32,
    max_step: u32,
}

impl Level {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Every harmonic limit below is positive."
    )]
    const fn new(offset: usize, bits: u32, harmonics: u32) -> Self {
        Self {
            offset,
            bits,
            max_step: (u32::MAX / 2) / harmonics,
        }
    }

    pub(super) fn for_step(step: u32) -> Self {
        LEVELS
            .into_iter()
            .find(|level| step <= level.max_step)
            .unwrap_or(LEVELS[5])
    }
}

const LEVELS: [Level; 6] = [
    Level::new(0, 8, 63),
    Level::new(256, 7, 31),
    Level::new(384, 6, 15),
    Level::new(448, 5, 7),
    Level::new(480, 5, 3),
    Level::new(512, 5, 1),
];
const FRAME_BYTES: usize = 544 / 2 * 3;

/// A borrowed bank of 1–256 single-cycle frames with six harmonic levels per frame.
#[derive(Clone, Copy, Debug)]
pub struct Bank<'a> {
    frames: &'a [[u8; FRAME_BYTES]],
    pub(super) last: u8,
}

impl<'a> Bank<'a> {
    /// Borrow the packed signed 12-bit PCM produced by `tools/import_bank.py`.
    /// Returns `None` for empty, partial or oversized banks.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, reason = "Validated 1–256 frames.")]
    pub const fn from_packed12(bytes: &'a [u8]) -> Option<Self> {
        let (frames, tail) = bytes.as_chunks::<FRAME_BYTES>();
        if !tail.is_empty() || frames.is_empty() || frames.len() > 256 {
            return None;
        }
        Some(Self {
            frames,
            last: frames.len().saturating_sub(1) as u8,
        })
    }

    // Position is a frame index with eight fractional bits. Interpolate phase
    // across each cycle's seam, and clamp frame lookup at the end of the bank.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Validated frame and level geometry; interpolation products fit i32."
    )]
    pub(super) fn sample(self, level: Level, phase: u32, position: u16) -> i32 {
        let index = usize::from(position >> 8);
        let point = (phase >> (32 - level.bits)) as usize;
        let next = (point + 1) & ((1 << level.bits) - 1);
        let phase_frac = i32::from((phase >> (24 - level.bits)).to_le_bytes()[0]);
        let cycle = |frame: &[u8; FRAME_BYTES]| {
            let read = |point: usize| {
                let sample = level.offset + point;
                let byte = sample + sample / 2;
                let word = i16::from_le_bytes([frame[byte], frame[byte + 1]]);
                // Expand the even (low 12 bits) or odd (high 12 bits) sample to Q15.
                i32::from(if sample & 1 == 0 {
                    word << 4
                } else {
                    word & !15
                })
            };
            lerp(read(point), read(next), phase_frac)
        };
        let a = cycle(&self.frames[index]);
        let fraction = i32::from(position & 255);
        if fraction == 0 {
            a
        } else {
            lerp(a, cycle(&self.frames[index + 1]), fraction)
        }
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "A difference of signed PCM16 endpoints times 255 fits i32."
)]
fn lerp(a: i32, b: i32, fraction: i32) -> i32 {
    a + (((b - a) * fraction) >> 8)
}

/// Two 64-frame WaveEdit Online banks, resonant saw first and vocal formants second.
pub static BANKS: [Bank<'static>; 2] = [
    Bank::from_packed12(include_bytes!("../banks/reso-saw.wt12"))
        .expect("complete resonant frames"),
    Bank::from_packed12(include_bytes!("../banks/vocal-formants.wt12"))
        .expect("complete vocal frames"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_samples_interpolate_through_the_wrap_and_across_frames() {
        let mut pcm = [0; FRAME_BYTES * 2];
        // First cycle falls from +32752 on the last sample to -32768 on the first.
        pcm[1] = 8;
        pcm[382] = 0xf0;
        pcm[383] = 0x7f;
        let bank = Bank::from_packed12(&pcm).expect("two frames");
        let level = Level::for_step(1);
        assert_eq!(bank.sample(level, 0, 0), -32768);
        assert_eq!(bank.sample(level, 0xff00_0000, 0), 32752);
        assert_eq!(bank.sample(level, 0xff80_0000, 0), -8);
        assert_eq!(bank.sample(level, 0, 128), -16384);
        assert_eq!(bank.sample(level, 0, 256), 0);
    }

    #[test]
    fn brighter_levels_stop_before_their_last_partial_reaches_nyquist() {
        for (i, levels) in LEVELS.windows(2).enumerate() {
            let boundary = levels[0].max_step;
            assert_eq!(Level::for_step(boundary).offset, LEVELS[i].offset);
            assert_eq!(
                Level::for_step(boundary.saturating_add(1)).offset,
                levels[1].offset
            );
        }
    }
}
