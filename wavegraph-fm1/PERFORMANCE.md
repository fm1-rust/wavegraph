# 303 real-time repair and remaining DSP headroom

The original repair below is checkpoint `b29af83`. A subsequent exact-DSP
optimization round is documented at the end of this file. Both binaries and
their respective results are retained; the original checkpoint is unchanged.

## What the failure was

The previously delivered `wavegraph-fm1-303-headroom-29f2abb.fwsc` can miss
audio deadlines in the strict emulator. The original package's SHA-256 is
`6b5cb861858225e3cf147817c0b9f8b687cea85d30bf1a73a71b2eb65900c375`.
Its native firmware emits complete zero-filled DMA halves during the failure;
the affected captures do not show PCM clipping. The upper-right indicator is
defined in `src/overload.rs`: amber means a recent deadline miss, and red means
repeated misses within one second. It is not a clipping meter.

A 128-frame block at 44,118 Hz lasts approximately 2.901 ms. Worker render time
alone is not the complete deadline test: CPU0's dispatch and refill work use
part of that interval. The firmware's minimum ready margin measures the actual
remaining time more directly.

## Changes in this repair

* The 303's OD is integrated into its existing output loop. It uses three
  one-pole states and one bounded smooth clip instead of seven states and two
  clips. The 64-sample drive/bypass ramps and exact settled OD=0 bypass remain.
  This is a calibrated musical approximation, not recovered TB-03 DSP.
* Unchanged track controls and unchanged quantized LFO offsets no longer rebuild
  control events. Sequencing, live events, modulation clocks and MIDI still
  advance every block. The previous-track snapshot is copied only on change.
* The pattern player is borrowed instead of copied out and back each block.
* CPU1 uses the existing settings revision to apply retained mixer, effects,
  pattern, chord and transport settings only when they change. Recovery,
  half-gain, cues, USB sessions, MIDI collection and metering remain per block.
* Sidechain compression processes the existing strip buffer in place. It no
  longer copies each second-slot input into 32-frame scratch spans or constructs
  four separate processing contexts for a 128-frame block. Its integer
  arithmetic, self-keying, external keying and bypass transition are unchanged.
* Five independent scalar caches retain the 303's glide, envelope-decay,
  attack, accent-drain and post-filter coupling coefficients. They add 40 bytes
  per voice and preserve the original calculation and rounding. Each relevant
  public parameter is checked at every span boundary. Dynamic ladder cutoff
  updates and the existing 2x ladder processing rate are unchanged.
* Cold instrument construction and released-voice tail handling have separate
  call frames. They no longer enlarge every active voice render frame.

Pump Panel remains preset 3. Its stored controls are now
`[18, 86, 36, 45, 120, 5, 92, 24]`: saw, 500 Hz cutoff, 86% resonance,
three-octave envelope, 450 ms decay, 92% accent and 67% OD. Channel saturation,
kick-keyed sidechain and the preset's gated Echo settings remain enabled.

## Matched emulator measurements

Both original and repaired firmware use the corrected strict emulator and
the same fixed 32-instruction stepping policy. The 7-second cases start
playback at 1.8 seconds; the Pump case selects preset 3 at 2.5 seconds.
Each case includes both 303 channels and the existing drum parts.

| Workload | Image | Worst worker render | Minimum ready margin | Silent DMA halves | Discarded blocks |
| --- | --- | ---: | ---: | ---: | ---: |
| Default presets, 7 s | Original | 2919.29 us | -164.00 us | 28 | 14 |
| Default presets, 7 s | Repaired | 2511.38 us | +160.42 us | 0 | 0 |
| Pump Panel, 7 s | Original | 2918.21 us | -163.04 us | 28 | 14 |
| Pump Panel, 7 s | Repaired | 2511.67 us | +160.13 us | 0 | 0 |
| Presets and live controls, 22 s | Repaired | 2541.79 us | +80.13 us | 0 | 0 |
| Transport, arp/latch and keys, 7 s | Repaired | 2448.21 us | +188.13 us | 0 | 0 |

The 22-second case exercises every preset, both 303 channels on Pump Panel,
waveform changes and live knob edits. Every accepted replay has zero firmware
DMA overruns, zero emulator clock ticks discarded, and no clipped, repeated or
unexpected zero DMA halves during the analyzed musical passage. The original
Pump capture contains a longest zero-filled gap of 1024 frames, about 23.2 ms.

Worst render time in the matched Pump case improves by approximately 13.9%.
This is a workload-specific render-time comparison, not a claim of a 13.9%
reduction in all CPU activity. The final stress case's 80 us reserve is modest.

The extracted final FWSC package also passes a separate coarse-slice Pump
replay: no silences/discards/overruns, 2441.29 us worst worker time and
178.79 us minimum ready margin. Different inter-core stepping policies can
change cache timing; this separate sensitivity result is not mixed into the
matched comparison above.

Strict mode uses measured instruction costs, a shared flash-cache model,
timed LCD traffic and both emulated cores. It reproduces the original fault,
but it is not a cycle-perfect hardware qualification. These tests establish
improvement under their specified workloads. Actual-device playback remains
the final timing confirmation, particularly with simultaneous USB traffic or
more demanding user patches.

