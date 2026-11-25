//! Chromatic playback behaves identically across audio-block boundaries.

use wavegraph_core::{Event, Process, ProcessContext, midi::Note};
use wavegraph_sampler::{Clip, PitchedMessage as Msg, PitchedSampler, Playback, PlaybackMode};

fn on(number: u8, velocity: u8) -> Msg {
    Msg::NoteOn(Note { number, velocity })
}

fn render<const N: usize>(voice: &mut PitchedSampler<'_>, messages: &[(usize, Msg)]) -> [i16; N] {
    let events: Vec<_> = messages
        .iter()
        .map(|&(frame, message)| Event {
            frame,
            port: 0,
            message,
        })
        .collect();
    let mut output = [99; N];
    voice.process(&mut ProcessContext {
        frames: N,
        inputs: &[],
        outputs: &mut [&mut output],
        events: &events,
        emit: &mut Err,
    });
    output
}

#[test]
fn native_pitch_octaves_and_source_clock_share_one_timeline() {
    let pcm = [1000_i16, 3000, -1000, -3000].map(i16::to_le_bytes);
    let clip = Clip::from_pcm16le(pcm.as_flattened(), 22_059, 60).expect("valid clip");
    let bank = [clip];
    let mut at_source = PitchedSampler::new(&bank, 22_059).expect("valid clock");
    assert_eq!(
        render::<6>(&mut at_source, &[(1, on(60, 127))]),
        [0, 1000, 3000, -1000, -3000, 0]
    );
    assert_eq!(
        render::<4>(&mut at_source, &[(0, on(72, 127))]),
        [1000, -1000, 0, 0]
    );
    assert_eq!(
        render::<3>(&mut at_source, &[(0, on(48, 127))]),
        [1000, 2000, 3000]
    );
    assert_eq!(
        render::<6>(&mut at_source, &[]),
        [1000, -1000, -2000, -3000, -1500, 0]
    );
    let mut at_dac = PitchedSampler::new(&bank, 44_118).expect("FM-1 rate");
    assert_eq!(
        render::<5>(&mut at_dac, &[(0, on(72, 127))]),
        [1000, 3000, -1000, -3000, 0]
    );
}

#[cfg(feature = "ima-adpcm")]
#[test]
fn ima_skips_independent_blocks_and_excludes_encoder_padding_from_loops() {
    let mut blocks = [[0_u8; 64]; 2];
    blocks[0][..4].copy_from_slice(&[0xe8, 3, 40, 0]);
    for word in blocks[0][4..].as_chunks_mut::<4>().0 {
        *word = [0x87, 0x31, 0xf2, 0xce];
    }
    blocks[1][..4].copy_from_slice(&[0x30, 0xf8, 22, 0]);
    for word in blocks[1][4..].as_chunks_mut::<2>().0 {
        *word = [0x67, 0x1a];
    }
    let data = blocks.as_flattened();
    let bank = [Clip::from_ima_adpcm(data, 125, 1000, 60).expect("125 real frames")];
    let mut voice = PitchedSampler::new(&bank, 1000).expect("valid clock");
    let all = render::<128>(&mut voice, &[(0, on(60, 127))]);
    // Reference values decoded by FFmpeg's adpcm_ima_wav decoder.
    assert_eq!(
        all[..9],
        [1000, 1631, 1541, 1787, 2310, 2650, 1724, 2, -2110]
    );
    assert_eq!(
        all[118..],
        [-9204, -32768, -32768, -2000, -1888, -1677, -1820, 0, 0, 0]
    );
    let trim = Playback {
        start: 97,
        mode: PlaybackMode::Loop,
        ..Playback::INITIAL
    };
    assert_eq!(
        render::<8>(&mut voice, &[(0, Msg::Playback(trim)), (0, on(60, 127))]),
        [-2000, -1888, -1677, -1820, -2000, -1888, -1677, -1820]
    );
    assert!(Clip::from_ima_adpcm(&data[..127], 125, 1000, 60).is_none());
    assert!(Clip::from_ima_adpcm(data, 243, 1000, 60).is_none());
    blocks[1][2] = 89;
    assert!(Clip::from_ima_adpcm(blocks.as_flattened(), 125, 1000, 60).is_none());
}

