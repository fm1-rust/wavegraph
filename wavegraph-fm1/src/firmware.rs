//! RTICX tasks for the FM-1 panel, audio DMA, scope capture and display.
#![allow(
    missing_docs,
    reason = "RTICX generates public task traits and initializers."
)]

use crate::{
    bass::Controls,
    capture::Trace,
    envelope::EnvelopeControls,
    fx::{Fx, FxPage},
    instrument::{Mixer, MixerBank, Part, Parts, Voice},
    organ::{Controls as OrganControls, OrganKey},
    sequencer::Sequencer,
};
use fm1_audio::MasterLevel;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Settings {
    parts: Parts,
    focus: Part,
    env_mode: bool,
    fx_view: bool,
    fx_page: FxPage,
    mixer_view: bool,
    mixer_bank: MixerBank,
    mixer: Mixer,
    seq: Sequencer,
    fx: Fx,
    master: MasterLevel,
    voice_tap: bool,
    menu_since: Option<u32>,
}

impl Settings {
    const INITIAL: Self = Self {
        parts: Parts::INITIAL,
        focus: Part::Mono,
        env_mode: false,
        fx_view: false,
        fx_page: FxPage::Reverb,
        mixer_view: false,
        mixer_bank: MixerBank::Volume,
        mixer: Mixer::INITIAL,
        seq: Sequencer::INITIAL,
        fx: Fx::INITIAL,
        master: MasterLevel::MIN,
        voice_tap: false,
        menu_since: None,
    };

