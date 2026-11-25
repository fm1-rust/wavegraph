//! Native-size two-bit coverage masks. Four pixels per byte, most significant first.
use crate::{assets::flash_asset, instrument::Part};
use fm1_graphics::Rgb565;
use fm1_graphics::embedded_graphics::pixelcolor::{RgbColor, raw::ToBytes};

pub(super) fn panel_type(part: Part) -> bool {
    matches!(
        part,
        Part::Bass | Part::Tr808 | Part::Tr909 | Part::Supersaw
    )
}

#[derive(Clone, Copy)]
pub(super) struct Mask {
    pub width: usize,
    pub height: usize,
    bytes: &'static [u8],
}
macro_rules! mask {
    ($w:literal, $h:literal, $file:literal) => {
        Mask {
            width: $w,
            height: $h,
            bytes: flash_asset!(
                u8,
                include_bytes!(concat!("../../assets/panel-type/", $file, ".raw"))
            ),
        }
    };
}

pub(super) const LEGEND: Mask = mask!(6, 10, "legend");
pub(super) const HEADING: Mask = mask!(7, 13, "heading");
pub(super) const GRID: Mask = mask!(12, 16, "grid");
pub(super) const BADGE: Mask = mask!(24, 32, "badge");
pub(super) const BASS: Mask = mask!(78, 20, "TB-303");
pub(super) const TR808: Mask = mask!(100, 32, "TR-808");
pub(super) const TR909: Mask = mask!(112, 32, "TR-909");
pub(super) const HOOVER: Mask = mask!(16, 32, "hoover");
pub(super) const JP: Mask = mask!(100, 16, "JP-8080");

impl Mask {
    pub fn frame(self, index: usize) -> Self {
        let stride = (self.width * self.height).div_ceil(4);
        Self {
            bytes: &self.bytes[index * stride..][..stride],
            ..self
        }
    }

    pub fn glyph(self, byte: u8) -> Self {
        self.frame(usize::from(
            if (32..127).contains(&byte) {
                byte
            } else {
                b'?'
            } - 32,
        ))
    }

    pub fn coverage(self, x: usize, y: usize) -> u8 {
        let i = y * self.width + x;
        (self.bytes[i / 4] >> (6 - 2 * (i % 4))) & 3
    }
}

pub(super) fn blend(pixel: &mut [u8; 2], ink: Rgb565, coverage: u8) {
    match coverage {
        0 => (),
        3 => *pixel = ink.to_be_bytes(),
        a => {
            let [hi, lo] = *pixel;
            let mix = |bg, fg| ((3 - a) * bg + a * fg + 1) / 3;
            *pixel = Rgb565::new(
                mix(hi >> 3, ink.r()),
                mix((hi & 7) << 3 | lo >> 5, ink.g()),
                mix(lo & 31, ink.b()),
            )
            .to_be_bytes();
        }
    }
}
