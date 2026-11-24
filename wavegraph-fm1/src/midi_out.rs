//! A bounded USB-MIDI output queue shared by the audio producer and USB owner.

use crate::{bass::CC_SLIDE, panel::Channel};
use heapless::{Deque, Vec};
use wavegraph::{Event, midi::MidiMessage};

type Packet = [u8; 4];

#[derive(Default)]
pub(super) struct Batch {
    packets: Vec<(u8, Packet), 96>,
    lost: bool,
}

impl Batch {
    pub(super) fn push(&mut self, channel: Channel, event: Event<MidiMessage>) {
        if let Some([status, key, value]) = event.message.encode(channel as u8) {
            // Clip and arp producers each emit in frame order. USB drains one
            // merged batch, retaining insertion order for pairs at the same frame.
            let frame = u8::try_from(event.frame).expect("event in a 128-frame block");
            let i = self.packets.partition_point(|(at, _)| *at <= frame);
            self.lost |= self
                .packets
                .insert(i, (frame, [status >> 4, status, key, value]))
                .is_err();
        }
    }

    pub(super) fn clear(&mut self) {
        self.packets.clear();
        self.lost = false;
    }
}

/// Session-scoped queue for the RTICX USB task.
#[derive(Debug, Default)]
pub struct OutputQueue {
    active: bool,
    revision: u32,
    queue: Deque<Packet, 128>,
}

impl OutputQueue {
    pub(super) fn session(&self) -> Option<u32> {
        self.active.then_some(self.revision)
    }

    pub(super) fn connect(&mut self, active: bool) {
        self.active = active;
        self.queue.clear();
        self.revision = self.revision.wrapping_add(1);
    }

    pub(super) fn append(&mut self, batch: &Batch) -> bool {
        if !self.active || (batch.packets.is_empty() && !batch.lost) {
            return false;
        }
        if batch.lost
            || self.queue.capacity().saturating_sub(self.queue.len()) < batch.packets.len()
        {
            self.connect(true);
            // The host may have missed a release. Silence each channel before resuming.
            for channel in Channel::ALL {
                for cc in [123, CC_SLIDE] {
                    self.queue
                        .push_back([11, 0xb0 | channel as u8, cc, 0])
                        .expect("four channels");
                }
            }
        }
        if !batch.lost {
            for &(_, packet) in &batch.packets {
                self.queue.push_back(packet).expect("reserved above");
            }
        }
        true
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
        assert!(out.session().is_none());
        out.append(&batch);
        assert_eq!(out.peek(&mut packets).1, 0);
        out.connect(true);
        assert!(out.session().is_some());
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
        batch.clear();
        batch.push(Channel::Four, at(0, MidiMessage::Off(51)));
        out.append(&batch);
        out.sent(revision, 2);
        assert_eq!(out.peek(&mut packets).1, 1);
        assert_eq!(packets[0], [8, 0x83, 51, 64]);
    }

    #[test]
    fn backpressure_releases_every_channel_before_sending_new_notes() {
        let mut out = OutputQueue::default();
        out.connect(true);
        let mut batch = Batch::default();
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
}
