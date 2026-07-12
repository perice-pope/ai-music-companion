//! #349 T4a — the jam-along chord chart: what the room played, as a timed
//! label sequence.
//!
//! Fed once per perception snapshot (~8 Hz) with what the strip is showing;
//! records a new entry only when the picture CHANGES — a labeled chord's
//! (root, quality) identity, or a transition into/out of the honest
//! "hearing several notes…" state. The result is the session's chord chart
//! sketch: labels with timestamps and confidences for the recap, and the
//! rolling lane's source of truth.
//!
//! Honesty rules carried through from T1: unresolved polyphony is recorded
//! AS unresolved (never a fabricated label), confidence rides every labeled
//! entry so the UI can draw its dots, and nothing here retains audio —
//! labels and timestamps only, fully offline.

use serde::{Deserialize, Serialize};

use crate::perception::PerceptionSnapshot;
use theory::ChordQuality;

/// Hard cap on chart length — hours of jamming stay bounded (~an entry per
/// chord change; 600 ≈ a long set). Oldest entries drop first: the recap
/// sketch is "what the session sounded like", and the END of a jam is the
/// part the player was just in.
const CHART_CAP: usize = 600;

/// One chart moment: a labeled chord, or an honest unresolved stretch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartEntry {
    /// Display label ("Bbmaj7", "C7/E") — empty for unresolved entries.
    pub label: String,
    /// Root pitch class for the RV colour accent; `None` when unresolved.
    pub root_pc: Option<u8>,
    /// Matched quality — the tap-to-row bridge's template key.
    pub quality: Option<ChordQuality>,
    /// Label confidence 0–1 at the moment it was recorded (0 = unresolved).
    pub confidence: f32,
    /// Session time the entry began, seconds.
    pub at_secs: f64,
    /// True for the "hearing several notes…" state — shown honestly in the
    /// lane and the sketch, never upgraded to a guess.
    pub unresolved: bool,
}

/// Records chart entries from the live perception stream.
#[derive(Debug, Default)]
pub struct ChartRecorder {
    entries: Vec<ChartEntry>,
    /// Identity of the last recorded state, for change detection:
    /// `Some(Some(..))` = labeled chord, `Some(None)` = unresolved,
    /// `None` = silence / single-line (nothing recorded).
    last: Option<Option<(u8, ChordQuality)>>,
}

