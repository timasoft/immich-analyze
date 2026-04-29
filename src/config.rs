use crate::{
    args::Interface, data_access::DataAccess, llamacpp::LlamaCppHostManager,
    ollama::OllamaHostManager,
};
use reqwest::Client;
use std::{num::NonZeroU32, sync::Arc, time::Duration};

#[derive(Debug, Clone)]
pub struct FileProcessingConfig {
    pub file_write_timeout: u64,
    pub file_check_interval: u64,
    pub overwrite_existing: bool,
    pub request_timeout: u64,
    pub max_retries: Option<NonZeroU32>,
    pub retry_delay_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub file_write_timeout: u64,
    pub file_check_interval: u64,
    pub event_cooldown: u64,
    pub timeout: u64,
    pub lang: String,
    pub overwrite_existing: bool,
    pub hosts: Vec<String>,
    pub interface: Interface,
    pub api_key: Option<String>,
    pub unavailable_duration: u64,
    pub api_poll_interval: u64,
    pub max_retries: Option<NonZeroU32>,
    pub retry_delay_seconds: u64,
}

#[derive(Clone, Copy)]
pub struct ProcessingContext<'a> {
    pub http_client: &'a Client,
    pub data_access: &'a DataAccess,
    pub model_name: &'a str,
    pub prompt: &'a str,
    pub timeout: u64,
    pub ollama_manager: Option<&'a Arc<OllamaHostManager>>,
    pub llamacpp_manager: Option<&'a Arc<LlamaCppHostManager>>,
    pub max_retries: Option<NonZeroU32>,
    pub retry_delay: Duration,
}
