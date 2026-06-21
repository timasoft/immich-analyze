use crate::{
    error::ImageAnalysisError,
    immich_api::{AssetMetadata, ExifInfo, PersonInfo, TagInfo},
};
use log::{debug, warn};
use serde::Serialize;
use tokio_postgres::Client as PgClient;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct ImageAnalysisResult {
    pub description: String,
    pub asset_id: Uuid,
}

/// Gets the existing description for an asset from database
pub async fn get_asset_description(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<Option<String>, ImageAnalysisError> {
    let query = "
        SELECT description FROM asset_exif
        WHERE \"assetId\" = $1
        AND description IS NOT NULL
        AND description != ''
    ";
    match client.query_opt(query, &[&asset_id]).await {
        Ok(Some(row)) => Ok(row.get::<_, Option<String>>(0)),
        Ok(None) => Ok(None),
        Err(err) => {
            eprintln!(
                "{}",
                rust_i18n::t!(
                    "database.error_checking_description",
                    error = err.to_string()
                )
            );
            Err(ImageAnalysisError::DatabaseError {
                error: err.to_string(),
            })
        }
    }
}

/// Check if asset already has description in database
pub async fn asset_has_description(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<bool, ImageAnalysisError> {
    let query = "
        SELECT EXISTS (
            SELECT 1 FROM asset_exif
            WHERE \"assetId\" = $1
            AND description IS NOT NULL
            AND description != ''
        )
    ";
    match client.query_one(query, &[&asset_id]).await {
        Ok(row) => Ok(row.get(0)),
        Err(err) => {
            eprintln!(
                "{}",
                rust_i18n::t!(
                    "database.error_checking_description",
                    error = err.to_string()
                )
            );
            Err(ImageAnalysisError::DatabaseError {
                error: err.to_string(),
            })
        }
    }
}

/// Update or create asset description in database
pub async fn update_or_create_asset_description(
    client: &PgClient,
    asset_id: Uuid,
    description: &str,
) -> Result<(), ImageAnalysisError> {
    println!(
        "{}",
        rust_i18n::t!("database.updating_asset", asset_id = asset_id)
    );
    let preview: String = description.chars().take(100).collect();
    println!(
        "{}",
        rust_i18n::t!(
            "database.description_length",
            length = description.len().to_string(),
            preview = preview
        )
    );

    let upsert_query = r#"
        INSERT INTO asset_exif (
            "assetId", description, "updatedAt", "updateId"
        ) VALUES (
            $1, $2, NOW(), immich_uuid_v7()
        )
        ON CONFLICT ("assetId") DO UPDATE
        SET description = EXCLUDED.description,
            "updatedAt" = NOW(),
            "updateId" = immich_uuid_v7()
    "#;

    match client
        .execute(upsert_query, &[&asset_id, &description])
        .await
    {
        Ok(_) => {
            println!(
                "{}",
                rust_i18n::t!("database.insert_success", asset_id = asset_id)
            );
            Ok(())
        }
        Err(err) => {
            eprintln!(
                "{}\n{}",
                rust_i18n::t!(
                    "database.insert_error",
                    asset_id = asset_id,
                    error = err.to_string()
                ),
                rust_i18n::t!("database.sql_query_details", query = upsert_query)
            );
            Err(ImageAnalysisError::DatabaseError {
                error: err.to_string(),
            })
        }
    }
}

/// Gets full metadata for an asset from the database for prompt enrichment.
pub async fn get_asset_metadata(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<AssetMetadata, ImageAnalysisError> {
    let query = r#"
        SELECT
            a."originalFileName",
            a."type",
            to_char(a."fileCreatedAt", 'YYYY-MM-DD HH24:MI:SS') AS "fileCreatedAt",
            to_char(a."localDateTime", 'YYYY-MM-DD HH24:MI:SS') AS "localDateTime",
            a.height,
            a.width,
            e.description,
            to_char(e."dateTimeOriginal", 'YYYY-MM-DD HH24:MI:SS') AS "dateTimeOriginal",
            e.city,
            e.state,
            e.country,
            e.make,
            e.model,
            e."lensModel",
            e."exposureTime",
            e."fNumber",
            e."focalLength",
            e.iso,
            e.rating,
            e."timeZone",
            (e."assetId" IS NOT NULL) AS exif_exists
        FROM asset a
        LEFT JOIN asset_exif e ON e."assetId" = a.id
        WHERE a.id = $1
    "#;

    let Some(row) = client.query_opt(query, &[&asset_id]).await.map_err(|err| {
        ImageAnalysisError::DatabaseError {
            error: format!("Failed to query asset metadata: {err}"),
        }
    })?
    else {
        return Err(ImageAnalysisError::DatabaseError {
            error: format!("Asset {asset_id} not found"),
        });
    };

    let original_file_name: Option<String> = row.get("originalFileName");
    let asset_type: Option<String> = row.get("type");
    let file_created_at: Option<String> = row.get("fileCreatedAt");
    let local_date_time: Option<String> = row.get("localDateTime");
    let height: Option<i32> = row.get("height");
    let width: Option<i32> = row.get("width");

    let exif_exists: bool = row.get("exif_exists");
    let exif_info = if exif_exists {
        let description: Option<String> = row.get("description");
        let date_time_original: Option<String> = row.get("dateTimeOriginal");
        let city: Option<String> = row.get("city");
        let state: Option<String> = row.get("state");
        let country: Option<String> = row.get("country");
        let make: Option<String> = row.get("make");
        let model: Option<String> = row.get("model");
        let lens_model: Option<String> = row.get("lensModel");
        let exposure_time: Option<String> = row.get("exposureTime");
        let f_number: Option<f64> = row.get("fNumber");
        let focal_length: Option<f64> = row.get("focalLength");
        let iso_raw: Option<i32> = row.get("iso");
        let rating_raw: Option<i32> = row.get("rating");
        let time_zone: Option<String> = row.get("timeZone");

        let iso = iso_raw.and_then(|value| {
            u32::try_from(value).map_or_else(
                |_| {
                    debug!("Invalid ISO value for asset {asset_id}: {value}");
                    None
                },
                Some,
            )
        });
        let rating = rating_raw.and_then(|value| {
            u8::try_from(value).map_or_else(
                |_| {
                    debug!("Invalid rating value for asset {asset_id}: {value}");
                    None
                },
                Some,
            )
        });

        Some(ExifInfo {
            description,
            city,
            state,
            country,
            make,
            model,
            date_time_original,
            lens_model,
            exposure_time,
            f_number,
            focal_length,
            iso,
            rating,
            time_zone,
        })
    } else {
        None
    };

    // Query people linked to this asset via asset_face
    let people_query = r#"
        SELECT p.name, to_char(p."birthDate", 'YYYY-MM-DD') AS birth_date
        FROM person p
        JOIN asset_face af ON af."personId" = p.id
        WHERE af."assetId" = $1
    "#;

    let people = match client.query(people_query, &[&asset_id]).await {
        Ok(rows) => rows
            .iter()
            .map(|row| {
                let name: String = row.get("name");
                let birth_date: Option<String> = row.get("birth_date");
                PersonInfo { name, birth_date }
            })
            .collect(),
        Err(err) => {
            warn!("Failed to query people for asset {asset_id}: {err}");
            Vec::new()
        }
    };

    // Query tags linked to this asset via tag_asset
    let tag_query = r#"
        SELECT t.value
        FROM tag t
        JOIN tag_asset ta ON ta."tagId" = t.id
        WHERE ta."assetId" = $1
    "#;

    let tags = match client.query(tag_query, &[&asset_id]).await {
        Ok(rows) => rows
            .iter()
            .map(|row| {
                let value: String = row.get("value");
                TagInfo { value }
            })
            .collect(),
        Err(err) => {
            warn!("Failed to query tags for asset {asset_id}: {err}");
            Vec::new()
        }
    };

    Ok(AssetMetadata {
        original_file_name,
        r#type: asset_type,
        file_created_at,
        local_date_time,
        height,
        width,
        // originalMimeType column does not exist in this Immich schema version
        original_mime_type: None,
        people,
        tags,
        exif_info,
    })
}

pub async fn check_database_connection(client: &PgClient) -> Result<bool, ImageAnalysisError> {
    let timeout_duration = std::time::Duration::from_secs(5);
    match tokio::time::timeout(timeout_duration, client.query("SELECT 1", &[])).await {
        Ok(Ok(_)) => {
            println!("{}", rust_i18n::t!("database.connection_success"));
            Ok(true)
        }
        Ok(Err(err)) => {
            eprintln!(
                "{}",
                rust_i18n::t!("error.database_query_failed", error = err.to_string())
            );
            Err(ImageAnalysisError::DatabaseError {
                error: format!(
                    "{}",
                    rust_i18n::t!("error.query_failed_error", error = err.to_string())
                ),
            })
        }
        Err(_) => {
            eprintln!("{}", rust_i18n::t!("error.database_timeout"));
            Err(ImageAnalysisError::DatabaseError {
                error: format!("{}", rust_i18n::t!("error.database_timeout")),
            })
        }
    }
}
