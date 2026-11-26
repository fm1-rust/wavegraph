//! The fixed 128-frame DSP job. All mutable graph state belongs to this worker.
use super::{Motion, Performance, Settings};
use crate::{
    bumper::Cue,
    instrument::{BUS_COUNT, Instrument, STRIP_BUS, mix_master},
    midi_out::Batch,
    mixer::Meter,
    panel::{Assignment, CHANNELS, Channel, Navigation, View},
};
use fm1_audio::OUTPUT_FRAMES;
use wavegraph_arp::Chord;
use wavegraph_sample::I24;

const _: () = assert!(OUTPUT_FRAMES == 128);

pub(super) struct RenderBlock {
    pub(super) settings: Settings,
    pub(super) settings_revision: u32,
    chords: [Chord; CHANNELS],
    pub(super) cue: Cue, // Immutable flash range. No panel or audio-task borrow.
    pub(super) half_gain: bool,
    pub(super) usb_session: Option<u32>,
    pub(super) capture_source: bool,
    pub(super) hold: bool,
    // CPU0 borrows the selected master until it returns this slot to CPU1.
    buses: &'static mut [[I24; OUTPUT_FRAMES]; BUS_COUNT],
    pub(super) source: [i32; OUTPUT_FRAMES],
    pub(super) motion: Motion,
    pub(super) bass_cycle: Option<usize>,
    pub(super) midi: Batch,
    pub(super) work: u32,
    pub(super) issued_at: u32,
    pub(super) ready_at: u32,
    pub(super) stack_free: Option<u32>,
}

impl RenderBlock {
    pub(super) fn new(buses: &'static mut [[I24; OUTPUT_FRAMES]; BUS_COUNT]) -> Self {
        Self {
            settings: Settings::INITIAL,
            settings_revision: 0,
            chords: core::array::from_fn(|_| Chord::new()),
            cue: Cue::EMPTY,
            half_gain: false,
            usb_session: None,
            capture_source: false,
            hold: false,
            buses,
            source: [0; OUTPUT_FRAMES],
            motion: Motion::default(),
            bass_cycle: None,
            midi: Batch::default(),
            work: 0,
            issued_at: 0,
            ready_at: 0,
            stack_free: None,
        }
    }

    pub(super) fn snapshot(&mut self, current: &Performance) {
        if current.revision != self.settings_revision {
            for (i, track) in current.settings.tracks.iter().enumerate() {
                if track.arp_chord_changed(Some(&self.settings.tracks[i])) {
                    self.chords[i].clone_from(&current.chords[i]);
                }
            }
            self.settings = current.settings;
            self.settings_revision = current.revision;
        }
    }
}

pub(super) struct Renderer {
    instrument: &'static mut Instrument<'static>,
    release: wavegraph_dsp::mailbox::Receiver<'static, ()>,
    measured: Option<u32>,
    next_frame: u64,
    applied_revision: Option<u32>,
    meter: Meter,
    assignments: [Assignment; CHANNELS],
}

impl Renderer {
    pub(super) fn new(
        instrument: &'static mut Instrument<'static>,
        release: wavegraph_dsp::mailbox::Receiver<'static, ()>,
    ) -> Self {
        Self {
            release,
            measured: None,
            instrument,
            next_frame: 0,
            applied_revision: None,
            meter: Meter::default(),
            assignments: Navigation::INITIAL.assignments,
        }
    }

