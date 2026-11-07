use anyhow::{Result, ensure};
use image::{
    GrayImage,
    imageops::{self, FilterType},
};
use ort::value::Tensor;

use crate::IndraOcr;

impl IndraOcr {
    pub(crate) fn upright(&mut self, gray: &GrayImage) -> Result<GrayImage> {
        let Some(session) = &mut self.orient else {
            return Ok(gray.clone());
        };
        let (ink, _, _) = page_input(gray, 768, true);
        let outputs =
            session.run(ort::inputs!["x" => Tensor::from_array(([1, 1, 768, 768], ink))?])?;
        let (_, probabilities) = outputs[0].try_extract_tensor::<f32>()?;
        ensure!(probabilities.len() == 4, "unexpected orientation output");
        let turn = probabilities
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        Ok(if probabilities[turn] <= 0.95 {
            gray.clone()
        } else {
            match turn {
                1 => imageops::rotate90(gray),
                2 => imageops::rotate180(gray),
                3 => imageops::rotate270(gray),
                _ => gray.clone(),
            }
        })
    }

    pub(crate) fn detect(&mut self, gray: &GrayImage) -> Result<Vec<[f32; 4]>> {
        let (mut boxes, mut scale) = self.detect_at(gray, self.long_side)?;
        if boxes.len() >= 8 {
            let mut heights: Vec<_> = boxes.iter().map(|b| b[3] - b[1]).collect();
            heights.sort_by(f32::total_cmp);
            let middle = heights.len() / 2;
            let median = (heights[middle] + heights[(heights.len() - 1) / 2]) * 0.5;
            if median < 9.0 {
                let side =
                    ((self.long_side as f32 * 12.0 / median.max(1.0)).min(2048.0) as u32 / 32) * 32;
                if side > self.long_side {
                    (boxes, scale) = self.detect_at(gray, side)?;
                }
            }
        }
        for b in &mut boxes {
            b[0] = (b[0] / scale).clamp(0.0, gray.width() as f32);
            b[1] = (b[1] / scale).clamp(0.0, gray.height() as f32);
            b[2] = (b[2] / scale).clamp(0.0, gray.width() as f32);
            b[3] = (b[3] / scale).clamp(0.0, gray.height() as f32);
        }
        boxes.retain(|b| b.iter().all(|n| n.is_finite()) && b[2] > b[0] && b[3] > b[1]);
        Ok(boxes)
    }

    fn detect_at(&mut self, gray: &GrayImage, side: u32) -> Result<(Vec<[f32; 4]>, f32)> {
        let (ink, width, height) = page_input(gray, side, false);
        let outputs = self
            .det
            .run(ort::inputs!["x" => Tensor::from_array(([1, 1, height, width], ink))?])?;
        let (shape, data) = outputs[0].try_extract_tensor::<f32>()?;
        ensure!(
            shape.len() == 4 && shape[0] == 1 && shape[1] == 3,
            "unexpected detector output: {shape:?}"
        );
        let h = usize::try_from(shape[2])?;
        let w = usize::try_from(shape[3])?;
        Ok((
            decode_boxes(data, w, h),
            side as f32 / gray.width().max(gray.height()) as f32,
        ))
    }
}

/// White padding is zero in the model's ink map (black is one).
fn page_input(gray: &GrayImage, side: u32, square: bool) -> (Vec<f32>, usize, usize) {
    let scale = side as f64 / gray.width().max(gray.height()) as f64;
    let w = ((gray.width() as f64 * scale).round_ties_even() as u32).max(1);
    let h = ((gray.height() as f64 * scale).round_ties_even() as u32).max(1);
    let resized = imageops::resize(gray, w, h, FilterType::Triangle);
    let pw = if square { side } else { w.div_ceil(32) * 32 } as usize;
    let ph = if square { side } else { h.div_ceil(32) * 32 } as usize;
    let mut ink = vec![0.0; pw * ph];
    for (x, y, pixel) in resized.enumerate_pixels() {
        ink[y as usize * pw + x as usize] = 1.0 - pixel[0] as f32 / 255.0;
    }
    (ink, pw, ph)
}

/// Four-connected components of the positive core logits, decoded using the
/// detector's top/bottom distance channels (output stride two).
fn decode_boxes(data: &[f32], width: usize, height: usize) -> Vec<[f32; 4]> {
    let area = width * height;
    let mut seen = vec![false; area];
    let mut stack = Vec::new();
    let mut boxes = Vec::new();
    for start in 0..area {
        if seen[start] || data[start] <= 0.0 || !data[start].is_finite() {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut left, mut right) = (width, 0);
        let (mut top, mut bottom, mut count) = (0.0, 0.0, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % width, i / width);
            left = left.min(x);
            right = right.max(x + 1);
            let cy = (y as f32 + 0.5) * 2.0;
            top += cy - data[area + i];
            bottom += cy + data[2 * area + i];
            count += 1;
            let neighbors = [
                (x > 0).then(|| i - 1),
                (x + 1 < width).then_some(i + 1),
                (y > 0).then(|| i - width),
                (y + 1 < height).then_some(i + width),
            ];
            for next in neighbors.into_iter().flatten() {
                if !seen[next] && data[next] > 0.0 && data[next].is_finite() {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        if count >= 2 {
            let y0 = top / count as f32;
            let y1 = bottom / count as f32;
            let expand = 0.15 * (y1 - y0).max(1.0);
            boxes.push([
                left as f32 * 2.0 - expand,
                y0,
                right as f32 * 2.0 + expand,
                y1,
            ]);
        }
    }
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_use_four_neighbors_and_distance_channels() {
        // Two diagonal components must stay separate. A single-pixel component
        // is noise. Distances put both real lines at y=[0, 8].
        let mut data = vec![-1.0; 3 * 4 * 3];
        for i in [0, 1, 6, 7, 8] {
            data[i] = 1.0;
            let cy = (i / 4) as f32 * 2.0 + 1.0;
            data[12 + i] = cy;
            data[24 + i] = 8.0 - cy;
        }
        let boxes = decode_boxes(&data, 4, 3);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0], [-1.2, 0.0, 5.2, 8.0]);
        assert_eq!(boxes[1], [2.8, 0.0, 9.2, 8.0]);
    }

    #[test]
    fn detector_padding_is_white_ink_and_divisible_by_32() {
        let gray = GrayImage::from_pixel(40, 20, image::Luma([0]));
        let (ink, w, h) = page_input(&gray, 40, false);
        assert_eq!((w, h), (64, 32));
        assert_eq!(ink[0], 1.0);
        assert_eq!(ink[40], 0.0);
        assert_eq!(ink[20 * w], 0.0);
    }
}
