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
    #[error("Ollama request timeout")]
    OllamaRequestTimeout,
    #[error("Llama.cpp request timeout")]
    LlamaCppRequestTimeout,
    #[error("Invalid API key")]
    InvalidApiKey,
    #[error("Invalid configuration: {error}")]
    InvalidConfig { error: String },
    #[error("HTTP client error: {error}")]
    HttpClientError { error: String },
}

impl ImageAnalysisError {
    /// Returns a user-facing localized error message
    #[must_use]
    pub fn user_message(&self) -> String {
        match self {
            ImageAnalysisError::EmptyFile { filename } => {
                rust_i18n::t!("error.empty_file", filename = filename).to_string()
            }
            ImageAnalysisError::HttpError {
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
            ImageAnalysisError::EmptyResponse { filename } => {
                rust_i18n::t!("error.empty_response", filename = filename).to_string()
            }
            ImageAnalysisError::JsonParsing { filename, error } => rust_i18n::t!(
                "error.json_parsing_with_details",
                filename = filename,
                error = error
            )
            .to_string(),
            ImageAnalysisError::FileWriteTimeout { filename, timeout } => rust_i18n::t!(
                "error.file_write_timeout_with_details",
                filename = filename,
                timeout = timeout.to_string()
            )
            .to_string(),
            ImageAnalysisError::DatabaseError { error } => {
                rust_i18n::t!("error.database_error", error = error).to_string()
            }
            ImageAnalysisError::AllHostsUnavailable => {
                rust_i18n::t!("error.all_hosts_unavailable").to_string()
            }
            ImageAnalysisError::OllamaRequestTimeout => {
                rust_i18n::t!("error.ollama_request_timeout").to_string()
            }
            ImageAnalysisError::LlamaCppRequestTimeout => {
                rust_i18n::t!("error.llamacpp_request_timeout").to_string()
            }
            ImageAnalysisError::ProcessingError { filename, error } => format!(
                "{}\n{}",
                error,
                rust_i18n::t!("error.critical_processing_error", filename = filename),
            ),
            ImageAnalysisError::AlreadyProcessed { filename } => {
                rust_i18n::t!("error.critical_processing_error", filename = filename).to_string()
            }
            ImageAnalysisError::InvalidUuid { filename } => format!(
                "{}\n{}",
                rust_i18n::t!("error.critical_processing_error", filename = filename),
                self
            ),
            ImageAnalysisError::InvalidImmichStructure { error } => format!(
                "{}\n{}",
                rust_i18n::t!("error.critical_processing_error", filename = "unknown"),
                error
            ),
            ImageAnalysisError::InvalidApiKey => format!(
                "{}\n{}",
                rust_i18n::t!("error.critical_processing_error", filename = "unknown"),
                self
            ),
            ImageAnalysisError::InvalidConfig { error } => format!(
                "{}\n{}",
                rust_i18n::t!("error.critical_processing_error", filename = "unknown"),
                error
            ),
            ImageAnalysisError::HttpClientError { error } => format!(
                "{}\n{}",
                rust_i18n::t!("error.critical_processing_error", filename = "unknown"),
                error
            ),
        }
    }

    /// Check if this error is retryable (transient)
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            // Retryable errors
            ImageAnalysisError::HttpError { status, .. } => {
                *status == 0 || (*status >= 500 && *status <= 599) || *status == 429
            }
            ImageAnalysisError::AllHostsUnavailable => true,
            ImageAnalysisError::OllamaRequestTimeout => true,
            ImageAnalysisError::LlamaCppRequestTimeout => true,
            ImageAnalysisError::HttpClientError { .. } => true,

            // Non-retryable errors
            ImageAnalysisError::EmptyFile { .. } => false,
            ImageAnalysisError::InvalidUuid { .. } => false,
            ImageAnalysisError::InvalidImmichStructure { .. } => false,
            ImageAnalysisError::InvalidApiKey => false,
            ImageAnalysisError::InvalidConfig { .. } => false,
            ImageAnalysisError::EmptyResponse { .. } => false,
            ImageAnalysisError::JsonParsing { .. } => false,
            ImageAnalysisError::AlreadyProcessed { .. } => false,
            ImageAnalysisError::DatabaseError { .. } => false,
            ImageAnalysisError::ProcessingError { .. } => false,
            ImageAnalysisError::FileWriteTimeout { .. } => false,
        }
    }
}
