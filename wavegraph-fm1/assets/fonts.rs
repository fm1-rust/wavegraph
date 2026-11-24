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
}
