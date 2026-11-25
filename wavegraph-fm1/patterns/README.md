# Factory MIDI clips

The upper 303's **01** clip adds Saturator Shape on beat two of bar four.
Local CC 20 rises on the B–E slide and returns to zero before the final G.
A value of 0 follows knob 4, and 127 adds the full Shape range. This
controller acts on a Saturator in slot 1 of that strip and stays local to
the FM-1. Notes, slides and synth CCs still go to USB MIDI.

Sampler pattern 01 holds C4 for the full four bars. The default AMEN BRK plays in time with the transport. At C4 the full take lasts sixteen
beats at the current tempo (122 BPM by default). The other seven
sampler patterns play D-minor phrases against the factory 303s.

Selecting MIN7 STAB recalls the eight companion clips in `house-m7/`. Pattern 01
answers the stock bassline with syncopated Dm7 and Am7 stabs. Its layered D/A
hit adds the ninth and eleventh. Patterns 02–08 use D, G and A roots so all
four recorded chord tones stay in D natural minor. A running lane switches
banks on the next bar. Panel keys, MIDI and recorded edits remain chromatic.

Slots 05–08 of the other banks adapt the **Acid Trip** and **Devil Fish** demo songs from
**AfterBirth AB-3389**, copyright © 2026 verytired, under the MIT license.
The editable MIDI files carry the credit and license in their copyright
metadata. The firmware includes the same notice in upper flash. [Notice](NOTICE.txt) · [AfterBirth source revision][afterbirth]

Each entry below is an AfterBirth pattern number. In slot 07 the arrow marks
the change from bar one to bar two.

| WaveGraph slot | Song | Low 303 | High 303 | 808 | 909 |
| --- | --- | --- | --- | --- | --- |
| 05 | Acid Trip | D1 | D1 | D1 | D2 |
| 06 | Acid Trip | D2 | D2 | D2 | D3 |
| 07 | Acid Trip | D5 → D3 | D3 → D1 | D1 → D2 | D4 → D5 |
| 08 | Devil Fish | B1 with B2 on open hats | B1 | B2 | B1 |

Both 303s move up two semitones. An upstream accent becomes MIDI velocity 115.
A slide joins the current step to the following sounding step if they are
adjacent, with CC 102 on the destination and a one-tick note overlap for
different pitches. In **Devil Fish**, AfterBirth's dynamic bank switching
reads B2 on steps 3, 7, 11 and 15 of the low 303. Slot 08 bakes those notes
into the clip so it can play independently of the selected 909 pattern.
[Acid Trip score][acid] · [Devil Fish score][devil]

The FM-1 plays the 808 clap rhythm on its snare, with softer ghost hits for
the separate snare line. The 909 uses a quiet snare for the rimshot. The
monosynth picks notes from the low line, the Super Saw extends the upper
answer an octave higher, and the organ stabs and wavetable pads are WaveGraph
arrangements in D minor. The master transport sets the tempo. The 303 knobs
remain playable throughout these clips.

[afterbirth]: https://github.com/verytired/afterbirth/tree/7ea95d9676a757d240378e504446624c9d671cab
[acid]: https://github.com/verytired/afterbirth/blob/7ea95d9676a757d240378e504446624c9d671cab/scripts/acid-trip.mjs
[devil]: https://github.com/verytired/afterbirth/blob/7ea95d9676a757d240378e504446624c9d671cab/scripts/devil-fish.mjs
