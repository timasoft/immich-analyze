use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use immich_analyze::{
    args::{Interface, OverwritePolicy},
    error::AnalysisError,
    host_manager::ImageAnalysisResult,
    immich_api::ApiProvider,
    utils::{
        BLOCKED_MARKER_PREFIX, OverwriteDecision, ProviderMessageClass, blocked_marker_text,
        build_final_description, check_overwrite_policy, classify_provider_message, closest_name,
        default_headers, determine_locale, extract_blocked_reason, format_error_chain,
        get_ai_block_pattern, image_bytes_to_png_base64, is_model_served,
    },
};
use reqwest::{StatusCode, header::USER_AGENT};
use std::{borrow::Cow, error::Error, fmt, io::Cursor};
use uuid::Uuid;

/// Nothing listens on port 1, so any request aimed here fails instantly with a connection error.
const UNREACHABLE_IMMICH_URL: &str = "http://127.0.0.1:1";

fn unreachable_provider() -> ApiProvider {
    ApiProvider::new(UNREACHABLE_IMMICH_URL, &["test-key".to_owned()]).expect("valid configuration")
}

fn analysis(description: &str) -> ImageAnalysisResult {
    ImageAnalysisResult {
        description: description.to_owned(),
        asset_id: Uuid::from_u128(1),
    }
}

fn png_bytes(width: u32, height: u32) -> Bytes {
    let picture = image::RgbImage::from_pixel(width, height, image::Rgb([12, 34, 56]));
    let mut encoded = Vec::new();
    picture
        .write_to(&mut Cursor::new(&mut encoded), image::ImageFormat::Png)
        .expect("in-memory PNG encoding works");
    Bytes::from(encoded)
}

fn decoded_dimensions(base64_png: &str) -> (u32, u32) {
    let raw = STANDARD.decode(base64_png).expect("result is valid base64");
    let picture = image::load_from_memory(&raw).expect("result decodes as an image");
    (picture.width(), picture.height())
}

#[test]
fn blocked_marker_text_records_status_for_failures() {
    let text = blocked_marker_text(StatusCode::BAD_REQUEST, "PROHIBITED_CONTENT");

    assert!(text.starts_with(BLOCKED_MARKER_PREFIX));
    assert_eq!(
        text,
        "IMMICH-ANALYZE:BLOCKED: PROHIBITED_CONTENT (HTTP 400 Bad Request)"
    );
}

#[test]
fn blocked_marker_text_omits_status_for_success() {
    assert_eq!(
        blocked_marker_text(StatusCode::OK, "odd but harmless"),
        "IMMICH-ANALYZE:BLOCKED: odd but harmless"
    );
}

#[test]
fn extract_blocked_reason_reads_back_what_was_written() {
    let text = blocked_marker_text(StatusCode::UNPROCESSABLE_ENTITY, "PROHIBITED_CONTENT");

    assert_eq!(
        extract_blocked_reason(&text).as_deref(),
        Some("PROHIBITED_CONTENT (HTTP 422 Unprocessable Entity)")
    );
}

#[test]
fn extract_blocked_reason_strips_the_ai_wrapper() {
    let wrapped = format!("[AI]{BLOCKED_MARKER_PREFIX}: NSFW[/AI]");

    assert_eq!(extract_blocked_reason(&wrapped).as_deref(), Some("NSFW"));
}

#[test]
fn extract_blocked_reason_ignores_regular_descriptions() {
    assert_eq!(extract_blocked_reason("A sunny beach"), None);
    assert_eq!(extract_blocked_reason("[AI]\nA sunny beach\n[/AI]"), None);
}

#[test]
fn extract_blocked_reason_rejects_markers_without_a_reason() {
    assert_eq!(extract_blocked_reason(BLOCKED_MARKER_PREFIX), None);
    assert_eq!(extract_blocked_reason("IMMICH-ANALYZE:BLOCKED:    "), None);
}

#[test]
fn extract_blocked_reason_finds_marker_inside_a_longer_description() {
    let description = "Human note\n[AI]IMMICH-ANALYZE:BLOCKED: blocked[/AI]\nmore text";

    assert_eq!(
        extract_blocked_reason(description).as_deref(),
        Some("blocked")
    );
}

#[test]
fn default_headers_identify_the_client() {
    let headers = default_headers();

    let expected = format!("immich-analyze/{}", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        headers.get(USER_AGENT).map(|value| value.to_str().ok()),
        Some(Some(expected.as_str()))
    );
}

#[test]
fn ai_block_pattern_matches_a_block_spanning_several_lines() {
    assert!(get_ai_block_pattern().is_match("[AI]\nfirst line\nsecond line\n[/AI]"));
}

