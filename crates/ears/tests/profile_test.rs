use ears::profile::{InstrumentProfile, ProfileError, ProfileLoader};
use std::path::{Path, PathBuf};

/// Helper to get the profiles directory (repo root / profiles)
fn profiles_dir() -> &'static Path {
    Path::new("profiles")
}

/// Create a unique temp directory for test isolation.
fn unique_temp_dir(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{}_{}_{}",
        test_name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn load_trumpet_profile_from_file() {
    let profile =
        InstrumentProfile::from_file(&profiles_dir().join("trumpet.json")).unwrap();
    assert_eq!(profile.name, "Trumpet");
    assert_eq!(profile.freq_min_hz, 165.0);
    assert_eq!(profile.freq_max_hz, 1047.0);
    assert_eq!(profile.vibrato_tolerance_cents, 20.0);
    assert_eq!(profile.tuning_corrections.len(), 2);
    assert_eq!(profile.tuning_corrections[0].cents_offset, 25.0);
}

#[test]
fn load_all_profiles_matches_file_count() {
    let profiles = ProfileLoader::load_all(profiles_dir()).unwrap();
    let json_count = std::fs::read_dir(profiles_dir())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .ok()
                .and_then(|e| e.path().extension().map(|ext| ext == "json"))
                .unwrap_or(false)
        })
        .count();
    assert_eq!(profiles.len(), json_count);
    assert!(profiles.len() >= 3, "Expected at least 3 profiles");
}

#[test]
fn load_by_name_finds_trumpet() {
    let profile = ProfileLoader::load_by_name(profiles_dir(), "trumpet").unwrap();
    assert_eq!(profile.name, "Trumpet");
}

#[test]
fn load_by_name_missing_instrument_errors() {
    let result = ProfileLoader::load_by_name(profiles_dir(), "kazoo");
    assert!(result.is_err());
}

#[test]
fn load_by_name_rejects_path_traversal() {
    let result = ProfileLoader::load_by_name(profiles_dir(), "../etc/passwd");
    assert!(matches!(result, Err(ProfileError::InvalidName(_))));
}

#[test]
fn load_by_name_rejects_empty_name() {
    let result = ProfileLoader::load_by_name(profiles_dir(), "");
    assert!(matches!(result, Err(ProfileError::InvalidName(_))));
}

#[test]
fn malformed_json_returns_error() {
    let temp_dir = unique_temp_dir("malformed");
    std::fs::create_dir_all(&temp_dir).unwrap();

    let bad_file = temp_dir.join("bad.json");
    std::fs::write(&bad_file, "{ not valid json }").unwrap();

    let result = InstrumentProfile::from_file(&bad_file);
    assert!(matches!(result, Err(ProfileError::Parse { .. })));

    std::fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn load_all_with_malformed_file_still_loads_valid_ones() {
    let temp_dir = unique_temp_dir("mixed");
    std::fs::create_dir_all(&temp_dir).unwrap();

    std::fs::copy(
        profiles_dir().join("trumpet.json"),
        temp_dir.join("trumpet.json"),
    )
    .unwrap();

    std::fs::write(temp_dir.join("broken.json"), "not json").unwrap();

    let profiles = ProfileLoader::load_all(&temp_dir).unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "Trumpet");

    std::fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn frequency_range_check_trumpet() {
    let profile =
        InstrumentProfile::from_file(&profiles_dir().join("trumpet.json")).unwrap();

    assert!(profile.is_in_frequency_range(440.0));
    assert!(profile.is_in_frequency_range(261.6));
    assert!(!profile.is_in_frequency_range(100.0));
    assert!(!profile.is_in_frequency_range(2000.0));
    assert!(profile.is_in_frequency_range(165.0));
    assert!(profile.is_in_frequency_range(1047.0));
}

#[test]
fn nonexistent_directory_returns_error() {
    let result = ProfileLoader::load_all(Path::new("/nonexistent/path"));
    assert!(matches!(result, Err(ProfileError::ReadDir { .. })));
}
