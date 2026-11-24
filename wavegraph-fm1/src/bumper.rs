//! One-shot playback of the logo cue, mixed before the physical MASTER gain.

use crate::assets::flash_asset;

const PCM: &[u8] = flash_asset!(
    u8,
    include_bytes!(concat!(env!("OUT_DIR"), "/bumper.delta"))
);
include!(concat!(env!("OUT_DIR"), "/bumper_index.rs"));

/// One range of the lossless PCM stream, starting at an independently decodable block.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Cue {
    bytes: &'static [u8],
    skip: usize,
    pub(super) len: usize,
}

impl Cue {
    pub(super) const EMPTY: Self = Self {
        bytes: &[],
        skip: 0,
        len: 0,
    };

    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "The host encoder bounds widths to 16 and the bit reservoir to 23 bits."
    )]
    pub(super) fn samples(self) -> impl Iterator<Item = i16> {
        let mut bytes = self.bytes.iter().copied();
        let mut previous = 0_i16;
        let mut slope = 0_i16;
        let mut frame = 0_u8;
        let mut bits = 0_u32;
        let mut count = 0_u8;
        let mut width = 0_u8;
        core::iter::from_fn(move || {
            if frame.is_multiple_of(128) {
                previous = 0;
                slope = 0;
            }
            if frame.is_multiple_of(16) {
                width = bytes.next()?;
            }
            while count < width {
                bits |= u32::from(bytes.next()?) << count;
                count += 8;
            }
            let packed = (bits & ((1 << width) - 1)) as u16;
            bits >>= width;
            count -= width;
            let residual = ((packed >> 1) as i16) ^ -((packed & 1) as i16);
            slope = slope.wrapping_add(residual);
            previous = previous.wrapping_add(slope);
            frame = frame.wrapping_add(1);
            Some(previous)
        })
        .skip(self.skip)
        .take(self.len)
    }

    #[cfg(test)]
    pub(super) const fn test_peak() -> Self {
        Self {
            bytes: &[16, 0xfc, 0xff, 0xf7, 0xff],
            skip: 0,
            len: 2,
        }
    }
}

/// Remaining frames belonging to the current splash.
#[derive(Debug, Default)]
pub struct Bumper {
    remaining: usize,
}

impl Bumper {
    pub(super) fn start(&mut self) {
        self.remaining = FRAMES;
    }
    pub(super) fn stop(&mut self) {
        self.remaining = 0;
    }

    /// Reserve a range under the lock; decoding happens in the master mix after unlocking.
    pub(super) fn take(&mut self, frames: usize, master: u16) -> Cue {
        if master <= 4 {
            self.stop();
        }
        let len = frames.min(self.remaining);
        if len == 0 {
            return Cue::EMPTY;
        }
        let start = FRAMES.saturating_sub(self.remaining);
        self.remaining = self.remaining.saturating_sub(len);
        Cue {
            bytes: &PCM[usize::from(OFFSETS[start / 128])..],
            skip: start % 128,
            len,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_logo_starts_playback_and_mute_cancels_it() {
        let mut bumper = Bumper::default();
        assert_eq!(bumper.take(128, 1023), Cue::EMPTY);
        for muted in [0, 1, 4] {
            bumper.start();
            assert_eq!(bumper.take(128, muted), Cue::EMPTY);
            assert_eq!(bumper.take(128, 1023), Cue::EMPTY);
        }
        bumper.start();
        assert_eq!(bumper.take(128, 512).len, 128);
        assert_eq!(bumper.take(128, 0), Cue::EMPTY);
        bumper.start();
        bumper.stop();
        assert_eq!(bumper.take(128, 512), Cue::EMPTY);
    }

    #[test]
    fn every_sample_survives_packing_and_arbitrary_dma_boundaries() {
        let pcm = include_bytes!(concat!(env!("OUT_DIR"), "/bumper.s16le"));
        let original = pcm.as_chunks::<2>().0;
        for size in [1, 83, 128, 283] {
            let mut bumper = Bumper::default();
            bumper.start();
            for block in original.chunks(size) {
                assert!(
                    bumper
                        .take(size, 512)
                        .samples()
                        .eq(block.iter().copied().map(i16::from_le_bytes))
                );
            }
            assert_eq!(bumper.take(size, 512), Cue::EMPTY);
        }
    }
}
