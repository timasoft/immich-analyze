use clap::Parser as _;
use immich_analyze::{
    args::{Args, Interface, OverwritePolicy, ThumbnailSize},
    config::MonitorConfig,
    utils::validate_args,
};

fn parse(extra: &[&str]) -> Args {
    try_parse(extra).expect("arguments parse successfully")
}

fn try_parse(extra: &[&str]) -> Result<Args, clap::Error> {
    let mut argv = vec!["immich-analyze"];
    argv.extend_from_slice(extra);
    Args::try_parse_from(argv)
}

#[test]
fn explicit_values_override_defaults() {
    let args = parse(&[
        "--monitor",
        "--overwrite-existing",
        "--preserve-human",
        "--enrich-prompt",
        "--interface",
        "llamacpp",
        "--thumbnail-size",
        "thumbnail",
        "--api-poll-interval",
        "42",
        "--max-concurrent",
        "8",
        "--max-image-size",
        "1024",
        "--max-retries",
        "3",
        "--model-name",
        "custom-model",
        "--lang",
        "ru",
    ]);

    assert!(args.monitor);
    assert!(args.overwrite_existing);
    assert!(args.preserve_human);
    assert!(args.enrich_prompt);
    assert_eq!(args.interface, Interface::Llamacpp);
    assert_eq!(args.thumbnail_size, ThumbnailSize::Thumbnail);
    assert_eq!(args.api_poll_interval, 42);
    assert_eq!(args.max_concurrent, 8);
    assert_eq!(args.max_image_size, 1024);
    assert_eq!(args.max_retries, 3);
    assert_eq!(args.model_name, "custom-model");
    assert_eq!(args.lang, "ru");
}

#[test]
fn overwrite_policy_takes_precedence_over_the_legacy_flag() {
    assert_eq!(
        parse(&["--overwrite-existing"]).effective_overwrite_policy(),
        OverwritePolicy::All
    );
    assert_eq!(
        parse(&["-O", "missing-ai"]).effective_overwrite_policy(),
        OverwritePolicy::MissingAi
    );
    assert_eq!(
        parse(&["-O", "none", "--overwrite-existing"]).effective_overwrite_policy(),
        OverwritePolicy::None
    );
}

#[test]
fn comma_separated_values_are_split() {
    let args = parse(&["--hosts", "http://a.example.com,http://b.example.com"]);

    assert_eq!(args.hosts, ["http://a.example.com", "http://b.example.com"]);

    let keys = parse(&["--immich-api-keys", "key-one,key-two"]).immich_api_keys;
    assert!(keys.contains(&"key-one".to_owned()));
    assert!(keys.contains(&"key-two".to_owned()));
}

#[test]
fn openrouter_defaults_to_their_own_host() {
    let args = parse(&["--interface", "openrouter"]);

    assert_eq!(args.interface, Interface::OpenRouter);
    assert_eq!(args.hosts, ["https://openrouter.ai/api"]);
}

#[test]
fn mutually_exclusive_flags_are_rejected() {
    assert!(try_parse(&["--preserve-human", "--disable-ai-wrapper"]).is_err());
}

#[test]
fn unknown_enum_values_are_rejected() {
    assert!(try_parse(&["--interface", "not-an-interface"]).is_err());
    assert!(try_parse(&["--overwrite-policy", "sometimes"]).is_err());
}

#[test]
fn validation_rejects_combined_with_monitor() {
    let args = parse(&["--combined", "--monitor"]);

    let error = validate_args(&args).expect_err("combined and monitor cannot be combined");

    assert_eq!(error.to_string(), "incompatible flags");
}

#[test]
fn monitor_config_is_derived_from_args() {
    let args = parse(&[
        "--monitor",
        "--enrich-prompt",
        "--preserve-human",
        "--thumbnail-size",
        "fullsize",
        "--api-poll-interval",
        "7",
        "-O",
        "all",
    ]);

    let config = MonitorConfig::from_args(&args, "ru");

    assert_eq!(config.lang, "ru");
    assert_eq!(config.overwrite_policy, OverwritePolicy::All);
    assert_eq!(config.api_poll_interval, 7);
    assert!(config.enrich_prompt);
    assert!(config.preserve_human);
    assert!(!config.disable_ai_wrapper);
    assert_eq!(config.thumbnail_size, ThumbnailSize::Fullsize);
}

#[test]
fn monitor_config_keeps_the_disabled_ai_wrapper_flag() {
    let args = parse(&["--monitor", "--disable-ai-wrapper"]);

    let config = MonitorConfig::from_args(&args, "en");

    assert!(config.disable_ai_wrapper);
    assert!(!config.preserve_human);
}
