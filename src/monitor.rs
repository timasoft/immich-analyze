use crate::{
    config::{MonitorConfig, ProcessingContext},
    error::ImageAnalysisError,
    health::mark_activity,
    host_manager::{HostManager, ImageAnalysisResult},
    immich_api::ImmichApiProvider,
    prompt_enricher::enrich_prompt_if_needed,
    utils::{
        OverwriteDecision, blocked_marker_text, build_final_description, check_overwrite_policy,
    },
};
use bytes::Bytes;
use log::error;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::mpsc as tokio_mpsc,
    time::MissedTickBehavior,
};
use uuid::Uuid;

/// Process a downloaded thumbnail and write the AI description back to the asset.
pub async fn process_new_asset(
    ctx: &ProcessingContext<'_>,
    image_data: Bytes,
    asset_id: Uuid,
) -> Result<(), ImageAnalysisError> {
    println!(
        "{}",
        rust_i18n::t!("monitor.asset_detected", asset_id = asset_id)
    );

    let existing_description = match check_overwrite_policy(
        ctx.immich_api_provider,
        &asset_id,
        ctx.overwrite_policy,
    )
    .await
    {
        Ok(OverwriteDecision::Skip) => {
            println!(
                "{}",
                rust_i18n::t!("monitor.asset_already_described", asset_id = asset_id)
            );
            return Ok(());
        }
        Ok(OverwriteDecision::SkipBlocked { reason }) => {
            println!(
                "{}",
                rust_i18n::t!(
                    "error.permanently_rejected",
                    asset_id = asset_id,
                    reason = reason
                )
            );
            return Ok(());
        }
        Ok(OverwriteDecision::AnalyzeFresh) => None,
        Ok(OverwriteDecision::PreserveExisting(desc)) => Some(desc),
        Err(err) => return Err(err),
    };

    let final_prompt = enrich_prompt_if_needed(ctx, &asset_id)
        .await
        .unwrap_or_else(|| ctx.prompt.to_owned());

    let (analysis, rejected) = match ctx
        .host_manager
        .analyze_image(image_data, &final_prompt, asset_id)
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
        Err(err) => {
            eprintln!("{}", err.user_message());
            return Err(err);
        }
    };

    if !rejected {
        println!(
            "{}",
            rust_i18n::t!("monitor.processing_success", asset_id = asset_id)
        );
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
        println!(
            "{}",
            rust_i18n::t!(
                "error.permanently_rejected",
                asset_id = asset_id,
                reason = analysis.description
            )
        );
    } else {
        println!(
            "{}",
            rust_i18n::t!("monitor.description_updated", asset_id = asset_id)
        );
    }

    if rejected {
        Err(ImageAnalysisError::PermanentlyRejected {
            asset_id,
            reason: analysis.description,
        })
    } else {
        Ok(())
    }
}

/// Monitor for new assets by polling the Immich API.
pub async fn monitor_folder(
    immich_api_provider: Arc<ImmichApiProvider>,
    prompt: &str,
    config: &MonitorConfig,
    host_manager: Arc<HostManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    rust_i18n::set_locale(&config.lang);

    let (stop_tx, mut stop_rx) = tokio_mpsc::channel(1);
    // Handle CTRL-C signal
    tokio::spawn({
        let lang_clone = config.lang.clone();
        async move {
            rust_i18n::set_locale(&lang_clone);
            let mut sigterm =
                signal(SignalKind::terminate()).expect("Failed to set up SIGTERM handler");
            let mut sigint =
                signal(SignalKind::interrupt()).expect("Failed to set up SIGINT handler");
            tokio::select! {
                _ = sigterm.recv() => {
                    println!("{}", rust_i18n::t!("monitor.stop_signal_received", signal = "SIGTERM"));
                }
                _ = sigint.recv() => {
                    println!("{}", rust_i18n::t!("monitor.stop_signal_received", signal = "SIGINT"));
                }
            }
            let _: Result<(), tokio_mpsc::error::SendError<()>> = stop_tx.send(()).await;
        }
    });

    let bg_ctx = BackgroundCtx {
        immich_api_provider,
        prompt: prompt.to_owned(),
        host_manager,
        processing_assets: Arc::new(Mutex::new(HashSet::<Uuid>::new())),
    };

    println!("{}", rust_i18n::t!("monitor.api_monitoring_started"));
    println!("{}", rust_i18n::t!("monitor.stop_instructions"));

    let mut known_assets: HashSet<Uuid> = HashSet::with_capacity(1 << 16);
    let mut poll_interval =
        tokio::time::interval(Duration::from_secs(u64::from(config.api_poll_interval)));
    poll_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut is_first_poll = true;
    let mut last_poll_time: Option<chrono::DateTime<chrono::Utc>> = None;

    loop {
        tokio::select! {
            Some(()) = stop_rx.recv() => {
                println!("{}", rust_i18n::t!("monitor.stopping_monitoring"));
                return Ok(());
            }
            _ = poll_interval.tick() => {
                mark_activity();
                handle_api_poll(
                    &mut known_assets,
                    &mut is_first_poll,
                    &mut last_poll_time,
                    config,
                    &bg_ctx,
                )
                .await;
            }
        }
    }
}

