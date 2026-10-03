use bytes::Bytes;
use immich_analyze::{args::Interface, error::AnalysisError, host_manager::HostManager};
use serde_json::json;
use std::{num::NonZeroU32, time::Duration};
use uuid::Uuid;

const HOST_A: &str = "http://a.example.com:11434";
const HOST_B: &str = "http://b.example.com:11434";

fn manager(hosts: &[&str], unavailable_duration: Duration) -> HostManager {
    HostManager::new(
        hosts.iter().map(|host| (*host).to_owned()).collect(),
        Interface::Ollama,
        reqwest::Client::new(),
        "test-model".to_owned(),
        5,
        Some(NonZeroU32::new(1).expect("a positive retry limit")),
        Duration::from_millis(1),
        unavailable_duration,
        None,
        0,
    )
}

#[test]
fn model_lists_are_read_from_the_interface_specific_field() {
    let ollama = json!({"models": [{"name": "qwen3-vl:latest"}, {"name": "llava:7b"}]});
    let openai = json!({"data": [{"id": "vendor/qwen3-vl"}]});

    assert_eq!(
        Interface::Ollama.model_names(&ollama),
        ["qwen3-vl:latest", "llava:7b"]
    );
    assert_eq!(
        Interface::OpenRouter.model_names(&openai),
        ["vendor/qwen3-vl"]
    );
    assert!(Interface::Ollama.model_names(&json!({})).is_empty());
}

#[test]
fn responses_are_read_from_the_interface_specific_field() {
    assert_eq!(
        Interface::Ollama.parse_response(&json!({"message": {"content": " a cat "}})),
        Some(" a cat ")
    );
    assert_eq!(
        Interface::Llamacpp
            .parse_response(&json!({"choices": [{"message": {"content": " a dog "}}]})),
        Some(" a dog ")
    );
}

#[test]
fn unusable_responses_yield_no_content() {
    assert_eq!(Interface::Ollama.parse_response(&json!({})), None);
    assert_eq!(
        Interface::Ollama.parse_response(&json!({"message": {"content": 42_i32}})),
        None
    );
    assert_eq!(
        Interface::Llamacpp.parse_response(&json!({"choices": []})),
        None
    );
    assert_eq!(
        Interface::Llamacpp.parse_response(&json!({"choices": [{"message": {}}]})),
        None
    );
}

#[test]
fn only_openrouter_reports_permanent_content_policy_blocks() {
    let blocked = json!({"error": {"code": 400_i32, "message": "PROHIBITED_CONTENT"}});

    assert_eq!(
        Interface::OpenRouter
            .permanent_rejection_message(&blocked)
            .as_deref(),
        Some("PROHIBITED_CONTENT")
    );
    assert_eq!(
        Interface::OpenRouter
            .permanent_rejection_message(&json!({"error": "content policy violation"}))
            .as_deref(),
        Some("content policy violation")
    );
    assert_eq!(
        Interface::OpenRouter
            .permanent_rejection_message(&json!({"error": {"message": "  blocked content  "}}))
            .as_deref(),
        Some("blocked content")
    );
    assert!(
        Interface::Ollama
            .permanent_rejection_message(&blocked)
            .is_none()
    );
    assert!(
        Interface::Llamacpp
            .permanent_rejection_message(&blocked)
            .is_none()
    );
}

#[test]
fn transient_and_unrelated_failures_are_not_permanent_blocks() {
    let openrouter = Interface::OpenRouter;

    assert!(
        openrouter
            .permanent_rejection_message(&json!({"error": {"message": "rate limit reached"}}))
            .is_none()
    );
    assert!(
        openrouter
            .permanent_rejection_message(&json!({"error": {"message": "something odd"}}))
            .is_none()
    );
    assert!(
        openrouter
            .permanent_rejection_message(&json!({"message": "prohibited content"}))
            .is_none()
    );
    assert!(
        openrouter
            .permanent_rejection_message(&json!({"error": 42_i32}))
            .is_none()
    );
}

#[test]
fn the_first_configured_host_is_used_while_everything_is_healthy() {
    let host_manager = manager(&[HOST_A, HOST_B], Duration::from_secs(60));

    assert_eq!(
        host_manager
            .get_available_host()
            .expect("a host is available"),
        HOST_A
    );
}

#[test]
fn hosts_marked_unavailable_are_skipped() {
    let host_manager = manager(&[HOST_A, HOST_B], Duration::from_secs(60));

    host_manager.mark_host_unavailable(HOST_A);

    assert_eq!(
        host_manager
            .get_available_host()
            .expect("the healthy host is used"),
        HOST_B
    );
}

#[test]
fn the_least_recently_unavailable_host_is_tried_when_all_are_down() {
    let host_manager = manager(&[HOST_A, HOST_B], Duration::from_secs(60));

    host_manager.mark_host_unavailable(HOST_B);
    std::thread::sleep(Duration::from_millis(5));
    host_manager.mark_host_unavailable(HOST_A);

    assert_eq!(
        host_manager
            .get_available_host()
            .expect("b host is retried"),
        HOST_B
    );
}

#[test]
fn unavailability_expires_once_the_window_has_passed() {
    let host_manager = manager(&[HOST_A, HOST_B], Duration::ZERO);

    host_manager.mark_host_unavailable(HOST_A);

    assert_eq!(
        host_manager
            .get_available_host()
            .expect("the entry expired"),
        HOST_A
    );
}

#[test]
fn permanently_unavailable_hosts_are_never_selected_again() {
    let host_manager = manager(&[HOST_A], Duration::from_secs(60));

    host_manager.mark_host_unavailable_forever(HOST_A);

    assert!(matches!(
        host_manager.get_available_host(),
        Err(AnalysisError::AllHostsUnavailable)
    ));
}

#[test]
fn permanent_unavailability_outranks_the_retry_fallback() {
    let host_manager = manager(&[HOST_A, HOST_B], Duration::from_secs(60));

    host_manager.mark_host_unavailable_forever(HOST_A);
    host_manager.mark_host_unavailable(HOST_B);

    assert_eq!(
        host_manager
            .get_available_host()
            .expect("the healthy host is used"),
        HOST_B
    );
}

#[tokio::test]
async fn missing_host_configuration_is_reported_instead_of_requested() {
    let host_manager = manager(&[], Duration::from_secs(60));

    assert!(matches!(
        host_manager.get_available_host(),
        Err(AnalysisError::AllHostsUnavailable)
    ));
    assert!(matches!(
        host_manager
            .check_model_available_and_mark_unavailable_hosts()
            .await,
        Err(AnalysisError::NoHostsConfigured)
    ));
}

#[tokio::test]
async fn analysis_rejects_undecodable_images_before_contacting_any_host() {
    let host_manager = manager(&[HOST_A], Duration::from_secs(60));

    let error = host_manager
        .analyze_image(
            Bytes::from_static(b"not an image"),
            "describe",
            Uuid::from_u128(1),
        )
        .await
        .expect_err("an undecodable image is rejected during encoding");

    assert!(matches!(error, AnalysisError::ProcessingError { .. }));
}
