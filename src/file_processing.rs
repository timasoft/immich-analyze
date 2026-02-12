use crate::{
    args::Interface,
    database::{ImageAnalysisResult, asset_has_description, update_or_create_asset_description},
    error::ImageAnalysisError,
    llamacpp::{LlamaCppHostManager, analyze_image as llamacpp_analyze_image},
    ollama::{OllamaHostManager, analyze_image as ollama_analyze_image},
    progress::SimpleProgress,
    utils::extract_uuid_from_preview_filename,
};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;
use tokio_postgres::Client as PgClient;

/// Get all preview image files from Immich thumbs directory using std::fs
pub fn get_immich_preview_files(immich_root: &Path) -> Result<Vec<PathBuf>, ImageAnalysisError> {
    let thumbs_dir = immich_root.join("thumbs");
    if !thumbs_dir.exists() {
        return Err(ImageAnalysisError::InvalidImmichStructure {
            error: format!(
                "{}",
                rust_i18n::t!(
                    "error.thumbs_directory_not_found",
                    path = thumbs_dir.display().to_string()
                )
            ),
        });
    }
    if !thumbs_dir.is_dir() {
        return Err(ImageAnalysisError::InvalidImmichStructure {
            error: format!(
                "{}",
                rust_i18n::t!(
                    "error.thumbs_path_not_directory",
                    path = thumbs_dir.display().to_string()
                )
            ),
        });
    }
    let mut preview_files = Vec::new();
    let mut stack = vec![thumbs_dir];
    while let Some(current_dir) = stack.pop() {
        match std::fs::read_dir(&current_dir) {
            Ok(entries) => {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.is_file() {
                        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                            if filename.contains("-preview.") {
                                preview_files.push(path);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    rust_i18n::t!(
                        "error.reading_directory",
                        path = current_dir.display().to_string(),
                        error = e.to_string()
                    )
                );
            }
        }
    }
    Ok(preview_files)
}

pub fn handle_no_files(
    files: &[PathBuf],
    ignore_existing: bool,
    immich_root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if files.is_empty() {
        if ignore_existing {
            println!(
                "{}",
                rust_i18n::t!(
                    "main.no_new_images_found_ignore_existing",
                    path = immich_root.display().to_string()
                )
            );
        } else {
            println!(
                "{}",
                rust_i18n::t!(
                    "main.no_new_images_found",
                    path = immich_root.display().to_string()
                )
            );
        }
        println!("{}", rust_i18n::t!("main.monitor_hint"));
        std::process::exit(0);
    }
    Ok(())
}

async fn process_file_with_existing_check(
    http_client: &Client,
    pg_client: &PgClient,
    path: &Path,
    model_name: &str,
    prompt: &str,
    timeout: u64,
    interface: &Interface,
    hosts: &[String],
    api_key: &Option<String>,
    unavailable_duration: u64,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    let asset_id = extract_uuid_from_preview_filename(&filename)?;
    if asset_has_description(pg_client, asset_id).await? {
        return Err(ImageAnalysisError::AlreadyProcessed { filename });
    }
    process_file(
        http_client,
        pg_client,
        path,
        model_name,
        prompt,
        timeout,
        interface,
        hosts,
        api_key,
        unavailable_duration,
    )
    .await
}

async fn process_file(
    http_client: &Client,
    pg_client: &PgClient,
    path: &Path,
    model_name: &str,
    prompt: &str,
    timeout: u64,
    interface: &Interface,
    hosts: &[String],
    api_key: &Option<String>,
    unavailable_duration: u64,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    match extract_uuid_from_preview_filename(
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown"),
    ) {
        Ok(_asset_id) => {
            let analysis = match interface {
                Interface::Ollama => {
                    let host_manager = OllamaHostManager::new(
                        hosts.to_vec(),
                        std::time::Duration::from_secs(unavailable_duration),
                    );
                    ollama_analyze_image(http_client, path, model_name, prompt, timeout, &host_manager).await?
                }
                Interface::Llamacpp => {
                    let host_manager = LlamaCppHostManager::new(
                        hosts.to_vec(),
                        api_key.clone(),
                        std::time::Duration::from_secs(unavailable_duration),
                    );
                    llamacpp_analyze_image(http_client, path, model_name, prompt, timeout, &host_manager).await?
                }
            };
            update_or_create_asset_description(pg_client, analysis.asset_id, &analysis.description)
                .await?;
            Ok(analysis)
        }
        Err(e) => Err(e),
    }
}

pub async fn process_files_concurrently(
    files: Vec<PathBuf>,
    http_client: &Client,
    pg_client: &Arc<PgClient>,
    args: &crate::args::Args,
    locale: &str,
    progress: Arc<Mutex<SimpleProgress>>,
) -> Vec<(String, Result<ImageAnalysisResult, ImageAnalysisError>)> {
    stream::iter(files.into_iter().map(|path| {
        let http_client = http_client.clone();
        let pg_client = Arc::clone(pg_client);
        let model_name = args.model_name.clone();
        let prompt = args.prompt.clone();
        let progress = Arc::clone(&progress);
        let lang = locale.to_string();
        let ignore_existing = args.ignore_existing;
        let path_clone = path.clone();
        let timeout = args.timeout;
        let interface = args.interface.clone();
        let hosts = args.hosts.clone();
        let api_key = args.api_key.clone();
        let unavailable_duration = args.unavailable_duration;
        async move {
            rust_i18n::set_locale(&lang);
            let filename = path_clone
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            {
                let mut progress_guard = progress.lock().await;
                progress_guard
                    .set_message(&rust_i18n::t!("progress.processing", filename = filename));
            }
            let result = if ignore_existing {
                process_file(
                    &http_client,
                    &pg_client,
                    &path_clone,
                    &model_name,
                    &prompt,
                    timeout,
                    &interface,
                    &hosts,
                    &api_key,
                    unavailable_duration,
                )
                .await
            } else {
                process_file_with_existing_check(
                    &http_client,
                    &pg_client,
                    &path_clone,
                    &model_name,
                    &prompt,
                    timeout,
                    &interface,
                    &hosts,
                    &api_key,
                    unavailable_duration,
                )
                .await
            };
            {
                let mut progress_guard = progress.lock().await;
                progress_guard
                    .set_message_and_inc(&rust_i18n::t!("progress.finished", filename = filename));
            }
            (filename, result)
        }
    }))
    .buffer_unordered(args.max_concurrent)
    .collect::<Vec<_>>()
    .await
}

pub fn display_results(
    results: &[(String, Result<ImageAnalysisResult, ImageAnalysisError>)],
    use_sorting: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", rust_i18n::t!("main.analysis_results"));
    println!("{}", "-".repeat(31));
    let mut successful = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut output_lines = Vec::new();
    for (filename, result) in results {
        match result {
            Ok(analysis) => {
                successful += 1;
                output_lines.push(format!(
                    "{} [{}] {}\n{}",
                    rust_i18n::t!("status.success"),
                    filename,
                    analysis.description,
                    "-".repeat(80)
                ));
            }
            Err(e) => {
                let (count_increment, line) = handle_error_result(filename, e);
                match count_increment {
                    "success" => successful += 1,
                    "failed" => failed += 1,
                    "skipped" => skipped += 1,
                    _ => {}
                }
                output_lines.push(line);
            }
        }
    }
    if use_sorting {
        output_lines.sort();
    }
    for line in output_lines {
        println!("{}", line);
    }
    print_statistics(successful, failed, skipped)?;
    Ok(())
}

fn handle_error_result(filename: &str, error: &ImageAnalysisError) -> (&'static str, String) {
    match error {
        ImageAnalysisError::AlreadyProcessed { filename } => (
            "skipped",
            format!(
                "{} [{}] {}\n{}",
                rust_i18n::t!("status.skipped"),
                filename,
                rust_i18n::t!("main.file_already_in_database", filename = filename),
                "-".repeat(80)
            ),
        ),
        ImageAnalysisError::InvalidUuid { filename } => (
            "skipped",
            format!(
                "{} [{}] {}\n{}",
                rust_i18n::t!("status.skipped"),
                filename,
                rust_i18n::t!("main.invalid_uuid_filename", filename = filename),
                "-".repeat(80)
            ),
        ),
        ImageAnalysisError::InvalidImmichStructure { error } => (
            "failed",
            format!(
                "{} [{}] {}\n{}",
                rust_i18n::t!("status.error"),
                filename,
                rust_i18n::t!("error.invalid_immich_structure", error = error),
                "-".repeat(80)
            ),
        ),
        _ => {
            let error_message = format_error_message(error, filename);
            (
                "failed",
                format!(
                    "{} [{}] {}\n{}",
                    rust_i18n::t!("status.error"),
                    filename,
                    error_message,
                    "-".repeat(80)
                ),
            )
        }
    }
}

fn format_error_message(error: &ImageAnalysisError, filename: &str) -> String {
    match error {
        ImageAnalysisError::EmptyFile { filename } => {
            rust_i18n::t!("error.empty_file", filename = filename).to_string()
        }
        ImageAnalysisError::HttpError {
            filename,
            status,
            response,
        } => rust_i18n::t!(
            "error.http_error_with_details",
            filename = filename,
            status = status.to_string(),
            response = response
        )
        .to_string(),
        ImageAnalysisError::EmptyResponse { filename } => {
            rust_i18n::t!("error.empty_response", filename = filename).to_string()
        }
        ImageAnalysisError::JsonParsing { filename, error } => rust_i18n::t!(
            "error.json_parsing_with_details",
            filename = filename,
            error = error
        )
        .to_string(),
        ImageAnalysisError::FileWriteTimeout { filename, timeout } => rust_i18n::t!(
            "error.file_write_timeout_with_details",
            filename = filename,
            timeout = timeout.to_string()
        )
        .to_string(),
        ImageAnalysisError::DatabaseError { error } => {
            rust_i18n::t!("error.database_error", error = error).to_string()
        }
        ImageAnalysisError::AllHostsUnavailable => {
            rust_i18n::t!("error.all_ollama_hosts_unavailable").to_string()
        }
        ImageAnalysisError::OllamaRequestTimeout => {
            rust_i18n::t!("error.ollama_request_timeout").to_string()
        }
        _ => rust_i18n::t!("error.critical_processing_error", filename = filename).to_string(),
    }
}

fn print_statistics(
    successful: u32,
    failed: u32,
    skipped: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let total = successful + failed + skipped;
    println!("{}", rust_i18n::t!("main.statistics"));
    println!(
        "{}",
        rust_i18n::t!("main.successful", count = successful.to_string())
    );
    println!(
        "{}",
        rust_i18n::t!("main.failed", count = failed.to_string())
    );
    if skipped > 0 {
        println!(
            "{}",
            rust_i18n::t!("main.skipped", count = skipped.to_string())
        );
    }
    println!(
        "{}",
        rust_i18n::t!("main.total_processed", count = total.to_string())
    );
    println!("{}", rust_i18n::t!("main.database_updates_complete"));
    if failed > 0 {
        print_error_recommendations()?;
    }
    Ok(())
}

fn print_error_recommendations() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", rust_i18n::t!("main.error_recommendations"));
    println!("• {}", rust_i18n::t!("recommendation.check_ollama_status"));
    println!("• {}", rust_i18n::t!("recommendation.check_file_sizes"));
    println!("• {}", rust_i18n::t!("recommendation.reduce_concurrency"));
    println!("• {}", rust_i18n::t!("recommendation.use_monitor_mode"));
    println!(
        "• {}",
        rust_i18n::t!("recommendation.check_database_connection")
    );
    println!(
        "• {}",
        rust_i18n::t!("recommendation.check_immich_structure")
    );
    println!("• {}", rust_i18n::t!("recommendation.check_ollama_servers"));
    Ok(())
}
