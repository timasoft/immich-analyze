use clap::{Parser, ValueEnum};

#[derive(ValueEnum, Debug, Clone, PartialEq)]
pub enum Interface {
    Ollama,
    Llamacpp,
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Enable folder monitoring mode
    #[arg(short, long)]
    pub monitor: bool,
    /// Enable combined mode: process existing images then monitor for new ones
    #[arg(short, long)]
    pub combined: bool,
    /// Ignore existing entries in database
    #[arg(short, long)]
    pub ignore_existing: bool,
    /// Path to Immich root directory (containing upload/, thumbs/ folders)
    #[arg(long, default_value = "/var/lib/immich")]
    pub immich_root: String,
    /// PostgreSQL connection string
    #[arg(
        long,
        default_value = "host=localhost user=postgres dbname=immich password=your_password"
    )]
    pub postgres_url: String,
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
    #[arg(long)]
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
}
