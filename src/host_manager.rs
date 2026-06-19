use crate::{
    args::Interface,
    error::ImageAnalysisError,
    utils::{extract_uuid_from_preview_filename, read_image_as_base64},
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
    pub fn endpoint(&self) -> &'static str {
        match self {
            Self::Ollama => "/api/chat",
            Self::Llamacpp => "/v1/chat/completions",
        }
    }

    /// Returns `true` if the interface supports Bearer token authentication.
    #[inline]
    pub fn supports_bearer_auth(&self) -> bool {
        match self {
            Self::Ollama => false,
            Self::Llamacpp => true,
        }
    }

    /// Parses the response JSON and extracts the content string for the given interface.
    pub fn parse_response<'a>(&self, json_value: &'a Value) -> Option<&'a str> {
        match self {
            Self::Ollama => json_value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str()),
            Self::Llamacpp => json_value
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str()),
        }
    }

    /// Builds the JSON request body specific to the AI service interface.
    pub fn build_request_body(&self, model_name: &str, prompt: &str, base64_image: &str) -> Value {
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
    #[allow(clippy::too_many_arguments)]
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

    pub async fn get_available_host(&self) -> Result<String, ImageAnalysisError> {
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

        if unavailable.len() < original_count {
            debug!(
                "Cleaned up {} expired unavailable hosts",
                original_count - unavailable.len()
            );
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

        error!("No {:?} hosts available at all", self.interface);
        Err(ImageAnalysisError::AllHostsUnavailable)
    }

    pub async fn mark_host_unavailable(&self, host: &str) {
        let mut unavailable = self
            .unavailable_hosts
            .lock()
            .expect("unavailable_hosts mutex poisoned");
        unavailable.insert(host.to_string(), Instant::now());
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
        let filename = image_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        info!(
            "Starting {:?} analysis for image: {}",
            self.interface, filename
        );
        debug!("Model: {}, Timeout: {}s", self.model_name, self.timeout);

        let asset_id = extract_uuid_from_preview_filename(&filename)?;
        let base64_image = read_image_as_base64(image_path, &filename)?;

        let request_body =
            self.interface
                .build_request_body(&self.model_name, prompt, &base64_image);

        let endpoint = self.interface.endpoint();

        let mut attempt: u32 = 0;
        let mut last_error = None;
        loop {
            attempt += 1;

            if self.max_retries.is_some() || attempt > 1 {
                info!(
                    "Retry attempt {}/{} for image {}",
                    attempt,
                    self.max_retries.map_or("∞".to_string(), |m| m.to_string()),
                    filename
                );
            }

            // Try each available host until we get a successful response
            for _ in 0..self.hosts.len() {
                let host = match self.get_available_host().await {
                    Ok(host) => host,
                    Err(e) => {
                        error!("Failed to get available {:?} host: {:?}", self.interface, e);
                        return Err(e);
                    }
                };

                let url = format!("{}{}", host.trim_end_matches('/'), endpoint);
                info!("Making {:?} request to: {}", self.interface, url);

                let mut request = self.client.post(&url).json(&request_body);

                if self.interface.supports_bearer_auth() {
                    if let Some(ref api_key) = self.api_key {
                        debug!("Adding Authorization header with API key");
                        request = request.header("Authorization", format!("Bearer {}", api_key));
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
                            let response_text = response.text().await.map_err(|e| {
                                error!("Failed to read response body: {}", e);
                                ImageAnalysisError::ProcessingError {
                                    filename: filename.clone(),
                                    error: e.to_string(),
                                }
                            })?;

                            debug!("Response body length: {} chars", response_text.len());

                            match serde_json::from_str::<Value>(&response_text) {
                                Ok(json_value) => {
                                    let content = self.interface.parse_response(&json_value);

                                    if let Some(description) = content {
                                        let description = description.trim().to_string();
                                        if description.is_empty() {
                                            warn!("Empty response for image: {}", filename);
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
                                            "Failed to extract content from response for {}",
                                            filename
                                        );
                                        last_error = Some(ImageAnalysisError::JsonParsing {
                                            filename: filename.clone(),
                                            error: "No content field found in response".to_string(),
                                        });
                                    }
                                }
                                Err(parse_error) => {
                                    error!(
                                        "Failed to parse response as JSON for {}: {}",
                                        filename, parse_error
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
                    Ok(Err(e)) => {
                        error!(
                            "{:?} request failed for {}: {}",
                            self.interface, filename, e
                        );
                        last_error = Some(ImageAnalysisError::HttpError {
                            status: 0,
                            filename: filename.clone(),
                            response: e.to_string(),
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
                self.mark_host_unavailable(&host).await;
            }

            if let Some(ref e) = last_error
                && !e.is_retryable()
            {
                return Err(e.clone());
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
