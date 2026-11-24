//! Play a short repeating WaveGraph monosynth phrase. Ctrl-C exits.
#![forbid(unsafe_code)]

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("The CoreAudio example runs on macOS");
}

#[cfg(target_os = "macos")]
fn main() -> std::io::Result<()> {
    use core::{num::NonZeroU32, time::Duration};
    use std::{env, io, thread, time::Instant};
    use wavegraph::{Event, bytecode::Program, dsp::Software};
    use wavegraph_audio::{
        DeviceHost, Direction, FRAMES, Playback, Submission,
        coreaudio::{Channels, Host, Output, Route},
    };
    use wavegraph_monosynth::{Message, Monosynth, Tuning};
    use wavegraph_sample::I24;

    const RATE: NonZeroU32 = NonZeroU32::new(48_000).expect("nonzero clock");
    const TUNING: Tuning = Tuning::new(RATE.get(), 1).expect("valid clock");
    let host = Host::new()?;
    let mut devices = host.devices()?;
    for d in devices.endpoints() {
        println!(
            "{} {:?}  {}  ({} ch, {} Hz)",
            d.id, d.direction, d.name, d.channels, d.rate.0
        );
    }
    let mut args = env::args().skip(1);
    let route = match args.next().as_deref() {
        Some("--list") => return Ok(()),
        Some("--output") => {
            let name = args
                .next()
                .ok_or_else(|| io::Error::other("--output requires an exact device name"))?;
            let raw = args
                .next()
                .unwrap_or_else(|| "1".into())
                .parse::<u16>()
                .map_err(io::Error::other)?;
            let index = raw
                .checked_sub(1)
                .ok_or_else(|| io::Error::other("first channel is 1"))?;
            let mut matches = devices
                .endpoints()
                .iter()
                .filter(|d| d.name == name && d.direction == Direction::Output);
            let d = matches
                .next()
                .ok_or_else(|| io::Error::other("output is unavailable"))?;
            if matches.next().is_some() {
                return Err(io::Error::other("ambiguous name; use the Id-based API"));
            }
            let channels = if d.channels == 1 && index == 0 {
                Channels::Mono(0)
            } else if index.checked_add(1).is_some_and(|r| r < d.channels) {
                Channels::Pair(index)
            } else {
                return Err(io::Error::other("no such channel pair"));
            };
            Route::Device(d.id, channels)
        }
        None => Route::Default,
        Some(_) => {
            return Err(io::Error::other(
                "usage: play [--list | --output NAME [FIRST_CHANNEL]]",
            ));
        }
    };
    let sdl = sdl3::init().map_err(io::Error::other)?;
    let mut events = sdl.event_pump().map_err(io::Error::other)?;
    let mut voice = Monosynth::new(&TUNING);
    let program = Program::<2>::decode(&[1, 1, 0, 0, 1, 2, 0, 96])
        .map_err(|e| io::Error::other(format!("{e:?}")))?;
    let mut nodes = [&mut voice];
    let mut patch = program
        .bind_nodes(&mut nodes)
        .map_err(|e| io::Error::other(format!("{e:?}")))?;
    let mut buses = [[I24::default(); FRAMES]; 2];
    let mut output = Output::new(RATE);
    let mut step = 0usize;
    let mut blocks = 0u8;
    let mut last_scan = None;
    let mut previous = None;
    loop {
        if last_scan.is_none_or(|at: Instant| at.elapsed() >= Duration::from_millis(250)) {
            for event in events.poll_iter() {
                if matches!(event, sdl3::event::Event::Quit { .. }) {
                    return Ok(());
                }
            }
            match host.devices() {
                Ok(next) => {
                    if next != devices {
                        eprintln!("Audio devices changed: {next:?}");
                        devices = next;
                    }
                    match output.sync(&host, &devices, route) {
                        Ok(status) if previous != Some(status) => {
                            eprintln!("Output {status:?}");
                            previous = Some(status);
                        }
                        Ok(_) => (),
                        Err(e) => eprintln!("Open failed: {e}"),
                    }
                }
                Err(e) => eprintln!("Scan failed: {e}"),
            }
            last_scan = Some(Instant::now());
        }
        let result = output.render(|out, ()| {
            let note = [57, 64, 69, 67][step];
            let event = Event {
                frame: 0,
                port: 0,
                message: if blocks == 0 {
                    Message::NoteOn {
                        note,
                        velocity: 100,
                    }
                } else {
                    Message::NoteOff(note)
                },
            };
            let events = if blocks == 0 || blocks == 70 {
                core::slice::from_ref(&event)
            } else {
                &[]
            };
            patch
                .render(&mut Software, &mut buses, &[events], |_, e| Err(e))
                .expect("fixed graph and event");
            for (frame, sample) in out.iter_mut().zip(buses[1]) {
                *frame = [sample.inner(); 2];
            }
            blocks = blocks.wrapping_add(1);
            if blocks == 94 {
                blocks = 0;
                step = (step.wrapping_add(1)) & 3;
            }
        });
        match result {
            Ok(Submission::Written) => continue,
            Err(e) => eprintln!("Playback failed: {e}"),
            _ => (),
        }
        thread::sleep(Duration::from_millis(1));
    }
}
