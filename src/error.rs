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
