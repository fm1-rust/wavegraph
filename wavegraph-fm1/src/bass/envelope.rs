//! A silent reference phrase for comparing the filter, accent sweep and VCA.

use super::Controls;
use wavegraph_303::{EnvelopeLevels, Envelopes};

pub(crate) const WIDTH: usize = 192;
pub(crate) const HEIGHT: u8 = 40;
// Four milliseconds per column. The three accents are 96 ms apart.
pub(crate) const STRIKES: [usize; 4] = [12, 84, 108, 132];
const GATE_ENDS: [usize; 4] = [54, 98, 122, 146];

pub(crate) fn traces(controls: Controls) -> [[u8; WIDTH]; 3] {
    let params = controls.voice_params();
    // Sixteen envelope samples per column resolve the 12 ms charging transient.
    let mut envelopes = Envelopes::new(4000.0);
    let mut levels = envelopes.advance(&params, 0);
    let mut traces = [[0; WIDTH]; 3];
    for x in 0..WIDTH {
        if STRIKES.contains(&x) {
            envelopes.note_on(x != STRIKES[0], false);
            levels = envelopes.advance(&params, 0);
        } else if GATE_ENDS.contains(&x) {
            envelopes.note_off();
        }
        for (trace, row) in traces.iter_mut().zip(rows(levels)) {
            trace[x] = row;
        }
        levels = envelopes.advance(&params, 16);
    }
    traces
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Each normalized control signal maps into the 40-pixel lane with a one-pixel margin."
)]
fn rows(levels: EnvelopeLevels) -> [u8; 3] {
    let signals = [
        // Eight octaves from 62.5 Hz to the voice's 16 kHz ladder limit.
        micromath::F32Ext::log2(levels.cutoff_hz / 62.5) / 8.0,
        levels.accent_octaves / 3.0,
        levels.vca_gain / 1.9,
    ];
    signals.map(|v| (1.5 + (1.0 - v.clamp(0.0, 1.0)) * f32::from(HEIGHT - 3)) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bass::Knob;

    #[test]
    fn preview_is_within_one_pixel_of_the_fm1_audio_rate() {
        // The FM-1 runs at 24 MHz / 544, or 3000/17 frames per four-millisecond column.
        for delta in [i8::MIN, 0, i8::MAX] {
            let mut controls = Controls::INITIAL;
            for knob in [Knob::Decay, Knob::Resonance, Knob::Accent, Knob::Attack] {
                controls.turn(knob, delta);
            }
            let preview = traces(controls);
            let params = controls.voice_params();
            let mut audio = Envelopes::new(24_000_000.0 / 544.0);
            let mut level = audio.advance(&params, 0);
            let mut phase = 0;
            for x in 0..WIDTH {
                if STRIKES.contains(&x) {
                    audio.note_on(x != STRIKES[0], false);
                    level = audio.advance(&params, 0);
                } else if GATE_ENDS.contains(&x) {
                    audio.note_off();
                }
                for (trace, row) in preview.iter().zip(rows(level)) {
                    assert!(trace[x].abs_diff(row) <= 1, "column {x}, detents {delta}");
                }
                phase += 3000;
                level = audio.advance(&params, phase / 17);
                phase %= 17;
            }
        }
    }

    #[test]
    fn panel_edits_change_their_own_signals() {
        let base = Controls::INITIAL;
        let [filter, accent, amp] = traces(base);
        let edit = |knob, delta| {
            let mut controls = base;
            controls.turn(knob, delta);
            traces(controls)
        };
        let [long, _, _] = edit(Knob::Decay, 80);
        assert_ne!(filter[..STRIKES[1]], long[..STRIKES[1]]);
        assert_eq!(filter[STRIKES[1]..], long[STRIKES[1]..]);
        let [deep, same_accent, same_amp] = edit(Knob::EnvMod, 24);
        assert_ne!(filter, deep);
        assert_eq!((same_accent, same_amp), (accent, amp));
        let [low, _, _] = edit(Knob::Cutoff, -12);
        assert_ne!(low[0], filter[0]);
        let [_, silent, flat_gain] = edit(Knob::Accent, -100);
        assert!(silent.iter().all(|&y| y == silent[0]));
        assert_eq!(amp[..STRIKES[1]], flat_gain[..STRIKES[1]]);
        assert_ne!(amp, flat_gain);
        let [_, slower_drain, _] = edit(Knob::Resonance, 50);
        assert_ne!(accent, slower_drain);
        assert!(slower_drain[WIDTH - 1] < accent[WIDTH - 1]);
        let [same_filter, same_accent, slower_amp] = edit(Knob::Attack, 12);
        assert_eq!((same_filter, same_accent), (filter, accent));
        assert_ne!(amp, slower_amp);
        for knob in [Knob::Tuning, Knob::Overdrive] {
            assert_eq!(edit(knob, 10), [filter, accent, amp]);
        }
    }
}
