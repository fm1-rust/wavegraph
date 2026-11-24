use super::*;
use crate::panel::PageButton;
extern crate alloc;
use alloc::vec::Vec as Output;
use wavegraph_arp::{Division, Mode};

fn play<const N: usize>(
    keys: &mut Keyboard<N>,
    rack: &mut Instrument<'_>,
    buses: &mut [[I24; 128]; BUS_COUNT],
) -> Result<Output<(Channel, Event<MidiMessage>)>, Error> {
    keys.snapshot();
    for channel in Channel::ALL {
        rack.sync_chord(
            channel,
            &keys.tracks[channel.index()],
            keys.keys.chord(channel),
        );
    }
    let mut out = Output::new();
    rack.render_with_midi(&keys.tracks, buses, |channel, event| {
        out.push((channel, event))
    })?;
    Ok(out)
}

#[test]
fn page_controls_are_local_to_each_channel_and_survive_navigation() {
    let mut keys = Keyboard::<8>::new();
    keys.nav.press(PageButton::Arp);
    assert_eq!(keys.nav.view, View::Arp);
    let settings = &mut keys.tracks[0].arp;
    settings.edit(1, [2, -1, 2, 40], true);
    assert!(settings.enabled && settings.config.latch);
    assert_eq!(settings.config.mode, Mode::UpDown);
    assert_eq!(settings.config.division, Division::EighthTriplet);
    assert_eq!((settings.config.octaves, settings.config.gate), (3, 100));
    keys.nav.turn(0, 1);
    assert_eq!((keys.nav.view, keys.nav.focus), (View::Arp, Channel::Two));
    assert_eq!(keys.tracks[1].arp, arp::Controls::INITIAL);
    keys.nav.press(PageButton::Home);
    keys.nav.turn(0, -1);
    keys.nav.press(PageButton::Arp);
    assert!(keys.tracks[keys.nav.focus.index()].arp.enabled);
    keys.nav.press(PageButton::Arp);
    assert_eq!(keys.nav.view, View::Scope);
}

#[test]
fn panel_and_usb_chords_emit_on_their_own_channels_and_release_on_bypass_or_swap()
-> Result<(), Error> {
    with_instrument(8_000, |rack| {
        let mut keys = Keyboard::<8>::new();
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        rack.transport.bpm = 120;
        for t in &mut keys.tracks[..2] {
            t.arp.edit(0, [0, 0, 0, 40], true);
        }
        keys.tracks[0].voice.transpose = 12;
        // Start from a panel C and USB E on the monosynth, plus an independent organ.
        keys.press(note(60, 93));
        keys.keys.midi(
            Channel::One,
            MidiAction::On(note(64, 85)),
            &mut keys.tracks[0],
        );
        keys.keys.midi(
            Channel::Two,
            MidiAction::On(note(67, 66)),
            &mut keys.tracks[1],
        );
        let first = play(&mut keys, rack, &mut buses)?;
        assert_eq!(
            first
                .iter()
                .map(|(c, e)| (*c, e.message))
                .collect::<Output<_>>(),
            [
                (Channel::One, MidiAction::On(note(72, 93))),
                (Channel::Two, MidiAction::On(note(67, 66)))
            ]
        );
        assert_eq!(rack.arp_note(Channel::One), Some(72));
        assert!(
            buses[Channel::One.source_bus()]
                .iter()
                .any(|s| s.inner() != 0)
        );
        for _ in 0..6 {
            assert!(play(&mut keys, rack, &mut buses)?.is_empty());
        }
        let step = play(&mut keys, rack, &mut buses)?;
        assert_eq!(
            step[..2],
            [
                (
                    Channel::One,
                    Event {
                        frame: 104,
                        port: 0,
                        message: MidiAction::Off(72)
                    }
                ),
                (
                    Channel::One,
                    Event {
                        frame: 104,
                        port: 0,
                        message: MidiAction::On(note(76, 85))
                    }
                )
            ]
        );
        keys.tracks[0].arp.enabled = false;
        keys.tracks[1] = Track::new(Assignment {
            part: Part::Bass,
            generation: 1,
        });
        keys.keys.clear(Channel::Two);
        let ends = play(&mut keys, rack, &mut buses)?;
        assert_eq!(
            ends.iter()
                .map(|(c, e)| (*c, e.message))
                .collect::<Output<_>>(),
            [
                (Channel::One, MidiAction::Off(76)),
                (Channel::Two, MidiAction::Off(67))
            ]
        );
        assert!(rack.events[0].iter().any(|e| matches!(
            e.message,
            Message::Mono(MonoMessage::NoteOn { note: 76, .. })
        )));
        assert!(rack.arp_note(Channel::One).is_none());
        Ok(())
    })
}

#[test]
fn a_latched_kit_owns_only_its_lane_and_panic_preserves_held_panel_keys() -> Result<(), Error> {
    with_instrument(8_000, |rack| {
        let mut keys = Keyboard::<8>::new();
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        keys.nav.focus = Channel::Four;
        let i = Channel::Four.index();
        keys.tracks[i].arp.edit(1, [0, 0, 0, 40], true);
        keys.keys.set_latch(Channel::Four, true);
        rack.transport.toggle();
        keys.press(note(53, 101)); // first panel pad is kit note 36
        keys.keys.midi(
            Channel::Four,
            MidiAction::On(note(42, 80)),
            &mut keys.tracks[i],
        );
        let out = play(&mut keys, rack, &mut buses)?;
        assert!(
            out.iter()
                .any(|(c, e)| *c == Channel::Four && e.message == MidiAction::On(note(36, 101)))
        );
        assert!(
            out.iter()
                .any(|(c, e)| *c != Channel::Four && matches!(e.message, MidiAction::On(_)))
        );
        assert_eq!(
            rack.events[i]
                .iter()
                .filter(|e| matches!(e.message, Message::Drum(DrumMessage::Trigger(..))))
                .count(),
            1
        );
        keys.keys.midi(
            Channel::Four,
            MidiAction::Control(123, 0),
            &mut keys.tracks[i],
        );
        play(&mut keys, rack, &mut buses)?;
        assert_eq!(rack.arp_note(Channel::Four), Some(36));
        keys.release(53);
        play(&mut keys, rack, &mut buses)?;
        assert_eq!(rack.arp_note(Channel::Four), Some(36));
        keys.keys.clear_midi();
        play(&mut keys, rack, &mut buses)?;
        assert_eq!(rack.arp_note(Channel::Four), Some(36));
        keys.keys.set_latch(Channel::Four, false);
        let end = play(&mut keys, rack, &mut buses)?;
        assert!(
            end.iter()
                .any(|(c, e)| *c == Channel::Four && e.message == MidiAction::Off(36))
        );
        assert_eq!(rack.arp_note(Channel::Four), None);
        assert!(rack.position.is_some());
        Ok(())
    })
}
