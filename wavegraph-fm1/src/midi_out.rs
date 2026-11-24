//! A bounded USB-MIDI output queue shared by the audio producer and USB owner.

use crate::{
    bass::{self, CC_SLIDE},
    panel::{CHANNELS, Channel},
};
use heapless::{Deque, Vec};
use wavegraph::{Event, midi::MidiMessage};

type Packet = [u8; 4];

const BLOCK_PACKETS: usize = 96;
const LATCHES: usize = bass::AUTOMATION_COUNT + 1;
const QUEUE_PACKETS: usize = BLOCK_PACKETS + CHANNELS * (2 + LATCHES);

fn latched_ccs() -> impl Iterator<Item = u8> {
    bass::automation_ccs().chain([CC_SLIDE])
}

#[derive(Clone, Copy)]
struct Latches([[u8; LATCHES]; CHANNELS]);

impl Default for Latches {
    fn default() -> Self {
        Self([[u8::MAX; LATCHES]; CHANNELS])
    }
}

impl Latches {
    fn set(&mut self, channel: Channel, cc: u8, value: u8) {
        let lane = &mut self.0[channel.index()];
        if cc == 121 {
            lane.fill(u8::MAX); // Reset All Controllers
        } else if let Some(i) = latched_ccs().position(|n| n == cc) {
            lane[i] = value;
        }
    }

    fn packets(&self) -> impl Iterator<Item = Packet> {
        Channel::ALL
            .into_iter()
            .zip(&self.0)
            .flat_map(|(ch, lane)| {
                latched_ccs().zip(lane).filter_map(move |(cc, &value)| {
                    (value < 128).then_some([11, 0xb0 | ch as u8, cc, value])
                })
            })
    }
}

#[derive(Default)]
pub(super) struct Batch {
    // One frame offset and three MIDI bytes per event in the cross-core slot.
    packets: Vec<(u8, [u8; 3]), BLOCK_PACKETS>,
    lost: bool,
    session: Option<u32>,
    at_start: Latches,
    current: Latches,
}

impl Batch {
    /// CPU1 owns this slot across renders, including blocks the DAC discards.
    pub(super) fn begin(&mut self, session: u32) {
        if self.session.replace(session) != Some(session) {
            self.current = Latches::default();
        }
        self.at_start = self.current;
        self.packets.clear();
        self.lost = false;
    }

    /// Update the receiver state to restore, without enqueueing a packet.
    pub(super) fn remember_cc(&mut self, channel: Channel, cc: u8, value: u8) {
        self.current.set(channel, cc, value);
    }

    pub(super) fn push(&mut self, channel: Channel, event: Event<MidiMessage>) {
        if let Some(bytes) = event.message.encode(channel as u8) {
            if let MidiMessage::Control(cc, value) = event.message {
                self.remember_cc(channel, cc, value);
            }
            // Clip and arp producers each emit in frame order. USB drains one
            // merged batch, retaining insertion order for pairs at the same frame.
            let frame = u8::try_from(event.frame).expect("event in a 128-frame block");
            let i = self.packets.partition_point(|(at, _)| *at <= frame);
            self.lost |= self.packets.insert(i, (frame, bytes)).is_err();
        }
    }
}

/// Session-scoped queue for the RTICX USB task.
#[derive(Debug, Default)]
pub struct OutputQueue {
    session: Option<u32>,
    revision: u32,
    chase: bool,
    queue: Deque<Packet, QUEUE_PACKETS>,
}

impl OutputQueue {
    pub(super) fn session(&self) -> Option<u32> {
        self.session
    }

    pub(super) fn connect(&mut self, active: bool) {
        self.retire();
        self.session = active.then_some(self.revision);
        self.chase = false;
    }

    fn retire(&mut self) {
        self.queue.clear();
        self.revision = self.revision.wrapping_add(1);
    }

    /// Retire partial writes and release every channel on the current USB session.
    pub(super) fn silence(&mut self) -> bool {
        if self.session.is_none() {
            return false;
        }
        self.retire();
        self.chase = true;
        for channel in Channel::ALL {
            for cc in [123, CC_SLIDE] {
                self.queue
                    .push_back([11, 0xb0 | channel as u8, cc, 0])
                    .expect("four channels");
            }
        }
        true
    }

    pub(super) fn append(&mut self, batch: &Batch) -> bool {
        if self.session.is_none() || self.session != batch.session {
            return false;
        }
        if batch.lost {
            return self.silence();
        }
        if self.queue.capacity().saturating_sub(self.queue.len()) < batch.packets.len() {
            self.silence();
        }
        let recovering = core::mem::take(&mut self.chase);
        if recovering {
            // At most two releases and LATCHES controls per channel precede the block.
            for packet in batch.at_start.packets() {
                self.queue.push_back(packet).expect("reserved for recovery");
            }
        }
        for &(_, [status, key, value]) in &batch.packets {
            self.queue
                .push_back([status >> 4, status, key, value])
                .expect("reserved above");
        }
        recovering || !batch.packets.is_empty()
    }

