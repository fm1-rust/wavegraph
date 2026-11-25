use super::*;
use crate::panel::PageButton;
use wavegraph_sampler::{Playback, PlaybackMode, Timbre};

fn first_frames(pcm: &[u8]) -> [i32; 128] {
    let frames = pcm.as_chunks::<2>().0;
    core::array::from_fn(|i| i32::from(i16::from_le_bytes(frames[i])) << 8)
}

#[test]
fn panel_and_midi_play_the_same_clip_an_octave_apart_on_independent_strips() -> Result<(), Error> {
    with_instrument(44_118, |engine| {
        let mut keys = Keyboard::<4>::new();
        for channel in [Channel::One, Channel::Two] {
            keys.nav.focus = channel;
            keys.nav.view = View::Scope;
            keys.nav.press(PageButton::Glo);
            keys.nav.turn(1, 0);
            select_instrument(&mut keys.nav, Part::Sampler);
            assert_eq!(keys.nav.part(), Part::Sampler);
            keys.tracks[channel.index()] =
                Track::factory(channel, keys.nav.assignments[channel.index()]);
            keys.tracks[channel.index()].turn_algorithm(View::Scope, None, 3);
        }
        keys.nav.focus = Channel::One;
        keys.nav.view = View::Scope;
        keys.press(note(60, 127));
        let (channel, action) = decode_midi([9, 0x91, 72, 127]).expect("channel two C5");
        keys.keys
            .midi(channel, action, &mut keys.tracks[channel.index()]);
        keys.snapshot();
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        engine.render(keys.tracks, &mut buses)?;
        let root = buses[Channel::One.source_bus()].map(I24::inner);
        let octave = buses[Channel::Two.source_bus()].map(I24::inner);
        assert_eq!(
            octave,
            first_frames(include_bytes!("../../../samples/rave/horn.s16le"))
        );
        assert!(root.iter().any(|&x| x != 0));
        assert!(
            root.iter()
                .step_by(2)
                .copied()
                .eq(octave[..64].iter().copied())
        );
        assert!(buses[MASTER_BUS].iter().any(|s| s.inner() != 0));

        keys.tracks[0].turn_algorithm(View::Scope, None, 1);
        assert_eq!(keys.tracks[0].voice.label(Part::Sampler), b"LASER ZAP");
        assert_eq!(keys.tracks[1].voice.label(Part::Sampler), b"AIRHORN  ");
        keys.release(60);
        engine.render(keys.tracks, &mut buses)?; // let the independent horn release finish
        keys.press(note(72, 127));
        engine.render(keys.tracks, &mut buses)?;
        let zap = buses[Channel::One.source_bus()].map(I24::inner);
        #[cfg(not(feature = "lofi-samples"))]
        assert_eq!(
            zap,
            first_frames(include_bytes!("../../../samples/rave/zap.s16le"))
        );
        #[cfg(feature = "lofi-samples")]
        {
            // FFmpeg's independent IMA decode, every sixth source frame at C5.
            let reference = [
                0, 0, 47, 63, 80, 63, 49, 33, 33, -17, -31, -15, -16, 127, 80,
            ];
            for (sample, expected) in zap.iter().step_by(9).zip(reference) {
                assert!(sample.abs_diff(expected << 8) <= 256);
            }
        }

        keys.tracks[0].turn_algorithm(View::Scope, None, 1);
        // Open the voiced stab to check its underlying PCM and root.
        keys.tracks[0].sample_program_mut().timbre = Timbre::DRY;
        keys.release(72);
        engine.render(keys.tracks, &mut buses)?;
        keys.press(note(55, 127));
        engine.render(keys.tracks, &mut buses)?;
        let stab = buses[Channel::One.source_bus()].map(I24::inner);
        #[cfg(not(feature = "lofi-samples"))]
        {
            // At G3 the master advances two source frames every three DAC frames.
            let expected = first_frames(include_bytes!("../../../samples/rave/stab.s16le"));
            for (sample, reference) in stab.iter().step_by(3).zip(expected.iter().step_by(2)) {
                assert!(sample.abs_diff(*reference) <= 256);
            }
        }
        #[cfg(feature = "lofi-samples")]
        {
            // Same root and duration. At G3 every ninth DAC frame lands on the
            // third source frame. Allow one PCM count for Q15 phase rounding.
            let reference = [
                0, 0, 106, 531, 754, 534, 273, 242, 266, -29, -246, -243, -268, -515, -570,
            ];
            for (sample, expected) in stab.iter().step_by(9).zip(reference) {
                assert!(sample.abs_diff(expected << 8) <= 256);
            }
        }
        let track = &mut keys.tracks[0];
        for (turn, name) in [
            (1, b"CLIPBOARD"),
            (1, b"AMEN BRK "),
            (1, b"MIN7 STAB"),
            (1, b"RAVE RISE"),
            (1, b"AIRHORN  "),
            (-1, b"RAVE RISE"),
        ] {
            track.turn_algorithm(View::Scope, None, turn);
            assert_eq!(track.voice.label(Part::Sampler), name);
        }
        Ok(())
    })
}

