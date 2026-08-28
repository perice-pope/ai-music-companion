//! Instrument profiles — loaded from JSON, no code changes needed to add instruments.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Errors that can occur when loading instrument profiles.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("failed to read profiles directory {path}: {source}")]
    ReadDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read profile {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse profile {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("no profiles loaded successfully. Errors:\n{}", errors.join("\n"))]
    NoProfilesLoaded { errors: Vec<String> },
    #[error("invalid profile name: {0}")]
    InvalidName(String),
    #[error("invalid profile data in {path}: {message}")]
    InvalidProfile { path: PathBuf, message: String },
}

/// An instrument profile defining detection parameters.
///
/// Profiles are the **single source of truth** for which instruments the
/// product supports. Adding an instrument = adding a JSON file, no code
/// changes. The desktop app's instrument selector, the backend's
/// validation, and the audio analysis pipeline all read from the same
/// profiles directory — so `name`, `family`, and the frequency range
/// stay authoritative.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentProfile {
    /// Display name (e.g., "Trumpet", "Voice")
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
    /// UI emoji (single glyph) — display-only. Lives on the profile so
    /// adding an instrument stays a one-file change. Defaults to empty
    /// string when a profile predates this field.
    #[serde(default)]
    pub emoji: String,
    /// Minimum pitch-detection confidence for an event to count as **voiced**
    /// (i.e. as practice). Breathy, vibrato-rich instruments like Voice detect
    /// at lower confidence than a piano, so a fixed gate silently dropped whole
    /// sung sessions (#185). Lower this for such instruments. Defaults to 0.5
    /// for profiles that predate this field.
    #[serde(default = "default_voiced_confidence_threshold")]
    pub voiced_confidence_threshold: f64,
}

