use crate::models::EntryKind;

/// Map a MIME type string to the coarse clipboard entry classification `EntryKind`.
pub fn kind_from_mime(mime: &str) -> EntryKind {
    if mime.starts_with("image/") {
        EntryKind::Image
    } else if mime == "text/uri-list" {
        EntryKind::File
    } else if mime.starts_with("text/") {
        EntryKind::Text
    } else {
        EntryKind::Unknown
    }
}

/// Map an image MIME type to its standard file extension, or `"bin"` if unrecognized.
pub fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/png" => "png",
        _ => "bin",
    }
}