#[derive(Clone)]
struct BackgroundCtx {
    immich_api_provider: Arc<ImmichApiProvider>,
    prompt: String,
    host_manager: Arc<HostManager>,
    processing_assets: Arc<Mutex<HashSet<Uuid>>>,
}

async fn handle_api_poll(
    known_assets: &mut HashSet<Uuid>,
    is_first_poll: &mut bool,
    last_poll_time: &mut Option<chrono::DateTime<chrono::Utc>>,
    config: &MonitorConfig,
    bg_ctx: &BackgroundCtx,
) {
    let assets_result = if *is_first_poll {
        bg_ctx.immich_api_provider.get_assets().await
    } else {
        #[expect(clippy::arithmetic_side_effects)]
        let buffer_secs = i64::from(config.api_poll_interval) * 2;
        let since_time = last_poll_time
            .unwrap_or_else(chrono::Utc::now)
            .checked_sub_signed(chrono::Duration::seconds(buffer_secs))
            .unwrap_or_else(chrono::Utc::now);
        let since_iso = since_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        bg_ctx
            .immich_api_provider
            .get_assets_since_timestamp(&since_iso)
            .await
    };

    match assets_result {
        Ok(assets) => {
            if *is_first_poll {
                for asset in &assets {
                    known_assets.insert(asset.id);
                }
                *is_first_poll = false;
                *last_poll_time = Some(chrono::Utc::now());
                log::info!(
                    "Initial sync complete: {} assets indexed, none processed",
                    assets.len()
                );
            } else {
                for asset in assets {
                    if known_assets.contains(&asset.id) {
                        continue;
                    }
                    {
                        let processing = bg_ctx
                            .processing_assets
                            .lock()
                            .expect("Failed to lock processing assets");
                        if processing.contains(&asset.id) {
                            continue;
                        }
                    }

                    known_assets.insert(asset.id);
                    {
                        let mut processing = bg_ctx
                            .processing_assets
                            .lock()
                            .expect("Failed to lock processing assets");
                        processing.insert(asset.id);
                    }

                    println!(
                        "{}",
                        rust_i18n::t!("monitor.api_asset_queued", asset_id = asset.id.to_string())
                    );

                    let bg_ctx_clone = bg_ctx.clone();
                    let asset_id = asset.id;
                    let config_clone = config.clone();

                    tokio::spawn(async move {
                        rust_i18n::set_locale(&config_clone.lang);

                        let thumbnail_data = match bg_ctx_clone
                            .immich_api_provider
                            .get_thumbnail_bytes(&asset_id, config_clone.thumbnail_size)
                            .await
                        {
                            Ok(data) => data,
                            Err(err) => {
                                error!("Failed to get thumbnail for asset {asset_id}: {err}");
                                bg_ctx_clone
                                    .processing_assets
                                    .lock()
                                    .expect("Failed to lock processing assets")
                                    .remove(&asset_id);
                                return;
                            }
                        };

                        let ctx = ProcessingContext::new(
                            &bg_ctx_clone.immich_api_provider,
                            &bg_ctx_clone.prompt,
                            &bg_ctx_clone.host_manager,
                            config_clone.overwrite_policy,
                            config_clone.enrich_prompt,
                            config_clone.preserve_human,
                            config_clone.disable_ai_wrapper,
                        );

                        let result = process_new_asset(&ctx, thumbnail_data, asset_id).await;

                        {
                            let mut processing = bg_ctx_clone
                                .processing_assets
                                .lock()
                                .expect("Failed to lock processing assets");
                            processing.remove(&asset_id);
                        }

                        if let Err(err) = result
                            && !err.is_silent_background_error()
                        {
                            error!(
                                "Background processing error for: {asset_id}: {}",
                                err.user_message()
                            );
                        }
                    });
                }
                *last_poll_time = Some(chrono::Utc::now());
            }
        }
        Err(err) => {
            error!("API polling failed: {err}");
        }
    }
}
