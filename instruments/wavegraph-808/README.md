# wavegraph-808

An allocation-free, synthesized four-part rhythm section for WaveGraph. The
kick rings a damped resonator with a short pitch transient. The snare blends
two resonances and filtered noise. Both hats gate the same six free-running
square waves. These are original, lightweight voice models.

| Voice | MIDI note | Kit outlet |
| --- | ---: | ---: |
| Kick | 36 | 1 |
| Snare | 38 | 2 |
| Closed hi-hat | 42 | 3 |
| Open hi-hat | 46 | 4 |

Outlet 0 carries the mono mix. The four stems and the mix come from the same
render. To insert an effect on a child, route its stem through the effect and
sum the desired stems. A meter or sidechain can tap a stem alongside the mix.

Construct `Tr808::new(numerator, denominator)` with the graph clock (for the
FM-1, 24_000_000 / 544 Hz). Send `Message::Strike` to inlet 0, or translate
MIDI note-ons with `Message::note_on`. Velocity zero and note-offs leave
one-shot tails ringing. `Message::Accent` sets the shared excitation boost for
subsequent hits. Set it before strikes on an accented step. A closed hat
chokes a ringing open hat. Both struck on the same frame give a short open
hat in either event order.

`BassDrum`, `SnareDrum` and `HiHat` also implement `Process<u8, S>`,
where the message is strike velocity. Connect a `Metal` signal to inlet 1
of each standalone hat. Share one source to reproduce the kit wiring.
Kick Tone and Decay, snare Tone and Snappy, and individual voice levels
accept values from 0 to 127.
