#![warn(non_ascii_idents)]

//! `immich_analyze` is the library behind the `immich-analyze` binary: it generates image
//! descriptions for [Immich](https://immich.app) with a vision-language model served by Ollama,
//! llama.cpp, or `OpenRouter`, and writes them back through the Immich REST API.
//!
//! Analysis of a single asset runs through [`utils::check_overwrite_policy`], which decides
//! whether it should be analyzed at all; [`prompt_enricher::enrich_prompt_if_needed`], which
//! optionally folds asset metadata into the prompt; [`host_manager::HostManager`], which decodes
//! the image and re-encodes it as a downscaled base64 PNG before sending it to a host with
//! failover between hosts and retries; [`utils::build_final_description`], which merges the answer
//! with any human-written text; and [`immich_api::ApiProvider`], which writes the result back.
//!
//! The main entry points are [`asset_processing::process_assets_concurrently`] for batches and
//! [`monitor::run`] for watching newly added assets; [`args::Args`] holds the configuration for
//! both.
//!
//! All modules are public so the individual pieces can be reused, but treat everything below the
//! entry points as internal: those APIs may change in any release without a major version bump.

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
