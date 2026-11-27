use super::*;
use crate::panel::PageButton;
use wavegraph_303::Destination;

fn consume_presets(tracks: &mut [Track; CHANNELS], nav: Navigation) {
    for channel in Channel::ALL {
        let i = channel.index();
        tracks[i].sync_panel(nav.bass_presets[i], nav.insert_kinds[i]);
    }
}

#[test]
fn latest_snapshot_recalls_each_channel_after_skipped_frames_and_focus_changes() {
    let mut nav = Navigation::INITIAL;
    let mut tracks = Track::rack(nav.assignments);
    let drums = [tracks[2], tracks[3]];

    nav.turn_algorithm(1);
    nav.turn_algorithm(1); // Both snapshots are dropped; CH1 still reaches Pump Panel.
    nav.turn(0, 1);
    nav.turn_algorithm(2); // CH2 starts at Harmony, then reaches ReBirth Bite.
    consume_presets(&mut tracks, nav);

    assert_eq!(nav.focus, Channel::Two);
    for (i, expected) in [(0, 2), (1, 3)] {
        assert_eq!(usize::from(tracks[i].bass_preset.index), expected);
        assert_eq!(tracks[i].bass, PRESETS[expected].controls);
        assert_eq!(tracks[i].voice.waveform, PRESETS[expected].waveform);
        assert_eq!(tracks[i].inserts, PRESETS[expected].inserts);
    }
    assert_eq!([tracks[2], tracks[3]], drums);
}

#[test]
fn repeated_snapshots_preserve_manual_voice_and_insert_edits() {
    let mut nav = Navigation::INITIAL;
    let mut track = Track::factory(Channel::One, nav.assignments[0]);
    nav.turn_algorithm(5);
    track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
    track.bass.turn(Knob::Cutoff, 2);
    track.turn_algorithm(View::Scope, Some(Control::Waveform), -1);
    track.inserts[0].edit([3, -4, 5, 6], true);
    track.inserts[1].edit([1, -20, 8, 0], false);
    let edited = track;

    for _ in 0..3 {
        track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
        assert_eq!(track, edited);
    }

    nav.turn_algorithm(1);
    track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
    assert_eq!(track.bass, PRESETS[6].controls);
    assert_eq!(track.inserts, PRESETS[6].inserts);
}

#[test]
fn later_insert_picker_choice_wins_after_a_skipped_preset_snapshot() {
    let mut nav = Navigation::INITIAL;
    let mut track = Track::factory(Channel::One, nav.assignments[0]);
    nav.turn_algorithm(5); // Acid Pluck selects saturation + phaser; this snapshot is dropped.
    nav.press(PageButton::Fx);
    nav.press(PageButton::Fx);
    nav.turn(2, 0); // Select its second insert.
    nav.turn_algorithm(1); // Advance Phaser -> Flanger.
    assert_eq!(nav.insert_kinds[0][1], InsertKind::Flanger);

    track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
    assert_eq!(track.bass, PRESETS[5].controls);
    assert_eq!(track.inserts[0], PRESETS[5].inserts[0]);
    assert_eq!(track.inserts[1], InsertKind::Flanger.initial(Part::Bass));

    track.inserts[1].edit([5, 0, 0, -8], false);
    let edited = track;
    track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
    assert_eq!(track, edited);
}

#[test]
fn a_whole_bank_turn_recalls_the_same_preset_after_live_edits() {
    let mut nav = Navigation::INITIAL;
    let mut track = Track::factory(Channel::One, nav.assignments[0]);
    track.bass.turn(Knob::Resonance, -30);
    track.turn_algorithm(View::Scope, Some(Control::Muffler), i8::MAX);
    assert_eq!(track.bass.muffler_label(), "2");
    track.inserts[0].edit([0, 0, 0, 40], false);
    let previous_selection = track.bass_preset;

    nav.turn_algorithm(i8::try_from(PRESETS.len()).expect("small sound bank"));
    assert_eq!(nav.bass_presets[0].index, previous_selection.index);
    assert_ne!(nav.bass_presets[0], previous_selection);
    track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
    assert_eq!(track.bass, PRESETS[0].controls);
    assert_eq!(track.inserts, PRESETS[0].inserts);
    assert_eq!(track.bass.muffler_label(), "OFF");

    track.bass.turn(Knob::Accent, -9);
    let edited = track;
    nav.turn_algorithm(0);
    track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
    assert_eq!(track, edited);
}

