//! Play a pattern with the bundled kit, three local WAVs, or an AudioRealism pack.

use std::{
    env,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};
use wavegraph::{Event, Process, bytecode::Program, dsp::Software};
use wavegraph_909::{DEMO_PATTERN, Samples, Tr909, Voice};
use wavegraph_sample::{I24, Sample};

fn load(path: &Path) -> io::Result<Vec<u8>> {
    let audio = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-ac", "1", "-ar", "44100", "-f", "s16le", "-"])
        .output()?;
    if audio.status.success() {
        Ok(audio.stdout)
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&audio.stderr).into_owned(),
        ))
    }
}

fn main() -> io::Result<()> {
    let mut paths: Vec<_> = env::args_os().skip(1).map(PathBuf::from).collect();
    if let [dir] = paths.as_slice() {
        paths = [
            "ClosedHat/ClosedHat909-decay050.wav",
            "OpenHat/OpendHat909-decay050.wav",
            "Ride/Ride909-tune050.wav",
        ]
        .map(|name| dir.join(name))
        .into();
    }
    let files;
    let samples = match paths.as_slice() {
        [] => Samples::X0XPROC1,
        [ch, oh, rd] => {
            files = [load(ch)?, load(oh)?, load(rd)?];
            let [closed_hat, open_hat, ride] = files.each_ref().map(Vec::as_slice);
            Samples {
                closed_hat,
                open_hat,
                ride,
            }
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "usage: cymbals [AUDIOREALISM_DIR | CLOSED.wav OPEN.wav RIDE.wav]",
            ));
        }
    };
    let mut kit = Tr909::new(samples)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "incomplete PCM16 frame"))?;
    let graph_error = |error| io::Error::other(format!("kit patch: {error:?}"));
    let program = Program::<1, 7>::decode(&[1, 1, 0, 0, 7]).map_err(graph_error)?;
    let mut nodes: [&mut dyn Process<Voice, I24>; 1] = [&mut kit];
    let mut patch = program.bind(&mut nodes).map_err(graph_error)?;
    // One sixteenth note at 126 BPM and 44.1 kHz.
    let mut buses = [[I24::default(); 5250]; 7];
    let mut out = io::BufWriter::new(io::stdout().lock());
    for step in DEMO_PATTERN.into_iter().chain([&[][..]; 16]) {
        let events: Vec<_> = step
            .iter()
            .map(|&message| Event {
                frame: 0,
                port: 0,
                message,
            })
            .collect();
        patch
            .render(&mut Software, &mut buses, &[&events], |_, e| Err(e))
            .map_err(graph_error)?;
        for sample in &buses[0] {
            out.write_all(&sample.to_sample::<i16>().to_le_bytes())?;
        }
    }
    out.flush()
}
