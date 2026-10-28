use anyhow::{Context as _, Result, bail};
use clap::Parser;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use crate::assets::{Assets, default_assets_dir, expand_tilde};
use crate::config::{VoiceOverrides, list_presets, load_config};
use crate::engine::{Engine, VoiceParams};
use crate::player::{Player, check_mpv_available};
use crate::queue::Queue;
use crate::text_splitter::split_text;
use crate::wav;

#[derive(Debug, Parser)]
#[command(
    name = "vvsay",
    version,
    about = "VOICEVOX CORE text-to-speech CLI with presets and auto-play"
)]
pub struct Cli {
    /// Text to say (or pipe from stdin)
    #[arg(value_name = "TEXT")]
    text: Option<String>,

    /// Text file to say
    #[arg(
        short = 't',
        long = "text",
        value_name = "FILE",
        conflicts_with = "text"
    )]
    file: Option<PathBuf>,

    /// Path of output WAV file (optional - will play with mpv if not specified)
    #[arg(short, long, value_name = "FILE")]
    out: Option<PathBuf>,

    /// Character name (e.g. ずんだもん)
    #[arg(short, long, value_name = "NAME")]
    narrator: Option<String>,

    /// Style name (default: the first style of the character)
    #[arg(short, long, value_name = "NAME")]
    style: Option<String>,

    /// Style ID (use --list-narrator to see IDs)
    #[arg(long, value_name = "ID", conflicts_with_all = ["narrator", "style"])]
    style_id: Option<u32>,

    /// Use voice preset (use --list-presets to see available presets)
    #[arg(short, long, value_name = "NAME")]
    preset: Option<String>,

    /// Speed scale (default 1.0)
    #[arg(long, value_name = "VALUE", allow_negative_numbers = true)]
    speed: Option<f32>,

    /// Pitch scale (default 0.0)
    #[arg(long, value_name = "VALUE", allow_negative_numbers = true)]
    pitch: Option<f32>,

    /// Intonation scale (default 1.0)
    #[arg(long, value_name = "VALUE", allow_negative_numbers = true)]
    intonation: Option<f32>,

    /// Volume scale (default 1.0)
    #[arg(long, value_name = "VALUE", allow_negative_numbers = true)]
    volume: Option<f32>,

    /// Silence before the speech in seconds (default 0.1)
    #[arg(long, value_name = "SECONDS", allow_negative_numbers = true)]
    pre_phoneme_length: Option<f32>,

    /// Silence after the speech in seconds (default 0.1)
    #[arg(long, value_name = "SECONDS", allow_negative_numbers = true)]
    post_phoneme_length: Option<f32>,

    /// Print characters and their styles
    #[arg(long)]
    list_narrator: bool,

    /// Print styles of the given character
    #[arg(long, value_name = "NARRATOR")]
    list_style: Option<String>,

    /// Print available presets
    #[arg(long)]
    list_presets: bool,

    /// Wait until other playback finishes, playing in the order of invocation
    #[arg(long)]
    queue: bool,

    /// Run in background (return immediately)
    #[arg(long)]
    bg: bool,

    /// Enable verbose output (show VOICEVOX CORE logs and timings)
    #[arg(short, long)]
    verbose: bool,
}