#[test]
fn chromatic_semitones_and_long_clips_keep_their_position() {
    let pcm = vec![0x40; 160_000];
    let bank = [Clip::from_pcm16le(&pcm, 44_118, 60).expect("80k frames")];
    for (key, expected) in [
        (48, 160_000_isize),
        (59, 84_757),
        (60, 80_000),
        (61, 75_510),
        (72, 40_000),
    ] {
        let mut voice = PitchedSampler::new(&bank, 44_118).expect("valid clock");
        render::<0>(&mut voice, &[(0, on(key, 127))]);
        let mut count = 0_isize;
        loop {
            let chunk = render::<128>(&mut voice, &[]);
            let frames = chunk.iter().take_while(|&&s| s != 0).count();
            count = count.saturating_add(isize::try_from(frames).expect("block length"));
            if frames != 128 {
                break;
            }
            assert!(count <= 160_000);
        }
        assert!(count.abs_diff(expected) <= 2, "key {key}: {count} frames");
    }
}

#[test]
fn selecting_retriggering_releasing_and_stopping_are_frame_timed() {
    let first = [8192_i16.to_le_bytes(); 256];
    let second = [(-8192_i16).to_le_bytes(); 256];
    let bank = [first.as_flattened(), second.as_flattened()]
        .map(|pcm| Clip::from_pcm16le(pcm, 1000, 60).expect("clip"));
    let mut voice = PitchedSampler::new(&bank, 1000).expect("valid clock");
    assert_eq!(
        render::<6>(
            &mut voice,
            &[
                (0, on(60, 127)),
                (1, Msg::Select(1)),
                (2, Msg::Select(99)),
                (3, on(60, 127))
            ]
        ),
        [8192, 8192, 8192, -8192, -8192, -8192]
    );
    assert_eq!(
        render::<2>(&mut voice, &[(0, Msg::NoteOff(59))]),
        [-8192; 2]
    );
    let tail = render::<68>(&mut voice, &[(2, Msg::NoteOff(60))]);
    assert_eq!(&tail[..4], &[-8192, -8192, -8192, -8064]);
    assert_eq!(&tail[65..], &[-128, 0, 0]);
    let shot = render::<72>(
        &mut voice,
        &[
            (
                0,
                Msg::Playback(Playback {
                    mode: PlaybackMode::OneShot,
                    ..Playback::INITIAL
                }),
            ),
            (0, on(60, 64)),
            (1, Msg::NoteOff(60)),
            (70, Msg::Stop),
        ],
    );
    assert!(shot[0] < -4000 && shot[0] > -4200);
    assert_eq!(&shot[..70], &[shot[0]; 70]);
    assert_eq!(&shot[70..], &[0, 0]);
}

#[test]
fn invalid_inputs_and_empty_sources_stay_silent() {
    assert!(Clip::from_pcm16le(&[0], 1, 60).is_none());
    assert!(Clip::from_pcm16le(&[], 0, 60).is_none());
    assert!(Clip::from_pcm16le(&[], 1, 128).is_none());
    assert!(PitchedSampler::new(&[], 1).is_none());
    let bank = [Clip::from_pcm16le(&[], 1, 0).expect("empty clip")];
    assert!(PitchedSampler::new(&bank, 0).is_none());
    let mut voice = PitchedSampler::new(&bank, 1).expect("empty clip in bank");
    assert_eq!(render::<2>(&mut voice, &[(0, on(127, 127))]), [0; 2]);
}

#[test]
fn trim_and_forward_loop_preserve_pitch_and_release_over_the_selected_time() {
    let pcm = [0_i16, 100, 200, 300, 400, 500, 600, 700].map(i16::to_le_bytes);
    let bank = [Clip::from_pcm16le(pcm.as_flattened(), 1000, 60).expect("clip")];
    let mut voice = PitchedSampler::new(&bank, 1000).expect("clock");
    let p = Playback {
        mode: PlaybackMode::Loop,
        start: 25,
        end: 75,
        release_ms: 4,
    };
    assert_eq!(
        render::<6>(&mut voice, &[(0, Msg::Playback(p)), (0, on(72, 127))]),
        [200, 400, 200, 400, 200, 400]
    );
    assert_eq!(
        render::<6>(&mut voice, &[(0, Msg::NoteOff(72))]),
        [200, 300, 100, 100, 0, 0]
    );
    // Multiple wraps per output frame still land at the correct position.
    assert_eq!(render::<3>(&mut voice, &[(0, on(108, 127))]), [200; 3]);
    assert_eq!(
        render::<7>(
            &mut voice,
            &[
                (0, Msg::Stop), // end the previous, independently looping pitch
                (
                    0,
                    Msg::Playback(Playback {
                        mode: PlaybackMode::OneShot,
                        ..p
                    })
                ),
                (0, on(60, 127)),
                (1, Msg::NoteOff(60))
            ]
        ),
        [200, 300, 400, 500, 0, 0, 0]
    );
}

