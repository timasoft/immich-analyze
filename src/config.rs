use crate::{
    args::{Interface, OverwritePolicy},
    data_access::DataAccess,
    host_manager::HostManager,
};
use std::num::NonZeroU32;

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub file_write_timeout: u64,
    pub file_check_interval: u64,
    pub event_cooldown: u64,
    pub timeout: u64,
    pub lang: String,
    pub overwrite_policy: OverwritePolicy,
    pub hosts: Vec<String>,
    pub interface: Interface,
    pub api_key: Option<String>,
    pub unavailable_duration: u64,
    pub api_poll_interval: u64,
    pub max_retries: Option<NonZeroU32>,
    pub retry_delay_seconds: u64,
    pub enrich_prompt: bool,
    pub preserve_human: bool,
}

#[derive(Clone, Copy)]
pub struct ProcessingContext<'a> {
    pub data_access: &'a DataAccess,
    pub prompt: &'a str,
    pub host_manager: &'a HostManager,
    pub overwrite_policy: OverwritePolicy,
    pub enrich_prompt: bool,
    pub preserve_human: bool,
}
