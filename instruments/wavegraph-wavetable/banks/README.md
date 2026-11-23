# WaveEdit Online banks

WaveEdit Online contributors released these banks under CC0-1.0. All 64 frames
appear in the original order. Each source WAV contains 64 consecutive
256-sample cycles, mono PCM16. [Source collection at the pinned revision][source] ·
[WaveEdit upload terms][terms] · [CC0-1.0](LICENSE-CC0).

| Bank | Source WAV | Source SHA-256 |
| --- | --- | --- |
| Resonant Saw | [RESO_SAW.WAV][saw] | `a248a29eed5cdcd6fedf64adb4af04c59088eca02b4b854d3c3285e35138be61` |
| Vocal Formants | [VOCAL_FO.WAV][vocal] | `72e259f4670c721eb30c439b3b10a35438479c2f2e2b4531d19db2582b9ec519` |

From the crate directory, use Python 3 and NumPy to regenerate either bank.

```sh
python3 tools/import_bank.py RESO_SAW.WAV banks/reso-saw.wt12
python3 tools/import_bank.py VOCAL_FO.WAV banks/vocal-formants.wt12
```

The importer removes DC, trims each frame to 63, 31, 15, 7, 3 and 1 harmonics,
and stores those versions in 256, 128, 64, 32, 32 and 32 samples respectively.
It attenuates the entire bank together if needed to fit the PCM range.
Samples are quantized to signed 12-bit PCM, two per three little-endian bytes
(low 12 bits first). Each frame occupies 816 bytes and each factory bank
occupies 51 KiB of read-only data. Python is only needed to import new banks.
Use `Bank::from_packed12(include_bytes!("my-bank.wt12"))` to borrow a converted
bank from flash.

[source]: https://github.com/smpldsnds/wavedit-online/tree/1a8d80f7a062f36eca212f92f24f2ca6329b88ff
[terms]: https://github.com/AndrewBelt/WaveEdit/blob/master/doc/manual.pug#waveedit-online
[saw]: https://github.com/smpldsnds/wavedit-online/blob/1a8d80f7a062f36eca212f92f24f2ca6329b88ff/samples/RESO_SAW.WAV
[vocal]: https://github.com/smpldsnds/wavedit-online/blob/1a8d80f7a062f36eca212f92f24f2ca6329b88ff/samples/VOCAL_FO.WAV