impl ChartRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold in one perception snapshot at `now_secs`.
    pub fn observe(&mut self, snapshot: &PerceptionSnapshot, now_secs: f64) {
        let state: Option<Option<(u8, ChordQuality)>> =
            match (&snapshot.chord, snapshot.hearing_polyphony) {
                (Some(c), _) => c.quality.map(|q| Some((c.root_pc, q))),
                (None, true) => Some(None),
                (None, false) => None,
            };
        if state == self.last {
            // Same picture: refresh the confidence of a still-ringing label
            // (the tracker's number firms up as the chord sustains).
            if let (Some(Some(_)), Some(c), Some(last_entry)) =
                (&state, &snapshot.chord, self.entries.last_mut())
            {
                if !last_entry.unresolved {
                    last_entry.confidence = c.confidence;
                    // The freshest display label wins too — a slash arriving
                    // mid-ring ("C" → "C/E") must not leave the chart naming
                    // a different chord than the strip showed (review N2).
                    if last_entry.label != c.label {
                        last_entry.label.clone_from(&c.label);
                    }
                }
            }
            return;
        }
        self.last = state;
        let entry = match (&snapshot.chord, snapshot.hearing_polyphony) {
            // A labeled chord WITHOUT a quality key is a wire-compat state
            // (old producer), unreachable live — both the identity match
            // above and this arm treat it as silence so the recorder can
            // never disagree with itself (or the lane, which skips it too).
            (Some(c), _) if c.quality.is_none() => return,
            (Some(c), _) => ChartEntry {
                label: c.label.clone(),
                root_pc: Some(c.root_pc),
                quality: c.quality,
                confidence: c.confidence,
                at_secs: now_secs,
                unresolved: false,
            },
            (None, true) => ChartEntry {
                label: String::new(),
                root_pc: None,
                quality: None,
                confidence: 0.0,
                at_secs: now_secs,
                unresolved: true,
            },
            // Silence / single line: a state change worth remembering for
            // dedup (so the same chord re-entered later records again), but
            // not a chart entry — a rest is the absence of a label.
            (None, false) => return,
        };
        self.entries.push(entry);
        if self.entries.len() > CHART_CAP {
            let overflow = self.entries.len() - CHART_CAP;
            self.entries.drain(..overflow);
        }
    }

    /// The chart so far, oldest first.
    pub fn entries(&self) -> &[ChartEntry] {
        &self.entries
    }

    /// Forget everything (new session).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perception::ChordReading;

    fn labeled(root_pc: u8, quality: ChordQuality, label: &str, conf: f32) -> PerceptionSnapshot {
        PerceptionSnapshot {
            chord: Some(ChordReading {
                root_pc,
                quality: Some(quality),
                label: label.to_owned(),
                bass_pc: None,
                confidence: conf,
            }),
            ..PerceptionSnapshot::EMPTY
        }
    }

    fn unresolved() -> PerceptionSnapshot {
        PerceptionSnapshot {
            hearing_polyphony: true,
            ..PerceptionSnapshot::EMPTY
        }
    }

    /// #349 T4a AC1 (chart layer): a 4-chord jam records exactly 4 labeled
    /// entries in play order with their timestamps, however many snapshots
    /// each chord rings for. Fails if dedup breaks (per-frame spam) or
    /// order/timing drifts.
    #[test]
    fn a_four_chord_jam_records_four_timed_entries() {
        let mut r = ChartRecorder::new();
        let chords = [
            (0u8, ChordQuality::Maj7, "Cmaj7"),
            (9, ChordQuality::Min7, "Am7"),
            (2, ChordQuality::Min7, "Dm7"),
            (7, ChordQuality::Dom7, "G7"),
        ];
        let mut t = 0.0;
        for (root, q, label) in chords {
            for _ in 0..12 {
                // ~1.5 s of ringing per chord at the 8 Hz snapshot cadence.
                r.observe(&labeled(root, q, label, 0.8), t);
                t += 0.125;
            }
        }
        let e = r.entries();
        assert_eq!(e.len(), 4, "one entry per chord change: {e:?}");
        let labels: Vec<&str> = e.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(labels, ["Cmaj7", "Am7", "Dm7", "G7"]);
        assert!(e.windows(2).all(|w| w[0].at_secs < w[1].at_secs));
        assert!(e.iter().all(|x| !x.unresolved && x.confidence > 0.0));
        assert_eq!(e[3].quality, Some(ChordQuality::Dom7), "tap-to-row key");
    }

    /// #349 T4a AC2: dense/atonal stretches record HONEST unresolved
    /// entries — never a fabricated label — and one entry per stretch, not
    /// per frame. Silence records nothing.
    #[test]
    fn atonal_stretches_record_honest_unresolved_entries() {
        let mut r = ChartRecorder::new();
        for i in 0..8 {
            r.observe(&unresolved(), i as f64 * 0.125);
        }
        r.observe(&PerceptionSnapshot::EMPTY, 1.1); // silence
        r.observe(&labeled(0, ChordQuality::Maj, "C", 0.9), 1.5);
        for i in 0..8 {
            r.observe(&unresolved(), 2.0 + i as f64 * 0.125);
        }
        let e = r.entries();
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(e[0].unresolved && e[0].label.is_empty() && e[0].confidence == 0.0);
        assert_eq!(e[1].label, "C");
        assert!(e[2].unresolved, "second stretch is its own entry");
    }

    /// A chord re-entered after silence records again (the chart is a
    /// timeline, not a set), and a ringing label's confidence refreshes in
    /// place as the tracker firms up.
    #[test]
    fn re_entry_records_and_confidence_refreshes() {
        let mut r = ChartRecorder::new();
        r.observe(&labeled(0, ChordQuality::Maj, "C", 0.55), 0.0);
        r.observe(&labeled(0, ChordQuality::Maj, "C", 0.9), 0.5);
        assert_eq!(r.entries().len(), 1);
        assert_eq!(r.entries()[0].confidence, 0.9, "refreshed in place");
        r.observe(&PerceptionSnapshot::EMPTY, 1.0); // release
        r.observe(&labeled(0, ChordQuality::Maj, "C", 0.8), 2.0);
        assert_eq!(r.entries().len(), 2, "re-entry is a new timeline moment");
    }

    /// A slash arriving mid-ring ("C" → "C/E", same root+quality) refreshes
    /// the entry's LABEL in place — the chart must never name a different
    /// chord than the strip showed (review M3, recorder side). And a
    /// quality-less labeled reading (wire-compat state) is silence in BOTH
    /// arms — recorded nowhere, breaking no dedup.
    #[test]
    fn slash_refreshes_in_place_and_quality_none_is_silence() {
        let mut r = ChartRecorder::new();
        r.observe(&labeled(0, ChordQuality::Maj, "C", 0.7), 0.0);
        r.observe(&labeled(0, ChordQuality::Maj, "C/E", 0.9), 0.4);
        assert_eq!(r.entries().len(), 1, "same identity: no duplicate");
        assert_eq!(r.entries()[0].label, "C/E", "freshest display label wins");
        assert_eq!(r.entries()[0].confidence, 0.9);

        // quality-None: dropped consistently (both arms), and it does not
        // count as a state change that would dedup-block the next chord.
        let mut r = ChartRecorder::new();
        let mut no_quality = labeled(0, ChordQuality::Maj, "C", 0.8);
        no_quality.chord.as_mut().unwrap().quality = None;
        r.observe(&no_quality, 0.0);
        assert!(r.entries().is_empty(), "quality-less label records nothing");
        r.observe(&labeled(0, ChordQuality::Maj, "C", 0.8), 0.5);
        assert_eq!(r.entries().len(), 1, "the real reading still lands");
    }

    /// The cap drops the OLDEST entries — a marathon jam keeps its ending.
    #[test]
    fn the_cap_keeps_the_end_of_a_marathon() {
        let mut r = ChartRecorder::new();
        for i in 0..(CHART_CAP + 50) {
            let root = (i % 12) as u8;
            let label = format!("chord-{i}");
            r.observe(&labeled(root, ChordQuality::Maj, &label, 0.7), i as f64);
            r.observe(&PerceptionSnapshot::EMPTY, i as f64 + 0.5);
        }
        assert_eq!(r.entries().len(), CHART_CAP);
        assert_eq!(
            r.entries().last().unwrap().label,
            format!("chord-{}", CHART_CAP + 49),
            "the newest entry survives"
        );
        r.clear();
        assert!(r.entries().is_empty());
    }
}
