use anyhow::{Result, ensure};
use image::{
    GrayImage, Luma,
    imageops::{self, FilterType},
};
use ort::value::Tensor;

use crate::{IndraOcr, Line, Vocabulary};

impl IndraOcr {
    pub(crate) fn read_lines(&mut self, gray: &GrayImage, boxes: &[[f32; 4]]) -> Result<Vec<Line>> {
        let mut lines = Vec::with_capacity(boxes.len());
        // A single crop at a time bounds peak memory, including very long lines.
        for &bbox in boxes {
            let (ink, width, padded_width) = crop_line(gray, bbox, self.height);
            let mut rec = &mut self.rec;
            let mut vocab = &self.vocab;
            if let Some((script, cjk, cjk_vocab)) = &mut self.cjk {
                let out = script.run(ort::inputs![
                    "x" => Tensor::from_array(([1, 1, self.height as usize, padded_width], ink.clone()))?,
                    "widths" => Tensor::from_array(([1], vec![width as i64]))?
                ])?;
                let (_, p) = out[0].try_extract_tensor::<f32>()?;
                ensure!(p.len() == 1, "unexpected script classifier output");
                if p[0] > 0.5 {
                    rec = cjk;
                    vocab = cjk_vocab;
                }
            }
            let out = rec.run(ort::inputs![
                "x" => Tensor::from_array(([1, 1, self.height as usize, padded_width], ink))?,
                "widths" => Tensor::from_array(([1], vec![width as i64]))?
            ])?;
            let (_, best) = out[0].try_extract_tensor::<i64>()?;
            let (_, pmax) = out[1].try_extract_tensor::<f32>()?;
            let (_, lengths) = out[2].try_extract_tensor::<i64>()?;
            ensure!(lengths.len() == 1, "unexpected recognizer length output");
            let length = usize::try_from(lengths[0])?;
            ensure!(
                length <= best.len() && length <= pmax.len(),
                "unexpected recognizer sequence length"
            );
            let (text, confidence) = decode_ctc(&best[..length], &pmax[..length], vocab)?;
            lines.push(Line {
                bbox,
                text,
                confidence,
            });
        }
        Ok(lines)
    }
}

fn crop_line(gray: &GrayImage, bbox: [f32; 4], height: u32) -> (Vec<f32>, usize, usize) {
    let [x0, y0, x1, y1] = bbox;
    let line_height = (y1 - y0).max(1.0);
    let left = (x0 - 0.3 * line_height).floor() as i32;
    let top = (y0 - 0.2 * line_height).floor() as i32;
    let right = (x1 + 0.3 * line_height).ceil() as i32;
    let bottom = (y1 + 0.2 * line_height).ceil() as i32;
    let w = (right - left).max(1) as u32;
    let h = (bottom - top).max(1) as u32;
    let crop = GrayImage::from_fn(w, h, |x, y| {
        let (sx, sy) = (x as i32 + left, y as i32 + top);
        if sx >= 0 && sy >= 0 && sx < gray.width() as i32 && sy < gray.height() as i32 {
            *gray.get_pixel(sx as u32, sy as u32)
        } else {
            Luma([255])
        }
    });
    let width = ((w as f64 * height as f64 / h as f64).round_ties_even() as u32).clamp(8, 4096);
    let resized = imageops::resize(&crop, width, height, FilterType::Triangle);
    let padded_width = width.div_ceil(8) as usize * 8;
    let mut ink = vec![0.0; padded_width * height as usize];
    for (x, y, p) in resized.enumerate_pixels() {
        ink[y as usize * padded_width + x as usize] = 1.0 - p[0] as f32 / 255.0;
    }
    (ink, width as usize, padded_width)
}

fn decode_ctc(ids: &[i64], probabilities: &[f32], vocab: &Vocabulary) -> Result<(String, f32)> {
    let mut tokens = Vec::new();
    let mut previous = 0;
    let mut confidence = 0.0;
    for (&id, &p) in ids.iter().zip(probabilities) {
        if id != 0 && id != previous {
            ensure!(
                id > 0 && id as usize <= vocab.chars.len(),
                "recognizer token outside vocabulary: {id}"
            );
            tokens.push(&vocab.chars[id as usize - 1]);
            confidence += p;
        }
        previous = id;
    }
    confidence /= tokens.len().max(1) as f32;
    let mut text = String::new();
    if confidence >= 0.5 {
        for (i, token) in tokens.iter().enumerate() {
            text.push_str(token);
            // Preserve the separator between a LaTeX command and a letter.
            if token.len() > 2
                && token.starts_with('\\')
                && token[1..].chars().all(char::is_alphabetic)
                && tokens
                    .get(i + 1)
                    .and_then(|s| s.chars().next())
                    .is_some_and(char::is_alphabetic)
            {
                text.push(' ');
            }
        }
    }
    Ok((text.trim().to_string(), confidence))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocabulary {
        Vocabulary {
            blank: 0,
            chars: vec!["a".into(), "b".into(), "ह".into(), "ि".into()],
        }
    }

    #[test]
    fn ctc_collapses_repeats_but_blank_separates_identical_letters() {
        let (text, confidence) =
            decode_ctc(&[1, 1, 0, 1, 2, 2, 3, 4], &[0.9; 8], &vocab()).unwrap();
        assert_eq!(text, "aabहि");
        assert!((confidence - 0.9).abs() < 1e-6);
    }

    #[test]
    fn low_confidence_and_blank_lines_have_no_text() {
        assert_eq!(decode_ctc(&[1, 2], &[0.2, 0.3], &vocab()).unwrap().0, "");
        assert_eq!(
            decode_ctc(&[0], &[1.0], &vocab()).unwrap(),
            (String::new(), 0.0)
        );
        assert!(decode_ctc(&[5], &[0.9], &vocab()).is_err());
    }

    #[test]
    fn crop_pads_outside_image_with_white() {
        let gray = GrayImage::from_pixel(20, 10, Luma([0]));
        let (ink, width, padded_width) = crop_line(&gray, [0.0, 0.0, 20.0, 10.0], 32);
        assert_eq!(padded_width % 8, 0);
        assert!(width <= padded_width);
        assert_eq!(ink[0], 0.0);
        assert!(ink.iter().any(|&v| v > 0.9));
    }
}