#[cfg(feature = "packed-pcm")]
#[test]
fn pitched_pcm1_matches_raw_for_every_width_and_skips_whole_groups() {
    let mut pcm = Vec::new();
    for width in 1..=16 {
        let min = i16::MIN >> (16 - width);
        pcm.extend([min, !min, -1, 0].into_iter().cycle().take(32));
    }
    pcm.extend([-9, 0, 14]);
    let bytes = fm1_assets::pcm::encode(&pcm).expect("PCM1");
    let raw: Vec<_> = pcm.into_iter().flat_map(i16::to_le_bytes).collect();
    assert!(Clip::from_pcm1(&bytes[..bytes.len() - 1], 22059, 43).is_none());
    let packed = [Clip::from_pcm1(&bytes, 22059, 43).expect("packed")];
    let raw = [Clip::from_pcm16le(&raw, 22059, 43).expect("raw")];
    let mut packed = PitchedSampler::new(&packed, 44118).expect("clock");
    let mut raw = PitchedSampler::new(&raw, 44118).expect("clock");
    for p in [
        Playback::INITIAL,
        Playback {
            mode: PlaybackMode::Loop,
            start: 3,
            end: 98,
            release_ms: 8,
        },
    ] {
        for key in [31, 43, 55, 56, 67, 120, 127] {
            let controls = [(0, Msg::Playback(p)), (0, on(key, 103))];
            assert_eq!(
                render::<37>(&mut packed, &controls),
                render::<37>(&mut raw, &controls)
            );
            for _ in 0..17 {
                assert_eq!(
                    render::<128>(&mut packed, &[]),
                    render::<128>(&mut raw, &[]),
                    "key {key}"
                );
            }
        }
    }
}

#[test]
fn amp_and_filter_have_independent_contours_and_release_a_held_loop() {
    use wavegraph_adsr::Shape;
    use wavegraph_sampler::Timbre;
    let pcm = [12_000_i16.to_le_bytes(); 64];
    let bank = [Clip::from_pcm16le(pcm.as_flattened(), 1000, 60).expect("valid PCM")];
    let mut voice = PitchedSampler::new(&bank, 1000).expect("1 kHz clock");
    let program = Timbre {
        amp: Shape::new(4_000, 0, 50, 4_000),
        filter: Shape::new(20_000, 0, 0, 0),
        ..Timbre::DRY
    };
    let region = Playback {
        mode: PlaybackMode::Loop,
        ..Playback::INITIAL
    };
    let out = render::<12>(
        &mut voice,
        &[
            (0, Msg::Playback(region)),
            (0, Msg::Timbre(program)),
            (0, on(60, 127)),
            (6, Msg::NoteOff(60)),
        ],
    );
    assert_eq!(
        out,
        [
            3000, 6000, 9000, 12000, 6000, 6000, 3375, 1500, 375, 0, 0, 0
        ]
    );
    assert_eq!(render::<2>(&mut voice, &[(0, on(60, 127))]), [3000, 6000]);
}

#[test]
fn cutoff_and_signed_filter_envelope_shape_the_sound_without_restarting_the_cursor() {
    use wavegraph_sampler::Timbre;
    let pcm: Vec<_> = (0..512)
        .map(|i| (if i % 4 < 2 { 10_000_i16 } else { -10_000_i16 }).to_le_bytes())
        .collect();
    let clip = Clip::from_pcm16le(pcm.as_flattened(), 8000, 60).expect("valid PCM");
    let bank = [clip, clip];
    let dark = Timbre {
        cutoff_hz: 500,
        resonance: 6,
        ..Timbre::DRY
    };
    let level = |depth| {
        let mut voice = PitchedSampler::new(&bank, 8000).expect("clock");
        let program = Timbre {
            env_semitones: depth,
            ..dark
        };
        render::<64>(&mut voice, &[(0, Msg::Timbre(program)), (0, on(60, 127))]);
        render::<64>(&mut voice, &[])
            .into_iter()
            .map(i16::unsigned_abs)
            .max()
            .expect("frames")
    };
    assert!(level(-12) < level(0));
    assert!(level(0) < level(12));

    let mut voice = PitchedSampler::new(&bank, 8000).expect("clock");
    let out = render::<48>(
        &mut voice,
        &[
            (0, Msg::Timbre(dark)),
            (0, on(60, 127)),
            (16, Msg::Select(1)),
            (16, Msg::Timbre(Timbre::DRY)),
            (34, Msg::Select(0)),
            (34, Msg::Timbre(Timbre::DRY)),
        ],
    );
    assert!(out[..34].iter().all(|sample| sample.unsigned_abs() < 3000));
    assert_eq!(out[34..38], [-10000, -10000, 10000, 10000]);
}

