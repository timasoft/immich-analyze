use crate::error::ImageAnalysisError;
use crate::immich_api::{AssetRef, ImmichApiProvider};
use crate::utils::extract_uuid_from_preview_filename;
use clap::ValueEnum;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_postgres::Client as PgClient;
use uuid::Uuid;

/// Mode of data access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DataAccessMode {
    /// Direct PostgreSQL database access (default, preferred)
    Database,
    /// Immich REST API access (alternative when DB access unavailable)
    ImmichApi,
}

/// Unified data access handle using enum dispatch.
///
/// This enum holds either a database connection or an API provider,
/// and dispatches method calls based on the active variant.
#[derive(Clone)]
pub enum DataAccess {
    /// Database-backed access using existing PostgreSQL functions
    Database {
        /// PostgreSQL client for direct database queries
        client: Arc<PgClient>,
        /// Root path to Immich data directory (for filesystem access to thumbs/)
        immich_root: PathBuf,
    },
    /// API-backed access using Immich REST API
    ImmichApi {
        /// Immich API provider for HTTP-based operations
        provider: Arc<ImmichApiProvider>,
    },
}

impl DataAccess {
    /// Creates a new database-backed data access handle.
    ///
    /// # Arguments
    /// * `client` - Arc-wrapped PostgreSQL client
    /// * `immich_root` - Path to Immich root directory (containing thumbs/)
    pub fn new_database(client: Arc<PgClient>, immich_root: PathBuf) -> Self {
        Self::Database {
            client,
            immich_root,
        }
    }

    /// Creates a new API-backed data access handle.
    ///
    /// # Arguments
    /// * `provider` - Arc-wrapped Immich API provider
    pub fn new_api(provider: Arc<ImmichApiProvider>) -> Self {
        Self::ImmichApi { provider }
    }

    /// Gets a list of assets that need processing (no description yet).
    ///
    /// # Database mode
    /// Uses `crate::file_processing::get_immich_preview_files` to scan the filesystem,
    ///
    /// # API mode
    /// Fetches from Immich API `/api/search/metadata` endpoint, returning all assets.
    ///
    /// # Returns
    /// Vector of AssetRef structs for assets awaiting description generation.
    pub async fn get_assets_to_process(&self) -> Result<Vec<AssetRef>, ImageAnalysisError> {
        match self {
            Self::Database {
                client: _,
                immich_root,
            } => {
                let preview_files = crate::file_processing::get_immich_preview_files(immich_root)?;

                let mut assets = Vec::new();
                for file_path in preview_files {
                    let filename = file_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");

                    match extract_uuid_from_preview_filename(filename) {
                        Ok(asset_id) => {
                            assets.push(AssetRef { id: asset_id });
                        }
                        Err(_) => {
                            continue;
                        }
                    }
                }
                Ok(assets)
            }
            Self::ImmichApi { provider } => provider.get_assets().await,
        }
    }

    /// Gets the filesystem path to the preview image for an asset.
    ///
    /// # Database mode
    /// Scans the `thumbs/` directory tree under `immich_root` to locate
    /// the preview file matching the asset UUID, then returns its path.
    ///
    /// # API mode
    /// Downloads from Immich API `/api/assets/{id}/thumbnail?size=preview` endpoint
    /// to a temporary file and returns the temp file path.
    /// Caller is responsible for cleaning up the temporary file.
    ///
    /// # Arguments
    /// * `asset_id` - UUID of the target asset
    ///
    /// # Returns
    /// PathBuf to the preview image file suitable for AI analysis.
    pub async fn get_preview_path(&self, asset_id: &Uuid) -> Result<PathBuf, ImageAnalysisError> {
        match self {
            Self::Database { immich_root, .. } => {
                Self::find_preview_file_in_thumbs(immich_root, asset_id)
            }
            Self::ImmichApi { provider } => provider.get_preview_path(asset_id).await,
        }
    }

    /// Helper: find preview file in thumbs directory tree for database mode.
    fn find_preview_file_in_thumbs(
        immich_root: &Path,
        asset_id: &Uuid,
    ) -> Result<PathBuf, ImageAnalysisError> {
        let thumbs_dir = immich_root.join("thumbs");
        let mut stack = vec![thumbs_dir];

        while let Some(current_dir) = stack.pop() {
            match std::fs::read_dir(&current_dir) {
                Ok(entries) => {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if path.is_dir() {
                            stack.push(path);
                        } else if path.is_file() {
                            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                            if !(filename.contains("_preview.") || filename.contains("-preview.")) {
                                continue;
                            }

                            if let Ok(found_id) = extract_uuid_from_preview_filename(filename)
                                && found_id == *asset_id
                            {
                                return Ok(path);
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("Error reading directory {}: {}", current_dir.display(), e);
                }
            }
        }

        Err(ImageAnalysisError::ProcessingError {
            filename: asset_id.to_string(),
            error: "Preview file not found in thumbs directory".to_string(),
        })
    }

    /// Updates or creates a description for an asset.
    ///
    /// # Database mode
    /// Uses existing `crate::database::update_or_create_asset_description` function
    /// to upsert the description into the `asset_exif` table.
    ///
    /// # API mode
    /// Sends PUT request to Immich API `/api/assets/{id}` with description payload.
    ///
    /// # Arguments
    /// * `asset_id` - UUID of the target asset
    /// * `description` - Generated description text to store
    pub async fn update_description(
        &self,
        asset_id: &Uuid,
        description: &str,
    ) -> Result<(), ImageAnalysisError> {
        match self {
            Self::Database { client, .. } => {
                crate::database::update_or_create_asset_description(client, *asset_id, description)
                    .await
            }
            Self::ImmichApi { provider } => {
                provider.update_description(asset_id, description).await
            }
        }
    }

    /// Checks if an asset already has a description.
    ///
    /// # Database mode
    /// Queries `asset_exif` table using existing `crate::database::asset_has_description`.
    ///
    /// # API mode
    /// Fetches asset metadata via API and checks exif_info.description field.
    ///
    /// # Arguments
    /// * `asset_id` - UUID of the target asset
    ///
    /// # Returns
    /// `true` if description exists and is non-empty, `false` otherwise.
    pub async fn has_description(&self, asset_id: &Uuid) -> Result<bool, ImageAnalysisError> {
        match self {
            Self::Database { client, .. } => {
                crate::database::asset_has_description(client, *asset_id).await
            }
            Self::ImmichApi { provider } => provider.has_description(asset_id).await,
        }
    }

    pub async fn cleanup_preview(&self, path: &PathBuf) -> Result<(), ImageAnalysisError> {
        if matches!(self, Self::ImmichApi { .. }) {
            tokio::fs::remove_file(path).await.ok();
        }
        Ok(())
    }
}
