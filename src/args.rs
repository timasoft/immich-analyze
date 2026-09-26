use clap::{Parser, ValueEnum};

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[value(rename_all = "lower")]
pub enum Interface {
    #[default]
    Ollama,
    Llamacpp,
    OpenRouter,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThumbnailSize {
    /// Preview rendition (higher resolution, slower analysis)
    #[default]
    Preview,
    /// Thumbnail rendition (lower resolution, faster analysis)
    Thumbnail,
    /// Fullsize rendition (highest resolution, slowest analysis)
    Fullsize,
}

impl std::fmt::Display for ThumbnailSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preview => write!(f, "preview"),
            Self::Thumbnail => write!(f, "thumbnail"),
            Self::Fullsize => write!(f, "fullsize"),
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverwritePolicy {
    /// Skip any asset that already has a description
    #[default]
    None,
    /// Process all assets regardless of existing descriptions
    All,
    /// Skip only if description contains [AI]...[/AI] block; process human-only and empty descriptions
    MissingAi,
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
#[expect(clippy::struct_excessive_bools)]
pub struct Args {
    /// Enable API monitoring mode
    #[arg(short, long)]
    pub monitor: bool,
    /// Enable combined mode: process existing images then monitor for new ones
    #[arg(short, long)]
    pub combined: bool,
    /// Overwrite existing asset descriptions (process all assets regardless of existing descriptions) (same as --overwrite-policy all)
    #[arg(short, long)]
    pub overwrite_existing: bool,
    /// Overwrite policy [default: none]:
    /// none (skip any with description),
    /// all (process everything),
    /// missing-ai (process only if no [AI]...[/AI] block).
    /// Takes precedence over --overwrite-existing.
    #[arg(short = 'O', long, value_enum)]
    pub overwrite_policy: Option<OverwritePolicy>,
    /// When overwriting or adding, preserve human-entered text by only replacing the [AI]...[/AI] block
    #[arg(short, long, conflicts_with = "disable_ai_wrapper")]
    pub preserve_human: bool,
    /// Immich API base URL (required)
    #[arg(long, env = "IMMICH_API_URL")]
    pub immich_api_url: Option<String>,
    /// Immich API authentication key(s) (required).
    /// Provide multiple keys comma-separated for multi-user setups.
    #[arg(
        long,
        env = "IMMICH_API_KEY",
        value_delimiter = ',',
        hide_env_values = true
    )]
    pub immich_api_keys: Vec<String>,
    /// API poll interval in seconds
    #[arg(long, default_value_t = 10)]
    pub api_poll_interval: u32,
    /// Which Immich thumbnail rendition to analyze: preview (higher
    /// resolution, slower analysis), thumbnail (lower resolution, faster)
    /// or fullsize (highest resolution, slowest)
    #[arg(long, value_enum, default_value = "preview")]
    pub thumbnail_size: ThumbnailSize,
    /// Downscale images whose longest edge exceeds this many pixels before sending
    /// them to the AI service (preserves aspect ratio). 0 disables downscaling
    #[arg(long, default_value_t = 0)]
    pub max_image_size: u32,
    /// Model name for image analysis
    #[arg(long, default_value = "qwen3-vl:4b-thinking-q4_K_M")]
    pub model_name: String,
    /// AI service interface type
    #[arg(long, value_enum, default_value = "ollama")]
    pub interface: Interface,
    #[expect(clippy::doc_markdown)]
    /// Host URLs (Ollama, llama.cpp server, or OpenRouter)
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "http://localhost:11434",
        default_value_if("interface", "openrouter", "https://openrouter.ai/api")
    )]
    pub hosts: Vec<String>,
    #[expect(clippy::doc_markdown)]
    /// API key for authentication (llama.cpp server or OpenRouter)
    #[arg(long, env = "IMMICH_ANALYZE_API_KEY", hide_env_values = true)]
    pub api_key: Option<String>,
    /// Disable the startup model existence check against the configured AI hosts
    #[arg(long, default_value_t = false)]
    pub no_preflight_model_check: bool,
    /// Maximum number of concurrent requests
    #[arg(long, default_value_t = 4)]
    pub max_concurrent: usize,
    /// Host availability check interval in seconds
    #[arg(long, default_value_t = 60)]
    pub unavailable_duration: u64,
    /// HTTP request timeout in seconds
    #[arg(long, default_value_t = 300)]
    pub timeout: u64,
    /// Prompt for generating image description
    #[arg(
        long,
        default_value = concat!(
            "Create a detailed description for the image for proper image search functionality. ",
            "In the response, provide only the description without introductory words. ",
            "Also specify the image format (Wallpaper, Screenshot, Drawing, City photo, Selfie, etc.). ",
            "The format must be correct. If in doubt, name the most likely option and don't think too long."
        )
    )]
    pub prompt: String,
    /// Interface language (ru, en)
    #[arg(long, default_value = "")]
    pub lang: String,
    /// Maximum number of retry attempts (0 = infinite)
    #[arg(long, default_value_t = 0)]
    pub max_retries: u32,
    /// Delay between retry cycles in seconds (fixed)
    #[arg(long, default_value_t = 5)]
    pub retry_delay_seconds: u64,
    /// Enable prompt enrichment with asset metadata (date, location, camera info)
    #[arg(long, default_value_t = false)]
    pub enrich_prompt: bool,
    /// Disable [AI]...[/AI] wrapper around AI-generated description
    #[arg(long, default_value_t = false, conflicts_with = "preserve_human")]
    pub disable_ai_wrapper: bool,
    /// Disable final output with analysis results and statistics after batch processing
    #[arg(long, default_value_t = false)]
    pub no_final_output: bool,
    /// Disable waiting for Immich to become available on startup
    #[arg(long, default_value_t = false)]
    pub no_wait_for_immich: bool,
    /// Maximum time in seconds to wait for Immich to become available (0 = no limit)
    #[arg(long, default_value_t = 120)]
    pub wait_timeout: u64,
    /// Interval in seconds between retry attempts when waiting for Immich
    #[arg(long, default_value_t = 5)]
    pub wait_retry_interval: u64,
    /// Port for health check HTTP server (0 to disable)
    #[arg(long, default_value_t = 3000)]
    pub health_port: u16,
}

impl Args {
    #[must_use]
    pub fn effective_overwrite_policy(&self) -> OverwritePolicy {
        match self.overwrite_policy {
            Some(policy) => policy,
            None if self.overwrite_existing => OverwritePolicy::All,
            None => OverwritePolicy::default(),
        }
    }
}
