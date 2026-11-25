//! Forward-only PCM reads for the pitch interpolator.

#[derive(Clone, Copy, Debug)]
pub(super) enum Pcm<'a> {
    Raw(&'a [[u8; 2]]),
    #[cfg(feature = "recording")]
    Shared(&'a [core::sync::atomic::AtomicI16]),
    #[cfg(feature = "ima-adpcm")]
    Ima {
        blocks: &'a [[u8; 64]],
        frames: u32,
    },
    #[cfg(feature = "packed-pcm")]
    Packed {
        groups: &'a [u8],
        frames: u32,
        predictive: bool,
    },
}

impl<'a> Pcm<'a> {
    pub(super) fn frames(self) -> u32 {
        match self {
            #[cfg(feature = "recording")]
            Self::Shared(frames) => u32::try_from(frames.len()).unwrap_or(u32::MAX),
            Self::Raw(frames) => u32::try_from(frames.len()).unwrap_or(u32::MAX),
            #[cfg(feature = "ima-adpcm")]
            Self::Ima { frames, .. } => frames,
            #[cfg(feature = "packed-pcm")]
            Self::Packed { frames, .. } => frames,
        }
    }

    pub(super) fn cursor(self) -> Cursor<'a> {
        match self {
            Self::Raw(frames) => Cursor::Raw(frames.iter()),
            #[cfg(feature = "recording")]
            Self::Shared(frames) => Cursor::Shared(frames.iter()),
            #[cfg(feature = "ima-adpcm")]
            Self::Ima { blocks, .. } => Cursor::Ima(super::ima::Decoder::new(blocks)),
            #[cfg(feature = "packed-pcm")]
            Self::Packed {
                groups,
                frames,
                predictive,
            } => {
                let packed = PackedCursor {
                    groups,
                    remaining: frames,
                    position: 0,
                };
                if predictive {
                    Cursor::Predictive {
                        packed,
                        sample: 0,
                        delta: 0,
                        phase: 0,
                    }
                } else {
                    Cursor::Packed(packed)
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Cursor<'a> {
    Raw(core::slice::Iter<'a, [u8; 2]>),
    #[cfg(feature = "recording")]
    Shared(core::slice::Iter<'a, core::sync::atomic::AtomicI16>),
    #[cfg(feature = "ima-adpcm")]
    Ima(super::ima::Decoder<'a>),
    #[cfg(feature = "packed-pcm")]
    Packed(PackedCursor<'a>),
    #[cfg(feature = "packed-pcm")]
    Predictive {
        packed: PackedCursor<'a>,
        sample: i16,
        delta: i16,
        phase: u8,
    },
}

impl Cursor<'_> {
    /// Skip source frames and consume the next one.
    pub(super) fn nth(&mut self, skip: u32) -> Option<i16> {
        match self {
            #[cfg(feature = "recording")]
            Self::Shared(frames) => frames
                .nth(usize::try_from(skip).ok()?)
                .map(|s| s.load(core::sync::atomic::Ordering::Relaxed)),
            Self::Raw(frames) => frames
                .nth(usize::try_from(skip).unwrap_or(usize::MAX))
                .copied()
                .map(i16::from_le_bytes),
            #[cfg(feature = "ima-adpcm")]
            Self::Ima(cursor) => cursor.nth(skip),
            #[cfg(feature = "packed-pcm")]
            Self::Packed(cursor) => cursor.nth(skip),
            #[cfg(feature = "packed-pcm")]
            Self::Predictive {
                packed,
                sample,
                delta,
                phase,
            } => {
                // Each 128-frame block resets both second-difference predictors. Seek over
                // whole blocks in the PCM1 stream, then integrate only the destination block.
                let target = skip.checked_add(u32::from(*phase))?;
                let offset = target % 128;
                let mut steps = skip;
                if target >= 128 {
                    let before = skip.saturating_sub(offset);
                    if before != 0 {
                        packed.nth(before.saturating_sub(1))?;
                    }
                    steps = offset;
                    *phase = 0;
                }
                if *phase == 0 {
                    *sample = 0;
                    *delta = 0;
                }
                for _ in 0..=steps {
                    *delta = delta.wrapping_add(packed.nth(0)?);
                    *sample = sample.wrapping_add(*delta);
                }
                *phase = offset.to_le_bytes()[0].wrapping_add(1) & 127;
                Some(*sample)
            }
        }
    }
}

#[cfg(feature = "packed-pcm")]
#[derive(Clone, Copy, Debug)]
pub(super) struct PackedCursor<'a> {
    groups: &'a [u8],
    remaining: u32,
    position: u8,
}

#[cfg(feature = "packed-pcm")]
impl PackedCursor<'_> {
    // PCM1 groups are independently byte-aligned. Jump across groups and read
    // just the requested frame, even when a high note skips many source frames.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "PCM1 validation bounds widths to 1..=16. A group has 32 frames and the caller's Q15 step skips at most 131072."
    )]
    fn nth(&mut self, skip: u32) -> Option<i16> {
        if skip >= self.remaining {
            self.remaining = 0;
            return None;
        }
        self.remaining -= skip + 1;
        let mut index = u32::from(self.position) + skip;
        while index >= 32 {
            let group_bytes = 1 + 4 * usize::from(self.groups[0]);
            self.groups = &self.groups[group_bytes..];
            index -= 32;
        }
        self.position = index as u8 + 1;
        let width = self.groups[0];
        let bit = usize::from(index as u8) * usize::from(width);
        let bytes = &self.groups[1 + bit / 8..];
        let word = u32::from_le_bytes([
            bytes[0],
            bytes.get(1).copied().unwrap_or(0),
            bytes.get(2).copied().unwrap_or(0),
            0,
        ]);
        let shift = 16 - width;
        Some((((word >> (bit % 8)) as u16) << shift).cast_signed() >> shift)
    }
}
