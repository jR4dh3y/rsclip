use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use rsclip_indra_ocr::{IndraOcr, default_model_dir, plain_text};

const HELP: &str = "rsclip-ocr IMAGE [stdout] [-l LANGUAGE] [--model-dir DIR] [--threads N]

Native Indra OCR; prints plain text. No Python or OpenCV dependency.
Accepts rsclip's Tesseract-style invocation: IMAGE stdout -l eng.
LANGUAGE is accepted for compatibility; Indra detects supported scripts automatically.
Models: --model-dir, INDRA_OCR_MODEL_DIR, or the rsclip cache directory.
Download models first using scripts/fetch-indra-models.sh.
--threads defaults to 4. Inputs: PNG, JPEG, WebP, ICO. No PDF support.";

#[derive(Debug)]
struct Options {
    image: PathBuf,
    model_dir: Option<PathBuf>,
    threads: usize,
}

fn parse(args: Vec<String>) -> Result<Options> {
    let mut args = args.into_iter();
    let mut image = None;
    let mut model_dir = None;
    let mut threads = 4;
    let mut stdout = false;
    let mut positional_only = false;
    while let Some(arg) = args.next() {
        if !positional_only {
            match arg.as_str() {
                "--" => {
                    positional_only = true;
                    continue;
                }
                "--model-dir" => {
                    model_dir = Some(PathBuf::from(
                        args.next().context("--model-dir needs a path")?,
                    ));
                    continue;
                }
                "--threads" => {
                    threads = args
                        .next()
                        .context("--threads needs a number")?
                        .parse()
                        .context("invalid --threads")?;
                    ensure!(threads > 0, "threads must be at least 1");
                    continue;
                }
                "-l" => {
                    let _language = args.next().context("-l needs a language")?;
                    continue;
                }
                value if value.starts_with('-') => bail!("unknown option: {value}"),
                _ => {}
            }
        }
        if image.is_none() {
            image = Some(PathBuf::from(arg));
        } else if arg == "stdout" && !stdout {
            stdout = true;
        } else {
            bail!("unexpected argument: {arg}; only stdout output is supported");
        }
    }
    Ok(Options {
        image: image.context("an image path is required; see --help")?,
        model_dir,
        threads,
    })
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("{HELP}");
        return Ok(());
    }
    if args == ["--version"] {
        println!("rsclip-ocr {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let options = parse(args)?;
    let model_dir = options
        .model_dir
        .map(Ok)
        .unwrap_or_else(default_model_dir)?;
    let mut ocr = IndraOcr::load(model_dir, options.threads)?;
    println!("{}", plain_text(&ocr.recognize_file(options.image)?));
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("rsclip-ocr: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_rsclip_command_contract_and_native_options() {
        let options = parse(
            [
                "/tmp/clip.png",
                "stdout",
                "-l",
                "eng",
                "--threads",
                "2",
                "--model-dir",
                "/tmp/models",
            ]
            .map(str::to_string)
            .to_vec(),
        )
        .unwrap();
        assert_eq!(options.image, PathBuf::from("/tmp/clip.png"));
        assert_eq!(options.model_dir, Some(PathBuf::from("/tmp/models")));
        assert_eq!(options.threads, 2);
    }

    #[test]
    fn rejects_incomplete_or_unsupported_invocations() {
        for args in [
            vec![],
            vec!["image.png", "-l"],
            vec!["image.png", "--threads", "0"],
            vec!["image.png", "output.txt"],
        ] {
            assert!(parse(args.into_iter().map(str::to_string).collect()).is_err());
        }
    }
}
