use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::theme::IconMode;

#[derive(Debug, Clone, Deserialize)]
pub struct Preset {
    pub label: String,
    pub command: String,
    /// Optional. When set, it merges into the keymap so a single key can load it.
    #[serde(default)]
    pub key: Option<String>,
}

/// `~/.config/git-foreach/config.toml`. Every field is optional.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Globs (`fqdn/user/repo`) of repos checked by default at startup.
    pub default_checked: Vec<String>,
    pub icons: IconMode,
    /// Key string → Action name.
    pub keymap: HashMap<String, String>,
    #[serde(rename = "preset")]
    pub presets: Vec<Preset>,
}

impl Config {
    /// Load the config. Missing / broken falls back to defaults (broken returns a message).
    pub fn load() -> (Config, Option<String>) {
        let path = config_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return (Config::default(), None), // absent is normal
        };
        match toml::from_str::<Config>(&text) {
            Ok(cfg) => (cfg, None),
            Err(e) => (
                Config::default(),
                Some(format!("⚠ config parse error: {e}")),
            ),
        }
    }
}

fn config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("git-foreach/config.toml");
    }
    dirs::home_dir()
        .unwrap_or_default()
        .join(".config/git-foreach/config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::IconMode;

    #[test]
    fn parses_full_config() {
        let toml = r#"
            default_checked = ["github.com/m5d215/*"]
            icons = "ascii"

            [keymap]
            r = "rescan"
            c = "cancel"

            [[preset]]
            label = "pull"
            command = "git pull --ff-only"
            key = "p"

            [[preset]]
            label = "status"
            command = "git status -s"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.default_checked, vec!["github.com/m5d215/*"]);
        assert_eq!(cfg.icons, IconMode::Ascii);
        assert_eq!(cfg.keymap.get("r").map(String::as_str), Some("rescan"));
        assert_eq!(cfg.presets.len(), 2);
        assert_eq!(cfg.presets[0].command, "git pull --ff-only");
        assert_eq!(cfg.presets[0].key.as_deref(), Some("p"));
    }

    #[test]
    fn empty_config_is_default() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.default_checked.is_empty());
        assert_eq!(cfg.icons, IconMode::Nerd);
        assert!(cfg.presets.is_empty());
    }
}
