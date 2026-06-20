use crate::{
    args::Interface,
    error::ImageAnalysisError,
    utils::{extract_uuid_from_preview_filename, filename_from_path, read_image_as_base64},
};
use log::{debug, error, info, warn};
use reqwest::Client;
use serde_json::Value;
use std::{
    collections::HashMap,
    num::NonZeroU32,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

impl Interface {
    /// Returns the API endpoint path for the given interface.
    #[inline]
    pub const fn endpoint(self) -> &'static str {
        match self {
            Self::Ollama => "/api/chat",
            Self::Llamacpp => "/v1/chat/completions",
        }
    }

    /// Returns `true` if the interface supports Bearer token authentication.
    #[inline]
    pub const fn supports_bearer_auth(self) -> bool {
        match self {
            Self::Ollama => false,
            Self::Llamacpp => true,
        }
    }

    /// Parses the response JSON and extracts the content string for the given interface.
    pub fn parse_response(self, json_value: &Value) -> Option<&str> {
        match self {
            Self::Ollama => json_value
                .get("message")
                .and_then(|msg| msg.get("content"))
                .and_then(|content| content.as_str()),
            Self::Llamacpp => json_value
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
            Self::Llamacpp => serde_json::json!({
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
                                    "url": format!("data:image/jpeg;base64,{}", base64_image)
                                }
                            }
                        ]
                    }
                ],
                "stream": false,
            }),
        }
    }
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
    unavailable_duration: Duration,
    api_key: Option<String>,
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
            unavailable_duration,
            api_key,
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
        let now = Instant::now();
        let original_count = unavailable.len();
        unavailable
            .retain(|_, timestamp| now.duration_since(*timestamp) < self.unavailable_duration);

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
            if !unavailable.contains_key(host) {
                info!("Selected available {:?} host: {}", self.interface, host);
                return Ok(host.clone());
            }
        }

        if let Some((host, timestamp)) = unavailable.iter().min_by_key(|(_, timestamp)| *timestamp)
        {
            warn!(
                "All {:?} hosts unavailable. Using oldest unavailable host: {} (unavailable for {:?})",
                self.interface,
                host,
                now.duration_since(*timestamp)
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

    pub async fn analyze_image(
        &self,
        image_path: &Path,
        prompt: &str,
    ) -> Result<crate::database::ImageAnalysisResult, ImageAnalysisError> {
        let filename = filename_from_path(image_path);

        info!(
            "Starting {:?} analysis for image: {}",
            self.interface, filename
        );
        debug!("Model: {}, Timeout: {}s", self.model_name, self.timeout);

        let asset_id = extract_uuid_from_preview_filename(&filename)?;
        let base64_image = read_image_as_base64(image_path, &filename).await?;

        let request_body =
            self.interface
                .build_request_body(&self.model_name, prompt, &base64_image);

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
                    filename
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
                            status.as_u16(),
                            status.canonical_reason().unwrap_or("")
                        );

                        if response.status().is_success() {
                            let response_text = response.text().await.map_err(|err| {
                                error!("Failed to read response body: {err}");
                                ImageAnalysisError::ProcessingError {
                                    filename: filename.clone(),
                                    error: err.to_string(),
                                }
                            })?;

                            debug!("Response body length: {} chars", response_text.len());

                            match serde_json::from_str::<Value>(&response_text) {
                                Ok(json_value) => {
                                    let content = self.interface.parse_response(&json_value);

                                    if let Some(raw_description) = content {
                                        let description = raw_description.trim().to_owned();
                                        if description.is_empty() {
                                            warn!("Empty response for image: {filename}");
                                            last_error = Some(ImageAnalysisError::EmptyResponse {
                                                filename: filename.clone(),
                                            });
                                        } else {
                                            info!(
                                                "{:?} analysis successful for {}, description length: {}",
                                                self.interface,
                                                filename,
                                                description.len()
                                            );
                                            return Ok(crate::database::ImageAnalysisResult {
                                                description,
                                                asset_id,
                                            });
                                        }
                                    } else {
                                        error!(
                                            "Failed to extract content from response for {filename}"
                                        );
                                        last_error = Some(ImageAnalysisError::JsonParsing {
                                            filename: filename.clone(),
                                            error: "No content field found in response".to_owned(),
                                        });
                                    }
                                }
                                Err(parse_error) => {
                                    error!(
                                        "Failed to parse response as JSON for {filename}: {parse_error}"
                                    );
                                    let error = ImageAnalysisError::JsonParsing {
                                        filename: filename.clone(),
                                        error: parse_error.to_string(),
                                    };
                                    if !error.is_retryable() {
                                        return Err(error);
                                    }
                                    last_error = Some(error);
                                }
                            }
                        } else {
                            let status = response.status().as_u16();
                            let response_text = response.text().await.unwrap_or_default();
                            error!(
                                "{:?} HTTP error {} for {}: {}",
                                self.interface, status, filename, response_text
                            );
                            let error = ImageAnalysisError::HttpError {
                                status,
                                filename: filename.clone(),
                                response: response_text,
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
                            self.interface, filename, err
                        );
                        last_error = Some(ImageAnalysisError::HttpError {
                            status: 0,
                            filename: filename.clone(),
                            response: err.to_string(),
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
                    filename,
                    self.retry_delay.as_secs()
                );
                tokio::time::sleep(self.retry_delay).await;
            } else {
                break;
            }
        }
        Err(last_error.unwrap_or(ImageAnalysisError::AllHostsUnavailable))
    }
}
