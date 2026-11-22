# Bundled cymbals

Kevin B. (Fugue State Audio) synthesized these three hits for the `x0xproc1`
kit in the [Stargate sample pack][kit]. His [contributor notes][notes] identify
its drum kits as original synthesis. The pack dedicates the samples to the
public domain under [CC0 1.0][license], including use in other instruments.

| WaveGraph voice | Upstream stereo WAV | Frames at 44.1 kHz | Mono PCM bytes |
| --- | --- | ---: | ---: |
| Closed hi-hat | `hihats/x0xproc1-hatclsd.wav` | 8,476 | 16,952 |
| Open hi-hat | `hihats/x0xproc1-hatopen.wav` | 23,223 | 46,446 |
| Ride | `cymbals/x0xproc1-ride.wav` | 42,363 | 84,726 |

The source WAVs are 24-bit stereo PCM. The conversion averages the channels
and applies a gain of 0.4 to leave room for overlapping hits in the mono mix. It
preserves each complete take at its original rate. To reproduce a clip, run
this command with the corresponding input and output paths.

```sh
ffmpeg -i INPUT.wav -af 'pan=mono|c0=0.2*c0+0.2*c1' -c:a pcm_s16le -f s16le OUTPUT.s16le
```

[kit]: https://github.com/stargatedaw/stargate-sample-pack/tree/dbfd6ec52d4ed53b60bdbea5fc6adf295127c027/stargate-sample-pack/fugue-state-audio/drums
[notes]: https://github.com/stargatedaw/stargate-sample-pack/blob/dbfd6ec52d4ed53b60bdbea5fc6adf295127c027/stargate-sample-pack/fugue-state-audio/readme.md
[license]: https://github.com/stargatedaw/stargate-sample-pack/blob/dbfd6ec52d4ed53b60bdbea5fc6adf295127c027/LICENSE
