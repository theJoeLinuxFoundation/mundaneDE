use std::path::Path;

use serde::Deserialize;
use tracing::warn;

use crate::animation::SpringParams;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MundaneConfig {
    pub reduced_motion: bool,
    pub displays: std::collections::BTreeMap<String, DisplayConfig>,
    pub spring: SpringSection,
    pub wm: WmSection,
    pub output: OutputSection,
    pub wallpaper: WallpaperSection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SpringSection {
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WmSection {
    pub attraction_radius: i32,
    /// Margin between a snapped window and the output edge. Accepts the legacy
    /// `gap_size` key as an alias for backward compatibility.
    #[serde(alias = "gap_size")]
    pub outer_gap: i32,
    /// Margin between two snapped windows. Independent of `outer_gap` so users
    /// can have generous breathing room from the screen edges while keeping
    /// windows close together.
    pub inner_gap: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OutputSection {
    pub width: i32,
    pub height: i32,
    /// Refresh rate in millihertz (e.g. 60000 = 60 Hz)
    pub refresh: i32,
}

/// How a wallpaper image is fitted to the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScalingMode {
    /// Scale to cover the whole output, cropping overflow (preserves aspect).
    #[serde(alias = "cover")]
    Fill,
    /// Scale to fit inside the output, letterboxing (preserves aspect).
    #[serde(alias = "contain")]
    Fit,
    /// Stretch to exactly the output size, ignoring aspect ratio.
    Stretch,
    /// Original size, centred on the output.
    Center,
    /// Original size, repeated to tile the output.
    Tile,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WallpaperSection {
    /// Path to a PNG or JPEG image. When `None`, use the built-in wallpaper.
    pub path: Option<String>,
    /// How the image is fitted to the output.
    pub mode: ScalingMode,
    /// Spring parameters driving the fade-in and crossfade of the wallpaper opacity.
    pub fade_stiffness: f64,
    pub fade_damping: f64,
    pub fade_mass: f64,
}

/// Preferred scanout mode and logical layout for a named connector.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayConfig {
    pub position: Option<[i32; 2]>,
    pub mode: Option<[i32; 3]>, // width, height, refresh in millihertz
    pub scale: Option<f64>,
    pub rotation: Option<u16>,
}

impl Default for MundaneConfig {
    fn default() -> Self {
        Self {
            reduced_motion: false,
            displays: Default::default(),
            spring: SpringSection::default(),
            wm: WmSection::default(),
            output: OutputSection::default(),
            wallpaper: WallpaperSection::default(),
        }
    }
}

impl Default for SpringSection {
    fn default() -> Self {
        Self {
            stiffness: 100.0,
            damping: 20.0,
            mass: 1.0,
        }
    }
}

impl Default for WmSection {
    fn default() -> Self {
        Self {
            attraction_radius: 24,
            outer_gap: 8,
            inner_gap: 8,
        }
    }
}

impl Default for OutputSection {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            refresh: 60_000,
        }
    }
}

impl Default for WallpaperSection {
    fn default() -> Self {
        Self {
            path: None,
            mode: ScalingMode::Fill,
            // zeta ~= 0.99: a smooth, near-critical fade with no visible overshoot.
            fade_stiffness: 50.0,
            fade_damping: 14.0,
            fade_mass: 1.0,
        }
    }
}

impl SpringSection {
    pub fn to_spring_params(&self) -> SpringParams {
        SpringParams {
            stiffness: self.stiffness,
            damping: self.damping,
            mass: self.mass,
        }
    }
}

impl WallpaperSection {
    /// Spring parameters for the wallpaper fade/crossfade animation.
    pub fn to_fade_params(&self) -> SpringParams {
        SpringParams {
            stiffness: self.fade_stiffness,
            damping: self.fade_damping,
            mass: self.fade_mass,
        }
    }
}

