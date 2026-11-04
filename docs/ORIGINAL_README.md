This build contains the original game's own files, for the private use of the person who owns the game, on their own computer. It must not be shared or published.

(For this app, that means only the numbers read from the person's own RX 12 presets. No iZotope
file is actually inside it. See CREDITS.md.)

# Speech Fixer

A standalone app that fixes speech volume in a recording, built on what RX 12 does:

1. **Find the speech.** Music, ambient noise and single loud bangs are left out of all measuring.
2. **Even out the speakers.** Quiet and loud speakers and parts are brought to the same level.
3. **Tame the peaks.** Peaks are squeezed so a few loud spikes no longer hold the volume down.
4. **Final loudness.** The speech is set to a loudness target (podcast −16 LUFS, audiobook, video,
   broadcast), with peaks kept under a ceiling.

Open a WAV, MP3, FLAC, M4A/AAC or OGG file, listen to the original and fixed versions side by
side, and save the fixed version as a 24-bit WAV. Every setting is explained in MECHANICS.md.

Options: `speech-fixer <file>` opens a file, `--screenshot out.png` saves the screen after 120
frames, and `--self-test out.json` runs the built-in checks on the demo recording.

## Listening
Sound plays through Windows' default speakers or headphones. Press Play (or Space), and press
1 or 2 to switch between the original and the fixed version at the same moment.

## Automatic fixing (for you or an AI assistant)
**Auto fix** in the window (or the A key) tunes the settings by itself. It finds enough speech,
then tightens the leveling until every part is within about 2 dB, while keeping your loudness
target.

An AI assistant such as Claude Code can run the same thing without the window and read the results:

    speech-fixer --process "recording.mp3" "recording (fixed).wav" --json
    speech-fixer --process "C:\Recordings" "C:\Recordings\fixed" --preset audiobook --json
    speech-fixer --process in.wav out.wav --set background=-6 --set preserve=0 --json

`--json` prints, for each file, the settings used and the measurements before and after: loudness,
speech loudness, highest peak, the gap between quiet and loud parts, and the share of speech.
The assistant can then adjust a setting with `--set` and run it again. `speech-fixer --help`
lists every option. Auto fix is on by default. `--manual` turns it off, and any setting given
with `--set` or `--preset` is left exactly as given.
