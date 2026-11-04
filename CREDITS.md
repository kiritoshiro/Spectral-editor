# Credits

No sound, picture, font or code file from iZotope RX 12 is copied into this app. All of its
sound processing, its look and its demo recording were made from scratch.

What was read from the person's own RX 12 install, for its numbers only (see [docs/MECHANICS.md](docs/MECHANICS.md)):

| original file | used for |
|---|---|
| C:\Program Files\Common Files\VST3\iZotope\RX 12 Spectral Editor.vst3 | showed that all RX processing lives in iZRX12Core.dll (function 0x180047b80) |
| C:\Program Files\iZotope\RX 12 Audio Editor\Cores\iZRX12Core.dll | module table and names (Leveler, Loudness Optimize, RXLoudness, DialogueExtract) |
| C:\Program Files\iZotope\RX 12 Audio Editor\win64\iZotope RX 12 Audio Editor.exe | the Leveler, Loudness Optimize and Loudness Control setting ranges, defaults, formulas and timing (read with Ghidra; addresses in MECHANICS.md) |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Leveler\Podcast Interview.xml | leveler target, responsiveness, preserve dynamic range |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Leveler\Streaming Dialogue.xml | leveler target level |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Loudness Optimize\Basic Start.xml | peak tamer boost |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Module Chain\Optimized Podcast Loudness.xml | peak tamer threshold, ratio, speed; order of the stages |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Module Chain\Podcast Delivery.xml | order of the stages |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Loudness Control\Podcast Delivery.xml | −16 LUFS, −1 dB ceiling |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Loudness Control\Audiobook Delivery.xml | −20 LUFS, −3 dB ceiling |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Loudness Control\Video Streaming Delivery.xml | −14 LUFS |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Loudness Control\EBU R128.xml | −23 LUFS |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Dialogue Isolate\Isolate Dialogue.xml | default speech sensitivity |
| C:\Program Files\iZotope\RX 12 Audio Editor\Presets\Dialogue Isolate\Light Dialogue Isolation.xml | idea for "lower music and noise" |

Libraries: macroquad, symphonia, hound, rustfft, rfd, cpal (all open source).
Loudness measurement follows the public ITU-R BS.1770-4 standard.
