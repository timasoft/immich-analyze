use crate::{args::Interface, llamacpp::LlamaCppHostManager, ollama::OllamaHostManager};
use reqwest::Client;
use std::sync::Arc;
use tokio_postgres::Client as PgClient;

#[derive(Debug, Clone)]
pub struct FileProcessingConfig {
    pub file_write_timeout: u64,
    pub file_check_interval: u64,
    pub ignore_existing: bool,
    pub request_timeout: u64,
}

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub file_write_timeout: u64,
    pub file_check_interval: u64,
    pub event_cooldown: u64,
    pub timeout: u64,
    pub lang: String,
    pub ignore_existing: bool,
    pub hosts: Vec<String>,
    pub interface: Interface,
    pub api_key: Option<String>,
    pub unavailable_duration: u64,
}

#[derive(Clone, Copy)]
pub struct ProcessingContext<'a> {
    pub http_client: &'a Client,
    pub pg_client: &'a PgClient,
    pub model_name: &'a str,
    pub prompt: &'a str,
    pub timeout: u64,
    pub ollama_manager: Option<&'a Arc<OllamaHostManager>>,
    pub llamacpp_manager: Option<&'a Arc<LlamaCppHostManager>>,
}
