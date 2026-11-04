# How Speech Fixer works, and where its numbers come from

## Where RX keeps this code
The installed RX 12 Spectral Editor plug-in is a 2 MB loader. Function `0x180047b80` reads the
`IZCORE / PATH` resource ("iZRX12Core.dll") and loads that 955 MB engine (ev_1f0c58b9…). Its
module table `FUN_180c43a70` lists Leveler (module 15), Loudness Control (`RXLoudness`, 18),
Dialogue Isolate (`DialogueExtract`, 24) and Loudness Optimize (47) (ev_fc691b35…).

The Leveler, Loudness Optimize and Loudness Control code itself is in the main program,
`C:\Program Files\iZotope\RX 12 Audio Editor\win64\iZotope RX 12 Audio Editor.exe`. That program
keeps its C++ class names (`DSP::Leveler::Leveler`, `DSP::Shared::Leveler::Leveler`,
`DSP::LoudnessOptimize::LoudnessOptimize`, `DSP::ConfigurableUpwardsCompress`, `DSP::RXLoudness::RXLoudness`,
`DSP::Shared::TwoPassLoudness`, `DSP::Shared::LoudnessMatch`). All addresses below are in that
program unless they say otherwise. Every value was read from its data with Ghidra's pseudo-C as the guide;
no iZotope code is copied, only the numbers and rules.

RX's podcast chain "Optimized Podcast Loudness" runs Loudness Control first and then Loudness
Optimize, and "Podcast Delivery" ends with Loudness Control. Speech Fixer runs: find speech →
Leveler → set loudness → Loudness Optimize → Loudness Control with limiter.

"estimated" means I looked and found no value in the code or presets; those are my own choices,
tested on the demo recording.

## 1. Find the speech (stands in for Dialogue Isolate)
RX's Dialogue Isolate is a trained neural separator (`DialogueExtractTwo`, BiRNN, onnxruntime /
tensorflow-lite models). Those models are iZotope's own files and can't go into this app, so a
hand-built speech finder scores every 10 ms on these cues:
| cue | rule | source |
|---|---|---|
| analysis frame | 35 ms rounded up to a power of two (2048 at 44.1/48 kHz), 10 ms step | frame length from inner Leveler `FUN_142f87de0` (`sr × 0.035`, min 8); step estimated |
| above the noise floor | ≥ 10 dB over the quietest 50 ms block within ±4 s | SNR Threshold 10 dB, set by `FUN_142bc3b50` (range 0–30, `0x1451438b8`); floor search estimated |
| syllable rhythm | speech-band level varies > 3.5 dB over 0.5 s | estimated |
| speech band | > 35 % of 60 Hz–8 kHz energy in 300 Hz–5 kHz | band edges from RX's Leveler speech/breath detector `FUN_142f864b0` (`0x1452d8618` = 300, `0x1452d8780` = 5000); 35 % share estimated |
| not hiss or clap | spectral flatness < 0.45 | estimated |
| a voice | periodicity (70–400 Hz pitch) > 0.4 | estimated |
| stops and starts | 15–90 % of the last 0.6 s is voiced | estimated |
| music veto | > 90 % voiced and < 5 dB rhythm → music, left out | estimated |
| syllable onsets | somewhere within ±0.5 s the speech-band level must rise ≥ 6 dB within 150 ms; fade-outs and decaying notes only fall, so they aren't speech (they were raised up to +20 dB before) | estimated |
| Sensitivity | default 6.0; threshold = 0.35 − (sens − 6) × 0.05 | default from preset "Isolate Dialogue" (6.03); mapping estimated |
| joining | gaps shorter than the gate release join one stretch (Dialog 0.6 s, Music 0.4 s), 150 ms padding | release times from `FUN_142bc3830`; padding estimated |

## 2. Even out the speakers (RX Leveler)
The RX Leveler module (`FUN_142bc27e0`) drives an inner leveler `DSP::Shared::Leveler::Leveler`
(`FUN_142f87de0`). Its settings, ranges and defaults:
| setting | range | RX default | Speech Fixer default | source |
|---|---|---|---|---|
| Target Level | −40 … 0 dB | −28 dB | −20 dB | range `0x1451350a0`, default `0x1452d8c68`; −20 from presets "Podcast Interview", "Streaming Dialogue" |
| Responsiveness | 0.3 … 7.1 | 2.0 | 1.0 | range `0x145bff338`, default `0x1451559fc`; 1.0 from preset "Podcast Interview" |
| Preserve Dynamic Range | 0 … 100 % | 50 % | 20 % | range `0x145135070`, default `0x1452d8474`; 20 from preset "Podcast Interview" |
| Optimize For | Dialog / Music | Dialog | Dialog | choice list `0x145135090` |

