use anyhow::{Result, ensure};
use ort::value::Tensor;

use crate::{IndraOcr, Line};

impl IndraOcr {
    pub(crate) fn sort_lines(&mut self, lines: &mut Vec<Line>, (w, h): (u32, u32)) -> Result<()> {
        let n = lines.len();
        if n < 2 {
            return Ok(());
        }
        let feats = line_features(lines, w as f32, h as f32);
        let outputs = self.order.run(ort::inputs![
            "feats" => Tensor::from_array(([1, n, 16], feats))?,
            "mask" => Tensor::from_array(([1, n], vec![true; n]))?
        ])?;
        let (_, logits) = outputs[0].try_extract_tensor::<f32>()?;
        ensure!(
            logits.len() == n * n && logits.iter().all(|x| x.is_finite()),
            "unexpected reading order output"
        );
        let order = decode_order(logits, n);
        let mut original: Vec<_> = std::mem::take(lines).into_iter().map(Some).collect();
        *lines = order
            .into_iter()
            .map(|i| original[i].take().unwrap())
            .collect();
        Ok(())
    }
}

fn line_features(lines: &[Line], width: f32, height: f32) -> Vec<f32> {
    let mut heights: Vec<_> = lines
        .iter()
        .map(|line| (line.bbox[3] - line.bbox[1]).max(1e-3))
        .collect();
    heights.sort_by(f32::total_cmp);
    let median = (heights[heights.len() / 2] + heights[(heights.len() - 1) / 2]) * 0.5;
    let mut out = Vec::with_capacity(lines.len() * 16);
    for line in lines {
        let [x0, y0, x1, y1] = line.bbox;
        let w = (x1 - x0).max(1e-3);
        let h = (y1 - y0).max(1e-3);
        out.extend([
            x0 / width,
            y0 / height,
            x1 / width,
            y1 / height,
            (x0 + x1) / (2.0 * width),
            (y0 + y1) / (2.0 * height),
            w / width,
            (h / median).ln(),
            (w / h).ln() / 4.0,
            median / height * 20.0,
        ]);
        let chars: Vec<_> = line.text.trim().chars().collect();
        if chars.is_empty() {
            out.extend([0.0; 6]);
            continue;
        }
        let first = chars[0];
        let prefix: String = chars.iter().take(3).collect();
        let numbered = prefix.trim_end_matches(['.', ')']);
        let is_list = "•-–▪◦*·→".contains(first)
            || (chars.len() > 3 && !numbered.is_empty() && numbered.chars().all(char::is_numeric));
        out.extend([
            (chars.len() as f32).ln_1p() / 5.0,
            chars.iter().filter(|c| c.is_numeric()).count() as f32 / chars.len() as f32,
            u8::from(is_list) as f32,
            u8::from(chars.last() == Some(&'-')) as f32,
            u8::from(".!?:;".contains(*chars.last().unwrap())) as f32,
            u8::from(first.is_uppercase()) as f32,
        ]);
    }
    out
}

/// Pairwise order ranking plus insertion search, following Indra's
/// decode_order. Layout/table block reconstruction is deliberately omitted.
fn decode_order(logits: &[f32], n: usize) -> Vec<usize> {
    // log sigmoid, evaluated without overflow.
    let lp: Vec<f32> = logits
        .iter()
        .map(|&x| x.min(0.0) - (-x.abs()).exp().ln_1p())
        .collect();
    let scores: Vec<f32> = (0..n)
        .map(|i| (0..n).map(|j| lp[j * n + i] - lp[i * n + j]).sum())
        .collect();
    let mut order: Vec<_> = (0..n).collect();
    order.sort_by(|&a, &b| scores[a].total_cmp(&scores[b]));
    for _ in 0..4 {
        let mut improved = false;
        for x in order.clone() {
            let current = order.iter().position(|&i| i == x).unwrap();
            let rest: Vec<_> = order.iter().copied().filter(|&i| i != x).collect();
            let mut gain: f32 = rest.iter().map(|&i| lp[x * n + i]).sum();
            let mut gains = vec![gain];
            for &i in &rest {
                gain += lp[i * n + x] - lp[x * n + i];
                gains.push(gain);
            }
            let best = (0..n)
                .max_by(|&a, &b| gains[a].total_cmp(&gains[b]))
                .unwrap();
            if gains[best] > gains[current] + 1e-6 {
                order = rest;
                order.insert(best, x);
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairwise_order_can_move_a_line_across_a_column() {
        // Desired order [2, 0, 3, 1], unrelated to detector component order.
        let desired = [2, 0, 3, 1];
        let mut logits = vec![0.0; 16];
        for (pos, &i) in desired.iter().enumerate() {
            for &j in &desired[pos + 1..] {
                logits[i * 4 + j] = 10.0;
                logits[j * 4 + i] = -10.0;
            }
        }
        assert_eq!(decode_order(&logits, 4), desired);
        assert_eq!(decode_order(&[0.0], 1), [0]);
    }
}
