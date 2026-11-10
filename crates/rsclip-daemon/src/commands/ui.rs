use std::os::unix::process::CommandExt;
use std::process::Command;

use anyhow::{Result, bail};
use rsclip_core::RsclipPaths;
use rsclip_core::notify::{SHOW_EVENT, TOGGLE_EVENT, send_ui_command};

/// Forward a hotkey to the resident UI over its notification socket.
///
/// `rsclip toggle` links GTK, so the process spends ~25 ms loading shared
/// libraries and registering on D-Bus before the resident UI hears about the
/// key press. This binary links neither; the datagram is sent within a
/// millisecond or two. When no UI is running, start one the usual way.
pub fn run(command: &str) -> Result<()> {
    let event = match command {
        "toggle" => TOGGLE_EVENT,
        "show" => SHOW_EVENT,
        other => bail!("unknown UI command: {other}"),
    };
    let paths = RsclipPaths::discover()?;
    if send_ui_command(&paths, event) {
        return Ok(());
    }

    // Prefer the `rsclip` installed beside this binary over one on PATH.
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("rsclip")))
        .filter(|path| path.is_file());
    let program = sibling.unwrap_or_else(|| "rsclip".into());
    let err = Command::new(program).arg(command).exec();
    bail!("starting rsclip {command}: {err}")
}
