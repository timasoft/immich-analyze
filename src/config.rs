use crate::{
    args::{Args, Interface, OverwritePolicy},
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
    pub api_poll_interval: u32,
    pub max_retries: Option<NonZeroU32>,
    pub retry_delay_seconds: u64,
    pub enrich_prompt: bool,
    pub preserve_human: bool,
    pub disable_ai_wrapper: bool,
}

impl MonitorConfig {
    #[must_use]
    pub fn from_args(args: &Args, lang: &str) -> Self {
        Self {
            file_write_timeout: args.file_write_timeout,
            file_check_interval: args.file_check_interval,
            event_cooldown: args.event_cooldown,
            timeout: args.timeout,
            lang: lang.to_owned(),
            overwrite_policy: args.effective_overwrite_policy(),
            hosts: args.hosts.clone(),
            interface: args.interface,
            api_key: args.api_key.clone(),
            unavailable_duration: args.unavailable_duration,
            api_poll_interval: args.api_poll_interval,
            max_retries: NonZeroU32::new(args.max_retries),
            retry_delay_seconds: args.retry_delay_seconds,
            enrich_prompt: args.enrich_prompt,
            preserve_human: args.preserve_human,
            disable_ai_wrapper: args.disable_ai_wrapper,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ProcessingContext<'a> {
    pub data_access: &'a DataAccess,
    pub prompt: &'a str,
    pub host_manager: &'a HostManager,
    pub overwrite_policy: OverwritePolicy,
    pub enrich_prompt: bool,
    pub preserve_human: bool,
    pub disable_ai_wrapper: bool,
}

impl<'a> ProcessingContext<'a> {
    #[must_use]
    pub const fn new(
        data_access: &'a DataAccess,
        prompt: &'a str,
        host_manager: &'a HostManager,
        overwrite_policy: OverwritePolicy,
        enrich_prompt: bool,
        preserve_human: bool,
        disable_ai_wrapper: bool,
    ) -> Self {
        Self {
            data_access,
            prompt,
            host_manager,
            overwrite_policy,
            enrich_prompt,
            preserve_human,
            disable_ai_wrapper,
        }
    }
}
