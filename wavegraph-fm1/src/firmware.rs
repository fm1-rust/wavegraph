//! RTICX tasks for the FM-1 panel, audio DMA, scope capture and display.
#![allow(
    missing_docs,
    reason = "RTICX generates public task traits and initializers."
)]

use crate::{
    bass::Controls,
    capture::Trace,
    envelope::EnvelopeControls,
    instrument::{Part, Parts, Voice},
};
use fm1_audio::MasterLevel;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Settings {
    parts: Parts,
    focus: Part,
    env_mode: bool,
    master: MasterLevel,
    voice_tap: bool,
}

impl Settings {
    const INITIAL: Self = Self {
        parts: Parts::INITIAL,
        focus: Part::Mono,
        env_mode: false,
        master: MasterLevel::MIN,
        voice_tap: false,
    };

    fn labels(self, half_gain: bool) -> Labels {
        Labels {
            voice: self.parts.voices[self.focus as usize],
            focus: self.focus,
            bass: self.parts.bass,
            env_mode: self.env_mode,
            mono_envelope: self.parts.mono_envelope,
            half_gain,
            voice_tap: self.voice_tap,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Labels {
    pub(super) voice: Voice,
    pub(super) focus: Part,
    pub(super) bass: Controls,
    pub(super) env_mode: bool,
    pub(super) mono_envelope: EnvelopeControls,
    pub(super) half_gain: bool,
    pub(super) voice_tap: bool,
}

#[derive(Clone, Copy)]
pub struct Frame {
    pub(super) trace: Trace,
    pub(super) labels: Labels,
}

#[fm1_rticx::app(dispatchers = [SOFT0, SOFT1])]
mod app {
    use super::{Frame, Labels, Settings};
    use crate::{
        capture::{Capture, FLAT},
        instrument::{BUS_COUNT, Gate, Instrument, Keyboard, Message, Node, Note, Part, Programs},
        screen::Screen,
    };
    use fm1_audio::{FRAME_RATE, I24, OUTPUT_FRAMES, Output, Refill, Stats};
    use fm1_controls::{Button, Encoder, Leds, PianoKey, Scan, TouchControls};
    use fm1_rticx::{
        bsp::{
            Board,
            display::{DisplayPort, SpiClock, on_spi1_interrupt},
            enter_download_mode,
            time::{Timers, cycles, ticks},
        },
        rticx::asynchronous::{
            channel::{Receiver, Sender},
            make_channel,
        },
        time::{Clock, ExtU32, Timebase, Timer5},
    };
    use static_cell::{ConstStaticCell, StaticCell};
    use wavegraph::{
        Process,
        control::{Controller, EditQueue, Host},
    };
    use wavegraph_303::Tb303;
    use wavegraph_monosynth::{Monosynth, Tuning};

    const SCANS: usize = 8;
    type PanelFrame = (Scan, Part, [Gate; 2]);
    static TUNING: Tuning = Tuning::new(FRAME_RATE.0, FRAME_RATE.1).expect("FM-1 sample clock");

    pub struct ScopeBlock {
        samples: [i32; OUTPUT_FRAMES],
        labels: Labels,
        gap: bool,
    }

    #[shared]
    struct Shared {
        settings: Settings,
        leds: Leds,
        frame: Frame,
        late: bool,
    }

    #[init]
    fn init() -> (Shared, TaskInits) {
        let board = Board::take().expect("CPU0 board");
        let timer = Clock::start(board.timers.with_panel_scan_rate());
        let clock = timer.clock();
        static PLAYERS: StaticCell<[Node; 2]> = StaticCell::new();
        static PROGRAMS: StaticCell<Programs> = StaticCell::new();
        type Nodes = [&'static mut dyn Process<Message, I24>; 2];
        static NODES: StaticCell<Nodes> = StaticCell::new();
        let programs = PROGRAMS.init_with(|| Programs::decode().expect("built-in programs"));
        let [mono, bass] = PLAYERS.init_with(|| {
            [
                Node::Mono(Monosynth::new(&TUNING)),
                Node::Bass(Tb303::new(FRAME_RATE.0 as f32 / FRAME_RATE.1 as f32)),
            ]
        });
        let nodes = NODES.init_with(|| [mono, bass]);
        let instrument = programs.bind(nodes).expect("instrument ports");

        static EDITS: ConstStaticCell<EditQueue<bool>> = ConstStaticCell::new(EditQueue::new());
        let (editor, host) = EDITS.take().split();
        let (scans, input) = make_channel!(PanelFrame, SCANS);
        let (tap, blocks) = make_channel!(ScopeBlock, 2);
        let leds = Leds::default();
        (
            Shared {
                settings: Settings::INITIAL,
                leds,
                frame: Frame {
                    trace: FLAT,
                    labels: Settings::INITIAL.labels(false),
                },
                late: false,
            },
            TaskInits {
                audio: Audio {
                    output: Output::start(board.audio).expect("audio clock"),
                    instrument,
                    host,
                    buses: [[I24::default(); OUTPUT_FRAMES]; BUS_COUNT],
                    first_frame: 0,
                    tap,
                    gap: false,
                    stats: Stats::default(),
                },
                panel: Panel {
                    timer,
                    controls: board.panel.into_touch_controls(),
                    keyboard: Keyboard::default(),
                    scans,
                    leds,
                    octave_since: None,
                },
                input: Input {
                    input,
                    editor,
                    settings: Settings::INITIAL,
                    half_gain: false,
                },
                observe: Observe {
                    blocks,
                    capture: Capture::new(),
                    previous: None,
                },
                draw: Draw {
                    port: Some(board.display),
                    clock,
                },
            },
        )
    }

    #[post_init]
    fn start() {
        Input::spawn(()).expect("one input task");
        Observe::spawn(()).expect("one scope task");
        Draw::spawn(()).expect("one display task");
    }

    #[task(binds = TIMER5, priority = 3, shared = [leds])]
    struct Panel {
        timer: Timer5,
        controls: TouchControls,
        keyboard: Keyboard<{ PianoKey::COUNT as usize }>,
        scans: Sender<'static, PanelFrame, SCANS>,
        leds: Leds,
        octave_since: Option<u32>,
    }

    impl RticTask for Panel {
        fn exec(&mut self) {
            self.timer.on_interrupt();
            let Some((scan, touch)) = self.controls.scan(&self.leds) else {
                return;
            };
            self.leds = self.shared().leds.lock(|leds| *leds);
            // The recovery chord stays available even if an application task stalls.
            if scan.is_down(Button::OctaveDown) && scan.is_down(Button::OctaveUp) {
                let now = ticks();
                if now.wrapping_sub(*self.octave_since.get_or_insert(now)) >= 2 * Timers::TICK_HZ {
                    enter_download_mode();
                }
            } else {
                self.octave_since = None;
            }
            self.keyboard
                .focus
                .turn_preset(scan.encoder_delta(Encoder::Presets));
            // Release first so a key swap in the same sweep can start a new phrase.
            for key in PianoKey::all().filter(|&key| scan.released(key)) {
                self.keyboard.release(53 + key.index());
            }
            for key in PianoKey::all().filter(|&key| scan.pressed(key)) {
                if let Some(velocity) = touch.velocity(key) {
                    self.keyboard.press(Note {
                        number: 53 + key.index(),
                        velocity,
                    });
                }
            }
            // Absolute ownership and both gates survive a dropped scan, including note-offs.
            let _ = self
                .scans
                .try_send((scan, self.keyboard.focus, self.keyboard.gates()));
        }
    }

    #[async_task(priority = 2, shared = [settings, leds])]
    struct Input {
        input: Receiver<'static, PanelFrame, SCANS>,
        editor: Controller<'static, bool>,
        settings: Settings,
        half_gain: bool,
    }

    impl RticAsyncTask for Input {
        type SpawnInput = ();
        async fn exec(&mut self, (): ()) {
            while let Ok((scan, focus, gates)) = self.input.recv().await {
                let before = self.settings;
                self.settings.focus = focus;
                if before.focus != focus {
                    self.settings.env_mode = false;
                }
                if scan.pressed(Button::Env) {
                    self.settings.env_mode = !self.settings.env_mode;
                }
                for (voice, gate) in self.settings.parts.voices.iter_mut().zip(gates) {
                    voice.gate = gate;
                }
                let voice = &mut self.settings.parts.voices[focus as usize];
                voice.turn_waveform(focus, scan.encoder_delta(Encoder::Algorithm));
                let knobs = [
                    Encoder::Knob1,
                    Encoder::Knob2,
                    Encoder::Knob3,
                    Encoder::Knob4,
                ]
                .map(|encoder| scan.encoder_delta(encoder));
                if self.settings.env_mode && focus == Part::Mono {
                    self.settings.parts.mono_envelope.turn(knobs);
                } else {
                    self.settings
                        .parts
                        .turn_select(focus, scan.encoder_delta(Encoder::Select));
                    self.settings.parts.turn_knobs(focus, knobs);
                }
                if scan.pressed(Button::Sel) {
                    self.settings.voice_tap = !self.settings.voice_tap;
                }
                if let Some(master) = scan.master() {
                    self.settings.master = master;
                }
                if before != self.settings {
                    let settings = self.settings;
                    self.shared().settings.lock(|current| *current = settings);
                }
                let route_changed = scan.pressed(Button::Fx);
                if route_changed && self.editor.commit(!self.half_gain).await.is_ok() {
                    self.half_gain = !self.half_gain;
                }
                if route_changed
                    || before.voice_tap != self.settings.voice_tap
                    || before.env_mode != self.settings.env_mode
                {
                    let env = self.settings.env_mode;
                    let tap = self.settings.voice_tap;
                    let half = self.half_gain;
                    self.shared().leds.lock(|leds| {
                        leds.set(Button::Env, env);
                        leds.set(Button::Fx, half);
                        leds.set(Button::Sel, tap);
                    });
                }
            }
        }
    }

    #[task(binds = ALNK0, priority = 4, shared = [settings, late])]
    struct Audio {
        output: Output,
        instrument: Instrument<'static>,
        buses: [[I24; OUTPUT_FRAMES]; BUS_COUNT],
        host: Host<'static, bool>,
        first_frame: u64,
        tap: Sender<'static, ScopeBlock, 2>,
        gap: bool,
        stats: Stats,
    }

    impl RticTask for Audio {
        fn exec(&mut self) {
            let started = cycles();
            let settings = self.shared().settings.lock(|s| *s);
            let mut revision = None;
            let result = self.output.render(settings.master, |out| {
                revision = Some(
                    self.host
                        .begin_block(|half| self.instrument.set_half_gain(half)),
                );
                let mono = self
                    .instrument
                    .render(settings.parts, &mut self.buses)
                    .expect("fixed patch");
                fm1_audio::mix_stereo(out, mono, mono);
            });
            let Some(revision) = revision else { return }; // No DMA half was ready.
            if result == Refill::Written {
                self.host.submitted(revision, self.first_frame);
            }
            self.first_frame = self.first_frame.wrapping_add(OUTPUT_FRAMES as u64);
            if result == Refill::Written && !self.tap.is_full() {
                let block = ScopeBlock {
                    samples: if settings.voice_tap {
                        self.buses[settings.focus.tap()].map(I24::inner)
                    } else {
                        core::array::from_fn(|i| self.output.block()[i][0])
                    },
                    labels: settings.labels(self.instrument.half_gain),
                    gap: self.gap,
                };
                self.gap = self.tap.try_send(block).is_err();
            } else {
                self.gap = true;
            }
            if self.stats.record(started, result) && self.stats.overruns == 1 {
                self.shared().late.lock(|late| *late = true);
            }
        }
    }

    #[async_task(priority = 2, shared = [frame])]
    struct Observe {
        blocks: Receiver<'static, ScopeBlock, 2>,
        capture: Capture,
        previous: Option<Labels>,
    }
    impl RticAsyncTask for Observe {
        type SpawnInput = ();
        async fn exec(&mut self, (): ()) {
            while let Ok(block) = self.blocks.recv().await {
                let changed = self.previous.replace(block.labels) != Some(block.labels);
                let silent = block.samples.iter().all(|&s| s == 0);
                let trace = self
                    .capture
                    .push(&block.samples, silent, changed || block.gap);
                if changed || trace.is_some() {
                    self.shared().frame.lock(|frame| {
                        // Publish the new page even while waiting for its first trigger.
                        if let Some(trace) = trace {
                            frame.trace = trace;
                        } else if frame.labels.focus != block.labels.focus
                            || frame.labels.voice_tap != block.labels.voice_tap
                        {
                            frame.trace = FLAT;
                        }
                        frame.labels = block.labels;
                    });
                }
            }
        }
    }

    #[task(binds = SPI1, priority = 1, init = generated)]
    struct SpiDone;
    impl RticTask for SpiDone {
        fn exec(&mut self) {
            on_spi1_interrupt();
        }
    }

    #[async_task(priority = 1, shared = [frame, late])]
    struct Draw {
        port: Option<DisplayPort>,
        clock: Clock,
    }
    impl RticAsyncTask for Draw {
        type SpawnInput = ();
        async fn exec(&mut self, (): ()) {
            let port = self.port.take().expect("one LCD");
            let Ok(mut lcd) = port.init_with_clock(SpiClock::Div1) else {
                return;
            };
            let mut screen = Screen::new();
            if screen.splash(&mut lcd, &mut self.clock).await.is_err() {
                return;
            }
            // A failed display transfer ends only this task. Audio and panel IRQs keep running.
            loop {
                let frame = self.shared().frame.lock(|f| *f);
                let late = self.shared().late.lock(|late| *late);
                if screen
                    .paint(&mut lcd, &mut self.clock, &frame, late)
                    .await
                    .is_err()
                {
                    return;
                }
                Clock::delay(30.millis()).await;
            }
        }
    }
}
