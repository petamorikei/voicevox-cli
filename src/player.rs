use anyhow::{Context as _, Result, bail};
use std::io::Write as _;
use std::process::{Child, ChildStdin, Command, Stdio};

use crate::wav::PcmFormat;

pub fn check_mpv_available() -> Result<()> {
    let available = Command::new("mpv")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !available {
        bail!(
            "mpv is required for playback. Install it (e.g. `brew install mpv`) or use -o to save to a file."
        );
    }
    Ok(())
}

/// An mpv process that plays raw PCM streamed through its stdin, so that sentences can be
/// appended while earlier ones are already playing.
pub struct Player {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Player {
    pub fn spawn(format: PcmFormat) -> Result<Self> {
        if format.bits_per_sample != 16 {
            bail!("Unsupported sample size: {} bits", format.bits_per_sample);
        }
        let mut child = Command::new("mpv")
            .arg("--no-video")
            .arg("--really-quiet")
            // VOICEVOX outputs mono audio. On some devices mpv's coreaudio output
            // fails with mono and falls back to avfoundation, which cuts off the
            // beginning and end of playback. Upmixing to stereo keeps coreaudio.
            .arg("--audio-channels=stereo")
            .arg("--demuxer=rawaudio")
            .arg(format!("--demuxer-rawaudio-rate={}", format.sample_rate))
            .arg(format!("--demuxer-rawaudio-channels={}", format.channels))
            .arg("--demuxer-rawaudio-format=s16le")
            .arg("-")
            .stdin(Stdio::piped())
            .spawn()
            .context("Failed to start mpv")?;
        let stdin = child.stdin.take();
        Ok(Self { child, stdin })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn write(&mut self, pcm: &[u8]) -> Result<()> {
        let stdin = self.stdin.as_mut().context("Player is already finished")?;
        stdin
            .write_all(pcm)
            .context("mpv stopped before playback finished")
    }

    /// Closes the stream and waits until mpv has played everything.
    pub fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        if !status.success() {
            bail!("mpv failed to play audio");
        }
        Ok(())
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Reached only on error paths: stop playback instead of leaving mpv running.
        if self.stdin.is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
