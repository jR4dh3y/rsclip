use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct RsclipPaths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub image_dir: PathBuf,
    pub thumb_dir: PathBuf,
    pub ocr_dir: PathBuf,
    pub favicon_dir: PathBuf,
    pub favicon_icon_dir: PathBuf,
    pub favicon_queue_dir: PathBuf,
    pub favicon_miss_dir: PathBuf,
    pub log_path: PathBuf,
    pub socket_path: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub history: HistoryConfig,
    #[serde(default)]
    pub paste: PasteConfig,
    #[serde(default)]
    pub ocr: OcrConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub links: LinksConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryConfig {
    #[serde(default = "default_history_max_entries")]
    pub max_entries: usize,
    /// Maximum text payload size; `0` explicitly disables the limit.
    #[serde(default = "default_max_text_bytes")]
    pub max_text_bytes: usize,
    /// Maximum image payload size; `0` explicitly disables the limit.
    #[serde(default = "default_max_image_bytes")]
    pub max_image_bytes: usize,
    #[serde(default = "default_true")]
    pub dedupe: bool,
    #[serde(default)]
    pub cleanup_unpinned_after_days: u32,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_entries: default_history_max_entries(),
            max_text_bytes: default_max_text_bytes(),
            max_image_bytes: default_max_image_bytes(),
            dedupe: true,
            cleanup_unpinned_after_days: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct PasteConfig {
    #[serde(default = "default_true")]
    pub auto_paste: bool,
    #[serde(default = "default_paste_delay_ms")]
    pub paste_delay_ms: u64,
    #[serde(default = "default_paste_method")]
    pub method: String,
}

impl Default for PasteConfig {
    fn default() -> Self {
        Self {
            auto_paste: true,
            paste_delay_ms: default_paste_delay_ms(),
            method: default_paste_method(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct OcrConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_ocr_command")]
    pub command: String,
    #[serde(default = "default_ocr_language")]
    pub default_language: String,
    #[serde(default = "default_ocr_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub auto_index: bool,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            command: default_ocr_command(),
            default_language: default_ocr_language(),
            timeout_seconds: default_ocr_timeout_seconds(),
            auto_index: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LinksConfig {
    #[serde(default)]
    pub favicon_cache: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_window_width")]
    pub window_width: i32,
    #[serde(default = "default_window_height")]
    pub window_height: i32,
    #[serde(default)]
    pub background_opacity: Option<f32>,
    #[serde(default)]
    pub resizable: bool,
    #[serde(default = "default_true")]
    pub preview_default: bool,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: i32,
    #[serde(default = "default_true")]
    pub show_footer_hints: bool,
    #[serde(default = "default_true")]
    pub reset_on_show: bool,
    #[serde(default = "default_true")]
    pub auto_focus_search: bool,
    #[serde(default = "default_start_view")]
    pub start_view: String,
    #[serde(default = "default_filter")]
    pub default_filter: String,
    #[serde(default = "default_sort")]
    pub default_sort: String,
    #[serde(default = "default_search_placeholder")]
    pub search_placeholder: String,
    #[serde(default = "default_secrets_search_placeholder")]
    pub secrets_search_placeholder: String,

    #[serde(default)]
    pub colors: UiColors,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            window_width: default_window_width(),
            window_height: default_window_height(),
            background_opacity: None,
            resizable: false,
            preview_default: true,
            sidebar_width: default_sidebar_width(),
            show_footer_hints: true,
            reset_on_show: true,
            auto_focus_search: true,
            start_view: default_start_view(),
            default_filter: default_filter(),
            default_sort: default_sort(),
            search_placeholder: default_search_placeholder(),
            secrets_search_placeholder: default_secrets_search_placeholder(),
            colors: UiColors::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct UiColors {
    pub shell_bg: Option<String>,
    pub shell_border: Option<String>,
    pub surface: Option<String>,
    pub surface_subtle: Option<String>,
    pub surface_overlay: Option<String>,
    pub preview_bg: Option<String>,
    pub preview_text_bg: Option<String>,
    pub scrim_bg: Option<String>,

    pub text: Option<String>,
    pub text_strong: Option<String>,
    pub text_muted: Option<String>,
    pub text_selected_muted: Option<String>,

    pub border: Option<String>,
    pub border_subtle: Option<String>,
    pub border_preview: Option<String>,
    pub border_dialog: Option<String>,

    pub hover_bg: Option<String>,
    pub selected_bg: Option<String>,

    pub accent: Option<String>,
    pub accent_hover: Option<String>,
    pub accent_text: Option<String>,

    pub destructive: Option<String>,
    pub destructive_border: Option<String>,
    pub destructive_text: Option<String>,
}

fn default_theme() -> String {
    "nonchalant-dark".to_string()
}

fn default_history_max_entries() -> usize {
    2000
}

/// Default maximum size for captured text payloads (1 MiB).
fn default_max_text_bytes() -> usize {
    1024 * 1024
}

/// Default maximum size for captured image payloads (10 MiB).
fn default_max_image_bytes() -> usize {
    10 * 1024 * 1024
}

fn default_true() -> bool {
    true
}

fn default_paste_delay_ms() -> u64 {
    140
}

fn default_paste_method() -> String {
    "wtype".to_string()
}

fn default_ocr_command() -> String {
    "tesseract".to_string()
}

fn default_ocr_language() -> String {
    "eng".to_string()
}

fn default_ocr_timeout_seconds() -> u64 {
    20
}

fn default_window_width() -> i32 {
    760
}

fn default_window_height() -> i32 {
    480
}

fn default_sidebar_width() -> i32 {
    290
}

fn default_start_view() -> String {
    "clipboard".to_string()
}

fn default_filter() -> String {
    "all".to_string()
}

fn default_sort() -> String {
    "default".to_string()
}

fn default_search_placeholder() -> String {
    "Search clipboard...".to_string()
}

fn default_secrets_search_placeholder() -> String {
    "Search secrets by name...".to_string()
}

impl RsclipPaths {
    pub fn discover() -> Result<Self> {
        let project = ProjectDirs::from("", "", "rsclip")
            .context("could not resolve XDG directories for rsclip")?;
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);

        let config_dir = project.config_dir().to_path_buf();
        let state_dir = project
            .state_dir()
            .unwrap_or(project.data_local_dir())
            .to_path_buf();
        let data_dir = project.data_dir().to_path_buf();
        let image_dir = data_dir.join("images");
        let thumb_dir = data_dir.join("thumbs");
        let ocr_dir = data_dir.join("ocr");
        let favicon_dir = data_dir.join("favicons");
        let favicon_icon_dir = favicon_dir.join("icons");
        let favicon_queue_dir = favicon_dir.join("queue");
        let favicon_miss_dir = favicon_dir.join("misses");

        Ok(Self {
            db_path: state_dir.join("rsclip.db"),
            log_path: state_dir.join("rsclip.log"),
            socket_path: runtime_dir.join("rsclip.sock"),
            config_dir,
            state_dir,
            data_dir,
            image_dir,
            thumb_dir,
            ocr_dir,
            favicon_dir,
            favicon_icon_dir,
            favicon_queue_dir,
            favicon_miss_dir,
        })
    }

    pub fn ensure(&self) -> Result<()> {
        for dir in [
            &self.config_dir,
            &self.state_dir,
            &self.data_dir,
            &self.image_dir,
            &self.thumb_dir,
            &self.ocr_dir,
            &self.favicon_dir,
            &self.favicon_icon_dir,
            &self.favicon_queue_dir,
            &self.favicon_miss_dir,
        ] {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(())
    }

    pub fn config_path(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }
}

impl AppConfig {
    pub fn load(paths: &RsclipPaths) -> Result<Self> {
        let path = paths.config_path();
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };

        toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_paths(name: &str) -> RsclipPaths {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rsclip-config-test-{name}-{}-{unique}",
            std::process::id()
        ));
        RsclipPaths {
            config_dir: root.join("config"),
            state_dir: root.join("state"),
            data_dir: root.join("data"),
            db_path: root.join("state").join("rsclip.db"),
            image_dir: root.join("data").join("images"),
            thumb_dir: root.join("data").join("thumbs"),
            ocr_dir: root.join("data").join("ocr"),
            favicon_dir: root.join("data").join("favicons"),
            favicon_icon_dir: root.join("data").join("favicons").join("icons"),
            favicon_queue_dir: root.join("data").join("favicons").join("queue"),
            favicon_miss_dir: root.join("data").join("favicons").join("misses"),
            log_path: root.join("state").join("rsclip.log"),
            socket_path: root.join("rsclip.sock"),
        }
    }

    #[test]
    fn missing_config_file_returns_defaults() {
        let config = AppConfig::load(&test_paths("missing"))
            .expect("missing config file should load defaults");

        assert_eq!(config.history.max_entries, 2000);
        assert_eq!(config.history.max_text_bytes, 1024 * 1024);
        assert_eq!(config.history.max_image_bytes, 10 * 1024 * 1024);
        assert!(config.history.dedupe);
        assert!(config.paste.auto_paste);
        assert_eq!(config.paste.paste_delay_ms, 140);
        assert_eq!(config.ocr.default_language, "eng");
        assert_eq!(config.ui.theme, "nonchalant-dark");
        assert_eq!(config.ui.window_width, 760);
        assert_eq!(config.ui.window_height, 480);
        assert!(config.ui.preview_default);
        assert!(config.ui.colors.accent.is_none());
        assert!(!config.links.favicon_cache);
    }

    #[test]
    fn empty_config_file_returns_defaults() {
        let paths = test_paths("empty");
        fs::create_dir_all(&paths.config_dir).expect("test config dir should be created");
        fs::write(paths.config_path(), "").expect("empty config file should be written");

        let config = AppConfig::load(&paths).expect("empty config file should load defaults");

        assert_eq!(config.history.max_entries, 2000);
        assert_eq!(config.ui.theme, "nonchalant-dark");
        assert!(config.ui.colors.text.is_none());
        assert!(config.ui.show_footer_hints);
        assert!(!config.links.favicon_cache);
    }

    #[test]
    fn partial_colors_only_override_provided_fields() {
        let paths = test_paths("partial");
        fs::create_dir_all(&paths.config_dir).expect("test config dir should be created");
        fs::write(
            paths.config_path(),
            r##"
[ui.colors]
accent = "#ff00aa"
accent_text = "#000000"
"##,
        )
        .expect("partial config file should be written");

        let config = AppConfig::load(&paths).expect("partial config file should load");

        assert_eq!(config.ui.colors.accent.as_deref(), Some("#ff00aa"));
        assert_eq!(config.ui.colors.accent_text.as_deref(), Some("#000000"));
        assert!(config.ui.colors.text.is_none());
    }

    #[test]
    fn invalid_toml_returns_error_with_path_context() {
        let paths = test_paths("invalid");
        fs::create_dir_all(&paths.config_dir).expect("test config dir should be created");
        let path = paths.config_path();
        fs::write(&path, "[ui").expect("invalid config fixture should be written");

        let err = AppConfig::load(&paths).unwrap_err();

        assert!(format!("{err:#}").contains(&path.display().to_string()));
    }

    #[test]
    fn theme_defaults_to_nonchalant_dark() {
        assert_eq!(AppConfig::default().ui.theme, "nonchalant-dark");
    }

    #[test]
    fn links_favicon_cache_parses_true() {
        let paths = test_paths("links");
        fs::create_dir_all(&paths.config_dir).expect("test config dir should be created");
        fs::write(
            paths.config_path(),
            r#"
[links]
favicon_cache = true
"#,
        )
        .expect("links config file should be written");

        let config = AppConfig::load(&paths).expect("links config file should load");

        assert!(config.links.favicon_cache);
    }

    #[test]
    fn history_max_entries_parses() {
        let paths = test_paths("history");
        fs::create_dir_all(&paths.config_dir).expect("test config dir should be created");
        fs::write(
            paths.config_path(),
            r#"
[history]
max_entries = 5000
max_text_bytes = 1024
max_image_bytes = 2048
dedupe = false
cleanup_unpinned_after_days = 30
"#,
        )
        .expect("history config file should be written");

        let config = AppConfig::load(&paths).expect("history config file should load");

        assert_eq!(config.history.max_entries, 5000);
        assert_eq!(config.history.max_text_bytes, 1024);
        assert_eq!(config.history.max_image_bytes, 2048);
        assert!(!config.history.dedupe);
        assert_eq!(config.history.cleanup_unpinned_after_days, 30);
    }

    #[test]
    fn zero_history_byte_limits_explicitly_select_unlimited_payloads() {
        let paths = test_paths("unlimited-history-payloads");
        fs::create_dir_all(&paths.config_dir).expect("test config dir should be created");
        fs::write(
            paths.config_path(),
            r#"
[history]
max_text_bytes = 0
max_image_bytes = 0
"#,
        )
        .expect("history config file should be written");

        let config = AppConfig::load(&paths).expect("history config file should load");

        assert_eq!(config.history.max_text_bytes, 0);
        assert_eq!(config.history.max_image_bytes, 0);
    }

    #[test]
    fn paste_ocr_and_ui_options_parse() {
        let paths = test_paths("custom");
        fs::create_dir_all(&paths.config_dir).expect("test config dir should be created");
        fs::write(
            paths.config_path(),
            r#"
[paste]
auto_paste = false
paste_delay_ms = 90
method = "wtype"

[ocr]
enabled = false
command = "custom-ocr"
default_language = "deu"
timeout_seconds = 45
auto_index = true

[ui]
window_width = 1000
window_height = 700
background_opacity = 0.82
resizable = true
preview_default = false
sidebar_width = 360
show_footer_hints = false
reset_on_show = false
auto_focus_search = false
start_view = "secrets"
default_filter = "links"
default_sort = "most-used"
search_placeholder = "Search history"
secrets_search_placeholder = "Search vault"
"#,
        )
        .expect("custom config file should be written");

        let config = AppConfig::load(&paths).expect("custom config file should load");

        assert!(!config.paste.auto_paste);
        assert_eq!(config.paste.paste_delay_ms, 90);
        assert_eq!(config.paste.method, "wtype");
        assert!(!config.ocr.enabled);
        assert_eq!(config.ocr.command, "custom-ocr");
        assert_eq!(config.ocr.default_language, "deu");
        assert_eq!(config.ocr.timeout_seconds, 45);
        assert!(config.ocr.auto_index);
        assert_eq!(config.ui.window_width, 1000);
        assert_eq!(config.ui.window_height, 700);
        assert_eq!(config.ui.background_opacity, Some(0.82));
        assert!(config.ui.resizable);
        assert!(!config.ui.preview_default);
        assert_eq!(config.ui.sidebar_width, 360);
        assert!(!config.ui.show_footer_hints);
        assert!(!config.ui.reset_on_show);
        assert!(!config.ui.auto_focus_search);
        assert_eq!(config.ui.start_view, "secrets");
        assert_eq!(config.ui.default_filter, "links");
        assert_eq!(config.ui.default_sort, "most-used");
        assert_eq!(config.ui.search_placeholder, "Search history");
        assert_eq!(config.ui.secrets_search_placeholder, "Search vault");
    }
}
