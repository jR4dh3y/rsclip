use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config::RsclipPaths;
use crate::mime::extension_for_mime;

/// Compute the hex-encoded BLAKE3 hash of a clipboard payload.
pub fn content_hash(payload: &[u8]) -> String {
    blake3::hash(payload).to_hex().to_string()
}

/// Persist an image payload to disk under the images directory using its content hash.
pub fn store_image(
    paths: &RsclipPaths,
    hash: &str,
    mime_type: &str,
    payload: &[u8],
) -> Result<PathBuf> {
    paths.ensure()?;
    let extension = extension_for_mime(mime_type);
    let path = paths.image_dir.join(format!("{hash}.{extension}"));
    if !path.exists() {
        fs::write(&path, payload).with_context(|| format!("writing image {}", path.display()))?;
    }
    Ok(path)
}
