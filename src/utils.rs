use crate::{
    args::{Interface, OverwritePolicy},
    error::ImageAnalysisError,
    host_manager::ImageAnalysisResult,
    immich_api::ImmichApiProvider,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use log::{debug, warn};
use regex::Regex;
use reqwest::{
    StatusCode,
    header::{HeaderMap, HeaderValue, USER_AGENT},
};
use std::{borrow::Cow, error::Error, io::Cursor, sync::OnceLock};
use strsim::levenshtein;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverwriteDecision {
    Skip,
    SkipBlocked { reason: String },
    AnalyzeFresh,
    PreserveExisting(String),
}

/// Distinctive marker embedded in a stored description for assets the AI provider
/// permanently rejected.
pub const BLOCKED_MARKER_PREFIX: &str = "IMMICH-ANALYZE:BLOCKED";

/// Builds the marker text persisted for a permanently rejected asset.
#[must_use]
pub fn blocked_marker_text(status: StatusCode, message: &str) -> String {
    if status.is_success() {
        format!("{BLOCKED_MARKER_PREFIX}: {message}")
    } else {
        format!("{BLOCKED_MARKER_PREFIX}: {message} (HTTP {status})")
    }
}

/// Extracts the reason recorded in a permanent-rejection marker, if the given
/// description is one. Returns `None` for any other description.
#[must_use]
pub fn extract_blocked_reason(description: &str) -> Option<String> {
    let (_, after) = description.split_once(BLOCKED_MARKER_PREFIX)?;
    let unwrapped = match after.split_once("[/AI]") {
        Some((before, _)) => before,
        None => after,
    };
    let reason = unwrapped.strip_prefix(": ").unwrap_or(unwrapped).trim();
    if reason.is_empty() {
        None
    } else {
        Some(reason.to_owned())
    }
}

/// Build the default HTTP header(s) for outgoing API requests.
#[must_use]
pub fn default_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(concat!("immich-analyze/", env!("CARGO_PKG_VERSION"))),
    );
    headers
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

static AI_BLOCK_PATTERN: OnceLock<Regex> = OnceLock::new();

pub fn get_ai_block_pattern() -> &'static Regex {
    AI_BLOCK_PATTERN
        .get_or_init(|| Regex::new(r"(?s)\[AI\].*?\[/AI\]").expect("Invalid AI block regex"))
}

pub fn image_bytes_to_png_base64(
    image_data: Bytes,
    asset_id: Uuid,
    max_image_size: u32,
) -> Result<String, ImageAnalysisError> {
    if image_data.is_empty() {
        return Err(ImageAnalysisError::EmptyFile { asset_id });
    }
    let image = image::load_from_memory(&image_data).map_err(|err| {
        ImageAnalysisError::ProcessingError {
            asset_id,
            error: format_error_chain(&err),
        }
    })?;
    drop(image_data);
    let mut png_data = Cursor::new(Vec::new());
    let output_image = if max_image_size > 0 && image.width().max(image.height()) > max_image_size {
        let resized = image.resize(
            max_image_size,
            max_image_size,
            image::imageops::FilterType::Lanczos3,
        );
        debug!(
            "Downscaling {asset_id} from {}x{} to {}x{}",
            image.width(),
            image.height(),
            resized.width(),
            resized.height()
        );
        drop(image);
        resized
    } else {
        image
    };
    output_image
        .write_to(&mut png_data, image::ImageFormat::Png)
        .map_err(|err| ImageAnalysisError::ProcessingError {
            asset_id,
            error: format_error_chain(&err),
        })?;
    Ok(STANDARD.encode(png_data.into_inner()))
}