#[test]
fn instrument_reassignment_retains_the_cursor_and_edited_channel_effects() {
    let mut nav = Navigation::INITIAL;
    nav.turn(0, 1);
    let mut track = Track::factory(Channel::Two, nav.assignments[1]);
    nav.turn_algorithm(4);
    track.sync_panel(nav.bass_presets[1], nav.insert_kinds[1]);
    track.inserts[0].edit([2, -3, 4, 5], false);
    let inserts = track.inserts;
    let cursor = track.bass_preset;

    nav.press(PageButton::Glo);
    nav.turn(1, 0); // Open the focused strip.
    for part in [Part::Mono, Part::Bass] {
        select_instrument(&mut nav, part);
        track.reassign(Channel::Two, nav.assignments[1]);
        assert_eq!(nav.bass_presets[1], cursor);
        track.sync_panel(nav.bass_presets[1], nav.insert_kinds[1]);
        assert_eq!(track.bass_preset, cursor);
        assert_eq!(track.assignment.part, part);
        assert_eq!(track.inserts, inserts);
    }
    assert_eq!(track.bass, PRESETS[5].controls);
    assert_eq!(track.voice.waveform, PRESETS[5].waveform);
}

#[test]
fn skipped_preset_then_reassignment_recovers_numeric_effect_settings() {
    let mut nav = Navigation::INITIAL;
    let mut track = Track::factory(Channel::One, nav.assignments[0]);
    nav.turn_algorithm(5); // Do not consume the phaser preset before changing instruments.
    nav.press(PageButton::Glo);
    nav.turn(1, 0);
    select_instrument(&mut nav, Part::Mono);
    track.reassign(Channel::One, nav.assignments[0]);
    track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);

    assert_eq!(track.assignment.part, Part::Mono);
    assert_eq!(track.bass_preset.index, 5);
    assert_eq!(track.inserts, PRESETS[5].inserts);
    assert_ne!(track.inserts[0], InsertKind::Saturator.initial(Part::Mono));
    assert_ne!(track.inserts[1], InsertKind::Phaser.initial(Part::Mono));
    let mono = track;
    track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
    assert_eq!(track, mono);

    select_instrument(&mut nav, Part::Bass);
    track.reassign(Channel::One, nav.assignments[0]);
    track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
    assert_eq!(track.bass, PRESETS[5].controls);
    assert_eq!(track.voice.waveform, PRESETS[5].waveform);
    assert_eq!(track.inserts, PRESETS[5].inserts);
}

#[test]
fn recall_keeps_held_notes_slide_transpose_and_other_performance_controls() -> Result<(), Error> {
    let mut nav = Navigation::INITIAL;
    let mut track = Track::factory(Channel::One, nav.assignments[0]);
    track.voice.gate = Gate {
        note: Some(note(50, 110)),
        phrase: 7,
    };
    track.voice.transpose = -12;
    track.bass.slide = true;
    track.lfo.enabled = true;
    track.arp.enabled = true;
    track.arp_revision = 19;
    track.voice_tap = false;
    let before = track;

    nav.turn_algorithm(2);
    track.sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
    assert_eq!(track.voice.gate, before.voice.gate);
    assert_eq!(track.voice.transpose, before.voice.transpose);
    assert!(track.bass.slide);
    assert_eq!(track.lfo, before.lfo);
    assert_eq!(track.arp, before.arp);
    assert_eq!(track.arp_revision, before.arp_revision);
    assert_eq!(track.voice_tap, before.voice_tap);
    assert_eq!(track.assignment, before.assignment);
    let events = track.events(Some(before))?;
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| !matches!(
        event.message,
        Message::Bass(BassMessage::NoteOn { .. } | BassMessage::NoteOff(_))
    )));
    Ok(())
}

