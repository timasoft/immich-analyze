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

// Add logging
use log::{debug, info, warn, error};

#[derive(Deserialize, Debug)]
pub struct LlamaCppResponse {
    pub choices: Vec<Choice>,
}

#[derive(Deserialize, Debug)]
pub struct Choice {
    pub message: Message,
}

#[derive(Deserialize, Debug)]
pub struct Message {
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct LlamaCppHostManager {
    pub hosts: Vec<String>,
    pub api_key: Option<String>,
    unavailable_hosts: Arc<Mutex<HashMap<String, Instant>>>,
    unavailable_duration: Duration,
}

impl LlamaCppHostManager {
    pub fn new(hosts: Vec<String>, api_key: Option<String>, unavailable_duration: Duration) -> Self {
        Self {
            hosts,
            api_key,
            unavailable_hosts: Arc::new(Mutex::new(HashMap::new())),
            unavailable_duration,
        }
    }

    pub async fn get_available_host(&self) -> Result<String, ImageAnalysisError> {
        debug!("Looking for available llamacpp hosts. Total hosts: {}", self.hosts.len());
        
        let mut unavailable = self.unavailable_hosts.lock().unwrap();
        // Clean up expired unavailability records
        let now = Instant::now();
        let original_count = unavailable.len();
        unavailable
            .retain(|_, timestamp| now.duration_since(*timestamp) < self.unavailable_duration);
        
        if unavailable.len() < original_count {
            debug!("Cleaned up {} expired unavailable hosts", original_count - unavailable.len());
        }
        
        debug!("Currently unavailable hosts: {:?}", unavailable.keys().collect::<Vec<_>>());
        
        // Try to find an available host
        for host in &self.hosts {
            if !unavailable.contains_key(host) {
                info!("Selected available llamacpp host: {}", host);
                return Ok(host.clone());
            }
        }
        
        // If all hosts are unavailable, try the one that became unavailable longest ago
        if let Some((host, timestamp)) = unavailable.iter().min_by_key(|(_, timestamp)| *timestamp) {
            warn!("All llamacpp hosts unavailable. Using oldest unavailable host: {} (unavailable for {:?})",
                  host, now.duration_since(*timestamp));
            return Ok(host.clone());
        }
        
        error!("No llamacpp hosts available at all");
        Err(ImageAnalysisError::AllHostsUnavailable)
    }

