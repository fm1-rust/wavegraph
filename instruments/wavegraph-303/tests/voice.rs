//! Check note events, oscillator/filter timing, and post-VCA drive against CHOMPI.

#[cfg(feature = "raw")]
use wavegraph_303::raw;
use wavegraph_303::{Drive, Voice, VoiceParams};

#[test]
fn accent_slide_and_drive() {
    let mut voice = Voice::new(48_000.0);
    let mut drive = Drive::new(48_000.0);
    let params = VoiceParams::default();
    // Last dry and driven sample of each 48-frame block in the C++ reference.
    let blocks = [
        (Some((36, true, false)), 0.005, -0.038_369_18, -0.038_369_18),
        (Some((43, false, true)), 0.65, -0.019_210_592, 0.005_572_676),
        (None, 1.0, -0.000_839_297_2, 0.098_428_27),
    ];
    for (note, amount, expected_dry, expected_driven) in blocks {
        match note {
            Some((pitch, accent, slide)) => voice.note_on(pitch, accent, slide),
            None => voice.note_off(),
        }
        let mut dry = [0.0; 48];
        voice.render_add(&params, &mut dry);
        let mut driven = dry;
        drive.process(amount, &mut driven);
        assert_eq!(voice.gate(), note.is_some());
        assert!(voice.sounding());
        if amount == 0.005 {
            assert_eq!(driven, dry);
        }
        assert!((dry[47] - expected_dry).abs() < 2e-6);
        assert!((driven[47] - expected_driven).abs() < 2e-6);
    }
}

#[test]
fn vca_attack_and_filter_overdrive_shape_the_onset() {
    fn onset(attack_s: f32, overdrive_db: f32) -> [f32; 512] {
        let mut voice = Voice::new(48_000.0);
        let params = VoiceParams {
            attack_s,
            overdrive_db,
            ..Default::default()
        };
        let mut samples = [0.0; 512];
        voice.note_on(48, false, false);
        voice.render_add(&params, &mut samples);
        samples
    }

    let stock = onset(0.0015, 0.0);
    let slow = onset(0.06, 0.0);
    let driven = onset(0.0015, 24.0);
    let energy = |samples: &[f32]| samples.iter().map(|x| x * x).sum::<f32>();
    assert!(energy(&slow) < energy(&stock) * 0.05);
    assert!(energy(&driven) > energy(&stock));
    // A change in output volume alone would leave zero residual after gain matching.
    let gain = stock.iter().zip(&driven).map(|(a, b)| a * b).sum::<f32>() / energy(&stock);
    let residual: f32 = stock
        .iter()
        .zip(&driven)
        .map(|(a, b)| (gain * a - b).powi(2))
        .sum();
    assert!(residual > energy(&driven) * 0.01);
}

#[test]
#[cfg(feature = "raw")]
fn coefficient_phase_and_drive_history_cross_block_boundaries() {
    for rate in [44_100.0, 24_000_000.0 / 544.0, 48_000.0, 96_000.0] {
        let mut voice = Voice::new(rate);
        let mut drive = Drive::new(rate);
        let mut reference = raw::x0x_Voice::default();
        let mut rack = raw::x0x_Fx::default();
        let mut delay = [raw::x0x_Fx_Frame::default(); 1024];
        // SAFETY: The delay lives through the final call. All other buffers are
        // disjoint and valid for exactly the count passed to each reference method.
        unsafe {
            reference.Init(rate);
            rack.Init(rate, delay.as_mut_ptr(), delay.len());
            for i in 0..48 {
                let note = if i < 12 { 36 } else { 50 };
                match i {
                    0 | 12 | 34 => {
                        voice.note_on(note, i == 0, i != 0);
                        reference.NoteOn(note, i == 0, i != 0);
                    }
                    24 => {
                        voice.note_off();
                        reference.NoteOff();
                    }
                    33 => {
                        voice.reset();
                        reference.Reset();
                    }
                    _ => {}
                }
                let count = [1, 3, 0, 13, 47, 48, 97][i % 7];
                let mut actual = [0.25; 97];
                let mut expected = actual;
                let mut left = [0.0; 97];
                let mut right = left;
                let params = VoiceParams {
                    square: i >= 16,
                    cutoff_hz: if i < 16 { 950.0 } else { 370.0 },
                    ..Default::default()
                };
                let reference_params = raw::x0x_VoiceParams {
                    square: params.square,
                    cutoff_hz: params.cutoff_hz,
                    ..Default::default()
                };
                voice.render_add(&params, &mut actual[..count]);
                reference.Process(&reference_params, expected.as_mut_ptr(), count);
                samples_match(&actual[..count], &expected[..count], 3e-7, "dry");
                assert_eq!(voice.gate(), reference.Gate());
                assert_eq!(voice.sounding(), reference.Sounding());
                let amount = [0.65, 0.005, 0.00501, 1.0][i % 4];
                drive.process(amount, &mut actual[..count]);
                rack.Set(&raw::x0x_Fx_Settings {
                    drive: amount,
                    ..Default::default()
                });
                rack.Process(
                    expected.as_ptr(),
                    core::ptr::null(),
                    left.as_mut_ptr(),
                    right.as_mut_ptr(),
                    count,
                );
                samples_match(&actual[..count], &left[..count], 3e-6, "driven");
                assert_eq!(left, right);
            }
        }
    }
}

