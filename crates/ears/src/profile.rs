//! Instrument profiles — loaded from JSON, no code changes needed to add instruments.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// An instrument profile defining detection parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentProfile {
    /// Display name (e.g., "Trumpet", "Soprano Voice")
    pub name: String,
    /// Instrument family
    pub family: InstrumentFamily,
    /// Minimum expected frequency in Hz
    pub freq_min_hz: f64,
    /// Maximum expected frequency in Hz
    pub freq_max_hz: f64,
    /// Vibrato tolerance in cents
    pub vibrato_tolerance_cents: f64,
    /// Expected attack type
    pub attack_type: AttackType,
    /// Tuning corrections for known intonation quirks
    #[serde(default)]
    pub tuning_corrections: Vec<TuningCorrection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentFamily {
    Brass,
    Voice,
    Strings,
    Woodwind,
    Keyboard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttackType {
    Tongued,
    Bowed,
    Breath,
    Struck,
    Plucked,
}

/// A known intonation correction for specific fingerings/positions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuningCorrection {
    /// Description of when this applies (e.g., "1+3 valve combination")
    pub description: String,
    /// Cent offset to apply (positive = sharp, negative = flat)
    pub cents_offset: f64,
}

impl InstrumentProfile {
    /// Load an instrument profile from a JSON file.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let profile: Self = serde_json::from_str(&contents)?;
        Ok(profile)
    }

    /// Check if a detected frequency falls within this instrument's expected range.
    pub fn is_in_frequency_range(&self, hz: f64) -> bool {
        hz >= self.freq_min_hz && hz <= self.freq_max_hz
    }
}

/// Loads all instrument profiles from a directory of JSON files.
pub struct ProfileLoader;

impl ProfileLoader {
    /// Load all `.json` files from the given directory as `InstrumentProfile`s.
    ///
    /// Returns an error if the directory can't be read. Individual file parse
    /// errors are collected — a single bad file won't prevent loading the rest.
    pub fn load_all(dir: &Path) -> anyhow::Result<Vec<InstrumentProfile>> {
        let mut profiles = Vec::new();
        let mut errors = Vec::new();

        let entries = std::fs::read_dir(dir)
            .map_err(|e| anyhow::anyhow!("Failed to read profiles directory {:?}: {}", dir, e))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                match InstrumentProfile::from_file(&path) {
                    Ok(profile) => profiles.push(profile),
                    Err(e) => errors.push(format!("{}: {}", path.display(), e)),
                }
            }
        }

        if !errors.is_empty() && profiles.is_empty() {
            anyhow::bail!(
                "Failed to load any profiles. Errors:\n{}",
                errors.join("\n")
            );
        }

        Ok(profiles)
    }

    /// Load a single profile by instrument name from a directory.
    pub fn load_by_name(dir: &Path, name: &str) -> anyhow::Result<InstrumentProfile> {
        let file_path = dir.join(format!("{}.json", name.to_lowercase()));
        InstrumentProfile::from_file(&file_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trumpet_profile() {
        let json = r#"{
            "name": "Trumpet",
            "family": "brass",
            "freq_min_hz": 165.0,
            "freq_max_hz": 1047.0,
            "vibrato_tolerance_cents": 20.0,
            "attack_type": "tongued",
            "tuning_corrections": [
                {
                    "description": "1+3 valve combination",
                    "cents_offset": 25.0
                }
            ]
        }"#;
        let profile: InstrumentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.name, "Trumpet");
        assert_eq!(profile.tuning_corrections.len(), 1);
    }
}
