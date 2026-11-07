# Native Indra OCR

Rust library and `rsclip-ocr` executable using the `ort` ONNX Runtime bindings.
No Python, PyTorch, OpenCV, service or GPU is required. The default Cargo build
downloads and statically links a prebuilt CPU ONNX Runtime. Weights are downloaded
separately, verified with SHA-256, and read locally during inference.

From the repository root:

```sh
cargo install --path crates/rsclip-indra-ocr --locked
bash scripts/fetch-indra-models.sh
rsclip-ocr screenshot.png
```

Set rsclip's existing OCR command in `~/.config/rsclip/config.toml`:

```toml
[ocr]
enabled = true
command = "rsclip-ocr"
default_language = "eng"
timeout_seconds = 20
auto_index = false
```

An absolute executable path also works. The UI button, `rsclipd ocr`, and automatic
indexing all use this setting. `rsclip-ocr` accepts `IMAGE stdout -l LANGUAGE`, the
same arguments rsclip passes to Tesseract. The language argument is accepted for
compatibility; Indra automatically routes supported scripts rather than selecting
a Tesseract language model. Existing OCR text is not recomputed automatically;
run OCR again or test on a fresh image when comparing engines.

```sh
rsclip-ocr screenshot.png --model-dir /path/to/models --threads 4
```

Model lookup: `--model-dir`, then `INDRA_OCR_MODEL_DIR`, then
`$XDG_CACHE_HOME/rsclip/indra/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8`
(`~/.cache` when unset on Linux). Pass the same path to the download script for
offline deployments. The complete model set is about 18 MB. The library requires
`det.onnx`, `rec.onnx`, `order.onnx`, and `meta.json`; orientation and Chinese
routing are enabled when their models are present. OCR never downloads files.

The library retains sessions across images:

```rust,no_run
use rsclip_indra_ocr::{IndraOcr, default_model_dir, plain_text};

fn main() -> anyhow::Result<()> {
    let mut ocr = IndraOcr::load(default_model_dir()?, 4)?;
    println!("{}", plain_text(&ocr.recognize_file("screenshot.png")?));
    Ok(())
}
```

Scope: PNG, JPEG, WebP and ICO; line detection with adaptive resolution, CTC
recognition, automatic quarter-turn correction, Chinese routing and pairwise
model reading order. Like upstream, text under 0.5 mean confidence is suppressed.
This is a port for clipboard images, not the complete document converter: PDFs,
deskew, layout blocks, table reconstruction and math recognition are unsupported.
Image resampling uses Rust's `image` filters rather than OpenCV, and reading order
uses the pairwise head without upstream block clustering, so results can differ.
Overlapping glyph fragments already included in a larger line are suppressed.
Dark themes and very small screenshot text need further accuracy evaluation.

Run unit tests with `cargo test -p rsclip-indra-ocr --locked`. Run the real-model
smoke test explicitly after downloading models:

```sh
cargo test -p rsclip-indra-ocr --test inference --locked -- --ignored
```

The crate's adapted code is Apache-2.0; rsclip's existing code remains MIT.
The [model weights have a separate license](https://huggingface.co/datalab-to/indra-ocr/blob/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8/LICENSE).
