//! Read-only factory assets. The FM-1 maps these bytes directly from upper flash.

#[cfg(not(feature = "packed-cymbals"))]
use wavegraph_909::Samples;
use wavegraph_wavetable::{BANKS, Bank};

pub(crate) use fm1_assets::flash_asset;

pub(super) const SAMPLE_NAMES: [[u8; 9]; 5] = [
    *b"AMEN BRK ",
    *b"RAVE RISE",
    *b"AIRHORN  ",
    *b"LASER ZAP",
    *b"RAVE STAB",
];

macro_rules! rave_clip {
    ($file:literal, $rate:literal, $root:literal) => {{
        #[cfg(feature = "packed-samples")]
        let clip = wavegraph_sampler::Clip::from_pcm1(
            flash_asset!(
                u8,
                include_bytes!(concat!(env!("OUT_DIR"), "/", $file, ".pcm1"))
            ),
            $rate,
            $root,
        );
        #[cfg(not(feature = "packed-samples"))]
        let clip = wavegraph_sampler::Clip::from_pcm16le(
            flash_asset!(
                u8,
                include_bytes!(concat!("../samples/rave/", $file, ".s16le"))
            ),
            $rate,
            $root,
        );
        clip.expect("valid factory sampler clip")
    }};
}

// SAFETY: these immutable clip descriptors point into read-only upper flash.
#[cfg_attr(target_os = "none", unsafe(link_section = ".fm1_assets"))]
pub(super) static RAVE_BANK: [wavegraph_sampler::Clip<'static>; SAMPLE_NAMES.len()] = [
    {
        // The two encodings contain the same sixteen beats. Only one enters the image.
        #[cfg(all(feature = "packed-samples", feature = "packed-cymbals"))]
        let (bytes, frames, hz) = (
            flash_asset!(u8, include_bytes!("../samples/rave/amen.ima")).as_slice(),
            192_000,
            24_400,
        );
        #[cfg(not(all(feature = "packed-samples", feature = "packed-cymbals")))]
        let (bytes, frames, hz) = (
            flash_asset!(u8, include_bytes!("../samples/rave/amen-compact.ima")).as_slice(),
            98_400,
            12_505,
        );
        wavegraph_sampler::Clip::from_ima_adpcm(bytes, frames, hz, 60)
            .expect("validated four-bar Amen break")
            .with_native_tempo(122)
    },
    wavegraph_sampler::Clip::from_ima_adpcm(
        flash_asset!(u8, include_bytes!("../samples/rave/rise.ima")),
        19_404,
        14_700,
        60,
    )
    .expect("validated rave rise"),
    rave_clip!("horn", 22_059, 60),
    rave_clip!("zap", 22_059, 60),
    rave_clip!("stab", 14_706, 43),
];

#[cfg(target_os = "none")]
#[used]
// SAFETY: the linker keeps this immutable text in read-only upper flash.
#[unsafe(link_section = ".fm1_assets.keep")]
static RAVE_NOTICE: [u8; include_bytes!("../samples/rave/NOTICE.txt").len()] =
    *include_bytes!("../samples/rave/NOTICE.txt");

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

#[cfg(not(feature = "packed-cymbals"))]
const CYMBALS: Samples<'static> = Samples {
    closed_hat: flash_asset!(u8, Samples::X0XPROC1.closed_hat),
    open_hat: flash_asset!(u8, Samples::X0XPROC1.open_hat),
    ride: flash_asset!(u8, Samples::X0XPROC1.ride),
};

#[cfg(not(feature = "packed-cymbals"))]
pub(super) fn tr909(rate: (u32, u32), decay: u8) -> Option<wavegraph_909::Tr909<'static>> {
    wavegraph_909::Tr909::with_decay(CYMBALS, rate.0, rate.1, decay)
}

#[cfg(feature = "packed-cymbals")]
pub(super) fn tr909(rate: (u32, u32), decay: u8) -> Option<wavegraph_909::Tr909<'static>> {
    use fm1_assets::pcm::PackedPcm16;
    use wavegraph_sampler::Sampler;
    macro_rules! cymbal {
        ($file:literal) => {
            PackedPcm16::parse(flash_asset!(
                u8,
                include_bytes!(concat!(env!("OUT_DIR"), "/", $file))
            ))
            .expect("built-in PCM1 clip")
        };
    }
    const CLIPS: [PackedPcm16<'static>; 3] = [
        cymbal!("closed-hat.pcm1"),
        cymbal!("open-hat.pcm1"),
        cymbal!("ride.pcm1"),
    ];
    wavegraph_909::Tr909::with_cymbals(CLIPS.map(Sampler::from_packed), rate.0, rate.1, decay)
}

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

#[cfg(all(test, feature = "packed-cymbals"))]
mod tests {
    use wavegraph_909::{Samples, Tr909, Voice};

    #[test]
    fn packed_cymbals_play_every_original_pcm_sample() {
        let clips = Samples::X0XPROC1;
        for (voice, pcm) in [
            (Voice::ClosedHat, clips.closed_hat),
            (Voice::OpenHat, clips.open_hat),
            (Voice::Ride, clips.ride),
        ] {
            let mut kit = super::tr909((44_100, 1), 80).expect("valid packed kit");
            let stem = Voice::ALL
                .iter()
                .position(|&v| v == voice)
                .expect("voice stem");
            let (samples, tail) = pcm.as_chunks::<2>();
            assert!(tail.is_empty());
            kit.trigger(voice);
            for (frame, &bytes) in samples.iter().enumerate() {
                assert_eq!(
                    kit.next_voices()[stem],
                    i16::from_le_bytes(bytes),
                    "{voice:?}, frame {frame}"
                );
            }
            assert!(!kit.voice_active(voice));
            assert_eq!(kit.next_voices()[stem], 0);
        }
    }

    #[test]
    fn packed_cymbals_match_the_raw_kit_through_chokes_and_retriggers() {
        let mut packed = super::tr909((44_100, 1), 80).expect("valid packed kit");
        let mut raw = Tr909::with_decay(Samples::X0XPROC1, 44_100, 1, 80).expect("valid raw kit");
        for frame in 0..43_000 {
            let events: &[Voice] = match frame {
                0 => &[Voice::Ride, Voice::OpenHat],
                31 | 255 => &[Voice::ClosedHat],
                32 => &[Voice::OpenHat, Voice::ClosedHat],
                128 => &[Voice::Ride],
                _ => &[],
            };
            for &voice in events {
                raw.trigger(voice);
                packed.trigger(voice);
            }
            assert_eq!(raw.next_voices(), packed.next_voices(), "frame {frame}");
        }
        assert!(!packed.voice_active(Voice::Ride));
    }
}
