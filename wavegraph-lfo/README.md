# wavegraph-lfo

An allocation-free, frame-clocked WaveGraph control source. Its Pd-style
message outlet emits a bipolar float at frame zero of each nonempty processing
block. The host sets the frequency in Hz, chooses the shape and maps the float
to a destination and depth. Phase is continuous across rate and shape edits.
Choose a block rate well above the highest modulation frequency.

The control-rate LFO and the audio-rate Flanger share the Q0.32 `Phase`
accumulator. A `PhaseControl` supplies the increment per audio frame and
an optional transport position. Audio processors can read its integer
triangle directly between control messages.
