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
    /// 1..=8 (8 = the octave). "1-2-3-5" is the classic opener.
    NoteSequence { degrees: Vec<u8> },
}

/// Why a recipe refused to compile — every message is written for the
/// player (the panel surfaces them verbatim).
#[derive(Debug, PartialEq, Eq)]
pub enum StarterError {
    Empty,
    BadDegree(u8),
    TooLong { notes: usize, cap: usize },
}

impl std::fmt::Display for StarterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StarterError::Empty => write!(f, "add a note or two first — then Begin"),
            StarterError::BadDegree(d) => {
                write!(f, "scale degrees run 1 to 8 — {d} isn't one")
            }
            StarterError::TooLong { notes, cap } => write!(
                f,
                "that opener has {notes} notes; {cap} is the ceiling — trim it a little"
            ),
        }
    }
}

/// Major-scale semitone offsets for degrees 1..=8.
const MAJOR_DEGREE_SEMITONES: [i8; 8] = [0, 2, 4, 5, 7, 9, 11, 12];

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
                    if !(1..=8).contains(&d) {
                        return Err(StarterError::BadDegree(d));
                    }
                    cell.push(MAJOR_DEGREE_SEMITONES[usize::from(d) - 1]);
                }
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

    /// Degree 0 and 9 refuse with the player-facing message naming the
    /// offender — a silent clamp would quietly reshape the music.
    #[test]
    fn out_of_range_degrees_refuse_by_name() {
        let err =
            composite_cell(&[StarterItem::NoteSequence { degrees: vec![9] }], 32).unwrap_err();
        assert_eq!(err, StarterError::BadDegree(9));
        assert!(err.to_string().contains('9'));
        let err =
            composite_cell(&[StarterItem::NoteSequence { degrees: vec![0] }], 32).unwrap_err();
        assert_eq!(err, StarterError::BadDegree(0));
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
}
