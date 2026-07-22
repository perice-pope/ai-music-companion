//! Session Starters (#419 S1) — the RV builder's recipe layer.
//!
//! A [`StarterRecipe`] is an ordered list of items the player composes in
//! the Openers panel; it compiles to ONE composite cell (the RV practice
//! unit) which the existing explore engine rows through 12 keys. S1 ships
//! the first two item kinds — explicit notes and scale-degree sequences —
//! matching the two enabled entries of the RV builder's menu; the rest of
//! the bank (intervals, chords, scales, enclosures, pattern directions)
//! arrives in S2 on the same shape.

use serde::{Deserialize, Serialize};

/// #419 S2a: chord kinds the bank offers — a curated subset of
/// [`theory::ChordQuality`]; the intervals come from THAT table (the
/// authoritative source), never re-declared here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StarterChord {
    MajorTriad,
    MinorTriad,
    DominantSeventh,
    MajorSeventh,
    MinorSeventh,
}

impl StarterChord {
    fn quality(self) -> theory::ChordQuality {
        use theory::ChordQuality as Q;
        match self {
            StarterChord::MajorTriad => Q::Maj,
            StarterChord::MinorTriad => Q::Min,
            StarterChord::DominantSeventh => Q::Dom7,
            StarterChord::MajorSeventh => Q::Maj7,
            StarterChord::MinorSeventh => Q::Min7,
        }
    }
}

/// #419 S2a: scale kinds the bank offers — a curated subset of
/// [`variations::ScaleType`]; semitones come from that catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StarterScale {
    Major,
    NaturalMinor,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
    Dorian,
    Mixolydian,
}

impl StarterScale {
    fn scale_type(self) -> variations::ScaleType {
        use variations::ScaleType as S;
        match self {
            StarterScale::Major => S::Major,
            StarterScale::NaturalMinor => S::NaturalMinor,
            StarterScale::MajorPentatonic => S::MajorPentatonic,
            StarterScale::MinorPentatonic => S::MinorPentatonic,
            StarterScale::Blues => S::Blues,
            StarterScale::Dorian => S::Dorian,
            StarterScale::Mixolydian => S::Mixolydian,
        }
    }
}

/// #419 S2a: enclosure styles offered as bank items — approach notes
/// then the root, offsets from [`variations::Enclosure`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StarterEnclosure {
    OneDownOneUp,
    OneUpOneDown,
}

impl StarterEnclosure {
    fn enclosure(self) -> variations::Enclosure {
        match self {
            StarterEnclosure::OneDownOneUp => variations::Enclosure::OneDownOneUp,
            StarterEnclosure::OneUpOneDown => variations::Enclosure::OneUpOneDown,
        }
    }
}

/// One buildable item in an opener recipe (#419). Wire shape is tagged so
/// the S2 item kinds extend without breaking stored recipes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StarterItem {
    /// Explicit notes as semitone offsets from the CELL's first note —
    /// the same convention as a lifted lick's cell, so `[0, 4, 7]` is a
    /// root-position major triad arpeggio wherever the row lands.
    Notes { offsets: Vec<i8> },
    /// A scale-degree sequence in the major scale of the cell's key:
    /// 1..=12, where 8 = the octave and 9..=12 keep walking the scale
    /// into the second octave with the compound-interval reading (9th =
    /// 14 semitones, 10th = 16, 11th = 17, 12th = 19). "1-2-3-5" is the
    /// classic opener; "1-2-3-5-8-12" rides the extension (#471 pt 3).
    ///
    /// Degrees are DIATONIC — steps of the major scale. The chromatic
    /// gesture (the panel's simple 12-note picker, the 12-tone row) is
    /// NOT degrees: it speaks [`StarterItem::Notes`] semitone offsets,
    /// which already say anything chromatic.
    NoteSequence { degrees: Vec<u8> },
    /// #419 S2a: a major-scale degree interval from the root — number
    /// 2..=8 ("a 3rd", "a 5th") compiles to root + that degree.
    Interval { number: u8 },
    /// #419 S2a: an ascending arpeggio of the named chord quality.
    Chord { kind: StarterChord },
    /// #419 S2a: the named scale as an ascending run, root to octave.
    Scale { kind: StarterScale },
    /// #419 S2a: approach notes then the root — the RV enclosure as an
    /// opener item (negative leading offsets are legal, like descending
    /// openers).
    Enclosure { style: StarterEnclosure },
}

