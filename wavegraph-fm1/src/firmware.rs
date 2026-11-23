//! RTICX tasks for the FM-1 panel, audio DMA, scope capture and display.
#![allow(
    missing_docs,
    reason = "RTICX generates public task traits and initializers."
)]

use crate::{
    bass::Controls,
    capture::Trace,
    envelope::{EnvelopeControls, EnvelopeTarget},
    fx::{BassFx, Fx},
    instrument::{Part, SawControls, Track, Voice},
    keys::PolyKey,
    mixer::Mixer,
    organ::Controls as OrganControls,
    panel::{CHANNELS, Channel, Navigation, View},
    sequencer::{DrumLamp, Position, Sequencer},
};
use fm1_audio::MasterLevel;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Settings {
    tracks: [Track; CHANNELS],
    nav: Navigation,
    mixer: Mixer,
    seq: Sequencer,
    fx: Fx,
    master: MasterLevel,
}

impl Settings {
    const INITIAL: Self = Self {
        tracks: Track::rack(Navigation::INITIAL.assignments),
        nav: Navigation::INITIAL,
        mixer: Mixer::INITIAL,
        seq: Sequencer::INITIAL,
        fx: Fx::INITIAL,
        master: MasterLevel::MIN,
    };

    fn labels(self, half_gain: bool) -> Labels {
        let channel = self.nav.focus;
        let track = self.tracks[channel.index()];
        Labels {
            channel,
            assignments: self.nav.assignments.map(|assignment| assignment.part),
            voice: track.voice,
            focus: track.assignment.part,
            bass: track.bass,
            organ: track.organ,
            drums808: track.drums808,
            saw: track.saw,
            wave_scan: track.wave.scan(track.voice.waveform % 2),
            poly_keys: track.poly_keys,
            preset_since: self.nav.preset_since,
            view: self.nav.view,
            bass_fx: track.bass_fx,
            bass_insert: self.nav.bass_insert(),
            mixer: self.mixer,
            seq: self.seq,
            fx: self.fx,
            voice_envelope: track.voice_envelope,
            half_gain,
            voice_tap: track.voice_tap,
        }
    }