impl MundaneConfig {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let cfg: Self = toml::from_str(text)?;
        for spring in [
            cfg.spring.to_spring_params(),
            cfg.wallpaper.to_fade_params(),
        ] {
            anyhow::ensure!(
                spring.stiffness.is_finite()
                    && spring.stiffness > 0.
                    && spring.mass.is_finite()
                    && spring.mass > 0.
                    && spring.damping.is_finite()
                    && spring.damping >= 0.,
                "invalid spring parameters"
            );
        }
        anyhow::ensure!(
            [cfg.wm.attraction_radius, cfg.wm.outer_gap, cfg.wm.inner_gap]
                .into_iter()
                .all(|v| (0..=16384).contains(&v)),
            "invalid gap or attraction radius"
        );
        anyhow::ensure!(
            [cfg.output.width, cfg.output.height, cfg.output.refresh]
                .into_iter()
                .all(|n| n > 0),
            "invalid output mode"
        );
        for display in cfg.displays.values() {
            anyhow::ensure!(
                display.scale.is_none_or(crate::outputs::valid_scale)
                    && display.position.is_none_or(crate::outputs::valid_position)
                    && display
                        .rotation
                        .is_none_or(|r| crate::outputs::rotation(r).is_some())
                    && display.mode.is_none_or(|m| m.into_iter().all(|n| n > 0)),
                "invalid display layout"
            );
        }
        Ok(cfg)
    }

    /// Load config from `~/.config/mundane/config.toml`, or return defaults if absent.
    pub fn load(path: Option<&Path>) -> Self {
        let path = path
            .map(|p| p.to_path_buf())
            .or_else(|| dirs_path().map(|d| d.join("config.toml")));

        let Some(path) = path else {
            return Self::default();
        };

        match std::fs::read_to_string(&path) {
            Ok(contents) => match Self::parse(&contents) {
                Ok(config) => config,
                Err(e) => {
                    warn!("failed to parse {}: {e}, using defaults", path.display());
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }
}

/// Path the compositor watches for hot reloads. None on the rare system where neither
/// XDG_CONFIG_HOME nor HOME is set.
pub fn config_file_path() -> Option<std::path::PathBuf> {
    dirs_path().map(|d| d.join("config.toml"))
}

fn dirs_path() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .map(|p| p.join("mundane"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let toml = include_str!("../config.example.toml");
        let cfg: MundaneConfig = toml::from_str(toml).expect("example config should parse");
        assert_eq!(cfg.wallpaper.mode, ScalingMode::Fill);
        assert!(cfg.wallpaper.path.is_none());
        assert_eq!(cfg.output.refresh, 60_000);
    }

    #[test]
    fn wm_section_accepts_legacy_gap_size_alias() {
        let raw = "attraction_radius = 24\ngap_size = 10\n";
        let wm: WmSection = toml::from_str(raw).unwrap();
        assert_eq!(wm.outer_gap, 10);
        assert_eq!(wm.inner_gap, 8); // default
    }

    #[test]
    fn wm_section_reads_separate_gaps() {
        let raw = "attraction_radius = 24\nouter_gap = 12\ninner_gap = 4\n";
        let wm: WmSection = toml::from_str(raw).unwrap();
        assert_eq!(wm.outer_gap, 12);
        assert_eq!(wm.inner_gap, 4);
    }

    #[test]
    fn invalid_edits_are_rejected_before_live_state_changes() {
        for text in [
            "[spring]\nmass=0",
            "[spring]\ndamping=nan",
            "[wallpaper]\nfade_stiffness=inf",
            "[wm]\ninner_gap=-1",
            "[displays.Head]\nscale=0.1",
        ] {
            assert!(MundaneConfig::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn scaling_mode_aliases() {
        let cover: WallpaperSection = toml::from_str("mode = \"cover\"").unwrap();
        assert_eq!(cover.mode, ScalingMode::Fill);
        let contain: WallpaperSection = toml::from_str("mode = \"contain\"").unwrap();
        assert_eq!(contain.mode, ScalingMode::Fit);
    }
}