    fn labels(self, half_gain: bool) -> Labels {
        Labels {
            voice: self.parts.voices[self.focus as usize],
            focus: self.focus,
            bass: self.parts.bass,
            organ: self.parts.organ,
            organ_keys: self.parts.organ_keys,
            menu_since: self.menu_since,
            env_mode: self.env_mode,
            fx_view: self.fx_view,
            fx_page: self.fx_page,
            mixer_view: self.mixer_view,
            mixer_bank: self.mixer_bank,
            mixer: self.mixer,
            seq: self.seq,
            fx: self.fx,
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
    pub(super) organ: OrganControls,
    pub(super) organ_keys: [OrganKey; wavegraph_organ::POLYPHONY],
    pub(super) menu_since: Option<u32>,
    pub(super) env_mode: bool,
    pub(super) fx_view: bool,
    pub(super) fx_page: FxPage,
    pub(super) mixer_view: bool,
    pub(super) mixer_bank: MixerBank,
    pub(super) mixer: Mixer,
    pub(super) seq: Sequencer,
    pub(super) fx: Fx,
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
        instrument::{
            BUS_COUNT, FxMixer, FxReturn, Gate, Instrument, Keyboard, Message, Node, Note, Part,
            Programs,
        },
        overload::Overloads,
        screen::Screen,
        sequencer::{DrumMachine, Sequencer},
    };
    use fm1_audio::{FRAME_RATE, I24, OUTPUT_FRAMES, Output, Refill, Stats};
    use fm1_controls::{Button, Encoder, Leds, PianoKey, PlayLedState, Scan, TouchControls};
    use fm1_rticx::{
        bsp::{
            Board,
            display::{DisplayPort, SpiClock, on_spi1_interrupt},
            enter_download_mode,
            time::{Timers, cycles, ticks},
            usb_midi::{Identity, MidiDevice, PacketBuffer, UsbLog, on_usb0_interrupt},
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
    use wavegraph_fx::{ChannelMixer, Delay, REVERB_SAMPLES, Reverb};
    use wavegraph_monosynth::{Monosynth, Tuning};
    use wavegraph_organ::Organ;

    const SCANS: usize = 8;
    type PanelFrame = (
        Scan,
        Sequencer,
        Part,
        [Gate; 3],
        [crate::organ::OrganKey; wavegraph_organ::POLYPHONY],
    );
    static TUNING: Tuning = Tuning::new(FRAME_RATE.0, FRAME_RATE.1).expect("FM-1 sample clock");
    const RATE_HZ: u32 = (FRAME_RATE.0 + FRAME_RATE.1 / 2) / FRAME_RATE.1;

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
        overloads: Overloads,
        beat_phase: Option<u8>,
    }

    #[init]
    fn init() -> (Shared, TaskInits) {
        let board = Board::take().expect("CPU0 board");
        let usb = board
            .take_usb()
            .expect("one USB device")
            .into_midi(Identity {
                vendor_id: 0x1209,
                product_id: 0x0001,
                manufacturer: "fm1-rust",
                product: "WaveGraph FM-1",
            });
        let timer = Clock::start(board.timers.with_panel_scan_rate());
        let clock = timer.clock();
        // Keep the twelve DMA-sized buses out of the task initializer and its stack.
        static BUSES: ConstStaticCell<[[I24; OUTPUT_FRAMES]; BUS_COUNT]> = ConstStaticCell::new(
            [[wavegraph_sample::types::i24::EQUILIBRIUM; OUTPUT_FRAMES]; BUS_COUNT],
        );
        static PLAYERS: StaticCell<[Node; 3]> = StaticCell::new();
        static ORGAN: ConstStaticCell<Organ> =
            ConstStaticCell::new(Organ::new(FRAME_RATE.0, FRAME_RATE.1).expect("organ clock"));
        static PROGRAMS: StaticCell<Programs> = StaticCell::new();
        static REVERB_STORAGE: ConstStaticCell<[i32; REVERB_SAMPLES]> =
            ConstStaticCell::new([0; REVERB_SAMPLES]);
        static DELAY_STORAGE: ConstStaticCell<[i16; RATE_HZ as usize]> =
            ConstStaticCell::new([0; RATE_HZ as usize]);
        type Effects = (FxMixer, FxReturn<Reverb<'static>>, FxReturn<Delay<'static>>);
        static EFFECTS: StaticCell<Effects> = StaticCell::new();
        static DRUMS: StaticCell<DrumMachine> = StaticCell::new();
        type Nodes = [&'static mut dyn Process<Message, I24>; 7];
        static NODES: StaticCell<Nodes> = StaticCell::new();
        let programs = PROGRAMS.init_with(|| Programs::decode().expect("built-in programs"));
        let [mono, organ, bass] = PLAYERS.init_with(|| {
            [
                Node::Mono(Monosynth::new(&TUNING).with_idle_suspension()),
                Node::Organ(ORGAN.take()),
                Node::Bass(
                    Tb303::new(FRAME_RATE.0 as f32 / FRAME_RATE.1 as f32).with_idle_suspension(),
                ),
            ]
        });
        let (mixer, reverb, delay) = EFFECTS.init_with(|| {
            (
                FxMixer(ChannelMixer::new()),
                FxReturn(Reverb::new(REVERB_STORAGE.take())),
                FxReturn(Delay::new(DELAY_STORAGE.take(), RATE_HZ).expect("one-second delay line")),
            )
        });
        let drums = DRUMS.init_with(|| DrumMachine::new(FRAME_RATE));
        let nodes = NODES.init_with(|| [mono, organ, bass, mixer, reverb, drums, delay]);
        let instrument = programs.bind(nodes).expect("instrument ports");

        static EDITS: ConstStaticCell<EditQueue<bool>> = ConstStaticCell::new(EditQueue::new());
        let (editor, host) = EDITS.take().split();
        let (scans, input) = make_channel!(PanelFrame, SCANS);
        let (tap, blocks) = make_channel!(ScopeBlock, 2);
        let mut leds = Leds::default();
        leds.set_play(PlayLedState::Red);
        (
            Shared {
                settings: Settings::INITIAL,
                leds,
                frame: Frame {
                    trace: FLAT,
                    labels: Settings::INITIAL.labels(false),
                },
                overloads: Overloads::default(),
                beat_phase: None,
            },
            TaskInits {
                audio: Audio {
                    output: Output::start(board.audio).expect("audio clock"),
                    instrument,
                    host,
                    buses: BUSES.take(),
                    first_frame: 0,
                    tap,
                    gap: false,
                    stats: Stats::default(),
                    last_start: None,
                },
                panel: Panel {
                    timer,
                    controls: board.panel.into_touch_controls(),
                    keyboard: Keyboard::default(),
                    seq: Sequencer::INITIAL,
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
                usb: Usb { device: usb, clock },
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
        Usb::spawn(()).expect("one USB task");
    }

    // The upper seven keys share an LED line with the velocity probe. End its
    // 100 µs column slot on time even when the audio graph is rendering.
    #[task(binds = TIMER5, priority = 4, shared = [leds])]
    struct Panel {
        timer: Timer5,
        controls: TouchControls,
        keyboard: Keyboard<{ PianoKey::COUNT as usize }>,
        seq: Sequencer,
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
            // Start with the UI button lamps each sweep, then add the physical key lamps.
            self.leds = self.shared().leds.lock(|leds| *leds);
            if scan.pressed(Button::Play) {
                self.seq.transport.toggle();
            }
            self.leds.set_play(if self.seq.transport.playing() {
                PlayLedState::Green
            } else {
                PlayLedState::Red
            });
            if scan.pressed(Button::Seq) {
                self.seq.toggle_page();
            } else if scan.pressed(Button::Home)
                || scan.pressed(Button::Env)
                || scan.pressed(Button::Fx)
                || scan.pressed(Button::Glo)
                || scan.encoder_delta(Encoder::Presets) != 0
            {
                self.seq.page = None;
            }
            self.seq.turn(
                scan.encoder_delta(Encoder::Select),
                scan.encoder_delta(Encoder::Knob3),
            );
            self.leds.set(Button::Seq, self.seq.page.is_some());
            let now = cycles();
            // Time the recovery chord against the crystal even if scan interrupts run late.
            if scan.is_down(Button::OctaveDown) && scan.is_down(Button::OctaveUp) {
                self.keyboard.octave = 0;
                if now.wrapping_sub(*self.octave_since.get_or_insert(now)) >= 2 * Timers::CRYSTAL_HZ
                {
                    enter_download_mode();
                }
            } else {
                self.octave_since = None;
                if scan.pressed(Button::OctaveDown) {
                    self.keyboard.shift_octave(-1);
                } else if scan.pressed(Button::OctaveUp) {
                    self.keyboard.shift_octave(1);
                }
            }
            let octave = self.keyboard.octave;
            // At 24 MHz these bits toggle every 350 ms and 175 ms respectively.
            let lit = match octave.unsigned_abs() {
                1 => now & (1 << 23) == 0,
                2 => now & (1 << 22) == 0,
                _ => true,
            };
            self.leds.set(Button::OctaveDown, octave < 0 && lit);
            self.leds.set(Button::OctaveUp, octave > 0 && lit);
            self.keyboard
                .focus
                .turn_preset(scan.encoder_delta(Encoder::Presets));
            // Release first so a key swap in the same sweep can start a new phrase.
            for key in PianoKey::all().filter(|&key| scan.released(key)) {
                self.keyboard.release(53 + key.index());
            }
            for key in PianoKey::all().filter(|&key| scan.is_down(key)) {
                self.leds.set(key, true);
                if scan.pressed(key)
                    && let Some(velocity) = touch.velocity(key)
                {
                    self.keyboard.press(Note {
                        number: 53 + key.index(),
                        velocity,
                    });
                }
            }
            // A full state snapshot survives dropped scans. The organ includes
            // strike generations so even same-pitch restrikes can be detected.
            let _ = self.scans.try_send((
                scan,
                self.seq,
                self.keyboard.focus,
                self.keyboard.gates(),
                self.keyboard.organ_keys(),
            ));
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
            while let Ok((scan, seq, focus, gates, organ_keys)) = self.input.recv().await {
                let before = self.settings;
                self.settings.focus = focus;
                self.settings.seq = seq;
                if seq.page.is_some() {
                    self.settings.env_mode = false;
                    self.settings.fx_view = false;
                    self.settings.mixer_view = false;
                    self.settings.menu_since = None;
                }
                if before.focus != focus || scan.encoder_delta(Encoder::Presets) != 0 {
                    self.settings.menu_since = Some(ticks());
                    self.settings.env_mode = false;
                }
                if scan.pressed(Button::Env) {
                    self.settings.menu_since = None;
                    self.settings.env_mode = !self.settings.env_mode;
                    if self.settings.env_mode {
                        self.settings.fx_view = false;
                        self.settings.mixer_view = false;
                    }
                }
                if scan.pressed(Button::Fx) {
                    self.settings.fx_view = !self.settings.fx_view;
                    if self.settings.fx_view {
                        self.settings.env_mode = false;
                        self.settings.mixer_view = false;
                    }
                }
                if scan.pressed(Button::Glo) {
                    self.settings.mixer_view = !self.settings.mixer_view;
                    if self.settings.mixer_view {
                        self.settings.menu_since = None;
                        self.settings.env_mode = false;
                        self.settings.fx_view = false;
                    }
                }
                if scan.pressed(Button::Home) {
                    self.settings.env_mode = false;
                    self.settings.fx_view = false;
                    self.settings.mixer_view = false;
                }
                for (voice, gate) in self.settings.parts.voices.iter_mut().zip(gates) {
                    voice.gate = gate;
                }
                self.settings.parts.organ_keys = organ_keys;
                let knobs = [
                    Encoder::Knob1,
                    Encoder::Knob2,
                    Encoder::Knob3,
                    Encoder::Knob4,
                ]
                .map(|encoder| scan.encoder_delta(encoder));
                let route_changed = if seq.page.is_some() {
                    false
                } else if self.settings.mixer_view {
                    self.settings
                        .mixer_bank
                        .turn(scan.encoder_delta(Encoder::Select));
                    self.settings
                        .mixer
                        .turn_knobs(self.settings.mixer_bank, knobs);
                    false
                } else if self.settings.fx_view {
                    self.settings
                        .fx_page
                        .turn(scan.encoder_delta(Encoder::Select));
                    self.settings
                        .fx
                        .edit(self.settings.fx_page, knobs, scan.pressed(Button::Sel));
                    // Retain the asynchronous graph-edit demonstration.
                    scan.encoder_delta(Encoder::Algorithm) != 0
                } else {
                    let voice = &mut self.settings.parts.voices[focus as usize];
                    voice.turn_waveform(focus, scan.encoder_delta(Encoder::Algorithm));
                    match (self.settings.env_mode, focus) {
                        (true, Part::Mono) => self.settings.parts.mono_envelope.turn(knobs),
                        (true, Part::Organ) => self.settings.parts.organ.envelope.turn_organ(knobs),
                        _ => {
                            self.settings
                                .parts
                                .turn_select(focus, scan.encoder_delta(Encoder::Select));
                            self.settings.parts.turn_knobs(focus, knobs);
                        }
                    }
                    if scan.pressed(Button::Sel) {
                        self.settings.voice_tap = !self.settings.voice_tap;
                    }
                    false
                };
                if let Some(master) = scan.master() {
                    self.settings.master = master;
                }
                if before != self.settings {
                    let settings = self.settings;
                    self.shared().settings.lock(|current| *current = settings);
                }
                if route_changed && self.editor.commit(!self.half_gain).await.is_ok() {
                    self.half_gain = !self.half_gain;
                }
                if before.voice_tap != self.settings.voice_tap
                    || before.env_mode != self.settings.env_mode
                    || before.fx_view != self.settings.fx_view
                    || before.mixer_view != self.settings.mixer_view
                    || before.fx.output(before.fx_page).enabled
                        != self.settings.fx.output(self.settings.fx_page).enabled
                {
                    let env = self.settings.env_mode;
                    let fx_view = self.settings.fx_view;
                    let mixer_view = self.settings.mixer_view;
                    let sel = if fx_view {
                        self.settings.fx.output(self.settings.fx_page).enabled
                    } else if mixer_view {
                        false
                    } else {
                        self.settings.voice_tap
                    };
                    self.shared().leds.lock(|leds| {
                        leds.set(Button::Env, env);
                        leds.set(Button::Fx, fx_view);
                        leds.set(Button::Glo, mixer_view);
                        leds.set(Button::Sel, sel);
                    });
                }
            }
        }
    }

    #[task(binds = ALNK0, priority = 3, shared = [settings, overloads, beat_phase])]
    struct Audio {
        output: Output,
        instrument: Instrument<'static>,
        buses: &'static mut [[I24; OUTPUT_FRAMES]; BUS_COUNT],
        host: Host<'static, bool>,
        first_frame: u64,
        tap: Sender<'static, ScopeBlock, 2>,
        gap: bool,
        stats: Stats,
        last_start: Option<u32>,
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
                self.instrument.set_mixer(settings.mixer);
                self.instrument.set_fx(settings.fx);
                self.instrument.transport = settings.seq.transport;
                let mono = self
                    .instrument
                    .render(settings.parts, self.buses)
                    .expect("fixed patch");
                fm1_audio::mix_stereo(out, mono, mono);
            });
            let Some(revision) = revision else { return }; // No DMA half was ready.
            if result == Refill::Written {
                self.host.submitted(revision, self.first_frame);
                // Keep the visual clock current even when scope capture drops a block.
                let phase = self.instrument.beat_phase;
                self.shared().beat_phase.lock(|current| *current = phase);
            }
            self.first_frame = self.first_frame.wrapping_add(OUTPUT_FRAMES as u64);
            if result == Refill::Written && !self.tap.is_full() {
                let block = ScopeBlock {
                    samples: if settings.voice_tap && !settings.fx_view && !settings.mixer_view {
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
            let previous = self.last_start.replace(started);
            if self.stats.record(started, result) {
                let work = cycles().wrapping_sub(started);
                let gap = previous.map_or(0, |prev| started.wrapping_sub(prev));
                let count = self.stats.overruns;
                let at = ticks();
                self.shared()
                    .overloads
                    .lock(|log| log.record(count, at, work, gap, result == Refill::Late));
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
                            || frame.labels.fx_view != block.labels.fx_view
                            || frame.labels.mixer_view != block.labels.mixer_view
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

    #[task(binds = USB0, priority = 1, init = generated)]
    struct UsbInterrupt;
    impl RticTask for UsbInterrupt {
        fn exec(&mut self) {
            on_usb0_interrupt();
        }
    }

    #[async_task(priority = 1, shared = [overloads])]
    struct Usb {
        device: MidiDevice,
        clock: Clock,
    }
    impl RticAsyncTask for Usb {
        type SpawnInput = ();
        async fn exec(&mut self, (): ()) {
            let mut rx = PacketBuffer::default();
            let mut log = UsbLog::new();
            log.record("WaveGraph ready").expect("short ASCII line");
            let mut logged = 0;
            loop {
                self.device.wait_for_activity(&mut self.clock).await;
                let packets = self.device.receive(&mut rx);
                if packets.contains(&[0x07, 0xf0, 0x7d, 0xf7]) {
                    let history = self.shared().overloads.lock(|history| *history);
                    for event in history.events() {
                        if event.number > logged {
                            log.record(&event.line()).expect("bounded ASCII log line");
                            logged = event.number;
                        }
                    }
                }
                log.service(&mut self.device, packets);
            }
        }
    }

    #[async_task(priority = 1, shared = [frame, overloads, beat_phase])]
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
                let severity = self.shared().overloads.lock(|log| log.severity(ticks()));
                let phase = self.shared().beat_phase.lock(|phase| *phase);
                if screen
                    .paint(&mut lcd, &mut self.clock, &frame, severity, phase)
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
