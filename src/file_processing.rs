use crate::{
    args::Interface,
    config::ProcessingContext,
    data_access::DataAccess,
    database::ImageAnalysisResult,
    error::ImageAnalysisError,
    immich_api::AssetRef,
    llamacpp::{LlamaCppHostManager, analyze_image as llamacpp_analyze_image},
    ollama::{OllamaHostManager, analyze_image as ollama_analyze_image},
    progress::SimpleProgress,
    utils::extract_uuid_from_preview_filename,
};
use futures::stream::{self, StreamExt};
use log::error;
use reqwest::Client;
use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

/// Get all preview image files from Immich thumbs directory using std::fs.
///
/// This function is used in database mode to scan the filesystem for preview files.
pub fn get_immich_preview_files(immich_root: &Path) -> Result<Vec<PathBuf>, ImageAnalysisError> {
    let thumbs_dir = immich_root.join("thumbs");
    if !thumbs_dir.exists() {
        return Err(ImageAnalysisError::InvalidImmichStructure {
            error: rust_i18n::t!(
                "error.thumbs_directory_not_found",
                path = thumbs_dir.display().to_string()
            )
            .to_string(),
        });
    }
    if !thumbs_dir.is_dir() {
        return Err(ImageAnalysisError::InvalidImmichStructure {
            error: rust_i18n::t!(
                "error.thumbs_path_not_directory",
                path = thumbs_dir.display().to_string()
            )
            .to_string(),
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
                    } else if path.is_file()
                        && let Some(filename) = path.file_name().and_then(|f| f.to_str())
                        && (filename.contains("_preview.") || filename.contains("-preview."))
                    {
                        preview_files.push(path);
                    }
                }
            }
            Err(e) => {
                error!(
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

async fn process_file_with_existing_check(
    ctx: &ProcessingContext<'_>,
    path: &Path,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    let asset_id = extract_uuid_from_preview_filename(&filename)?;

    if ctx.data_access.has_description(&asset_id).await? {
        return Err(ImageAnalysisError::AlreadyProcessed { filename });
    }
    process_file(ctx, path).await
}

async fn process_file(
    ctx: &ProcessingContext<'_>,
    path: &Path,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    let http_client = ctx.http_client;
    let data_access = ctx.data_access;
    let model_name = ctx.model_name;
    let prompt = ctx.prompt;
    let ollama_manager = ctx.ollama_manager;
    let llamacpp_manager = ctx.llamacpp_manager;
    let timeout = ctx.timeout;

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let asset_id = extract_uuid_from_preview_filename(&filename)?;

    let preview_path = data_access.get_preview_path(&asset_id).await?;

    let analysis = match (ollama_manager, llamacpp_manager) {
        (Some(manager), _) => {
            ollama_analyze_image(
                http_client,
                &preview_path,
                model_name,
                prompt,
                timeout,
                manager,
                ctx.max_retries,
                ctx.retry_delay,
            )
            .await?
        }
        (_, Some(manager)) => {
            llamacpp_analyze_image(
                http_client,
                &preview_path,
                model_name,
                prompt,
                timeout,
                manager,
                ctx.max_retries,
                ctx.retry_delay,
            )
            .await?
        }
        (None, None) => return Err(ImageAnalysisError::AllHostsUnavailable),
    };

    let _ = data_access.cleanup_preview(&preview_path).await;

    data_access
        .update_description(&analysis.asset_id, &analysis.description)
        .await?;

    Ok(analysis)
}

pub async fn process_files_concurrently(
    assets: Vec<AssetRef>,
    http_client: &Client,
    data_access: &DataAccess,
    args: &crate::args::Args,
    locale: &str,
    progress: Arc<Mutex<SimpleProgress>>,
) -> Vec<(String, Result<ImageAnalysisResult, ImageAnalysisError>)> {
    // Create host managers once for all files to preserve unavailable host state
    let unavailable_duration = Duration::from_secs(args.unavailable_duration);

    let ollama_manager: Option<Arc<OllamaHostManager>> = if args.interface == Interface::Ollama {
        Some(Arc::new(OllamaHostManager::new(
            args.hosts.clone(),
            unavailable_duration,
        )))
    } else {
        None
    };

    let llamacpp_manager: Option<Arc<LlamaCppHostManager>> =
        if args.interface == Interface::Llamacpp {
            Some(Arc::new(LlamaCppHostManager::new(
                args.hosts.clone(),
                args.api_key.clone(),
                unavailable_duration,
            )))
        } else {
            None
        };

    stream::iter(assets.into_iter().map(|asset| {
        let http_client = http_client.clone();
        let data_access = data_access.clone();
        let model_name = args.model_name.clone();
        let prompt = args.prompt.clone();
        let progress = Arc::clone(&progress);
        let lang = locale.to_string();
        let overwrite_existing = args.overwrite_existing;
        let asset_id = asset.id;
        let timeout = args.timeout;
        let ollama_manager = ollama_manager.clone();
        let llamacpp_manager = llamacpp_manager.clone();

        async move {
            rust_i18n::set_locale(&lang);
            let path = match data_access.get_preview_path(&asset_id).await {
                Ok(p) => p,
                Err(e) => {
                    let filename = asset_id.to_string();
                    {
                        let mut progress_guard = progress.lock().await;
                        progress_guard
                            .set_message(&rust_i18n::t!("progress.error", filename = filename));
                    }

                    {
                        let mut progress_guard = progress.lock().await;
                        progress_guard.set_message_and_inc(&rust_i18n::t!(
                            "progress.finished",
                            filename = filename
                        ));
                    }

                    return (filename, Err(e));
                }
            };
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            {
                let mut progress_guard = progress.lock().await;
                progress_guard
                    .set_message(&rust_i18n::t!("progress.processing", filename = filename));
            }

            let ctx = ProcessingContext {
                http_client: &http_client,
                data_access: &data_access,
                model_name: &model_name,
                prompt: &prompt,
                timeout,
                ollama_manager: ollama_manager.as_ref(),
                llamacpp_manager: llamacpp_manager.as_ref(),
                max_retries: NonZeroU32::new(args.max_retries),
                retry_delay: Duration::from_secs(args.retry_delay_seconds),
            };

            let result = if overwrite_existing {
                process_file(&ctx, &path).await
            } else {
                process_file_with_existing_check(&ctx, &path).await
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
            rust_i18n::t!("error.all_hosts_unavailable").to_string()
        }
        ImageAnalysisError::OllamaRequestTimeout => {
            rust_i18n::t!("error.ollama_request_timeout").to_string()
        }
        ImageAnalysisError::LlamaCppRequestTimeout => {
            rust_i18n::t!("error.llamacpp_request_timeout").to_string()
        }
        e => {
            format!(
                "{}\n{}",
                rust_i18n::t!("error.critical_processing_error", filename = filename),
                e
            )
        }
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
    println!("• {}", rust_i18n::t!("recommendation.check_service_status"));
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
    println!("• {}", rust_i18n::t!("recommendation.check_ai_servers"));
    Ok(())
}
