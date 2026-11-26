//! Graphite rack face with a blue stripe and a luminous Alpha-style readout.
use super::*;
use crate::instrument::HooverPage;
const STYLE: WaveformStyle = style::HOOVER.scope;
const LCD: Rgb565 = STYLE.trace;
const STRIPE: Rgb565 = style::HOOVER.accent;
impl Screen {
    pub(super) async fn paint_hoover_env(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        labels: &Labels,
    ) -> Result<(), Error> {
        let p = labels.hoover;
        let target = labels.hoover_page;
        let page = Page::HooverEnvelope(labels.channel, p, target);
        let previous = self.page.replace(page);
        if previous == Some(page) {
            return Ok(());
        }
        if !matches!(previous, Some(Page::HooverEnvelope(..))) {
            self.change_view(STYLE);
            lcd.as_async(clock)?
                .fill_rect(0, 0, SIZE, SIZE, STYLE.background.into_storage())
                .await?;
        }
        // Display time in panel units so short segments stay legible.
        let [a, b, c, r] = p.times.map(|t| usize::from(t) + 3);
        let total = a + b + c + 24 + r;
        let [l1, l2, l3] = p.levels.map(i32::from);
        let trace = core::array::from_fn(|x| {
            let mut pos = x * total / (WIDTH - 1);
            let mut level = 0;
            for (width, to) in [(a, l1), (b, l2), (c, l3), (24, l3), (r, 0)] {
                if pos < width {
                    level += (to - level) * pos as i32 / width as i32;
                    break;
                }
                pos -= width;
                level = to;
            }
            (172 - level) as u8
        });
        self.plot
            .paint(lcd, clock, self.strip, (0, 0), STYLE, &trace)
            .await?;
        self.text.text(0, 0, b"HOOVER  DCO VCF VCA", LCD);
        self.text.text(0, 1, b"SEL TIME LVL DCO MOD", STRIPE);
        let columns = match target {
            HooverPage::Times => 4..8,
            HooverPage::Levels => 9..12,
            HooverPage::Dco => 13..16,
            HooverPage::Tone => 17..20,
        };
        self.text.highlight(1, columns, style::HOOVER.selected);
        let (names, values): (&[u8], [u8; 4]) = match target {
            HooverPage::Times => (b"T1   T2   T3   T4  ", p.times),
            HooverPage::Levels => (
                b"L1   L2   L3   RES ",
                [p.levels[0], p.levels[1], p.levels[2], p.resonance],
            ),
            HooverPage::Tone => (
                b"HPF  ENV  RATE NOI ",
                [p.hpf, p.filter_env, p.pwm_rate, p.noise],
            ),
            HooverPage::Dco => (
                b"PUL  SAW  SUB  LVL ",
                [p.pulse_wave, p.saw_wave, p.sub_wave, p.sub_level],
            ),
        };
        self.text.text(0, 13, names, STRIPE);
        for (i, v) in values.into_iter().enumerate() {
            self.text.number(i * 5, 14, 3, u32::from(v), LCD);
        }
        self.text.paint_async(lcd, clock).await
    }
}

pub(super) async fn header(
    lcd: &mut PanelDisplay<'_>,
    pixels: &mut [[u8; 2]],
    selector: &mut Selector,
    labels: &Labels,
) -> Result<(), Error> {
    use fm1_graphics::embedded_graphics::{
        prelude::{Point, Size},
        text::Alignment,
    };
    let ink = Rgb565::new(2, 8, 5);
    for y in (0..40).step_by(BAND) {
        let mut band = mixer::Band::new(pixels, 0, y, 228, BAND as u16, STYLE.background);
        band.rect(Point::new(0, 34), Size::new(228, 3), STRIPE);
        band.title("HOOVER", Point::new(8, 5), palette::WHITE);
        band.label(
            match labels.mode {
                wavegraph_hoover::VoiceMode::Poly => "POLY SYNTH",
                mode => crate::panel::Polyphony::label(mode),
            },
            Point::new(9, 21),
            Alignment::Left,
            STRIPE,
        );
        band.rect(Point::new(98, 5), Size::new(105, 24), LCD);
        band.label(
            wavegraph_hoover::Patch::PRESETS[usize::from(labels.voice.waveform)].0,
            Point::new(105, 12),
            Alignment::Left,
            ink,
        );
        band.label(
            ["1", "2", "3", "4"][labels.channel.index()],
            Point::new(195, 12),
            Alignment::Right,
            ink,
        );
        band.mask(
            Point::new(72, 1),
            lettering::HOOVER,
            0,
            [style::HOOVER.ink; 2],
        );
        band.compose(selector);
        lcd.write_rect(0, y, 228, BAND as u16, band.pixels).await?;
    }
    lcd.flush().await
}