/// Default voiced-confidence gate (0.5) for profiles that don't set one.
fn default_voiced_confidence_threshold() -> f64 {
    0.5
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentFamily {
    Brass,
    Voice,
    Strings,
    Woodwind,
    Keyboard,
}

impl InstrumentFamily {
    /// Human-readable, title-cased name used for UI badges and the
    /// `InstrumentInfo` IPC surface. The serde representation is
    /// snake_case (JSON on disk); this is the display form.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Brass => "Brass",
            Self::Voice => "Voice",
            Self::Strings => "Strings",
            Self::Woodwind => "Woodwind",
            Self::Keyboard => "Keyboard",
        }
    }
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
    pub fn from_file(path: &Path) -> Result<Self, ProfileError> {
        let contents = std::fs::read_to_string(path).map_err(|source| ProfileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let profile: Self =
            serde_json::from_str(&contents).map_err(|source| ProfileError::Parse {
                path: path.to_path_buf(),
                source,
            })?;

        // Semantic validation — reject well-typed but nonsensical profiles
        if !profile.freq_min_hz.is_finite()
            || !profile.freq_max_hz.is_finite()
            || profile.freq_min_hz < 0.0
            || profile.freq_max_hz < profile.freq_min_hz
        {
            return Err(ProfileError::InvalidProfile {
                path: path.to_path_buf(),
                message: format!(
                    "invalid frequency range: min={}, max={}",
                    profile.freq_min_hz, profile.freq_max_hz
                ),
            });
        }

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
    pub fn load_all(dir: &Path) -> Result<Vec<InstrumentProfile>, ProfileError> {
        let mut profiles = Vec::new();
        let mut errors = Vec::new();

        let entries = std::fs::read_dir(dir).map_err(|source| ProfileError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })?;

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(source) => {
                    errors.push(format!("{}: {}", dir.display(), source));
                    continue;
                }
            };
            let path = entry.path();

            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                match InstrumentProfile::from_file(&path) {
                    Ok(profile) => profiles.push(profile),
                    Err(e) => errors.push(format!("{}: {}", path.display(), e)),
                }
            }
        }

        if !errors.is_empty() && profiles.is_empty() {
            return Err(ProfileError::NoProfilesLoaded { errors });
        }

        Ok(profiles)
    }

    /// Load a single profile by instrument name from a directory.
    ///
    /// Name must contain only ASCII alphanumerics, hyphens, or underscores.
    pub fn load_by_name(dir: &Path, name: &str) -> Result<InstrumentProfile, ProfileError> {
        let valid = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !valid {
            return Err(ProfileError::InvalidName(name.to_string()));
        }
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

    /// The repo's `profiles/` directory, resolved from this crate's location.
    fn workspace_profiles_dir() -> std::path::PathBuf {
        // CARGO_MANIFEST_DIR is crates/ears — profiles/ lives at the workspace root.
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("ears crate has a parent")
            .parent()
            .expect("workspace root exists")
            .join("profiles")
    }

    /// Locks the real-file invariant for every workspace-shipped profile.
    ///
    /// The repo's `profiles/` directory is the source of truth for which
    /// instruments the product supports. If a JSON file is malformed, its
    /// frequency range nonsensical, or its family enum unknown, this test
    /// fails with the offending filename — preventing a silent regression
    /// where a new profile is checked in but fails to parse at runtime.
    ///
    /// Update the expected count below when you add or remove a profile.
    #[test]
    fn all_workspace_profiles_load_cleanly() {
        let profiles_dir = workspace_profiles_dir();

        let loaded =
            ProfileLoader::load_all(&profiles_dir).expect("every checked-in profile must parse");

        // `load_all` is lenient: individual parse errors are collected and it
        // still returns Ok when at least one profile loaded. That means a
        // malformed canonical file could be silently skipped if some other
        // `.json` happened to parse in its place. Cross-check the loaded
        // count against the number of JSON files on disk so any skipped
        // file fails the test, regardless of the expected-count assertion
        // below.
        let json_count = std::fs::read_dir(&profiles_dir)
            .expect("profiles dir should be readable")
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
            .count();
        assert_eq!(
            loaded.len(),
            json_count,
            "every checked-in JSON profile in {:?} should parse; loaded {} of {}",
            profiles_dir,
            loaded.len(),
            json_count,
        );

        // If you add or remove a profile file, update this count. The
        // explicit number guards against accidental deletion as strongly
        // as it guards against accidental addition.
        assert_eq!(
            loaded.len(),
            10,
            "expected 10 profiles in {:?}, got {:?}",
            profiles_dir,
            loaded.iter().map(|p| &p.name).collect::<Vec<_>>()
        );

        // Every expected instrument by display name must be present — the
        // filename-to-display-name mapping is silent otherwise.
        let names: Vec<&str> = loaded.iter().map(|p| p.name.as_str()).collect();
        for expected in &[
            "Trumpet",
            "Trombone",
            "French Horn",
            "Violin",
            "Cello",
            "Flute",
            "Clarinet",
            "Voice",
            "Piano",
        ] {
            assert!(
                names.contains(expected),
                "missing profile {expected} in {:?}",
                names
            );
        }
    }

    /// A piano has 88 keys, A0 (27.5 Hz) to C8 (4186 Hz). The shipped
    /// profile's floor must admit A0 — it was 28 Hz for a while, which
    /// silently excluded the bottom key from every range check and from
    /// the #471-4 fold window (the row could never reach the real A0).
    /// The floor is also pinned snug: a floor deeper than A0 would let
    /// the range claim notes no piano has.
    #[test]
    fn piano_profile_spans_the_full_88_keys() {
        let loaded = ProfileLoader::load_all(&workspace_profiles_dir())
            .expect("workspace profiles must load");
        let piano = loaded
            .iter()
            .find(|p| p.name == "Piano")
            .expect("Piano profile ships with the workspace");

        assert!(
            piano.is_in_frequency_range(27.5),
            "A0 (27.5 Hz) must be in the piano's range; floor is {} Hz",
            piano.freq_min_hz
        );
        assert!(
            piano.is_in_frequency_range(4186.0),
            "C8 (4186 Hz) must be in the piano's range; ceiling is {} Hz",
            piano.freq_max_hz
        );
        assert!(
            !piano.is_in_frequency_range(27.4),
            "below A0 is not a piano note; floor is {} Hz",
            piano.freq_min_hz
        );
    }
}
