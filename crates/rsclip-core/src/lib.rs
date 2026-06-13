pub mod classify;
pub mod cli;
pub mod colors;
pub mod config;
pub mod db;
pub mod favicons;
pub mod format;
pub mod links;
pub mod mime;
pub mod models;
pub mod notify;
pub mod ocr;
pub mod paste;
pub mod secrets;
pub mod storage;

pub use classify::classify_payload;
pub use config::{
    AppConfig, HistoryConfig, LinksConfig, OcrConfig, PasteConfig, RsclipPaths, UiColors, UiConfig,
};
pub use db::Database;
pub use models::{ClipboardEntry, EntryData, EntryKind, NewEntry, NewEntryData};