    pub(super) fn peek(&self, out: &mut [Packet]) -> (u32, usize) {
        let n = out.len().min(self.queue.len());
        for (dst, src) in out.iter_mut().zip(&self.queue) {
            *dst = *src;
        }
        (self.revision, n)
    }

    pub(super) fn sent(&mut self, revision: u32, count: usize) {
        if revision == self.revision {
            for _ in 0..count {
                self.queue.pop_front();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wavegraph::midi::Note;

    fn at(frame: usize, message: MidiMessage) -> Event<MidiMessage> {
        Event {
            frame,
            port: 0,
            message,
        }
    }

    #[test]
    fn independent_arpeggiators_and_clips_merge_in_frame_order() {
        let mut out = OutputQueue::default();
        out.connect(true);
        let mut batch = Batch::default();
        batch.begin(out.session().expect("connected"));
        for (channel, frame, message) in [
            (Channel::One, 48, MidiMessage::Control(74, 120)),
            (Channel::Two, 30, MidiMessage::Off(60)),
            (
                Channel::Two,
                30,
                MidiMessage::On(Note {
                    number: 64,
                    velocity: 87,
                }),
            ),
            (Channel::Four, 4, MidiMessage::Off(42)),
        ] {
            batch.push(channel, at(frame, message));
        }
        out.append(&batch);
        let mut packets = [[0; 4]; 4];
        assert_eq!(out.peek(&mut packets).1, 4);
        assert_eq!(
            packets,
            [
                [8, 0x83, 42, 64],
                [8, 0x81, 60, 64],
                [9, 0x91, 64, 87],
                [11, 0xb0, 74, 120]
            ]
        );
    }

    #[test]
    fn partial_usb_writes_retry_the_suffix_and_retire_stale_sessions() {
        let mut out = OutputQueue::default();
        let mut batch = Batch::default();
        assert!(!out.append(&batch));
        out.connect(true);
        batch.begin(out.session().expect("connected"));
        batch.push(
            Channel::Two,
            at(
                0,
                MidiMessage::On(Note {
                    number: 45,
                    velocity: 100,
                }),
            ),
        );
        batch.push(Channel::Two, at(0, MidiMessage::Control(74, 89)));
        batch.push(Channel::Two, at(0, MidiMessage::Off(45)));
        let mut packets = [[0; 4]; 16];
        out.append(&batch);
        let (revision, n) = out.peek(&mut packets);
        assert_eq!(
            &packets[..n],
            &[[9, 0x91, 45, 100], [11, 0xb1, 74, 89], [8, 0x81, 45, 64]]
        );
        out.sent(revision, 1);
        assert_eq!(out.peek(&mut packets).1, 2);
        assert_eq!(packets[0], [11, 0xb1, 74, 89]);
        out.connect(false);
        assert!(out.session().is_none());
        out.connect(true);
        assert!(!out.append(&batch));
        batch.begin(out.session().expect("reconnected"));
        batch.push(Channel::Four, at(0, MidiMessage::Off(51)));
        out.append(&batch);
        out.sent(revision, 2);
        assert_eq!(out.peek(&mut packets).1, 1);
        assert_eq!(packets[0], [8, 0x83, 51, 64]);
    }

    #[test]
    fn silence_retires_a_partial_usb_batch_and_releases_an_already_delivered_note() {
        let mut out = OutputQueue::default();
        assert!(!out.silence());
        out.connect(true);
        let mut batch = Batch::default();
        batch.begin(out.session().expect("connected"));
        batch.push(
            Channel::One,
            at(
                0,
                MidiMessage::On(Note {
                    number: 60,
                    velocity: 90,
                }),
            ),
        );
        batch.push(Channel::One, at(12, MidiMessage::Off(60)));
        assert!(out.append(&batch));
        let mut packets = [[0; 4]; 8];
        let (old, count) = out.peek(&mut packets);
        assert_eq!(count, 2);
        assert_eq!(packets[0], [9, 0x90, 60, 90]);
        out.sent(old, 1);
        // USB has peeked the unsent suffix when CPU0 declares the fault.
        assert_eq!(out.peek(&mut packets), (old, 1));
        assert!(out.silence());
        out.sent(old, 1);
        let (current, count) = out.peek(&mut packets);
        assert_ne!(current, old);
        assert_eq!(count, 8);
        for (pair, channel) in packets.as_chunks::<2>().0.iter().zip(Channel::ALL) {
            assert_eq!(
                *pair,
                [
                    [11, 0xb0 | channel as u8, 123, 0],
                    [11, 0xb0 | channel as u8, CC_SLIDE, 0]
                ]
            );
        }
    }

    #[test]
    fn backpressure_releases_every_channel_before_sending_new_notes() {
        let mut out = OutputQueue::default();
        out.connect(true);
        let mut batch = Batch::default();
        batch.begin(out.session().expect("connected"));
        for _ in 0..80 {
            batch.push(Channel::One, at(0, MidiMessage::Control(74, 50)));
        }
        out.append(&batch);
        out.append(&batch);
        let mut packets = [[0; 4]; 9];
        assert_eq!(out.peek(&mut packets).1, 9);
        for (i, c) in Channel::ALL.iter().enumerate() {
            assert_eq!(packets[i * 2], [11, 0xb0 | *c as u8, 123, 0]);
            assert_eq!(packets[i * 2 + 1], [11, 0xb0 | *c as u8, CC_SLIDE, 0]);
        }
        assert_eq!(packets[8], [11, 0xb0, 74, 50]);
    }

    #[test]
    fn first_accepted_block_restores_the_factory_cutoff_after_an_ambiguous_refill() {
        use crate::{
            instrument::Track,
            panel::Navigation,
            sequencer::{PatternPlayer, Transport},
        };
        use fm1_audio::{FRAME_RATE, OUTPUT_FRAMES};

        let mut player = PatternPlayer::new(FRAME_RATE);
        let tracks = Track::rack(Navigation::INITIAL.assignments);
        let mut transport = Transport::INITIAL;
        transport.toggle();
        let mut out = OutputQueue::default();
        out.connect(true);
        let session = out.session().expect("connected");
        let mut batch = Batch::default();
        let mut receiver = None;
        let mut dropped = false;
        for _ in 0..1000 {
            batch.begin(session);
            player
                .render(
                    OUTPUT_FRAMES,
                    transport,
                    &tracks,
                    [true; CHANNELS],
                    |_, _| Ok(()),
                    |c, e| batch.push(c, e),
                )
                .expect("factory clip");
            let cutoff = player.automation(Channel::One).and_then(|a| a.get(74));
            if !dropped && receiver == Some(28) && cutoff == Some(29) {
                // An uncertain interval rejects this job without skipping any DSP frames.
                out.silence();
                dropped = true;
                continue;
            }
            if dropped {
                assert!(!batch.packets.iter().any(|(_, p)| p[..2] == [0xb0, 74]));
            }
            out.append(&batch);
            let mut packets = [[0; 4]; QUEUE_PACKETS];
            let (revision, n) = out.peek(&mut packets);
            for &[cin, status, cc, value] in &packets[..n] {
                if (cin, status, cc) == (11, 0xb0, 74) {
                    receiver = Some(value);
                }
            }
            out.sent(revision, n);
            if dropped {
                break;
            }
        }
        assert!(dropped, "factory cutoff advanced from 28 to 29");
        assert_eq!(receiver, Some(29));
    }

    #[test]
    fn repeated_drops_resume_with_controls_before_notes_and_isolate_reconnects() {
        let mut out = OutputQueue::default();
        out.connect(true);
        let session = out.session().expect("connected");
        let mut batch = Batch::default();
        batch.begin(session);
        batch.push(Channel::Two, at(0, MidiMessage::Control(74, 28)));
        batch.push(
            Channel::Two,
            at(
                1,
                MidiMessage::On(Note {
                    number: 40,
                    velocity: 90,
                }),
            ),
        );
        out.silence();
        // A second discarded render changes two latches. Its attacks stay discarded.
        batch.begin(session);
        batch.push(Channel::Two, at(0, MidiMessage::Control(CC_SLIDE, 127)));
        batch.push(Channel::Two, at(3, MidiMessage::Control(74, 29)));
        let mut packets = [[0; 4]; 16];
        let (rev, n) = out.peek(&mut packets);
        assert_eq!(n, CHANNELS * 2);
        out.sent(rev, n);
        batch.begin(session);
        batch.push(
            Channel::Two,
            at(
                0,
                MidiMessage::On(Note {
                    number: 48,
                    velocity: 90,
                }),
            ),
        );
        batch.push(Channel::Two, at(7, MidiMessage::Control(74, 31)));
        assert!(out.append(&batch));
        let (rev, n) = out.peek(&mut packets);
        assert_eq!(
            &packets[..n],
            &[
                [11, 0xb1, 74, 29],
                [11, 0xb1, CC_SLIDE, 127],
                [9, 0x91, 48, 90],
                [11, 0xb1, 74, 31],
            ]
        );
        out.sent(rev, n);
        batch.begin(session);
        assert!(!out.append(&batch));
        out.connect(false);
        out.connect(true);
        out.silence();
        assert!(!out.append(&batch));
        batch.begin(out.session().expect("new session"));
        out.append(&batch);
        assert_eq!(out.peek(&mut packets).1, CHANNELS * 2);
    }
}
