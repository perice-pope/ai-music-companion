//! On-device Optical Music Recognition: a scanned or PDF-exported piece of
//! **sheet music** turned into **MusicXML** — the single format the renderer
//! and score follower understand.
//!
//! OMR is the notation counterpart to the [`transcribe`] crate's audio path:
//! a different *front-end* that produces the same MusicXML, so the Tauri import
//! command can feed the result straight into the existing
//! `list_parts` → "which part?" picker → library flow shipped for MusicXML
//! import. (See `docs/architecture/score-import-and-transcription.md`.)
//!
//! ## Honesty contract
//!
//! OMR is approximate. This crate never invents an accuracy score; it returns a
//! calm [`OmrQuality`] signal so the UI can warn when a scan yielded almost
//! nothing ([`OmrQuality::low_content`]), and the import flow always shows a
//! "read from a scan — check it looks right" note. A scan that yields no
//! readable music fails *here*, at import, with [`OmrError::Empty`] rather than
//! silently later when the cursor won't move.
//!
//! ## Offline by default, engine delivered at build time
//!
//! Recognition runs through an [`OmrEngine`]. The shipped engine
//! ([`SidecarOmrEngine`]) invokes a bundled, frozen `oemer` binary
//! out-of-process: **no network at run time**, and no Python/JRE in our address
//! space. The binary is delivered the same way the ONNX Runtime is — fetched
//! into the Tauri resource dir at build/package time (`scripts/fetch-omr-engine.sh`,
//! gitignored). When it isn't bundled, the engine reports
//! [`OmrError::EngineUnavailable`] instead of pretending — the same "degrade,
//! don't fabricate" rule the rest of the app follows offline.

mod error;
mod sidecar;

pub use error::OmrError;
pub use sidecar::SidecarOmrEngine;

/// A recognition back-end: sheet-music **PDF bytes** → **MusicXML string**.
///
/// Kept as a trait so the pipeline is testable without the heavy bundled engine
/// (see [`StaticOmrEngine`]) and so the engine can be swapped later — e.g. an
/// in-process Rust port of oemer's post-processing — without touching callers.
pub trait OmrEngine {
    /// Recognize the sheet music in a PDF document, returning MusicXML.
    fn recognize_pdf(&self, pdf: &[u8]) -> Result<String, OmrError>;
}

/// A calm quality signal for an OMR result — never a fake accuracy score.
///
/// Counts are derived from the emitted MusicXML with a cheap tag scan; they
/// exist to answer "did we actually read anything?", not to grade the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OmrQuality {
    /// Number of `<score-part>` declarations the engine emitted.
    pub part_count: usize,
    /// Rough number of `<measure>` elements recognized across the score.
    pub measure_count: usize,
    /// The scan yielded no measures — the read likely failed, and the UI should
    /// say so plainly rather than present an empty staff as a real score.
    pub low_content: bool,
}

/// A recognized score: the MusicXML the engine produced plus its quality signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recognized {
    /// MusicXML ready to hand to the existing import path
    /// (`list_parts` → part picker → `import_musicxml`).
    pub music_xml: String,
    /// How much was actually read — drives the calm "this looks empty" warning.
    pub quality: OmrQuality,
}

/// PDF magic number: every PDF document begins with `%PDF-`.
const PDF_MAGIC: &[u8] = b"%PDF-";

/// True when `bytes` begin with the PDF magic number. Cheap front-door check so
/// a non-PDF (e.g. an image or a stray text file) fails fast with a clear
/// message instead of being shipped to the engine.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(PDF_MAGIC)
}

/// Recognize a sheet-music PDF into MusicXML using `engine`.
///
/// Validates that the input is a PDF and that the engine's output is usable
/// MusicXML before returning, so a malformed scan fails at import with a clear
/// error (mirrors the MusicXML import contract). The returned MusicXML is the
/// canonical format the renderer and follower consume — OMR is just another
/// front-end producing it.
pub fn pdf_to_musicxml<E: OmrEngine + ?Sized>(
    engine: &E,
    pdf: &[u8],
) -> Result<Recognized, OmrError> {
    if !is_pdf(pdf) {
        return Err(OmrError::NotPdf);
    }
    let music_xml = engine.recognize_pdf(pdf)?;
    if !looks_like_musicxml(&music_xml) {
        return Err(OmrError::Empty);
    }
    let quality = quality_of(&music_xml);
    Ok(Recognized { music_xml, quality })
}

/// Cheap structural check that a string is a MusicXML score document.
///
/// We do not fully parse here — the `brain` crate owns real MusicXML parsing.
/// This only rejects obvious non-scores so the caller gets a clean
/// [`OmrError::Empty`] instead of a confusing downstream parse error.
fn looks_like_musicxml(s: &str) -> bool {
    s.contains("<score-partwise") || s.contains("<score-timewise")
}

