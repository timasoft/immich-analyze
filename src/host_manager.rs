use crate::{
    args::Interface,
    error::{ErrorSubject, ImageAnalysisError},
    utils::{
        ProviderMessageClass, classify_provider_message, closest_name, format_error_chain,
        image_bytes_to_png_base64, is_model_served,
    },
};
use bytes::Bytes;
use log::{debug, error, info, warn};
use reqwest::{Client, StatusCode, header::HeaderValue};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug)]
pub struct ImageAnalysisResult {
    pub description: String,
    pub asset_id: Uuid,
}

impl Interface {
    /// Returns the API endpoint path for the given interface.
    #[inline]
    pub const fn endpoint(self) -> &'static str {
        match self {
            Self::Ollama => "/api/chat",
            Self::Llamacpp | Self::OpenRouter => "/v1/chat/completions",
        }
    }

    /// Returns the models list endpoint path for the given interface.
    #[inline]
    pub const fn models_list_endpoint(self) -> &'static str {
        match self {
            Self::Ollama => "/api/tags",
            Self::Llamacpp | Self::OpenRouter => "/v1/models",
        }
    }

    /// Returns `true` if the interface supports Bearer token authentication.
    #[inline]
    pub const fn supports_bearer_auth(self) -> bool {
        match self {
            Self::Ollama => false,
            Self::Llamacpp | Self::OpenRouter => true,
        }
    }

    /// Returns `true` if a host without a models list can be verified via a test completion.
    #[inline]
    pub const fn supports_completion_fallback(self) -> bool {
        match self {
            Self::Ollama | Self::OpenRouter => false,
            Self::Llamacpp => true,
        }
    }

    /// Returns `true` if the interface uses Ollama-style name:tag aliasing
    /// (a bare name like `llava` is equivalent to `llava:latest`).
    #[inline]
    pub const fn ollama_tag_semantics(self) -> bool {
        match self {
            Self::Ollama => true,
            Self::Llamacpp | Self::OpenRouter => false,
        }
    }

    /// Returns the provider-reported message when this response body is a permanent
    /// content-policy rejection for this interface. Only messages that positively signal
    /// a content-policy block qualify; transient and unknown messages do not.
    pub fn permanent_rejection_message(self, json_value: &Value) -> Option<String> {
        match self {
            Self::Ollama | Self::Llamacpp => None,
            Self::OpenRouter => {
                let error = json_value.get("error")?;
                let message = match error {
                    Value::String(msg) => Some(msg.as_str()),
                    Value::Object(_) => error.get("message").and_then(Value::as_str),
                    _ => None,
                }?;
                let trimmed = message.trim();
                (classify_provider_message(trimmed) == ProviderMessageClass::ContentPolicyBlock)
                    .then(|| trimmed.to_owned())
            }
        }
    }

    /// Builds the minimal test completion request body for the given interface.
    pub fn build_test_completion_body(self, model_name: &str) -> Value {
        match self {
            Self::Ollama => serde_json::json!({
                "model": model_name,
                "messages": [
                    {
                        "role": "user",
                        "content": "Reply with OK"
                    }
                ],
                "stream": false,
            }),
            Self::Llamacpp | Self::OpenRouter => serde_json::json!({
                "model": model_name,
                "messages": [
                    {
                        "role": "user",
                        "content": "Reply with OK"
                    }
                ],
                "max_tokens": 8_i32,
                "stream": false,
            }),
        }
    }

    /// Extracts model identifiers from a models list response body.
    pub fn model_names(self, json_value: &Value) -> Vec<String> {
        match self {
            Self::Ollama => json_value
                .get("models")
                .and_then(Value::as_array)
                .map_or_else(Vec::new, |models| {
                    models
                        .iter()
                        .filter_map(|model| model.get("name").and_then(Value::as_str))
                        .map(str::to_owned)
                        .collect()
                }),
            Self::Llamacpp | Self::OpenRouter => json_value
                .get("data")
                .and_then(Value::as_array)
                .map_or_else(Vec::new, |models| {
                    models
                        .iter()
                        .filter_map(|model| model.get("id").and_then(Value::as_str))
                        .map(str::to_owned)
                        .collect()
                }),
        }
    }

    /// Parses the response JSON and extracts the content string for the given interface.
    pub fn parse_response(self, json_value: &Value) -> Option<&str> {
        match self {
            Self::Ollama => json_value
                .get("message")
                .and_then(|msg| msg.get("content"))
                .and_then(|content| content.as_str()),
            Self::Llamacpp | Self::OpenRouter => json_value
                .get("choices")
                .and_then(|choices| choices.as_array())
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|msg| msg.get("content"))
                .and_then(|content| content.as_str()),
        }
    }

    /// Builds the JSON request body specific to the AI service interface.
    pub fn build_request_body(self, model_name: &str, prompt: &str, base64_image: &str) -> Value {
        match self {
            Self::Ollama => serde_json::json!({
                "model": model_name,
                "messages": [
                    {
                        "role": "user",
                        "content": prompt,
                        "images": [base64_image]
                    }
                ],
                "stream": false,
            }),
            Self::Llamacpp | Self::OpenRouter => serde_json::json!({
                "model": model_name,
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": prompt
                            },
                            {
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:image/png;base64,{}", base64_image)
                                }
                            }
                        ]
                    }
                ],
                "stream": false,
            }),
        }
    }

    /// Returns interface-specific HTTP headers to attach to requests, if any.
    pub fn additional_headers(self) -> Option<Vec<(&'static str, HeaderValue)>> {
        match self {
            Self::Ollama | Self::Llamacpp => None,
            Self::OpenRouter => Some(vec![
                (
                    "HTTP-Referer",
                    HeaderValue::from_static("https://github.com/timasoft/immich-analyze"),
                ),
                (
                    "X-OpenRouter-Title",
                    HeaderValue::from_static("immich-analyze"),
                ),
            ]),
        }
    }
}

