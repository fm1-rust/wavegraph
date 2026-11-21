//! Pin the on-wire representation at the host/device boundary.
use wavegraph_core::{
    Op,
    bytecode::{DecodeError, EncodeError},
};

#[test]
fn every_instruction_round_trips_and_leaves_the_next_opcode_unconsumed() {
    let inputs = [7, 2];
    for (op, bytes) in [
        (
            Op::Call {
                object: 9,
                inputs: &inputs,
                outputs: 3,
            },
            &[1, 9, 2, 3, 7, 2][..],
        ),
        (
            Op::Gain {
                input: 12,
                level: 128,
            },
            &[2, 12, 128],
        ),
        (Op::Mix { left: 7, right: 4 }, &[3, 7, 4]),
    ] {
        let mut dest = [0xff; 7];
        let len = op.encode(&mut dest).expect("fits");
        assert_eq!(&dest[..len], bytes);
        assert_eq!(Op::decode(&dest), Ok((op, &dest[len..])));
        assert!(dest[len..].iter().all(|&b| b == 0xff));
        for end in 0..len {
            assert_eq!(Op::decode(&bytes[..end]), Err(DecodeError::Truncated));
            let mut short = [0xfc; 7];
            assert_eq!(
                op.encode(&mut short[..end]),
                Err(EncodeError::BufferTooSmall)
            );
            assert_eq!(short, [0xfc; 7]);
        }
    }
}

#[test]
fn call_operands_use_the_full_one_byte_input_count() {
    let inputs = [31; 256];
    let call = Op::Call {
        object: 255,
        inputs: &inputs[..255],
        outputs: 255,
    };
    let mut bytes = [0; 259];
    assert_eq!(call.encode(&mut bytes), Ok(259));
    assert_eq!(Op::decode(&bytes), Ok((call, &[][..])));
    let too_many = Op::Call {
        object: 0,
        inputs: &inputs,
        outputs: 0,
    };
    assert_eq!(too_many.encode(&mut bytes), Err(EncodeError::TooManyInputs));
    assert_eq!(Op::decode(&bytes), Ok((call, &[][..])));
}
