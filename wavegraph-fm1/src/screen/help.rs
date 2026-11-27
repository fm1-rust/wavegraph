//! Momentary, read-only guide to the controls on the current page.

use super::{BAND, Page, Screen, mixer};
use crate::{
    firmware::Labels,
    fx::BeatEffect,
    instrument::Part,
    panel::{PresetRole, View},
    sequencer::SeqPage,
};
use fm1_graphics::{
    embedded_graphics::{prelude::Point, text::Alignment},
    palette,
};
use fm1_rticx::{
    bsp::display::{Display, Error, SIZE},
    time::Clock,
};

fn card(labels: &Labels) -> (&'static str, &'static [&'static str]) {
    if matches!(labels.preset_role, PresetRole::Audition(_)) {
        return (
            "ASSIGN INSTRUMENT",
            &[
                "PRESETS   Browse instruments",
                "Pause to hear the highlighted one",
                "Keys      Play the focused strip",
                "EDIT      Keep choice and exit",
            ],
        );
    }
    match labels.view {
        View::Scope if labels.focus == Part::Bass => (
            if labels.bass_control.is_some() {
                "ALGO      Edit selected control"
            } else {
                "ALGO      Recall a 303 sound"
            },
            &[
                "PRESETS   Choose channel 1-4",
                "SELECT    Choose a 303 dial",
                "HOME      Clear dial selection",
                "Hold EDIT to assign an instrument",
                "Tap GLO   Open four-strip mixer",
                "Hold GLO  Mix while you play",
                "FX        Open Beat FX",
            ],
        ),
        View::Scope => {
            let local = match labels.focus {
                Part::Bass | Part::Hoover | Part::Fm => "ALGO      Recall a sound",
                Part::Mono => "ALGO      Oscillator waveform",
                Part::Organ => "ALGO      Organ registration",
                Part::Wavetable => "ALGO      Wavetable bank",
                Part::Sampler => "ALGO      Sample or clipboard",
                Part::Supersaw => "K3 / K4   Detune / saw mix",
                Part::Tr808 | Part::Tr909 => "Keys      Play drum voices",
            };
            (
                local,
                &[
                    "PRESETS   Choose channel 1-4",
                    "Hold EDIT to assign an instrument",
                    "Tap GLO   Open four-strip mixer",
                    "Hold GLO  Mix while you play",
                    "FX / ENV  Beat FX / envelope",
                    "ARP / LFO Arpeggiator / modulation",
                ],
            )
        }
        View::Mixer => (
            "MIXER / LEVELS",
            &[
                "K1-4      Channel levels 1-4",
                "PRESETS   Focus a channel",
                "SEL       Channel on/off",
                "SELECT    Edit one strip",
                "Hold EDIT to assign an instrument",
                "EDIT      Clipboard or tape loop",
            ],
        ),
        View::Channel => (
            "CHANNEL STRIP",
            &[
                "K1        Level",
                "K2 / K3   Reverb / echo sends",
                "PRESETS   Focus a channel",
                "SEL       Channel on/off",
                "Hold EDIT to assign an instrument",
                "HOME      Back to four faders",
            ],
        ),
        View::Fx if labels.insert_slot.is_some() => (
            "CHANNEL FX / INSERT",
            &[
                "SELECT    Choose insert slot",
                "PRESETS   Choose channel",
                "ALGO      Assign effect to slot",
                "K1-4      Effect parameters",
                "SEL       Bypass selected insert",
                "FX        Open Beat FX",
                "HOME      Return to sends",
            ],
        ),
        View::Fx => (
            "CHANNEL FX",
            &[
                "PRESETS   Choose channel",
                "SELECT    Choose insert slot",
                "K2 / K3   Reverb / echo sends",
                "FX        Open Beat FX",
                "GLO       Open master FX",
            ],
        ),
        View::MasterFx(None) => (
            "MASTER FX",
            &[
                "SELECT    Choose rack device",
                "ALGO left   Half output (-6 dB)",
                "ALGO right  Full output (0 dB)",
                "EDIT      Open Beat FX",
                "FX        Return to mixer",
                "GLO       Return to FX bank",
            ],
        ),
        View::MasterFx(Some(_)) => (
            "MASTER FX DEVICE",
            &[
                "SELECT    Choose rack device",
                "K1-4      Edit device",
                "SEL       Bypass device",
                "EDIT      Open Beat FX",
                "HOME      Back to master rack",
            ],
        ),
        View::BeatFx(effect) => {
            let title = match effect {
                BeatEffect::Echo => "SHARED ECHO / WHITE KEYS: SEND STEPS",
                BeatEffect::Slicer => "SLICER / 16 WHITE KEYS: TOGGLE STEPS",
                BeatEffect::Filter => "PCF / PATTERN CONTROLLED FILTER",
                BeatEffect::Flanger => "FLANGER / SEL: APPLY OR BYPASS",
            };
            let lines: &'static [&'static str] = if effect == BeatEffect::Filter {
                &[
                    "16 white  Toggle trigger steps",
                    "Black 1-8 Recall rhythm and enable",
                    "Black 9/10  Low-pass / band-pass",
                    "K1 Beats  K2 Cutoff  K3 Res  K4 Env",
                    "ALGO: Decay       SEL: On/off",
                    "SELECT: Effect    PRESETS: Channel",
                    "FX: Channel FX    GLO: Master FX",
                ]
            } else {
                &[
                    "SELECT    Choose effect",
                    "PRESETS   Choose channel",
                    "K1        Shared beat division",
                    "K2-4      Edit shown parameters",
                    "SEL       Apply, bypass or gate Echo",
                    "FX        Open Channel FX",
                    "GLO  Master FX        HOME  Back",
                ]
            };
            (title, lines)
        }
        View::Clipboard => (
            "PCM CLIPBOARD",
            &[
                "PRESETS   Source channel",
                "K1 / K2   Length / root note",
                "K3        Sample or tape loop",
                "REC       Arm or finish capture",
                "EDIT      Return to mixer",
            ],
        ),
        View::VoiceMode => (
            "VOICE MODE",
            &[
                "SELECT    Poly, chord or unison",
                "ALGO / K1 Chord in chord mode",
                "SEL       Next mode",
                "PRESETS   Choose channel",
                "EDIT      Return to instrument",
            ],
        ),
        View::SampleEdit => (
            "SAMPLE EDIT",
            &[
                "ALGO      Choose sample",
                "K1        Playback mode",
                "K2 / K3   Region start / end",
                "K4        Keyboard tracking",
                "EDIT      Return to instrument",
            ],
        ),
        View::Envelope if labels.focus == Part::Bass => (
            "303 ENVELOPE",
            &[
                "SELECT    Choose panel control",
                "ALGO      Sound or selected value",
                "HOME      Clear selection",
                "K1-4      Cutoff, res, env, decay",
            ],
        ),
        View::Envelope if labels.focus == Part::Hoover => (
            "HOOVER ENVELOPE",
            &[
                "SELECT / SEL  Next parameter bank",
                "K1-4          Edit shown parameters",
                "PRESETS       Choose channel",
                "ENV           Return to instrument",
            ],
        ),
        View::Envelope => (
            "ENVELOPE",
            &[
                "K1-4      Edit displayed controls",
                "PRESETS   Choose channel",
                "ENV       Return to instrument",
            ],
        ),
        View::Lfo => (
            "LFO",
            &[
                "K1-4      Shape, rate, depth, target",
                "SELECT    Free or synced rate",
                "SEL       Enable / bypass",
                "PRESETS   Choose channel",
            ],
        ),
        View::Arp => (
            "ARPEGGIATOR",
            &[
                "K1 / K2   Order / rate",
                "K3 / K4   Octaves / gate",
                "SELECT    Latch on / off",
                "SEL       Enable / bypass",
                "PRESETS   Choose channel",
            ],
        ),
        View::Sequencer => match labels.seq.page {
            Some(SeqPage::Step) => (
                "STEP EDIT",
                &[
                    "K1 / K2   Step / bar",
                    "Keys      Write at cursor",
                    "SEL + key Erase",
                    "EDIT      Back to pattern",
                ],
            ),
            Some(SeqPage::Tempo) => ("TEMPO", &["K3        Set BPM", "HOME      Back to pattern"]),
            Some(SeqPage::Record) => (
                "NOTE RECORDING",
                &[
                    "Keys      Record into pattern",
                    "REC       End note recording",
                    "HOME      Back",
                ],
            ),
            Some(SeqPage::Pattern) | None => (
                "PATTERN BANK",
                &[
                    "PRESETS   Choose channel",
                    "White 1-8 Choose pattern",
                    "SEL       Pattern mute",
                    "SELECT    Tempo page",
                    "EDIT      Step editor",
                    "REC       Record notes",
                ],
            ),
        },
    }
}

