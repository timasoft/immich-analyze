use crate::{error::ImageAnalysisError, utils::extract_uuid_from_preview_filename};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Deserialize, Debug)]
pub struct ChatResponse {
    pub message: Message,
}

#[derive(Deserialize, Debug)]
pub struct Message {
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct OllamaHostManager {
    pub hosts: Vec<String>,
    unavailable_hosts: Arc<Mutex<HashMap<String, Instant>>>,
    unavailable_duration: Duration,
}

impl OllamaHostManager {
    pub fn new(hosts: Vec<String>, unavailable_duration: Duration) -> Self {
        Self {
            hosts,
            unavailable_hosts: Arc::new(Mutex::new(HashMap::new())),
            unavailable_duration,
        }
    }

    pub async fn get_available_host(&self) -> Result<String, ImageAnalysisError> {
        let mut unavailable = self.unavailable_hosts.lock().unwrap();
        // Clean up expired unavailability records
        let now = Instant::now();
        unavailable
            .retain(|_, timestamp| now.duration_since(*timestamp) < self.unavailable_duration);
        // Try to find an available host
        for host in &self.hosts {
            if !unavailable.contains_key(host) {
                return Ok(host.clone());
            }
        }
        // If all hosts are unavailable, try the one that became unavailable longest ago
        if let Some((host, _)) = unavailable.iter().min_by_key(|(_, timestamp)| *timestamp) {
            return Ok(host.clone());
        }
        Err(ImageAnalysisError::AllHostsUnavailable)
    }

    pub async fn mark_host_unavailable(&self, host: &str) {
        let mut unavailable = self.unavailable_hosts.lock().unwrap();
        unavailable.insert(host.to_string(), Instant::now());
        println!(
            "{}",
            rust_i18n::t!("ollama.host_marked_unavailable", host = host)
        );
    }
}

/// Analyze image using Ollama API with fallback to multiple hosts
pub async fn analyze_image(
    client: &Client,
    image_path: &Path,
    model_name: &str,
    prompt: &str,
    timeout: u64,
    host_manager: &OllamaHostManager,
) -> Result<crate::database::ImageAnalysisResult, ImageAnalysisError> {
    let filename = image_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    let asset_id = extract_uuid_from_preview_filename(&filename)?;
    let metadata =
        std::fs::metadata(image_path).map_err(|e| ImageAnalysisError::ProcessingError {
            filename: filename.clone(),
            error: e.to_string(),
        })?;
    if metadata.len() == 0 {
        return Err(ImageAnalysisError::EmptyFile { filename });
    }
    let mut image_file =
        std::fs::File::open(image_path).map_err(|e| ImageAnalysisError::ProcessingError {
            filename: filename.clone(),
            error: e.to_string(),
        })?;
    let mut image_data = Vec::new();
    image_file
        .read_to_end(&mut image_data)
        .map_err(|e| ImageAnalysisError::ProcessingError {
            filename: filename.clone(),
            error: e.to_string(),
        })?;
    let base64_image = STANDARD.encode(&image_data);
    let request_body = serde_json::json!({
        "model": model_name,
        "messages": [
            {
                "role": "user",
                "content": prompt,
                "images": [base64_image]
            }
        ],
        "stream": false,
    });
    let mut last_error = None;
    // Try each available host until we get a successful response
    for _attempt in 0..host_manager.hosts.len() {
        let host = match host_manager.get_available_host().await {
            Ok(host) => host,
            Err(e) => return Err(e),
        };
        let ollama_url = format!("{}/api/chat", host.trim_end_matches('/'));
        match tokio::time::timeout(Duration::from_secs(timeout.saturating_add(1)), async {
            client.post(&ollama_url).json(&request_body).send().await
        })
        .await
        {
            Ok(Ok(response)) => {
                if response.status().is_success() {
                    let response_text =
                        response
                            .text()
                            .await
                            .map_err(|e| ImageAnalysisError::ProcessingError {
                                filename: filename.clone(),
                                error: e.to_string(),
                            })?;
                    match serde_json::from_str::<ChatResponse>(&response_text) {
                        Ok(chat_response) => {
                            let description = chat_response.message.content.trim().to_string();
                            if description.is_empty() {
                                last_error = Some(ImageAnalysisError::EmptyResponse {
                                    filename: filename.clone(),
                                });
                            } else {
                                return Ok(crate::database::ImageAnalysisResult {
                                    description,
                                    asset_id,
                                });
                            }
                        }
                        Err(parse_error) => {
                            // Fallback parsing attempt
                            if let Ok(json_value) = serde_json::from_str::<Value>(&response_text) {
                                if let Some(content) = json_value
                                    .get("message")
                                    .and_then(|m| m.get("content"))
                                    .and_then(|c| c.as_str())
                                {
                                    let description = content.trim().to_string();
                                    if !description.is_empty() {
                                        return Ok(crate::database::ImageAnalysisResult {
                                            description,
                                            asset_id,
                                        });
                                    }
                                }
                            }
                            last_error = Some(ImageAnalysisError::JsonParsing {
                                filename: filename.clone(),
                                error: parse_error.to_string(),
                            });
                        }
                    }
                } else {
                    let status = response.status().as_u16();
                    let response_text = response.text().await.unwrap_or_default();
                    last_error = Some(ImageAnalysisError::HttpError {
                        status,
                        filename: filename.clone(),
                        response: response_text,
                    });
                }
            }
            Ok(Err(e)) => {
                last_error = Some(ImageAnalysisError::HttpError {
                    status: 0,
                    filename: filename.clone(),
                    response: e.to_string(),
                });
            }
            Err(_) => {
                last_error = Some(ImageAnalysisError::OllamaRequestTimeout);
            }
        }
        // Mark current host as unavailable
        host_manager.mark_host_unavailable(&host).await;
    }
    Err(last_error.unwrap_or(ImageAnalysisError::AllHostsUnavailable))
}
