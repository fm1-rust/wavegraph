//! Incremental rasterization of the live trace and its beat pulse.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Coordinates and extents are bounded by the 240-pixel display."
)]

use crate::capture::{Trace, WIDTH};
use core::ops::Range;

pub(super) const GRID: u8 = 24;
pub(super) const TRAIL: usize = 32;
const MIN_STRIPE: usize = 6;
const _: () = assert!(WIDTH.is_multiple_of(24));

/// Narrow stripes track steep harmonic traces; drum hits favor fewer LCD windows.
#[derive(Clone, Copy)]
pub(super) enum Stripes {
    Fine = 6,
    Normal = 12,
    Wide = 24,
}

type Pixel = [u8; 2];

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Pen {
    pub(super) ink: Pixel,
    pub(super) halo: Option<Pixel>,
}

pub(super) struct Palette {
    pens: [Pen; TRAIL + 1],
    grid: Pixel,
    center: Pixel,
    background: Pixel,
}

impl Palette {
    pub(super) fn new(
        background: Pixel,
        grid: Pixel,
        center: Pixel,
        pens: [Pen; TRAIL + 1],
    ) -> Self {
        Self {
            pens,
            grid,
            center,
            background,
        }
    }
}

#[derive(Eq, PartialEq)]
pub(super) struct Sweep {
    /// Absolute LCD coordinates, one endpoint per column.
    pub(super) trace: Trace,
    pub(super) head: Option<usize>,
}

#[derive(Clone, Copy)]
pub(super) struct Window {
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) width: usize,
    pub(super) height: usize,
}

