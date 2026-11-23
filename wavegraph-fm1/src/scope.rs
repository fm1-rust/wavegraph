//! Incremental rasterization of the live trace and its beat pulse.
#![allow(
    clippy::arithmetic_side_effects,
    reason = "Coordinates and extents are bounded by the 240-pixel display."
)]

use crate::capture::{Trace, WIDTH};
use core::ops::Range;

pub(super) const GRID: u8 = 24;
pub(super) const TRAIL: usize = 32;
const STRIPE: usize = 12;
const _: () = assert!(WIDTH.is_multiple_of(STRIPE));

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
    row: [Pixel; WIDTH],
}

impl Palette {
    pub(super) fn new(
        background: Pixel,
        grid: Pixel,
        center: Pixel,
        pens: [Pen; TRAIL + 1],
    ) -> Self {
        let mut row = [background; WIDTH];
        for pixel in row.iter_mut().step_by(usize::from(GRID)) {
            *pixel = grid;
        }
        Self {
            pens,
            grid,
            center,
            row,
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
    top: usize,
    end: usize,
}

impl Span {
    const EMPTY: Self = Self { top: WIDTH, end: 0 };

    fn union(self, other: Self) -> Self {
        Self {
            top: self.top.min(other.top),
            end: self.end.max(other.end),
        }
    }

    fn with_halo(self, pen: Pen) -> Self {
        let margin = usize::from(pen.halo.is_some());
        Self {
            top: self.top.saturating_sub(margin),
            end: (self.end + margin).min(WIDTH),
        }
    }
}

/// Two independent spans per stripe let a note jump without sending the grid between traces.
pub(super) struct Damage([[Span; 2]; WIDTH / STRIPE]);

impl Damage {
    pub(super) fn full(rows: Range<usize>, overlay: Option<Window>) -> Self {
        Self(core::array::from_fn(|i| {
            let mut span = Span {
                top: rows.start,
                end: rows.end,
            };
            if let Some(list) = overlay
                && i * STRIPE < list.x + list.width
                && (i + 1) * STRIPE > list.x
            {
                span = span.union(Span {
                    top: list.y,
                    end: list.y + list.height,
                });
            }
            [span, Span::EMPTY]
        }))
    }

    pub(super) fn windows(&self) -> impl Iterator<Item = Window> + '_ {
        let mut x = 0;
        self.0.chunk_by(|a, b| a == b).flat_map(move |group| {
            let left = x;
            let width = group.len() * STRIPE;
            x += width;
            group[0]
                .into_iter()
                .filter(|s| s.top < s.end)
                .map(move |s| Window {
                    x: left,
                    y: s.top,
                    width,
                    height: s.end - s.top,
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
                top,
                end: bottom + 1,
            },
            palette.pens[age],
        )
    }

    pub(super) fn damage(&self, previous: &Self, palette: &Palette) -> Damage {
        Damage(core::array::from_fn(|i| {
            let mut old = Span::EMPTY;
            let mut new = Span::EMPTY;
            for x in i * STRIPE..(i + 1) * STRIPE {
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
            if new.top <= old.end + 2 {
                [old.union(new), Span::EMPTY]
            } else {
                [old, new]
            }
        }))
    }

    pub(super) fn render(&self, w: Window, palette: &Palette, pixels: &mut [Pixel]) {
        let grid_row = &palette.row[w.x..w.x + w.width];
        for (dy, row) in pixels.chunks_exact_mut(w.width).enumerate() {
            let y = w.y + dy;
            if y == WIDTH / 2 {
                row.fill(palette.center);
            } else if y.is_multiple_of(usize::from(GRID)) {
                row.fill(palette.grid);
            } else {
                row.copy_from_slice(grid_row);
            }
        }
        let end = w.y + w.height;
        for dx in 0..w.width {
            let (span, pen) = self.column(w.x + dx, palette);
            for y in span.top.max(w.y)..span.end.min(end) {
                pixels[(y - w.y) * w.width + dx] = pen.ink;
            }
            if let Some(halo) = pen.halo {
                for y in [span.top.checked_sub(1), Some(span.end)]
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
        let palette = palette();
        let flat = [120; WIDTH];
        let saw = core::array::from_fn(|x| 34 + u8::try_from(x % 171).expect("bounded ramp"));
        let square = core::array::from_fn(|x| if (x / 24) % 2 == 0 { 205 } else { 34 });
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
                || Damage::full(32..208, None),
                |old| sweep.damage(old, &palette),
            );
            let mut touched = vec![false; lcd.len()];
            for w in damage.windows().flat_map(|w| w.strips(CAPACITY)) {
                assert!(w.width * w.height <= CAPACITY);
                let mut strip = vec![OUTSIDE; w.width * w.height];
                sweep.render(w, &palette, &mut strip);
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
                let paper = if !(32..208).contains(&y) {
                    OUTSIDE
                } else if y == 120 {
                    CENTER
                } else if x % 24 == 0 || y % 24 == 0 {
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
            .damage(&old, &palette)
            .windows()
            .map(|w| (w.x, w.y, w.width, w.height))
            .collect();
        assert_eq!(windows, [(0, 40, WIDTH, 1), (0, 200, WIDTH, 1)]);
        assert_eq!(new.damage(&new, &palette).windows().count(), 0);
    }
}
