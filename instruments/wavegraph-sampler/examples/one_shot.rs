//! Render a PCM clip through a WaveGraph patch and write mono s16le to stdout.

use std::{
    env, fs,
    io::{self, Write},
};
use wavegraph_dsp::{Event, Process, bytecode::Program, dsp::Software};
use wavegraph_sample::{I24, Sample};
use wavegraph_sampler::{Message, Sampler};

const BLOCK_FRAMES: usize = 128;

fn main() -> io::Result<()> {
    let path = env::args_os()
        .nth(1)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "pass a mono s16le file"))?;
    let pcm = fs::read(path)?;
    let mut voice = Sampler::from_pcm16le(&pcm)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "incomplete PCM16 frame"))?;
    let graph_error = |error| io::Error::other(format!("one-shot patch: {error:?}"));
    // Format version 1, call object 0 with no signal inputs and one output.
    let program = Program::<1, 1>::decode(&[1, 1, 0, 0, 1]).map_err(graph_error)?;
    let mut objects: [&mut dyn Process<Message, I24>; 1] = [&mut voice];
    let mut patch = program.bind(&mut objects).map_err(graph_error)?;
    let mut bus = [[I24::default(); BLOCK_FRAMES]];
    let trigger = [Event {
        frame: 0,
        port: 0,
        message: Message::Trigger,
    }];
    let mut events = trigger.as_slice();
    let mut out = io::BufWriter::new(io::stdout().lock());
    for chunk in pcm.chunks(BLOCK_FRAMES * 2) {
        patch
            .render(&mut Software, &mut bus, &[events], |_, event| Err(event))
            .map_err(graph_error)?;
        for sample in &bus[0][..chunk.len() / 2] {
            out.write_all(&sample.to_sample::<i16>().to_le_bytes())?;
        }
        events = &[];
    }
    out.flush()
}
