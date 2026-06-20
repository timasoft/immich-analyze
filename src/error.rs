use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum ImageAnalysisError {
    #[error("Empty file: {filename}")]
    EmptyFile { filename: String },
    #[error("HTTP error {status} for {filename}: {response}")]
    HttpError {
        status: u16,
        filename: String,
        response: String,
    },
    #[error("Empty response for {filename}")]
    EmptyResponse { filename: String },
    #[error("JSON parsing error for {filename}: {error}")]
    JsonParsing { filename: String, error: String },
    #[error("File write timeout {timeout}s for {filename}")]
    FileWriteTimeout { timeout: u64, filename: String },
    #[error("Processing error for {filename}: {error}")]
    ProcessingError { filename: String, error: String },
    #[error("Already processed: {filename}")]
    AlreadyProcessed { filename: String },
    #[error("Database error: {error}")]
    DatabaseError { error: String },
    #[error("Invalid UUID in filename: {filename}")]
    InvalidUuid { filename: String },
    #[error("Invalid Immich structure: {error}")]
    InvalidImmichStructure { error: String },
    #[error("All AI service hosts are unavailable")]
    AllHostsUnavailable,
    #[error("AI service request timeout")]
    AiRequestTimeout,
    #[error("Invalid API key")]
    InvalidApiKey,
    #[error("Invalid configuration: {error}")]
    InvalidConfig { error: String },
    #[error("HTTP client error: {error}")]
    HttpClientError { error: String },
    #[error("IO error for {path}: {error}")]
    IoError { path: String, error: String },
}

impl ImageAnalysisError {
    /// Returns a user-facing localized error message
    #[must_use]
    pub fn user_message(&self) -> String {
        match self {
            Self::EmptyFile { filename } => {
                rust_i18n::t!("error.empty_file", filename = filename).to_string()
            }
            Self::HttpError {
                status,
                filename,
                response,
            } => rust_i18n::t!(
                "error.http_error_with_details",
                filename = filename,
                status = status.to_string(),
                response = response
            )
            .to_string(),
            Self::EmptyResponse { filename } => {
                rust_i18n::t!("error.empty_response", filename = filename).to_string()
            }
            Self::JsonParsing { filename, error } => rust_i18n::t!(
                "error.json_parsing_with_details",
                filename = filename,
                error = error
            )
            .to_string(),
            Self::FileWriteTimeout { filename, timeout } => rust_i18n::t!(
                "error.file_write_timeout_with_details",
                filename = filename,
                timeout = timeout.to_string()
            )
            .to_string(),
            Self::DatabaseError { error } => {
                rust_i18n::t!("error.database_error", error = error).to_string()
            }
            Self::AllHostsUnavailable => rust_i18n::t!("error.all_hosts_unavailable").to_string(),
            Self::AiRequestTimeout => rust_i18n::t!("error.ai_request_timeout").to_string(),
            Self::ProcessingError { filename, error } => format!(
                "{}\n{}",
                error,
                rust_i18n::t!("error.critical_processing_error", filename = filename),
            ),
            Self::AlreadyProcessed { filename } => {
                rust_i18n::t!("main.file_already_in_database", filename = filename).to_string()
            }
            Self::InvalidUuid { filename } => format!(
                "{}\n{}",
                rust_i18n::t!("error.critical_processing_error", filename = filename),
                self
            ),
            Self::InvalidImmichStructure { error }
            | Self::InvalidConfig { error }
            | Self::HttpClientError { error } => format!(
                "{}\n{}",
                rust_i18n::t!("error.critical_processing_error", filename = "unknown"),
                error
            ),
            Self::InvalidApiKey => format!(
                "{}\n{}",
                rust_i18n::t!("error.critical_processing_error", filename = "unknown"),
                self
            ),
            Self::IoError { path, error } => {
                rust_i18n::t!("error.io_error", path = path, error = error).to_string()
            }
        }
    }

    /// Check if this error is retryable (transient)
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            // Retryable errors
            Self::HttpError { status, .. } => {
                *status == 0 || (*status >= 500 && *status <= 599) || *status == 429
            }
            Self::AllHostsUnavailable | Self::AiRequestTimeout | Self::HttpClientError { .. } => {
                true
            }

            // Non-retryable errors
            Self::EmptyFile { .. }
            | Self::InvalidUuid { .. }
            | Self::InvalidImmichStructure { .. }
            | Self::InvalidApiKey
            | Self::InvalidConfig { .. }
            | Self::EmptyResponse { .. }
            | Self::JsonParsing { .. }
            | Self::AlreadyProcessed { .. }
            | Self::DatabaseError { .. }
            | Self::ProcessingError { .. }
            | Self::FileWriteTimeout { .. }
            | Self::IoError { .. } => false,
        }
    }
}