impl Screen {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "At most seven guide lines fit within the panel"
    )]
    pub(crate) async fn help(
        &mut self,
        lcd: &mut Display,
        clock: &mut Clock,
        labels: &Labels,
    ) -> Result<(), Error> {
        if self.page == Some(Page::Help) {
            return Ok(());
        }
        self.badge = None;
        self.selector.select(None, None);
        let (title, lines) = card(labels);
        let mut lcd = lcd.as_async(clock)?;
        for y in (0..SIZE).step_by(BAND) {
            let mut band = mixer::Band::new(self.strip, 0, y, SIZE, BAND as u16, mixer::BACKGROUND);
            band.title("PANEL GUIDE", Point::new(8, 5), palette::WHITE);
            band.label(
                labels.channel.label(),
                Point::new(208, 7),
                Alignment::Right,
                mixer::LABEL,
            );
            band.label(title, Point::new(8, 33), Alignment::Left, palette::CYAN);
            for (i, line) in lines.iter().enumerate() {
                band.label(
                    line,
                    Point::new(8, 60 + 23 * i as i32),
                    Alignment::Left,
                    palette::WHITE,
                );
            }
            band.label(
                "RELEASE HOME TO RETURN",
                Point::new(8, 222),
                Alignment::Left,
                mixer::LABEL,
            );
            lcd.write_rect_buffered(0, y, SIZE, BAND as u16, band.pixels.as_flattened())
                .await?;
        }
        lcd.flush().await?;
        self.page = Some(Page::Help);
        Ok(())
    }
}
