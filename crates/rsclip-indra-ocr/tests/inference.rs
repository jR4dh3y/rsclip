//! Opt-in tests using the real pinned models. Ordinary CI never needs weights.
use std::{path::Path, process::Command};

use rsclip_indra_ocr::{IndraOcr, default_model_dir, plain_text};

const EXPECTED: &str =
    "Native Rust OCR\nThe quick brown fox jumps over the lazy dog.\nClipboard text 1234567890";

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/printed.png")
}

#[test]
#[ignore = "requires the Indra models; run scripts/fetch-indra-models.sh first"]
fn real_models_read_printed_text_and_reuse_sessions() {
    let mut ocr = IndraOcr::load(default_model_dir().unwrap(), 2).unwrap();
    let lines = ocr.recognize_file(fixture()).unwrap();
    assert_eq!(plain_text(&lines), EXPECTED);
    let blank = image::GrayImage::from_pixel(320, 200, image::Luma([255]));
    assert_eq!(plain_text(&ocr.recognize(&blank).unwrap()), "");
    assert_eq!(
        plain_text(&ocr.recognize_file(fixture()).unwrap()),
        EXPECTED
    );
}

#[test]
#[ignore = "requires the Indra models; run scripts/fetch-indra-models.sh first"]
fn binary_accepts_rsclip_tesseract_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_rsclip-ocr"))
        .arg(fixture())
        .args(["stdout", "-l", "eng"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), EXPECTED);
}
