use crate::{
    args::{Interface, OverwritePolicy},
    config::{MonitorConfig, ProcessingContext},
    data_access::DataAccess,
    error::ImageAnalysisError,
    immich_api::ImmichApiProvider,
    llamacpp::{LlamaCppHostManager, analyze_image as llamacpp_analyze_image},
    ollama::{OllamaHostManager, analyze_image as ollama_analyze_image},
    prompt_enricher::enrich_prompt_if_needed,
    utils::{extract_uuid_from_preview_filename, get_ai_block_pattern},
};
use log::{error, warn};
use notify::{
    event::ModifyKind,
    {Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher},
};
use reqwest::Client;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::mpsc as tokio_mpsc,
    time::MissedTickBehavior,
};
use uuid::Uuid;

/// Process new file with stability checking using data_access abstraction.
pub async fn process_new_file(
    ctx: &ProcessingContext<'_>,
    preview_path: &Path,
    file_write_timeout: u64,
    file_check_interval: u64,
) -> Result<(), ImageAnalysisError> {
    let http_client = ctx.http_client;
    let data_access = ctx.data_access;
    let model_name = ctx.model_name;
    let ollama_manager = ctx.ollama_manager;
    let llamacpp_manager = ctx.llamacpp_manager;

    let filename = preview_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    println!(
        "{}",
        rust_i18n::t!("monitor.file_detected", filename = filename)
    );
    let start_time = Instant::now();
    let mut last_size = 0;
    let mut stable_count = 0;
    let timeout_duration = Duration::from_secs(file_write_timeout);
    let check_interval = Duration::from_millis(file_check_interval);
    // Wait for file to be stable
    while start_time.elapsed() < timeout_duration {
        if let Ok(metadata) = std::fs::metadata(preview_path) {
            let current_size = metadata.len();
            if current_size == last_size && current_size > 0 {
                stable_count += 1;
                if stable_count >= 3 {
                    break;
                }
            } else {
                stable_count = 0;
                last_size = current_size;
            }
        }
        tokio::time::sleep(check_interval).await;
    }
    if start_time.elapsed() >= timeout_duration {
        return Err(ImageAnalysisError::FileWriteTimeout {
            timeout: file_write_timeout,
            filename: filename.clone(),
        });
    }
    println!(
        "{}",
        rust_i18n::t!("monitor.file_stable", filename = filename)
    );
    let asset_id = extract_uuid_from_preview_filename(&filename)?;

    let existing_description = match ctx.overwrite_policy {
        OverwritePolicy::All => None,
        OverwritePolicy::None => {
            if data_access.has_description(&asset_id).await? {
                println!(
                    "{}",
                    rust_i18n::t!("monitor.file_already_in_db", filename = filename)
                );
                return Ok(());
            }
            None
        }
        OverwritePolicy::MissingAi => match data_access.get_description(&asset_id).await {
            Ok(Some(desc)) => {
                if get_ai_block_pattern().is_match(&desc) {
                    println!(
                        "{}",
                        rust_i18n::t!("monitor.file_already_in_db", filename = filename)
                    );
                    return Ok(());
                }
                Some(desc)
            }
            Ok(None) => None,
            Err(e) => return Err(e),
        },
    };

    let final_prompt = enrich_prompt_if_needed(ctx, &asset_id)
        .await
        .unwrap_or_else(|| ctx.prompt.to_string());

    let result = match (ollama_manager, llamacpp_manager) {
        (Some(manager), _) => {
            ollama_analyze_image(
                http_client,
                preview_path,
                model_name,
                &final_prompt,
                ctx.timeout,
                manager,
                ctx.max_retries,
                ctx.retry_delay,
            )
            .await
        }
        (_, Some(manager)) => {
            llamacpp_analyze_image(
                http_client,
                preview_path,
                model_name,
                &final_prompt,
                ctx.timeout,
                manager,
                ctx.max_retries,
                ctx.retry_delay,
            )
            .await
        }
        (None, None) => Err(ImageAnalysisError::AllHostsUnavailable),
    };

    match result {
        Ok(analysis) => {
            println!(
                "{}",
                rust_i18n::t!("monitor.processing_success", filename = filename)
            );

            let ai_wrapped = format!("[AI]\n{}\n[/AI]", analysis.description.trim());
            let final_description = if ctx.preserve_human {
                let existing = match existing_description {
                    Some(desc) => desc,
                    None => match data_access.get_description(&analysis.asset_id).await {
                        Ok(Some(desc)) => desc,
                        Ok(None) => ai_wrapped.clone(),
                        Err(e) => {
                            warn!(
                                "Failed to get existing description for asset {}, cannot preserve human text: {}",
                                analysis.asset_id, e
                            );
                            return Err(e);
                        }
                    },
                };

                let re = get_ai_block_pattern();
                if re.is_match(&existing) {
                    re.replace(&existing, format!("\n{}\n", ai_wrapped))
                        .trim()
                        .to_string()
                } else {
                    format!("{}\n\n{}", existing.trim(), ai_wrapped)
                }
            } else {
                ai_wrapped
            };

            data_access
                .update_description(&analysis.asset_id, &final_description)
                .await?;
            println!(
                "{}",
                rust_i18n::t!("monitor.database_updated", filename = filename)
            );
            Ok(())
        }
        Err(e) => {
            crate::utils::handle_processing_error(&e, &filename);
            Err(e)
        }
    }
}