#[test]
fn varispeed_fits_sixteen_beats_at_the_fractional_dac_clock() {
    // Match both factory Amen encodings. A flat recording makes the true EOF
    // observable even where the musical take has silence.
    for (rate, frames) in [(24_400, 192_000), (12_505, 98_400)] {
        let pcm = vec![16_384_i16.to_le_bytes(); frames];
        let bank = [Clip::from_pcm16le(pcm.as_flattened(), rate, 60)
            .expect("four bars")
            .with_native_tempo(122)];
        for bpm in [30, 90, 122, 160, 300] {
            let mut voice =
                PitchedSampler::at_sample_rate(&bank, 24_000_000.0 / 544.0).expect("FM-1 clock");
            render::<0>(&mut voice, &[(0, Msg::Tempo(bpm)), (0, on(60, 127))]);
            let expected = (16_u64 * 60 * 24_000_000).div_ceil(544 * u64::from(bpm));
            let mut count = 0_u64;
            loop {
                let block = render::<128>(&mut voice, &[]);
                let n = block.iter().take_while(|&&x| x != 0).count();
                count += u64::try_from(n).expect("length");
                if n != 128 {
                    break;
                }
                assert!(count <= expected + 1);
            }
            assert!(
                count.abs_diff(expected) <= 1,
                "{bpm} BPM: {count} vs {expected}"
            );
        }
    }
}

#[test]
fn live_tempo_changes_preserve_phase_and_only_affect_tempo_tagged_clips() {
    let pcm: Vec<_> = (0_i16..64).map(|i| (i * 100).to_le_bytes()).collect();
    let plain = Clip::from_pcm16le(pcm.as_flattened(), 16_000, 60).expect("PCM");
    let bank = [plain.with_native_tempo(120), plain];
    let mut voice = PitchedSampler::new(&bank, 16_000).expect("clock");
    assert_eq!(
        render::<8>(
            &mut voice,
            &[(0, on(60, 127)), (2, Msg::Select(1)), (3, Msg::Tempo(240))]
        ),
        [0, 100, 200, 300, 500, 700, 900, 1100]
    );
    assert_eq!(
        render::<6>(&mut voice, &[(0, on(60, 127)), (2, Msg::Tempo(90))]),
        [0, 100, 200, 300, 400, 500]
    );
}

#[test]
fn velocity_and_key_tracking_change_filter_brightness_independently_of_level() {
    use wavegraph_sampler::Timbre;
    let pcm: Vec<_> = (0..512)
        .map(|i| (if i % 8 < 4 { 10_000_i16 } else { -10_000_i16 }).to_le_bytes())
        .collect();
    let bank = [Clip::from_pcm16le(pcm.as_flattened(), 8000, 60).expect("PCM")];
    let level = |note, velocity, tracking, velocity_depth| {
        let mut voice = PitchedSampler::new(&bank, 8000).expect("clock");
        let p = Timbre {
            cutoff_hz: 1000,
            key_tracking: tracking,
            velocity_semitones: velocity_depth,
            ..Timbre::DRY
        };
        render::<64>(&mut voice, &[(0, Msg::Timbre(p)), (0, on(note, velocity))]);
        render::<64>(&mut voice, &[])
            .into_iter()
            .map(i16::unsigned_abs)
            .max()
            .expect("frames")
    };
    assert_eq!(level(60, 127, 0, 36), level(60, 127, 0, 0));
    assert!(level(60, 64, 0, 36) < level(60, 64, 0, 0));
    assert_eq!(level(60, 127, 100, 0), level(60, 127, 0, 0));
    assert!(level(72, 127, 100, 0) > level(72, 127, 0, 0));
}

