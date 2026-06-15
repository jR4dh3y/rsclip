use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rsclip_core::notify::notify_favicons_changed;
use rsclip_core::{AppConfig, RsclipPaths};
use tracing::{error, info, warn};

pub fn run_watchers() -> Result<()> {
    require_command("wl-paste")?;
    require_wayland_display()?;
    let paths = RsclipPaths::discover()?;
    paths.ensure()?;
    let config = AppConfig::load(&paths)?;
    if config.links.favicon_cache {
        start_favicon_worker(paths.clone());
    }

    info!("starting rsclip clipboard watchers");
    let mut watchers = vec![
        Watcher::new("text", "text", "text/plain"),
        Watcher::new("image", "image/png", "image/png"),
    ];

    loop {
        for watcher in &mut watchers {
            watcher.ensure_running()?;
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn start_favicon_worker(paths: RsclipPaths) {
    thread::spawn(move || {
        info!("starting favicon cache worker");
        loop {
            if let Err(err) = process_one_favicon_job(&paths) {
                warn!("favicon worker failed: {err:#}");
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn process_one_favicon_job(paths: &RsclipPaths) -> Result<()> {
    let mut queue_entries = match std::fs::read_dir(&paths.favicon_queue_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect::<Vec<_>>(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "reading favicon queue {}",
                    paths.favicon_queue_dir.display()
                )
            });
        }
    };
    queue_entries.sort();

    let Some(queue_path) = queue_entries.into_iter().next() else {
        return Ok(());
    };

    let domain = match crate::favicons::read_queue_domain(&queue_path) {
        Ok(domain) => domain,
        Err(err) => {
            warn!("{err:#}");
            std::fs::remove_file(&queue_path)
                .with_context(|| format!("removing {}", queue_path.display()))?;
            return Ok(());
        }
    };

    if !rsclip_core::favicons::should_enqueue(paths, &domain) {
        std::fs::remove_file(&queue_path)
            .with_context(|| format!("removing {}", queue_path.display()))?;
        return Ok(());
    }

    match crate::favicons::fetch_and_cache_domain(paths, &domain) {
        Ok(()) => {
            std::fs::remove_file(&queue_path)
                .with_context(|| format!("removing {}", queue_path.display()))?;
            notify_favicons_changed(paths);
        }
        Err(err) => {
            warn!("favicon fetch failed for {domain}: {err:#}");
            std::fs::remove_file(&queue_path)
                .with_context(|| format!("removing {}", queue_path.display()))?;
        }
    }

    Ok(())
}

struct Watcher {
    label: &'static str,
    wl_type: &'static str,
    mime_type: &'static str,
    child: Option<Child>,
}

impl Watcher {
    fn new(label: &'static str, wl_type: &'static str, mime_type: &'static str) -> Self {
        Self {
            label,
            wl_type,
            mime_type,
            child: None,
        }
    }

    fn ensure_running(&mut self) -> Result<()> {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait()? {
                None => return Ok(()),
                Some(status) => warn!("{} watcher exited with {}", self.label, status),
            }
        }

        let exe = std::env::current_exe().context("finding current executable")?;
        let child = Command::new("wl-paste")
            .arg("--type")
            .arg(self.wl_type)
            .arg("--watch")
            .arg(exe)
            .arg("store")
            .arg("--mime")
            .arg(self.mime_type)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawning {} wl-paste watcher", self.label))?;
        info!("started {} watcher", self.label);
        self.child = Some(child);
        Ok(())
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut()
            && let Err(err) = child.kill()
        {
            error!("failed to stop {} watcher: {err}", self.label);
        }
    }
}

fn require_command(command: &str) -> Result<()> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {command} >/dev/null"))
        .status()
        .with_context(|| format!("checking for {command}"))?;
    if !status.success() {
        bail!("{command} is required but was not found in PATH");
    }
    Ok(())
}

fn require_wayland_display() -> Result<()> {
    let Some(display) = std::env::var_os("WAYLAND_DISPLAY") else {
        bail!(
            "WAYLAND_DISPLAY is not set; import the compositor environment before starting rsclipd"
        );
    };
    let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
        bail!("XDG_RUNTIME_DIR is not set; cannot locate the Wayland socket");
    };

    let socket_path = std::path::PathBuf::from(runtime_dir).join(&display);
    if !socket_path.exists() {
        bail!(
            "Wayland socket {} does not exist; restart rsclipd after updating WAYLAND_DISPLAY",
            socket_path.display()
        );
    }

    Ok(())
}
