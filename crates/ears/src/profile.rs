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