/// Monitor for new files using data_access abstraction.
///
/// # Database mode
/// Uses filesystem watcher on thumbs/ directory.
///
/// # ImmichApi mode
/// Uses polling via get_assets_to_process() to detect new assets.
pub async fn monitor_folder(
    model_name: &str,
    data_access: DataAccess,
    prompt: &str,
    config: &MonitorConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    rust_i18n::set_locale(&config.lang);
    let http_client = Client::builder()
        .timeout(Duration::from_secs(config.timeout))
        .build()?;

    let (stop_tx, mut stop_rx) = tokio_mpsc::channel(1);
    // Handle CTRL-C signal
    tokio::spawn({
        let stop_tx = stop_tx.clone();
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
            let _ = stop_tx.send(()).await;
        }
    });

    let unavailable_duration = Duration::from_secs(config.unavailable_duration);
    let ollama_manager: Option<Arc<OllamaHostManager>> = if config.interface == Interface::Ollama {
        Some(Arc::new(OllamaHostManager::new(
            config.hosts.clone(),
            unavailable_duration,
        )))
    } else {
        None
    };
    let llamacpp_manager: Option<Arc<LlamaCppHostManager>> =
        if config.interface == Interface::Llamacpp {
            Some(Arc::new(LlamaCppHostManager::new(
                config.hosts.clone(),
                config.api_key.clone(),
                unavailable_duration,
            )))
        } else {
            None
        };

    let bg_ctx = BackgroundCtx {
        http_client,
        data_access: data_access.clone(),
        model_name: model_name.to_string(),
        prompt: prompt.to_string(),
        ollama_manager,
        llamacpp_manager,
    };

    match &data_access {
        // ========== DATABASE MODE: filesystem monitoring ==========
        DataAccess::Database { immich_root, .. } => {
            let thumbs_dir = immich_root.join("thumbs");
            if !thumbs_dir.exists() {
                return Err(Box::new(ImageAnalysisError::InvalidImmichStructure {
                    error: rust_i18n::t!(
                        "error.thumbs_directory_not_found",
                        path = thumbs_dir.display().to_string()
                    )
                    .to_string(),
                }));
            }

            println!(
                "{}",
                rust_i18n::t!(
                    "monitor.folder_monitoring_started",
                    path = thumbs_dir.display().to_string()
                )
            );
            println!("{}", rust_i18n::t!("monitor.stop_instructions"));

            let (event_tx, event_rx): (
                Sender<notify::Result<notify::Event>>,
                Receiver<notify::Result<notify::Event>>,
            ) = mpsc::channel();

            let mut watcher = RecommendedWatcher::new(event_tx, Config::default())?;
            watcher.watch(&thumbs_dir, RecursiveMode::Recursive)?;

            let processing_files = Arc::new(Mutex::new(HashSet::<String>::new()));
            let mut last_events: HashMap<String, Instant> = HashMap::new();
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    Some(_) = stop_rx.recv() => {
                        println!("{}", rust_i18n::t!("monitor.stopping_monitoring"));
                        drop(watcher);
                        return Ok(());
                    }
                    _ = interval.tick() => {
                        handle_fs_events(
                            &event_rx,
                            &mut last_events,
                            &processing_files,
                            config,
                            &bg_ctx,
                        )
                        .await;
                    }
                }
            }
        }

        // ========== IMMICH API MODE: polling-based monitoring ==========
        DataAccess::ImmichApi { provider } => {
            println!("{}", rust_i18n::t!("monitor.api_monitoring_started"));
            println!("{}", rust_i18n::t!("monitor.stop_instructions"));

            let processing_assets = Arc::new(Mutex::new(HashSet::<Uuid>::new()));
            let mut known_assets: HashSet<Uuid> = HashSet::with_capacity(65_536);
            let mut poll_interval =
                tokio::time::interval(Duration::from_secs(config.api_poll_interval));
            poll_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

            let mut is_first_poll = true;
            let mut last_poll_time: Option<chrono::DateTime<chrono::Utc>> = None;

            loop {
                tokio::select! {
                    Some(_) = stop_rx.recv() => {
                        println!("{}", rust_i18n::t!("monitor.stopping_monitoring"));
                        return Ok(());
                    }
                    _ = poll_interval.tick() => {
                        handle_api_poll(
                            provider,
                            &mut known_assets,
                            &processing_assets,
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
    }
}

#[derive(Clone)]
struct BackgroundCtx {
    http_client: Client,
    data_access: DataAccess,
    model_name: String,
    prompt: String,
    ollama_manager: Option<Arc<OllamaHostManager>>,
    llamacpp_manager: Option<Arc<LlamaCppHostManager>>,
}

async fn handle_fs_events(
    event_rx: &Receiver<notify::Result<notify::Event>>,
    last_events: &mut HashMap<String, Instant>,
    processing_files: &Arc<Mutex<HashSet<String>>>,
    config: &MonitorConfig,
    bg_ctx: &BackgroundCtx,
) {
    while let Ok(event) = event_rx.try_recv() {
        match event {
            Ok(event) => {
                if let EventKind::Create(_) | EventKind::Modify(ModifyKind::Data(_)) = event.kind
                    && let Some(path) = event.paths.first()
                {
                    let path = path.as_path();
                    if path.is_file()
                        && let Some(filename) = path.file_name().and_then(|n| n.to_str())
                    {
                        let filename = filename.to_string();
                        if !filename.contains("_preview.") && !filename.contains("-preview.") {
                            continue;
                        }

                        let now = Instant::now();
                        let cooldown_duration = Duration::from_secs(config.event_cooldown);
                        if let Some(last_time) = last_events.get(&filename)
                            && now.duration_since(*last_time) < cooldown_duration
                        {
                            println!(
                                "{}",
                                rust_i18n::t!(
                                    "monitor.skipping_duplicate_event",
                                    filename = filename,
                                    cooldown = config.event_cooldown.to_string()
                                )
                            );
                            continue;
                        }
                        last_events.insert(filename.clone(), now);

                        {
                            let files = processing_files
                                .lock()
                                .expect("Failed to lock processing files");
                            if files.contains(&filename) {
                                println!(
                                    "{}",
                                    rust_i18n::t!(
                                        "monitor.file_already_processing",
                                        filename = filename
                                    )
                                );
                                continue;
                            }
                        }

                        println!(
                            "{}",
                            rust_i18n::t!("monitor.file_queued", filename = filename)
                        );
                        {
                            let mut files = processing_files
                                .lock()
                                .expect("Failed to lock processing files");
                            files.insert(filename.clone());
                        }

                        let bg_ctx_clone = bg_ctx.clone();
                        let path_clone = path.to_path_buf();
                        let filename_clone = filename.clone();
                        let processing_files_clone = Arc::clone(processing_files);
                        let config_clone = config.clone();

                        tokio::spawn(async move {
                            rust_i18n::set_locale(&config_clone.lang);
                            let ctx = ProcessingContext {
                                http_client: &bg_ctx_clone.http_client,
                                data_access: &bg_ctx_clone.data_access,
                                model_name: &bg_ctx_clone.model_name,
                                prompt: &bg_ctx_clone.prompt,
                                timeout: config_clone.timeout,
                                ollama_manager: bg_ctx_clone.ollama_manager.as_ref(),
                                llamacpp_manager: bg_ctx_clone.llamacpp_manager.as_ref(),
                                overwrite_policy: config_clone.overwrite_policy,
                                max_retries: config_clone.max_retries,
                                retry_delay: Duration::from_secs(config_clone.retry_delay_seconds),
                                enrich_prompt: config_clone.enrich_prompt,
                                preserve_human: config_clone.preserve_human,
                            };
                            let result = process_new_file(
                                &ctx,
                                &path_clone,
                                config_clone.file_write_timeout,
                                config_clone.file_check_interval,
                            )
                            .await;
                            {
                                let mut files = processing_files_clone
                                    .lock()
                                    .expect("Failed to lock processing files");
                                files.remove(&filename_clone);
                            }
                            if let Err(e) = result {
                                if let ImageAnalysisError::AlreadyProcessed { filename: _ } = e {
                                    // Expected when ignoring existing files
                                } else {
                                    error!(
                                        "{}",
                                        rust_i18n::t!(
                                            "error.background_processing_error",
                                            filename = filename_clone
                                        )
                                    );
                                }
                            }
                        });
                    }
                }
            }
            Err(e) => {
                error!(
                    "{}",
                    rust_i18n::t!(
                        "error.filesystem_monitoring_error_details",
                        error = e.to_string()
                    )
                );
            }
        }
    }
}

async fn handle_api_poll(
    provider: &ImmichApiProvider,
    known_assets: &mut HashSet<Uuid>,
    processing_assets: &Arc<Mutex<HashSet<Uuid>>>,
    is_first_poll: &mut bool,
    last_poll_time: &mut Option<chrono::DateTime<chrono::Utc>>,
    config: &MonitorConfig,
    bg_ctx: &BackgroundCtx,
) {
    let assets_result = if *is_first_poll {
        provider.get_assets().await
    } else {
        let buffer_secs = config.api_poll_interval * 2;
        let since_time = last_poll_time
            .unwrap_or_else(chrono::Utc::now)
            .checked_sub_signed(chrono::Duration::seconds(buffer_secs as i64))
            .unwrap_or_else(chrono::Utc::now);
        let since_iso = since_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        provider.get_assets_since_timestamp(&since_iso).await
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
                        let processing = processing_assets
                            .lock()
                            .expect("Failed to lock processing assets");
                        if processing.contains(&asset.id) {
                            continue;
                        }
                    }

                    known_assets.insert(asset.id);
                    {
                        let mut processing = processing_assets
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
                    let processing_assets_clone = Arc::clone(processing_assets);
                    let config_clone = config.clone();

                    tokio::spawn(async move {
                        rust_i18n::set_locale(&config_clone.lang);

                        let preview_path =
                            match bg_ctx_clone.data_access.get_preview_path(&asset_id).await {
                                Ok(path) => path,
                                Err(e) => {
                                    error!("Failed to get preview for asset {}: {}", asset_id, e);
                                    let mut processing = processing_assets_clone
                                        .lock()
                                        .expect("Failed to lock processing assets");
                                    processing.remove(&asset_id);
                                    return;
                                }
                            };

                        let ctx = ProcessingContext {
                            http_client: &bg_ctx_clone.http_client,
                            data_access: &bg_ctx_clone.data_access,
                            model_name: &bg_ctx_clone.model_name,
                            prompt: &bg_ctx_clone.prompt,
                            timeout: config_clone.timeout,
                            ollama_manager: bg_ctx_clone.ollama_manager.as_ref(),
                            llamacpp_manager: bg_ctx_clone.llamacpp_manager.as_ref(),
                            overwrite_policy: config_clone.overwrite_policy,
                            max_retries: config_clone.max_retries,
                            retry_delay: Duration::from_secs(config_clone.retry_delay_seconds),
                            enrich_prompt: config_clone.enrich_prompt,
                            preserve_human: config_clone.preserve_human,
                        };

                        let result = process_new_file(
                            &ctx,
                            &preview_path,
                            config_clone.file_write_timeout,
                            config_clone.file_check_interval,
                        )
                        .await;

                        let _ = bg_ctx_clone
                            .data_access
                            .cleanup_preview(&preview_path)
                            .await;

                        {
                            let mut processing = processing_assets_clone
                                .lock()
                                .expect("Failed to lock processing assets");
                            processing.remove(&asset_id);
                        }

                        if let Err(e) = result {
                            if let ImageAnalysisError::AlreadyProcessed { .. } = e {
                                // Expected when ignoring existing files
                            } else {
                                error!(
                                    "{}",
                                    rust_i18n::t!(
                                        "error.background_processing_error",
                                        filename = asset_id.to_string()
                                    )
                                );
                            }
                        }
                    });
                }
                *last_poll_time = Some(chrono::Utc::now());
            }
        }
        Err(e) => {
            error!(
                "{}",
                rust_i18n::t!("error.api_polling_failed", error = e.to_string())
            );
        }
    }
}
