use crate::{
    args::OverwritePolicy, data_access::DataAccess, database::ImageAnalysisResult,
    error::ImageAnalysisError,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use log::warn;
use regex::Regex;
use std::{borrow::Cow, path::Path, str::FromStr as _, sync::OnceLock};
use tokio::io::AsyncReadExt as _;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverwriteDecision {
    Skip,
    AnalyzeFresh,
    PreserveExisting(String),
}

/// Get system locale from environment variables
pub fn get_system_locale() -> String {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .map_or_else(
            |_| "en".to_owned(),
            |locale_str| {
                locale_str
                    .split('.')
                    .next()
                    .unwrap_or("en")
                    .split('_')
                    .next()
                    .unwrap_or("en")
                    .to_lowercase()
            },
        )
}

static PREVIEW_PATTERN: OnceLock<Regex> = OnceLock::new();

static UUID_PATTERN: OnceLock<Regex> = OnceLock::new();

static AI_BLOCK_PATTERN: OnceLock<Regex> = OnceLock::new();

pub fn get_ai_block_pattern() -> &'static Regex {
    AI_BLOCK_PATTERN
        .get_or_init(|| Regex::new(r"(?s)\[AI\].*?\[/AI\]").expect("Invalid AI block regex"))
}

pub fn extract_uuid_from_preview_filename(filename: &str) -> Result<Uuid, ImageAnalysisError> {
    let preview_pattern = PREVIEW_PATTERN.get_or_init(|| {
        Regex::new("([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})[-_]preview")
            .expect("Invalid preview filename regex")
    });
    let uuid_pattern = UUID_PATTERN.get_or_init(|| {
        Regex::new("([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})")
            .expect("Invalid uuid regex")
    });
    if let Some(captures) = preview_pattern.captures(filename)
        && let Some(uuid_str) = captures.get(1)
    {
        return Uuid::from_str(uuid_str.as_str()).map_err(|_| ImageAnalysisError::InvalidUuid {
            filename: filename.to_owned(),
        });
    }
    if let Some(captures) = uuid_pattern.captures(filename)
        && let Some(uuid_str) = captures.get(1)
    {
        return Uuid::from_str(uuid_str.as_str()).map_err(|_| ImageAnalysisError::InvalidUuid {
            filename: filename.to_owned(),
        });
    }
    Err(ImageAnalysisError::InvalidUuid {
        filename: filename.to_owned(),
    })
}

pub fn is_preview_filename(filename: &str) -> bool {
    filename.contains("_preview.") || filename.contains("-preview.")
}

/// Extract filename from a path, falling back to "unknown".
#[must_use]
pub fn filename_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_owned()
}

pub async fn read_image_as_base64(
    image_path: &Path,
    filename: &str,
) -> Result<String, ImageAnalysisError> {
    let metadata = tokio::fs::metadata(image_path).await.map_err(|err| {
        ImageAnalysisError::ProcessingError {
            filename: filename.to_owned(),
            error: err.to_string(),
        }
    })?;
    if metadata.len() == 0 {
        return Err(ImageAnalysisError::EmptyFile {
            filename: filename.to_owned(),
        });
    }
    let mut image_file = tokio::fs::File::open(image_path).await.map_err(|err| {
        ImageAnalysisError::ProcessingError {
            filename: filename.to_owned(),
            error: err.to_string(),
        }
    })?;
    let mut image_data = Vec::new();
    image_file
        .read_to_end(&mut image_data)
        .await
        .map_err(|err| ImageAnalysisError::ProcessingError {
            filename: filename.to_owned(),
            error: err.to_string(),
        })?;
    Ok(STANDARD.encode(&image_data))
}

/// Check overwrite policy and return decision on how to handle the asset.
pub async fn check_overwrite_policy(
    data_access: &DataAccess,
    asset_id: &Uuid,
    overwrite_policy: OverwritePolicy,
) -> Result<OverwriteDecision, ImageAnalysisError> {
    match overwrite_policy {
        OverwritePolicy::All => Ok(OverwriteDecision::AnalyzeFresh),
        OverwritePolicy::None => {
            if data_access.has_description(asset_id).await? {
                return Ok(OverwriteDecision::Skip);
            }
            Ok(OverwriteDecision::AnalyzeFresh)
        }
        OverwritePolicy::MissingAi => match data_access.get_description(asset_id).await {
            Ok(Some(desc)) => {
                if get_ai_block_pattern().is_match(&desc) {
                    return Ok(OverwriteDecision::Skip);
                }
                Ok(OverwriteDecision::PreserveExisting(desc))
            }
            Ok(None) => Ok(OverwriteDecision::AnalyzeFresh),
            Err(err) => Err(err),
        },
    }
}

pub async fn build_final_description(
    analysis: &ImageAnalysisResult,
    data_access: &DataAccess,
    preserve_human: bool,
    existing_description: Option<String>,
) -> Result<String, ImageAnalysisError> {
    let ai_wrapped = format!("[AI]\n{}\n[/AI]", analysis.description.trim());

    if !preserve_human {
        return Ok(ai_wrapped);
    }

    let existing = match existing_description {
        Some(desc) => desc,
        None => match data_access.get_description(&analysis.asset_id).await {
            Ok(Some(desc)) => desc,
            Ok(None) => ai_wrapped.clone(),
            Err(err) => {
                warn!(
                    "Failed to get existing description for asset {}, cannot preserve human text: {}",
                    analysis.asset_id, err
                );
                return Err(err);
            }
        },
    };

    let re = get_ai_block_pattern();
    if re.is_match(&existing) {
        Ok(re
            .replace(&existing, format!("\n{ai_wrapped}\n"))
            .trim()
            .to_owned())
    } else {
        Ok(format!("{}\n\n{}", existing.trim(), ai_wrapped))
    }
}

pub fn determine_locale(
    user_lang: &str,
    system_locale: &str,
    available_locales: &[Cow<'_, str>],
) -> String {
    if !user_lang.is_empty() {
        let user_locale_lower = user_lang.to_lowercase();

        if available_locales
            .iter()
            .any(|loc| loc.as_ref().eq_ignore_ascii_case(&user_locale_lower))
        {
            return user_locale_lower;
        }

        let available_locales_str = available_locales.join(", ");
        eprintln!(
            "{}",
            rust_i18n::t!(
                "autodetect.locale_not_supported",
                locale = user_locale_lower,
                available = available_locales_str
            )
        );
    }

    if available_locales
        .iter()
        .any(|loc| loc.as_ref().eq_ignore_ascii_case(system_locale))
    {
        return system_locale.to_owned();
    }
    "en".to_owned()
}

pub fn validate_args(args: &crate::args::Args) -> Result<(), Box<dyn std::error::Error>> {
    if args.combined && args.monitor {
        eprintln!("{}", rust_i18n::t!("error.incompatible_flags"));
        eprintln!("{}", rust_i18n::t!("error.combined_monitor_conflict"));
        eprintln!("{}", rust_i18n::t!("error.use_combined_or_monitor"));
        Err("incompatible flags".into())
    } else {
        Ok(())
    }
}

pub fn validate_immich_directory(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        return Err(format!(
            "{}",
            rust_i18n::t!(
                "error.directory_not_found",
                path = path.display().to_string()
            )
        )
        .into());
    }
    if !path.is_dir() {
        return Err(format!(
            "{}",
            rust_i18n::t!("error.not_a_directory", path = path.display().to_string())
        )
        .into());
    }
    Ok(())
}
