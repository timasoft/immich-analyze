use crate::error::ImageAnalysisError;
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
        Err(e) => {
            eprintln!(
                "{}",
                rust_i18n::t!("database.error_checking_description", error = e.to_string())
            );
            Err(ImageAnalysisError::DatabaseError {
                error: e.to_string(),
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
        Err(e) => {
            eprintln!(
                "{}",
                rust_i18n::t!("database.error_checking_description", error = e.to_string())
            );
            Err(ImageAnalysisError::DatabaseError {
                error: e.to_string(),
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
        Err(e) => {
            eprintln!(
                "{}\n{}",
                rust_i18n::t!(
                    "database.insert_error",
                    asset_id = asset_id,
                    error = e.to_string()
                ),
                rust_i18n::t!("database.sql_query_details", query = upsert_query)
            );
            Err(ImageAnalysisError::DatabaseError {
                error: e.to_string(),
            })
        }
    }
}

pub async fn check_database_connection(client: &PgClient) -> Result<bool, ImageAnalysisError> {
    let timeout_duration = std::time::Duration::from_secs(5);
    match tokio::time::timeout(timeout_duration, client.query("SELECT 1", &[])).await {
        Ok(Ok(_)) => {
            println!("{}", rust_i18n::t!("database.connection_success"));
            Ok(true)
        }
        Ok(Err(e)) => {
            eprintln!(
                "{}",
                rust_i18n::t!("error.database_query_failed", error = e.to_string())
            );
            Err(ImageAnalysisError::DatabaseError {
                error: format!(
                    "{}",
                    rust_i18n::t!("error.query_failed_error", error = e.to_string())
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