#[test]
fn four_cursors_retrigger_by_pitch_and_steal_released_then_oldest() {
    let pcm = [1000_i16, 2000, 4000, 8000, 16000].map(|v| [v.to_le_bytes(); 32]);
    let bank = pcm
        .each_ref()
        .map(|v| Clip::from_pcm16le(v.as_flattened(), 1000, 60).expect("clip"));
    let mut player = PitchedSampler::new(&bank, 1000).expect("clock");
    let looped = Playback {
        mode: PlaybackMode::Loop,
        ..Playback::INITIAL
    };
    let mut start = vec![(0, Msg::Playback(looped))];
    for i in 0..4_u8 {
        start.extend([(0, Msg::Select(i)), (0, on(60 + i, 127))]);
    }
    assert_eq!(render::<4>(&mut player, &start), [15000; 4]);
    // Key 62 is younger than 60, but its release tail is available first.
    assert_eq!(
        render::<3>(
            &mut player,
            &[
                (0, on(62, 0)),
                (0, Msg::Select(4)),
                (0, on(64, 127)),
                (1, Msg::NoteOff(62))
            ]
        ),
        [27000; 3]
    );
    // A restrike replaces pitch 60 and makes it the newest. 61 is now the oldest.
    assert_eq!(
        render::<2>(
            &mut player,
            &[
                (0, Msg::Select(2)),
                (0, on(60, 127)),
                (1, Msg::Select(0)),
                (1, on(67, 127))
            ]
        ),
        [30000, 29000]
    );
    assert_eq!(
        render::<3>(&mut player, &[(0, Msg::NoteOff(61)), (1, Msg::Stop)]),
        [29000, 0, 0]
    );
}

#[test]
fn overlapping_voices_sum_in_a_wide_accumulator() {
    let pcm = [30_000_i16, 20_000, -24_000, -30_000].map(|v| [v.to_le_bytes(); 64]);
    let bank = pcm
        .each_ref()
        .map(|v| Clip::from_pcm16le(v.as_flattened(), 1000, 60).expect("clip"));
    let mut player = PitchedSampler::new(&bank, 1000).expect("clock");
    let mut events = Vec::new();
    for i in 0..4_u8 {
        events.extend([
            (usize::from(i), Msg::Select(i)),
            (usize::from(i), on(60 + i, 127)),
        ]);
    }
    assert_eq!(
        render::<4>(&mut player, &events),
        [30000, 32767, 26000, -4000]
    );
}

#[cfg(feature = "packed-pcm")]
#[test]
fn predictive_pcm_matches_raw_through_block_seeks_loops_and_chords() {
    let pcm: Vec<_> = (0_u16..773)
        .map(|i| i.wrapping_mul(907).rotate_left(3).cast_signed())
        .collect();
    let mut residuals = pcm.clone();
    for block in residuals.chunks_mut(128) {
        let (mut prev, mut slope) = (0_i16, 0_i16);
        for x in block {
            let d = x.wrapping_sub(prev);
            prev = *x;
            *x = d.wrapping_sub(slope);
            slope = d;
        }
    }
    let encoded = fm1_assets::pcm::encode(&residuals).expect("PCM1");
    let raw: Vec<_> = pcm.into_iter().flat_map(i16::to_le_bytes).collect();
    let raw = [Clip::from_pcm16le(&raw, 22059, 43).expect("raw")];
    let packed = [Clip::from_delta_pcm1(&encoded, 22059, 43).expect("predictive")];
    let mut actual = PitchedSampler::new(&packed, 44118).expect("clock");
    let mut reference = PitchedSampler::new(&raw, 44118).expect("clock");
    for start in [0, 16, 33, 80, 95] {
        let clip = Playback {
            mode: PlaybackMode::Loop,
            start,
            end: 99,
            release_ms: 15,
        };
        let events = [
            (0, Msg::Stop),
            (0, Msg::Playback(clip)),
            (0, on(55, 54)),
            (5, on(79, 21)),
            (7, on(118, 44)),
            (13, on(127, 24)),
        ];
        assert_eq!(
            render::<29>(&mut reference, &events),
            render::<29>(&mut actual, &events)
        );
        for _ in 0..8 {
            assert_eq!(
                render::<128>(&mut reference, &[]),
                render::<128>(&mut actual, &[])
            );
        }
    }
}
