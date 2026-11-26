//! Exercise the kit and its dry outlets through the patch interpreter.

use wavegraph_909::{HandClap, Samples, Tr909, Voice, Voice::*};
use wavegraph_dsp::{Event, Process, ProcessContext, bytecode::Program, dsp::Software};
use wavegraph_sample::{I24, Sample};

const SAMPLES: Samples<'_> = Samples {
    closed_hat: &[1, 0, 2, 0, 3, 0],
    open_hat: &[4, 0, 5, 0, 6, 0, 7, 0],
    ride: &[16, 0, 17, 0, 18, 0, 19, 0],
};

fn hit(frame: usize, message: Voice) -> Event<Voice> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn render<const N: usize>(kit: &mut Tr909<'_>, hits: &[Event<Voice>]) -> [[i16; N]; 7] {
    let program = Program::<1, 7>::decode(&[1, 1, 0, 0, 7]).expect("one seven-output call");
    let mut objects: [&mut dyn Process<Voice, I24>; 1] = [kit];
    let mut patch = program.bind(&mut objects).expect("kit ports");
    let mut buses = [[I24::from_sample(77_i16); N]; 7];
    patch
        .render(&mut Software, &mut buses, &[hits], |_, e| Err(e))
        .expect("valid events");
    buses.map(|bus| bus.map(Sample::to_sample))
}

#[test]
fn hats_choke_on_the_strike_frame_while_the_ride_rings_across_blocks() {
    let mut kit = Tr909::new(SAMPLES).expect("whole frames");
    let a = render::<3>(&mut kit, &[hit(1, OpenHat), hit(2, Ride)]);
    assert_eq!(a[0], [0, 4, 21]);
    assert_eq!(a[OpenHat.outlet()], [0, 4, 5]);
    assert_eq!(a[Ride.outlet()], [0, 0, 16]);
    let b = render::<3>(
        &mut kit,
        &[hit(0, ClosedHat), hit(1, ClosedHat), hit(2, OpenHat)],
    );
    assert_eq!(b[0], [18, 19, 23]);
    assert_eq!(b[ClosedHat.outlet()], [1, 1, 0]);
    assert_eq!(b[OpenHat.outlet()], [0, 0, 4]);
    assert_eq!(b[Ride.outlet()], [17, 18, 19]);
}

#[test]
fn simultaneous_hats_select_open_in_either_order_even_across_a_zero_frame_call() {
    for (first, second) in [(OpenHat, ClosedHat), (ClosedHat, OpenHat)] {
        let mut kit = Tr909::new(SAMPLES).expect("whole frames");
        let mut split = kit;
        let expected = render::<2>(&mut kit, &[hit(0, first), hit(0, second)]);
        assert_eq!(expected[OpenHat.outlet()], [4, 5]);
        assert_eq!(expected[ClosedHat.outlet()], [0, 0]);
        render::<0>(&mut split, &[hit(0, first)]);
        assert_eq!(render::<2>(&mut split, &[hit(0, second)]), expected);
    }
}

#[test]
fn mix_saturates_after_summing_and_preserves_the_dry_outputs() {
    let peaks = &[0xff, 0x7f, 0, 0x80, 0xff, 0x7f];
    let mut kit = Tr909::new(Samples {
        closed_hat: peaks,
        open_hat: &[],
        ride: &[0xff, 0x7f, 0, 0x80, 0, 0x80],
    })
    .expect("whole frames");
    let buses = render::<3>(&mut kit, &[hit(0, ClosedHat), hit(0, Ride)]);
    assert_eq!(buses[0], [i16::MAX, i16::MIN, -1]);
    assert_eq!(buses[ClosedHat.outlet()], [i16::MAX, i16::MIN, i16::MAX]);
    assert_eq!(buses[Ride.outlet()], [i16::MAX, i16::MIN, i16::MIN]);
}

#[test]
fn incomplete_pcm_is_rejected_at_construction() {
    assert!(
        Tr909::new(Samples {
            ride: &[1],
            ..SAMPLES
        })
        .is_none()
    );
}