impl Window {
    /// Split only windows too large for one DMA staging strip.
    pub(super) fn strips(self, capacity: usize) -> impl Iterator<Item = Self> {
        let rows = capacity / self.width;
        (0..self.height).step_by(rows).map(move |dy| Self {
            y: self.y + dy,
            height: rows.min(self.height - dy),
            ..self
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Span {
    top: u8,
    end: u8,
}

impl Span {
    const EMPTY: Self = Self {
        top: WIDTH as u8,
        end: 0,
    };

    fn union(self, other: Self) -> Self {
        Self {
            top: self.top.min(other.top),
            end: self.end.max(other.end),
        }
    }

    fn with_halo(self, pen: Pen) -> Self {
        let margin = u8::from(pen.halo.is_some());
        Self {
            top: self.top.saturating_sub(margin),
            end: (self.end + margin).min(WIDTH as u8),
        }
    }
}

/// Two independent spans per stripe let a note jump without sending the grid between traces.
pub(super) struct Damage {
    spans: [[Span; 2]; WIDTH / MIN_STRIPE],
    stripe: usize,
}

impl Damage {
    fn new(stripes: Stripes) -> Self {
        Self {
            spans: [[Span::EMPTY; 2]; WIDTH / MIN_STRIPE],
            stripe: stripes as usize,
        }
    }

    pub(super) fn full(rows: Range<usize>, overlay: Option<Window>) -> Self {
        let mut damage = Self::new(Stripes::Normal);
        let stripe = damage.stripe;
        for (i, pair) in damage.spans[..WIDTH / stripe].iter_mut().enumerate() {
            let mut span = Span {
                top: rows.start as u8,
                end: rows.end as u8,
            };
            if let Some(list) = overlay
                && i * stripe < list.x + list.width
                && (i + 1) * stripe > list.x
            {
                span = span.union(Span {
                    top: list.y as u8,
                    end: (list.y + list.height) as u8,
                });
            }
            pair[0] = span;
        }
        damage
    }

    pub(super) fn windows(&self) -> impl Iterator<Item = Window> + '_ {
        let mut x = 0;
        self.spans[..WIDTH / self.stripe]
            .chunk_by(|a, b| a == b)
            .flat_map(move |group| {
                let left = x;
                let width = group.len() * self.stripe;
                x += width;
                group[0]
                    .into_iter()
                    .filter(|s| s.top < s.end)
                    .map(move |s| Window {
                        x: left,
                        y: usize::from(s.top),
                        width,
                        height: usize::from(s.end - s.top),
                    })
            })
    }
}

pub(super) fn segment(trace: &Trace, x: usize) -> (usize, usize) {
    let a = usize::from(trace[x.saturating_sub(1)]);
    let b = usize::from(trace[x]);
    (a.min(b), a.max(b))
}

impl Sweep {
    fn column(&self, x: usize, palette: &Palette) -> (Span, Pen) {
        let (top, bottom) = segment(&self.trace, x);
        let age = self
            .head
            .map_or(TRAIL, |h| ((h + WIDTH - x) % WIDTH).min(TRAIL));
        (
            Span {
                top: top as u8,
                end: (bottom + 1) as u8,
            },
            palette.pens[age],
        )
    }

    pub(super) fn damage(&self, previous: &Self, palette: &Palette, stripes: Stripes) -> Damage {
        let mut damage = Damage::new(stripes);
        let stripe = damage.stripe;
        for (i, pair) in damage.spans[..WIDTH / stripe].iter_mut().enumerate() {
            let mut old = Span::EMPTY;
            let mut new = Span::EMPTY;
            for x in i * stripe..(i + 1) * stripe {
                let before = previous.column(x, palette);
                let after = self.column(x, palette);
                if before != after {
                    old = old.union(before.0.with_halo(before.1));
                    new = new.union(after.0.with_halo(after.1));
                }
            }
            if new.top < old.top {
                core::mem::swap(&mut old, &mut new);
            }
            // Bridge gaps of up to two rows to avoid opening another LCD window.
            *pair = if new.top <= old.end + 2 {
                [old.union(new), Span::EMPTY]
            } else {
                [old, new]
            };
        }
        damage
    }

    pub(super) fn render(&self, w: Window, palette: &Palette, center: usize, pixels: &mut [Pixel]) {
        let grid = usize::from(GRID);
        let first_grid = (grid - w.x % grid) % grid;
        for (dy, row) in pixels.chunks_exact_mut(w.width).enumerate() {
            let y = w.y + dy;
            if y == center {
                row.fill(palette.center);
            } else if y.abs_diff(center).is_multiple_of(grid) {
                row.fill(palette.grid);
            } else {
                row.fill(palette.background);
                for pixel in row.iter_mut().skip(first_grid).step_by(grid) {
                    *pixel = palette.grid;
                }
            }
        }
        let end = w.y + w.height;
        for dx in 0..w.width {
            let (span, pen) = self.column(w.x + dx, palette);
            for y in usize::from(span.top).max(w.y)..usize::from(span.end).min(end) {
                pixels[(y - w.y) * w.width + dx] = pen.ink;
            }
            if let Some(halo) = pen.halo {
                for y in [
                    usize::from(span.top).checked_sub(1),
                    Some(usize::from(span.end)),
                ]
                .into_iter()
                .flatten()
                {
                    if (w.y..end).contains(&y) {
                        pixels[(y - w.y) * w.width + dx] = halo;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: Pixel = [0, 0];
    const LINE: Pixel = [1, 1];
    const CENTER: Pixel = [2, 2];
    const OUTSIDE: Pixel = [255, 255];
    const CAPACITY: usize = 8 * WIDTH;

    fn palette() -> Palette {
        Palette::new(
            BG,
            LINE,
            CENTER,
            core::array::from_fn(|age| match age {
                0..=3 => Pen {
                    ink: [9, 9],
                    halo: Some([8, 8]),
                },
                4..TRAIL => Pen {
                    ink: [6, 6],
                    halo: None,
                },
                _ => Pen {
                    ink: [5, 5],
                    halo: None,
                },
            }),
        )
    }

    #[test]
    fn dirty_windows_match_a_full_repaint_across_wrap_stop_and_silence() {
        for stripes in [Stripes::Fine, Stripes::Normal, Stripes::Wide] {
            let palette = palette();
            let center: u8 = 152;
            let flat = [center; WIDTH];
            let saw = core::array::from_fn(|x| 66 + u8::try_from(x % 171).expect("bounded ramp"));
            let square = core::array::from_fn(|x| if (x / 24) % 2 == 0 { 237 } else { 66 });
            let mut lcd = vec![OUTSIDE; WIDTH * WIDTH];
            let mut previous = None;
            for (trace, head) in [
                (flat, None),
                (saw, Some(239)),
                (square, Some(0)),
                (square, Some(1)),
                (saw, Some(16)),
                (saw, None),
                (flat, None),
                (flat, None),
            ] {
                let sweep = Sweep { trace, head };
                let damage = previous.as_ref().map_or_else(
                    || Damage::full(64..240, None),
                    |old| sweep.damage(old, &palette, stripes),
                );
                let mut touched = vec![false; lcd.len()];
                for w in damage.windows().flat_map(|w| w.strips(CAPACITY)) {
                    assert!(w.width * w.height <= CAPACITY);
                    let mut strip = vec![OUTSIDE; w.width * w.height];
                    sweep.render(w, &palette, usize::from(center), &mut strip);
                    for (dy, row) in strip.chunks_exact(w.width).enumerate() {
                        let i = (w.y + dy) * WIDTH + w.x;
                        assert!(touched[i..i + w.width].iter().all(|&p| !p));
                        touched[i..i + w.width].fill(true);
                        lcd[i..i + w.width].copy_from_slice(row);
                    }
                }
                for (i, pixel) in lcd.iter().enumerate() {
                    let (x, y) = (i % WIDTH, i / WIDTH);
                    let a = usize::from(trace[x.saturating_sub(1)]);
                    let b = usize::from(trace[x]);
                    let age = head.map_or(TRAIL, |head| (head + WIDTH - x) % WIDTH);
                    let pen = palette.pens[age.min(TRAIL)];
                    let paper = if !(64..240).contains(&y) {
                        OUTSIDE
                    } else if y == usize::from(center) {
                        CENTER
                    } else if x % 24 == 0 || y % 24 == usize::from(center) % 24 {
                        LINE
                    } else {
                        BG
                    };
                    let expected = if (a.min(b)..=a.max(b)).contains(&y) {
                        pen.ink
                    } else if y + 1 == a.min(b) || y == a.max(b) + 1 {
                        pen.halo.unwrap_or(paper)
                    } else {
                        paper
                    };
                    assert_eq!(*pixel, expected, "x={x}, y={y}, head={head:?}");
                }
                previous = Some(sweep);
            }
        }
    }

    #[test]
    fn opening_the_list_only_extends_the_drum_viewport_under_the_list() {
        let list = Window {
            x: 48,
            y: 46,
            width: 144,
            height: 147,
        };
        let windows: Vec<_> = Damage::full(56..184, Some(list))
            .windows()
            .map(|w| (w.x, w.y, w.width, w.height))
            .collect();
        assert_eq!(
            windows,
            [(0, 56, 48, 128), (48, 46, 144, 147), (192, 56, 48, 128)]
        );
    }

    #[test]
    fn a_large_jump_sends_two_scanlines_and_an_identical_frame_sends_nothing() {
        let old = Sweep {
            trace: [40; WIDTH],
            head: None,
        };
        let new = Sweep {
            trace: [200; WIDTH],
            head: None,
        };
        let palette = palette();
        let windows: Vec<_> = new
            .damage(&old, &palette, Stripes::Normal)
            .windows()
            .map(|w| (w.x, w.y, w.width, w.height))
            .collect();
        assert_eq!(windows, [(0, 40, WIDTH, 1), (0, 200, WIDTH, 1)]);
        assert_eq!(
            new.damage(&new, &palette, Stripes::Normal)
                .windows()
                .count(),
            0
        );
    }
}
