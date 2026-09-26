#![warn(non_ascii_idents)]

use clap::Parser as _;
use std::{num::NonZeroU32, sync::Arc, time::Duration};

mod args;
mod asset_processing;
mod config;
mod error;
mod health;
mod host_manager;
mod immich_api;
mod monitor;
mod progress;
mod prompt_enricher;
mod utils;

use args::{Args, OverwritePolicy};
use asset_processing::process_assets_concurrently;
use config::MonitorConfig;
use host_manager::HostManager;
use monitor::monitor_folder;
use progress::SimpleProgress;
use utils::{
    default_headers, determine_locale, format_error_chain, get_system_locale, validate_args,
};

use crate::immich_api::ImmichApiProvider;

rust_i18n::i18n!("locales", fallback = "en");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logger to enable debug logging
    env_logger::init();

    let system_locale = get_system_locale();
    let available_locales = rust_i18n::available_locales!();
    let args = Args::parse();

    let final_locale = determine_locale(&args.lang, &system_locale, &available_locales);
    rust_i18n::set_locale(&final_locale);
    println!(
        "{}",
        rust_i18n::t!("autodetect.locale_selected", locale = final_locale)
    );

    validate_args(&args)?;

    // Start health check HTTP server for Docker HEALTHCHECK
    let health_port = args.health_port;
    tokio::spawn(async move {
        health::start_health_server(health_port).await;
    });

    // Create data access based on mode
    let immich_api_provider = {
        let api_url = args
            .immich_api_url
            .as_ref()
            .ok_or("IMMICH_API_URL required. Set via --immich-api-url or IMMICH_API_URL env var")?;
        if args.immich_api_keys.is_empty() {
            return Err("IMMICH_API_KEY required. Set via --immich-api-keys or IMMICH_API_KEY env var (comma-separated for multiple keys)".into());
        }
        let provider = immich_api::ImmichApiProvider::new(api_url, &args.immich_api_keys)?;
        if !args.no_wait_for_immich {
            let timeout_display = if args.wait_timeout == 0 {
                "∞".to_owned()
            } else {
                args.wait_timeout.to_string()
            };
            println!(
                "{}",
                rust_i18n::t!("main.waiting_for_immich", timeout = timeout_display)
            );
            provider
                .wait_until_ready(args.wait_timeout, args.wait_retry_interval)
                .await?;
            println!("{}", rust_i18n::t!("main.immich_ready"));
        }
        println!(
            "{}",
            rust_i18n::t!(
                "main.immich_api_connected",
                api_url = api_url,
                key_count = args.immich_api_keys.len().to_string()
            )
        );
        Arc::new(provider)
    };

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(args.timeout))
        .default_headers(default_headers())
        .build()?;
    let host_manager = Arc::new(HostManager::new(
        args.hosts.clone(),
        args.interface,
        http_client,
        args.model_name.clone(),
        args.timeout,
        NonZeroU32::new(args.max_retries),
        Duration::from_secs(args.retry_delay_seconds),
        Duration::from_secs(args.unavailable_duration),
        args.api_key.clone(),
        args.max_image_size,
    ));

    if !args.no_preflight_model_check
        && let Err(err) = host_manager
            .check_model_available_and_mark_unavailable_hosts()
            .await
    {
        eprintln!(
            "{}",
            rust_i18n::t!(
                "error.model_preflight_check_failed",
                error = err.user_message()
            )
        );
        std::process::exit(1);
    }

    if args.combined {
        run_combined_mode(
            args.clone(),
            immich_api_provider,
            &final_locale,
            host_manager,
        )
        .await?;
    } else if args.monitor {
        run_monitor_mode(&args, immich_api_provider, &final_locale, host_manager).await?;
    } else {
        run_batch_mode(&args, &immich_api_provider, &final_locale, host_manager).await?;
    }

    Ok(())
}

async fn run_combined_mode(
    args: Args,
    immich_api_provider: Arc<ImmichApiProvider>,
    locale: &str,
    host_manager: Arc<HostManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", rust_i18n::t!("main.combined_mode_activated"));
    let batch_handle = {
        let args_clone = args.clone();
        let immich_api_provider_clone = Arc::clone(&immich_api_provider);
        let locale_clone = locale.to_owned();
        let host_manager_clone = Arc::clone(&host_manager);
        tokio::spawn(async move {
            println!("{}", rust_i18n::t!("main.processing_existing_images"));
            if let Err(err) = run_batch_mode(
                &args_clone,
                &immich_api_provider_clone,
                &locale_clone,
                host_manager_clone,
            )
            .await
            {
                eprintln!(
                    "{}",
                    rust_i18n::t!(
                        "error.batch_mode_failed",
                        error = format_error_chain(err.as_ref())
                    )
                );
            }
            println!("{}", rust_i18n::t!("main.batch_mode_completed"));
        })
    };
    println!(
        "{}",
        rust_i18n::t!("main.monitor_mode_started_in_background")
    );
    run_monitor_mode(&args, immich_api_provider, locale, host_manager).await?;
    let _: Result<(), tokio::task::JoinError> = batch_handle.await;
    Ok(())
}

async fn run_monitor_mode(
    args: &Args,
    immich_api_provider: Arc<ImmichApiProvider>,
    locale: &str,
    host_manager: Arc<HostManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", rust_i18n::t!("main.monitor_mode_activated"));
    let overwrite_policy = args.effective_overwrite_policy();
    match overwrite_policy {
        OverwritePolicy::All => println!("{}", rust_i18n::t!("main.ignore_existing_enabled")),
        OverwritePolicy::MissingAi => println!("{}", rust_i18n::t!("main.missing_ai_enabled")),
        OverwritePolicy::None => {}
    }
    let monitor_config = MonitorConfig::from_args(args, locale);
    monitor_folder(
        immich_api_provider,
        &args.prompt,
        &monitor_config,
        host_manager,
    )
    .await?;
    Ok(())
}

async fn run_batch_mode(
    args: &Args,
    immich_api_provider: &ImmichApiProvider,
    locale: &str,
    host_manager: Arc<HostManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    let assets = immich_api_provider.get_assets().await?;

    println!(
        "{}",
        rust_i18n::t!("main.images_to_process", count = assets.len().to_string())
    );
    println!(
        "{}",
        rust_i18n::t!("main.model_name", name = args.model_name)
    );
    println!(
        "{}",
        rust_i18n::t!(
            "main.max_concurrent",
            count = args.max_concurrent.to_string()
        )
    );
    println!(
        "{}",
        rust_i18n::t!("main.timeout", seconds = args.timeout.to_string())
    );
    let overwrite_policy = args.effective_overwrite_policy();
    match overwrite_policy {
        OverwritePolicy::All => println!("{}", rust_i18n::t!("main.ignore_existing_enabled")),
        OverwritePolicy::MissingAi => println!("{}", rust_i18n::t!("main.missing_ai_enabled")),
        OverwritePolicy::None => {}
    }

    let progress = Arc::new(tokio::sync::Mutex::new(SimpleProgress::new(
        assets.len() as u64,
        &rust_i18n::t!("progress.processing_complete"),
    )));

    let results = process_assets_concurrently(
        assets,
        immich_api_provider,
        args,
        locale,
        progress,
        host_manager,
    )
    .await;

    if !args.no_final_output {
        asset_processing::display_results(&results, args.max_concurrent > 1);
    }
    Ok(())
}
