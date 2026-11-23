//! Assignable wavetable programs and their four FM-1 encoders.

use wavegraph_wavetable::{Mode, Scan};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Controls {
    programs: [Scan; 2],
}

impl Controls {
    pub(super) const INITIAL: Self = Self {
        programs: [Scan::RESONANT, Scan::VOCAL],
    };

    pub(super) fn scan(self, bank: u8) -> Scan {
        self.programs[usize::from(bank)]
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A signed detent times 50 fits i16."
    )]
    pub(super) fn turn(&mut self, bank: u8, [start, end, time, mode]: [i8; 4]) {
        let scan = &mut self.programs[usize::from(bank)];
        scan.start = scan.start.saturating_add_signed(start).min(127);
        scan.end = scan.end.saturating_add_signed(end).min(127);
        scan.travel_ms = scan
            .travel_ms
            .saturating_add_signed(i16::from(time) * 50)
            .clamp(50, 8_000);
        const MODES: [Mode; 4] = [Mode::Stepped, Mode::Smooth, Mode::OneShot, Mode::PingPong];
        let i = MODES
            .iter()
            .position(|m| *m == scan.mode)
            .expect("four scan modes");
        scan.mode = MODES[i.wrapping_add_signed(isize::from(mode)) % MODES.len()];
    }
}