    fn sel_lamp(self) -> bool {
        match self.nav.view {
            View::Fx => {
                self.nav.bass_insert() && self.tracks[self.nav.focus.index()].bass_fx.enabled
            }
            View::MasterFx(Some(row)) => self.fx.enabled(row),
            View::Envelope if self.nav.part() == Part::Supersaw => {
                self.tracks[self.nav.focus.index()].saw.target == EnvelopeTarget::Filter
            }
            View::Scope | View::Envelope => self.tracks[self.nav.focus.index()].voice_tap,
            View::Sequencer => self.seq.enabled[self.seq.channel.index()],
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Labels {
    pub(super) channel: Channel,
    pub(super) assignments: [Part; CHANNELS],
    pub(super) voice: Voice,
    pub(super) focus: Part,
    pub(super) bass: Controls,
    pub(super) organ: OrganControls,
    pub(super) drums808: wavegraph_808::Controls,
    pub(super) saw: SawControls,
    pub(super) wave_scan: wavegraph_wavetable::Scan,
    pub(super) poly_keys: [PolyKey; wavegraph_organ::POLYPHONY],
    pub(super) preset_since: Option<u32>,
    pub(super) view: View,
    pub(super) bass_fx: BassFx,
    pub(super) bass_insert: bool,
    pub(super) mixer: Mixer,
    pub(super) seq: Sequencer,
    pub(super) fx: Fx,
    pub(super) voice_envelope: EnvelopeControls,
    pub(super) half_gain: bool,
    pub(super) voice_tap: bool,
}

#[derive(Clone, Copy, Default)]
pub struct Motion {
    pub(super) position: Option<Position>,
    pub(super) bass_automation: Option<(Channel, u8, u8)>,
    pub(super) peaks: [u8; CHANNELS],
    pub(super) reduction: u8,
    pub(super) drums: [[DrumLamp; 6]; CHANNELS],
    pub(super) wave_frames: [u8; CHANNELS],
}

#[derive(Clone, Copy)]
pub struct Frame {
    pub(super) trace: Trace,
    pub(super) labels: Labels,
}

#[fm1_rticx::app(dispatchers = [SOFT0, SOFT1])]
mod app {
    use super::{Frame, Labels, Motion, Settings};
    use crate::{
        bumper::Bumper,
        capture::{Capture, FLAT, Scale},
        inserts::BassInserts,
        instrument::{
            BUS_COUNT, DRY_HEADROOM, FxMixer, FxNode, Instrument, Master, Message, Node, Note,
            Part, Programs, RoutedEvents, STRIP_BUS, Track,
        },
        keys::{Keys, MidiAction, PanelPads, Piano, Press, decode_midi},
        mixer::Meter,
        overload::Overloads,
        panel::{Assignment, CHANNELS, Channel, Navigation, PageButton, View, preset_age},
        screen::Screen,
        sequencer::{DrumKits, SeqPage, Sequencer},
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
    use heapless::Vec;
    use static_cell::{ConstStaticCell, StaticCell};
    use wavegraph::{
        Process,
        control::{Controller, EditQueue, Host},
    };
    use wavegraph_fx::{ChannelMixer, Delay, MasterBus, REVERB_SAMPLES, Reverb};
    use wavegraph_monosynth::Tuning;
    use wavegraph_organ::Organ;

    const SCANS: usize = 8;
    const MIDI_EVENTS: usize = 32;
    const PIANO_KEYS: usize = PianoKey::COUNT as usize;

    pub struct PanelFrame {
        scan: Scan,
        seq: Sequencer,
        nav: Navigation,
        pads: [PanelPads; CHANNELS],
        held: Vec<Press, PIANO_KEYS>,
    }

    pub struct MidiEvent {
        epoch: u32,
        generation: u32,
        channel: Channel,
        action: MidiAction,
    }

    static TUNING: Tuning = Tuning::new(FRAME_RATE.0, FRAME_RATE.1).expect("FM-1 sample clock");
    const RATE_HZ: u32 = (FRAME_RATE.0 + FRAME_RATE.1 / 2) / FRAME_RATE.1;

    pub struct ScopeBlock {
        samples: [i32; OUTPUT_FRAMES],
        bass_cycle: Option<usize>,
        labels: Labels,
        gap: bool,
    }

    #[shared]
    struct Shared {
        settings: Settings,
        bumper: Bumper,
        leds: Leds,
        frame: Frame,
        overloads: Overloads,
        motion: Motion,
        midi_epoch: u32,
    }

    // RTICX keeps its main frame while interrupt tasks run. Initialize the graph
    // in a separate frame and hand the audio task its permanent SRAM storage.
    #[inline(never)]
    fn initialize_graph() -> &'static mut Instrument<'static> {
        static ORGAN: Organ = crate::organ::instrument(FRAME_RATE.0, FRAME_RATE.1);
        static DRUMS: StaticCell<DrumKits> = StaticCell::new();
        static PLAYERS: [StaticCell<Node<'static>>; CHANNELS] =
            [const { StaticCell::new() }; CHANNELS];
        // Keep the four channel queues in SRAM throughout RTICX initialization.
        static EVENTS: ConstStaticCell<[RoutedEvents; CHANNELS]> =
            ConstStaticCell::new([const { RoutedEvents::new() }; CHANNELS]);
        static PROGRAMS: StaticCell<Programs> = StaticCell::new();
        static REVERB_STORAGE: ConstStaticCell<[i32; REVERB_SAMPLES]> =
            ConstStaticCell::new([0; REVERB_SAMPLES]);
        static DELAY_STORAGE: ConstStaticCell<[i16; RATE_HZ as usize]> =
            ConstStaticCell::new([0; RATE_HZ as usize]);
        type Effects = (
            FxMixer,
            FxNode<Reverb<'static>>,
            FxNode<Delay<'static>>,
            Master,
        );
        static EFFECTS: StaticCell<Effects> = StaticCell::new();
        static INSERTS: StaticCell<BassInserts> = StaticCell::new();
        type Nodes = [&'static mut dyn Process<Message, I24>; 9];
        static NODES: StaticCell<Nodes> = StaticCell::new();
        let programs = PROGRAMS.init_with(|| Programs::decode().expect("built-in programs"));
        let drums = DRUMS.init_with(|| DrumKits::new(FRAME_RATE));
        let [one, two, three, four] = PLAYERS.each_ref().map(|cell| {
            cell.init_with(|| {
                Node::new(
                    &TUNING,
                    &ORGAN,
                    drums,
                    FRAME_RATE.0 as f32 / FRAME_RATE.1 as f32,
                )
            })
        });
        let (mixer, reverb, delay, master) = EFFECTS.init_with(|| {
            (
                FxMixer(
                    ChannelMixer::new()
                        .with_boost()
                        .with_dry_headroom(DRY_HEADROOM),
                ),
                FxNode(Reverb::new(REVERB_STORAGE.take())),
                FxNode(Delay::new(DELAY_STORAGE.take(), RATE_HZ).expect("one-second delay line")),
                Master(MasterBus::new(RATE_HZ).expect("audio sample clock")),
            )
        });
        let inserts = INSERTS.init_with(|| BassInserts::new(RATE_HZ));
        let nodes =
            NODES.init_with(|| [one, two, three, four, mixer, reverb, delay, inserts, master]);
        static INSTRUMENT: StaticCell<Instrument<'static>> = StaticCell::new();
        INSTRUMENT.init_with(|| {
            programs
                .bind(nodes, EVENTS.take(), FRAME_RATE)
                .expect("instrument ports")
        })
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
        // Keep the DMA-sized buses out of the task initializer and its stack.
        static BUSES: ConstStaticCell<[[I24; OUTPUT_FRAMES]; BUS_COUNT]> = ConstStaticCell::new(
            [[wavegraph_sample::types::i24::EQUILIBRIUM; OUTPUT_FRAMES]; BUS_COUNT],
        );
        let instrument = initialize_graph();

        static EDITS: ConstStaticCell<EditQueue<bool>> = ConstStaticCell::new(EditQueue::new());
        let (editor, host) = EDITS.take().split();
        let (scans, input) = make_channel!(PanelFrame, SCANS);
        let (midi_out, midi_in) = make_channel!(MidiEvent, MIDI_EVENTS);
        static KEYS: ConstStaticCell<Keys<PIANO_KEYS>> = ConstStaticCell::new(Keys::new());
        static PIANO: ConstStaticCell<Piano<PIANO_KEYS>> = ConstStaticCell::new(Piano::new());
        static HELD: ConstStaticCell<Vec<Press, PIANO_KEYS>> = ConstStaticCell::new(Vec::new());
        let (tap, blocks) = make_channel!(ScopeBlock, 2);
        let mut leds = Leds::default();
        leds.set_play(PlayLedState::Red);
        leds.set(Button::Sel, Settings::INITIAL.sel_lamp());
        (
            Shared {
                settings: Settings::INITIAL,
                bumper: Bumper::default(),
                leds,
                frame: Frame {
                    trace: FLAT,
                    labels: Settings::INITIAL.labels(false),
                },
                overloads: Overloads::default(),
                motion: Motion::default(),
                midi_epoch: 0,
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
                    meter: Meter::default(),
                    assignments: Navigation::INITIAL.assignments,
                },
                panel: Panel {
                    timer,
                    controls: board.panel.into_touch_controls(),
                    piano: PIANO.take(),
                    last_melodic: Channel::One,
                    nav: Navigation::INITIAL,
                    seq: Sequencer::INITIAL,
                    scans,
                    leds,
                    octave_since: None,
                },
                input: Input {
                    input,
                    midi: midi_in,
                    midi_epoch: 0,
                    keys: KEYS.take(),
                    held: HELD.take(),
                    editor,
                    settings: Settings::INITIAL,
                    half_gain: false,
                },
                observe: Observe {
                    blocks,
                    capture: Capture::new(),
                    previous: None,
                },
                usb: Usb {
                    device: usb,
                    clock,
                    midi: midi_out,
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
        Usb::spawn(()).expect("one USB task");
    }

    // The upper seven keys share an LED line with the velocity probe. End its
    // 100 µs column slot on time even when the audio graph is rendering.
    #[task(binds = TIMER5, priority = 4, shared = [leds])]
    struct Panel {
        timer: Timer5,
        controls: TouchControls,
        piano: &'static mut Piano<PIANO_KEYS>,
        last_melodic: Channel,
        nav: Navigation,
        seq: Sequencer,
        scans: Sender<'static, PanelFrame, SCANS>,
        leds: Leds,
        octave_since: Option<u32>,
    }

    impl RticTask for Panel {
        // This scanner preempts audio every 100 us. Its instructions share the flash
        // cache with the voices unless startup copies them into executable SRAM.
        // SAFETY: the FM-1 linker and startup place and initialize .data before IRQs run.
        #[unsafe(link_section = ".data.wavegraph_panel")]
        #[inline(never)]
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
            let page = [
                (Button::Home, PageButton::Home),
                (Button::Glo, PageButton::Glo),
                (Button::Fx, PageButton::Fx),
                (Button::Env, PageButton::Env),
                (Button::Seq, PageButton::Seq),
            ]
            .into_iter()
            .find(|(button, _)| scan.pressed(*button));
            let home = scan.pressed(Button::Home);
            let previous_view = self.nav.view;
            let previous_channel = self.nav.focus;
            if !matches!(self.nav.view, View::MasterFx(_))
                && ((scan.pressed(Button::Glo) && scan.is_down(Button::Fx))
                    || (scan.pressed(Button::Fx) && scan.is_down(Button::Glo)))
            {
                self.nav.press(PageButton::GlobalFx);
            } else if let Some((_, button)) = page
                && !(home && self.seq.back())
            {
                self.nav.press(button);
            }
            let presets = scan.encoder_delta(Encoder::Presets);
            if !home {
                self.nav.turn(scan.encoder_delta(Encoder::Select), presets);
            }
            // Keep the detent time in the navigation snapshot even if the scan queue is full.
            let channel_changed = self.nav.focus != previous_channel;
            let show_picker = match self.nav.view {
                View::Scope => channel_changed,
                View::Channel => true,
                _ => false,
            };
            if !home && presets != 0 && show_picker {
                self.nav.preset_since = Some(ticks());
            } else if home
                || channel_changed
                || self.nav.view != previous_view
                || self
                    .nav
                    .preset_since
                    .is_some_and(|since| preset_age(since, ticks()).is_none())
            {
                self.nav.preset_since = None;
            }
            if (self.nav.view == View::Sequencer) != self.seq.page.is_some() {
                self.seq.toggle_page();
            }
            if self.nav.view == View::Sequencer {
                self.seq.channel = self.nav.focus;
                self.seq.turn_bar(scan.encoder_delta(Encoder::Knob2));
                if scan.pressed(Button::Sel) {
                    self.seq.toggle_track();
                }
            }
            if !home {
                self.seq.turn(
                    scan.encoder_delta(Encoder::Select),
                    scan.encoder_delta(Encoder::Knob3),
                );
            }
            self.leds.set(Button::Seq, self.seq.page.is_some());
            self.leds.set(
                Button::Home,
                matches!(self.nav.view, View::Channel | View::MasterFx(Some(_)))
                    || self.seq.page == Some(SeqPage::Tempo),
            );
            self.leds.set(
                Button::Glo,
                self.nav.view.is_mixer() || matches!(self.nav.view, View::MasterFx(_)),
            );
            self.leds.set(
                Button::Fx,
                matches!(self.nav.view, View::Fx | View::MasterFx(_)),
            );
            self.leds.set(Button::Env, self.nav.view == View::Envelope);
            let now = cycles();
            // Time the recovery chord against the crystal even if scan interrupts run late.
            if scan.is_down(Button::OctaveDown) && scan.is_down(Button::OctaveUp) {
                self.piano.octave = 0;
                if now.wrapping_sub(*self.octave_since.get_or_insert(now)) >= 2 * Timers::CRYSTAL_HZ
                {
                    enter_download_mode();
                }
            } else {
                self.octave_since = None;
                if scan.pressed(Button::OctaveDown) {
                    self.piano.shift_octave(-1);
                } else if scan.pressed(Button::OctaveUp) {
                    self.piano.shift_octave(1);
                }
            }
            let octave = self.piano.octave;
            // At 24 MHz these bits toggle every 350 ms and 175 ms respectively.
            let lit = match octave.unsigned_abs() {
                1 => now & (1 << 23) == 0,
                2 => now & (1 << 22) == 0,
                _ => true,
            };
            self.leds.set(Button::OctaveDown, octave < 0 && lit);
            self.leds.set(Button::OctaveUp, octave > 0 && lit);
            // Release first so a key swap in the same sweep can start a new phrase.
            for key in PianoKey::all().filter(|&key| scan.released(key)) {
                self.piano.release(53 + key.index());
            }
            let channel = self.nav.piano_channel(&mut self.last_melodic);
            let assignment = self.nav.assignments[channel.index()];
            for key in PianoKey::all().filter(|&key| scan.is_down(key)) {
                self.leds.set(key, true);
                if scan.pressed(key) {
                    let velocity = match touch.velocity(key) {
                        Some(velocity) => velocity,
                        None if assignment.part.is_kit() => 127,
                        None => continue,
                    };
                    self.piano.press(
                        channel,
                        assignment,
                        Note {
                            number: 53 + key.index(),
                            velocity,
                        },
                    );
                }
            }
            if !self.scans.is_full() {
                let _ = self.scans.try_send(PanelFrame {
                    scan,
                    seq: self.seq,
                    nav: self.nav,
                    pads: self.piano.pads,
                    held: self.piano.held.clone(),
                });
            }
        }
    }

    #[async_task(priority = 2, shared = [settings, leds, midi_epoch])]
    struct Input {
        input: Receiver<'static, PanelFrame, SCANS>,
        midi: Receiver<'static, MidiEvent, MIDI_EVENTS>,
        midi_epoch: u32,
        keys: &'static mut Keys<PIANO_KEYS>,
        held: &'static mut Vec<Press, PIANO_KEYS>,
        editor: Controller<'static, bool>,
        settings: Settings,
        half_gain: bool,
    }

    impl RticAsyncTask for Input {
        type SpawnInput = ();
        async fn exec(&mut self, (): ()) {
            while let Ok(PanelFrame {
                scan,
                seq,
                nav,
                pads,
                held,
            }) = self.input.recv().await
            {
                let before = self.settings;
                let channel = nav.focus;
                let view = nav.view;
                self.settings.nav = nav;
                self.settings.seq = seq;
                let tracks = &mut self.settings.tracks;
                for channel in Channel::ALL {
                    let assignment = nav.assignments[channel.index()];
                    if tracks[channel.index()].assignment != assignment {
                        self.keys.clear(channel);
                        tracks[channel.index()] = Track::factory(channel, assignment);
                    }
                    pads[channel.index()].apply(&mut tracks[channel.index()]);
                }
                if *self.held != held {
                    self.keys.panel(self.held, &held, tracks);
                    *self.held = held;
                }
                let epoch = self.shared().midi_epoch.lock(|epoch| *epoch);
                if epoch != self.midi_epoch {
                    self.keys.clear_midi();
                    self.midi_epoch = epoch;
                }
                while let Ok(event) = self.midi.try_recv() {
                    let track = &mut self.settings.tracks[event.channel.index()];
                    if event.epoch == epoch && event.generation == track.assignment.generation {
                        self.keys.midi(event.channel, event.action, track);
                    }
                }
                for channel in Channel::ALL {
                    self.keys
                        .snapshot(channel, &mut self.settings.tracks[channel.index()]);
                }
                let knobs = [
                    Encoder::Knob1,
                    Encoder::Knob2,
                    Encoder::Knob3,
                    Encoder::Knob4,
                ]
                .map(|encoder| scan.encoder_delta(encoder));
                let select = scan.encoder_delta(Encoder::Select);
                let algorithm = scan.encoder_delta(Encoder::Algorithm);
                let sel = scan.pressed(Button::Sel);
                match view {
                    View::Mixer => self.settings.mixer.turn_faders(knobs),
                    View::Channel => self.settings.mixer.turn_knobs(channel, knobs),
                    View::MasterFx(Some(row)) => self.settings.fx.edit(row, knobs, sel),
                    View::Fx => {
                        if nav.bass_insert() {
                            self.settings.tracks[channel.index()]
                                .bass_fx
                                .edit(knobs, sel);
                        } else {
                            self.settings
                                .mixer
                                .turn_knobs(channel, [0, knobs[1], knobs[2], 0]);
                        }
                    }
                    View::Scope | View::Envelope => {
                        let track = &mut self.settings.tracks[channel.index()];
                        let part = track.assignment.part;
                        track.voice.turn_waveform(part, algorithm);
                        track.edit_panel(view, select, knobs, sel);
                    }
                    View::MasterFx(None) | View::Sequencer => (),
                }
                if let Some(master) = scan.master() {
                    self.settings.master = master;
                }
                if before != self.settings {
                    let settings = self.settings;
                    self.shared().settings.lock(|current| *current = settings);
                }
                let sel_lamp = self.settings.sel_lamp();
                if before.sel_lamp() != sel_lamp {
                    self.shared()
                        .leds
                        .lock(|leds| leds.set(Button::Sel, sel_lamp));
                }
                if matches!(view, View::Fx | View::MasterFx(_))
                    && algorithm != 0
                    && self.editor.commit(!self.half_gain).await.is_ok()
                {
                    self.half_gain = !self.half_gain;
                }
            }
        }
    }

    #[task(binds = ALNK0, priority = 3, shared = [settings, overloads, motion, bumper])]
    struct Audio {
        output: Output,
        instrument: &'static mut Instrument<'static>,
        buses: &'static mut [[I24; OUTPUT_FRAMES]; BUS_COUNT],
        host: Host<'static, bool>,
        first_frame: u64,
        tap: Sender<'static, ScopeBlock, 2>,
        gap: bool,
        stats: Stats,
        last_start: Option<u32>,
        meter: Meter,
        assignments: [Assignment; CHANNELS],
    }

    impl RticTask for Audio {
        fn exec(&mut self) {
            let started = cycles();
            let settings = self.shared().settings.lock(|s| *s);
            let mut bumper = self.shared().bumper;
            let mut revision = None;
            let result = self.output.render(settings.master, |out| {
                revision = Some(
                    self.host
                        .begin_block(|half| self.instrument.set_half_gain(half)),
                );
                self.instrument.set_mixer(settings.mixer);
                self.instrument.set_fx(settings.fx);
                self.instrument.transport = settings.seq.transport;
                self.instrument.sequences = settings.seq.enabled;
                self.instrument.cue = bumper.lock(|b| b.take(out.len(), settings.master.raw()));
                let mono = self
                    .instrument
                    .render(settings.tracks, self.buses)
                    .expect("fixed patch");
                for (frame, sample) in out.iter_mut().zip(mono) {
                    *frame = [sample.inner(); 2];
                }
            });
            let Some(revision) = revision else { return }; // No DMA half was ready.
            if result == Refill::Written {
                self.host.submitted(revision, self.first_frame);
                // Keep metering and the visual clock current even when scope capture drops a block.
                for i in 0..CHANNELS {
                    if self.assignments[i] != settings.nav.assignments[i] {
                        self.meter.clear(i);
                    }
                }
                self.assignments = settings.nav.assignments;
                let peaks = if settings.nav.view.is_mixer() {
                    self.meter
                        .update(settings.mixer, &self.buses[STRIP_BUS..][..CHANNELS])
                } else {
                    self.meter = Meter::default();
                    [0; CHANNELS]
                };
                let position = self.instrument.position;
                let bass_automation = self.instrument.bass_automation(settings.nav.focus);
                let drums = self.instrument.drums;
                let wave_frames = self.instrument.wave_frames;
                let reduction = self.instrument.reduction;
                self.shared().motion.lock(|motion| {
                    *motion = Motion {
                        position,
                        bass_automation,
                        peaks,
                        reduction,
                        drums,
                        wave_frames,
                    };
                });
            }
            self.first_frame = self.first_frame.wrapping_add(OUTPUT_FRAMES as u64);
            if result == Refill::Written && !self.tap.is_full() {
                let labels = settings.labels(self.instrument.half_gain);
                let block = ScopeBlock {
                    samples: match (labels.view, labels.voice_tap) {
                        (View::Mixer | View::Channel | View::MasterFx(_), _) => [0; OUTPUT_FRAMES],
                        (View::Fx, _) | (View::Scope, true) => {
                            labels
                                .channel
                                .scope_samples(labels.focus, labels.view, self.buses)
                        }
                        _ => core::array::from_fn(|i| self.output.block()[i][0]),
                    },
                    bass_cycle: self.instrument.bass_cycles[labels.channel.index()],
                    labels,
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
                let labels = block.labels;
                let previous = self.previous.replace(labels);
                let changed = previous != Some(labels);
                let source_changed = previous.is_none_or(|old| {
                    old.view != labels.view
                        || old.channel != labels.channel
                        || old.focus != labels.focus
                        || old.voice_tap != labels.voice_tap
                });
                let voice_scope = labels.view == View::Scope && labels.voice_tap;
                let bass_scope = voice_scope && labels.focus == Part::Bass;
                // Keep the 303 trigger and the post-insert FX trace continuous through knob edits.
                if block.gap
                    || source_changed
                    || (changed && !bass_scope && labels.view != View::Fx)
                {
                    self.capture.reset();
                }
                let trace = if bass_scope {
                    self.capture.push_bass(&block.samples, block.bass_cycle)
                } else {
                    let scale = if voice_scope && labels.focus == Part::Wavetable {
                        Scale::Auto
                    } else {
                        Scale::Fixed
                    };
                    self.capture.push(&block.samples, scale)
                };
                if changed || trace.is_some() {
                    self.shared().frame.lock(|frame| {
                        // Publish the new page even while waiting for its first trigger.
                        if let Some(trace) = trace {
                            frame.trace = trace;
                        } else if source_changed {
                            frame.trace = FLAT;
                        }
                        frame.labels = labels;
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

    #[async_task(priority = 1, shared = [overloads, settings, midi_epoch])]
    struct Usb {
        device: MidiDevice,
        clock: Clock,
        midi: Sender<'static, MidiEvent, MIDI_EVENTS>,
    }
    impl RticAsyncTask for Usb {
        type SpawnInput = ();
        async fn exec(&mut self, (): ()) {
            let mut rx = PacketBuffer::default();
            let mut log = UsbLog::new();
            log.record("WaveGraph ready").expect("short ASCII line");
            let mut logged = 0;
            let mut session = None;
            let mut epoch = 0_u32;
            loop {
                if session.is_some() {
                    // Some cable removals raise no IRQ, so sample host frame progress.
                    if Clock::timeout_after(
                        100.millis(),
                        self.device.wait_for_activity(&mut self.clock),
                    )
                    .await
                    .is_err()
                    {
                        self.device.poll();
                    }
                } else {
                    self.device.wait_for_activity(&mut self.clock).await;
                }
                let packets = self.device.receive(&mut rx);
                let active = self.device.session();
                if session != active {
                    session = active;
                    epoch = epoch.wrapping_add(1);
                    self.shared().midi_epoch.lock(|current| *current = epoch);
                }
                if !packets.is_empty() {
                    let assignments = self.shared().settings.lock(|s| s.nav.assignments);
                    for (channel, action) in packets.iter().copied().filter_map(decode_midi) {
                        let generation = assignments[channel.index()].generation;
                        if self
                            .midi
                            .try_send(MidiEvent {
                                epoch,
                                generation,
                                channel,
                                action,
                            })
                            .is_err()
                        {
                            // Retire this batch if the input task fell behind, including any lost releases.
                            epoch = epoch.wrapping_add(1);
                            self.shared().midi_epoch.lock(|current| *current = epoch);
                            break;
                        }
                    }
                }
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

    #[async_task(priority = 1, shared = [frame, overloads, motion, bumper])]
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
            let mut bumper = self.shared().bumper;
            let splash = screen
                .splash(&mut lcd, &mut self.clock, || bumper.lock(Bumper::start))
                .await;
            bumper.lock(Bumper::stop);
            if splash.is_err() {
                return;
            }
            // A failed display transfer ends only this task. Audio and panel IRQs keep running.
            loop {
                // Count paint time toward the frame period and sample the latest frame on a miss.
                let next_frame = Clock::now() + 30.millis();
                let frame = self.shared().frame.lock(|f| *f);
                let severity = self.shared().overloads.lock(|log| log.severity(ticks()));
                let motion = self.shared().motion.lock(|motion| *motion);
                if screen
                    .paint(&mut lcd, &mut self.clock, &frame, severity, motion)
                    .await
                    .is_err()
                {
                    return;
                }
                Clock::delay_until(next_frame).await;
            }
        }
    }
}
