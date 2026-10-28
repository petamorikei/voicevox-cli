use anyhow::{Context as _, Result, bail};
use std::path::{Path, PathBuf};
use voicevox_core::{
    CharacterMeta, StyleId, StyleMeta, StyleType, blocking::Onnxruntime, blocking::VoiceModelFile,
};

pub const SETUP_GUIDE: &str = "\
VOICEVOX CORE assets (ONNX Runtime, Open JTalk dictionary, voice models) are required.
Run `vvsay --setup` to download them (you will be asked to accept the terms of use).
See README for manual installation.";

/// Files required to run VOICEVOX CORE.
#[derive(Debug, Clone)]
pub struct Assets {
    pub onnxruntime: PathBuf,
    pub dict: PathBuf,
    pub vvms: Vec<PathBuf>,
}

/// A voice chosen by the user: either a raw style ID or a character name with an optional style name.
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceSelector {
    StyleId(u32),
    Name {
        narrator: String,
        style: Option<String>,
    },
}

/// A resolved voice and the VVM file that contains it.
#[derive(Debug, Clone)]
pub struct ResolvedVoice {
    pub style_id: StyleId,
    pub vvm: PathBuf,
    pub label: String,
}

pub fn default_assets_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    Ok(home.join(".local").join("share").join("vvsay"))
}

/// Expands a leading `~/` so that `assets_dir` in the config file can be written naturally.
pub fn expand_tilde(path: &str) -> Result<PathBuf> {
    match path.strip_prefix("~/") {
        Some(rest) => Ok(dirs::home_dir()
            .context("Could not find home directory")?
            .join(rest)),
        None => Ok(PathBuf::from(path)),
    }
}

impl Assets {
    pub fn locate(root: &Path) -> Result<Self> {
        let missing = |what: &str| {
            anyhow::anyhow!("{what} not found under {}\n\n{SETUP_GUIDE}", root.display())
        };

        let onnxruntime = find_onnxruntime(&root.join("onnxruntime").join("lib"))
            .ok_or_else(|| missing("ONNX Runtime library"))?;
        let dict = find_dict(&root.join("dict")).ok_or_else(|| missing("Open JTalk dictionary"))?;
        let vvms = find_vvms(&root.join("models").join("vvms"));
        if vvms.is_empty() {
            return Err(missing("Voice models (*.vvm)"));
        }

        Ok(Self {
            onnxruntime,
            dict,
            vvms,
        })
    }

    /// Reads character metadata from every VVM without loading the models.
    /// Characters spread across several VVMs are merged into one entry.
    pub fn characters(&self) -> Result<Vec<(CharacterMeta, Vec<PathBuf>)>> {
        let mut characters: Vec<(CharacterMeta, Vec<PathBuf>)> = Vec::new();
        for path in &self.vvms {
            let vvm = VoiceModelFile::open(path)
                .with_context(|| format!("Failed to open voice model {}", path.display()))?;
            for meta in vvm.metas().iter() {
                let talk_styles: Vec<StyleMeta> = meta
                    .styles
                    .iter()
                    .filter(|style| is_talk(style))
                    .cloned()
                    .collect();
                if talk_styles.is_empty() {
                    continue;
                }
                match characters
                    .iter_mut()
                    .find(|(c, _)| c.speaker_uuid == meta.speaker_uuid)
                {
                    Some((character, paths)) => {
                        character.styles.extend(talk_styles);
                        paths.push(path.clone());
                    }
                    None => {
                        let mut character = meta.clone();
                        character.styles = talk_styles;
                        characters.push((character, vec![path.clone()]));
                    }
                }
            }
        }
        Ok(characters)
    }

    pub fn resolve(&self, selector: &VoiceSelector) -> Result<ResolvedVoice> {
        for path in &self.vvms {
            let vvm = VoiceModelFile::open(path)
                .with_context(|| format!("Failed to open voice model {}", path.display()))?;
            for character in vvm.metas().iter() {
                let found =
                    character
                        .styles
                        .iter()
                        .filter(|s| is_talk(s))
                        .find(|s| match selector {
                            VoiceSelector::StyleId(id) => s.id.0 == *id,
                            VoiceSelector::Name { narrator, style } => {
                                &character.name == narrator
                                    && style.as_ref().is_none_or(|name| &s.name == name)
                            }
                        });
                if let Some(style) = found {
                    return Ok(ResolvedVoice {
                        style_id: style.id,
                        vvm: path.clone(),
                        label: format!("{} ({})", character.name, style.name),
                    });
                }
            }
        }

        match selector {
            VoiceSelector::StyleId(id) => {
                bail!("Unknown style ID: {id}. Use --list-narrator to see available voices.")
            }
            VoiceSelector::Name {
                narrator,
                style: None,
            } => {
                bail!("Unknown narrator: {narrator}. Use --list-narrator to see available voices.")
            }
            VoiceSelector::Name {
                narrator,
                style: Some(style),
            } => bail!(
                "Unknown voice: {narrator} ({style}). Use --list-style \"{narrator}\" to see available styles."
            ),
        }
    }
}

fn is_talk(style: &StyleMeta) -> bool {
    matches!(style.r#type, StyleType::Talk | StyleType::StreamingTalk)
}

fn read_dir_paths(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default()
}

/// The downloader may fetch a different ONNX Runtime version than the one voicevox_core prefers,
/// so the library is looked up by prefix instead of by exact filename.
fn find_onnxruntime(lib_dir: &Path) -> Option<PathBuf> {
    let recommended = lib_dir.join(Onnxruntime::LIB_RECOMMENDED_VERSIONED_FILENAME);
    if recommended.exists() {
        return Some(recommended);
    }
    let prefix = format!("{}voicevox_onnxruntime", std::env::consts::DLL_PREFIX);
    let mut candidates: Vec<PathBuf> = read_dir_paths(lib_dir)
        .into_iter()
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with(&prefix) && n.contains(std::env::consts::DLL_EXTENSION)
            })
        })
        .collect();
    candidates.sort();
    candidates.pop()
}

fn find_dict(dict_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = read_dir_paths(dict_dir)
        .into_iter()
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("open_jtalk_dic"))
        })
        .collect();
    candidates.sort();
    candidates.pop()
}

/// Returns VVM files ordered by their numeric file stem (0.vvm, 1.vvm, ..., 10.vvm).
fn find_vvms(vvm_dir: &Path) -> Vec<PathBuf> {
    let mut vvms: Vec<PathBuf> = read_dir_paths(vvm_dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "vvm"))
        .collect();
    vvms.sort_by_key(|p| {
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        (stem.parse::<u64>().unwrap_or(u64::MAX), stem.to_string())
    });
    vvms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vvms_are_sorted_numerically() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["10.vvm", "2.vvm", "0.vvm", "s0.vvm", "readme.txt"] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        let names: Vec<String> = find_vvms(dir.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["0.vvm", "2.vvm", "10.vvm", "s0.vvm"]);
    }

    #[test]
    fn onnxruntime_is_found_by_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let name = format!(
            "{}voicevox_onnxruntime.1.17.3.{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_EXTENSION
        );
        std::fs::write(dir.path().join(&name), b"").unwrap();
        assert_eq!(find_onnxruntime(dir.path()), Some(dir.path().join(name)));
    }

    #[test]
    fn missing_assets_mention_setup() {
        let dir = tempfile::tempdir().unwrap();
        let err = Assets::locate(dir.path()).unwrap_err().to_string();
        assert!(err.contains("vvsay --setup"));
    }
}
