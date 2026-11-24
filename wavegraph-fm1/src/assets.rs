//! Read-only factory assets. The FM-1 maps these bytes directly from upper flash.

use wavegraph_909::Samples;
use wavegraph_wavetable::{BANKS, Bank};

pub(crate) use fm1_assets::flash_asset;

// Keep the upstream notice with standalone firmware and emulator images.
#[cfg(target_os = "none")]
#[used]
// SAFETY: the linker keeps this immutable text in read-only upper flash.
#[unsafe(link_section = ".fm1_assets.keep")]
static PATTERN_NOTICE: [u8; include_bytes!("../patterns/NOTICE.txt").len()] =
    *include_bytes!("../patterns/NOTICE.txt");

#[cfg(target_os = "none")]
pub(crate) mod fonts {
    use fm1_graphics::embedded_graphics::{
        image::ImageRaw,
        mono_font::{MonoFont, ascii},
    };

    // The ASCII atlases have 16 glyphs per row. Reuse the upstream font metrics
    // and glyph mapping with the bitmap stored in the FM-1 asset partition.
    macro_rules! font {
        ($name:ident) => {
            pub(crate) const $name: MonoFont<'static> = MonoFont {
                image: ImageRaw::new(
                    super::flash_asset!(
                        u8,
                        include_bytes!(concat!(env!("OUT_DIR"), "/", stringify!($name), ".raw"))
                    ),
                    16 * ascii::$name.character_size.width,
                ),
                ..ascii::$name
            };
        };
    }

    font!(FONT_6X10);
    font!(FONT_7X13_BOLD);
    font!(FONT_9X15_BOLD);
}

pub(crate) const CYMBALS: Samples<'static> = Samples {
    closed_hat: flash_asset!(u8, Samples::X0XPROC1.closed_hat),
    open_hat: flash_asset!(u8, Samples::X0XPROC1.open_hat),
    ride: flash_asset!(u8, Samples::X0XPROC1.ride),
};

// SAFETY: the catalog contains immutable references to the upper-flash banks.
#[cfg_attr(target_os = "none", unsafe(link_section = ".fm1_assets"))]
pub(crate) static WAVETABLES: [Bank<'static>; 2] = [
    Bank::from_packed12(flash_asset!(u8, BANKS[0].as_packed12())).expect("resonant frames"),
    Bank::from_packed12(flash_asset!(u8, BANKS[1].as_packed12())).expect("vocal frames"),
];

// Carry the font copyright and license with the raster assets in standalone images.
#[cfg(target_os = "none")]
#[used]
// SAFETY: the linker keeps this immutable notice in read-only upper flash.
#[unsafe(link_section = ".fm1_assets.keep")]
static FONT_NOTICE: [u8; include_bytes!("../assets/panel-type/OFL.txt").len()] =
    *include_bytes!("../assets/panel-type/OFL.txt");
