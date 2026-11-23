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

pub(crate) const CYMBALS: Samples<'static> = Samples {
    closed_hat: flash_asset!(u8, Samples::X0XPROC1.closed_hat),
    open_hat: flash_asset!(u8, Samples::X0XPROC1.open_hat),
    ride: flash_asset!(u8, Samples::X0XPROC1.ride),
};

pub(crate) static WAVETABLES: [Bank<'static>; 2] = [
    Bank::from_packed12(flash_asset!(u8, BANKS[0].as_packed12())).expect("resonant frames"),
    Bank::from_packed12(flash_asset!(u8, BANKS[1].as_packed12())).expect("vocal frames"),
];