#[test]
fn clap_process_matches_the_kit_stem_across_blocks_and_retriggers() {
    let mut clap = HandClap::new(44_100, 1).expect("audio clock");
    let mut dry = [0_i16; 1024];
    clap.process(&mut ProcessContext {
        frames: dry.len(),
        inputs: &[],
        outputs: &mut [&mut dry],
        events: &[16, 700].map(|frame| Event {
            frame,
            port: 0,
            message: (),
        }),
        emit: &mut Err,
    });
    assert_eq!(&dry[..16], &[0; 16]);
    assert!(dry[16..100].iter().any(|&s| s != 0));
    assert_ne!(&dry[16..80], &dry[700..764]);

    let mut kit = Tr909::new(SAMPLES).expect("whole frames");
    let first = render::<512>(&mut kit, &[hit(16, Clap), hit(300, Ride)]);
    let second = render::<512>(&mut kit, &[hit(188, Clap)]);
    assert_eq!(first[Clap.outlet()], dry[..512]);
    assert_eq!(second[Clap.outlet()], dry[512..]);
    assert_eq!(first[0][300], dry[300] + 16);
    assert_eq!(second[0], second[Clap.outlet()]);
}

#[test]
fn clap_bursts_keep_their_timing_at_different_graph_rates() {
    assert!(HandClap::new(0, 1).is_none());
    assert!(Tr909::with_sample_rate(SAMPLES, 1, 0).is_none());
    for rate in [32_000_u32, 44_100, 48_000, 96_000] {
        let mut clap = HandClap::new(rate, 1).expect("audio clock");
        clap.trigger();
        let hit: Vec<_> = (0..rate).map(|_| i64::from(clap.next_frame())).collect();
        let window = (rate / 400) as usize;
        let energy = |start| {
            hit[start..start + window]
                .iter()
                .map(|s| s * s)
                .sum::<i64>()
        };
        for burst in 0..3 {
            let start = ((rate * 107 * burst + 5_000) / 10_000) as usize;
            assert!(energy(start) > 3 * energy(start + 3 * window));
        }
        assert!(energy((rate / 30) as usize) > energy((rate / 10) as usize));
        assert!(energy((rate / 10) as usize) > 0);
        assert!(hit[(rate * 99 / 100) as usize..].iter().all(|&s| s == 0));
    }
}

#[test]
fn all_six_voices_reach_their_own_stem_and_mono_render() {
    for voice in Voice::ALL {
        let mut kit = Tr909::new(Samples::X0XPROC1).expect("bundled kit");
        let mut mono = kit;
        let buses = render::<1024>(&mut kit, &[hit(0, voice)]);
        assert!(buses[voice.outlet()].iter().any(|&s| s != 0), "{voice:?}");
        assert_eq!(buses[0], buses[voice.outlet()]);
        for other in Voice::ALL.into_iter().filter(|&v| v != voice) {
            assert_eq!(buses[other.outlet()], [0; 1024]);
        }
        mono.trigger(voice);
        assert_eq!(
            buses[0],
            core::array::from_fn(|_| mono.next_mix_frame::<i16>())
        );
        mono.stop();
        assert_eq!(mono.next_mix_frame::<i16>(), 0);
    }
}

#[test]
fn short_decay_reduces_modeled_tails_and_preserves_the_cymbal_recordings() {
    for voice in Voice::ALL {
        let mut stock = Tr909::new(Samples::X0XPROC1).expect("44.1 kHz kit");
        let mut short = Tr909::with_decay(Samples::X0XPROC1, 44_100, 1, 55).expect("short kit");
        stock.trigger(voice);
        short.trigger(voice);
        let mut tail = [0_i64; 2];
        for frame in 0..9000 {
            let pair = [
                stock.next_voices()[voice as usize],
                short.next_voices()[voice as usize],
            ];
            if matches!(voice, ClosedHat | OpenHat | Ride) || frame == 0 {
                assert_eq!(pair[0], pair[1]);
            }
            if frame >= 4410 {
                for (energy, sample) in tail.iter_mut().zip(pair) {
                    *energy += i64::from(sample).pow(2);
                }
            }
        }
        if matches!(voice, Kick | Snare | Clap) {
            assert!(tail[1] < tail[0] / 2);
        }
    }
}

