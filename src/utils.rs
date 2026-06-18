use crate::{data_access::DataAccess, database::ImageAnalysisResult, error::ImageAnalysisError};
use base64::{Engine, engine::general_purpose::STANDARD};
use log::warn;
use regex::Regex;
use std::{borrow::Cow, io::Read, path::Path, str::FromStr, sync::OnceLock};
use uuid::Uuid;

/// Get system locale from environment variables
pub fn get_system_locale() -> String {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .map(|s| {
            s.split('.')
                .next()
                .unwrap_or("en")
                .split('_')
                .next()
                .unwrap_or("en")
                .to_lowercase()
        })
        .unwrap_or_else(|_| "en".to_string())
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
        Regex::new(r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})[-_]preview")
            .expect("Invalid preview filename regex")
    });
    let uuid_pattern = UUID_PATTERN.get_or_init(|| {
        Regex::new(r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})")
            .expect("Invalid uuid regex")
    });
    if let Some(captures) = preview_pattern.captures(filename)
        && let Some(uuid_str) = captures.get(1)
    {
        return Uuid::from_str(uuid_str.as_str()).map_err(|_| ImageAnalysisError::InvalidUuid {
            filename: filename.to_string(),
        });
    }
    if let Some(captures) = uuid_pattern.captures(filename)
        && let Some(uuid_str) = captures.get(1)
    {
        return Uuid::from_str(uuid_str.as_str()).map_err(|_| ImageAnalysisError::InvalidUuid {
            filename: filename.to_string(),
        });
    }
    Err(ImageAnalysisError::InvalidUuid {
        filename: filename.to_string(),
    })
}

pub fn is_preview_filename(filename: &str) -> bool {
    filename.contains("_preview.") || filename.contains("-preview.")
}

pub fn read_image_as_base64(
    image_path: &Path,
    filename: &str,
) -> Result<String, ImageAnalysisError> {
    let metadata =
        std::fs::metadata(image_path).map_err(|e| ImageAnalysisError::ProcessingError {
            filename: filename.to_string(),
            error: e.to_string(),
        })?;
    if metadata.len() == 0 {
        return Err(ImageAnalysisError::EmptyFile {
            filename: filename.to_string(),
        });
    }
    let mut image_file =
        std::fs::File::open(image_path).map_err(|e| ImageAnalysisError::ProcessingError {
            filename: filename.to_string(),
            error: e.to_string(),
        })?;
    let mut image_data = Vec::new();
    image_file
        .read_to_end(&mut image_data)
        .map_err(|e| ImageAnalysisError::ProcessingError {
            filename: filename.to_string(),
            error: e.to_string(),
        })?;
    Ok(STANDARD.encode(&image_data))
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
            Err(e) => {
                warn!(
                    "Failed to get existing description for asset {}, cannot preserve human text: {}",
                    analysis.asset_id, e
                );
                return Err(e);
            }
        },
    };

    let re = get_ai_block_pattern();
    if re.is_match(&existing) {
        Ok(re
            .replace(&existing, format!("\n{}\n", ai_wrapped))
            .trim()
            .to_string())
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
        return system_locale.to_string();
    }
    "en".to_string()
}

pub fn validate_args(args: &crate::args::Args) -> Result<(), Box<dyn std::error::Error>> {
    if args.combined && args.monitor {
        eprintln!("{}", rust_i18n::t!("error.incompatible_flags"));
        eprintln!("{}", rust_i18n::t!("error.combined_monitor_conflict"));
        eprintln!("{}", rust_i18n::t!("error.use_combined_or_monitor"));
        std::process::exit(1);
    }
    Ok(())
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
