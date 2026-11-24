//! Dry outlets, independently patchable voices and sample-timed kit behavior.

use wavegraph_808::{BassDrum, Controls, HiHat, Message, Metal, SnareDrum, Tr808, Voice, Voice::*};
use wavegraph_dsp::{Event, Process, bytecode::Program, dsp::Software};
use wavegraph_sample::{I24, Sample};

const RATE: (u32, u32) = (24_000_000, 544);

fn kit() -> Tr808 {
    Tr808::new(RATE.0, RATE.1).expect("FM-1 clock")
}

fn event<M>(frame: usize, message: M) -> Event<M> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn render<const N: usize>(kit: &mut Tr808, events: &[Event<Message>]) -> [[i16; N]; 5] {
    let program = Program::<1>::decode(&[1, 1, 0, 0, 5]).expect("kit call");
    let mut nodes: [&mut dyn Process<Message, I24>; 1] = [kit];
    let mut buses = [[I24::from_sample(17_i16); N]; 5];
    program
        .bind(&mut nodes)
        .expect("one mix and four stems")
        .render(&mut Software, &mut buses, &[events], |_, e| Err(e))
        .expect("timed hits");
    buses.map(|b| b.map(Sample::to_sample))
}

#[test]
fn midi_retriggers_chords_and_mix_taps_survive_block_boundaries() {
    let mut whole = kit();
    let mut split = whole;
    let strike =
        |frame, note, velocity| event(frame, Message::note_on(note, velocity).expect("mapped"));
    let hits = [
        strike(19, 36, 84),
        strike(19, 38, 72),
        strike(19, 42, 60),
        strike(65, 38, 111),
        strike(127, 36, 83),
        strike(128, 46, 113),
    ];
    let all = render::<256>(&mut whole, &hits);
    let a = render::<128>(&mut split, &hits[..5]);
    let b = render::<128>(&mut split, &[strike(0, 46, 113)]);
    for ((bus, a), b) in all.iter().zip(a).zip(b) {
        assert_eq!(bus[..128], a);
        assert_eq!(bus[128..], b);
        assert!(bus[..19].iter().all(|&s| s == 0));
    }
    for i in 0..256 {
        let sum: i32 = all[1..].iter().map(|bus| i32::from(bus[i])).sum();
        assert_eq!(i32::from(all[0][i]), sum);
    }
    for voice in Voice::ALL {
        assert!(all[voice.outlet()].iter().any(|&s| s != 0));
    }
    assert_eq!(Message::note_on(36, 0), None);
    assert_eq!(Message::note_on(39, 127), None);
}

#[test]
fn a_patch_of_standalone_children_matches_the_kit_stems() {
    let mut kick = BassDrum::new(RATE.0, RATE.1).expect("clock");
    let mut snare = SnareDrum::new(RATE.0, RATE.1).expect("clock");
    let mut metal = Metal::new(RATE.0, RATE.1).expect("clock");
    let mut closed = HiHat::closed(RATE.0, RATE.1).expect("clock");
    let mut open = HiHat::open(RATE.0, RATE.1).expect("clock");
    let mut nodes: [&mut dyn Process<u8, I24>; 5] =
        [&mut kick, &mut snare, &mut metal, &mut closed, &mut open];
    // The same metal bus (2) feeds the two hat nodes.
    let program = Program::<5>::decode(&[
        1, 1, 0, 0, 1, 1, 1, 0, 1, 1, 2, 0, 1, 1, 3, 1, 1, 2, 1, 4, 1, 1, 2,
    ])
    .expect("five voices/sources");
    let mut patch = program.bind(&mut nodes).expect("patchable voice ports");
    let mut kits = [kit(); 4];
    for block in 0..4 {
        let hits: &[Event<u8>] = if block == 2 {
            &[event(23, 71), event(108, 90)]
        } else {
            &[]
        };
        let mut buses = [[I24::default(); 128]; 5];
        patch
            .render(
                &mut Software,
                &mut buses,
                &[hits, hits, &[], hits, hits],
                |_, e| Err(e),
            )
            .expect("render");
        for ((voice, kit), bus) in Voice::ALL.into_iter().zip(&mut kits).zip([0, 1, 3, 4]) {
            let strikes: Vec<_> = hits
                .iter()
                .map(|h| event(h.frame, Message::Strike(voice, h.message)))
                .collect();
            let outputs = render::<128>(kit, &strikes);
            assert_eq!(
                outputs[voice.outlet()],
                buses[bus].map(Sample::to_sample::<i16>)
            );
            assert_eq!(outputs[0], outputs[voice.outlet()]);
        }
    }
}

