use crate::error::ImageAnalysisError;
use reqwest::{
    Client,
    header::{HeaderMap, HeaderValue},
};
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;
use url::Url;
use uuid::Uuid;

/// Reference to an asset with minimal metadata needed for processing.
#[derive(Debug, Clone)]
pub struct AssetRef {
    /// Unique identifier of the asset (UUID)
    pub id: Uuid,
}

/// Internal response structure for asset metadata.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetResponse {
    pub id: String,
    #[serde(default)]
    pub exif_info: Option<ExifInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExifInfo {
    pub description: Option<String>,
}

/// Response wrapper for paginated search results.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetSearchResponse {
    assets: AssetSearchResult,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetSearchResult {
    items: Vec<AssetResponse>,
    #[serde(default)]
    next_page: Option<String>,
}

/// Provider for accessing Immich data via the REST API.
/// Supports multiple API keys for multi-user setups.
#[derive(Clone)]
pub struct ImmichApiProvider {
    /// HTTP clients with authentication headers (one per API key)
    clients: Vec<Client>,
    /// Base URL of the Immich server (e.g., "https://immich.example.com")
    base_url: Url,
}

impl std::fmt::Debug for ImmichApiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImmichApiProvider")
            .field("base_url", &self.base_url)
            .field("clients", &format!("{} clients", self.clients.len()))
            .finish()
    }
}