/// Check overwrite policy and return decision on how to handle the asset.
pub async fn check_overwrite_policy(
    immich_api_provider: &ImmichApiProvider,
    asset_id: &Uuid,
    overwrite_policy: OverwritePolicy,
) -> Result<OverwriteDecision, ImageAnalysisError> {
    if !immich_api_provider.asset_exists(asset_id).await? {
        return Err(ImageAnalysisError::AssetNotFound {
            asset_id: *asset_id,
        });
    }
    match overwrite_policy {
        OverwritePolicy::All => Ok(OverwriteDecision::AnalyzeFresh),
        OverwritePolicy::None => {
            if immich_api_provider.has_description(asset_id).await? {
                if let Some(desc) = immich_api_provider.get_description(asset_id).await?
                    && let Some(reason) = extract_blocked_reason(&desc)
                {
                    return Ok(OverwriteDecision::SkipBlocked { reason });
                }
                return Ok(OverwriteDecision::Skip);
            }
            Ok(OverwriteDecision::AnalyzeFresh)
        }
        OverwritePolicy::MissingAi => match immich_api_provider.get_description(asset_id).await {
            Ok(Some(desc)) => {
                if let Some(reason) = extract_blocked_reason(&desc) {
                    return Ok(OverwriteDecision::SkipBlocked { reason });
                }
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
    immich_api_provider: &ImmichApiProvider,
    preserve_human: bool,
    existing_description: Option<String>,
    disable_ai_wrapper: bool,
) -> Result<String, ImageAnalysisError> {
    if disable_ai_wrapper {
        return Ok(analysis.description.trim().to_owned());
    }

    let ai_wrapped = format!("[AI]\n{}\n[/AI]", analysis.description.trim());

    if !preserve_human {
        return Ok(ai_wrapped);
    }

    let existing = match existing_description {
        Some(desc) => desc,
        None => match immich_api_provider
            .get_description(&analysis.asset_id)
            .await
        {
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

pub fn validate_args(args: &crate::args::Args) -> Result<(), Box<dyn Error>> {
    if args.combined && args.monitor {
        eprintln!("{}", rust_i18n::t!("error.incompatible_flags"));
        eprintln!("{}", rust_i18n::t!("error.combined_monitor_conflict"));
        eprintln!("{}", rust_i18n::t!("error.use_combined_or_monitor"));
        Err("incompatible flags".into())
    } else {
        if args.disable_ai_wrapper
            && args.effective_overwrite_policy() == OverwritePolicy::MissingAi
        {
            println!("{}", rust_i18n::t!("warning.disable_ai_wrapper_missing_ai"));
        }
        Ok(())
    }
}

/// Format the full error chain (including `source()`) so that serde field names
/// and byte offsets are not discarded by a bare `.to_string()`.
pub fn format_error_chain(err: &dyn Error) -> String {
    let mut msg = err.to_string();
    let mut source = err.source();
    while let Some(inner) = source {
        msg.push_str(". Caused by: ");
        msg.push_str(&inner.to_string());
        source = inner.source();
    }
    msg
}

const MAX_SUGGESTION_DISTANCE: usize = 5;

pub fn closest_name(name: &str, available: &[String]) -> Option<String> {
    let mut closest: Option<(usize, &str)> = None;
    for candidate in available {
        let distance = levenshtein(name, candidate);
        if distance <= MAX_SUGGESTION_DISTANCE
            && closest.is_none_or(|(best_distance, _)| distance < best_distance)
        {
            closest = Some((distance, candidate));
        }
    }
    closest.map(|(_, candidate)| candidate.to_owned())
}

/// Normalizes a model name to a canonical form for comparison.
///
/// Ollama treats a bare name (`name`) as an alias for `name:latest`, so bare names
/// are canonicalized to the full `name:latest`.
///
/// Other interfaces are left as-is.
fn normalize_model_name(interface: Interface, name: &str) -> Cow<'_, str> {
    if !interface.ollama_tag_semantics() || name.contains(':') {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(format!("{name}:latest"))
    }
}

/// Returns `true` if the configured model is served as one of the available models.
/// Whether two names refer to the same model is decided by the interface's naming semantics.
pub fn is_model_served(interface: Interface, model: &str, available: &[String]) -> bool {
    let target = normalize_model_name(interface, model);
    available
        .iter()
        .any(|served| normalize_model_name(interface, served) == target)
}

/// How a provider-reported error message should be classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderMessageClass {
    ContentPolicyBlock,
    Transient,
    Unknown,
}

/// Classifies a provider error message. A message is only treated as a permanent
/// content-policy block when it positively matches known content-policy indicators;
/// anything else is either transient or unknown.
pub fn classify_provider_message(message: &str) -> ProviderMessageClass {
    const CONTENT_POLICY_INDICATORS: &[&str] = &[
        "prohibited",
        "content filter",
        "content policy",
        "policy violation",
        "blocked content",
        "disallowed",
    ];
    const TRANSIENT_INDICATORS: &[&str] = &[
        "rate limit",
        "too many requests",
        "temporarily",
        "unavailable",
        "not loaded",
        "not available",
        "capacity",
        "overloaded",
        "overload",
        "busy",
        "retry",
        "try again",
        "maintenance",
        "high demand",
        "exhausted",
    ];
    let lower = message.to_lowercase();
    if CONTENT_POLICY_INDICATORS
        .iter()
        .any(|indicator| lower.contains(indicator))
    {
        ProviderMessageClass::ContentPolicyBlock
    } else if TRANSIENT_INDICATORS
        .iter()
        .any(|indicator| lower.contains(indicator))
    {
        ProviderMessageClass::Transient
    } else {
        ProviderMessageClass::Unknown
    }
}
