use immich_analyze::error::{AnalysisError, Subject};
use reqwest::StatusCode;
use uuid::Uuid;

#[test]
fn server_errors_and_rate_limits_are_retried_while_client_errors_are_not() {
    let retryable = |status| {
        AnalysisError::HttpError {
            status,
            subject: Subject::AssetsList,
            response: String::new(),
        }
        .is_retryable()
    };

    assert!(retryable(StatusCode::INTERNAL_SERVER_ERROR));
    assert!(retryable(StatusCode::BAD_GATEWAY));
    assert!(retryable(StatusCode::SERVICE_UNAVAILABLE));
    assert!(retryable(StatusCode::TOO_MANY_REQUESTS));
    assert!(!retryable(StatusCode::BAD_REQUEST));
    assert!(!retryable(StatusCode::UNAUTHORIZED));
    assert!(!retryable(StatusCode::FORBIDDEN));
    assert!(!retryable(StatusCode::NOT_FOUND));
}

#[test]
fn transport_failures_describe_the_offending_asset() {
    rust_i18n::set_locale("en");
    let asset = Uuid::from_u128(9);

    let with_asset = AnalysisError::HttpClientError {
        asset_id: Some(asset),
        error: "connection refused".to_owned(),
    }
    .user_message();
    let without_asset = AnalysisError::HttpClientError {
        asset_id: None,
        error: "connection refused".to_owned(),
    }
    .user_message();

    assert!(with_asset.contains(&asset.to_string()));
    assert!(with_asset.contains("connection refused"));
    assert!(!without_asset.contains(&asset.to_string()));
    assert!(without_asset.contains("connection refused"));
}

#[test]
fn missing_model_suggests_the_closest_match() {
    rust_i18n::set_locale("en");

    let with_suggestion = AnalysisError::ModelNotFound {
        model: "qwen3-vl:4b".to_owned(),
        host: "http://localhost:11434".to_owned(),
        closest: Some("qwen3-vl:8b".to_owned()),
    }
    .user_message();
    let without_suggestion = AnalysisError::ModelNotFound {
        model: "qwen3-vl:4b".to_owned(),
        host: "http://localhost:11434".to_owned(),
        closest: None,
    }
    .user_message();

    assert!(with_suggestion.contains("qwen3-vl:4b"));
    assert!(with_suggestion.contains("http://localhost:11434"));
    assert!(with_suggestion.contains("qwen3-vl:8b"));
    assert!(without_suggestion.contains("qwen3-vl:4b"));
    assert!(without_suggestion.contains("http://localhost:11434"));
    assert!(!without_suggestion.contains("qwen3-vl:8b"));
}

#[test]
fn rejections_echo_the_provider_and_caller_input() {
    rust_i18n::set_locale("en");
    let asset = Uuid::from_u128(11);

    let rejection = AnalysisError::ProviderRejected {
        status: StatusCode::BAD_REQUEST,
        asset_id: asset,
        message: "PROHIBITED_CONTENT".to_owned(),
    }
    .user_message();

    assert!(rejection.contains(&asset.to_string()));
    assert!(rejection.contains("400"));
    assert!(rejection.contains("PROHIBITED_CONTENT"));
}
