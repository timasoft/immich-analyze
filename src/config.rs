use crate::{
    args::{Args, OverwritePolicy, ThumbnailSize},
    host_manager::HostManager,
    immich_api::ImmichApiProvider,
};

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub lang: String,
    pub overwrite_policy: OverwritePolicy,
    pub api_poll_interval: u32,
    pub enrich_prompt: bool,
    pub preserve_human: bool,
    pub disable_ai_wrapper: bool,
    pub thumbnail_size: ThumbnailSize,
}

impl MonitorConfig {
    #[must_use]
    pub fn from_args(args: &Args, lang: &str) -> Self {
        Self {
            lang: lang.to_owned(),
            overwrite_policy: args.effective_overwrite_policy(),
            api_poll_interval: args.api_poll_interval,
            enrich_prompt: args.enrich_prompt,
            preserve_human: args.preserve_human,
            disable_ai_wrapper: args.disable_ai_wrapper,
            thumbnail_size: args.thumbnail_size,
        }
    }
}

#[derive(Clone)]
pub struct ProcessingContext<'a> {
    pub immich_api_provider: &'a ImmichApiProvider,
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
        immich_api_provider: &'a ImmichApiProvider,
        prompt: &'a str,
        host_manager: &'a HostManager,
        overwrite_policy: OverwritePolicy,
        enrich_prompt: bool,
        preserve_human: bool,
        disable_ai_wrapper: bool,
    ) -> Self {
        Self {
            immich_api_provider,
            prompt,
            host_manager,
            overwrite_policy,
            enrich_prompt,
            preserve_human,
            disable_ai_wrapper,
        }
    }
}