    // Keep the DSP call frame out of the RTICX dispatcher's frame.
    #[inline(never)]
    fn render_graph(&mut self, first_frame: u64, block: &mut RenderBlock) {
        let settings = &block.settings;
        let nav = &settings.nav;
        let graph = &mut *self.instrument;
        let skipped = first_frame.wrapping_sub(self.next_frame);
        if skipped != 0 {
            // CPU0 stops an unresponsive worker after one second.
            let frames = usize::try_from(skipped).expect("one outstanding render slot");
            graph
                .skip_frames(frames, |c, cc, value| {
                    if block.usb_session.is_some() {
                        block.midi.remember_cc(c, cc, value);
                    }
                })
                .expect("bounded recovery events");
        }
        self.next_frame = first_frame.wrapping_add(OUTPUT_FRAMES as u64);
        if graph.half_gain != block.half_gain {
            graph
                .set_half_gain(block.half_gain)
                .expect("routes checked at boot");
        }
        if self.applied_revision != Some(block.settings_revision) {
            // A snapshot revision covers the retained settings and arp chords.
            // Recovery advances the player but does not modify these controls.
            Self::apply_settings(graph, block);
            self.applied_revision = Some(block.settings_revision);
        }
        graph.cue = block.cue;
        if let Some(session) = block.usb_session {
            block.midi.begin(session);
        }
        graph
            .render_with_midi(&settings.tracks, block.buses, |channel, event| {
                if block.usb_session.is_some() {
                    block.midi.push(channel, event);
                }
            })
            .expect("fixed patch");
        let i = nav.focus.index();
        if block.capture_source {
            nav.focus
                .copy_scope(nav.part(), nav.view, block.buses, &mut block.source);
        }
        block.bass_cycle = graph.bass_cycles[i];
        for i in 0..CHANNELS {
            if self.assignments[i] != nav.assignments[i] {
                self.meter.clear(i);
            }
        }
        self.assignments = nav.assignments;
        let peaks = if nav.view.is_mixer() {
            self.meter
                .update(settings.mixer, &block.buses[STRIP_BUS..][..CHANNELS])
        } else {
            self.meter = Meter::default();
            [0; CHANNELS]
        };
        block.motion = Motion {
            clipboard: graph.recorder.status,
            position: graph.position,
            slicer_step: graph.slicer_step(),
            transport: Some(settings.seq.transport),
            banks: graph.banks(),
            bass_automation: graph.bass_automation(nav.focus),
            peaks,
            reduction: graph.reduction,
            drums: graph.drums,
            wave_frames: graph.wave_frames,
            arp_notes: Channel::ALL.map(|c| graph.arp_note(c)),
        };
    }

    // Keep unchanged blocks out of the control copies and pattern selection.
    // A separate call also keeps their temporaries out of the hot render frame.
    #[inline(never)]
    fn apply_settings(graph: &mut Instrument<'_>, block: &RenderBlock) {
        let settings = &block.settings;
        let nav = &settings.nav;
        for channel in Channel::ALL {
            let i = channel.index();
            graph.sync_chord(channel, &settings.tracks[i], &block.chords[i]);
        }
        graph.set_mixer(settings.mixer);
        graph.set_fx(settings.fx);
        graph.beat_steps = nav.beat_steps;
        graph.slicer_steps = nav.slicer_steps;
        graph.transport = settings.seq.transport;
        graph.recording_request = nav.clip_request;
        graph.sequences = settings.seq.enabled;
        graph.select_patterns(Channel::ALL.map(|c| {
            settings
                .seq
                .slot(c, settings.tracks[c.index()].assignment.part)
        }));
    }
}

impl Settings {
    pub(super) fn source_tap(&self) -> bool {
        self.nav.view == View::Fx
            || (matches!(self.nav.view, View::Scope | View::SampleEdit)
                && self.tracks[self.nav.focus.index()].voice_tap)
    }
}

impl wavegraph_fm1_core::AudioBlock for RenderBlock {
    fn write(&self, out: &mut fm1_audio::OutputBlock) {
        mix_master(self.buses, self.half_gain, out);
    }
}

impl wavegraph_fm1_core::Render for Renderer {
    type Block = RenderBlock;

    fn render(&mut self, first_frame: u64, block: &mut RenderBlock) {
        use fm1_rticx::bsp::time::{Timers, cycles};
        let started = cycles();
        self.render_graph(first_frame, block);
        block.work = cycles().wrapping_sub(started);
        block.stack_free = if self
            .measured
            .is_none_or(|at| started.wrapping_sub(at) >= Timers::CRYSTAL_HZ)
        {
            self.measured = Some(started);
            Some(wavegraph_fm1_core::peer::cpu1_stack_free())
        } else {
            None
        };
        if cfg!(feature = "late-block-probe") && block.hold {
            while self.release.try_recv().is_none() {
                core::hint::spin_loop();
            }
        }
        block.ready_at = cycles();
    }
}
