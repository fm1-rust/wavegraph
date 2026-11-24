//! Read-only factory assets. The FM-1 maps these bytes directly from upper flash.

use wavegraph_909::Samples;
use wavegraph_wavetable::{BANKS, Bank};

/// Put the array itself in the asset section. Placing only a slice there
/// would leave its backing bytes in the code partition.
macro_rules! flash_asset {
    ($item:ty, $source:expr) => {{
        const SOURCE: &[$item] = $source;
        // SAFETY: the linker places these immutable arrays at their natural
        // alignment in read-only XIP. The packager installs that section intact.
        #[cfg_attr(target_os = "none", unsafe(link_section = ".wavegraph_assets"))]
        static DATA: [$item; SOURCE.len()] = *SOURCE.first_chunk().expect("source length");
        &DATA
    }};
}
pub(crate) use flash_asset;

// Keep the upstream notice with standalone firmware and emulator images.
#[cfg(target_os = "none")]
#[used]
// SAFETY: the linker keeps this immutable text in read-only upper flash.
#[unsafe(link_section = ".wavegraph_assets.notices")]
static PATTERN_NOTICE: [u8; include_bytes!("../patterns/NOTICE.txt").len()] =
    *include_bytes!("../patterns/NOTICE.txt");

#[cfg(target_os = "none")]
pub(crate) mod fonts {
    use embedded_graphics::{
        image::ImageRaw,
        mono_font::{MonoFont, ascii},
    };

    // The ASCII atlases have 16 glyphs per row. Reuse the upstream font metrics
    // and glyph mapping with the bitmap stored in the FM-1 asset partition.
    macro_rules! font {
        ($name:ident, $file:literal) => {
            pub(crate) const $name: MonoFont<'static> = MonoFont {
                image: ImageRaw::new(
                    super::flash_asset!(u8, include_bytes!(concat!("../assets/fonts/", $file))),
                    16 * ascii::$name.character_size.width,
                ),
                ..ascii::$name
            };
        };
    }

    font!(FONT_6X9, "font_6x9.raw");
    font!(FONT_6X10, "font_6x10.raw");
    font!(FONT_7X13_BOLD, "font_7x13_bold.raw");
    font!(FONT_9X15_BOLD, "font_9x15_bold.raw");
}

pub(crate) const CYMBALS: Samples<'static> = Samples {
    closed_hat: flash_asset!(u8, Samples::X0XPROC1.closed_hat),
    open_hat: flash_asset!(u8, Samples::X0XPROC1.open_hat),
    ride: flash_asset!(u8, Samples::X0XPROC1.ride),
};

// SAFETY: the catalog contains immutable references to the upper-flash banks.
#[cfg_attr(target_os = "none", unsafe(link_section = ".wavegraph_assets"))]
pub(crate) static WAVETABLES: [Bank<'static>; 2] = [
    Bank::from_packed12(flash_asset!(u8, BANKS[0].as_packed12())).expect("resonant frames"),
    Bank::from_packed12(flash_asset!(u8, BANKS[1].as_packed12())).expect("vocal frames"),
];

// Carry the font copyright and license with the raster assets in standalone images.
#[cfg(target_os = "none")]
#[used]
// SAFETY: the linker keeps this immutable notice in read-only upper flash.
#[unsafe(link_section = ".wavegraph_assets.notices")]
static FONT_NOTICE: [u8; include_bytes!("../assets/panel-type/OFL.txt").len()] =
    *include_bytes!("../assets/panel-type/OFL.txt");