enum HostCheck {
    Present,
    Missing { closest: Option<String> },
    Failed(ImageAnalysisError),
}

#[derive(Debug, Clone)]
pub struct HostManager {
    hosts: Vec<String>,
    interface: Interface,
    client: Client,
    model_name: String,
    timeout: u64,
    max_retries: Option<NonZeroU32>,
    retry_delay: Duration,
    unavailable_hosts: Arc<Mutex<HashMap<String, Instant>>>,
    permanently_unavailable_hosts: Arc<Mutex<HashSet<String>>>,
    unavailable_duration: Duration,
    api_key: Option<String>,
    max_image_size: u32,
}

impl HostManager {
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        hosts: Vec<String>,
        interface: Interface,
        client: Client,
        model_name: String,
        timeout: u64,
        max_retries: Option<NonZeroU32>,
        retry_delay: Duration,
        unavailable_duration: Duration,
        api_key: Option<String>,
        max_image_size: u32,
    ) -> Self {
        Self {
            hosts,
            interface,
            client,
            model_name,
            timeout,
            max_retries,
            retry_delay,
            unavailable_hosts: Arc::new(Mutex::new(HashMap::new())),
            permanently_unavailable_hosts: Arc::new(Mutex::new(HashSet::new())),
            unavailable_duration,
            api_key,
            max_image_size,
        }
    }

    pub fn get_available_host(&self) -> Result<String, ImageAnalysisError> {
        debug!(
            "Looking for available {:?} hosts. Total hosts: {}",
            self.interface,
            self.hosts.len()
        );

        let mut unavailable = self
            .unavailable_hosts
            .lock()
            .expect("unavailable_hosts mutex poisoned");
        let permanently_unavailable = self
            .permanently_unavailable_hosts
            .lock()
            .expect("permanently_unavailable_hosts mutex poisoned");
        let now = Instant::now();
        let original_count = unavailable.len();
        unavailable.retain(|_, timestamp| {
            now.saturating_duration_since(*timestamp) < self.unavailable_duration
        });

        if let Some(removed_count) = original_count.checked_sub(unavailable.len())
            && removed_count > 0
        {
            debug!("Cleaned up {removed_count} expired unavailable hosts");
        }

        debug!(
            "Currently unavailable hosts: {:?}",
            unavailable.keys().collect::<Vec<_>>()
        );

        for host in &self.hosts {
            if !unavailable.contains_key(host) && !permanently_unavailable.contains(host) {
                info!("Selected available {:?} host: {}", self.interface, host);
                return Ok(host.clone());
            }
        }
        drop(permanently_unavailable);

        if let Some((host, timestamp)) = unavailable.iter().min_by_key(|(_, timestamp)| *timestamp)
        {
            warn!(
                "All {:?} hosts unavailable. Using oldest unavailable host: {} (unavailable for {:?})",
                self.interface,
                host,
                now.saturating_duration_since(*timestamp)
            );
            return Ok(host.clone());
        }
        drop(unavailable);

        error!("No {:?} hosts available at all", self.interface);
        Err(ImageAnalysisError::AllHostsUnavailable)
    }

    pub fn mark_host_unavailable(&self, host: &str) {
        self.unavailable_hosts
            .lock()
            .expect("unavailable_hosts mutex poisoned")
            .insert(host.to_owned(), Instant::now());
        println!(
            "{}",
            rust_i18n::t!("host_manager.host_marked_unavailable", host = host)
        );
    }

    pub fn mark_host_unavailable_forever(&self, host: &str) {
        self.permanently_unavailable_hosts
            .lock()
            .expect("permanently_unavailable_hosts mutex poisoned")
            .insert(host.to_owned());
        println!(
            "{}",
            rust_i18n::t!(
                "host_manager.host_marked_permanently_unavailable",
                host = host
            )
        );
    }

    pub async fn analyze_image(
        &self,
        image_data: Bytes,
        prompt: &str,
        asset_id: Uuid,
    ) -> Result<ImageAnalysisResult, ImageAnalysisError> {
        info!(
            "Starting {:?} analysis for asset: {asset_id}",
            self.interface
        );
        debug!("Model: {}, Timeout: {}s", self.model_name, self.timeout);

        let max_image_size = self.max_image_size;
        let base64_image = tokio::task::spawn_blocking(move || {
            image_bytes_to_png_base64(image_data, asset_id, max_image_size)
        })
        .await
        .map_err(|err| ImageAnalysisError::ProcessingError {
            asset_id,
            error: format_error_chain(&err),
        })??;

        let request_body =
            self.interface
                .build_request_body(&self.model_name, prompt, &base64_image);
        drop(base64_image);

        let endpoint = self.interface.endpoint();

        let mut attempt: u32 = 0;
        let mut last_error = None;
        loop {
            attempt = attempt.saturating_add(1);

            if self.max_retries.is_some() || attempt > 1 {
                info!(
                    "Retry attempt {}/{} for image {}",
                    attempt,
                    self.max_retries
                        .map_or_else(|| "∞".to_owned(), |max| max.to_string()),
                    asset_id
                );
            }

            // Try each available host until we get a successful response
            for _ in 0..self.hosts.len() {
                let host = match self.get_available_host() {
                    Ok(host) => host,
                    Err(err) => {
                        error!(
                            "Failed to get available {:?} host: {:?}",
                            self.interface, err
                        );
                        return Err(err);
                    }
                };

                let url = format!("{}{}", host.trim_end_matches('/'), endpoint);
                info!("Making {:?} request to: {}", self.interface, url);

                let mut request = self.client.post(&url).json(&request_body);

                if self.interface.supports_bearer_auth() {
                    if let Some(api_key) = &self.api_key {
                        debug!("Adding Authorization header with API key");
                        request = request.header("Authorization", format!("Bearer {api_key}"));
                    } else {
                        debug!("No API key provided for {:?} request", self.interface);
                    }
                }

                if let Some(additional_headers) = self.interface.additional_headers() {
                    for (header, value) in additional_headers {
                        debug!("Adding custom header: {header}={value:?}");
                        request = request.header(header, value);
                    }
                }

                match tokio::time::timeout(
                    Duration::from_secs(self.timeout.saturating_add(1)),
                    async {
                        debug!("Sending {:?} request...", self.interface);
                        request.send().await
                    },
                )
                .await
                {
                    Ok(Ok(response)) => {
                        let status = response.status();
                        debug!(
                            "Received {:?} response: {} {}",
                            self.interface,
                            status,
                            status.canonical_reason().unwrap_or("")
                        );

                        if response.status().is_success() {
                            let response_text = response.text().await.map_err(|err| {
                                error!("Failed to read response body: {err}");
                                ImageAnalysisError::ProcessingError {
                                    asset_id,
                                    error: format_error_chain(&err),
                                }
                            })?;

                            debug!("Response body length: {} chars", response_text.len());

                            match serde_json::from_str::<Value>(&response_text) {
                                Ok(json_value) => {
                                    if let Some(provider_message) =
                                        self.interface.permanent_rejection_message(&json_value)
                                    {
                                        error!(
                                            "{:?} provider rejected request for {}: {}",
                                            self.interface, asset_id, provider_message
                                        );
                                        return Err(ImageAnalysisError::ProviderRejected {
                                            status,
                                            asset_id,
                                            message: provider_message,
                                        });
                                    }
                                    let content = self.interface.parse_response(&json_value);

                                    if let Some(raw_description) = content {
                                        let description = raw_description.trim().to_owned();
                                        if description.is_empty() {
                                            warn!("Empty response for image: {asset_id}");
                                            last_error = Some(ImageAnalysisError::EmptyResponse {
                                                asset_id,
                                            });
                                        } else {
                                            info!(
                                                "{:?} analysis successful for {}, description length: {}",
                                                self.interface,
                                                asset_id,
                                                description.len()
                                            );
                                            return Ok(ImageAnalysisResult {
                                                description,
                                                asset_id,
                                            });
                                        }
                                    } else {
                                        error!(
                                            "Failed to extract content from response for {asset_id}"
                                        );
                                        last_error = Some(ImageAnalysisError::JsonParsing {
                                            subject: ErrorSubject::Asset(asset_id),
                                            error: "No content field found in response".to_owned(),
                                        });
                                    }
                                }
                                Err(parse_error) => {
                                    error!(
                                        "Failed to parse response as JSON for {asset_id}: {parse_error}"
                                    );
                                    let error = ImageAnalysisError::JsonParsing {
                                        subject: ErrorSubject::Asset(asset_id),
                                        error: format_error_chain(&parse_error),
                                    };
                                    if !error.is_retryable() {
                                        return Err(error);
                                    }
                                    last_error = Some(error);
                                }
                            }
                        } else {
                            let response_text = response.text().await.unwrap_or_default();
                            error!(
                                "{:?} HTTP error {} for {}: {}",
                                self.interface, status, asset_id, response_text
                            );
                            let error = if matches!(
                                status,
                                StatusCode::BAD_REQUEST
                                    | StatusCode::FORBIDDEN
                                    | StatusCode::UNPROCESSABLE_ENTITY
                            ) && let Ok(json_value) =
                                serde_json::from_str::<Value>(&response_text)
                                && let Some(provider_message) =
                                    self.interface.permanent_rejection_message(&json_value)
                            {
                                ImageAnalysisError::ProviderRejected {
                                    status,
                                    asset_id,
                                    message: provider_message,
                                }
                            } else {
                                ImageAnalysisError::HttpError {
                                    status,
                                    subject: ErrorSubject::Asset(asset_id),
                                    response: response_text,
                                }
                            };
                            if !error.is_retryable() {
                                return Err(error);
                            }
                            last_error = Some(error);
                        }
                    }
                    Ok(Err(err)) => {
                        error!(
                            "{:?} request failed for {}: {}",
                            self.interface, asset_id, err
                        );
                        last_error = Some(ImageAnalysisError::HttpClientError {
                            asset_id: Some(asset_id),
                            error: format_error_chain(&err),
                        });
                    }
                    Err(_) => {
                        last_error = Some(ImageAnalysisError::AiRequestTimeout);
                    }
                }
                warn!(
                    "Marking {:?} host as unavailable due to error: {}",
                    self.interface, host
                );
                self.mark_host_unavailable(&host);
            }

            if let Some(last_err) = &last_error
                && !last_err.is_retryable()
            {
                return Err(last_err.clone());
            }

            if self.max_retries.is_none_or(|max| attempt < max.get()) {
                info!(
                    "All hosts failed for {}, waiting {}s before retry",
                    asset_id,
                    self.retry_delay.as_secs()
                );
                tokio::time::sleep(self.retry_delay).await;
            } else {
                break;
            }
        }
        Err(last_error.unwrap_or(ImageAnalysisError::AllHostsUnavailable))
    }

    pub async fn check_model_available_and_mark_unavailable_hosts(
        &self,
    ) -> Result<(), ImageAnalysisError> {
        if self.hosts.is_empty() {
            return Err(ImageAnalysisError::NoHostsConfigured);
        }

        let mut last_error: Option<ImageAnalysisError> = None;
        let mut at_least_one = false;
        for host in &self.hosts {
            match self.check_host(host).await {
                HostCheck::Present => {
                    info!(
                        "Configured model '{}' found on host {host}",
                        self.model_name
                    );
                    at_least_one = true;
                }
                HostCheck::Missing { closest } => {
                    warn!(
                        "Configured model '{}' not found on host {host}",
                        self.model_name
                    );
                    self.mark_host_unavailable_forever(host);
                    last_error = Some(ImageAnalysisError::ModelNotFound {
                        model: self.model_name.clone(),
                        host: host.clone(),
                        closest,
                    });
                }
                HostCheck::Failed(err) => {
                    error!("Model preflight check failed for host {host}: {err}");
                    self.mark_host_unavailable(host);
                    last_error = Some(err);
                }
            }
        }

        if at_least_one {
            Ok(())
        } else {
            Err(last_error.unwrap_or(ImageAnalysisError::NoHostsConfigured))
        }
    }

    async fn check_host(&self, host: &str) -> HostCheck {
        let url = format!(
            "{}{}",
            host.trim_end_matches('/'),
            self.interface.models_list_endpoint()
        );
        let mut request = self.client.get(&url);
        if self.interface.supports_bearer_auth()
            && let Some(key) = self.api_key.as_deref()
        {
            request = request.header("Authorization", format!("Bearer {key}"));
        }

        let available: Vec<String> = match request.send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<Value>().await {
                    Ok(value) => self.interface.model_names(&value),
                    Err(err) => {
                        return HostCheck::Failed(ImageAnalysisError::JsonParsing {
                            subject: ErrorSubject::ModelsList,
                            error: format_error_chain(&err),
                        });
                    }
                }
            }
            Ok(_) if self.interface.supports_completion_fallback() => {
                return self.test_completion(host).await;
            }
            Ok(response) => {
                let status = response.status();
                return HostCheck::Failed(ImageAnalysisError::HttpError {
                    status,
                    subject: ErrorSubject::ModelsList,
                    response: response.text().await.unwrap_or_default(),
                });
            }
            Err(err) => {
                return HostCheck::Failed(ImageAnalysisError::HttpClientError {
                    asset_id: None,
                    error: format_error_chain(&err),
                });
            }
        };

        let closest = closest_name(&self.model_name, &available);
        if is_model_served(self.interface, &self.model_name, &available) {
            HostCheck::Present
        } else if self.interface.supports_completion_fallback() {
            match self.test_completion(host).await {
                HostCheck::Missing { .. } => HostCheck::Missing { closest },
                host_check => host_check,
            }
        } else {
            HostCheck::Missing { closest }
        }
    }

    /// Verifies the model via a minimal chat completion, used when a host does not expose a models list
    async fn test_completion(&self, host: &str) -> HostCheck {
        let url = format!(
            "{}{}",
            host.trim_end_matches('/'),
            self.interface.endpoint()
        );
        let mut request = self
            .client
            .post(&url)
            .json(&self.interface.build_test_completion_body(&self.model_name));
        if self.interface.supports_bearer_auth()
            && let Some(key) = self.api_key.as_deref()
        {
            request = request.header("Authorization", format!("Bearer {key}"));
        }

        match request.send().await {
            Ok(response) if response.status().is_success() => HostCheck::Present,
            Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {
                HostCheck::Missing { closest: None }
            }
            Ok(response) => HostCheck::Failed(ImageAnalysisError::HttpError {
                status: response.status(),
                subject: ErrorSubject::TestCompletion,
                response: response.text().await.unwrap_or_default(),
            }),
            Err(err) => HostCheck::Failed(ImageAnalysisError::HttpClientError {
                asset_id: None,
                error: format_error_chain(&err),
            }),
        }
    }
}