#[test]
fn simultaneous_hats_make_a_short_open_hat_in_either_order() {
    let mut reference = kit();
    let expected = render::<2048>(
        &mut reference,
        &[
            event(0, Message::Strike(OpenHat, 108)),
            event(0, Message::Strike(ClosedHat, 77)),
        ],
    );
    assert!(expected[ClosedHat.outlet()].iter().all(|&v| v == 0));
    assert!(expected[OpenHat.outlet()][..200].iter().any(|&v| v != 0));
    assert!(!reference.voice_active(OpenHat));
    for (first, second) in [
        ((ClosedHat, 77), (OpenHat, 108)),
        ((OpenHat, 108), (ClosedHat, 77)),
    ] {
        let mut kit = kit();
        render::<0>(&mut kit, &[event(0, Message::Strike(first.0, first.1))]);
        assert_eq!(
            render::<2048>(&mut kit, &[event(0, Message::Strike(second.0, second.1))]),
            expected
        );
    }
    let mut ringing = kit();
    render::<2048>(&mut ringing, &[event(0, Message::Strike(OpenHat, 108))]);
    assert!(ringing.voice_active(OpenHat));
    render::<2048>(&mut ringing, &[event(0, Message::Strike(ClosedHat, 77))]);
    assert!(!ringing.voice_active(OpenHat));
}

#[test]
fn velocity_accent_and_voice_levels_are_independent() {
    let hits = |kick| {
        [
            event(0, Message::Strike(Kick, kick)),
            event(0, Message::Strike(Snare, 82)),
        ]
    };
    let base = render::<512>(&mut kit(), &hits(64));
    let loud = render::<512>(&mut kit(), &hits(112));
    assert_ne!(base[Kick.outlet()], loud[Kick.outlet()]);
    assert_eq!(base[Snare.outlet()], loud[Snare.outlet()]);
    let peak = |s: &[i16]| s.iter().map(|s| s.unsigned_abs()).max().expect("nonempty");
    let mut accented = kit();
    accented.send(Message::Accent(127));
    let accented = render::<512>(&mut accented, &hits(64));
    for v in [Kick, Snare] {
        assert!(peak(&accented[v.outlet()]) > peak(&base[v.outlet()]));
    }
    let mut muted = kit();
    muted.send(Message::Level(Kick, 0));
    let muted = render::<512>(&mut muted, &hits(64));
    assert_eq!(muted[0], base[Snare.outlet()]);
}

#[test]
fn kick_settles_near_51_hz_and_decay_changes_the_tail_at_two_rates() {
    for rate in [32_000_u32, 48_000] {
        let mut kick = BassDrum::new(rate, 1).expect("clock");
        let mut short = kick;
        short.set_decay(0);
        kick.set_decay(127);
        short.trigger(100);
        kick.trigger(100);
        let mut tail = [0_i64; 2];
        let mut crossings = Vec::new();
        let mut previous = 0;
        for frame in 0..rate / 2 {
            let long = kick.next_frame();
            let short = short.next_frame();
            if frame > rate / 8 && previous < 0 && long >= 0 {
                crossings.push(frame);
            }
            if frame > rate / 4 {
                tail[0] += i64::from(short).pow(2);
                tail[1] += i64::from(long).pow(2);
            }
            previous = long;
        }
        assert!(crossings.len() >= 15);
        for pair in crossings.windows(2) {
            assert!((rate / 52..=rate / 50).contains(&(pair[1] - pair[0])));
        }
        assert!(tail[1] > tail[0] * 20);
    }
    assert!(Tr808::new(0, 1).is_none() && Tr808::new(44_100, 0).is_none());
}

#[test]
fn snare_tone_changes_the_shell_pitch_and_snappy_adds_wires() {
    let snare = |tone, snappy| {
        let mut kit = kit();
        kit.send(Message::Controls(Controls {
            snare_tone: tone,
            snappy,
            ..Controls::INITIAL
        }));
        render::<4096>(&mut kit, &[event(0, Message::Strike(Snare, 110))])[Snare.outlet()]
    };
    let crossings = |s: &[i16]| s.windows(2).filter(|p| p[0] < 0 && p[1] >= 0).count();
    let low = crossings(&snare(0, 0));
    let high = crossings(&snare(127, 0));
    assert!((15..=17).contains(&low));
    assert!((29..=31).contains(&high));
    assert!(crossings(&snare(127, 127)) > high * 3);
}
