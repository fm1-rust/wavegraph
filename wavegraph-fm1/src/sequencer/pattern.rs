//! Original four-bar factory sketch. These notes and curves are shared by
//! playback and the SEQ view, so the display describes the events being played.

use crate::{instrument::Part, panel::Channel};
use wavegraph_909::Voice;

pub(crate) const STEPS: usize = 64;

/// One bar of quarter-note kicks, a layered backbeat and short offbeat hats.
pub(crate) fn drums(part: Part, step: usize) -> &'static [Voice] {
    if part == Part::Tr808 {
        // Answer the 909 on unoccupied sixteenths. Leave its downbeats,
        // backbeats and offbeat hats clear. A brief open hat leads into bar one.
        return match step % STEPS {
            7 | 29 | 39 | 61 => &[Voice::Kick],
            15 | 25 | 47 | 57 => &[Voice::Snare],
            3 | 11 | 19 | 35 | 43 | 51 => &[Voice::ClosedHat],
            23 | 55 | 63 => &[Voice::OpenHat],
            _ => &[],
        };
    }
    match step % 8 {
        0 => &[Voice::Kick],
        4 => &[Voice::Kick, Voice::Snare, Voice::Clap],
        2 | 6 => &[Voice::ClosedHat],
        _ => &[],
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Step {
    pub(crate) notes: &'static [u8],
    pub(crate) velocity: u8,
    /// Gate duration in eighths of a sixteenth note.
    pub(crate) gate: u8,
    /// Keep this bass note's gate open and slide into the following step.
    pub(crate) slide: bool,
}

const REST: Step = Step {
    notes: &[],
    velocity: 0,
    gate: 0,
    slide: false,
};

const fn note(notes: &'static [u8], velocity: u8, gate: u8, slide: bool) -> Step {
    Step {
        notes,
        velocity,
        gate,
        slide,
    }
}

const MONO: [Step; STEPS] = {
    let mut pattern = [REST; STEPS];
    pattern[13] = note(&[69], 38, 9, false);
    pattern[29] = note(&[72], 34, 7, false);
    pattern[45] = note(&[67], 38, 9, false);
    pattern[61] = note(&[71], 34, 7, false);
    pattern
};

// Leave long upper-register gates so the detuned saws have time to beat.
const SUPER_SAW: [Step; STEPS] = {
    let mut p = [REST; STEPS];
    p[0] = note(&[69], 100, 36, false);
    p[10] = note(&[65], 86, 12, false);
    p[16] = note(&[72], 98, 42, false);
    p[27] = note(&[69], 90, 16, false);
    p[32] = note(&[67], 100, 36, false);
    p[42] = note(&[64], 86, 12, false);
    p[48] = note(&[71], 98, 42, false);
    p[59] = note(&[67], 90, 16, false);
    p
};

// Rootless Dm7 and Cmaj7. Put the third on top, clear of the acid bass,
// and alternate held chords with shorter answers.
const ORGAN: [Step; STEPS] = {
    let mut pattern = [REST; STEPS];
    pattern[2] = note(&[57, 60, 65], 118, 36, false);
    pattern[11] = note(&[57, 60, 65], 104, 15, false);
    pattern[21] = note(&[57, 60, 65], 116, 44, false);
    pattern[34] = note(&[55, 59, 64], 118, 36, false);
    pattern[43] = note(&[55, 59, 64], 104, 15, false);
    pattern[57] = note(&[55, 59, 64], 116, 36, false);
    pattern
};

// Each outgoing slide is followed by a real note, with no intervening gate-off.
// Accents are deliberately sparse even when the automation gets aggressive.
const BASS: [Step; STEPS] = {
    let mut pattern = [REST; STEPS];
    pattern[0] = note(&[38], 116, 5, false);
    pattern[3] = note(&[38], 78, 5, false);
    pattern[6] = note(&[45], 82, 8, true);
    pattern[7] = note(&[48], 80, 5, false);
    pattern[10] = note(&[38], 82, 6, false);
    pattern[14] = note(&[50], 114, 8, true);
    pattern[15] = note(&[48], 80, 5, false);
    pattern[16] = note(&[38], 108, 5, false);
    pattern[19] = note(&[38], 79, 5, false);
    pattern[22] = note(&[45], 83, 8, true);
    pattern[23] = note(&[50], 82, 6, false);
    pattern[26] = note(&[38], 86, 5, false);
    pattern[30] = note(&[48], 110, 6, false);
    pattern[32] = note(&[36], 116, 5, false);
    pattern[35] = note(&[36], 78, 5, false);
    pattern[38] = note(&[43], 82, 8, true);
    pattern[39] = note(&[47], 80, 5, false);
    pattern[42] = note(&[36], 82, 6, false);
    pattern[46] = note(&[48], 114, 8, true);
    pattern[47] = note(&[47], 80, 5, false);
    pattern[48] = note(&[36], 108, 5, false);
    pattern[51] = note(&[36], 79, 5, false);
    pattern[54] = note(&[43], 83, 8, true);
    pattern[55] = note(&[48], 82, 6, false);
    pattern[58] = note(&[36], 86, 5, false);
    pattern[62] = note(&[38], 112, 6, false);
    pattern
};

// Break the former organ's Dm7 and Cmaj7 voicings into a second monophonic
// phrase. Tied pairs use the 303's held gate; rests leave space for the sends.
const HARMONY: [Step; STEPS] = {
    let mut p = [REST; STEPS];
    p[2] = note(&[57], 99, 5, false);
    p[5] = note(&[60], 78, 8, true);
    p[6] = note(&[65], 84, 5, false);
    p[11] = note(&[60], 104, 4, false);
    p[17] = note(&[57], 88, 4, false);
    p[21] = note(&[60], 102, 8, true);
    p[22] = note(&[65], 78, 7, false);
    p[27] = note(&[69], 86, 4, false);
    p[34] = note(&[55], 100, 5, false);
    p[37] = note(&[59], 81, 8, true);
    p[38] = note(&[64], 83, 5, false);
    p[43] = note(&[59], 99, 4, false);
    p[49] = note(&[55], 88, 4, false);
    p[53] = note(&[59], 105, 8, true);
    p[54] = note(&[64], 82, 7, false);
    p[59] = note(&[55], 85, 4, false);
    p
};

pub(crate) fn step(part: Part, channel: Channel, index: usize) -> Step {
    let index = index % STEPS;
    match part {
        Part::Mono => MONO[index],
        Part::Supersaw => SUPER_SAW[index],
        Part::Organ => ORGAN[index],
        Part::Bass if channel == Channel::Two => HARMONY[index],
        Part::Bass => BASS[index],
        Part::Tr909 | Part::Tr808 => REST,
    }
}

/// Display units; playback retains fractional percentages and decibels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Automation {
    pub(crate) cutoff_hz: u16,
    pub(crate) resonance: u8,
    pub(crate) overdrive_db: u8,
}

