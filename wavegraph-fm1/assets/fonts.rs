//! Pack the upstream ASCII atlas pixels for the firmware's upper-flash partition.

use fm1_graphics::embedded_graphics::{
    framebuffer::{Framebuffer, buffer_size},
    image::Image,
    mono_font::ascii,
    pixelcolor::{BinaryColor, raw::BigEndian},
    prelude::*,
};
use std::{fs, path::Path};

pub(super) fn write_atlases(out: &Path) {
    // Upstream ASCII atlases have 16 columns and six rows. Draw the complete
    // image into a one-bit framebuffer to obtain its public, packed MSB-first data.
    macro_rules! write_font {
        ($font:ident) => {{
            const W: usize = 16 * ascii::$font.character_size.width as usize;
            const H: usize = 6 * ascii::$font.character_size.height as usize;
            let mut atlas = Framebuffer::<
                BinaryColor,
                _,
                BigEndian,
                W,
                H,
                { buffer_size::<BinaryColor>(W, H) },
            >::new();
            assert_eq!(atlas.size(), ascii::$font.image.size());
            let Ok(()) = Image::new(&ascii::$font.image, Point::zero()).draw(&mut atlas);
            fs::write(out.join(concat!(stringify!($font), ".raw")), atlas.data())
                .expect("write font atlas");
        }};
    }

    write_font!(FONT_6X10);
    write_font!(FONT_7X13_BOLD);
    write_font!(FONT_9X15_BOLD);

    // One native u16 load supplies all twelve picker pixels of a GRID scanline.
    // The original renderer draws ink for coverage 2 or 3 (the high bit of
    // each two-bit pixel). Keep bit 15 at the left for a straight row blit.
    const GRID: &[u8; 95 * 16 * 3] = include_bytes!("panel-type/grid.raw");
    let rows: Vec<u16> = GRID
        .as_chunks::<3>()
        .0
        .iter()
        .map(|line| {
            let mut ink = 0_u16;
            for x in 0..12 {
                if line[x / 4] & [0x80, 0x20, 0x08, 0x02][x % 4] != 0 {
                    ink |= 0x8000 >> x;
                }
            }
            ink
        })
        .collect();
    fs::write(out.join("picker-rows.rs"), format!("&{rows:?}")).expect("write picker glyph rows");
}
