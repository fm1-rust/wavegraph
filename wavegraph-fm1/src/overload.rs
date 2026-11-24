//! Audio deadline misses captured by the DMA task and read by the LCD and USB tasks.

use core::fmt::Write;
use heapless::String;

const INDICATOR_TICKS: u32 = 100; // One second on the 100 Hz panel clock.

/// One complete timing snapshot fills the BSP's 43 printable USB log bytes.
/// Decimal microseconds match the LCD; hexadecimal counters keep all 32 bits.
pub(super) fn timing_line(
    boot: u32,
    worker: u32,
    refill: u32,
    commit: u32,
    overruns: u32,
    silences: u32,
) -> String<43> {
    let us = |ticks: u32| ticks.div_ceil(24).min(99_999);
    let boot = char::from_digit(boot, 16).unwrap_or('?');
    let mut line = String::new();
    write!(
        line,
        "B{boot} D{:05} F{:05} W{:05} O{overruns:08X} S{silences:08X}",
        us(worker),
        us(refill),
        us(commit),
    )
    .expect("fixed-width timing snapshot");
    line
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Severity {
    Amber,
    Red,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Overrun {
    pub(super) number: u32,
    at: u32,
    work: u32,
    gap: u32,
    dma_late: bool,
}

impl Overrun {
    pub(super) fn line(self) -> String<43> {
        let mut line = String::new();
        let ms = u64::from(self.at) * 10;
        // TIMER4 runs at 24 MHz. Cap unusually long gaps to keep the whole
        // event in one BSP USB-MIDI log line even after billions of misses.
        let micros = |ticks: u32| (ticks / 24).min(99_999);
        write!(
            line,
            "XR{} T{ms} W{} G{}",
            self.number,
            micros(self.work),
            micros(self.gap)
        )
        .expect("bounded decimal fields");
        if self.dma_late {
            line.push_str(" DMA").expect("fits one log line");
        }
        line
    }
}

/// Recent audio deadline misses, shared by the firmware tasks.
#[derive(Clone, Copy, Debug, Default)]
pub struct Overloads {
    // Two short lines fit one USB log transfer. Always keep the newest event.
    recent: [Overrun; 2],
}

impl Overloads {
    pub(super) fn record(&mut self, number: u32, at: u32, work: u32, gap: u32, dma_late: bool) {
        self.recent = [
            self.recent[1],
            Overrun {
                number,
                at,
                work,
                gap,
                dma_late,
            },
        ];
    }

    pub(super) fn severity(&self, now: u32) -> Option<Severity> {
        let [previous, last] = self.recent;
        if last.number == 0 || now.wrapping_sub(last.at) >= INDICATOR_TICKS {
            None
        } else if previous.number != 0 && last.at.wrapping_sub(previous.at) < INDICATOR_TICKS {
            Some(Severity::Red)
        } else {
            Some(Severity::Amber)
        }
    }

    pub(super) fn events(&self) -> impl Iterator<Item = Overrun> + '_ {
        self.recent.iter().copied().filter(|e| e.number != 0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn timing_snapshot_always_fits_one_reply_and_rounds_up_like_the_lcd() {
        assert_eq!(
            timing_line(1, 24, 25, 0, 15, 2),
            "B1 D00001 F00002 W00000 O0000000F S00000002"
        );
        assert_eq!(
            timing_line(u32::MAX, u32::MAX, u32::MAX, u32::MAX, u32::MAX, u32::MAX),
            "B? D99999 F99999 W99999 OFFFFFFFF SFFFFFFFF"
        );
    }

    #[test]
    fn dot_expires_and_repeated_misses_turn_it_red_across_tick_wrap() {
        let mut health = Overloads::default();
        assert_eq!(health.severity(0), None);
        health.record(1, u32::MAX - 10, 48_000, 69_632, false);
        assert_eq!(health.severity(0), Some(Severity::Amber));
        health.record(2, 1, 70_000, 69_700, true);
        assert_eq!(health.severity(50), Some(Severity::Red));
        assert_eq!(health.severity(101), None);
        health.record(3, 150, 48_000, 69_632, false);
        assert_eq!(health.severity(150), Some(Severity::Amber));
        let mut events = health.events();
        assert_eq!(events.next().unwrap().line(), "XR2 T10 W2916 G2904 DMA");
        assert_eq!(events.next().unwrap().line(), "XR3 T1500 W2000 G2901");
    }

    #[test]
    fn worst_case_record_fits_one_usb_log_line() {
        let mut health = Overloads::default();
        health.record(u32::MAX, u32::MAX, u32::MAX, u32::MAX, true);
        assert_eq!(
            health.events().last().unwrap().line(),
            "XR4294967295 T42949672950 W99999 G99999 DMA"
        );
    }
}
