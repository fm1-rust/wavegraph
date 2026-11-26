//! Frequency-selective gain reduction and detector routing.
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_fx::{Message, SidechainDucker};
use wavegraph_sample::I24;

fn event(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn render<const N: usize>(
    insert: &mut SidechainDucker<3>,
    x: [i32; N],
    keys: [i32; 2],
    controls: &[Event<Message>],
) -> [i32; N] {
    let mut out = [I24::default(); N];
    let inputs = [x, [keys[0]; N], [keys[1]; N]].map(|bus| bus.map(I24::new_unchecked));
    insert.process(&mut ProcessContext {
        frames: N,
        inputs: &inputs.each_ref().map(|bus| &bus[..]),
        outputs: &mut [&mut out],
        events: controls,
        emit: &mut Err,
    });
    out.map(I24::inner)
}

#[test]
fn selecting_a_key_at_a_frame_ducks_only_the_program() {
    assert!(SidechainDucker::<1>::new(1000, 64, 128).is_none());
    assert!(SidechainDucker::<3>::new(0, 64, 128).is_none());
    let mut insert = SidechainDucker::new(1000, 64, 128).expect("valid gains");
    let keys = [0, -(1 << 23)];
    assert_eq!(render(&mut insert, [4096; 128], keys, &[]), [4096; 128]);
    assert_eq!(
        render(
            &mut insert,
            [4096; 5],
            keys,
            &[event(2, Message::SidechainKey(2))]
        ),
        [4096, 4096, 3072, 2048, 2048]
    );
    let recovery = render(&mut insert, [4096; 90], [0; 2], &[]);
    assert!(recovery.windows(2).all(|p| p[0] <= p[1]));
    assert!(recovery[0] > 2048 && recovery[0] < 2100);
    assert_eq!(recovery[89], 4096);
    let mut silent = SidechainDucker::new(44_118, 64, 176).expect("valid gains");
    assert_eq!(render(&mut silent, [0; 512], [1 << 22; 2], &[]), [0; 512]);
}

#[test]
fn low_band_ducks_deeper_and_bypass_returns_sample_exact_program() {
    let mut insert = SidechainDucker::new(44_118, 32, 160).expect("valid gains");
    let keys = [1 << 22, 0];
    let dc = [65_536; 1024];
    assert_eq!(render(&mut insert, dc, keys, &[])[1023], 16_384);
    // An alternating signal has negligible energy below the shelf.
    let highs = core::array::from_fn::<_, 1024, _>(|i| if i % 2 == 0 { 65_536 } else { -65_536 });
    let high = render(&mut insert, highs, keys, &[]);
    assert!((48_000..50_000).contains(&high[1023].abs()));
    let bypass = render(
        &mut insert,
        highs,
        keys,
        &[event(120, Message::Enabled(false))],
    );
    assert_eq!(bypass[119], high[1023]);
    assert_eq!(&bypass[184..], &highs[184..]);
    let straight = render(
        &mut insert,
        dc,
        keys,
        &[
            event(0, Message::SidechainDepth(0)),
            event(0, Message::Enabled(true)),
        ],
    );
    assert_eq!(straight[1023], 81_920);
}

#[test]
fn serial_in_place_matches_graph_routing_across_keys_bypass_and_block_boundaries() {
    let mut graph = SidechainDucker::<3>::new(44_118, 32, 176).expect("valid gains");
    let mut serial = graph;
    for round in 0..18 {
        let self_key = round % 2 == 0;
        let controls = [
            event(0, Message::SidechainKey(if self_key { 0 } else { 2 })),
            event(0, Message::Enabled(round % 3 != 0)),
            event(
                0,
                Message::SidechainDepth(if round % 4 == 0 { 0 } else { 75 }),
            ),
            event(
                0,
                Message::SidechainHz(if round % 5 == 0 { 1000 } else { 180 }),
            ),
        ];
        let program = core::array::from_fn::<_, 257, _>(|i| {
            if round >= 12 {
                0
            } else {
                [
                    0,
                    (1 << 23) - 1,
                    -(1 << 23),
                    65_536,
                    -65_536,
                    123_457,
                    -832_003,
                ][i % 7]
            }
        });
        let keys = if round >= 12 {
            [0; 2]
        } else {
            [1 << 22, -(1 << 21)]
        };
        let expected = render(&mut graph, program, keys, &controls);
        render(&mut serial, [], [0; 2], &controls);
        let mut actual = program.map(I24::new_unchecked);
        let external = [I24::new_unchecked(keys[1]); 257];
        // Include empty spans, the former 32-frame scratch boundary, and a
        // bypass ramp ending inside a span. Self-keying must read the dry sample.
        for (start, end) in [(0, 0), (0, 1), (1, 32), (32, 64), (64, 129), (129, 257)] {
            serial.process_in_place(
                &mut actual[start..end],
                if self_key {
                    None
                } else {
                    Some(&external[start..end])
                },
            );
        }
        assert_eq!(actual.map(I24::inner), expected, "round {round}");
    }
}
