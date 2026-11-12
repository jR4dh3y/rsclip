//! Native Indra image OCR. Models stay loaded between calls to [`IndraOcr::recognize`].
//!
//! Preprocessing and decoding are adapted from datalab-to/indra (Apache-2.0).
//! See NOTICE and LICENSE. Model weights have their own license.

mod detect;
mod order;
mod recognize;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use image::{GrayImage, ImageReader};
use ort::{environment::Environment, session::Session};
use serde::Deserialize;

pub const MODEL_REVISION: &str = "ed345c7f158eddbc7dee0c6a7269afb75a94f7e8";

/// A detected line in upright image coordinates, with mean CTC peak confidence.
#[derive(Clone, Debug)]
pub struct Line {
    pub bbox: [f32; 4],
    pub text: String,
    pub confidence: f32,
}

/// Reusable CPU inference sessions. No subprocesses or network requests.
pub struct IndraOcr {
    det: Session,
    rec: Session,
    order: Session,
    orient: Option<Session>,
    cjk: Option<(Session, Session, Vocabulary)>,
    vocab: Vocabulary,
    long_side: u32,
    height: u32,
}

#[derive(Deserialize)]
struct Metadata {
    det_long_side: u32,
    rec_height: u32,
    vocab: String,
    vocab_cjk: Option<String>,
}

#[derive(Deserialize)]
struct Vocabulary {
    #[serde(default)]
    blank: usize,
    chars: Vec<String>,
}

impl Vocabulary {
    fn parse(json: &str) -> Result<Self> {
        let vocab: Self = serde_json::from_str(json).context("parsing Indra vocabulary")?;
        ensure!(
            vocab.blank == 0 && !vocab.chars.is_empty(),
            "unsupported Indra vocabulary"
        );
        Ok(vocab)
    }
}

/// Same location used by scripts/fetch-indra-models.sh. An explicit directory
/// or INDRA_OCR_MODEL_DIR can be used for offline deployment.
pub fn default_model_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("INDRA_OCR_MODEL_DIR") {
        return Ok(path.into());
    }
    let dirs = directories::BaseDirs::new().context("finding the user cache directory")?;
    Ok(dirs.cache_dir().join("rsclip/indra").join(MODEL_REVISION))
}

impl IndraOcr {
    /// Load a pinned Indra model set from disk. `threads` must be positive.
    pub fn load(model_dir: impl AsRef<Path>, threads: usize) -> Result<Self> {
        ensure!(threads > 0, "threads must be at least 1");
        let dir = model_dir.as_ref();
        let metadata: Metadata =
            serde_json::from_slice(&std::fs::read(dir.join("meta.json")).with_context(|| {
                format!(
                    "reading {}/meta.json; run scripts/fetch-indra-models.sh first",
                    dir.display()
                )
            })?)?;
        ensure!(
            (32..=2048).contains(&metadata.det_long_side),
            "unsupported detector size"
        );
        ensure!(metadata.rec_height == 32, "unsupported recognizer height");
        let env = ort::init()
            .with_name("rsclip-indra")
            .with_telemetry(false)
            .build()?;
        let session = |name: &str| load_session(&env, &dir.join(name), threads);
        let cjk = match (
            dir.join("script.onnx").is_file(),
            dir.join("rec_cjk.onnx").is_file(),
        ) {
            (false, false) => None,
            (true, true) => Some((
                session("script.onnx")?,
                session("rec_cjk.onnx")?,
                Vocabulary::parse(
                    metadata
                        .vocab_cjk
                        .as_deref()
                        .context("missing CJK vocabulary")?,
                )?,
            )),
            _ => anyhow::bail!("Chinese OCR requires both script.onnx and rec_cjk.onnx"),
        };
        Ok(Self {
            det: session("det.onnx")?,
            rec: session("rec.onnx")?,
            order: session("order.onnx")?,
            orient: dir
                .join("orient.onnx")
                .is_file()
                .then(|| session("orient.onnx"))
                .transpose()?,
            cjk,
            vocab: Vocabulary::parse(&metadata.vocab)?,
            long_side: metadata.det_long_side,
            height: metadata.rec_height,
        })
    }

    /// Decode PNG, JPEG, WebP or ICO from disk with image crate resource limits.
    pub fn recognize_file(&mut self, path: impl AsRef<Path>) -> Result<Vec<Line>> {
        let gray = read_image(path.as_ref())?;
        self.recognize(&gray)
    }