## Validation and limits

The 303, effects and FM-1 suites pass 226 tests, along with formatting and
Clippy checks. The cache regression compares exact float samples and oscillator
wrap positions across 50,580 samples at three sample rates with direct parameter
edits, resonance modulation, accents, slides, release, reset and idle restart.
The track-control regression compares complete PCM, MIDI and ordered graph
events across presets and all instrument types.

The in-place compressor and cold-tail extraction produce byte-identical
7-second and 22-second emulator audio. Adding the coefficient and settings
caches also preserves all musical PCM from playback at 1.8 seconds onward,
including transport, arp/latch and physical-key changes. The runner's UART MIDI
injection does not reach this firmware's USB-MIDI input, so the replay does not
validate USB-MIDI ingress. Host graph tests separately cover MIDI event behavior.
Faster startup moves the identical boot
cue by one 128-frame block; it is the only difference in those full captures.

The native firmware builds and the actual installer simulation validates
layout, CRCs, exact app/assets installation, idempotent reinstallation and
preservation of unrelated flash sectors. The app is 466,492 bytes; assets are
396,611 bytes; installation ends at 0xd7000, leaving 36,864 whole-sector bytes.
The static SRAM-to-stack guard is 1,664 bytes against the unchanged 1,024-byte
linker requirement. Observed free worker stack increases from 1,676 bytes in
the original Pump replay to 3,288 bytes in the final stress replay. Stack and
flash capacity are separate from the audio timing margin.

The final FWSC SHA-256 is
`655af6194c2ff1ea02537683b2bbab8dc1ab6ddf327267db6fd5dbb774d88933`.
The reproduction bundle records the runner, scripts, emulator source/patch,
image hashes, native addresses and raw timing summaries. It also contains the
original Pump Panel demo's render harness and measurement notes.

## Candidates identified at the first checkpoint

1. **Specialize settled saturator spans.** `wavegraph-fx/src/insert.rs` computes
   soft and hard curves and blends them in the per-sample `Saturator::tick`
   path. Once ramps settle, a block path can select only the required curve,
   prepare fixed gains once, and omit unnecessary fully-dry/fully-wet blending.
   Default presets actually exercise this path. Preserve signed integer
   rounding and exact transition state; compare complete output samples.
2. **Continue measured code-locality work.** `Renderer::render_graph`,
   `Node::process` and `Track::direct_events` still contain large instrument or
   control dispatch bodies. PC and shared-cache profiles should identify cold
   reassignment/recovery arms to separate. Blanket inlining is not reliable:
   an earlier candidate reduced some average work but worsened deadline misses.
   SRAM capacity and register spills must be checked together with timing.
3. **Propagate proven silence and settled bypass.** The delay, 303 and drum
   voices already have useful idle paths; the reverb still runs its combs and
   diffusers on initially empty buffers. Track whether internal delay state is
   truly silent. Muting an output is insufficient because tails must evolve
   correctly before unmuting. A sub-PCM cutoff would be a deliberate behavior
   change, not automatically an exact optimization.
4. **Apply coefficient caching selectively elsewhere.** Repeated pitch, filter
   and modulation setup in other instruments may offer similar savings. Cache
   the smallest complete set of dependencies and preserve event boundaries.
   Avoid reducing dynamic filter update rates merely to save work.
5. **Measure fused native Q23 gain/mix kernels.** The final hardware mixer already
   uses `fm1_audio::mix::planar_q23` with native DSP enabled. More opportunity may
   exist in channel send mixing and insert/master arithmetic. Preserve wide
   intermediate sums, signed rounding and final-only clipping; saturating each
   partial sum would change cancellation. Merely replacing the graph's generic
   backend would do little for this routing program, which is mostly node calls.

These are candidates for measurement, not promised speedups. Approximate
nonlinear tables, lower oversampling and altered filter topology have a larger
sound tradeoff and are not the next step while exact setup/copy/kernel savings
remain available.

## Follow-up: settled saturation and the ordinary ladder path

Two additional optimizations preserve the signal-processing calculations:

* `Saturator::process_in_place` runs the original scalar kernel through any
  blend/shape transition, then selects a settled soft, hard or interpolated
  curve. Fully wet output avoids redundant interpolation; settled bypass
  returns immediately. Fixed drive and output level are prepared once per
  span. The public `tick` and generic sample-offset `Process` implementation
  remain unchanged as independent references.
* The 303 ladder checks positive stage drive and finite reciprocal once per
  internal step, then runs the same four scaled nonlinear stages. Zero,
  negative and tiny raw-API drives retain their original arithmetic in a
  separate helper. `fast_tanh`, the feedback solve, filter coefficients and
  the two-times filter rate are unchanged.

### Matched results for this round

All runs use the same corrected strict emulator and fixed32 stepping as the
checkpoint. There are zero firmware silences, discards and service overruns,
and zero discarded emulator clock ticks, in each completed candidate run.