#[test]
fn ai_block_pattern_ignores_text_without_a_closing_marker() {
    let pattern = get_ai_block_pattern();

    assert!(!pattern.is_match("[AI]\nno closing marker"));
    assert!(!pattern.is_match("no markers at all"));
}

#[test]
fn ai_block_pattern_matches_two_blocks_separately() {
    let description = "[AI]\nfirst\n[/AI] between [AI]\nsecond\n[/AI]";

    assert_eq!(get_ai_block_pattern().find_iter(description).count(), 2);
}

#[test]
fn locale_is_taken_from_the_user_request_when_supported() {
    let locales = [Cow::Borrowed("en"), Cow::Borrowed("ru")];

    assert_eq!(determine_locale("RU", "en", &locales), "ru");
    assert_eq!(determine_locale("en", "ru", &locales), "en");
}

#[test]
fn locale_falls_back_to_the_system_locale_when_the_request_is_unsupported() {
    let locales = [Cow::Borrowed("en"), Cow::Borrowed("ru")];

    assert_eq!(determine_locale("de", "ru", &locales), "ru");
}

#[test]
fn locale_falls_back_to_english_when_nothing_matches() {
    let locales = [Cow::Borrowed("en")];

    assert_eq!(determine_locale("", "de", &locales), "en");
    assert_eq!(determine_locale("fr", "de", &locales), "en");
}

#[test]
fn closest_name_suggests_near_misses_only() {
    let available = ["llava:latest".to_owned(), "qwen3-vl:8b".to_owned()];

    assert_eq!(
        closest_name("qwen3-vl:8b", &available).as_deref(),
        Some("qwen3-vl:8b")
    );
    assert_eq!(
        closest_name("qwen3-vl:8c", &available).as_deref(),
        Some("qwen3-vl:8b")
    );
    assert_eq!(closest_name("completely-different", &available), None);
    assert_eq!(closest_name("llava:latest", &[]), None);
}

#[test]
fn ollama_treats_bare_model_names_as_latest_aliases() {
    let available = ["llava:latest".to_owned()];

    assert!(is_model_served(Interface::Ollama, "llava", &available));
    assert!(is_model_served(
        Interface::Ollama,
        "llava:latest",
        &available
    ));
    assert!(!is_model_served(Interface::Ollama, "llava:7b", &available));
}

#[test]
fn openai_compatible_interfaces_compare_model_names_verbatim() {
    let available = ["llava:latest".to_owned()];

    assert!(is_model_served(
        Interface::Llamacpp,
        "llava:latest",
        &available
    ));
    assert!(!is_model_served(Interface::Llamacpp, "llava", &available));
    assert!(!is_model_served(Interface::OpenRouter, "llava", &available));
}

#[test]
fn provider_messages_are_classified_by_cause() {
    for message in [
        "PROHIBITED_CONTENT",
        "This violates our Content Policy",
        "blocked content detected",
        "input disallowed by provider",
    ] {
        assert_eq!(
            classify_provider_message(message),
            ProviderMessageClass::ContentPolicyBlock,
            "{message} should be a permanent block"
        );
    }

    for message in [
        "model is not loaded yet",
        "rate limit reached, try again",
        "server overloaded",
        "service temporarily unavailable",
    ] {
        assert_eq!(
            classify_provider_message(message),
            ProviderMessageClass::Transient,
            "{message} should be transient"
        );
    }

    for message in ["something went wrong", "", "GPU out of memory"] {
        assert_eq!(
            classify_provider_message(message),
            ProviderMessageClass::Unknown,
            "{message} should be unknown"
        );
    }
}

#[test]
fn content_policy_blocks_win_over_transient_wording() {
    assert_eq!(
        classify_provider_message("content policy violation, please retry later"),
        ProviderMessageClass::ContentPolicyBlock
    );
}

#[derive(Debug)]
struct ErrorLayer {
    message: &'static str,
    source: Option<Box<Self>>,
}

impl fmt::Display for ErrorLayer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl Error for ErrorLayer {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|inner| &**inner as &(dyn Error + 'static))
    }
}

#[test]
fn format_error_chain_walks_the_whole_source_chain() {
    let leaf = ErrorLayer {
        message: "leaf",
        source: None,
    };
    let middle = ErrorLayer {
        message: "middle",
        source: Some(Box::new(leaf)),
    };
    let top = ErrorLayer {
        message: "top",
        source: Some(Box::new(middle)),
    };

    assert_eq!(
        format_error_chain(&top),
        "top. Caused by: middle. Caused by: leaf"
    );
}

