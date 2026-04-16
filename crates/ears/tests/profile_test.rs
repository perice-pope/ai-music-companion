use ears::profile::{InstrumentProfile, ProfileLoader};
use std::path::Path;

/// Helper to get the profiles directory (repo root / profiles)
fn profiles_dir() -> &'static Path {
    // Tests run from the workspace root, so profiles/ is relative to that
    Path::new("profiles")
}

#[test]
fn load_trumpet_profile_from_file() {
    let profile = InstrumentProfile::from_file(&profiles_dir().join("trumpet.json")).unwrap();
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
    // We have trumpet.json, voice.json, violin.json
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
fn malformed_json_returns_error() {
    let temp_dir = std::env::temp_dir().join("profile_test_malformed");
    std::fs::create_dir_all(&temp_dir).unwrap();

    let bad_file = temp_dir.join("bad.json");
    std::fs::write(&bad_file, "{ not valid json }").unwrap();

    let result = InstrumentProfile::from_file(&bad_file);
    assert!(result.is_err());

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn load_all_with_malformed_file_still_loads_valid_ones() {
    let temp_dir = std::env::temp_dir().join("profile_test_mixed");
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Copy a valid profile
    std::fs::copy(profiles_dir().join("trumpet.json"), temp_dir.join("trumpet.json")).unwrap();

    // Add a malformed file
    std::fs::write(temp_dir.join("broken.json"), "not json").unwrap();

    let profiles = ProfileLoader::load_all(&temp_dir).unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "Trumpet");

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn frequency_range_check_trumpet() {
    let profile = InstrumentProfile::from_file(&profiles_dir().join("trumpet.json")).unwrap();

    // A4 (440 Hz) is within trumpet range
    assert!(profile.is_in_frequency_range(440.0));

    // Middle C (261 Hz) is within trumpet range
    assert!(profile.is_in_frequency_range(261.6));

    // Below trumpet range
    assert!(!profile.is_in_frequency_range(100.0));

    // Above trumpet range
    assert!(!profile.is_in_frequency_range(2000.0));

    // Exact boundaries
    assert!(profile.is_in_frequency_range(165.0));
    assert!(profile.is_in_frequency_range(1047.0));
}

#[test]
fn nonexistent_directory_returns_error() {
    let result = ProfileLoader::load_all(Path::new("/nonexistent/path"));
    assert!(result.is_err());
}