impl ImmichApiProvider {
    /// Creates a new Immich API provider.
    ///
    /// # Arguments
    /// * `base_url` - Base URL of the Immich server
    /// * `api_keys` - List of API keys for authentication (created in Immich web UI)
    ///
    /// # Errors
    /// Returns an error if the URL is invalid or any API key contains invalid characters.
    pub fn new(base_url: &str, api_keys: &[String]) -> Result<Self, ImageAnalysisError> {
        let base_url = Url::parse(base_url).map_err(|e| ImageAnalysisError::InvalidConfig {
            error: e.to_string(),
        })?;

        let clients: Vec<Client> = api_keys
            .iter()
            .map(|api_key| {
                let mut headers = HeaderMap::new();
                let header_value = HeaderValue::from_str(api_key)
                    .map_err(|_| ImageAnalysisError::InvalidApiKey)?;
                headers.insert("x-api-key", header_value);

                Client::builder()
                    .default_headers(headers)
                    .timeout(Duration::from_secs(30))
                    .build()
                    .map_err(|e| ImageAnalysisError::HttpClientError {
                        error: e.to_string(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        if clients.is_empty() {
            return Err(ImageAnalysisError::InvalidConfig {
                error: "At least one API key is required".to_string(),
            });
        }

        Ok(Self { clients, base_url })
    }

    /// Fetches all assets from the Immich library.
    ///
    /// Fully paginates each API key separately (multi-user support).
    /// Tries all keys for each page request on failure.
    ///
    /// # Returns
    /// Vec<AssetRef> containing all assets with their ID and original path.
    pub async fn get_assets(&self) -> Result<Vec<AssetRef>, ImageAnalysisError> {
        const PAGE_SIZE: usize = 1000;
        let mut all_assets = Vec::new();

        let search_url = self.base_url.join("/api/search/metadata").map_err(|e| {
            ImageAnalysisError::InvalidConfig {
                error: e.to_string(),
            }
        })?;

        for client in &self.clients {
            let mut page: usize = 1;
            loop {
                let body = serde_json::json!({
                    "page": page,
                    "size": PAGE_SIZE,
                    "withExif": true
                });

                let response = client
                    .post(search_url.clone())
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| ImageAnalysisError::HttpError {
                        status: 0,
                        filename: "assets_list".to_string(),
                        response: e.to_string(),
                    })?;

                if !response.status().is_success() {
                    let status = response.status().as_u16();
                    let body = response.text().await.unwrap_or_default();
                    return Err(ImageAnalysisError::HttpError {
                        status,
                        filename: "assets_list".to_string(),
                        response: body,
                    });
                }

                let search_result: AssetSearchResponse =
                    response
                        .json()
                        .await
                        .map_err(|e| ImageAnalysisError::JsonParsing {
                            filename: "assets_list".to_string(),
                            error: e.to_string(),
                        })?;

                if search_result.assets.items.is_empty() {
                    break;
                }

                for item in search_result.assets.items {
                    let asset_id =
                        Uuid::parse_str(&item.id).map_err(|_| ImageAnalysisError::InvalidUuid {
                            filename: item.id.clone(),
                        })?;

                    all_assets.push(AssetRef { id: asset_id });
                }

                if search_result.assets.next_page.is_none() {
                    break;
                }
                page += 1;
            }
        }

        Ok(all_assets)
    }

    /// Fetches assets created after a specific timestamp from the Immich library.
    ///
    /// Uses the `createdAfter` filter to retrieve only assets added to Immich
    /// after the specified date. This is useful for incremental polling in monitor mode.
    /// Fully paginates each API key separately (multi-user support).
    ///
    /// # Arguments
    /// * `since` - ISO 8601 formatted datetime string (e.g., "2024-01-15T10:30:00.000Z")
    ///   representing the cutoff timestamp. Assets created after this time
    ///   will be included in the results.
    ///
    /// # Returns
    /// Vec<AssetRef> containing assets created after the specified timestamp.
    ///
    /// # Example
    /// ```rust
    /// let since = "2024-01-01T00:00:00.000Z";
    /// let assets = provider.get_assets_since_timestamp(since).await?;
    /// ```
    pub async fn get_assets_since_timestamp(
        &self,
        since: impl Into<String>,
    ) -> Result<Vec<AssetRef>, ImageAnalysisError> {
        const PAGE_SIZE: usize = 1000;
        let mut all_assets = Vec::new();
        let since_str = since.into();

        let search_url = self.base_url.join("/api/search/metadata").map_err(|e| {
            ImageAnalysisError::InvalidConfig {
                error: e.to_string(),
            }
        })?;

        for client in &self.clients {
            let mut page: usize = 1;
            loop {
                let body = serde_json::json!({
                    "page": page,
                    "size": PAGE_SIZE,
                    "withExif": true,
                    "createdAfter": since_str
                });

                let response = client
                    .post(search_url.clone())
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| ImageAnalysisError::HttpError {
                        status: 0,
                        filename: "assets_list".to_string(),
                        response: e.to_string(),
                    })?;

                if !response.status().is_success() {
                    let status = response.status().as_u16();
                    let body = response.text().await.unwrap_or_default();
                    return Err(ImageAnalysisError::HttpError {
                        status,
                        filename: "assets_list".to_string(),
                        response: body,
                    });
                }

                let search_result: AssetSearchResponse =
                    response
                        .json()
                        .await
                        .map_err(|e| ImageAnalysisError::JsonParsing {
                            filename: "assets_list".to_string(),
                            error: e.to_string(),
                        })?;

                if search_result.assets.items.is_empty() {
                    break;
                }

                for item in search_result.assets.items {
                    let asset_id =
                        Uuid::parse_str(&item.id).map_err(|_| ImageAnalysisError::InvalidUuid {
                            filename: item.id.clone(),
                        })?;

                    all_assets.push(AssetRef { id: asset_id });
                }

                if search_result.assets.next_page.is_none() {
                    break;
                }
                page += 1;
            }
        }

        Ok(all_assets)
    }

    /// Gets the filesystem path to the preview image for an asset.
    ///
    /// For API mode, this downloads the preview to a temporary file and returns its path.
    /// The caller is responsible for cleaning up the temporary file after use.
    /// Tries all API keys until one succeeds.
    ///
    /// # Arguments
    /// * `asset_id` - UUID of the asset
    ///
    /// # Returns
    /// PathBuf to the preview image file (either existing file or downloaded temp file)
    pub async fn get_preview_path(&self, asset_id: &Uuid) -> Result<PathBuf, ImageAnalysisError> {
        let url = self
            .base_url
            .join(&format!("/api/assets/{}/thumbnail?size=preview", asset_id))
            .map_err(|e| ImageAnalysisError::InvalidConfig {
                error: e.to_string(),
            })?;

        let mut last_error = None;
        for client in &self.clients {
            let response = client.get(url.clone()).send().await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let bytes = resp
                        .bytes()
                        .await
                        .map_err(|e| ImageAnalysisError::HttpError {
                            status: 0,
                            filename: asset_id.to_string(),
                            response: e.to_string(),
                        })?;

                    let temp_path = std::env::temp_dir().join(format!("{}_preview.tmp", asset_id));
                    tokio::fs::write(&temp_path, &bytes).await.map_err(|e| {
                        ImageAnalysisError::ProcessingError {
                            filename: asset_id.to_string(),
                            error: e.to_string(),
                        }
                    })?;

                    return Ok(temp_path);
                }
                Ok(resp) => {
                    last_error = Some(ImageAnalysisError::HttpError {
                        status: resp.status().as_u16(),
                        filename: asset_id.to_string(),
                        response: resp.text().await.unwrap_or_default(),
                    });
                }
                Err(e) => {
                    last_error = Some(ImageAnalysisError::HttpError {
                        status: 0,
                        filename: asset_id.to_string(),
                        response: e.to_string(),
                    });
                }
            }
        }

        Err(last_error.unwrap_or_else(|| ImageAnalysisError::HttpError {
            status: 0,
            filename: asset_id.to_string(),
            response: "No API keys available".to_string(),
        }))
    }

    /// Updates the description for an asset.
    /// Tries all API keys until one succeeds.
    ///
    /// # Arguments
    /// * `asset_id` - UUID of the asset
    /// * `description` - New description text
    pub async fn update_description(
        &self,
        asset_id: &Uuid,
        description: &str,
    ) -> Result<(), ImageAnalysisError> {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct UpdateRequest<'a> {
            description: &'a str,
        }

        let url = self
            .base_url
            .join(&format!("/api/assets/{}", asset_id))
            .map_err(|e| ImageAnalysisError::InvalidConfig {
                error: e.to_string(),
            })?;

        let body = UpdateRequest { description };

        let mut last_error = None;
        for client in &self.clients {
            let response = client.put(url.clone()).json(&body).send().await;

            match response {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                Ok(resp) => {
                    last_error = Some(ImageAnalysisError::HttpError {
                        status: resp.status().as_u16(),
                        filename: asset_id.to_string(),
                        response: resp.text().await.unwrap_or_default(),
                    });
                }
                Err(e) => {
                    last_error = Some(ImageAnalysisError::HttpError {
                        status: 0,
                        filename: asset_id.to_string(),
                        response: e.to_string(),
                    });
                }
            }
        }

        Err(last_error.unwrap_or_else(|| ImageAnalysisError::HttpError {
            status: 0,
            filename: asset_id.to_string(),
            response: "No API keys available".to_string(),
        }))
    }

