#![warn(non_ascii_idents)]

pub mod args;
pub mod asset_processing;
pub mod config;
pub mod error;
pub mod health;
pub mod host_manager;
pub mod immich_api;
pub mod monitor;
pub mod progress;
pub mod prompt_enricher;
pub mod utils;

rust_i18n::i18n!("locales", fallback = "en");