/// Derive the quality signal from the MusicXML with a cheap tag scan.
///
/// A tag count is enough for the "did we read anything?" heuristic; exact
/// part/measure semantics belong to the `brain` parser, which the import
/// command calls separately to get the real part names for the picker.
fn quality_of(xml: &str) -> OmrQuality {
    // Trailing space avoids matching the document root `<score-partwise …>`
    // (which never has a space immediately after `…part`).
    let part_count = xml.matches("<score-part ").count();
    // `<measure` matches opening tags only; `</measure>` does not contain it.
    let measure_count = xml.matches("<measure").count();
    OmrQuality {
        part_count,
        measure_count,
        low_content: measure_count == 0,
    }
}

/// An [`OmrEngine`] that returns a fixed MusicXML string regardless of input.
///
/// It performs **no** recognition — it exists to test the pipeline and the
/// import wiring deterministically without the heavy bundled engine, and as a
/// building block for fixtures. Real OMR uses [`SidecarOmrEngine`].
pub struct StaticOmrEngine {
    musicxml: String,
}

impl StaticOmrEngine {
    /// Build an engine that always returns `musicxml`.
    pub fn new(musicxml: impl Into<String>) -> Self {
        Self {
            musicxml: musicxml.into(),
        }
    }
}

impl OmrEngine for StaticOmrEngine {
    fn recognize_pdf(&self, _pdf: &[u8]) -> Result<String, OmrError> {
        Ok(self.musicxml.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_PDF: &[u8] = b"%PDF-1.7\n%minimal\n";

    /// A tiny two-part, two-measure score for the quality-count assertions.
    const TWO_PART_XML: &str = r#"<?xml version="1.0"?>
<score-partwise version="4.0">
  <part-list>
    <score-part id="P1"><part-name>Trumpet</part-name></score-part>
    <score-part id="P2"><part-name>Trombone</part-name></score-part>
  </part-list>
  <part id="P1">
    <measure number="1"></measure>
    <measure number="2"></measure>
  </part>
  <part id="P2">
    <measure number="1"></measure>
    <measure number="2"></measure>
  </part>
</score-partwise>"#;

    #[test]
    fn is_pdf_recognizes_magic() {
        assert!(is_pdf(MINIMAL_PDF));
        assert!(is_pdf(b"%PDF-1.4 trailing"));
        assert!(!is_pdf(b"not a pdf"));
        assert!(!is_pdf(b"")); // empty input is not a PDF
        assert!(!is_pdf(b"PDF-1.7")); // missing leading %
    }

    #[test]
    fn pdf_to_musicxml_returns_engine_output_with_quality() {
        let engine = StaticOmrEngine::new(TWO_PART_XML);
        let got = pdf_to_musicxml(&engine, MINIMAL_PDF).expect("valid pdf + valid xml");
        assert_eq!(got.music_xml, TWO_PART_XML);
        assert_eq!(got.quality.part_count, 2);
        assert_eq!(got.quality.measure_count, 4);
        assert!(!got.quality.low_content);
    }

    #[test]
    fn non_pdf_input_is_rejected_before_the_engine_runs() {
        let engine = StaticOmrEngine::new(TWO_PART_XML);
        let err = pdf_to_musicxml(&engine, b"GIF89a").expect_err("not a pdf");
        assert!(matches!(err, OmrError::NotPdf));
    }

    #[test]
    fn engine_output_that_is_not_musicxml_is_empty_not_a_score() {
        let engine = StaticOmrEngine::new("the model could not read this page");
        let err = pdf_to_musicxml(&engine, MINIMAL_PDF).expect_err("not musicxml");
        assert!(matches!(err, OmrError::Empty));
    }

    #[test]
    fn a_score_with_no_measures_flags_low_content() {
        let empty_score = r#"<score-partwise version="4.0">
            <part-list><score-part id="P1"><part-name>Flute</part-name></score-part></part-list>
            <part id="P1"></part>
        </score-partwise>"#;
        let engine = StaticOmrEngine::new(empty_score);
        let got = pdf_to_musicxml(&engine, MINIMAL_PDF).expect("structurally a score");
        assert_eq!(got.quality.measure_count, 0);
        assert!(
            got.quality.low_content,
            "no measures → warn the read failed"
        );
    }

    #[test]
    fn quality_does_not_miscount_the_document_root_as_a_part() {
        // `<score-partwise>` must not be counted as a `<score-part>`.
        let one_part = r#"<score-partwise version="4.0">
            <part-list><score-part id="P1"><part-name>Oboe</part-name></score-part></part-list>
            <part id="P1"><measure number="1"></measure></part>
        </score-partwise>"#;
        let q = quality_of(one_part);
        assert_eq!(q.part_count, 1);
        assert_eq!(q.measure_count, 1);
    }
}
