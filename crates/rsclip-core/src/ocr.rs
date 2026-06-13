use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

pub fn run_tesseract(image_path: &str, language: &str) -> Result<String> {
    run_tesseract_with_options(image_path, language, "tesseract", 0)
}

pub fn run_tesseract_with_options(
    image_path: &str,
    language: &str,
    command: &str,
    timeout_seconds: u64,
) -> Result<String> {
    let mut child = Command::new(command)
        .arg(image_path)
        .arg("stdout")
        .arg("-l")
        .arg(language)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {command}"))?;

    if timeout_seconds > 0 {
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        loop {
            if child
                .try_wait()
                .with_context(|| format!("waiting for {command}"))?
                .is_some()
            {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("{command} timed out after {timeout_seconds}s");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    let output = child
        .wait_with_output()
        .with_context(|| format!("waiting for {command}"))?;
    if !output.status.success() {
        bail!(
            "{command} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
