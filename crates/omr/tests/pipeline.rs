//! End-to-end pipeline coverage through the public API, using the dependency-free
//! [`StaticOmrEngine`] so the contract is verified without the bundled engine.

use omr::{pdf_to_musicxml, OmrError, StaticOmrEngine};

const PDF: &[u8] = b"%PDF-1.7\nfake scan bytes\n";

const SCORE: &str = r#"<?xml version="1.0"?>
<score-partwise version="4.0">
  <part-list>
    <score-part id="P1"><part-name>Violin</part-name></score-part>
  </part-list>
  <part id="P1"><measure number="1"></measure></part>
</score-partwise>"#;

#[test]
fn pdf_is_recognized_into_musicxml_ready_for_the_part_picker() {
    let engine = StaticOmrEngine::new(SCORE);
    let recognized = pdf_to_musicxml(&engine, PDF).expect("recognition succeeds");

    // The output is the canonical MusicXML the existing import path consumes.
    assert!(recognized.music_xml.contains("<score-partwise"));
    assert_eq!(recognized.quality.part_count, 1);
    assert_eq!(recognized.quality.measure_count, 1);
    assert!(!recognized.quality.low_content);
}

#[test]
fn a_non_pdf_drop_is_rejected_with_a_clear_error() {
    let engine = StaticOmrEngine::new(SCORE);
    let err = pdf_to_musicxml(&engine, b"\x89PNG\r\n").expect_err("png is not a pdf");
    assert!(matches!(err, OmrError::NotPdf));
}
