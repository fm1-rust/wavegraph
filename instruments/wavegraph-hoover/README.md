# wavegraph-hoover

A four-voice, allocation-free Alpha Juno-style instrument. The starting tone pairs a pulse-width-modulated pulse and a chopped PWM saw with a 25% pulse sub two octaves down, a little noise, a bass-boost/high-pass stage, a resonant four-pole low-pass and short ensemble delays. One envelope moves pitch, filter and amplitude together. T1, T2 and T3 reach independent levels L1, L2 and L3. L3 holds until key-up. T4 releases from the instantaneous level, even during the initial sweep.

The [What the patch readout](https://llamamusic.com/mks50/Alpha_HS_APR_Patch_Sample_Printout.PDF) gives levels 68/127/127. The default retains these and shortens the times to roughly 4/42/214/48 ms. This is a performance patch, with approximate digital oscillator, filter and delay models. The [MKS-50 service notes](https://www.kiwitechnics.com/downloads/Kiwi-2/Roland%20MKS-50%20Service%20Manual.pdf) show parallel chorus paths. Here two oppositely modulated fractional taps share a band-limited delay line. The stereo API exposes both returns. The mono Process outlet folds them for WaveGraph's mixer strips.

On the FM-1 choose HOOVER on any channel. Knobs 1–4 edit cutoff, pitch sweep, PWM depth and chorus rate. Turn chorus to zero for the dry sound. On ENV, turn or press SELECT to cycle through TIME, LVL, DCO and MOD.

In his [account of recording Mentasm](https://web03125.microsites03.redbull.com/2014/05/key-tracks-mundo-muzique-on-mentasm/), Mundo Muzique describes a pitch-bent MIDI performance, sampling the complete riff into a Casio FZ-1, and changing the sampler filter and LFO. The interview gives no session envelope values. The short times here are WaveGraph settings.

ALGORITHM recalls three programs. Fast Hoover is the short default. What The restores the documented factory time and level numbers on WaveGraph's approximate time scale. Twin Peak is a softer polysynth patch whose shared contour opens, dips and swells again under a held chord.

Boochow's [Hoover oscillator walkthrough](https://blog.boochow.com/article/hoover-for-nts1.html) documents the pulse, chopped saw and quarter-duty sub. The GPL [JC-303](https://midilab.co/docs/dsp/jc303/) also offers an Alpha Juno oscillator. The WaveGraph signal path is an independent implementation.

The DCO page selects the pulse, saw and sub waveform and the sub level. The level page also exposes resonance on knob 4. MOD sets the four-position high-pass, filter envelope depth, PWM rate and noise level. At PWM rate zero the depth control sets a static width. These controls follow the waveform and HPF diagrams in the [Roland manual, pp. 20–22](https://cdn.roland.com/assets/media/pdf/JUNO-1_OM.pdf). The scope emblem is a 16 × 32 reduction of the public-domain [upright vacuum cleaner by Anonymous](https://openclipart.org/detail/13321/vacuum-cleaner-upright).
