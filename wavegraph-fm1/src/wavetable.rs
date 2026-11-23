//! Assignable wavetable programs and their four FM-1 encoders.

use crate::envelope::EnvelopeControls;
use wavegraph_wavetable::{Mode, Scan};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Program {
    pub(super) scan: Scan,
    pub(super) position_envelope: EnvelopeControls,
}

impl Program {
    pub(super) const fn new(scan: Scan) -> Self {
        Self {
            scan,
            position_envelope: EnvelopeControls::WAVETABLE_POSITION,
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A signed detent times 50 or plus a mode index fits i16."
    )]
    pub(super) fn turn(&mut self, [start, end, time, mode]: [i8; 4]) {
        const MODES: [Mode; 5] = [
            Mode::Stepped,
            Mode::Smooth,
            Mode::OneShot,
            Mode::PingPong,
            Mode::Envelope,
        ];
        let scan = &mut self.scan;
        let i = (i16::from(scan.mode as u8) + i16::from(mode))
            .rem_euclid(i16::try_from(MODES.len()).expect("five scan modes"));
        scan.mode = MODES[usize::try_from(i).expect("wrapped scan mode")];
        scan.start = scan.start.saturating_add_signed(start).min(127);
        scan.end = scan.end.saturating_add_signed(end).min(127);
        if scan.mode == Mode::Envelope {
            self.position_envelope.turn([time, 0, 0, 0]);
        } else {
            scan.travel_ms = scan
                .travel_ms
                .saturating_add_signed(i16::from(time) * 50)
                .clamp(50, 8_000);
        }
    }
}
