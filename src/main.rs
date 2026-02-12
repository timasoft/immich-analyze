use clap::Parser;
use std::{path::Path, sync::Arc};
use tokio_postgres::{Client as PgClient, NoTls};

mod args;
mod config;
mod database;
mod error;
mod file_processing;
mod llamacpp;
mod monitor;
mod ollama;
mod progress;
mod utils;

use args::Args;
use config::MonitorConfig;
use file_processing::{get_immich_preview_files, handle_no_files, process_files_concurrently};
use monitor::monitor_folder;
use progress::SimpleProgress;
use utils::{determine_locale, get_system_locale, validate_args, validate_immich_directory};

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
    let immich_root = Path::new(&args.immich_root);
    validate_immich_directory(immich_root)?;
    let (pg_client, connection) = tokio_postgres::connect(&args.postgres_url, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!(
                "{}",
                rust_i18n::t!("error.postgres_connection_error", error = e.to_string())
            );
        }
    });
    let pg_client_arc = Arc::new(pg_client);
    println!(
        "{}",
        rust_i18n::t!("main.postgres_connected", url = args.postgres_url)
    );
    if let Err(e) = database::check_database_connection(&pg_client_arc).await {
        eprintln!(
            "{}",
            rust_i18n::t!("error.database_connection_failed", error = e.to_string())
        );
        std::process::exit(1);
    }
    if args.combined {
        run_combined_mode(immich_root, args.clone(), &pg_client_arc, &final_locale).await?;
    } else if args.monitor {
        run_monitor_mode(immich_root, &args, &pg_client_arc, &final_locale).await?;
    } else {
        run_batch_mode(immich_root, &args, &pg_client_arc, &final_locale).await?;
    }
    Ok(())
}

async fn run_combined_mode(
    immich_root: &Path,
    args: Args,
    pg_client: &Arc<PgClient>,
    locale: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", rust_i18n::t!("main.combined_mode_activated"));
    let batch_handle = {
        let immich_root = immich_root.to_path_buf();
        let args = args.clone();
        let pg_client = Arc::clone(pg_client);
        let locale = locale.to_string();
        tokio::spawn(async move {
            println!("{}", rust_i18n::t!("main.processing_existing_images"));
            if let Err(e) = run_batch_mode(&immich_root, &args, &pg_client, &locale).await {
                eprintln!(
                    "{}",
                    rust_i18n::t!("error.batch_mode_failed", error = e.to_string())
                );
            }
            println!("{}", rust_i18n::t!("main.batch_mode_completed"));
        })
    };
    println!(
        "{}",
        rust_i18n::t!("main.monitor_mode_started_in_background")
    );
    run_monitor_mode(immich_root, &args, pg_client, locale).await?;
    let _ = batch_handle.await;
    Ok(())
}

async fn run_monitor_mode(
    immich_root: &Path,
    args: &Args,
    pg_client: &Arc<PgClient>,
    locale: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", rust_i18n::t!("main.monitor_mode_activated"));
    if args.ignore_existing {
        println!("{}", rust_i18n::t!("main.ignore_existing_enabled"));
    }
    let monitor_config = MonitorConfig {
        file_write_timeout: args.file_write_timeout,
        file_check_interval: args.file_check_interval,
        event_cooldown: args.event_cooldown,
        timeout: args.timeout,
        lang: locale.to_string(),
        ignore_existing: args.ignore_existing,
        hosts: args.hosts.clone(),
        interface: args.interface.clone(),
        api_key: args.api_key.clone(),
        unavailable_duration: args.unavailable_duration,
    };
    monitor_folder(
        immich_root,
        &args.model_name,
        Arc::clone(pg_client),
        &args.prompt,
        &monitor_config,
    )
    .await?;
    Ok(())
}

async fn run_batch_mode(
    immich_root: &Path,
    args: &Args,
    pg_client: &Arc<PgClient>,
    locale: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        rust_i18n::t!(
            "main.database_connected",
            path = "Immich PostgreSQL database"
        )
    );
    let preview_files = get_immich_preview_files(immich_root)?;
    handle_no_files(&preview_files, args.ignore_existing, immich_root)?;
    println!(
        "{}",
        rust_i18n::t!(
            "main.images_to_process",
            count = preview_files.len().to_string()
        )
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
    if args.ignore_existing {
        println!("{}", rust_i18n::t!("main.ignore_existing_enabled"));
    }
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(args.timeout))
        .build()?;
    let progress = Arc::new(tokio::sync::Mutex::new(SimpleProgress::new(
        preview_files.len() as u64,
        &rust_i18n::t!("progress.processing_complete"),
    )));
    let results = process_files_concurrently(
        preview_files,
        &http_client,
        pg_client,
        args,
        locale,
        progress,
    )
    .await;
    file_processing::display_results(&results, args.max_concurrent > 1)?;
    Ok(())
}