#[test]
fn a_one_shot_retriggers_on_a_new_strike_but_not_when_an_older_key_is_uncovered() {
    let mut keys = Keyboard::<4>::new();
    keys.nav.assignments[0] = Assignment::new(Part::Sampler);
    keys.tracks[0] = Track::new(keys.nav.assignments[0]);
    keys.tracks[0].edit_panel(View::SampleEdit, 0, [-1, 0, 0, 0], false);
    keys.press(note(60, 80));
    keys.press(note(64, 90));
    let struck = keys.tracks[0];
    keys.release(64);
    let released = keys.tracks[0];
    assert!(
        !released
            .events(Some(struck))
            .expect("release")
            .iter()
            .any(|e| matches!(e.message, Message::Sampler(SampleMessage::NoteOn(_))))
    );
    keys.press(note(60, 80));
    assert!(
        keys.tracks[0]
            .events(Some(released))
            .expect("restrike")
            .iter()
            .any(|e| e.message == Message::Sampler(SampleMessage::NoteOn(note(60, 80))))
    );
}

#[test]
fn releasing_the_fifth_key_does_not_relaunch_a_stolen_amen() {
    let mut keys = Keyboard::<5>::new();
    keys.nav.assignments[0] = Assignment::new(Part::Sampler);
    keys.tracks[0] = Track::new(keys.nav.assignments[0]);
    for pitch in 60..65 {
        keys.press(note(pitch, 100));
    }
    let old = keys.tracks[0];
    keys.release(64);
    assert_eq!(
        keys.tracks[0]
            .events(Some(old))
            .expect("release")
            .as_slice(),
        &[at(Message::Sampler(SampleMessage::NoteOff(64)))]
    );
}

#[test]
fn changing_to_one_shot_while_holding_a_loop_still_releases_the_loop() {
    let mut keys = Keyboard::<4>::new();
    keys.nav.assignments[0] = Assignment::new(Part::Sampler);
    keys.tracks[0] = Track::new(keys.nav.assignments[0]);
    keys.tracks[0].edit_panel(View::SampleEdit, 0, [2, 0, 0, 0], false);
    keys.press(note(60, 80));
    keys.press(note(64, 90));
    keys.tracks[0].edit_panel(View::SampleEdit, 0, [-2, 0, 0, 0], false);
    let before = keys.tracks[0];
    keys.release(64);
    assert_eq!(
        keys.tracks[0]
            .events(Some(before))
            .expect("key release")
            .as_slice(),
        &[at(Message::Sampler(SampleMessage::NoteOff(64)))]
    );
}

#[test]
fn edit_sliders_set_loop_trim_and_keyboard_tracking_on_the_audio_voice() {
    let mut track = Track::new(Assignment::new(Part::Sampler));
    let before = track;
    track.edit_panel(View::SampleEdit, 0, [2, 18, -35, 10], false);
    let playback = Playback {
        mode: PlaybackMode::Loop,
        start: 18,
        end: 65,
        release_ms: 0,
    };
    assert_eq!(track.sample_program().playback, playback);
    assert!(
        track
            .events(Some(before))
            .expect("track events")
            .iter()
            .any(|e| e.message == Message::Sampler(SampleMessage::Playback(playback)))
    );
    assert_eq!(track.sample_program().timbre.key_tracking, 10);
    track.edit_panel(View::SampleEdit, 0, [0, i8::MAX, i8::MIN, i8::MAX], false);
    assert_eq!(
        (
            track.sample_program().playback.start,
            track.sample_program().playback.end,
            track.sample_program().timbre.key_tracking
        ),
        (64, 65, 100)
    );
}

