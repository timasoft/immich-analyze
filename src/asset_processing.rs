use crate::{
    config::ProcessingContext,
    error::ImageAnalysisError,
    health::mark_activity,
    host_manager::{HostManager, ImageAnalysisResult},
    immich_api::{AssetRef, ImmichApiProvider},
    progress::SimpleProgress,
    prompt_enricher::enrich_prompt_if_needed,
    utils::{
        OverwriteDecision, blocked_marker_text, build_final_description, check_overwrite_policy,
        cleanup_thumbnail,
    },
};
use futures::stream::{self, StreamExt as _};
use log::warn;
use std::{path::Path, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

async fn process_asset_with_existing_check(
    ctx: &ProcessingContext<'_>,
    path: &Path,
    asset_id: Uuid,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    match check_overwrite_policy(ctx.immich_api_provider, &asset_id, ctx.overwrite_policy).await? {
        OverwriteDecision::Skip => Err(ImageAnalysisError::AlreadyProcessed { asset_id }),
        OverwriteDecision::SkipBlocked { reason } => {
            Err(ImageAnalysisError::PermanentlyRejected { asset_id, reason })
        }
        OverwriteDecision::AnalyzeFresh => process_asset(ctx, path, asset_id, None).await,
        OverwriteDecision::PreserveExisting(desc) => {
            process_asset(ctx, path, asset_id, Some(desc)).await
        }
    }
}

async fn process_asset(
    ctx: &ProcessingContext<'_>,
    path: &Path,
    asset_id: Uuid,
    existing_description: Option<String>,
) -> Result<ImageAnalysisResult, ImageAnalysisError> {
    let final_prompt = enrich_prompt_if_needed(ctx, &asset_id)
        .await
        .unwrap_or_else(|| ctx.prompt.to_owned());

    let (analysis, rejected) = match ctx
        .host_manager
        .analyze_image(path, &final_prompt, asset_id)
        .await
    {
        Ok(analysis) => (analysis, false),
        Err(ImageAnalysisError::ProviderRejected {
            status, message, ..
        }) => (
            ImageAnalysisResult {
                asset_id,
                description: blocked_marker_text(status, &message),
            },
            true,
        ),
        Err(err) => return Err(err),
    };

    if let Err(err) = cleanup_thumbnail(path).await {
        warn!("Failed to cleanup thumbnail: {err}");
    }

    let final_description = build_final_description(
        &analysis,
        ctx.immich_api_provider,
        ctx.preserve_human,
        existing_description,
        ctx.disable_ai_wrapper,
    )
    .await?;

    ctx.immich_api_provider
        .update_description(&analysis.asset_id, &final_description)
        .await?;

    if rejected {
        return Err(ImageAnalysisError::PermanentlyRejected {
            asset_id: analysis.asset_id,
            reason: analysis.description,
        });
    }

    Ok(analysis)
}

pub async fn process_assets_concurrently(
    assets: Vec<AssetRef>,
    immich_api_provider: &ImmichApiProvider,
    args: &crate::args::Args,
    locale: &str,
    progress: Arc<Mutex<SimpleProgress>>,
    host_manager: Arc<HostManager>,
) -> Vec<(String, Result<ImageAnalysisResult, ImageAnalysisError>)> {
    stream::iter(assets.into_iter().map(|asset| {
        let prompt = args.prompt.clone();
        let progress_clone = Arc::clone(&progress);
        let lang = locale.to_owned();
        let overwrite_policy = args.effective_overwrite_policy();
        let asset_id = asset.id;
        let host_manager_clone = Arc::clone(&host_manager);

        async move {
            rust_i18n::set_locale(&lang);
            mark_activity();
            let thumbnail_path = match immich_api_provider
                .clone()
                .get_thumbnail_path(&asset_id, args.thumbnail_size)
                .await
            {
                Ok(thumbnail_path) => thumbnail_path,
                Err(err) => {
                    let failed_asset_id = asset_id.to_string();
                    progress_clone
                        .lock()
                        .await
                        .set_message_and_inc(&rust_i18n::t!(
                            "progress.error",
                            asset_id = failed_asset_id,
                            error = err.user_message()
                        ));

                    return (failed_asset_id, Err(err));
                }
            };
            progress_clone
                .lock()
                .await
                .set_message(&rust_i18n::t!("progress.processing", asset_id = asset_id));

            let ctx = ProcessingContext::new(
                immich_api_provider,
                &prompt,
                &host_manager_clone,
                overwrite_policy,
                args.enrich_prompt,
                args.preserve_human,
                args.disable_ai_wrapper,
            );

            let result = process_asset_with_existing_check(&ctx, &thumbnail_path, asset_id).await;
            match &result {
                Err(
                    ImageAnalysisError::AlreadyProcessed { .. }
                    | ImageAnalysisError::AssetNotFound { .. }
                    | ImageAnalysisError::PermanentlyRejected { .. },
                ) => {
                    progress_clone
                        .lock()
                        .await
                        .set_message_and_dec_total(&rust_i18n::t!(
                            "progress.skipped",
                            asset_id = asset_id
                        ));
                }
                _ => {
                    progress_clone
                        .lock()
                        .await
                        .set_message_and_inc(&rust_i18n::t!(
                            "progress.finished",
                            asset_id = asset_id
                        ));
                }
            }
            (asset_id.to_string(), result)
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
    let mut blocked = 0_u32;
    let mut output_lines = Vec::new();
    for (asset_id, result) in results {
        match result {
            Ok(analysis) => {
                successful = successful.saturating_add(1);
                output_lines.push(format!(
                    "{} [{}] {}\n{}",
                    rust_i18n::t!("status.success"),
                    asset_id,
                    analysis.description,
                    "-".repeat(80)
                ));
            }
            Err(err) => {
                let (count_increment, line) = handle_error_result(asset_id, err);
                match count_increment {
                    "success" => successful = successful.saturating_add(1),
                    "failed" => failed = failed.saturating_add(1),
                    "skipped" => skipped = skipped.saturating_add(1),
                    "blocked" => blocked = blocked.saturating_add(1),
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
    print_statistics(successful, failed, skipped, blocked);
}

fn handle_error_result(asset_id: &str, error: &ImageAnalysisError) -> (&'static str, String) {
    match error {
        ImageAnalysisError::AlreadyProcessed { .. } => (
            "skipped",
            format!(
                "{} [{}] {}\n{}",
                rust_i18n::t!("status.skipped"),
                asset_id,
                rust_i18n::t!("main.asset_already_described", asset_id = asset_id),
                "-".repeat(80)
            ),
        ),
        ImageAnalysisError::PermanentlyRejected { reason, .. } => (
            "blocked",
            format!(
                "{} [{}] {}\n{}",
                rust_i18n::t!("status.blocked"),
                asset_id,
                rust_i18n::t!(
                    "error.permanently_rejected",
                    asset_id = asset_id,
                    reason = reason
                ),
                "-".repeat(80)
            ),
        ),
        ImageAnalysisError::AssetNotFound { .. } => (
            "skipped",
            format!(
                "{} [{}] {}\n{}",
                rust_i18n::t!("status.skipped"),
                asset_id,
                rust_i18n::t!("error.asset_not_found_in_library", asset_id = asset_id),
                "-".repeat(80)
            ),
        ),
        _ => (
            "failed",
            format!(
                "{} [{}] {}\n{}",
                rust_i18n::t!("status.error"),
                asset_id,
                error.user_message(),
                "-".repeat(80)
            ),
        ),
    }
}

fn print_statistics(successful: u32, failed: u32, skipped: u32, blocked: u32) {
    #[expect(clippy::arithmetic_side_effects)]
    let total = u64::from(successful) + u64::from(failed) + u64::from(skipped) + u64::from(blocked);
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
    if blocked > 0 {
        println!(
            "{}",
            rust_i18n::t!("main.blocked", count = blocked.to_string())
        );
        println!("{}", rust_i18n::t!("main.blocked_hint"));
    }
    println!(
        "{}",
        rust_i18n::t!("main.total_processed", count = total.to_string())
    );
    println!("{}", rust_i18n::t!("main.asset_descriptions_updated"));
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
    println!("• {}", rust_i18n::t!("recommendation.check_ai_servers"));
}