| Workload | Checkpoint worst worker | Follow-up worst worker | Checkpoint minimum ready margin | Follow-up minimum ready margin |
| --- | ---: | ---: | ---: | ---: |
| Default presets, 7 s | 2511.38 us | 2314.71 us | +160.42 us | +350.54 us |
| Pump Panel, 7 s | 2511.67 us | 2313.50 us | +160.13 us | +350.42 us |
| Presets and live controls, 22 s | 2541.79 us | 2421.25 us | +80.13 us | +178.92 us |

The settled saturator alone improved regular playback but left the limiting
12.518-second job essentially unchanged. The ladder specialization reduces
that same job from 2523.625 us in the saturator-only candidate to 2421.250 us.
This is why the final decision uses the stress-case margin as well as the
shorter playback tests.

Relative to the original failing firmware, matched Pump worst worker time
falls from 2918.21 to 2313.50 us, approximately 20.7%. Relative to the first
repair, the minimum measured stress reserve increases from 80.125 to
178.917 us. These remain workload-specific emulator measurements, not a
guarantee for arbitrary patches or physical hardware.

The extracted follow-up FWSC passes the separate large-slice Pump check:
2320.625 us worst worker, 318.333 us minimum ready margin, zero silence,
discard, overrun and dropped-emulator-tick counts, with no clipped, repeated
or unexpected zero DMA buffers.

### Sound and control timing verification

All 229 host tests pass: 32 in the 303, 40 in effects and 157 in FM-1. Clippy
for all three packages and all targets passes with warnings denied. The
saturator differential checks compare 1,839,056 sample positions against its
original scalar implementation, including interrupted ramps, every ramp
offset, arbitrary partitions, extreme controls and Q23 rails. The ladder
oracle compares 116,736 output samples and all five changed state values,
including raw bypass and reciprocal-overflow cases at three sample rates.

The complete default and Pump emulator WAVs are byte-identical to the first
repair. The long bank replay is not entirely identical: the faster worker
accepts the Acid Pluck recall one 128-frame block earlier. Read-only native
traces establish the same settings revision and identical insert-control
bytes at audio frame 485120 in the follow-up versus 485248 in the preceding
candidate. The difference persists through the Phaser and its shared-effect
tail; it is not evidence of changed filter arithmetic. No full-bank PCM
identity claim is made. USB-MIDI ingress remains outside the validated scope.

The target 303 renderer shrinks from 5602 to 5332 bytes; its local stack
reservation falls from 336 to 240 bytes, with the same register-save area.
The overall observed worker-stack minimum remains 3288 free bytes because
another call path sets that watermark. Static SRAM guard improves from
1664 to 1920 bytes. Native app size is 467532 bytes, assets are 396611 bytes,
installation ends at 0xd8000, and 32768 whole-sector bytes remain. The actual
installer simulation passes again.

Follow-up FWSC SHA-256:
`c64784b6cc34ed268cd406ea8f28880d8c3ac8053ca3b7edde269546662997de`.

### Checking the Super Saw comparison

A separate read-only experiment uses the first repair with the same Arp
display, stopped transport, all faders64, zero sends, all eight insert slots
empty, master effects off, no Slicer or Beat Flanger, and master900. RAM
inspection confirms sixteen active sustained Super Saw voices across four
instances, each at velocity121/127. The two-303 comparison has two open gates
at notes59/65 and retains their internal OD at50% and44.4%. Thus only external
effects are disabled; the 303 OD is included.

| Current-checkpoint workload | Worst worker | Minimum ready margin | Silence halves / discards | Settled median worker, 7–8.25 s |
| --- | ---: | ---: | ---: | ---: |
| Four Super Saw instances, sixteen notes | 2888.17 us | -136.54 us | 2 / 1 | 2619.67 us |
| Two 303 instances, internal OD on | 1739.79 us | +990.33 us | 0 / 0 | 1536.79 us |

The sixteen-voice case has one onset miss; it sustains the later held-note
interval without further misses. This does not reconstruct an earlier user
patch or prove a physical-device limit. The current build also omits the
optional Super Saw `pi32v2-simd` feature. The useful finding is that two 303s
with OD are comfortably below this model's deadline when the rest of the
demo workload is removed. Voice counts alone do not explain whole-patch cost.

Super Saw uses seven integer ramps per note, linear filtering, a folded
halfband decimator and shared modulation for matching voice states. The 303
instead performs up to ten rational nonlinear evaluations per output sample
inside its oversampled ladder. This explains differing per-voice costs
without implying that OD itself must be expensive.

### Next exact opportunities, not yet implemented

Measure cache hit rates before adding a base-cutoff logarithm cache, a fixed
pitch-increment cache, or an input-gain exponential cache. Identical clamped
ladder cutoff targets could also skip repeated prewarps, provided feedback is
still updated when resonance changes. Retain the current arithmetic and
control boundaries and validate forced recomputation against cached output.
The optional native Super Saw ramp kernel is another measured-build candidate;
its timing and emulator instruction coverage should be checked separately.
