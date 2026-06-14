use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::rc::Rc;

use anyhow::{Context, Result};
use gtk4 as gtk;
use gtk4::prelude::*;
use rsclip_core::notify::{CHANGE_EVENT, FAVICON_EVENT};

use crate::actions::refresh::{refresh_entries_if_changed, rerender_current_list};
use crate::actions::set_footer;
use crate::state::AppState;

pub(crate) fn install_change_listener(
    state: &Rc<AppState>,
    window: &gtk::ApplicationWindow,
    socket_path: &Path,
) -> Result<()> {
    match std::fs::remove_file(socket_path) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "removing stale notification socket {}",
                    socket_path.display()
                )
            });
        }
    }

    let socket = UnixDatagram::bind(socket_path)
        .with_context(|| format!("binding notification socket {}", socket_path.display()))?;
    socket
        .set_nonblocking(true)
        .context("setting notification socket nonblocking")?;
    let fd = socket.as_raw_fd();

    {
        let state = Rc::clone(state);
        let window = window.clone();
        gtk::glib::source::unix_fd_add_local(fd, gtk::glib::IOCondition::IN, move |_, _| {
            let mut buf = [0_u8; 64];
            let mut changed = false;
            let mut favicons_changed = false;
            loop {
                match socket.recv(&mut buf) {
                    Ok(size) => {
                        changed |= &buf[..size] == CHANGE_EVENT;
                        favicons_changed |= &buf[..size] == FAVICON_EVENT;
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(err) => {
                        set_footer(&state, &format!("Notification listener failed: {err}"));
                        return gtk::glib::ControlFlow::Break;
                    }
                }
            }

            if changed
                && window.is_visible()
                && let Err(err) = refresh_entries_if_changed(&state)
            {
                set_footer(&state, &format!("Refresh failed: {err:#}"));
            }
            if favicons_changed && window.is_visible() {
                rerender_current_list(&state);
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    Ok(())
}
