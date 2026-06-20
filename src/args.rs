use crate::data_access::DataAccessMode;
use clap::{Parser, ValueEnum};

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interface {
    #[default]
    Ollama,
    Llamacpp,
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
    /// Enable folder monitoring mode
    #[arg(short, long)]
    pub monitor: bool,
    /// Enable combined mode: process existing images then monitor for new ones
    #[arg(short, long)]
    pub combined: bool,
    /// Overwrite existing entries in database (process all files regardless of existing descriptions) (same as --overwrite-policy all)
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
    #[arg(short, long)]
    pub preserve_human: bool,
    /// Path to Immich root directory (containing upload/, thumbs/ folders)
    #[arg(long, default_value = "/var/lib/immich")]
    pub immich_root: String,
    /// `PostgreSQL` connection string (used only in database mode)
    #[arg(
        long,
        default_value = "host=localhost user=postgres dbname=immich password=your_password"
    )]
    pub postgres_url: String,
    /// Data access mode: database (direct `PostgreSQL`) or api (Immich REST API)
    #[arg(short, long, value_enum, default_value = "database")]
    pub data_access_mode: DataAccessMode,
    /// Immich API base URL (required when using api access mode)
    #[arg(long, env = "IMMICH_API_URL")]
    pub immich_api_url: Option<String>,
    /// Immich API authentication key(s) (required when using api access mode).
    /// Provide multiple keys comma-separated for multi-user setups.
    #[arg(
        long,
        env = "IMMICH_API_KEY",
        value_delimiter = ',',
        hide_env_values = true
    )]
    pub immich_api_keys: Vec<String>,
    /// API poll interval in seconds (for Immich API mode)
    #[arg(long, default_value_t = 10)]
    pub api_poll_interval: u32,
    /// Ollama model name for image analysis
    #[arg(long, default_value = "qwen3-vl:4b-thinking-q4_K_M")]
    pub model_name: String,
    /// AI service interface type
    #[arg(long, value_enum, default_value = "ollama")]
    pub interface: Interface,
    /// Host URLs (Ollama or llama.cpp server)
    #[arg(long, default_value = "http://localhost:11434", value_delimiter = ',')]
    pub hosts: Vec<String>,
    /// API key for authentication (llama.cpp server)
    #[arg(long, env = "IMMICH_ANALYZE_API_KEY", hide_env_values = true)]
    pub api_key: Option<String>,
    /// Maximum number of concurrent requests
    #[arg(long, default_value_t = 4)]
    pub max_concurrent: usize,
    /// Host availability check interval in seconds
    #[arg(long, default_value_t = 60)]
    pub unavailable_duration: u64,
    /// HTTP request timeout in seconds
    #[arg(long, default_value_t = 300)]
    pub timeout: u64,
    /// File write timeout in seconds
    #[arg(long, default_value_t = 30)]
    pub file_write_timeout: u64,
    /// File stability check interval in milliseconds
    #[arg(long, default_value_t = 500)]
    pub file_check_interval: u64,
    /// Minimum time between processing identical events in seconds
    #[arg(long, default_value_t = 2)]
    pub event_cooldown: u64,
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
    /// Disable final output with analysis results and statistics after batch processing
    #[arg(long, default_value_t = false)]
    pub no_final_output: bool,
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
