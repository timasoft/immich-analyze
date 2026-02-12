use thiserror::Error;

#[derive(Debug, Error)]
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
    #[error("All Ollama hosts are unavailable")]
    AllHostsUnavailable,
    #[error("Ollama request timeout")]
    OllamaRequestTimeout,
    #[error("Llama.cpp request timeout")]
    LlamaCppRequestTimeout,
}