// Half-bar control points. The repeated endpoint makes the loop continuous.
// Cutoff, resonance (%) and drive (dB) rise together, then fold back down.
const CURVES: [[u16; 3]; 9] = [
    [180, 76, 12],
    [380, 88, 18],
    [1200, 94, 27],
    [300, 80, 15],
    [220, 74, 14],
    [620, 89, 22],
    [1800, 93, 28],
    [540, 84, 20],
    [180, 76, 12],
];

// The upper square voice opens on the bass line's quieter half-bars.
const HARMONY_CURVES: [[u16; 3]; 9] = [
    [920, 72, 16],
    [620, 68, 14],
    [480, 66, 15],
    [1160, 76, 20],
    [840, 70, 16],
    [610, 69, 14],
    [450, 67, 15],
    [1360, 79, 22],
    [920, 72, 16],
];

/// Interpolate within one step, with `fraction` measured in units of 1/256
/// of a sixteenth note.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "The interpolated values stay within their positive u8/u16 keyframe ranges."
)]
pub(crate) fn automation(channel: Channel, step: usize, fraction: u8) -> Automation {
    let [cutoff, resonance, drive] = automation_values(channel, step, fraction);
    Automation {
        cutoff_hz: cutoff as u16,
        resonance: resonance as u8,
        overdrive_db: drive as u8,
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "The wrapped step is below 64 and interpolation stays between bounded keyframes."
)]
pub(super) fn automation_values(channel: Channel, step: usize, fraction: u8) -> [f32; 3] {
    let step = step % STEPS;
    let segment = step / 8;
    let curve = if channel == Channel::Two {
        &HARMONY_CURVES
    } else {
        &CURVES
    };
    let within = u16::try_from(step % 8).expect("within one segment") * 256 + u16::from(fraction);
    let t = f32::from(within) / 2048.0;
    core::array::from_fn(|lane| {
        let from = f32::from(curve[segment][lane]);
        let to = f32::from(curve[segment + 1][lane]);
        from + (to - from) * t
    })
}