#[test]
fn format_error_chain_handles_errors_without_a_source() {
    let lone = ErrorLayer {
        message: "only layer",
        source: None,
    };

    assert_eq!(format_error_chain(&lone), "only layer");
    assert_eq!(format_error_chain(&std::io::Error::other("plain")), "plain");
}

#[test]
fn image_bytes_to_png_base64_rejects_empty_input() {
    let asset = Uuid::from_u128(1);

    let result = image_bytes_to_png_base64(Bytes::new(), asset, 1024);

    assert_eq!(result, Err(AnalysisError::EmptyFile { asset_id: asset }));
}

#[test]
fn image_bytes_to_png_base64_rejects_input_that_is_not_an_image() {
    let asset = Uuid::from_u128(1);

    let result =
        image_bytes_to_png_base64(Bytes::from_static(b"this is not an image"), asset, 1024);

    assert!(
        matches!(
            &result,
            Err(AnalysisError::ProcessingError { asset_id, error })
                if *asset_id == asset && !error.is_empty()
        ),
        "a non-image must fail as a processing error naming the asset and a reason, got {result:?}"
    );
}

#[test]
fn image_bytes_to_png_base64_encodes_a_png() {
    let encoded = image_bytes_to_png_base64(png_bytes(40, 20), Uuid::from_u128(1), 0)
        .expect("valid image is re-encoded");

    assert_eq!(decoded_dimensions(&encoded), (40, 20));
}

#[test]
fn image_bytes_to_png_base64_downscales_the_longest_edge() {
    let encoded =
        image_bytes_to_png_base64(png_bytes(64, 32), Uuid::from_u128(1), 16).expect("valid image");

    assert_eq!(decoded_dimensions(&encoded), (16, 8));
}

#[test]
fn image_bytes_to_png_base64_keeps_images_that_fit_the_limit() {
    let encoded =
        image_bytes_to_png_base64(png_bytes(64, 32), Uuid::from_u128(1), 128).expect("valid image");

    assert_eq!(decoded_dimensions(&encoded), (64, 32));
}

#[tokio::test]
async fn build_final_description_wraps_ai_text_in_markers() {
    let description = build_final_description(
        &analysis("  A cat on a keyboard  "),
        &unreachable_provider(),
        false,
        None,
        false,
    )
    .await
    .expect("no lookup needed");

    assert_eq!(description, "[AI]\nA cat on a keyboard\n[/AI]");
}

#[tokio::test]
async fn build_final_description_can_skip_the_wrapper() {
    let description = build_final_description(
        &analysis("  A cat on a keyboard  "),
        &unreachable_provider(),
        true,
        None,
        true,
    )
    .await
    .expect("no lookup needed");

    assert_eq!(description, "A cat on a keyboard");
}

#[tokio::test]
async fn build_final_description_replaces_an_existing_ai_block() {
    let existing = "Human note\n[AI]\nold text\n[/AI]\nTrailing note".to_owned();

    let description = build_final_description(
        &analysis("new text"),
        &unreachable_provider(),
        true,
        Some(existing),
        false,
    )
    .await
    .expect("no lookup needed");

    assert_eq!(
        description,
        "Human note\n\n[AI]\nnew text\n[/AI]\n\nTrailing note"
    );
}

#[tokio::test]
async fn build_final_description_appends_when_there_is_no_ai_block() {
    let description = build_final_description(
        &analysis("new text"),
        &unreachable_provider(),
        true,
        Some("Human only".to_owned()),
        false,
    )
    .await
    .expect("no lookup needed");

    assert_eq!(description, "Human only\n\n[AI]\nnew text\n[/AI]");
}

#[tokio::test]
async fn build_final_description_looks_up_unknown_existing_text() {
    let result = build_final_description(
        &analysis("new text"),
        &unreachable_provider(),
        true,
        None,
        false,
    )
    .await;

    assert!(
        matches!(result, Err(AnalysisError::HttpClientError { .. })),
        "a failed lookup must not silently drop the human text"
    );
}

#[tokio::test]
async fn overwrite_policy_all_analyzes_even_when_existence_cannot_be_checked() {
    let decision = check_overwrite_policy(
        &unreachable_provider(),
        &Uuid::from_u128(1),
        OverwritePolicy::All,
    )
    .await
    .expect("a failed existence check is treated as \"the asset is there\"");

    assert_eq!(decision, OverwriteDecision::AnalyzeFresh);
}

#[tokio::test]
async fn overwrite_policy_none_reports_lookup_failures() {
    let result = check_overwrite_policy(
        &unreachable_provider(),
        &Uuid::from_u128(1),
        OverwritePolicy::None,
    )
    .await;

    assert!(matches!(result, Err(AnalysisError::HttpClientError { .. })));
}
