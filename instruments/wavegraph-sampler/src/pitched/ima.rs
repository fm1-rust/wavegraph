//! Mono Microsoft IMA ADPCM. Each block begins with an independent predictor.

pub(super) const FRAMES: u32 = 1 + 2 * (64 - 4);
const STEP: [i16; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX_DELTA: [i8; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

#[derive(Clone, Debug)]
pub(super) struct Decoder<'a> {
    blocks: &'a [[u8; 64]],
    next: u8,
    predictor: i16,
    index: u8,
}

impl<'a> Decoder<'a> {
    pub(super) fn new(blocks: &'a [[u8; 64]]) -> Self {
        Self {
            blocks,
            next: 0,
            predictor: 0,
            index: 0,
        }
    }

    // A seek can skip whole blocks without touching their contents. At most
    // 120 nibbles need decoding, even for a trim near the end of a long clip.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "Block offsets are below 121. The signed prediction is clamped to i16 before narrowing."
    )]
    pub(super) fn nth(&mut self, skip: u32) -> Option<i16> {
        let mut target = u32::from(self.next).checked_add(skip)?;
        if target >= FRAMES {
            self.blocks = self.blocks.get(usize::try_from(target / FRAMES).ok()?..)?;
            target %= FRAMES;
            self.next = 0;
        }
        let block = self.blocks.first()?;
        if self.next == 0 {
            self.predictor = i16::from_le_bytes([block[0], block[1]]);
            self.index = block[2];
            self.next = 1;
        }
        while u32::from(self.next) <= target {
            let i = usize::from(self.next - 1);
            let nibble = (block[4 + i / 2] >> (4 * (i % 2))) & 15;
            let magnitude = nibble & 7;
            let step = i32::from(STEP[usize::from(self.index)]);
            let delta = ((2 * i32::from(magnitude) + 1) * step) >> 3;
            let prediction =
                i32::from(self.predictor) + if nibble & 8 == 0 { delta } else { -delta };
            self.predictor = prediction.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            self.index = self
                .index
                .saturating_add_signed(INDEX_DELTA[usize::from(magnitude)])
                .min(88);
            self.next += 1;
        }
        Some(self.predictor)
    }
}