#[test]
fn kick_settles_to_a_low_fundamental_and_decays() {
    let mut kick = wavegraph_909::BassDrum::new(48_000, 1).expect("clock");
    assert_eq!(kick.next_frame(), 0);
    kick.trigger();
    let samples: Vec<_> = (0..24_000).map(|_| kick.next_frame()).collect();
    let crossings: Vec<_> = samples
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0] < 0 && pair[1] >= 0)
        .map(|(n, _)| n)
        .collect();
    // Between 60 and 180 ms the oscillator should be within 2 Hz of 50 Hz.
    let settled: Vec<_> = crossings
        .into_iter()
        .filter(|n| (2880..8640).contains(n))
        .collect();
    assert!(settled.len() >= 5);
    for cycle in settled.windows(2) {
        assert!((923..=1000).contains(&(cycle[1] - cycle[0])));
    }
    assert!(samples[14_400..].iter().all(|&sample| sample == 0));
}

#[test]
fn the_demo_has_a_kick_on_every_quarter_and_uses_the_whole_kit() {
    let pattern = wavegraph_909::DEMO_PATTERN;
    for (index, step) in pattern.iter().enumerate() {
        assert_eq!(step.contains(&Kick), index % 4 == 0);
        assert_eq!(step.contains(&Snare), index % 8 == 4);
        assert_eq!(step.contains(&Clap), index % 8 == 4);
    }
    for voice in Voice::ALL {
        assert!(pattern.iter().any(|step| step.contains(&voice)));
    }
}

#[test]
fn editing_a_live_decay_changes_only_its_modeled_voice() {
    for (index, voice) in [Kick, Snare, Clap].into_iter().enumerate() {
        let mut stock = Tr909::with_decay(SAMPLES, 44_100, 1, 55).expect("kit clock");
        for v in [Kick, Snare, Clap] {
            stock.trigger(v);
        }
        for _ in 0..2400 {
            stock.next_voices();
        }
        stock.trigger(OpenHat);
        stock.trigger(Ride);
        let mut short = stock;
        let mut decay = [55; 3];
        decay[index] = 5;
        short.set_decay(decay);
        let mut changed = false;
        for _ in 0..2048 {
            let a = short.next_voices();
            let b = stock.next_voices();
            for v in Voice::ALL {
                if v == voice {
                    changed |= a[v as usize] != b[v as usize];
                } else {
                    assert_eq!(a[v as usize], b[v as usize]);
                }
            }
        }
        assert!(changed, "{voice:?} decay affects its ringing tail");
    }
}

#[test]
fn active_stems_match_samplewise_strikes_chokes_and_releases() {
    let mut block = Tr909::new(SAMPLES).expect("test kit");
    let mut reference = block;
    for (voices, len) in [
        (&[OpenHat, Ride][..], 0),
        (&[ClosedHat, Kick][..], 1),
        (&[Snare, Clap][..], 3),
        (&[ClosedHat, Ride][..], 63),
        (&[][..], 127),
        (&[OpenHat, ClosedHat][..], 128),
    ] {
        for &v in voices {
            reference.trigger(v);
            block.trigger(v);
        }
        let mut stems = [[0; 6]; 128];
        block.render_active_stems(&mut [0; 128][..len], |voice, samples| {
            for (frame, sample) in stems.iter_mut().zip(samples) {
                frame[voice as usize] = *sample;
            }
        });
        for frame in &stems[..len] {
            assert_eq!(*frame, reference.next_voices());
        }
        for voice in Voice::ALL {
            assert_eq!(block.voice_active(voice), reference.voice_active(voice));
        }
    }
}
