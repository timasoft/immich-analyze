use crate::{
    config::ProcessingContext,
    data_access::DataAccess,
    database::ImageAnalysisResult,
    error::ImageAnalysisError,
    host_manager::HostManager,
    immich_api::AssetRef,
    progress::SimpleProgress,
    prompt_enricher::enrich_prompt_if_needed,
    utils::{
        build_final_description, check_overwrite_policy, extract_uuid_from_preview_filename,
        is_preview_filename,
    },
};
use futures::stream::{self, StreamExt as _};
use log::{error, warn};
use reqwest::Client;
use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

/// Get all preview image files from Immich thumbs directory.
///
/// This function is used in database mode to scan the filesystem for preview files.
pub async fn get_immich_preview_files(
    immich_root: &Path,
) -> Result<Vec<PathBuf>, ImageAnalysisError> {
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
        match tokio::fs::read_dir(&current_dir).await {
            Ok(mut entries) => {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.is_file()
                        && let Some(filename) = path.file_name().and_then(|name| name.to_str())
                        && is_preview_filename(filename)
                    {
                        preview_files.push(path);
                    }
                }
            }
            Err(err) => {
                error!("Error reading directory {}: {}", current_dir.display(), err);
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
        .to_owned();
    let asset_id = extract_uuid_from_preview_filename(&filename)?;

    let existing_description =
        check_overwrite_policy(ctx.data_access, &asset_id, &filename, ctx.overwrite_policy).await?;

    process_file(ctx, path, existing_description).await
}

async fn process_file(
    ctx: &ProcessingContext<'_>,
    path: &Path,
    existing_description: Option<String>,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    let data_access = ctx.data_access;

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_owned();

    let asset_id = extract_uuid_from_preview_filename(&filename)?;

    let preview_path = data_access.get_preview_path(&asset_id).await?;
    let final_prompt = enrich_prompt_if_needed(ctx, &asset_id)
        .await
        .unwrap_or_else(|| ctx.prompt.to_owned());

    let analysis = ctx
        .host_manager
        .analyze_image(&preview_path, &final_prompt)
        .await?;

    if let Err(err) = data_access.cleanup_preview(&preview_path).await {
        warn!("Failed to cleanup preview: {err}");
    }

    let final_description = build_final_description(
        &analysis,
        data_access,
        ctx.preserve_human,
        existing_description,
    )
    .await?;

    data_access
        .update_description(&analysis.asset_id, &final_description)
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
    // Create host manager once for all files to preserve unavailable host state
    let unavailable_duration = Duration::from_secs(args.unavailable_duration);

    let host_manager = Arc::new(HostManager::new(
        args.hosts.clone(),
        args.interface,
        http_client.clone(),
        args.model_name.clone(),
        args.timeout,
        NonZeroU32::new(args.max_retries),
        Duration::from_secs(args.retry_delay_seconds),
        unavailable_duration,
        args.api_key.clone(),
    ));

    stream::iter(assets.into_iter().map(|asset| {
        let prompt = args.prompt.clone();
        let progress_clone = Arc::clone(&progress);
        let lang = locale.to_owned();
        let overwrite_policy = args.effective_overwrite_policy();
        let asset_id = asset.id;
        let host_manager_clone = Arc::clone(&host_manager);

        async move {
            rust_i18n::set_locale(&lang);
            let preview_path = match data_access.get_preview_path(&asset_id).await {
                Ok(preview_path) => preview_path,
                Err(err) => {
                    let filename = asset_id.to_string();
                    progress_clone
                        .lock()
                        .await
                        .set_message(&rust_i18n::t!("progress.error", filename = filename));

                    progress_clone
                        .lock()
                        .await
                        .set_message_and_inc(&rust_i18n::t!("progress.error", filename = filename));

                    return (filename, Err(err));
                }
            };
            let filename = preview_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_owned();
            progress_clone
                .lock()
                .await
                .set_message(&rust_i18n::t!("progress.processing", filename = filename));

            let ctx = ProcessingContext {
                data_access,
                prompt: &prompt,
                host_manager: &host_manager_clone,
                overwrite_policy,
                enrich_prompt: args.enrich_prompt,
                preserve_human: args.preserve_human,
            };

            let result = process_file_with_existing_check(&ctx, &preview_path).await;
            progress_clone
                .lock()
                .await
                .set_message_and_inc(&rust_i18n::t!("progress.finished", filename = filename));
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
) {
    println!("{}", rust_i18n::t!("main.analysis_results"));
    println!("{}", "-".repeat(31));
    let mut successful = 0_u32;
    let mut failed = 0_u32;
    let mut skipped = 0_u32;
    let mut output_lines = Vec::new();
    for (filename, result) in results {
        match result {
            Ok(analysis) => {
                successful = successful.saturating_add(1);
                output_lines.push(format!(
                    "{} [{}] {}\n{}",
                    rust_i18n::t!("status.success"),
                    filename,
                    analysis.description,
                    "-".repeat(80)
                ));
            }
            Err(err) => {
                let (count_increment, line) = handle_error_result(filename, err);
                match count_increment {
                    "success" => successful = successful.saturating_add(1),
                    "failed" => failed = failed.saturating_add(1),
                    "skipped" => skipped = skipped.saturating_add(1),
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
        println!("{line}");
    }
    print_statistics(successful, failed, skipped);
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
                rust_i18n::t!("error.invalid_uuid_filename", filename = filename),
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
        _ => (
            "failed",
            format!(
                "{} [{}] {}\n{}",
                rust_i18n::t!("status.error"),
                filename,
                error.user_message(),
                "-".repeat(80)
            ),
        ),
    }
}

fn print_statistics(successful: u32, failed: u32, skipped: u32) {
    #[expect(clippy::arithmetic_side_effects)]
    let total = u64::from(successful) + u64::from(failed) + u64::from(skipped);
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
        print_error_recommendations();
    }
}

fn print_error_recommendations() {
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
}
