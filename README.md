# Spectral-editor / Speech Fixer

A standalone offline Rust desktop app for improving speech volume in recordings. The application and executable are called **Speech Fixer**. It finds speech, levels quiet and loud speakers, controls dynamics, sets speech loudness and limits peaks. It has waveform comparison, playback, automatic fixing and a headless batch-processing CLI. It is not a general-purpose spectral painting editor.

No iZotope installation, VST host, activation, model download or network service is needed. The original development notes describe independent code informed by observed RX 12 parameters and behavior, with a heuristic speech detector rather than iZotope's neural model. See [CREDITS.md](CREDITS.md) and [docs/MECHANICS.md](docs/MECHANICS.md) for provenance and remaining estimates.

## Downloads and use

Tagged builds appear in [Releases](https://github.com/kiritoshiro/Spectral-editor/releases). Development builds are downloadable from successful [Actions runs](https://github.com/kiritoshiro/Spectral-editor/actions/workflows/desktop.yml) while artifacts are retained.

- **Windows 10/11 x64:** extract `Spectral-editor-windows-x86_64.zip` and run `Speech-Fixer.exe`. Rust and a separate Visual C++ runtime are not required. Builds are unsigned.
- **Linux x64:** Ubuntu 22.04+ or a compatible desktop with glibc 2.35+, OpenGL drivers, working ALSA audio and XDG portal services for dialogs. Run `chmod +x Spectral-editor-linux-x86_64.AppImage`, then `./Spectral-editor-linux-x86_64.AppImage`. Without FUSE use `./Spectral-editor-linux-x86_64.AppImage --appimage-extract-and-run`. Application libraries are bundled; core OS libraries, graphics drivers and desktop services come from the host.

Open a file, or start with the built-in demo. Use **Auto fix** (A) to tune the leveler. Space plays/pauses; 1 and 2 compare original/fixed audio at the same position. Save to a separate filename to preserve your source.

Inputs include WAV/AIFF, MP3, FLAC, M4A/AAC, ALAC and OGG Vorbis; container support depends on the codec. Surround input keeps only the first two channels. Output is 24-bit WAV at the source sample rate, with samples clamped to [-1, 1]. This is volume processing, not guaranteed source separation when speech overlaps music.

## Command line

Use the packaged Windows `Speech-Fixer.exe`, the Linux AppImage, or the source-build executable `speech-fixer`:

```sh
speech-fixer --help
speech-fixer --process recording.mp3 recording-fixed.wav --json
speech-fixer --process recordings fixed --preset audiobook --json
speech-fixer --process in.wav out.wav --set background=-6 --set preserve=0 --json
```

Auto fix is on by default. `--manual` uses settings as given; explicit `--preset` and `--set` values remain locked during automatic tuning. Targets include podcast (-16 LUFS), audiobook (-20), video (-14), and broadcast (-23). `--json` returns settings and before/after measurements. Nonzero process exits indicate an error. Use separate input/output paths; the existing exporter can overwrite a destination.

## Development and checks

The Cargo project is at the repository root. Rustup selects the pinned Rust 1.98.1 toolchain. Windows needs Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK. On Ubuntu install:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev libgl1-mesa-dev libegl1-mesa-dev
```

```sh
cargo run --locked
cargo fmt --all -- --check
cargo test --release --locked
cargo build --release --locked
```

The headless Cargo regression checks the synthetic demo's speech detection, clap rejection, speaker-level convergence, loudness, peak ceiling, automatic fixing and 24-bit WAV roundtrip. It does not require graphics or sound hardware. The original GUI test remains available as `speech-fixer --self-test report.json`; it needs a desktop and records individual pass/fail results in JSON. `--screenshot out.png` captures the window. Do not infer success from the original GUI test's process exit code alone; inspect each JSON step.

Source map: `src/main.rs` owns GUI, CLI and job orchestration; `src/dsp.rs` analysis and processing; `src/audio_io.rs` audio files and generated demo; `src/player.rs` playback. Development notes and parameter evidence live in `docs/MECHANICS.md` and `docs/mechanics.json`. Original credits and README are preserved, with the original README at `docs/ORIGINAL_README.md`.

## Release workflow

[Desktop builds and releases](.github/workflows/desktop.yml) runs formatting, headless tests, builds and packaging for Windows x64 and Linux x64 on main pushes, pull requests and manual dispatch. It uploads a portable Windows ZIP, a Linux AppImage and SHA-256 files. Linux packaging uses [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy) and smoke-tests the packaged CLI with `--help`.

Update Cargo.toml/Cargo.lock, commit the release version, then push a matching tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

`v*` tags publish both packages in one GitHub Release only after both platform jobs pass. Development artifacts expire after 14 days. Signing, installers, automatic updates, macOS and ARM builds are not configured.

## Import and limitations

Imported on 2026-11-04 from the original Craft Anything project's `app` folder. Only current Rust source, Cargo files and development documentation are retained; generated targets, screenshots, backup source files, session logs, private audio and proprietary reference files are excluded.

The original README labeled the build private-use only, while CREDITS states that no iZotope sound, image, font or code files are included. That historical notice is preserved in `docs/ORIGINAL_README.md`; no new application license or redistribution grant is made by this import. Dependencies retain their own licenses. Review the original provenance and licensing before binary redistribution.

The speech finder is heuristic; real music/speech mixtures can be misclassified. Long recordings are held in RAM. Synthetic checks do not establish listening quality, exact RX equivalence, clean-machine hardware compatibility or compliance with any delivery specification.