/// Why a recipe refused to compile — every message is written for the
/// player (the panel surfaces them verbatim).
#[derive(Debug, PartialEq, Eq)]
pub enum StarterError {
    Empty,
    BadDegree(u8),
    BadInterval(u8),
    TooLong { notes: usize, cap: usize },
}

impl std::fmt::Display for StarterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StarterError::Empty => write!(f, "add a note or two first — then Begin"),
            StarterError::BadDegree(d) => {
                write!(f, "scale degrees run 1 to 12 — {d} isn't one")
            }
            StarterError::BadInterval(n) => {
                write!(
                    f,
                    "intervals run a 2nd to an octave (2 to 8) — {n} isn't one"
                )
            }
            StarterError::TooLong { notes, cap } => write!(
                f,
                "that opener has {notes} notes; {cap} is the ceiling — trim it a little"
            ),
        }
    }
}

/// Major-scale semitone offsets for degrees 1..=12 — one octave plus the
/// compound extension (9th, 10th, 11th, 12th), the octave-extension
/// semantics [`StarterItem::NoteSequence`] documents (#471 pt 3).
const MAJOR_DEGREE_SEMITONES: [i8; 12] = [0, 2, 4, 5, 7, 9, 11, 12, 14, 16, 17, 19];

/// Compile a recipe's items into one composite cell: semitone offsets from
/// the cell's first note, in play order — the exact wire shape
/// `coach::start_explore_cell` rows through 12 keys. `cap` bounds the total
/// note count (callers pass [`crate::coach::LIFT_MAX_NOTES`]).
pub fn composite_cell(items: &[StarterItem], cap: usize) -> Result<Vec<i8>, StarterError> {
    let mut cell: Vec<i8> = Vec::new();
    for item in items {
        match item {
            StarterItem::Notes { offsets } => cell.extend_from_slice(offsets),
            StarterItem::NoteSequence { degrees } => {
                for &d in degrees {
                    if !(1..=12).contains(&d) {
                        return Err(StarterError::BadDegree(d));
                    }
                    cell.push(MAJOR_DEGREE_SEMITONES[usize::from(d) - 1]);
                }
            }
            StarterItem::Interval { number } => {
                if !(2..=8).contains(number) {
                    return Err(StarterError::BadInterval(*number));
                }
                cell.push(0);
                cell.push(MAJOR_DEGREE_SEMITONES[usize::from(*number) - 1]);
            }
            // Delegation, not duplication: the interval formulas live in
            // theory (chords) and variations (scales, enclosures).
            StarterItem::Chord { kind } => {
                cell.extend(kind.quality().intervals().iter().map(|&s| s as i8));
            }
            StarterItem::Scale { kind } => {
                cell.extend(kind.scale_type().semitones().iter().map(|&s| s as i8));
                cell.push(12);
            }
            StarterItem::Enclosure { style } => {
                // approach_semitones() is already &[i8] — no cast.
                cell.extend_from_slice(style.enclosure().approach_semitones());
                cell.push(0);
            }
        }
    }
    if cell.is_empty() {
        return Err(StarterError::Empty);
    }
    if cell.len() > cap {
        return Err(StarterError::TooLong {
            notes: cell.len(),
            cap,
        });
    }
    Ok(cell)
}