    pub async fn mark_host_unavailable(&self, host: &str) {
        let mut unavailable = self.unavailable_hosts.lock().unwrap();
        unavailable.insert(host.to_string(), Instant::now());
        println!(
            "{}",
            rust_i18n::t!("llamacpp.host_marked_unavailable", host = host)
        );
    }
}

/// Analyze image using llama.cpp server API with fallback to multiple hosts
pub async fn analyze_image(
    client: &Client,
    image_path: &Path,
    model_name: &str,
    prompt: &str,
    timeout: u64,
    host_manager: &LlamaCppHostManager,
) -> Result<crate::database::ImageAnalysisResult, ImageAnalysisError> {
    let filename = image_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    
    info!("Starting llamacpp analysis for image: {}", filename);
    debug!("Model: {}, Timeout: {}s", model_name, timeout);
    
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
    
    // llama.cpp server expects OpenAI-compatible format
    let request_body = serde_json::json!({
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
    });
    
    let mut last_error = None;
    // Try each available host until we get a successful response
    for _attempt in 0..host_manager.hosts.len() {
        let host = match host_manager.get_available_host().await {
            Ok(host) => host,
            Err(e) => {
                error!("Failed to get available llamacpp host: {:?}", e);
                return Err(e);
            }
        };
        
        // llama.cpp server typically uses /v1/chat/completions endpoint
        let llamacpp_url = format!("{}/v1/chat/completions", host.trim_end_matches('/'));
        info!("Making llamacpp request to: {}", llamacpp_url);
        
        let mut request = client.post(&llamacpp_url).json(&request_body);
        
        // Add Authorization header if API key is provided
        if let Some(ref api_key) = host_manager.api_key {
            debug!("Adding Authorization header with API key: {}...", &api_key[..8.min(api_key.len())]);
            request = request.header("Authorization", format!("Bearer {}", api_key));
        } else {
            debug!("No API key provided for llamacpp request");
        }
        
        match tokio::time::timeout(Duration::from_secs(timeout.saturating_add(1)), async {
            debug!("Sending llamacpp request...");
            request.send().await
        })
        .await
        {
            Ok(Ok(response)) => {
                let status = response.status();
                debug!("Received llamacpp response: {} {}", status.as_u16(), status.canonical_reason().unwrap_or(""));
                
                if response.status().is_success() {
                    let response_text =
                        response
                            .text()
                            .await
                            .map_err(|e| {
                                error!("Failed to read llamacpp response body: {}", e);
                                ImageAnalysisError::ProcessingError {
                                    filename: filename.clone(),
                                    error: e.to_string(),
                                }
                            })?;
                    
                    debug!("llamacpp response body length: {} chars", response_text.len());
                    debug!("llamacpp response body (first 200 chars): {}", &response_text[..200.min(response_text.len())]);
                    
                    match serde_json::from_str::<LlamaCppResponse>(&response_text) {
                        Ok(llamacpp_response) => {
                            debug!("Successfully parsed llamacpp response with {} choices", llamacpp_response.choices.len());
                            if let Some(choice) = llamacpp_response.choices.first() {
                                let description = choice.message.content.trim().to_string();
                                if description.is_empty() {
                                    warn!("llamacpp returned empty content for image: {}", filename);
                                    last_error = Some(ImageAnalysisError::EmptyResponse {
                                        filename: filename.clone(),
                                    });
                                } else {
                                    info!("llamacpp analysis successful for {}, description length: {}", filename, description.len());
                                    return Ok(crate::database::ImageAnalysisResult {
                                        description,
                                        asset_id,
                                    });
                                }
                            } else {
                                warn!("llamacpp response has no choices for image: {}", filename);
                                last_error = Some(ImageAnalysisError::EmptyResponse {
                                    filename: filename.clone(),
                                });
                            }
                        }
                        Err(parse_error) => {
                            warn!("Failed to parse llamacpp response as LlamaCppResponse: {}", parse_error);
                            debug!("Attempting fallback JSON parsing...");
                            
                            // Fallback parsing attempt
                            if let Ok(json_value) = serde_json::from_str::<Value>(&response_text) {
                                debug!("Fallback JSON parsing successful");
                                if let Some(choices) = json_value.get("choices") {
                                    if let Some(first_choice) = choices.get(0) {
                                        if let Some(content) = first_choice
                                            .get("message")
                                            .and_then(|m| m.get("content"))
                                            .and_then(|c| c.as_str())
                                        {
                                            let description = content.trim().to_string();
                                            if !description.is_empty() {
                                                info!("llamacpp analysis successful via fallback parsing for {}, description length: {}", filename, description.len());
                                                return Ok(crate::database::ImageAnalysisResult {
                                                    description,
                                                    asset_id,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                            error!("Failed to parse llamacpp response with both methods for {}: {}", filename, parse_error);
                            last_error = Some(ImageAnalysisError::JsonParsing {
                                filename: filename.clone(),
                                error: parse_error.to_string(),
                            });
                        }
                    }
                } else {
                    let status = response.status().as_u16();
                    let response_text = response.text().await.unwrap_or_default();
                    error!("llamacpp HTTP error {} for {}: {}", status, filename, response_text);
                    last_error = Some(ImageAnalysisError::HttpError {
                        status,
                        filename: filename.clone(),
                        response: response_text,
                    });
                }
            }
            Ok(Err(e)) => {
                error!("llamacpp request failed for {}: {}", filename, e);
                last_error = Some(ImageAnalysisError::HttpError {
                    status: 0,
                    filename: filename.clone(),
                    response: e.to_string(),
                });
            }
            Err(_) => {
                error!("llamacpp request timeout for {} (timeout: {}s)", filename, timeout);
                last_error = Some(ImageAnalysisError::LlamaCppRequestTimeout);
            }
        }
        // Mark current host as unavailable
        warn!("Marking llamacpp host as unavailable due to error: {}", host);
        host_manager.mark_host_unavailable(&host).await;
        // Mark current host as unavailable
        host_manager.mark_host_unavailable(&host).await;
    }
    Err(last_error.unwrap_or(ImageAnalysisError::AllHostsUnavailable))
}