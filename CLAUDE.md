# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`voicevox-cli` provides the `vvsay` command: a text-to-speech CLI built on VOICEVOX CORE (`voicevox_core` Rust crate, linked directly).
It is the VOICEVOX counterpart of `voicepeak-cli` (`vpsay`): presets, auto-play with mpv, stdin/file input, `--bg`.

## Development Commands

`voicevox_core` builds Open JTalk with CMake. In this environment CMake comes from mise without a global version, so run cargo through it:

- `mise exec cmake@4.4.3 -- cargo build`
- `mise exec cmake@4.4.3 -- cargo test`
- `mise exec cmake@4.4.3 -- cargo clippy --all-targets`
- `cargo fmt`

Runtime assets (ONNX Runtime, Open JTalk dictionary, VVM models) live in `~/.local/share/vvsay` (see README). Tests do not need them; manual runs do.

## Architecture

Library + binary crate, no `mod.rs`:

- `src/cli.rs` - clap definition and the main flow (input → voice resolution → optional fork → synthesis → play/save)
- `src/config.rs` - `~/.config/vvsay/config.toml`, presets, precedence (CLI > preset > VOICEVOX defaults)
- `src/assets.rs` - Locating assets, scanning VVM metadata (without loading models) to resolve a voice to a style ID and VVM file
- `src/engine.rs` - `VoiceParams` validation, Synthesizer setup (loads only the needed VVM), segmented and streamed synthesis
- `src/text_splitter.rs` - Sentence splitting so playback can start after the first sentence (sentences themselves are never split, to keep whole-sentence intonation)
- `src/wav.rs` - WAV parsing/encoding
- `src/player.rs` - mpv fed with raw PCM through stdin, so sentences can be appended while playing
- `src/queue.rs` - `--queue` ticket queue shared between processes (JSON state + file lock in `~/.local/state/vvsay`)

## Things to keep in mind

- **Fork before ONNX Runtime is initialized** (`--bg`). Voice resolution and parameter validation run before the fork so that errors reach the caller. `VoiceModelFile::open` in the blocking API does not spawn threads.
- **Memory grows faster than linearly with the audio length of one synthesis call** (12 s ≈ 0.7 GB, 47 s ≈ 2.5 GB, 95 s ≈ 6.6 GB peak RSS, mostly in `create_audio_feature`). `Engine::synthesize` therefore splits each AudioQuery into segments of ≤30 s at accent phrase boundaries (preferring phrases followed by a pause) and pads long leading/trailing silence with zero samples. `speed` must be > 0: VOICEVOX's type accepts 0, which makes the audio length `usize::MAX`.
- **Streaming**: each segment goes through `create_audio_feature` and is then `render`ed in growing slices (0.5 s, 1 s, 3 s, ...), passed to a sink as soon as each slice is ready. The joined slices are bit-identical to `synthesis` of the whole segment (verified), and first audio comes ~0.15 s after the engine is ready regardless of the sentence length.
- **Queue entries are tied to both vvsay and its mpv**: mpv keeps playing buffered PCM after vvsay is killed, so an entry is pruned only when both PIDs are dead.
- The downloader may install a different ONNX Runtime version than `Onnxruntime::LIB_RECOMMENDED_VERSIONED_FILENAME`; the library is found by prefix.
- Parallel synthesis was measured and rejected: ONNX Runtime already uses all cores (~10% gain at the cost of loading the model per synthesizer).