    /// Checks if an asset already has a description via API.
    /// Tries all API keys until one succeeds.
    ///
    /// # Arguments
    /// * `asset_id` - UUID of the asset
    ///
    /// # Returns
    /// `true` if description exists and is non-empty, `false` otherwise.
    pub async fn has_description(&self, asset_id: &Uuid) -> Result<bool, ImageAnalysisError> {
        let url = self
            .base_url
            .join(&format!("/api/assets/{}", asset_id))
            .map_err(|e| ImageAnalysisError::InvalidConfig {
                error: e.to_string(),
            })?;

        let mut last_error = None;
        for client in &self.clients {
            let response = client.get(url.clone()).send().await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let asset: AssetResponse =
                        resp.json()
                            .await
                            .map_err(|e| ImageAnalysisError::JsonParsing {
                                filename: asset_id.to_string(),
                                error: e.to_string(),
                            })?;

                    return Ok(asset
                        .exif_info
                        .as_ref()
                        .and_then(|e| e.description.as_ref())
                        .is_some_and(|d| !d.is_empty()));
                }
                Ok(resp) => {
                    last_error = Some(ImageAnalysisError::HttpError {
                        status: resp.status().as_u16(),
                        filename: asset_id.to_string(),
                        response: resp.text().await.unwrap_or_default(),
                    });
                }
                Err(e) => {
                    last_error = Some(ImageAnalysisError::HttpError {
                        status: 0,
                        filename: asset_id.to_string(),
                        response: e.to_string(),
                    });
                }
            }
        }

        Err(last_error.unwrap_or_else(|| ImageAnalysisError::HttpError {
            status: 0,
            filename: asset_id.to_string(),
            response: "No API keys available".to_string(),
        }))
    }
}