#[cfg(feature = "raw")]
fn samples_match(actual: &[f32], expected: &[f32], tolerance: f32, label: &str) {
    for (&a, &b) in actual.iter().zip(expected) {
        assert!((a - b).abs() <= tolerance, "{label}: {a} vs {b}");
    }
}

#[test]
fn odd_splits_preserve_the_control_clock() {
    let mut contiguous = Voice::new(48_000.0);
    let mut split = Voice::new(48_000.0);
    let settings = VoiceParams::default();
    let mut a = [0.25; 257];
    let mut b = a;
    contiguous.note_on(43, true, false);
    split.note_on(43, true, false);
    contiguous.render_add(&settings, &mut a);
    for block in b.chunks_mut(7) {
        split.render_add(&settings, block);
    }
    assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
}

#[cfg(feature = "raw")]
#[test]
fn release_ends_on_the_reference_frame() {
    for rate in [44_100.0, 24_000_000.0 / 544.0, 48_000.0, 96_000.0] {
        let mut voice = Voice::new(rate);
        let mut reference = raw::x0x_Voice::default();
        let params = VoiceParams::default();
        let raw_params = raw::x0x_VoiceParams::default();
        // SAFETY: Both calls receive a live parameter block and writable storage
        // for the requested count.
        unsafe {
            reference.Init(rate);
            for attack_frames in [1, 31, 900] {
                voice.reset();
                reference.Reset();
                voice.note_on(36, true, false);
                reference.NoteOn(36, true, false);
                let mut attack = [0.0; 900];
                voice.render_add(&params, &mut attack[..attack_frames]);
                attack.fill(0.0);
                reference.Process(&raw_params, attack.as_mut_ptr(), attack_frames);
                voice.note_off();
                reference.NoteOff();
                assert!(voice.sounding() && reference.Sounding());
                for _ in 0..4096 {
                    voice.render_add(&params, &mut [0.0]);
                    reference.Process(&raw_params, [0.0].as_mut_ptr(), 1);
                    assert_eq!(voice.sounding(), reference.Sounding());
                    if !voice.sounding() {
                        break;
                    }
                }
                assert!(!voice.sounding(), "release did not end at {rate} Hz");
            }
        }
    }
}

#[test]
fn closely_spaced_accents_retain_charge_across_gates() {
    use wavegraph_303::Envelopes;
    let params = VoiceParams {
        resonance: 1.0,
        accent: 1.0,
        ..VoiceParams::default()
    };
    for samples_per_ms in [4_u16, 48] {
        let ms = usize::from(samples_per_ms);
        let mut env = Envelopes::new(f32::from(samples_per_ms) * 1000.0);
        let mut previous_peak = 0.0;
        for _ in 0..3 {
            env.note_on(true, false);
            let mut peak = 0.0_f32;
            for _ in 0..56 {
                peak = peak.max(env.advance(&params, ms).accent_octaves);
            }
            assert!(peak > previous_peak && peak < 3.0);
            previous_peak = peak;
            env.note_off();
            let tail = env.advance(&params, 40 * ms);
            assert!(tail.accent_octaves > 0.0 && tail.accent_octaves < peak);
        }
        // A long rest discharges the sweep. The following plain note adds no charge.
        env.note_off();
        let faded = env.advance(&params, 4_000 * ms);
        env.note_on(false, false);
        assert!(faded.accent_octaves < 0.001);
        assert!(env.advance(&params, 96 * ms).accent_octaves <= faded.accent_octaves);
    }
}
