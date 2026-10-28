//! `vvsay --setup`: downloads VOICEVOX CORE assets with the official downloader.
//!
//! The downloader asks the user to accept the terms of use of the ONNX Runtime and the voice
//! models, reading the answer from stdin. By default stdin is passed through, so the user answers
//! in the terminal (or pipes the answer in). vvsay answers "y" only when the user explicitly
//! passes `--accept-terms`.

use anyhow::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};
use std::io::{self, Read as _, Write as _};
use std::os::fd::AsFd as _;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;

use crate::assets::Assets;

/// Version of the official downloader. Keep in sync with the `voicevox_core` tag in Cargo.toml,
/// and update [`DOWNLOADERS`] together (digests are listed on the GitHub release).
const DOWNLOADER_VERSION: &str = "0.17.1";

/// (target_os, target_arch, asset name, SHA-256)
const DOWNLOADERS: [(&str, &str, &str, &str); 4] = [
    (
        "macos",
        "aarch64",
        "download-osx-arm64",
        "0ccc1c5260a9210e105134c58761dc6ae0f11b7a489de25d74408d45a2055e02",
    ),
    (
        "macos",
        "x86_64",
        "download-osx-x64",
        "4a25e607492f4868058ef2f7947f0ca0a9d5f79acd937317cd88013d63800d6d",
    ),
    (
        "linux",
        "aarch64",
        "download-linux-arm64",
        "2f54707b8969ef6b65d5afdcededf0809a53f73548efeffefac93c840ae0e4cc",
    ),
    (
        "linux",
        "x86_64",
        "download-linux-x64",
        "4ba6dc22bfacab5dd1a7cc0dc074ca714d920526ce90813aec06dc86531ddd8f",
    ),
];

/// Returns the downloader asset name and its SHA-256 for this platform.
fn downloader_asset() -> Result<(&'static str, &'static str)> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    DOWNLOADERS
        .iter()
        .find(|(o, a, _, _)| *o == os && *a == arch)
        .map(|&(_, _, name, sha256)| (name, sha256))
        .with_context(|| {
            format!(
                "vvsay --setup does not support {os}/{arch}. See README for manual installation."
            )
        })
}

fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<()> {
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected_hex {
        bail!("Checksum mismatch for the downloader (expected {expected_hex}, got {actual})");
    }
    Ok(())
}

fn confirm(question: &str) -> Result<bool> {
    eprint!("{question} [y/N] ");
    io::stderr().flush()?;
    let answer = read_line_unbuffered()?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// Reads one line from stdin byte by byte. `io::stdin()` is buffered and could consume input
/// meant for the downloader (e.g. `printf 'y\ny\n' | vvsay --setup`), so only this line is read.
fn read_line_unbuffered() -> Result<String> {
    let mut stdin = std::fs::File::from(io::stdin().as_fd().try_clone_to_owned()?);
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while stdin.read(&mut byte)? == 1 && byte[0] != b'\n' {
        line.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

/// Runs the downloader answering "y" to its terms-of-use prompt (asked once for all terms).
///
/// The downloader shows the terms in a pager that reads keys from the terminal even when stdin is
/// piped, which would block. The pager (minus) prints the text and returns when stdout is not a
/// terminal, so stdout is piped through vvsay: the terms are still shown and logged. Progress
/// bars and the prompt go to stderr, which stays attached.
fn run_accepting_terms(mut command: Command) -> Result<ExitStatus> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to run the downloader")?;
    let mut stdout = child
        .stdout
        .take()
        .context("Downloader stdout is not piped")?;
    let forward = thread::spawn(move || io::copy(&mut stdout, &mut io::stdout()));
    // Closing stdin afterwards makes any further prompt fail instead of waiting forever.
    child
        .stdin
        .take()
        .context("Downloader stdin is not piped")?
        .write_all(b"y\n")?;
    let status = child.wait()?;
    let _ = forward.join();
    Ok(status)
}

pub fn run_setup(assets_dir: &Path, accept_terms: bool) -> Result<()> {
    if Assets::locate(assets_dir).is_ok()
        && !confirm(&format!(
            "VOICEVOX CORE assets are already installed in {}. Download them again?",
            assets_dir.display()
        ))?
    {
        return Ok(());
    }

    let (asset, sha256) = downloader_asset()?;
    let workdir = tempfile::tempdir()?;
    let downloader = workdir.path().join("download");
    let url = format!(
        "https://github.com/VOICEVOX/voicevox_core/releases/download/{DOWNLOADER_VERSION}/{asset}"
    );

    eprintln!("Fetching the VOICEVOX CORE downloader {DOWNLOADER_VERSION} ({asset})...");
    let status = Command::new("curl")
        .args(["-fSL", "--proto", "=https", "-o"])
        .arg(&downloader)
        .arg(&url)
        .status()
        .context("Failed to run curl. Install curl, or see README for manual installation.")?;
    if !status.success() {
        bail!("Failed to download {url} ({status})");
    }
    verify_sha256(&std::fs::read(&downloader)?, sha256)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&downloader, std::fs::Permissions::from_mode(0o755))?;
    }

    let mut command = Command::new(&downloader);
    command
        .args(["--only", "onnxruntime", "dict", "models"])
        .args(["--models-pattern", "[0-9]*.vvm"])
        .arg("-o")
        .arg(assets_dir);
    let status = if accept_terms {
        eprintln!("Running the downloader, accepting the terms of use (--accept-terms).\n");
        run_accepting_terms(command)?
    } else {
        eprintln!(
            "Running the downloader. Read the terms of use and answer the prompt yourself.\n"
        );
        command.status().context("Failed to run the downloader")?
    };
    if !status.success() {
        bail!(
            "The downloader failed ({status}).\n\
             If GitHub's rate limit was hit, run again with: GH_TOKEN=$(gh auth token) vvsay --setup"
        );
    }

    let assets = Assets::locate(assets_dir)?;
    eprintln!(
        "\nSetup complete: {} voice models in {}",
        assets.vvms.len(),
        assets_dir.display()
    );
    eprintln!(
        "Next: `vvsay --list-narrator` to see voices, then `vvsay -n ずんだもん \"こんにちは\"`."
    );
    eprintln!("Set `default_preset` in ~/.config/vvsay/config.toml to omit -n.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_is_verified() {
        // SHA-256 of "abc"
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify_sha256(b"abc", abc).is_ok());
        assert!(verify_sha256(b"abd", abc).is_err());
    }

    #[test]
    fn downloader_exists_for_this_platform() {
        if cfg!(any(target_os = "macos", target_os = "linux"))
            && cfg!(any(target_arch = "aarch64", target_arch = "x86_64"))
        {
            let (name, sha256) = downloader_asset().unwrap();
            assert!(name.starts_with("download-"));
            assert_eq!(sha256.len(), 64);
        }
    }
}