#[test]
fn sampler_env_selects_independent_contours_and_recalls_each_sounds_program() {
    let mut nav = Navigation::INITIAL;
    nav.assignments[0] = Assignment::new(Part::Sampler);
    nav.focus = Channel::One;
    let mut track = Track::factory(nav.focus, nav.assignments[0]);
    track.turn_algorithm(nav.view, None, 3);
    let horn = track.sample_program();
    track.edit_panel(View::Scope, 0, [-8, 12, 9, -7], false);
    let edited = track.sample_program().timbre;
    assert_eq!(edited.cutoff_hz, 7937);
    assert_eq!(
        (
            edited.resonance,
            edited.env_semitones,
            edited.velocity_semitones
        ),
        (12, 9, 17)
    );
    nav.press(PageButton::Env);
    assert_eq!(nav.view, View::Envelope);
    track.edit_panel(nav.view, 0, [1, 2, -10, 4], false);
    let amp = track.sample_program().timbre.amp;
    assert_ne!(amp, horn.timbre.amp);
    assert_eq!(track.sample_program().timbre.filter, horn.timbre.filter);
    track.edit_panel(nav.view, 0, [2, 3, -6, 1], true);
    assert_eq!(track.envelope_target, EnvelopeTarget::Modulation);
    let program = track.sample_program();
    assert_eq!(program.timbre.amp, amp);
    assert_ne!(program.timbre.filter, horn.timbre.filter);
    track.turn_algorithm(nav.view, None, 1);
    assert_eq!(track.voice.waveform, 3);

    nav.press(PageButton::Env);
    track.turn_algorithm(nav.view, None, -3);
    assert_eq!(track.sample_program().timbre, Timbre::DRY);
    assert_eq!(track.sample_program().playback.mode, PlaybackMode::OneShot);
    nav.press(PageButton::Edit);
    assert_eq!(nav.view, View::SampleEdit);
    track.edit_panel(nav.view, 0, [0, 7, -12, 25], false);
    nav.press(PageButton::Edit);
    assert_eq!(nav.view, View::Scope);
    track.turn_algorithm(nav.view, None, 3);
    assert_eq!(track.sample_program(), program);
}

#[test]
fn replacing_the_808_plays_the_dry_amen_and_follows_live_tempo_without_retriggering()
-> Result<(), Error> {
    // Independent PCM16 reference points from FFmpeg's IMA WAV decoder.
    #[cfg(all(
        feature = "packed-samples",
        feature = "packed-cymbals",
        not(feature = "lofi-samples")
    ))]
    let (rate, start, faster) = (
        24400,
        [
            0, -698, -2437, -4982, -5499, -14989, -15394, -8389, -3379, 3197, 9357, 4822,
        ],
        [
            -7660, -619, 9890, 18288, 17543, 15209, 15214, 8122, -692, -14365, -20513, -20397,
        ],
    );
    #[cfg(not(all(
        feature = "packed-samples",
        feature = "packed-cymbals",
        not(feature = "lofi-samples")
    )))]
    let (rate, start, faster) = (
        12505,
        [
            0, 699, 606, -9232, -7526, 14021, 2687, -7880, -14931, -7965, -2135, 3704,
        ],
        [
            -9062, -25510, -20158, -14807, 6208, 20495, 24122, 17047, 2600, -15154, -22888, -16519,
        ],
    );

    with_instrument(rate, |engine| {
        let channel = Channel::Three;
        let mut tracks = Track::rack(Navigation::INITIAL.assignments);
        assert_eq!(tracks[channel.index()].assignment.part, Part::Tr808);
        let track = &mut tracks[channel.index()];
        *track = Track::factory(channel, Assignment::new(Part::Sampler));
        engine.sequences = [false, false, true, false];
        engine.transport.toggle();
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        for (bpm, reference) in [(122, start), (244, faster)] {
            engine.transport.bpm = bpm;
            engine.render(tracks, &mut buses)?;
            let actual = &buses[channel.source_bus()];
            for (sample, expected) in actual.iter().step_by(7).zip(reference) {
                assert!(
                    sample.inner().abs_diff(expected << 8) <= 256,
                    "{bpm} BPM: {sample:?} vs {expected}"
                );
            }
        }
        engine.transport.toggle();
        engine.render(tracks, &mut buses)?;
        assert!(
            buses[channel.source_bus()]
                .iter()
                .all(|sample| sample.inner() == 0)
        );
        Ok(())
    })
}

