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
/// `Leaning` ("leaning G# major toward the end"); one that carried the
/// session but lost the strip by the close is `Drifted` ("mostly G# major —
/// wandering by the end", #404); a session whose tracking never firmed up
/// enough to claim any key is `Unsettled` ("the key kept moving") — and one
/// that produced no tonal readings at all carries no claim whatsoever
/// (percussive material isn't "moving", it's silent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyClaimStrength {
    /// The key held for a solid majority of the tracked session — state it
    /// plainly.
    Asserted,
    /// The key settled late or was contested, and it IS where the strip
    /// ended — hedge every reference to it, anchored to the session's close.
    Leaning,
    /// The key carried the session's tracking mass, but the live reading had
    /// wandered off it by session close (#404) — a whole-session claim only;
    /// never phrase it as where the session ended.
    Drifted,
    /// Tonal readings existed but never firmed up into a claimable key —
    /// say the key kept moving. `MusicalFingerprint::key` is `None`.
    Unsettled,
}

impl KeyClaimStrength {
    /// `true` for the hedged claims (`Leaning`, `Drifted`) — a hedged key
    /// must never be stated as, or build on, a flat fact (mode-named
    /// flavours, "sat firmly" strengths, degree attributions).
    pub fn hedged(self) -> bool {
        matches!(self, Self::Leaning | Self::Drifted)
    }
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
    /// settled late or contested the vote, `Drifted` when it carried the
    /// session but the strip wandered off it by the close (#404), `Unsettled`
    /// (with `key` = `None`) when readings existed but never firmed into a
    /// claim. `None` on
    /// fingerprints serialised before this field existed — readers must treat
    /// that as `Asserted` (the legacy behavior) so old recaps don't
    /// retroactively hedge — and on sessions with no tonal readings at all.
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

    /// #404 AC7 — the drifted claim's wire form, and its hedged contract:
    /// `"drifted"` round-trips, and `hedged()` covers exactly the two
    /// claims that must never read as flat fact. Fails if a rename breaks
    /// persisted recaps or a new consumer treats `Drifted` as asserted.
    #[test]
    fn drifted_claim_round_trips_and_is_hedged() {
        let json = serde_json::to_string(&KeyClaimStrength::Drifted).expect("serialize");
        assert_eq!(json, "\"drifted\"");
        let back: KeyClaimStrength = serde_json::from_str("\"drifted\"").expect("deserialize");
        assert_eq!(back, KeyClaimStrength::Drifted);

        assert!(KeyClaimStrength::Drifted.hedged());
        assert!(KeyClaimStrength::Leaning.hedged());
        assert!(!KeyClaimStrength::Asserted.hedged());
        assert!(!KeyClaimStrength::Unsettled.hedged());
    }
}