How they work (from the code):
| rule | value | source |
|---|---|---|
| Responsiveness → inner Time Scale | TS = (R − 0.3) / 1.7, so 0 … 4 | `FUN_142bc27e0` (constants `0x1452d7570` = 0.3, `0x1452d7a40` = 1.7) |
| level window | Hann-weighted running level (`RunningHanns<double>`), ±2.59 s at TS 1 | `FUN_142f87de0` (`sr × 2.59`, `0x1452d7dc0`), `FUN_142f86f50`; scaling by TS estimated |
| Preserve Dynamic Range → Maximum Gain | ((P − 100) / −100)² × 30 dB: 0 % → ±30 dB, 20 % → ±19.2 dB, 50 % → ±7.5 dB, 100 % → 0 | `FUN_142bc4d50` (`0x1452d8500` = 100, `0x1452d8ca0` = −100, `0x1452d8420` = 30) |
| two passes | pass 1 measures the gated level of the whole file; offset = Target − mean if ≥ 1 s passed the gate. Pass 2 rides the gain on top of that offset | `FUN_142f8c110`, `FUN_142f8c790` |
| dB → gain | exp(dB × 0.115129), only when the offset is ≥ −400 dB | `FUN_142f8c790` (`0x1452d7498`, `0x1452d8cd8`) |
| riding limit | the ride around the offset stays within ±Maximum Gain | gain computer `FUN_142f95720` clamps to ±Maximum Gain; the offset is applied to the input first (`FUN_142f8e9e0` multiplies each block by it), so the clamp works around the offset |
| relative gate | moments whose nearby speech level is more than 25 dB under the whole file's speech level aren't ridden (backstop for fade-outs and noise) | estimated |
| Level Range | 1 dB band around the target: inside it nothing is ridden, outside it the gain only brings the level to the nearer band edge (T − LR or T + LR) | rule from gain computer `FUN_142f95720`; 1.0 set by `FUN_142bc3b50` (range 0–10, default 2) |
| gate | SNR Threshold 10 dB | `FUN_142bc3b50` |
| Optimize For: Dialog | gate speed 0.1 s, release 0.6 s | `FUN_142bc3830` |
| Optimize For: Music | gate speed 0.5 s, release 0.4 s | `FUN_142bc3830` |
| smoothing | gain smoothed (zero-phase) with the gate speed as time constant | gate speed from `FUN_142bc3830`; this use estimated |
| Curve Fineness | 10 (range 0–10) | `FUN_142bc3b50`; not used (it shapes RX's drawn curve) |
| inner limiter | on; ceiling −0.1 dB, attack 0.1 s, hold 0.4 s, release 0.5 s, nodes every 0.2 s | `FUN_142bc3b50`, ranges `0x1451435e0`–`0x145143660`. Speech Fixer leaves peaks to the final limiter (estimated choice) |
| Lower music and noise | 0 dB (off), down to −20 dB, outside speech | Speech Fixer's own; idea from Dialogue Isolate "Light Dialogue Isolation" Noise Gain −8 dB |

## 3. Squeeze the dynamics (RX Loudness Optimize)
`DSP::LoudnessOptimize::LoudnessOptimize` (`FUN_142ce0800`, parameters `FUN_142ce0f80`) runs one
`DSP::ConfigurableUpwardsCompress` per channel. It's an **upward** compressor: it lifts the quiet parts
toward the peaks, it doesn't push the peaks down.
| setting | range | RX default | Speech Fixer default | source |
|---|---|---|---|---|
| Boost | 0 … 12 dB | 0 | +3 dB | range `0x145139090`; +3 from preset "Basic Start" and chain "Optimized Podcast Loudness" |
| Ratio | 1.5 … 30 | 4 | 4 | range `0x1451391a8`, default `0x1452d8000` |
| Threshold | −48 … 0 dB | −9 | −9 | range `0x1451391a0`, default `0x1452d8c04` |
| Speed | 1 … 500 ms | 120 | 120 | range `0x1451391b0`, default `0x1452d8538` |

| rule | value | source |
|---|---|---|
| gain law | knee K = T − B·R/(R−1). Below K: +B dB. Between K and T: B·(T − x)/(T − K) (output rises at 1/R). Above T: 0 dB | `FUN_143156da0` (4096-entry table filled by `FUN_143156600`, input 0 … 1.25 full scale) |
| detector | peak hold over ±Speed ms (`RunningMax<float>`), then a one-pole envelope, coefficient 1 − exp(−1 / (Speed/1000 × sr)) | `FUN_1431562a0`, `FUN_143156460` (`0x1452d86e4` = 1000, `0x1452d725c` = 0.001) |
| working level | loudness set to the final target first, as in "Optimized Podcast Loudness" | chain order from preset; doing it inside the app estimated |

## 4. Final loudness (RX Loudness Control)
`DSP::RXLoudness::RXLoudness` (`FUN_142bd3460`, parameters `FUN_142bd36c0`) uses
`DSP::Shared::TwoPassLoudness` and `DSP::Shared::LoudnessMatch` (with a short-term limiter).
| setting | range | RX default | Speech Fixer default | source |
|---|---|---|---|---|
| Loudness (target) | −42 … −5 LUFS | −24 | −16 | range `0x145135800`, default `0x1452d8c54`; −16 from preset "Podcast Delivery" |
| Margin (peak ceiling) | −20 … 0 dB | −2 | −1 | range `0x145135818`, default `0x1452d8aa8`; −1 from "Podcast Delivery" |
| Tolerance | 0.1 … 10 LU | 2 | 0.5 | range `0x145135810`, default `0x1452d7c68`; 0.5 from "Podcast Delivery" |
| Max Short-Term Loudness | −25 … 0 | −18 | off | range `0x1451357f0`, default `0x1452d8c34`; presets turn "Limit Short-Term Loudness" off |
| Enable Gate / Integrated | on / on | on | on | `FUN_142bd36c0` |
| Audiobook / Video / Broadcast | −20 / −14 / −23 LUFS | | | presets "Audiobook Delivery" (margin −3), "Video Streaming Delivery", "EBU R128" |
| Measure the speech only | on | | | dialogue gating estimated |
| Loudness meter | ITU-R BS.1770-4: K-weighting, 400 ms blocks, −70 LUFS absolute and −10 LU relative gate | | | published standard (RX's `LUFSComputer`) |
| error when too short | RX refuses integrated loudness on very short audio ("at least a few seconds") | | | message table in iZRX12Core.dll `FUN_180bcb990` |
| Limiter | 5 ms look-ahead, 80 ms release, peaks between samples estimated at 4× | | | estimated |
| Passes | up to 3, stop when within Tolerance | | | two-pass idea from `TwoPassLoudness`; count estimated |

## Controls
| key | does |
|---|---|
| Space | play / stop |
| 1 / 2 | listen to the original / the fixed version (switches at the same moment) |
| Left / Right | 5 s back / forward; click a waveform to jump |
| O, S, D | open, save the fixed WAV (24-bit), load the demo |
| A | auto fix |

## On screen
Original waveform (blue) with the volume change curve (orange) and found speech (green). The fixed
waveform is in teal, with the loudness of each speech part on both. The measurements table shows whole-track and
speech loudness, highest peak, the gap between quiet and loud parts (10th–90th percentile of 3 s
speech loudness) and the share of speech. The four module cards sit on the right, like RX's module chain.

## Auto fix (window button, A key, and the command line's default)
| rule | value | source |
|---|---|---|
| more sensitive when little speech is found | below 25 % speech: Sensitivity +1.5, up to 2 times (max 9) | estimated |
| tighten leveling until parts agree | stop at a quiet-to-loud gap of 2 dB or less | estimated |
| each tightening step | Preserve −10 % (min 0), Responsiveness × 0.75 (min 0.5) | estimated |
| passes | up to 4 tightening passes | estimated |
| settings given with --set / --preset | never changed by auto fix | by design |

## Listening
Sound goes straight to Windows' default output device (cpal, WASAPI). Both versions share one
timeline, so 1 / 2 switches at exactly the same moment. Linear resampling is used when the file's
rate differs from the device's (estimated choice).

## RX's per-sample Leveler (read; used where noted)
| part | what it does | source |
|---|---|---|
| per-sample loop | level → dB (ln × 8.6859, floor −400 dB), speech/breath flag, gain from the gain computer plus the De-esser gain, applied as exp(dB × 0.115129) | `FUN_142f8a770` |
| level | mean square of the pre-filtered signal, two one-pole averages turned into a Hann-weighted running mean (`RunningHanns`), channels combined | `FUN_142f8a080`; Speech Fixer uses K-weighted power with a Hann window |
| gain computer | dead band ±Level Range around Target, correction to the band edge, breath/noise reductions blended in, clamp ±Maximum Gain, then smoothed | `FUN_142f95720`; noise control is off in RX's Leveler (mode 2 set by `FUN_142bc3b50`) |
| speech/breath detector | per frame: harmonic peaks in 100–4000 Hz (peaks ≥ 16, each counted as peak^0.2, × 0.3) plus peak-energy share in 300–5000 Hz; harmonicity weight ramps from 2 to 6 (× 0.25); band energy against its slow average; speech-shape term exp(−((c1+1)/15)² − ((c2+6.5)/10)²) on two cepstral values; threshold 10 − Breath Sensitivity % × 20 dB, × 0.6; a small classifier `FUN_142f47010` decides | `FUN_142f864b0` (`0x1452d8338`, `0x1452d7508`, `0x1452d7c68`, `0x1452d8108`, `0x1452d754c`, `0x1452d8388`, `0x1452d7690`, `0x1452d8310`, `0x1452d8170`). Speech Fixer takes its 300–5000 Hz band; the classifier's weights weren't read, so the rest of the speech finder stays its own (estimated) |

## Still open
How Time Scale stretches the 2.59 s Hann window: the value is stored at offset 0x2a0 of the inner
leveler, but the code that reads it wasn't found. Also the weights of the speech/breath classifier
`FUN_142f47010`. Speech Fixer uses a window of 2.59 s × Time Scale and its own speech cues.
