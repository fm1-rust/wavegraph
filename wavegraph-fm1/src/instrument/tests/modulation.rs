use super::*;

#[test]
fn every_knob_destination_sends_native_changes_and_restores_the_panel() -> Result<(), Error> {
    with_instrument(44_118, |rack| {
        let mut tracks = Track::TEST_TRACKS;
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        rack.sequences.fill(false);
        for part in Part::ALL {
            if part == Part::Bass {
                continue;
            } // Native offsets are also exercised under clip automation.
            let channel = Channel::Two;
            let i = channel.index();
            tracks[i] = Track::factory(channel, Assignment::new(part));
            rack.render(tracks, &mut buses)?;
            for (slot, target) in part.lfo_targets().iter().enumerate() {
                let Route::Knob { index, span } = target.route else {
                    unreachable!()
                };
                let mut heard = false;
                for (wave, sign) in [(2, -1), (3, 1)] {
                    let mut base = tracks[i];
                    base.lfo = crate::lfo::Controls::INITIAL;
                    base.lfo.edit(
                        part,
                        0,
                        [wave, 0, 127, i8::try_from(slot).expect("short catalog")],
                        true,
                    );
                    tracks[i] = base;
                    let mut effective = base;
                    let mut knobs = [0; 4];
                    knobs[usize::from(index)] = sign * i8::try_from(span).expect("encoder range");
                    effective.turn_knobs(knobs);
                    let sent = effective.events(Some(base))?;
                    heard |= !sent.is_empty();
                    rack.render(tracks, &mut buses)?;
                    assert_eq!(
                        rack.events[i].as_slice(),
                        sent.as_slice(),
                        "{part:?} {}",
                        target.name
                    );
                    assert!(rack.events[0].is_empty());
                    assert_eq!(rack.previous.as_ref().expect("rendered")[i], base);
                    tracks[i].lfo.enabled = false;
                    rack.render(tracks, &mut buses)?;
                    assert_eq!(
                        rack.events[i].as_slice(),
                        base.events(Some(effective))?.as_slice(),
                        "restore {}",
                        target.name
                    );
                }
                assert!(heard, "{part:?} {} emits a parameter change", target.name);
            }
        }
        Ok(())
    })
}

#[test]
fn retarget_and_mod_wheel_zero_restore_the_latest_manual_value() -> Result<(), Error> {
    with_instrument(44_118, |rack| {
        let mut tracks = Track::TEST_TRACKS;
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        let mut keys = Keys::<1>::new();
        let part = Part::Supersaw;
        tracks[0] = Track::factory(Channel::One, Assignment::new(part));
        tracks[0].lfo.edit(part, 0, [3, 0, 127, 2], true);
        rack.render(tracks, &mut buses)?;
        tracks[0].turn_knobs([0, 0, 3, 0]);
        tracks[0].lfo.edit(part, 0, [0, 0, 0, 1], false);
        rack.render(tracks, &mut buses)?;
        assert!(rack.events[0].contains(&at(SawMessage::Detune(tracks[0].saw.detune))));
        assert!(rack.events[0].contains(&at(SawMessage::Mix(
            tracks[0].saw.mix.saturating_add(64).min(127)
        ))));
        keys.midi(Channel::One, MidiAction::Control(1, 0), &mut tracks[0]);
        rack.render(tracks, &mut buses)?;
        assert!(rack.events[0].contains(&at(SawMessage::Mix(tracks[0].saw.mix))));
        assert!(tracks[0].lfo.enabled);
        Ok(())
    })
}
