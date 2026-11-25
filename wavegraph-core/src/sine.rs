// round(32767 * sin(i * π / 128)) over one quadrant.
const SINE: [i16; 65] = [
    0, 804, 1608, 2410, 3212, 4011, 4808, 5602, 6393, 7179, 7962, 8739, 9512, 10278, 11039, 11793,
    12539, 13279, 14010, 14732, 15446, 16151, 16846, 17530, 18204, 18868, 19519, 20159, 20787,
    21403, 22005, 22594, 23170, 23731, 24279, 24811, 25329, 25832, 26319, 26790, 27245, 27683,
    28105, 28510, 28898, 29268, 29621, 29956, 30273, 30571, 30852, 31113, 31356, 31580, 31785,
    31971, 32137, 32285, 32412, 32521, 32609, 32678, 32728, 32757, 32767,
];

/// Interpolated sine at a wrapping 32-bit phase (one turn), in −32767..=32767.
#[must_use]
#[inline]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Quarter-wave interpolation fits i32"
)]
pub fn sine_q15(phase: u32) -> i32 {
    let quarter = phase & 0x3fff_ffff;
    let folded = if phase & 0x4000_0000 == 0 {
        quarter
    } else {
        (1 << 30) - quarter
    };
    let index = (folded >> 24) as usize;
    let a = i32::from(SINE[index]);
    let b = i32::from(SINE[(index + 1).min(64)]);
    let frac = i32::from(((folded >> 16) & 255) as u8);
    let sample = a + (((b - a) * frac) >> 8);
    if phase & 0x8000_0000 == 0 {
        sample
    } else {
        -sample
    }
}
