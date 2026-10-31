# voicevox-cli

**[日本語](README_ja.md) | English**

`vvsay` is a text-to-speech command built on [VOICEVOX CORE](https://github.com/VOICEVOX/voicevox_core), with presets and automatic playback.

## Features

- 🎵 **Auto-play with mpv** - Plays the speech when no output file is given
- ⚡ **Fast start** - Playback starts ~0.6 s after invocation, regardless of the text length: speech is rendered in small slices and streamed to the player while the rest is being synthesized
- 📝 **Voice presets** - Save combinations of character, style and parameters
- 📜 **Long text support** - No length limit. Long sentences are synthesized in segments to keep memory usage bounded
- 🧾 **Ordered playback** - `--queue` waits for other playback and plays in the order of invocation
- 🔄 **Pipe input support** - `echo "テキスト" | vvsay`
- 🚀 **Background execution** - `--bg` returns shell control immediately
- ⚙️ **Configuration file** - `~/.config/vvsay/config.toml`

## Requirements

- macOS or Linux. On Linux, arm64 is tested (Debian 12: install, `--setup`, synthesis, and playback through PulseAudio); x64 is not tested yet
- [mpv](https://mpv.io/) for playback (`brew install mpv`, or the distribution's `mpv` package). On Linux, mpv plays through the desktop session's PipeWire or PulseAudio
- For building: Rust 1.89+, a C++ compiler, and [CMake](https://cmake.org/) (`brew install cmake`, or `build-essential` and `cmake` on Debian/Ubuntu)
- VOICEVOX CORE assets (see below)

## Installation

```bash
cargo install --git https://github.com/petamorikei/voicevox-cli
```

or from a local clone:

```bash
cargo install --path .
```

`voicevox_core` is not published on crates.io, so this tool is installed from git rather than from crates.io.

### Downloading VOICEVOX CORE assets

`vvsay` needs the VOICEVOX ONNX Runtime, the Open JTalk dictionary and the voice models (VVM files).
Run the following in a terminal:

```bash
vvsay --setup
```

It fetches the official VOICEVOX CORE downloader (0.17.1, checksum-verified) and runs it.
**The downloader asks you to accept the terms of use of the ONNX Runtime and the voice models; read them and answer yourself.**
vvsay never answers on your behalf by default. To automate setup (e.g. in CI) after reading and agreeing to the terms, run `vvsay --setup --accept-terms`: the terms are still printed, and vvsay answers "yes" to the prompt. If the assets are already installed, it does nothing unless "yes" is given to the re-download question on stdin.

- Assets are installed into `~/.local/share/vvsay` (or `assets_dir` in the configuration file).
- Only talk models are downloaded (song models are not used).
- If you hit GitHub's rate limit, run `GH_TOKEN=$(gh auth token) vvsay --setup`.
- Running it again after installation asks whether to download again.

<details>
<summary>Manual installation</summary>

```bash
mkdir -p ~/.local/share/vvsay && cd ~/.local/share/vvsay
curl -sSfL https://github.com/VOICEVOX/voicevox_core/releases/download/0.17.1/download-osx-arm64 -o download
chmod +x download
./download --only onnxruntime dict models --models-pattern '[0-9]*.vvm' -o ~/.local/share/vvsay
```

Use `download-osx-x64` on Intel Macs, `download-linux-x64` / `download-linux-arm64` on Linux.
</details>

### Terms of use

The voice models are subject to the **VOICEVOX voice model terms of use** (`~/.local/share/vvsay/models/TERMS.txt`) and to the terms of each voice library (character).
Most characters require a credit such as `VOICEVOX:ずんだもん` when you publish generated audio. Check each character's terms before publishing.

## Usage

```bash
# Speak with a character (the first style is used when -s is omitted)
vvsay -n ずんだもん "こんにちは、ずんだもんなのだ。"

# Choose a style
vvsay -n ずんだもん -s あまあま "こんにちは"

# Or a style ID
vvsay --style-id 1 "こんにちは"

# Save to a WAV file instead of playing
vvsay -n ずんだもん -o output.wav "こんにちは"

# Read from a file or a pipe
vvsay -n ずんだもん -t input.txt
cat document.txt | vvsay -p zundamon

# Adjust the voice
vvsay -n ずんだもん --speed 1.2 --pitch 0.05 --intonation 1.2 --volume 1.0 "こんにちは"

# List characters/styles and presets
vvsay --list-narrator
vvsay --list-style ずんだもん
vvsay --list-presets

# Run in background
vvsay --bg -p zundamon "ビルドを開始するのだ"

# Play in the order of invocation without overlapping
vvsay --bg --queue -p zundamon "ビルドを開始するのだ"
vvsay --bg --queue -p zundamon "ビルドが完了したのだ"
```

### Playback queue

| Invocation | Behavior |
|---|---|
| without `--queue` (default) | Plays immediately. May overlap with other playback. |
| with `--queue` | Synthesizes immediately, but starts playing only after all other playback (with or without `--queue`) has finished. Queued invocations play in the order they were started. |

`--queue` has no effect with `-o`.

## Configuration

`~/.config/vvsay/config.toml` is created on first run.

```toml
default_preset = "zundamon"
# assets_dir = "~/.local/share/vvsay"

[[presets]]
name = "zundamon"
narrator = "ずんだもん"
style = "ノーマル"
speed = 1.1

[[presets]]
name = "metan-whisper"
narrator = "四国めたん"
style = "ささやき"
pitch = 0.03
intonation = 1.2
```

- `default_preset`: Preset used when `-p` is omitted
- `assets_dir`: Location of VOICEVOX CORE assets (default `~/.local/share/vvsay`)
- `presets[].narrator`: Character name as shown by `--list-narrator`
- `presets[].style`: Style name (optional; the first style of the character when omitted)
- `presets[].speed` / `pitch` / `intonation` / `volume` / `pre_phoneme_length` / `post_phoneme_length`: Optional, same as the command line options

### Parameter precedence

1. Command line options
2. Preset (`-p`, or `default_preset`)
3. VOICEVOX defaults

Each item is overridden individually, e.g. `vvsay -p zundamon -s あまあま` uses the preset's parameters with the あまあま style.

## Command line options

```
Usage: vvsay [OPTIONS] [TEXT]

Arguments:
  [TEXT]  Text to say (or pipe from stdin)

Options:
  -t, --text <FILE>                    Text file to say
  -o, --out <FILE>                     Path of output WAV file (optional - will play with mpv if not specified)
  -n, --narrator <NAME>                Character name (e.g. ずんだもん)
  -s, --style <NAME>                   Style name (default: the first style of the character)
      --style-id <ID>                  Style ID (use --list-narrator to see IDs)
  -p, --preset <NAME>                  Use voice preset (use --list-presets to see available presets)
      --speed <VALUE>                  Speed scale (default 1.0)
      --pitch <VALUE>                  Pitch scale (default 0.0)
      --intonation <VALUE>             Intonation scale (default 1.0)
      --volume <VALUE>                 Volume scale (default 1.0)
      --pre-phoneme-length <SECONDS>   Silence before the speech in seconds (default 0.1)
      --post-phoneme-length <SECONDS>  Silence after the speech in seconds (default 0.1)
      --list-narrator                  Print characters and their styles
      --list-style <NARRATOR>          Print styles of the given character
      --list-presets                   Print available presets
      --setup                          Download VOICEVOX CORE assets with the official downloader (asks you to accept the terms of use)
      --accept-terms                   With --setup: accept the terms of use without asking (only after you have read them)
      --queue                          Wait until other playback finishes, playing in the order of invocation
      --bg                             Run in background (return immediately)
  -v, --verbose                        Enable verbose output (show VOICEVOX CORE logs and timings)
  -h, --help                           Print help
  -V, --version                        Print version
```

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE).
VOICEVOX CORE, the VOICEVOX ONNX Runtime and the voice models are distributed under their own terms.
