use reqwest::StatusCode;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, Clone)]
pub enum ImageAnalysisError {
    #[error("Empty file: {asset_id}")]
    EmptyFile { asset_id: Uuid },
    #[error("HTTP error {status} for {subject}: {response}")]
    HttpError {
        status: StatusCode,
        subject: String,
        response: String,
    },
    #[error("Empty response for {asset_id}")]
    EmptyResponse { asset_id: Uuid },
    #[error("JSON parsing error for {subject}: {error}")]
    JsonParsing { subject: String, error: String },
    #[error("Processing error for {asset_id}: {error}")]
    ProcessingError { asset_id: Uuid, error: String },
    #[error("Already processed: {asset_id}")]
    AlreadyProcessed { asset_id: Uuid },
    #[error("Invalid asset ID: {asset_id}")]
    InvalidUuid { asset_id: String },
    #[error("All AI service hosts are unavailable")]
    AllHostsUnavailable,
    #[error("AI service request timeout")]
    AiRequestTimeout,
    #[error("Invalid API key")]
    InvalidApiKey,
    #[error("Invalid configuration: {error}")]
    InvalidConfig { error: String },
    #[error("HTTP client error: {error}")]
    HttpClientError {
        asset_id: Option<Uuid>,
        error: String,
    },
    #[error("Asset not found: {asset_id}")]
    AssetNotFound { asset_id: Uuid },
    #[error("No AI service hosts configured")]
    NoHostsConfigured,
    #[error("AI service {host} does not serve model '{model}'")]
    ModelNotFound {
        model: String,
        host: String,
        closest: Option<String>,
    },
    #[error("AI service rejected the request (HTTP {status}) for {asset_id}: {message}")]
    ProviderRejected {
        status: StatusCode,
        asset_id: Uuid,
        message: String,
    },
    #[error("Permanently rejected for {asset_id}: {reason}")]
    PermanentlyRejected { asset_id: Uuid, reason: String },
}

impl ImageAnalysisError {
    /// Returns a user-facing localized error message
    #[must_use]
    pub fn user_message(&self) -> String {
        match self {
            Self::EmptyFile { asset_id } => {
                rust_i18n::t!("error.empty_file", asset_id = asset_id).to_string()
            }
            Self::HttpError {
                status,
                subject,
                response,
            } => rust_i18n::t!(
                "error.http_error_with_details",
                subject = subject,
                status = status.to_string(),
                response = response
            )
            .to_string(),
            Self::EmptyResponse { asset_id } => {
                rust_i18n::t!("error.empty_response", asset_id = asset_id).to_string()
            }
            Self::JsonParsing { subject, error } => rust_i18n::t!(
                "error.json_parsing_with_details",
                subject = subject,
                error = error
            )
            .to_string(),
            Self::AllHostsUnavailable => rust_i18n::t!("error.all_hosts_unavailable").to_string(),
            Self::AiRequestTimeout => rust_i18n::t!("error.ai_request_timeout").to_string(),
            Self::ProcessingError { asset_id, error } => format!(
                "{}\n{}",
                error,
                rust_i18n::t!("error.critical_processing_error", asset_id = asset_id),
            ),
            Self::AlreadyProcessed { asset_id } => {
                rust_i18n::t!("main.asset_already_described", asset_id = asset_id).to_string()
            }
            Self::InvalidUuid { asset_id } => format!(
                "{}\n{}",
                rust_i18n::t!("error.critical_processing_error", asset_id = asset_id),
                self
            ),
            Self::HttpClientError { asset_id, error } => asset_id.as_ref().map_or_else(
                || rust_i18n::t!("error.ai_host_connection_failed", error = error).to_string(),
                |asset| {
                    rust_i18n::t!(
                        "error.ai_host_connection_failed_for_asset",
                        asset_id = asset,
                        error = error
                    )
                    .to_string()
                },
            ),
            Self::InvalidConfig { error } => {
                rust_i18n::t!("error.invalid_config", error = error).to_string()
            }
            Self::InvalidApiKey => rust_i18n::t!("error.invalid_api_key").to_string(),
            Self::AssetNotFound { asset_id } => {
                rust_i18n::t!("error.asset_not_found_in_library", asset_id = asset_id).to_string()
            }
            Self::NoHostsConfigured => rust_i18n::t!("error.no_hosts_configured").to_string(),
            Self::ModelNotFound {
                model,
                host,
                closest,
            } => {
                let message =
                    rust_i18n::t!("error.model_not_found", model = model, host = host).to_string();
                if let Some(nearest) = closest {
                    format!(
                        "{message}\n{}",
                        rust_i18n::t!("error.did_you_mean", model = nearest)
                    )
                } else {
                    message
                }
            }
            Self::ProviderRejected {
                status,
                asset_id,
                message,
            } => rust_i18n::t!(
                "error.provider_rejected",
                asset_id = asset_id,
                status = status.to_string(),
                message = message
            )
            .to_string(),
            Self::PermanentlyRejected { asset_id, reason } => rust_i18n::t!(
                "error.permanently_rejected",
                asset_id = asset_id,
                reason = reason
            )
            .to_string(),
        }
    }

    /// Check if this error is retryable (transient)
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            // Retryable errors
            Self::HttpError { status, .. } => {
                status.is_server_error() || *status == StatusCode::TOO_MANY_REQUESTS
            }
            Self::AllHostsUnavailable | Self::AiRequestTimeout | Self::HttpClientError { .. } => {
                true
            }

            // Non-retryable errors
            Self::EmptyFile { .. }
            | Self::InvalidUuid { .. }
            | Self::InvalidApiKey
            | Self::InvalidConfig { .. }
            | Self::EmptyResponse { .. }
            | Self::JsonParsing { .. }
            | Self::AlreadyProcessed { .. }
            | Self::ProcessingError { .. }
            | Self::AssetNotFound { .. }
            | Self::NoHostsConfigured
            | Self::ModelNotFound { .. }
            | Self::ProviderRejected { .. }
            | Self::PermanentlyRejected { .. } => false,
        }
    }

    /// Returns `true` for errors that are expected outcomes of background polling and
    /// should be silenced rather than logged as failures in the monitor loop.
    #[must_use]
    pub const fn is_silent_background_error(&self) -> bool {
        match self {
            // Silenced errors
            Self::AlreadyProcessed { .. }
            | Self::AssetNotFound { .. }
            | Self::PermanentlyRejected { .. } => true,

            // Errors that are logged as background failures
            Self::EmptyFile { .. }
            | Self::InvalidUuid { .. }
            | Self::InvalidApiKey
            | Self::InvalidConfig { .. }
            | Self::HttpError { .. }
            | Self::EmptyResponse { .. }
            | Self::JsonParsing { .. }
            | Self::ProcessingError { .. }
            | Self::AllHostsUnavailable
            | Self::AiRequestTimeout
            | Self::HttpClientError { .. }
            | Self::NoHostsConfigured
            | Self::ModelNotFound { .. }
            | Self::ProviderRejected { .. } => false,
        }
    }
}
