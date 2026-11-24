//! The fixed 128-frame DSP job. All mutable graph state belongs to this worker.
use super::{Motion, Performance, Settings};
use crate::{
    bumper::Cue,
    instrument::{BUS_COUNT, Instrument, STRIP_BUS},
    midi_out::Batch,
    mixer::Meter,
    panel::{Assignment, CHANNELS, Channel, Navigation, View},
};
use fm1_audio::{I24, OUTPUT_FRAMES};
use wavegraph_arp::Chord;

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
    pub(super) pcm: [i32; OUTPUT_FRAMES],
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
    pub(super) fn new() -> Self {
        Self {
            settings: Settings::INITIAL,
            settings_revision: 0,
            chords: core::array::from_fn(|_| Chord::new()),
            cue: Cue::EMPTY,
            half_gain: false,
            usb_session: None,
            capture_source: false,
            hold: false,
            pcm: [0; OUTPUT_FRAMES],
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
    buses: &'static mut [[I24; OUTPUT_FRAMES]; BUS_COUNT],
    next_frame: u64,
    meter: Meter,
    assignments: [Assignment; CHANNELS],
}

impl Renderer {
    pub(super) fn new(
        instrument: &'static mut Instrument<'static>,
        buses: &'static mut [[I24; OUTPUT_FRAMES]; BUS_COUNT],
    ) -> Self {
        Self {
            instrument,
            buses,
            next_frame: 0,
            meter: Meter::default(),
            assignments: Navigation::INITIAL.assignments,
        }
    }

    // Keep the DSP call frame out of the RTICX dispatcher's frame.
    #[inline(never)]
    pub(super) fn render(&mut self, first_frame: u64, block: &mut RenderBlock) {
        let settings = &block.settings;
        let nav = settings.nav;
        let graph = &mut *self.instrument;
        let skipped = first_frame.wrapping_sub(self.next_frame);
        if skipped != 0 {
            // CPU0 stops an unresponsive worker after one second.
            let frames = usize::try_from(skipped).expect("one outstanding render slot");
            graph.skip_frames(frames).expect("bounded recovery events");
        }
        self.next_frame = first_frame.wrapping_add(OUTPUT_FRAMES as u64);
        if graph.half_gain != block.half_gain {
            graph
                .set_half_gain(block.half_gain)
                .expect("routes checked at boot");
        }
        for channel in Channel::ALL {
            let i = channel.index();
            graph.sync_chord(channel, &settings.tracks[i], &block.chords[i]);
        }
        graph.set_mixer(settings.mixer);
        graph.set_fx(settings.fx);
        graph.transport = settings.seq.transport;
        graph.sequences = settings.seq.enabled;
        graph.select_patterns(Channel::ALL.map(|c| {
            settings
                .seq
                .slot(c, settings.tracks[c.index()].assignment.part)
        }));
        graph.cue = block.cue;
        if let Some(session) = block.usb_session {
            block.midi.begin(session);
        }
        let mono = graph
            .render_with_midi(&settings.tracks, self.buses, |channel, event| {
                if block.usb_session.is_some() {
                    block.midi.push(channel, event);
                }
            })
            .expect("fixed patch");
        for (out, sample) in block.pcm.iter_mut().zip(mono) {
            *out = sample.inner();
        }
        let i = nav.focus.index();
        if block.capture_source {
            nav.focus
                .copy_scope(nav.part(), nav.view, self.buses, &mut block.source);
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
                .update(settings.mixer, &self.buses[STRIP_BUS..][..CHANNELS])
        } else {
            self.meter = Meter::default();
            [0; CHANNELS]
        };
        block.motion = Motion {
            position: graph.position,
            banks: graph.banks(),
            bass_automation: graph.bass_automation(nav.focus),
            peaks,
            reduction: graph.reduction,
            drums: graph.drums,
            wave_frames: graph.wave_frames,
            arp_notes: Channel::ALL.map(|c| graph.arp_note(c)),
        };
    }
}

impl Settings {
    pub(super) fn source_tap(&self) -> bool {
        self.nav.view == View::Fx
            || (self.nav.view == View::Scope && self.tracks[self.nav.focus.index()].voice_tap)
    }
}
