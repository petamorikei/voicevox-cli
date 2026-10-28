use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::assets::VoiceSelector;
use crate::engine::VoiceParams;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoicePreset {
    pub name: String,
    pub narrator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(flatten)]
    pub params: VoiceParams,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets_dir: Option<String>,
    #[serde(default)]
    pub presets: Vec<VoicePreset>,
}

/// Voice settings given on the command line.
#[derive(Debug, Default)]
pub struct VoiceOverrides {
    pub preset: Option<String>,
    pub narrator: Option<String>,
    pub style: Option<String>,
    pub style_id: Option<u32>,
    pub params: VoiceParams,
}

pub fn get_config_path() -> Result<PathBuf> {
    let home_dir = dirs::home_dir().context("Could not find home directory")?;
    let config_dir = home_dir.join(".config").join("vvsay");

    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }

    Ok(config_dir.join("config.toml"))
}

pub fn load_config() -> Result<Config> {
    let config_path = get_config_path()?;

    if !config_path.exists() {
        let default_config = Config::default();
        save_config(&default_config)?;
        return Ok(default_config);
    }

    let content = fs::read_to_string(&config_path)?;
    toml::from_str(&content).with_context(|| format!("Failed to parse {}", config_path.display()))
}

pub fn save_config(config: &Config) -> Result<()> {
    let config_path = get_config_path()?;
    let content = toml::to_string_pretty(config)?;
    fs::write(&config_path, content)?;
    Ok(())
}

impl Config {
    fn find_preset(&self, name: &str) -> Option<&VoicePreset> {
        self.presets.iter().find(|preset| preset.name == name)
    }

    /// Resolves the voice and parameters with the precedence: CLI > preset > VOICEVOX defaults.
    /// The preset is the one given by `-p`, or `default_preset` when `-p` is omitted.
    pub fn resolve_voice(
        &self,
        overrides: &VoiceOverrides,
    ) -> Result<(VoiceSelector, VoiceParams)> {
        let preset = match &overrides.preset {
            Some(name) => Some(self.find_preset(name).with_context(|| {
                format!("Unknown preset: {name}. Use --list-presets to see available presets.")
            })?),
            None => match &self.default_preset {
                Some(name) => {
                    let preset = self.find_preset(name);
                    if preset.is_none() {
                        eprintln!("Warning: default_preset \"{name}\" is not defined in presets");
                    }
                    preset
                }
                None => None,
            },
        };

        let params = overrides
            .params
            .or(preset.map(|p| p.params).unwrap_or_default());

        if let Some(id) = overrides.style_id {
            return Ok((VoiceSelector::StyleId(id), params));
        }

        let narrator = overrides
            .narrator
            .clone()
            .or_else(|| preset.map(|p| p.narrator.clone()));
        let Some(narrator) = narrator else {
            bail!("No narrator specified. Use --narrator, --style-id, or configure a preset.");
        };
        let style = overrides
            .style
            .clone()
            .or_else(|| preset.and_then(|p| p.style.clone()));

        Ok((VoiceSelector::Name { narrator, style }, params))
    }
}

pub fn list_presets(config: &Config) {
    println!("Available presets:");
    for preset in &config.presets {
        let marker = if Some(&preset.name) == config.default_preset.as_ref() {
            " (default)"
        } else {
            ""
        };
        let style = preset.style.as_deref().unwrap_or("default style");
        let params = preset.params.describe();
        let params = if params.is_empty() {
            String::new()
        } else {
            format!(", {params}")
        };
        println!(
            "  {} - {} ({}{}){}",
            preset.name, preset.narrator, style, params, marker
        );
    }

    if let Some(ref default) = config.default_preset {
        println!("\nDefault preset: {}", default);
    } else {
        println!("\nNo default preset set");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
default_preset = "zundamon"

[[presets]]
name = "zundamon"
narrator = "ずんだもん"
style = "ノーマル"
speed = 1.1
pitch = 0.05

[[presets]]
name = "metan"
narrator = "四国めたん"
"#;

    fn sample() -> Config {
        toml::from_str(SAMPLE).unwrap()
    }

    fn name(narrator: &str, style: Option<&str>) -> VoiceSelector {
        VoiceSelector::Name {
            narrator: narrator.to_string(),
            style: style.map(str::to_string),
        }
    }

    #[test]
    fn parses_preset_params() {
        let config = sample();
        assert_eq!(config.presets[0].params.speed, Some(1.1));
        assert_eq!(config.presets[0].params.pitch, Some(0.05));
        assert_eq!(config.presets[1].style, None);
        assert_eq!(config.presets[1].params, VoiceParams::default());
    }

    #[test]
    fn default_preset_is_used_without_options() {
        let (voice, params) = sample().resolve_voice(&VoiceOverrides::default()).unwrap();
        assert_eq!(voice, name("ずんだもん", Some("ノーマル")));
        assert_eq!(params.speed, Some(1.1));
    }

    #[test]
    fn cli_overrides_preset_fields() {
        let overrides = VoiceOverrides {
            style: Some("あまあま".into()),
            params: VoiceParams {
                speed: Some(1.5),
                ..Default::default()
            },
            ..Default::default()
        };
        let (voice, params) = sample().resolve_voice(&overrides).unwrap();
        assert_eq!(voice, name("ずんだもん", Some("あまあま")));
        assert_eq!(params.speed, Some(1.5));
        assert_eq!(params.pitch, Some(0.05));
    }

    #[test]
    fn explicit_preset_replaces_default() {
        let overrides = VoiceOverrides {
            preset: Some("metan".into()),
            ..Default::default()
        };
        let (voice, params) = sample().resolve_voice(&overrides).unwrap();
        assert_eq!(voice, name("四国めたん", None));
        assert_eq!(params, VoiceParams::default());
    }

    #[test]
    fn style_id_wins_over_names() {
        let overrides = VoiceOverrides {
            style_id: Some(3),
            ..Default::default()
        };
        let (voice, params) = sample().resolve_voice(&overrides).unwrap();
        assert_eq!(voice, VoiceSelector::StyleId(3));
        assert_eq!(params.speed, Some(1.1));
    }

    #[test]
    fn unknown_preset_is_an_error() {
        let overrides = VoiceOverrides {
            preset: Some("nope".into()),
            ..Default::default()
        };
        assert!(sample().resolve_voice(&overrides).is_err());
    }

    #[test]
    fn narrator_is_required_without_preset() {
        let err = Config::default()
            .resolve_voice(&VoiceOverrides::default())
            .unwrap_err();
        assert!(err.to_string().contains("No narrator specified"));
    }
}
