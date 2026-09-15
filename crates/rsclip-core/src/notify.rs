use std::os::unix::net::UnixDatagram;

use crate::config::RsclipPaths;

/// Datagram payload signal indicating clipboard entries were added or updated.
pub const CHANGE_EVENT: &[u8] = b"changed";
/// Datagram payload signal indicating new favicons were cached.
pub const FAVICON_EVENT: &[u8] = b"favicons";

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
