//! Musical fingerprint — the unified, session-level read of a student's
//! musicianship.
//!
//! Across Phase 4 we grew four independent session-level measurements (tone,
//! key/mode, intonation, groove) that were each carried as a separate `Option`
//! field on [`crate::session::SessionRecap`]. That worked, but it scattered the
//! "what we measured about this session" contract across four parallel shapes.
//!
//! [`MusicalFingerprint`] consolidates them into a single representation. It is
//! the one contract the upcoming personalization / cultural-relevance engine
//! consumes, so it stays deliberately small and central: four
//! confidence-gated dimensions, nothing more.
//!
//! # Evidence gates
//!
//! Each dimension is present **only when its evidence gate passed**. The gates
//! are the existing per-dimension aggregation rules in [`crate::coaching`]
//! (enough distinct pitch classes + confident fit for the key; enough notes for
//! intonation; enough onsets for groove; at least one toned phrase for tone).
//! A `None` dimension means "we did not measure this honestly", never "the
//! value was zero". Building code must reuse those gates — do not loosen them.

use serde::{Deserialize, Serialize};

/// How firmly the session's key may be stated (#316).
///
/// The live tracker wanders early in most sessions — that's real playing, not
/// a bug. The honesty rule is that the recap must not claim more certainty
/// than the tracking earned: a key that dominated the session is `Asserted`
/// ("G# major"); one that settled only late, or that the vote contested, is
/// `Leaning` ("leaning G# major toward the end"). A session that never
/// settled carries no key at all (`MusicalFingerprint::key` = `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyClaimStrength {
    /// The key held for a solid majority of the tracked session — state it
    /// plainly.
    Asserted,
    /// The key settled late or was contested — hedge every reference to it.
    Leaning,
}

/// The four session-level measurements of a student's musicianship, each
/// present only when its evidence gate passed.
///
/// This replaces the four scattered `session_*` fields that previously lived on
/// [`crate::session::SessionRecap`]. Every dimension is an `Option`: `Some`
/// when the session produced enough evidence to report it honestly, `None`
/// otherwise. Use [`MusicalFingerprint::is_empty`] to tell "nothing measured"
/// apart from "some dimensions measured".
///
/// `#[serde(default)]` on each field keeps the JSON forward-compatible: a
/// fingerprint serialised before a dimension existed still deserialises (the
/// missing dimension defaults to `None`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicalFingerprint {
    /// Session-level tone aggregate (mean of the phrases that carried tone),
    /// present when tone analysis ran over at least one phrase.
    #[serde(default)]
    pub tone: Option<tone::ToneDescriptor>,
    /// Session-level key/mode estimate over all phrases' pitches, present only
    /// when the fit was confident enough to state plainly.
    #[serde(default)]
    pub key: Option<theory::KeyEstimate>,
    /// How firmly the recap may state `key` (#316 display honesty): `Asserted`
    /// when the key dominated the session's live tracking, `Leaning` when it
    /// settled late or contested the vote. `None` on fingerprints serialised
    /// before this field existed — readers must treat that as `Asserted` (the
    /// legacy behavior) so old recaps don't retroactively hedge. Meaningful
    /// only when `key` is `Some`.
    #[serde(default)]
    pub key_claim: Option<KeyClaimStrength>,
    /// Session-level intonation summary (cents vs equal temperament + per-degree
    /// tuning tendencies), present only when enough notes were observed to
    /// report honestly.
    #[serde(default)]
    pub intonation: Option<theory::IntonationSummary>,
    /// Session-level groove descriptor (tempo, swing, timing consistency),
    /// present only when enough onsets were observed.
    #[serde(default)]
    pub groove: Option<groove::GrooveDescriptor>,
}

impl MusicalFingerprint {
    /// `true` when no dimension was measured (all four `None`). A recap with an
    /// empty fingerprint should carry `None` rather than `Some(empty)`, so the
    /// "nothing measured" case round-trips as the absence of a fingerprint.
    pub fn is_empty(&self) -> bool {
        self.tone.is_none()
            && self.key.is_none()
            && self.intonation.is_none()
            && self.groove.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tone() -> tone::ToneDescriptor {
        tone::ToneDescriptor {
            brightness: 0.6,
            warmth: 0.5,
            air_noise: 0.2,
            core_clarity: 0.8,
            vibrato_quality: 0.55,
        }
    }

    #[test]
    fn is_empty_is_true_only_when_all_dimensions_absent() {
        let empty = MusicalFingerprint {
            tone: None,
            key: None,
            key_claim: None,
            intonation: None,
            groove: None,
        };
        assert!(empty.is_empty());

        let with_tone = MusicalFingerprint {
            tone: Some(sample_tone()),
            ..empty.clone()
        };
        assert!(!with_tone.is_empty());
    }

    #[test]
    fn serde_roundtrips_and_legacy_json_defaults_to_none() {
        let fp = MusicalFingerprint {
            tone: Some(sample_tone()),
            key: None,
            key_claim: None,
            intonation: None,
            groove: None,
        };
        let json = serde_json::to_string(&fp).expect("serialize");
        let back: MusicalFingerprint = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, fp);

        // A fingerprint serialised before any dimension existed (all fields
        // absent) must still deserialise, defaulting every dimension to None.
        let legacy: MusicalFingerprint = serde_json::from_str("{}").expect("empty object loads");
        assert!(legacy.is_empty());

        // #316 AC6: a fingerprint persisted BEFORE key_claim existed — key
        // present, no claim field — still parses, with the claim defaulting
        // to None (readers treat that as the legacy asserted behavior).
        let pre_316 = r#"{"key":{"tonic":8,"mode":"ionian","confidence":0.7,"margin":0.2}}"#;
        let legacy: MusicalFingerprint =
            serde_json::from_str(pre_316).expect("pre-#316 fingerprint loads");
        assert!(legacy.key.is_some());
        assert_eq!(
            legacy.key_claim, None,
            "absent field defaults, never errors"
        );
    }
}
