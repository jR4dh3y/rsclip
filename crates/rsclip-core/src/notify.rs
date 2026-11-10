use std::os::unix::net::UnixDatagram;

use crate::config::RsclipPaths;

/// Datagram payload signal indicating clipboard entries were added or updated.
pub const CHANGE_EVENT: &[u8] = b"changed";
/// Datagram payload signal indicating new favicons were cached.
pub const FAVICON_EVENT: &[u8] = b"favicons";

/// Datagram asking the resident UI to toggle its overlay.
pub const TOGGLE_EVENT: &[u8] = b"toggle";
/// Datagram asking the resident UI to show its overlay.
pub const SHOW_EVENT: &[u8] = b"show";

/// Ask the resident UI to toggle or show its overlay.
///
/// Returns `false` when no UI is listening, so the caller can start one. A
/// hotkey client that only does this never loads GTK: linking libgtk-4 alone
/// costs ~20 ms of process startup before `main` runs.
pub fn send_ui_command(paths: &RsclipPaths, event: &[u8]) -> bool {
    UnixDatagram::unbound()
        .and_then(|socket| socket.send_to(event, &paths.socket_path))
        .is_ok()
}

/// Send a Unix datagram notification to the running UI instance that entries changed.
pub fn notify_changed(paths: &RsclipPaths) {
    notify(paths, CHANGE_EVENT);
}

/// Send a Unix datagram notification to the running UI instance that favicons changed.
pub fn notify_favicons_changed(paths: &RsclipPaths) {
    notify(paths, FAVICON_EVENT);
}

fn notify(paths: &RsclipPaths, event: &[u8]) {
    if let Ok(socket) = UnixDatagram::unbound() {
        let _ = socket.send_to(event, &paths.socket_path);
    }
}