    /// Recognize printed text and return lines in model reading order. This
    /// image-only pipeline does not implement PDF parsing, tables or math.
    pub fn recognize(&mut self, gray: &GrayImage) -> Result<Vec<Line>> {
        ensure!(gray.width() > 0 && gray.height() > 0, "empty image");
        ensure!(
            u64::from(gray.width()) * u64::from(gray.height()) <= 100_000_000,
            "image exceeds 100 megapixels"
        );
        let gray = self.upright(gray)?;
        let boxes = self.detect(&gray)?;
        ensure!(
            boxes.len() <= 2048,
            "image has too many text lines (maximum 2048)"
        );
        let mut lines = self.read_lines(&gray, &boxes)?;
        remove_duplicate_fragments(&mut lines);
        self.sort_lines(&mut lines, gray.dimensions())?;
        Ok(lines)
    }
}

/// Decode and bound image allocation before model inference.
fn read_image(path: &Path) -> Result<GrayImage> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    reader.limits(limits);
    let decoder = reader.into_decoder()?;
    let (width, height) = image::ImageDecoder::dimensions(&decoder);
    ensure!(
        u64::from(width) * u64::from(height) <= 100_000_000,
        "image exceeds 100 megapixels"
    );
    let rgba = image::DynamicImage::from_decoder(decoder)
        .with_context(|| format!("decoding {}", path.display()))?
        .into_rgba8();
    // Composite transparency onto white before grayscale conversion.
    let gray = GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0.map(u32::from);
        let luma = (299 * r + 587 * g + 114 * b + 500) / 1000;
        image::Luma([((luma * a + 255 * (255 - a) + 127) / 255) as u8])
    });
    Ok(gray)
}

fn load_session(env: &Environment, path: &Path, threads: usize) -> Result<Session> {
    use ort::ep::CPU;
    Session::builder(env)?
        .with_intra_threads(threads)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .with_inter_threads(1)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .with_memory_pattern(false)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .with_execution_providers([CPU::default().with_arena_allocator(false).build()])
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .commit_from_file(path)
        .with_context(|| format!("loading {}", path.display()))
}

// The detector can emit a disconnected core for a leading glyph as well as
// the full line. Suppress it only if the larger line already contains its text
// and covers most of its box; repeated text elsewhere is preserved.
fn remove_duplicate_fragments(lines: &mut Vec<Line>) {
    let keep: Vec<_> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let [x0, y0, x1, y1] = line.bbox;
            let area = (x1 - x0) * (y1 - y0);
            !lines.iter().enumerate().any(|(j, other)| {
                if i == j
                    || line.text.is_empty()
                    || other.text.len() <= line.text.len()
                    || !other.text.contains(&line.text)
                {
                    return false;
                }
                let [a0, b0, a1, b1] = other.bbox;
                let overlap =
                    (x1.min(a1) - x0.max(a0)).max(0.0) * (y1.min(b1) - y0.max(b0)).max(0.0);
                overlap > area * 0.75
            })
        })
        .collect();
    let mut index = 0;
    lines.retain(|_| {
        let retain = keep[index];
        index += 1;
        retain
    });
}

/// Join recognized lines as plain text, omitting low-confidence empty lines.
pub fn plain_text(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| line.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_image_before_decoding_pixels() {
        let path = std::env::temp_dir().join(format!("rsclip-size-{}.png", std::process::id()));
        GrayImage::from_pixel(2, 2, image::Luma([128]))
            .save(&path)
            .unwrap();
        assert_eq!(read_image(&path).unwrap().dimensions(), (2, 2));
        let mut png = std::fs::read(&path).unwrap();
        // Change only IHDR and its CRC. The tiny compressed pixel stream is
        // deliberately invalid for the new dimensions and must never decode.
        png[16..20].copy_from_slice(&12_000_u32.to_be_bytes());
        png[20..24].copy_from_slice(&12_000_u32.to_be_bytes());
        let mut crc = !0_u32;
        for byte in &png[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & (0_u32.wrapping_sub(crc & 1)));
            }
        }
        png[29..33].copy_from_slice(&(!crc).to_be_bytes());
        std::fs::write(&path, png).unwrap();
        let error = read_image(&path).unwrap_err();
        assert!(
            error.to_string().contains("image exceeds 100 megapixels"),
            "{error:#}"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn removes_overlapping_glyph_fragment_but_keeps_separate_repeated_text() {
        let line = |bbox, text: &str| Line {
            bbox,
            text: text.into(),
            confidence: 0.99,
        };
        let mut lines = vec![
            line([0.0, 0.0, 10.0, 10.0], "T"),
            line([0.0, 0.0, 100.0, 10.0], "The text"),
            line([0.0, 20.0, 10.0, 30.0], "T"),
            line([0.0, 0.0, 10.0, 10.0], "Q"),
        ];
        remove_duplicate_fragments(&mut lines);
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["The text", "T", "Q"]
        );
    }
}
