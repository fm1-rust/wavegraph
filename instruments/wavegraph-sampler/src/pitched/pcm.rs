//! Forward-only PCM reads for the pitch interpolator.

#[derive(Clone, Copy, Debug)]
pub(super) enum Pcm<'a> {
    Raw(&'a [[u8; 2]]),
    #[cfg(feature = "ima-adpcm")]
    Ima {
        blocks: &'a [[u8; 64]],
        frames: u32,
    },
    #[cfg(feature = "packed-pcm")]
    Packed {
        groups: &'a [u8],
        frames: u32,
    },
}

impl<'a> Pcm<'a> {
    pub(super) fn frames(self) -> u32 {
        match self {
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
            #[cfg(feature = "ima-adpcm")]
            Self::Ima { blocks, .. } => Cursor::Ima(super::ima::Decoder::new(blocks)),
            #[cfg(feature = "packed-pcm")]
            Self::Packed { groups, frames } => Cursor::Packed(PackedCursor {
                groups,
                remaining: frames,
                position: 0,
            }),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Cursor<'a> {
    Raw(core::slice::Iter<'a, [u8; 2]>),
    #[cfg(feature = "ima-adpcm")]
    Ima(super::ima::Decoder<'a>),
    #[cfg(feature = "packed-pcm")]
    Packed(PackedCursor<'a>),
}

impl Cursor<'_> {
    /// Skip source frames and consume the next one.
    pub(super) fn nth(&mut self, skip: u32) -> Option<i16> {
        match self {
            Self::Raw(frames) => frames
                .nth(usize::try_from(skip).unwrap_or(usize::MAX))
                .copied()
                .map(i16::from_le_bytes),
            #[cfg(feature = "ima-adpcm")]
            Self::Ima(cursor) => cursor.nth(skip),
            #[cfg(feature = "packed-pcm")]
            Self::Packed(cursor) => cursor.nth(skip),
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