/// A named, saveable opener recipe (S1 persists name + items; recall UI
/// lands with S4's "yesterday's opener").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StarterRecipe {
    pub name: String,
    pub items: Vec<StarterItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Items CONCATENATE in play order into one cell — the RV builder's
    /// append-onto-the-staff semantics.
    #[test]
    fn items_concatenate_in_order() {
        let cell = composite_cell(
            &[
                StarterItem::NoteSequence {
                    degrees: vec![1, 2, 3],
                },
                StarterItem::Notes {
                    offsets: vec![7, 4, 0],
                },
            ],
            32,
        )
        .unwrap();
        assert_eq!(cell, vec![0, 2, 4, 7, 4, 0]);
    }

    /// Degree mapping is the major scale, 1-based, with 8 as the octave —
    /// the "1-2-3-5" and "1-3-5-8" openers must mean what a musician means.
    #[test]
    fn degrees_map_to_the_major_scale() {
        let cell = composite_cell(
            &[StarterItem::NoteSequence {
                degrees: vec![1, 3, 5, 8],
            }],
            32,
        )
        .unwrap();
        assert_eq!(cell, vec![0, 4, 7, 12]);
    }

    /// Degree 0 and 13 refuse with the player-facing message naming the
    /// offender — a silent clamp would quietly reshape the music.
    #[test]
    fn out_of_range_degrees_refuse_by_name() {
        let err =
            composite_cell(&[StarterItem::NoteSequence { degrees: vec![13] }], 32).unwrap_err();
        assert_eq!(err, StarterError::BadDegree(13));
        assert!(err.to_string().contains("13"));
        assert!(err.to_string().contains("1 to 12"));
        let err =
            composite_cell(&[StarterItem::NoteSequence { degrees: vec![0] }], 32).unwrap_err();
        assert_eq!(err, StarterError::BadDegree(0));
    }

    /// #471 pt 3: degrees 9..=12 keep walking the major scale into the
    /// second octave — the compound-interval reading (9th = 14, 10th =
    /// 16, 11th = 17, 12th = 19). The founder's "1-2-3-5-8-12" compiles;
    /// a wrong table here ships wrong music with every suite green.
    #[test]
    fn degrees_extend_through_the_second_octave() {
        let cell = composite_cell(
            &[StarterItem::NoteSequence {
                degrees: vec![8, 9, 10, 11, 12],
            }],
            32,
        )
        .unwrap();
        assert_eq!(cell, vec![12, 14, 16, 17, 19]);
        let cell = composite_cell(
            &[StarterItem::NoteSequence {
                degrees: vec![1, 2, 3, 5, 8, 12],
            }],
            32,
        )
        .unwrap();
        assert_eq!(cell, vec![0, 2, 4, 7, 12, 19]);
    }

    /// An empty recipe refuses calmly (the Begin button's polite no).
    #[test]
    fn empty_recipe_refuses_calmly() {
        assert_eq!(composite_cell(&[], 32).unwrap_err(), StarterError::Empty);
        // Items that compile to nothing count as empty too.
        assert_eq!(
            composite_cell(&[StarterItem::Notes { offsets: vec![] }], 32).unwrap_err(),
            StarterError::Empty
        );
    }

    /// The lift cap applies to the COMPOSITE, not per item — an opener is
    /// still a cell, and cells have a ceiling.
    #[test]
    fn the_cap_bounds_the_composite() {
        let long = StarterItem::Notes {
            offsets: vec![0; 20],
        };
        let err = composite_cell(&[long.clone(), long], 32).unwrap_err();
        assert_eq!(err, StarterError::TooLong { notes: 40, cap: 32 });
    }

    /// Negative offsets (below the cell's first note) are legal notes —
    /// descending openers exist.
    #[test]
    fn descending_offsets_are_legal() {
        let cell = composite_cell(
            &[StarterItem::Notes {
                offsets: vec![0, -3, -5],
            }],
            32,
        )
        .unwrap();
        assert_eq!(cell, vec![0, -3, -5]);
    }

    /// The PANEL's exact wire JSON deserializes (review MF4): a
    /// `#[serde(rename)]` on any field survived every suite because the
    /// impl tests construct enums directly and the frontend mocks invoke.
    /// This literal string is what OpenersPanel actually sends — if it
    /// stops parsing, every tap in the app is broken, so this must be red.
    #[test]
    fn panel_wire_json_deserializes() {
        let items: Vec<StarterItem> = serde_json::from_str(
            r#"[{"type":"notes","offsets":[4]},{"type":"note_sequence","degrees":[1,2,3,5]}]"#,
        )
        .unwrap();
        assert_eq!(
            items,
            vec![
                StarterItem::Notes { offsets: vec![4] },
                StarterItem::NoteSequence {
                    degrees: vec![1, 2, 3, 5],
                },
            ]
        );
        assert_eq!(composite_cell(&items, 32).unwrap(), vec![4, 0, 2, 4, 7]);
    }

    /// The wire shape round-trips (stored recipes must survive releases).
    #[test]
    fn recipe_serde_round_trips() {
        let recipe = StarterRecipe {
            name: "morning thirds".into(),
            items: vec![
                StarterItem::NoteSequence {
                    degrees: vec![1, 3, 2, 4],
                },
                StarterItem::Notes { offsets: vec![0] },
            ],
        };
        let json = serde_json::to_string(&recipe).unwrap();
        assert!(json.contains("note_sequence"), "tagged wire shape: {json}");
        let back: StarterRecipe = serde_json::from_str(&json).unwrap();
        assert_eq!(back, recipe);
    }

    // ── #419 S2a: the item bank ─────────────────────────────────────────

    /// Each bank kind compiles to the AUTHORITATIVE table's offsets —
    /// delegation pinned against theory/variations, not re-derived here
    /// by copying the same constants twice.
    #[test]
    fn bank_items_delegate_to_the_authoritative_tables() {
        // Interval: a 5th = root + 7.
        assert_eq!(
            composite_cell(&[StarterItem::Interval { number: 5 }], 32).unwrap(),
            vec![0, 7]
        );
        // Chords: exactly ChordQuality::intervals(), in order.
        for (kind, quality) in [
            (StarterChord::MajorTriad, theory::ChordQuality::Maj),
            (StarterChord::MinorTriad, theory::ChordQuality::Min),
            (StarterChord::DominantSeventh, theory::ChordQuality::Dom7),
            (StarterChord::MajorSeventh, theory::ChordQuality::Maj7),
            (StarterChord::MinorSeventh, theory::ChordQuality::Min7),
        ] {
            let cell = composite_cell(&[StarterItem::Chord { kind }], 32).unwrap();
            let expected: Vec<i8> = quality.intervals().iter().map(|&s| s as i8).collect();
            assert_eq!(cell, expected, "{kind:?}");
        }
        // Scales: catalog semitones + the octave — EXHAUSTIVE (review MF3:
        // a swapped mapping on an untested variant shipped wrong music
        // with every suite green).
        for (kind, scale) in [
            (StarterScale::Major, variations::ScaleType::Major),
            (
                StarterScale::NaturalMinor,
                variations::ScaleType::NaturalMinor,
            ),
            (
                StarterScale::MajorPentatonic,
                variations::ScaleType::MajorPentatonic,
            ),
            (
                StarterScale::MinorPentatonic,
                variations::ScaleType::MinorPentatonic,
            ),
            (StarterScale::Blues, variations::ScaleType::Blues),
            (StarterScale::Dorian, variations::ScaleType::Dorian),
            (StarterScale::Mixolydian, variations::ScaleType::Mixolydian),
        ] {
            let cell = composite_cell(&[StarterItem::Scale { kind }], 32).unwrap();
            let mut expected: Vec<i8> = scale.semitones().iter().map(|&s| s as i8).collect();
            expected.push(12);
            assert_eq!(cell, expected, "{kind:?}");
        }
        // Enclosures: approach offsets then the target root — both styles.
        for (style, enclosure) in [
            (
                StarterEnclosure::OneDownOneUp,
                variations::Enclosure::OneDownOneUp,
            ),
            (
                StarterEnclosure::OneUpOneDown,
                variations::Enclosure::OneUpOneDown,
            ),
        ] {
            let cell = composite_cell(&[StarterItem::Enclosure { style }], 32).unwrap();
            let mut expected: Vec<i8> = enclosure.approach_semitones().to_vec();
            expected.push(0);
            assert_eq!(cell, expected, "{style:?}");
        }
        // Interval boundaries: 2 (first slot) and 8 (last slot — the
        // off-by-one mutation panics out of bounds here).
        assert_eq!(
            composite_cell(&[StarterItem::Interval { number: 2 }], 32).unwrap(),
            vec![0, 2]
        );
        assert_eq!(
            composite_cell(&[StarterItem::Interval { number: 8 }], 32).unwrap(),
            vec![0, 12]
        );
    }

    /// Out-of-range intervals refuse by name, like degrees do.
    #[test]
    fn bad_intervals_refuse_by_name() {
        for n in [0u8, 1, 9] {
            let err = composite_cell(&[StarterItem::Interval { number: n }], 32).unwrap_err();
            assert_eq!(err, StarterError::BadInterval(n));
            assert!(err.to_string().contains("2 to 8"));
        }
    }

    /// Bank items mix with S1 items in play order under the same cap.
    #[test]
    fn bank_items_compose_with_s1_items() {
        let cell = composite_cell(
            &[
                StarterItem::Enclosure {
                    style: StarterEnclosure::OneDownOneUp,
                },
                StarterItem::Chord {
                    kind: StarterChord::MajorTriad,
                },
                StarterItem::NoteSequence { degrees: vec![8] },
            ],
            32,
        )
        .unwrap();
        assert_eq!(cell, vec![-1, 1, 0, 0, 4, 7, 12]);
    }

    /// The PANEL's wire JSON for every S2a kind deserializes — same
    /// end-to-end pin as S1's (a serde rename breaks every tap silently).
    #[test]
    fn s2a_panel_wire_json_deserializes() {
        // EVERY kind literal the panel can send (review MF2: a serde
        // rename on any of the 14 shipped silently when only one variant
        // per enum was pinned).
        let json = r#"[
            {"type":"interval","number":3},
            {"type":"chord","kind":"major_triad"},
            {"type":"chord","kind":"minor_triad"},
            {"type":"chord","kind":"dominant_seventh"},
            {"type":"chord","kind":"major_seventh"},
            {"type":"chord","kind":"minor_seventh"},
            {"type":"scale","kind":"major"},
            {"type":"scale","kind":"natural_minor"},
            {"type":"scale","kind":"major_pentatonic"},
            {"type":"scale","kind":"minor_pentatonic"},
            {"type":"scale","kind":"blues"},
            {"type":"scale","kind":"dorian"},
            {"type":"scale","kind":"mixolydian"},
            {"type":"enclosure","style":"one_down_one_up"},
            {"type":"enclosure","style":"one_up_one_down"}]"#;
        let items: Vec<StarterItem> = serde_json::from_str(json).unwrap();
        assert_eq!(items.len(), 15);
        assert_eq!(items[0], StarterItem::Interval { number: 3 });
        let chords: Vec<StarterChord> = items[1..6]
            .iter()
            .map(|i| match i {
                StarterItem::Chord { kind } => *kind,
                other => panic!("expected chord, got {other:?}"),
            })
            .collect();
        assert_eq!(
            chords,
            vec![
                StarterChord::MajorTriad,
                StarterChord::MinorTriad,
                StarterChord::DominantSeventh,
                StarterChord::MajorSeventh,
                StarterChord::MinorSeventh,
            ]
        );
        let scales: Vec<StarterScale> = items[6..13]
            .iter()
            .map(|i| match i {
                StarterItem::Scale { kind } => *kind,
                other => panic!("expected scale, got {other:?}"),
            })
            .collect();
        assert_eq!(
            scales,
            vec![
                StarterScale::Major,
                StarterScale::NaturalMinor,
                StarterScale::MajorPentatonic,
                StarterScale::MinorPentatonic,
                StarterScale::Blues,
                StarterScale::Dorian,
                StarterScale::Mixolydian,
            ]
        );
        assert_eq!(
            items[13],
            StarterItem::Enclosure {
                style: StarterEnclosure::OneDownOneUp,
            }
        );
        assert_eq!(
            items[14],
            StarterItem::Enclosure {
                style: StarterEnclosure::OneUpOneDown,
            }
        );
    }

    /// S1's stored wire shapes still parse — the new arms are additive.
    #[test]
    fn s1_wire_shapes_still_deserialize() {
        let items: Vec<StarterItem> = serde_json::from_str(
            r#"[{"type":"notes","offsets":[4]},{"type":"note_sequence","degrees":[1,2,3,5]}]"#,
        )
        .unwrap();
        assert_eq!(items.len(), 2);
        // And a recipe holding every arm round-trips.
        let recipe = StarterRecipe {
            name: "everything".into(),
            items: vec![
                StarterItem::Interval { number: 6 },
                StarterItem::Chord {
                    kind: StarterChord::MinorSeventh,
                },
                StarterItem::Scale {
                    kind: StarterScale::Mixolydian,
                },
                StarterItem::Enclosure {
                    style: StarterEnclosure::OneDownOneUp,
                },
            ],
        };
        let json = serde_json::to_string(&recipe).unwrap();
        let back: StarterRecipe = serde_json::from_str(&json).unwrap();
        assert_eq!(back, recipe);
    }
}