impl Cli {
    fn overrides(&self) -> VoiceOverrides {
        VoiceOverrides {
            preset: self.preset.clone(),
            narrator: self.narrator.clone(),
            style: self.style.clone(),
            style_id: self.style_id,
            params: VoiceParams {
                speed: self.speed,
                pitch: self.pitch,
                intonation: self.intonation,
                volume: self.volume,
                pre_phoneme_length: self.pre_phoneme_length,
                post_phoneme_length: self.post_phoneme_length,
            },
        }
    }
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    if cli.verbose {
        tracing_subscriber::fmt()
            .with_writer(io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .init();
    }

    let config = load_config()?;

    if cli.list_presets {
        list_presets(&config);
        return Ok(());
    }

    let assets_dir = match &config.assets_dir {
        Some(dir) => expand_tilde(dir)?,
        None => default_assets_dir()?,
    };
    let assets = Assets::locate(&assets_dir)?;

    if cli.list_narrator {
        return list_narrators(&assets, None);
    }
    if let Some(narrator) = &cli.list_style {
        return list_narrators(&assets, Some(narrator));
    }

    let input_text = read_input(&cli)?;
    let chunks = split_text(&input_text);
    if chunks.is_empty() {
        bail!("Nothing to say: the input text has no readable characters");
    }

    let (selector, params) = config.resolve_voice(&cli.overrides())?;
    params.validate()?;
    let voice = assets.resolve(&selector)?;
    if cli.verbose {
        eprintln!(
            "Voice: {} [style_id={}] from {}",
            voice.label,
            voice.style_id,
            voice.vvm.display()
        );
        if !params.describe().is_empty() {
            eprintln!("Params: {}", params.describe());
        }
        eprintln!("Chunks: {}", chunks.len());
    }

    let playing = cli.out.is_none();
    if playing {
        check_mpv_available()?;
    }
    let queue = Queue::new()?;
    let use_queue = playing && cli.queue;

    // Fork before ONNX Runtime is initialized: it starts worker threads, which do not survive fork.
    if cli.bg {
        if fork_into_background(use_queue.then_some(&queue))? == ForkResult::Parent {
            return Ok(());
        }
        if use_queue && !queue.is_enqueued(std::process::id())? {
            // The parent failed to register us; take a ticket ourselves.
            queue.enqueue(std::process::id())?;
        }
    } else if use_queue {
        queue.enqueue(std::process::id())?;
    }

    let started = Instant::now();
    let engine = Engine::new(&assets, &voice, params)?;
    if cli.verbose {
        eprintln!(
            "Engine ready in {:.0} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }

    match &cli.out {
        Some(out) => save(&engine, &chunks, out),
        None => play(
            &engine,
            &chunks,
            &queue,
            use_queue,
            cli.verbose.then_some(started),
        ),
    }
}

fn read_input(cli: &Cli) -> Result<String> {
    if let Some(text) = &cli.text {
        Ok(text.clone())
    } else if let Some(file_path) = &cli.file {
        std::fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read {}", file_path.display()))
    } else if !io::stdin().is_terminal() {
        // Read from stdin if available (pipe input)
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        Ok(buffer.trim().to_string())
    } else {
        bail!("Either text argument, --text file, or pipe input must be specified")
    }
}

fn list_narrators(assets: &Assets, filter: Option<&str>) -> Result<()> {
    let characters = assets.characters()?;
    let mut found = false;
    for (character, _) in characters
        .iter()
        .filter(|(c, _)| filter.is_none_or(|name| c.name == name))
    {
        found = true;
        println!("{}", character.name);
        for style in &character.styles {
            println!("  {:>4}: {}", style.id.0, style.name);
        }
    }
    if let Some(name) = filter
        && !found
    {
        bail!("Unknown narrator: {name}. Use --list-narrator to see available voices.");
    }
    Ok(())
}

fn save(engine: &Engine, chunks: &[String], out: &Path) -> Result<()> {
    let mut format = None;
    let mut pcm = Vec::new();
    for chunk in chunks {
        engine.synthesize(chunk, &mut |piece_format, piece| {
            if *format.get_or_insert(piece_format) != piece_format {
                bail!("Synthesized audio formats differ between sentences");
            }
            pcm.extend_from_slice(piece);
            Ok(())
        })?;
    }
    let format = format.context("No audio was synthesized")?;
    std::fs::write(out, wav::encode(format, &pcm)?)
        .with_context(|| format!("Failed to write {}", out.display()))?;
    println!("Audio saved to: {}", out.display());
    Ok(())
}

/// Synthesizes on a worker thread and streams each rendered piece to mpv as soon as it is ready,
/// so playback starts after the first slice of the first sentence regardless of the text length.
fn play(
    engine: &Engine,
    chunks: &[String],
    queue: &Queue,
    use_queue: bool,
    timer: Option<Instant>,
) -> Result<()> {
    thread::scope(|scope| {
        // Owned by this closure so that the worker stops early if playback fails.
        let (tx, rx) = mpsc::channel();
        scope.spawn(move || {
            for chunk in chunks {
                let result = engine.synthesize(chunk, &mut |format, pcm| {
                    tx.send(Ok((format, pcm.to_vec())))
                        .map_err(|_| anyhow::anyhow!("Playback stopped"))
                });
                if let Err(e) = result {
                    // Fails only if playback has already stopped, in which case nobody needs it.
                    let _ = tx.send(Err(e));
                    break;
                }
            }
        });

        let (format, first) = rx.recv().context("Synthesis stopped unexpectedly")??;
        let pid = std::process::id();
        let lease = if use_queue {
            queue.wait_turn(pid)?
        } else {
            queue.start_playing(pid)?
        };
        let mut player = Player::spawn(format)?;
        lease.set_player(player.pid())?;
        if let Some(started) = timer {
            eprintln!(
                "Playback started in {:.0} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }

        player.write(&first)?;
        for result in rx {
            let (chunk_format, pcm) = result?;
            if chunk_format != format {
                bail!("Synthesized audio formats differ between sentences");
            }
            player.write(&pcm)?;
        }
        player.finish()?;
        drop(lease);
        Ok(())
    })
}

#[derive(Debug, PartialEq)]
enum ForkResult {
    Parent,
    Child,
}

/// Detaches into a background process. With `queue`, the parent takes the ticket on behalf of the
/// child before returning, so the order of invocation is kept even for back-to-back calls.
#[cfg(unix)]
fn fork_into_background(queue: Option<&Queue>) -> Result<ForkResult> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        bail!("Failed to create pipe for background process");
    }
    let [read_fd, write_fd] = fds;

    unsafe {
        match libc::fork() {
            -1 => bail!("Failed to fork background process"),
            0 => {
                // Child: wait until the parent has registered our ticket (or exited), then detach
                libc::close(write_fd);
                let mut buf = [0u8; 1];
                libc::read(read_fd, buf.as_mut_ptr().cast(), 1);
                libc::close(read_fd);

                libc::setsid();
                let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
                if devnull >= 0 {
                    libc::dup2(devnull, libc::STDOUT_FILENO);
                    libc::dup2(devnull, libc::STDERR_FILENO);
                    libc::close(devnull);
                }
                Ok(ForkResult::Child)
            }
            child => {
                libc::close(read_fd);
                let result = match queue {
                    Some(queue) => queue.enqueue(child as u32),
                    None => Ok(()),
                };
                libc::write(write_fd, [1u8].as_ptr().cast(), 1);
                libc::close(write_fd);
                result.map(|()| ForkResult::Parent)
            }
        }
    }
}

#[cfg(not(unix))]
fn fork_into_background(_queue: Option<&Queue>) -> Result<ForkResult> {
    bail!("--bg flag is only supported on Unix systems")
}
