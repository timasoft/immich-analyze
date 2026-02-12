use crate::error::ImageAnalysisError;
use regex::Regex;
use std::{path::Path, str::FromStr};
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

/// Extract UUID from preview filename (works with Immich naming pattern)
pub fn extract_uuid_from_preview_filename(filename: &str) -> Result<Uuid, ImageAnalysisError> {
    static PREVIEW_PATTERN: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| {
        Regex::new(r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})-preview")
            .expect("Invalid preview filename regex")
    });
    static UUID_PATTERN: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| {
        Regex::new(r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})")
            .expect("Invalid uuid regex")
    });
    if let Some(captures) = PREVIEW_PATTERN.captures(filename) {
        if let Some(uuid_str) = captures.get(1) {
            return Uuid::from_str(uuid_str.as_str()).map_err(|_| ImageAnalysisError::InvalidUuid {
                filename: filename.to_string(),
            });
        }
    }
    if let Some(captures) = UUID_PATTERN.captures(filename) {
        if let Some(uuid_str) = captures.get(1) {
            return Uuid::from_str(uuid_str.as_str()).map_err(|_| ImageAnalysisError::InvalidUuid {
                filename: filename.to_string(),
            });
        }
    }
    Err(ImageAnalysisError::InvalidUuid {
        filename: filename.to_string(),
    })
}

pub fn determine_locale(
    user_lang: &str,
    system_locale: &str,
    available_locales: &[&str],
) -> String {
    if !user_lang.is_empty() {
        let user_locale = user_lang.to_lowercase();
        if available_locales.iter().any(|&loc| loc == user_locale) {
            return user_locale;
        }
        let available_locales_str = available_locales.join(", ");
        eprintln!(
            "{}",
            rust_i18n::t!(
                "autodetect.locale_not_supported",
                locale = user_locale,
                available = available_locales_str
            )
        );
    }
    if available_locales.contains(&system_locale) {
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

pub fn handle_processing_error(error: &ImageAnalysisError, filename: &str) {
    match error {
        ImageAnalysisError::EmptyFile { filename } => {
            eprintln!("{}", rust_i18n::t!("error.empty_file", filename = filename));
        }
        ImageAnalysisError::HttpError {
            filename,
            status,
            response,
        } => {
            eprintln!(
                "{}",
                rust_i18n::t!(
                    "error.http_error_with_details",
                    filename = filename,
                    status = status.to_string(),
                    response = response
                )
            );
        }
        ImageAnalysisError::EmptyResponse { filename } => {
            eprintln!(
                "{}",
                rust_i18n::t!("error.empty_response", filename = filename)
            );
        }
        ImageAnalysisError::JsonParsing { filename, error } => {
            eprintln!(
                "{}",
                rust_i18n::t!(
                    "error.json_parsing_with_details",
                    filename = filename,
                    error = error
                )
            );
        }
        ImageAnalysisError::FileWriteTimeout { filename, timeout } => {
            eprintln!(
                "{}",
                rust_i18n::t!(
                    "error.file_write_timeout_with_details",
                    filename = filename,
                    timeout = timeout.to_string()
                )
            );
        }
        ImageAnalysisError::AllHostsUnavailable => {
            eprintln!("{}", rust_i18n::t!("error.all_ollama_hosts_unavailable"));
        }
        ImageAnalysisError::OllamaRequestTimeout => {
            eprintln!("{}", rust_i18n::t!("error.ollama_request_timeout"));
        }
        _ => {
            eprintln!(
                "{}",
                rust_i18n::t!("error.critical_processing_error", filename = filename)
            );
        }
    }
}