#[test]
fn every_preset_uses_post_vca_od_and_sel_keeps_the_scope_action() -> Result<(), Error> {
    for preset in PRESETS {
        let mut track = Track::new(Assignment::new(Part::Bass));
        let before = track;
        track.bass = preset.controls;
        track.voice.waveform = preset.waveform;
        let events = track.events(Some(before))?;
        assert!(events.contains(&at(BassMessage::PedalDrive(
            f32::from(track.bass.detent(Knob::Overdrive)) / 36.0,
        ))));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.message, Message::Bass(BassMessage::OverdriveDb(_))))
        );
        assert_eq!(track.bass.readout(Knob::Overdrive).0, b"OD       ");
        assert_eq!(track.bass.readout(Knob::Overdrive).2, b"PCT  ");
        let saved = track.bass;
        let scope = track.voice_tap;
        track.edit_panel(View::Scope, 0, [0; 4], true);
        assert_eq!(track.bass, saved);
        assert_eq!(track.voice_tap, !scope);
    }
    Ok(())
}

#[test]
fn automated_od_releases_to_the_recalled_preset_when_sequence_is_muted() -> Result<(), Error> {
    with_instrument(44_100, |rack| {
        let mut nav = Navigation::INITIAL;
        let mut tracks = Track::rack(nav.assignments);
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        rack.transport.toggle();
        rack.render(tracks, &mut buses)?;
        let (_, owned) = rack
            .bass_automation(Channel::One)
            .expect("factory automation");
        let amount = f32::from(owned.get(17).expect("CC17")) / 127.0;
        assert!(rack.events[0].contains(&at(BassMessage::PedalDrive(amount))));
        assert!(
            !rack.events[0]
                .iter()
                .any(|event| matches!(event.message, Message::Bass(BassMessage::OverdriveDb(_))))
        );

        nav.turn_algorithm(2);
        tracks[0].sync_panel(nav.bass_presets[0], nav.insert_kinds[0]);
        rack.render(tracks, &mut buses)?;
        assert!(rack.bass_automation(Channel::One).is_some());
        assert!(!rack.events[0].contains(&at(BassMessage::PedalDrive(24.0 / 36.0))));
        rack.sequences[0] = false;
        rack.render(tracks, &mut buses)?;
        assert!(rack.bass_automation(Channel::One).is_none());
        assert!(rack.events[0].contains(&at(BassMessage::PedalDrive(24.0 / 36.0))));
        assert!(
            !rack.events[0]
                .iter()
                .any(|event| matches!(event.message, Message::Bass(BassMessage::OverdriveDb(_))))
        );
        Ok(())
    })
}

#[test]
fn midi_and_lfo_od_use_the_pedal_without_changing_the_saved_amount() -> Result<(), Error> {
    with_instrument(44_100, |rack| {
        let mut tracks = Track::rack(Navigation::INITIAL.assignments);
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        rack.sequences.fill(false);
        let mut keys = Keys::<1>::new();
        let (channel, cc) = decode_midi([11, 0xb0, 17, 93]).expect("CC17");
        keys.midi(channel, cc, &mut tracks[0]);
        tracks[0].lfo.edit(Part::Bass, 0, [3, 0, 95, 7], true);
        let saved = tracks[0].bass;
        rack.render(tracks, &mut buses)?;
        assert!(rack.events[0].iter().any(|event| matches!(event.message,
            Message::Bass(BassMessage::PedalDrive(value)) if (value - 93.0 / 127.0).abs() < 0.000_001)));
        assert!(rack.events[0].contains(&at(BassMessage::Modulate(Destination::PedalDrive, 1.0))));
        assert_eq!(tracks[0].bass, saved);
        tracks[0].lfo.enabled = false;
        rack.render(tracks, &mut buses)?;
        assert!(rack.events[0].contains(&at(BassMessage::Modulate(Destination::PedalDrive, 0.0))));
        assert_eq!(tracks[0].bass, saved);
        Ok(())
    })
}
