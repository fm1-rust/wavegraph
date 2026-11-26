//! Compare the optimized graph with the original per-block control conversion.
use super::*;
extern crate alloc;
use alloc::vec::Vec as Output;

#[derive(Debug, PartialEq)]
struct Capture {
    pcm: Output<i32>,
    midi: Output<(usize, Channel, Event<MidiMessage>)>,
    graph: Output<(usize, Channel, Event<Message>)>,
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "The fixed block range bounds the instrument sweep offsets and generations."
)]
fn capture<const CACHED: bool>() -> Result<Capture, Error> {
    let mut captured = Capture {
        pcm: Output::new(),
        midi: Output::new(),
        graph: Output::new(),
    };
    with_instrument(44_118, |rack| {
        let mut nav = Navigation::INITIAL;
        let mut tracks = Track::rack(nav.assignments);
        let mut keys = Keys::<8>::new();
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        let mut mixer = Mixer::INITIAL;
        let mut fx = Fx::INITIAL;
        rack.transport.toggle();
        for block in 0..700 {
            match block {
                20 => nav.turn_algorithm(2),      // Pump Panel and its channel effects.
                30..=36 => nav.turn_algorithm(1), // Every preset, including a full turn.
                50 => keys.midi(Channel::One, MidiAction::On(note(60, 112)), &mut tracks[0]),
                80 => {
                    keys.midi(Channel::One, MidiAction::Off(60), &mut tracks[0]);
                    keys.midi(Channel::One, MidiAction::On(note(60, 112)), &mut tracks[0]);
                }
                100 => keys.midi(Channel::One, MidiAction::Off(60), &mut tracks[0]),
                120 => tracks[0].lfo.edit(Part::Bass, 0, [1, 2, 30, 7], true),
                150 => tracks[0].bass.turn(Knob::Overdrive, -3),
                180 => tracks[0].lfo.enabled = false, // Send the native zero modulation.
                200 => {
                    keys.clear(Channel::Two);
                    tracks[1].reassign(
                        Channel::Two,
                        Assignment {
                            part: Part::Supersaw,
                            generation: 1,
                        },
                    );
                    keys.midi(Channel::Two, MidiAction::On(note(48, 90)), &mut tracks[1]);
                }
                210 => tracks[1].lfo.edit(Part::Supersaw, 0, [1, 2, 50, 2], true),
                240 => tracks[1].turn_knobs([0, 0, 5, 0]),
                260 => tracks[1].lfo.enabled = false, // Restore the edited detune center.
                280 => {
                    tracks[0].arp.enabled = true;
                    keys.midi(Channel::One, MidiAction::On(note(60, 102)), &mut tracks[0]);
                    keys.midi(Channel::One, MidiAction::On(note(67, 94)), &mut tracks[0]);
                }
                300 => tracks[0].arp.enabled = false, // Direct gate takes ownership back.
                320 | 660 => rack.skip_frames(256, |c, cc, value| {
                    captured.midi.push((
                        block,
                        c,
                        Event {
                            frame: 0,
                            port: 0,
                            message: MidiMessage::Control(cc, value),
                        },
                    ));
                })?,
                340 => keys.midi(Channel::One, MidiAction::Control(123, 0), &mut tracks[0]),
                360 | 380 => rack.transport.toggle(),
                _ => (),
            }
            if (400..640).contains(&block) {
                let slot = (block - 400) / 24;
                let channel = Channel::Two;
                if (block - 400) % 24 == 0 {
                    keys.clear(channel);
                    tracks[1].reassign(
                        channel,
                        Assignment {
                            part: Part::ALL[slot],
                            generation: u32::try_from(slot + 2).expect("short instrument sweep"),
                        },
                    );
                    keys.midi(channel, MidiAction::On(note(48, 109)), &mut tracks[1]);
                } else if (block - 400) % 24 == 12 {
                    keys.midi(channel, MidiAction::Off(48), &mut tracks[1]);
                }
            }
            for channel in Channel::ALL {
                let i = channel.index();
                if tracks[i].bass_preset != nav.bass_presets[i] {
                    let sound = PRESETS[usize::from(nav.bass_presets[i].index)];
                    sound.apply_beat_fx(channel, &mut fx, &mut mixer);
                    tracks[i].sync_panel(nav.bass_presets[i], nav.insert_kinds[i]);
                }
                keys.snapshot(channel, &mut tracks[i]);
                rack.sync_chord(channel, &tracks[i], keys.chord(channel));
            }
            rack.set_mixer(mixer);
            rack.set_fx(fx);
            rack.beat_steps = nav.beat_steps;
            rack.slicer_steps = nav.slicer_steps;
            let settings_changed = !CACHED
                || rack.previous.as_ref() != Some(&tracks)
                || rack.previous_mixer != Some(mixer)
                || rack.previous_fx != Some(fx);
            captured.pcm.extend(
                rack.render_controlled::<128, CACHED>(
                    &tracks,
                    settings_changed,
                    &mut buses,
                    |c, e| captured.midi.push((block, c, e)),
                )?
                .iter()
                .map(|s| s.inner()),
            );
            for channel in Channel::ALL {
                captured.graph.extend(
                    rack.events[channel.index()]
                        .iter()
                        .map(|e| (block, channel, *e)),
                );
            }
        }
        Ok(())
    })?;
    Ok(captured)
}

#[test]
fn control_cache_preserves_complete_audio_midi_and_graph_events() -> Result<(), Error> {
    let before = capture::<false>()?;
    let after = capture::<true>()?;
    assert!(before.pcm.iter().any(|&x| x != 0));
    assert!(!before.midi.is_empty());
    assert!(!before.graph.is_empty());
    assert_eq!(
        after.midi, before.midi,
        "MIDI after edits, arp takeover and skipped blocks"
    );
    assert_eq!(
        after.graph, before.graph,
        "ordered instrument events, including modulation restore"
    );
    assert_eq!(
        after.pcm, before.pcm,
        "every output sample across preset and instrument changes"
    );
    Ok(())
}
