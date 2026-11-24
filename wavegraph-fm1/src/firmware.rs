//! RTICX tasks for the FM-1 panel, audio DMA, scope capture and display.
#![allow(
    missing_docs,
    reason = "RTICX generates public task traits and initializers."
)]

mod render;
mod worker;

use crate::{
    bass::{Controls, Knob},
    capture::Trace,
    envelope::{EnvelopeControls, EnvelopeTarget},
    fx::{BassFx, Fx},
    instrument::{Part, SawControls, Track, Voice},
    keys::PolyKey,
    mixer::Mixer,
    organ::Controls as OrganControls,
    panel::{CHANNELS, Channel, Navigation, View},
    sequencer::{BankPosition, DrumLamp, Position, Sequencer, pattern::Automation},
    wavetable::Program as WaveProgram,
};
use fm1_audio::MasterLevel;
use wavegraph_arp::Chord;

// Publish the chord and its settings under one lock. Only changed, armed
// channels copy chord storage between the input and audio tasks.
pub struct Performance {
    settings: Settings,
    revision: u32,
    chords: &'static mut [Chord; CHANNELS],
}

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

    fn labels(&self, half_gain: bool) -> Labels {
        let channel = self.nav.focus;
        let track = self.tracks[channel.index()];
        Labels {
            channel,
            assignments: self.nav.assignments.map(|assignment| assignment.part),
            voice: track.voice,
            arp: track.arp,
            focus: track.assignment.part,
            bass: track.bass,
            bass_knob: self.nav.bass_knob(),
            organ: track.organ,
            drums808: track.drums808,
            saw: track.saw,
            wave: track.wave_program(),
            envelope_target: track.envelope_target,
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

    fn sel_lamp(&self) -> bool {
        match self.nav.view {
            View::Arp => self.tracks[self.nav.focus.index()].arp.enabled,
            View::Fx => {
                self.nav.bass_insert() && self.tracks[self.nav.focus.index()].bass_fx.enabled
            }
            View::MasterFx(Some(row)) => self.fx.enabled(row),
            View::Envelope if matches!(self.nav.part(), Part::Supersaw | Part::Wavetable) => {
                self.tracks[self.nav.focus.index()].envelope_target == EnvelopeTarget::Modulation
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
    pub(super) arp: crate::arp::Controls,
    pub(super) focus: Part,
    pub(super) bass: Controls,
    pub(super) bass_knob: Option<Knob>,
    pub(super) organ: OrganControls,
    pub(super) drums808: wavegraph_808::Controls,
    pub(super) saw: SawControls,
    pub(super) wave: WaveProgram,
    pub(super) envelope_target: EnvelopeTarget,
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
    pub(super) bass_automation: Option<(Channel, Automation)>,
    pub(super) banks: [BankPosition; CHANNELS],
    pub(super) peaks: [u8; CHANNELS],
    pub(super) arp_notes: [Option<u8>; CHANNELS],
    pub(super) reduction: u8,
    pub(super) drums: [[DrumLamp; 6]; CHANNELS],
    pub(super) wave_frames: [u8; CHANNELS],
}

/// Renderer and DMA service timings in 24 MHz TIMER4 ticks. Peaks cover this boot.
#[derive(Clone, Copy, Default)]
pub struct Timing {
    pub(super) boot: u32,
    pub(super) visible: bool,
    pub(super) refill: u32,
    pub(super) playing_refill: u32,
    pub(super) commit: u32,
    pub(super) irq: u32,
    pub(super) overruns: u32,
    pub(super) worker: u32,
    pub(super) worker_stack: Option<u32>,
    pub(super) silences: u32,
    pub(super) discards: u32,
    pub(super) last_discard: u32,
    pub(super) last_edit: u32,
}

#[derive(Clone, Copy)]
pub struct Frame {
    pub(super) trace: Trace,
    pub(super) labels: Labels,
}

#[fm1_rticx::app(dispatchers = [SOFT0, SOFT1])]
mod app {
    use super::render::{RenderBlock, Renderer};
    use super::worker::{self, BlockJob, Worker};
    use super::{Frame, Labels, Motion, Performance, Settings, Timing};
    use crate::{
        bumper::Bumper,
        capture::{Capture, FLAT, Scale, ScopeUpdates},
        handoff::Handoff,
        inserts::BassInserts,
        instrument::{
            AudioProcess, BUS_COUNT, DRY_HEADROOM, FxMixer, FxNode, Instrument, Master, Node, Note,
            Part, Programs, RoutedEvents, Track,
        },
        keys::{Keys, MidiAction, PanelPads, Piano, Press, decode_midi},
        midi_out::OutputQueue,
        overload::Overloads,
        panel::{CHANNELS, Channel, Navigation, PageButton, View, pattern_press_lamp, preset_age},
        screen::Screen,
        sequencer::{DrumKits, SeqPage, Sequencer, pattern::Slot},
    };
    use core::pin::pin;
    use fm1_audio::{FRAME_RATE, I24, OUTPUT_FRAMES, Output, Refill, Stats};
    use fm1_controls::{Button, Encoder, Leds, PianoKey, PlayLedState, Scan, TouchControls};
    use fm1_rticx::{
        bsp::{
            Board, Cpu0,
            display::{DisplayPort, SpiClock, on_spi1_interrupt},
            time::{Timers, cycles, ticks},
            usb_midi::{Identity, MidiDevice, PacketBuffer, UsbLog, on_usb0_interrupt},
        },
        rticx::asynchronous::{
            channel::{Receiver, Sender},
            make_channel,
        },
        time::{Clock, ExtU32, Timebase, Timer5},
    };
    use futures_util::future::{Either, select};
    use heapless::Vec;
    use static_cell::{ConstStaticCell, StaticCell};
    use wavegraph::{
        control::{Controller, EditQueue, Host, Revision},
        mailbox,
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
        samples: Option<[i32; OUTPUT_FRAMES]>,
        bass_cycle: Option<usize>,
        labels: Labels,
        gap: bool,
    }

    #[shared]
    struct Shared {
        settings: Performance,
        bumper: Bumper,
        leds: Leds,
        frame: Frame,
        overloads: Overloads,
        timing: Timing,
        stream: Option<Playback>,
        motion: Motion,
        midi_epoch: u32,
        outbox: OutputQueue,
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
        type Nodes = [&'static mut AudioProcess<'static>; 9];
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
        let instrument = INSTRUMENT.init_with(|| {
            programs
                .bind(nodes, EVENTS.take(), FRAME_RATE)
                .expect("instrument ports")
        });
        for half in [true, false] {
            instrument
                .set_half_gain(half)
                .expect("both fixed routes fit");
        }
        instrument
    }

    #[init]
    fn init() -> (Shared, TaskInits) {
        // SAFETY: RTICX init runs once on CPU0 with interrupts disabled.
        // The reset prefix held CPU1 before CRT. Board::take starts TIMER4
        // after the BSP bootstrap has finished using it for the ROM handshake.
        let boot = unsafe { crate::peer::start() };
        let board = Board::take().expect("CPU0 board");
        let port = board.take_usb().expect("one USB device");
        let identity = Identity {
            vendor_id: 0x1209,
            product_id: 0x0001,
            manufacturer: "fm1-rust",
            product: "WaveGraph FM-1",
        };
        #[cfg(not(feature = "qualified-usb"))]
        let usb = port.try_into_midi(identity).ok();
        // SAFETY: this opt-in image requires the bench operator to qualify the
        // controller, host and physical connection for the BSP's complete
        // USB0 receive DMA contract, including reset, suspend and reconnect.
        #[cfg(feature = "qualified-usb")]
        let usb = Some(unsafe { port.assume_midi_dma() }.into_midi(identity));
        let timer = Clock::start(board.timers.with_panel_scan_rate());
        let clock = timer.clock();
        // Keep the DMA-sized buses out of the task initializer and its stack.
        static BUSES: ConstStaticCell<[[I24; OUTPUT_FRAMES]; BUS_COUNT]> = ConstStaticCell::new(
            [[wavegraph_sample::types::i24::EQUILIBRIUM; OUTPUT_FRAMES]; BUS_COUNT],
        );
        let instrument = initialize_graph();
        // SAFETY: the reset prefix clears this linker-reserved SRAM, including
        // the StaticCell flag, before main. The unique slot moves between tasks.
        #[unsafe(link_section = ".wavegraph.job")]
        static SLOT: StaticCell<RenderBlock> = StaticCell::new();
        let slot = SLOT.init_with(RenderBlock::new);
        static JOBS: ConstStaticCell<mailbox::Mailbox<BlockJob>> =
            ConstStaticCell::new(mailbox::Mailbox::new());
        static DONE: ConstStaticCell<mailbox::Mailbox<BlockJob>> =
            ConstStaticCell::new(mailbox::Mailbox::new());
        static GATE: ConstStaticCell<mailbox::Mailbox<()>> =
            ConstStaticCell::new(mailbox::Mailbox::new());
        let (mut issue, jobs) = JOBS.take().split();
        let (completed, done) = DONE.take().split();
        let (release, gate) = GATE.take().split();

        static EDITS: ConstStaticCell<EditQueue<bool>> = ConstStaticCell::new(EditQueue::new());
        let (editor, mut host) = EDITS.take().split();
        let mut handoff = Handoff::new(slot);
        // Render the initial, muted settings before starting the DMA clock.
        let first = handoff
            .issue(|_| host.begin_block(|_| Ok(())))
            .expect("initial slot");
        issue.try_send(first).ok().expect("empty render queue");
        if boot == 1 {
            worker::install(Worker {
                renderer: Renderer::new(instrument, BUSES.take()),
                jobs,
                completed,
                release: gate,
            });
        }
        let (scans, input) = make_channel!(PanelFrame, SCANS);
        let (wake_usb, outbound) = make_channel!((), 1);
        let (midi_out, midi_in) = make_channel!(MidiEvent, MIDI_EVENTS);
        // The RTICX entry frame lives on the user stack while interrupts run.
        // Keep full MIDI chords in SRAM alongside the key-owner tables.
        static CHORDS: ConstStaticCell<[wavegraph_arp::Chord; CHANNELS]> =
            ConstStaticCell::new([const { wavegraph_arp::Chord::new() }; CHANNELS]);
        static KEYS: ConstStaticCell<Keys<PIANO_KEYS>> = ConstStaticCell::new(Keys::new());
        static PIANO: ConstStaticCell<Piano<PIANO_KEYS>> = ConstStaticCell::new(Piano::new());
        static HELD: ConstStaticCell<Vec<Press, PIANO_KEYS>> = ConstStaticCell::new(Vec::new());
        let (tap, blocks) = make_channel!(ScopeBlock, 2);
        let mut leds = Leds::default();
        leds.set_play(PlayLedState::Red);
        leds.set(Button::Sel, Settings::INITIAL.sel_lamp());
        (
            Shared {
                settings: Performance {
                    settings: Settings::INITIAL,
                    revision: 0,
                    chords: CHORDS.take(),
                },
                bumper: Bumper::default(),
                leds,
                frame: Frame {
                    trace: FLAT,
                    labels: Settings::INITIAL.labels(false),
                },
                overloads: Overloads::default(),
                timing: Timing {
                    boot,
                    ..Timing::default()
                },
                stream: None,
                motion: Motion::default(),
                midi_epoch: 0,
                outbox: OutputQueue::default(),
            },
            TaskInits {
                start_audio: StartAudio {
                    port: Some(board.audio.into()),
                    completed: Some(done),
                    host: (boot == 1).then_some(host),
                },
                audio: Audio {
                    handoff,
                    jobs: issue,
                    missing_since: None,
                    release,
                    half_gain: false,
                    pending_edit: None,
                    gated: false,
                    wake_usb: usb.as_ref().map(|_| wake_usb),
                    playback: None,
                    tap,
                    scope_updates: ScopeUpdates::default(),
                    stats: Stats::default(),
                    last_start: None,
                },
                panel: Panel {
                    timer,
                    controls: board.panel.into_touch_controls().into(),
                    piano: PIANO.take(),
                    nav: Navigation::INITIAL,
                    seq: Sequencer::INITIAL,
                    scans,
                    leds,
                    octave_since: None,
                    pattern_pressed_at: None,
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
                    device: usb.map(Cpu0::from),
                    clock,
                    midi: midi_out,
                    outbound,
                },
                draw: Draw {
                    port: Some(board.display.into()),
                    clock,
                },
            },
        )
    }

    #[post_init]
    fn start() {
        StartAudio::spawn(()).expect("one audio startup");
        Input::spawn(()).expect("one input task");
        Observe::spawn(()).expect("one scope task");
        Draw::spawn(()).expect("one display task");
        Usb::spawn(()).expect("one USB task");
    }

    // The upper seven keys share an LED line with the velocity probe. End its
    // 100 µs column slot on time even when the audio graph is rendering.
    #[task(binds = TIMER5, priority = 5, shared = [leds, motion, timing])]
    struct Panel {
        timer: Timer5,
        controls: Cpu0<TouchControls>,
        piano: &'static mut Piano<PIANO_KEYS>,
        nav: Navigation,
        seq: Sequencer,
        scans: Sender<'static, PanelFrame, SCANS>,
        leds: Leds,
        octave_since: Option<u32>,
        pattern_pressed_at: Option<u32>,
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
            if scan.pressed(Button::Save) || scan.released(Button::Save) {
                self.shared()
                    .timing
                    .lock(|t| t.visible = scan.is_down(Button::Save));
            }
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
                (Button::Arp, PageButton::Arp),
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
                self.seq
                    .turn_bar(self.nav.part(), scan.encoder_delta(Encoder::Knob2));
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
                    || self.nav.bass_knob().is_some()
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
            self.leds.set(Button::Arp, self.nav.view == View::Arp);
            let now = cycles();
            // Time the recovery chord against the crystal even if scan interrupts run late.
            if scan.is_down(Button::OctaveDown) && scan.is_down(Button::OctaveUp) {
                self.piano.octave = 0;
                if now.wrapping_sub(*self.octave_since.get_or_insert(now)) >= 2 * Timers::CRYSTAL_HZ
                {
                    crate::peer::download();
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
            let channel = self.nav.focus;
            let assignment = self.nav.assignments[channel.index()];
            // Releases always reach their original owners, including across page changes.
            for key in PianoKey::all().filter(|&key| scan.released(key)) {
                self.piano
                    .event(channel, assignment, MidiAction::Off(53 + key.index()));
            }
            if self.seq.page == Some(SeqPage::Pattern) {
                let tick = ticks();
                if channel_changed {
                    self.pattern_pressed_at = None;
                }
                for (key, slot) in Slot::KEYS {
                    if scan.pressed(PianoKey::new(key).expect("physical bank key")) {
                        self.seq.choose(assignment.part, slot);
                        self.pattern_pressed_at = Some(tick);
                    }
                }
                let press_lamp = pattern_press_lamp(&mut self.pattern_pressed_at, tick);
                self.seq.pressed_key_dark = press_lamp == Some(false);
                let requested = self.seq.slot(channel, assignment.part);
                let active = if self.seq.transport.playing() {
                    self.shared().motion.lock(|m| m.banks[channel.index()].slot)
                } else {
                    requested
                };
                for (key, slot) in Slot::KEYS {
                    let lamp = match (slot == requested, press_lamp) {
                        (true, Some(lamp)) => lamp,
                        _ => slot != active || (now & (1 << 22)) == 0,
                    };
                    self.leds
                        .set(PianoKey::new(key).expect("physical bank key"), lamp);
                }
            } else {
                self.pattern_pressed_at = None;
                self.seq.pressed_key_dark = false;
                for key in PianoKey::all().filter(|&key| scan.is_down(key)) {
                    self.leds.set(key, true);
                    if scan.pressed(key) {
                        let velocity = match touch.velocity(key) {
                            Some(velocity) => velocity,
                            None if assignment.part.is_kit() => 127,
                            None => continue,
                        };
                        self.piano.event(
                            channel,
                            assignment,
                            MidiAction::On(Note {
                                number: 53 + key.index(),
                                velocity,
                            }),
                        );
                    }
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
                let old_lamp = self.settings.sel_lamp();
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
                    for track in &mut self.settings.tracks {
                        track.bass.slide = false;
                    }
                    self.midi_epoch = epoch;
                }
                while let Ok(event) = self.midi.try_recv() {
                    let track = &mut self.settings.tracks[event.channel.index()];
                    if event.epoch == epoch && event.generation == track.assignment.generation {
                        self.keys.midi(event.channel, event.action, track);
                    }
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
                    View::Arp => self.settings.tracks[channel.index()]
                        .arp
                        .edit(select, knobs, sel),
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
                        track.turn_algorithm(nav.bass_knob(), algorithm);
                        track.edit_panel(view, select, knobs, sel);
                    }
                    View::MasterFx(None) | View::Sequencer => (),
                }
                if let Some(master) = scan.master() {
                    self.settings.master = master;
                }
                for channel in Channel::ALL {
                    let track = &mut self.settings.tracks[channel.index()];
                    self.keys
                        .set_latch(channel, track.arp.enabled && track.arp.config.latch);
                    self.keys.snapshot(channel, track);
                }
                let settings = &self.settings;
                self.shared().settings.lock(|current| {
                    if *settings != current.settings {
                        for channel in Channel::ALL {
                            let i = channel.index();
                            if settings.tracks[i]
                                .arp_chord_changed(Some(&current.settings.tracks[i]))
                            {
                                current.chords[i].clone_from(self.keys.chord(channel));
                            }
                        }
                        current.settings = *settings;
                        current.revision = current.revision.wrapping_add(1);
                    }
                });
                let sel_lamp = settings.sel_lamp();
                if old_lamp != sel_lamp {
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

    pub struct Playback {
        output: &'static mut Output,
        completed: mailbox::Receiver<'static, BlockJob>,
        first: Option<BlockJob>,
        host: Option<Host<'static, bool>>,
    }

    // Give the cold graph its first full render before starting the DMA clock.
    #[async_task(priority = 2, shared = [stream, timing])]
    struct StartAudio {
        port: Option<Cpu0<fm1_audio::AudioPort>>,
        completed: Option<mailbox::Receiver<'static, BlockJob>>,
        host: Option<Host<'static, bool>>,
    }

    impl RticAsyncTask for StartAudio {
        type SpawnInput = ();
        async fn exec(&mut self, (): ()) {
            let Some(host) = self.host.take() else { return };
            let mut completed = self.completed.take().expect("one DSP worker");
            let started = cycles();
            let first = loop {
                let fault = if !crate::peer::healthy() {
                    Some(4)
                } else if cycles().wrapping_sub(started) >= Timers::CRYSTAL_HZ {
                    Some(5)
                } else {
                    None
                };
                if let Some(code) = fault {
                    crate::peer::stop();
                    self.shared().timing.lock(|t| t.boot = code);
                    return;
                }
                if let Some(first) = completed.try_recv() {
                    break first;
                }
                Clock::delay(1.millis()).await;
            };
            static OUTPUT: StaticCell<Output> = StaticCell::new();
            let port = self.port.take().expect("start once").into_inner();
            self.shared().stream.lock(|stream| {
                let output = OUTPUT.init_with(|| Output::start(port).expect("audio clock"));
                // Clear the muted half left pending by the BSP's 5 ms DAC startup.
                output.render(fm1_audio::MasterLevel::MIN, |_| {});
                *stream = Some(Playback {
                    output,
                    completed,
                    first: Some(first),
                    host: Some(host),
                });
            });
        }
    }

    #[task(binds = ALNK0, priority = 4, shared = [settings, overloads, timing, motion, bumper, outbox, stream])]
    struct Audio {
        playback: Option<Playback>,
        missing_since: Option<u32>,
        handoff: Handoff<&'static mut RenderBlock>,
        jobs: mailbox::Sender<'static, BlockJob>,
        release: mailbox::Sender<'static, ()>,
        gated: bool,
        half_gain: bool,
        pending_edit: Option<Revision>,
        wake_usb: Option<Sender<'static, (), 1>>,
        tap: Sender<'static, ScopeBlock, 2>,
        scope_updates: ScopeUpdates,
        stats: Stats,
        last_start: Option<u32>,
    }

    impl RticTask for Audio {
        fn exec(&mut self) {
            let started = cycles();
            let mut shared = self.shared();
            let (master, playing) = shared
                .settings
                .lock(|s| (s.settings.master, s.settings.seq.transport.playing()));
            let mut done = None;
            let mut current = false;
            let playback = self.playback.get_or_insert_with(|| {
                shared
                    .stream
                    .lock(Option::take)
                    .expect("DMA started with a primed block")
            });
            let output = &mut *playback.output;
            let Some(host) = &mut playback.host else {
                output.render(master, |_| {});
                return;
            };
            let healthy = crate::peer::healthy();
            let result = output.render(master, |out| {
                if !healthy {
                    return;
                }
                done = playback
                    .first
                    .take()
                    .or_else(|| playback.completed.try_recv());
                current = self.handoff.fill(out, done.as_ref(), |b| &b.pcm);
            });
            let refill_ended = cycles();
            if result == Refill::WouldBlock {
                return;
            }
            // A transient deadline miss leaves the slot with CPU1. Only a
            // reported fault or a full second without a return stops the peer.
            if done.is_some() {
                self.missing_since = None;
            } else {
                let since = *self.missing_since.get_or_insert(started);
                let code = if !healthy {
                    Some(4)
                } else if started.wrapping_sub(since) >= Timers::CRYSTAL_HZ {
                    Some(5)
                } else {
                    None
                };
                if let Some(code) = code {
                    crate::peer::stop();
                    playback.host = None;
                    shared.timing.lock(|t| t.boot = code);
                    return;
                }
            }
            let written = result == Refill::Written;
            let silence = written && !current;
            if cfg!(feature = "late-block-probe") && silence && self.gated {
                self.release.try_send(()).expect("one gated job");
                self.gated = false;
            }
            let mut stack_free = None;
            let mut work = 0;
            let mut discarded = None;
            let mut edit_sequence = None;
            if let Some(job) = &done {
                let block = &*job.slot;
                work = block.work;
                stack_free = block.stack_free;
                discarded = (!current).then(|| job.sequence());
                // Deliver note-offs even if the corresponding PCM expired. A
                // disconnect or queue reset retires the captured USB session.
                if let Some(session) = block.usb_session
                    && shared
                        .outbox
                        .lock(|q| q.session() == Some(session) && q.append(&block.midi))
                    && let Some(wake) = &mut self.wake_usb
                {
                    let _ = wake.try_send(());
                }
                if written && current {
                    if self.pending_edit == Some(job.revision()) {
                        edit_sequence = Some(job.sequence());
                        self.pending_edit = None;
                    }
                    shared.motion.lock(|m| *m = block.motion);
                    let settings = &block.settings;
                    let visible = settings.nav.view.shows_trace(settings.nav.part());
                    self.scope_updates.publish(
                        (block.settings_revision, block.half_gain),
                        visible,
                        |gap| {
                            if self.tap.is_full()
                                || (visible && settings.source_tap() && !block.capture_source)
                            {
                                return false;
                            }
                            self.tap
                                .try_send(ScopeBlock {
                                    samples: visible.then(|| {
                                        if settings.source_tap() {
                                            block.source
                                        } else {
                                            core::array::from_fn(|i| output.block()[i][0])
                                        }
                                    }),
                                    labels: settings.labels(block.half_gain),
                                    bass_cycle: block.bass_cycle,
                                    gap,
                                })
                                .is_ok()
                        },
                    );
                }
            }
            if !(written && current) {
                self.scope_updates.missed();
            }
            self.handoff.finish(host, result, &mut done);
            let job = self.handoff.issue(|block| {
                shared.settings.lock(|s| block.snapshot(s));
                block.cue = shared.bumper.lock(|b| b.take(OUTPUT_FRAMES, master.raw()));
                block.usb_session = shared.outbox.lock(|q| q.session());
                block.capture_source = block.settings.source_tap() && !self.tap.is_full();
                block.hold = false;
                let mut edited = false;
                let revision = host.begin_block(|half| {
                    self.half_gain = half;
                    edited = true;
                    Ok(()) // Both fixed programs were validated against these nodes at boot.
                });
                block.half_gain = self.half_gain;
                if edited {
                    self.pending_edit = Some(revision);
                    block.hold =
                        cfg!(feature = "late-block-probe") && shared.timing.lock(|t| t.visible);
                    self.gated = block.hold;
                }
                revision
            });
            if let Some(job) = job {
                self.jobs
                    .try_send(job)
                    .ok()
                    .expect("free slot implies empty job queue");
            }
            let previous = self.last_start.replace(started);
            if self.stats.record_with_refill(started, refill_ended, result) {
                let elapsed = cycles().wrapping_sub(started);
                let gap = previous.map_or(0, |prev| started.wrapping_sub(prev));
                shared.overloads.lock(|log| {
                    log.record(
                        self.stats.overruns,
                        ticks(),
                        elapsed,
                        gap,
                        result == Refill::Late,
                    )
                });
            }
            shared.timing.lock(|t| {
                let refill = refill_ended.wrapping_sub(started);
                t.refill = t.refill.max(refill);
                if playing {
                    t.playing_refill = t.playing_refill.max(refill);
                }
                t.commit = self.stats.peak_commit_ticks;
                t.irq = self.stats.peak_ticks;
                t.overruns = self.stats.overruns;
                t.worker = t.worker.max(work);
                if stack_free.is_some() {
                    t.worker_stack = stack_free;
                }
                t.silences = t.silences.saturating_add(u32::from(silence));
                if let Some(sequence) = discarded {
                    t.discards = t.discards.saturating_add(1);
                    t.last_discard = sequence;
                }
                if let Some(sequence) = edit_sequence {
                    t.last_edit = sequence;
                }
            });
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
                let trace = block.samples.as_ref().and_then(|samples| {
                    if bass_scope {
                        self.capture.push_bass(samples, block.bass_cycle)
                    } else {
                        let scale = if voice_scope && labels.focus == Part::Wavetable {
                            Scale::Auto
                        } else {
                            Scale::Fixed
                        };
                        self.capture.push(samples, scale)
                    }
                });
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

    #[async_task(priority = 1, shared = [overloads, settings, midi_epoch, outbox, timing])]
    struct Usb {
        device: Option<Cpu0<MidiDevice>>,
        clock: Clock,
        midi: Sender<'static, MidiEvent, MIDI_EVENTS>,
        outbound: Receiver<'static, (), 1>,
    }
    impl RticAsyncTask for Usb {
        type SpawnInput = ();
        async fn exec(&mut self, (): ()) {
            let Some(mut device) = self.device.take() else {
                return;
            };
            let mut rx = PacketBuffer::default();
            let mut tx = PacketBuffer::default();
            let mut log = UsbLog::new();
            log.record("WaveGraph ready").expect("short ASCII line");
            let mut logged = Some(0);
            let mut session = None;
            let mut epoch = 0_u32;
            loop {
                if session.is_some() {
                    // Wake on the first outgoing block as well as on incoming USB.
                    // Poll frame progress periodically for cable pulls without an IRQ.
                    let poll = {
                        let activity = pin!(Clock::timeout_after(
                            100.millis(),
                            device.wait_for_activity(&mut self.clock)
                        ));
                        let output = pin!(self.outbound.recv());
                        !matches!(select(activity, output).await, Either::Left((Ok(()), _)))
                    };
                    if poll {
                        device.poll();
                    }
                } else {
                    device.wait_for_activity(&mut self.clock).await;
                }
                let packets = device.receive(&mut rx);
                let active = device.session();
                if session != active {
                    session = active;
                    self.shared().outbox.lock(|q| q.connect(active.is_some()));
                    epoch = epoch.wrapping_add(1);
                    self.shared().midi_epoch.lock(|current| *current = epoch);
                }
                if !packets.is_empty() {
                    let assignments = self.shared().settings.lock(|s| s.settings.nav.assignments);
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
                    let t = self.shared().timing.lock(|t| *t);
                    if t.visible || t.boot != 1 {
                        let line = crate::overload::timing_line(
                            t.boot, t.worker, t.refill, t.commit, t.overruns, t.silences,
                        );
                        log.record(&line).expect("one timing snapshot");
                        logged = None;
                    } else {
                        // The full-width timing line evicts the log text. The
                        // IRQ's event history survives, including disconnects.
                        if logged.is_none() {
                            log.record("WaveGraph ready").expect("short ASCII line");
                        }
                        let last = logged.get_or_insert(0);
                        let history = self.shared().overloads.lock(|history| *history);
                        for event in history.events() {
                            if event.number > *last {
                                log.record(&event.line()).expect("bounded ASCII log line");
                                *last = event.number;
                            }
                        }
                    }
                }
                log.service(&mut device, packets);
                if active.is_some() {
                    let (revision, count) = self.shared().outbox.lock(|q| q.peek(&mut tx));
                    if count != 0 {
                        let sent = device.send(&tx[..count]);
                        self.shared().outbox.lock(|q| q.sent(revision, sent));
                    }
                }
            }
        }
    }

    #[async_task(priority = 1, shared = [frame, overloads, motion, bumper, timing])]
    struct Draw {
        port: Option<Cpu0<DisplayPort>>,
        clock: Clock,
    }
    impl RticAsyncTask for Draw {
        type SpawnInput = ();
        async fn exec(&mut self, (): ()) {
            let port = self.port.take().expect("one LCD");
            let Ok(mut lcd) = port.into_inner().init_with_clock(SpiClock::Div1) else {
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
                let timing = self.shared().timing.lock(|t| *t);
                if timing.visible || timing.boot != 1 {
                    if screen
                        .timing(&mut lcd, &mut self.clock, timing)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    Clock::delay_until(next_frame).await;
                    continue;
                }
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