#[test]
fn house_m7_stacks_panel_and_midi_keys_on_the_same_strip() -> Result<(), Error> {
    let keys_and_velocity = [(50, 101), (57, 83), (62, 118), (69, 91)];
    let play = |mask: u8| {
        let mut result = [0_i32; 128];
        with_instrument(44_118, |engine| {
            let mut keys = Keyboard::<4>::new();
            keys.nav.assignments[0] = Assignment::new(Part::Sampler);
            keys.tracks[0] = Track::new(keys.nav.assignments[0]);
            keys.tracks[0].turn_algorithm(View::Scope, None, 1);
            for (i, (key, velocity)) in keys_and_velocity.into_iter().enumerate() {
                if mask & (1 << i) == 0 {
                    continue;
                }
                if i < 2 {
                    keys.press(note(key, velocity));
                } else {
                    let (channel, action) =
                        decode_midi([9, 0x90, key, velocity]).expect("MIDI note");
                    keys.keys.midi(channel, action, &mut keys.tracks[0]);
                }
            }
            keys.snapshot();
            let mut buses = [[I24::default(); 128]; BUS_COUNT];
            for _ in 0..6 {
                engine.render(keys.tracks, &mut buses)?;
            }
            result = buses[Channel::One.source_bus()].map(I24::inner);
            Ok(())
        })?;
        Ok::<_, Error>(result)
    };
    let mut sum = [0_i32; 128];
    for i in 0..4 {
        let voice = play(1 << i)?;
        assert!(voice.iter().any(|&s| s != 0));
        for (dst, s) in sum.iter_mut().zip(voice) {
            *dst += s;
        }
    }
    assert_eq!(play(15)?, sum);
    Ok(())
}

#[test]
fn lit_pad_identity_survives_octave_transpose_and_seq_rec() {
    use crate::sequencer::edit::Edits;
    let channel = Channel::One;
    let assignment = Assignment::new(Part::Sampler);
    let mut track = Track::factory(channel, assignment);
    track.turn_select(7);
    track.edit_panel(View::SampleEdit, 0, [3, 0, 0, 0], false);
    let mut piano = Piano::<16>::new();
    piano.shift_octave(2);
    let mut seq = Sequencer::INITIAL;
    seq.channel = channel;
    seq.record();
    let at = seq
        .edit_address(Part::Sampler, None)
        .expect("armed sixteenth");
    let edits = Edits::new();
    for pitch in crate::assets::PAD_NOTES.into_iter().chain([60]) {
        let key = note(pitch, 93);
        let played = piano
            .keyboard_note(channel, assignment, key, track.amen_slices(), false)
            .expect("lit key");
        assert_eq!(played, key);
        assert_eq!(track.key_transpose(), 0);
        assert!(edits.write(at, played, false));
        let step = edits.step(at);
        assert!(step.events(0).any(|msg| msg == MidiAction::On(key)));
        assert!(step.events(6).any(|msg| msg == MidiAction::Off(pitch)));
        piano.release(pitch);
        edits.restore(at);
    }
    assert_eq!(
        piano.keyboard_note(channel, assignment, note(54, 127), true, false),
        None
    );
    track.select_slice(55);
    track.edit_panel(View::SampleEdit, 0, [0, 12, -7, 0], false);
    track.select_slice(67);
    assert_eq!(track.sample_pads().trims[1], [12, 93]);
    assert_eq!(track.sample_pads().trims[7], [0, 100]);
    track.select_slice(60);
    track.edit_panel(View::SampleEdit, 0, [0, 8, -2, 0], false);
    assert_eq!(track.sample_program().playback.start, 0);
    assert_eq!(track.sample_program().playback.end, 100);
}

#[test]
fn one_clipboard_take_plays_from_two_sampler_strips_at_different_pitches() -> Result<(), Error> {
    use crate::clipboard::State;
    let check_chord = |engine: &mut Instrument<'_>, part| {
        let source = Channel::One;
        let mut tracks = Track::TEST_TRACKS;
        let assignment = Assignment::new(part);
        tracks[0] = Track::factory(source, assignment);
        for channel in [Channel::Three, Channel::Four] {
            let track = &mut tracks[channel.index()];
            *track = Track::factory(channel, Assignment::new(Part::Sampler));
            track.voice.waveform = crate::assets::CLIPBOARD_SLOT;
        }
        engine.recording_request.press(source, assignment);
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        engine.render(tracks, &mut buses)?;
        assert_eq!(engine.recorder.status.state, State::Armed);
        // While waiting for sound, select one beat at native rate and root it at D4.
        engine.recording_request.options.turn(-1, 2);
        for (i, pitch) in [62, 65, 69, 72].into_iter().enumerate() {
            tracks[0].poly_keys[i] = PolyKey {
                note: Some(note(pitch, 110)),
                strike: u32::try_from(i + 1).expect("four keys"),
            };
        }
        let mut reference = Vec::<i32, 512>::new();
        for _ in 0..3 {
            engine.render(tracks, &mut buses)?;
            let bus = &buses[source.source_bus()];
            let start = if reference.is_empty() {
                bus.iter()
                    .position(|s| s.inner().unsigned_abs() >= 256)
                    .unwrap_or(bus.len())
            } else {
                0
            };
            for s in &bus[start..] {
                reference.push((s.inner() >> 8) << 8).expect("three blocks");
            }
        }
        assert_eq!(engine.recorder.status.state, State::Recording);
        // Stage a one-bar take at C4 before manually finishing the D4 chord.
        engine.recording_request.options.turn(1, -2);
        engine.recording_request.press(source, assignment);
        engine.render(tracks, &mut buses)?;
        let take = engine.recorder.status.take.expect("finished chord");
        assert_eq!(take.root, 62);
        assert_eq!(usize::from(take.frames), reference.len());
        for (channel, pitch) in [(Channel::Three, 62), (Channel::Four, 74)] {
            tracks[channel.index()].poly_keys[0] = PolyKey {
                note: Some(note(pitch, 127)),
                strike: 1,
            };
        }
        engine.render(tracks, &mut buses)?;
        assert_eq!(
            &buses[Channel::Three.source_bus()].map(I24::inner),
            &reference[..128]
        );
        assert!(
            buses[Channel::Four.source_bus()]
                .iter()
                .map(|s| s.inner())
                .eq(reference.iter().copied().step_by(2).take(128))
        );
        // Arming or cancelling before the next downbeat preserves the existing take.
        let mut transport = Transport::INITIAL;
        transport.toggle();
        let recorder = &mut engine.recorder;
        engine.recording_request.press(source, assignment);
        recorder.process(engine.recording_request, &tracks, transport, None, &buses);
        assert_eq!(recorder.status.state, State::Armed);
        assert_eq!(recorder.status.take, Some(take));
        engine.recording_request.press(source, assignment);
        recorder.process(engine.recording_request, &tracks, transport, None, &buses);
        assert_eq!(recorder.status.take, Some(take));

        // Record two bars from frame 17. The recorder must fit the complete loop,
        // including its final short decimation group, and publish the new root.
        engine.recording_request.press(source, assignment);
        recorder.process(engine.recording_request, &tracks, transport, None, &buses);
        assert_eq!(recorder.status.state, State::Armed);
        engine.recording_request.options.turn(1, 0); // Extend to two bars before the downbeat.
        recorder.process(
            engine.recording_request,
            &tracks,
            transport,
            Some(17),
            &buses,
        );
        assert_eq!(recorder.status.take, None);
        assert_eq!(recorder.status.state, State::Recording);
        engine.recording_request.options.turn(-2, 7); // Next take only.
        let frames = (44_118_u32 * 60 * 8).div_ceil(u32::from(transport.bpm));
        let divisor = frames.div_ceil(32_768);
        for _ in 0..frames.saturating_sub(111).div_ceil(128) {
            recorder.process(engine.recording_request, &tracks, transport, None, &buses);
        }
        let replacement = recorder.status.take.expect("automatic loop end");
        assert_eq!(u32::from(replacement.frames), frames.div_ceil(divisor));
        assert_eq!(u32::from(replacement.hz), (44_118 + divisor / 2) / divisor);
        assert_eq!(replacement.root, 60);
        assert_eq!(recorder.status.state, State::Idle);
        Ok(())
    };
    for part in [Part::Fm, Part::Hoover] {
        with_instrument(44_118, |engine| check_chord(engine, part))?;
    }
    Ok(())
}
