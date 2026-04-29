use crate::{
    args::Interface,
    config::{FileProcessingConfig, MonitorConfig},
    data_access::DataAccess,
    error::ImageAnalysisError,
    llamacpp::{LlamaCppHostManager, analyze_image as llamacpp_analyze_image},
    ollama::{OllamaHostManager, analyze_image as ollama_analyze_image},
    utils::extract_uuid_from_preview_filename,
};
use log::error;
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
    ctx: &crate::config::ProcessingContext<'_>,
    preview_path: &Path,
    config: &FileProcessingConfig,
) -> Result<(), ImageAnalysisError> {
    let http_client = ctx.http_client;
    let data_access = ctx.data_access;
    let model_name = ctx.model_name;
    let prompt = ctx.prompt;
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
    let timeout_duration = Duration::from_secs(config.file_write_timeout);
    let check_interval = Duration::from_millis(config.file_check_interval);
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
            timeout: config.file_write_timeout,
            filename: filename.clone(),
        });
    }
    println!(
        "{}",
        rust_i18n::t!("monitor.file_stable", filename = filename)
    );
    let asset_id = extract_uuid_from_preview_filename(&filename)?;

    if !config.overwrite_existing && data_access.has_description(&asset_id).await? {
        println!(
            "{}",
            rust_i18n::t!("monitor.file_already_in_db", filename = filename)
        );
        return Ok(());
    }

    let result = match (ollama_manager, llamacpp_manager) {
        (Some(manager), _) => {
            ollama_analyze_image(
                http_client,
                preview_path,
                model_name,
                prompt,
                config.request_timeout,
                manager,
                config.max_retries,
                Duration::from_secs(config.retry_delay_seconds),
            )
            .await
        }
        (_, Some(manager)) => {
            llamacpp_analyze_image(
                http_client,
                preview_path,
                model_name,
                prompt,
                config.request_timeout,
                manager,
                config.max_retries,
                Duration::from_secs(config.retry_delay_seconds),
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
            data_access
                .update_description(&analysis.asset_id, &analysis.description)
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
                                                println!("{}", rust_i18n::t!("monitor.skipping_duplicate_event",
                                                    filename = filename,
                                                    cooldown = config.event_cooldown.to_string()
                                                ));
                                                continue;
                                            }
                                            last_events.insert(filename.clone(), now);

                                            {
                                                let files = processing_files.lock().expect("Failed to lock processing files");
                                                if files.contains(&filename) {
                                                    println!("{}", rust_i18n::t!("monitor.file_already_processing", filename = filename));
                                                    continue;
                                                }
                                            }

                                            println!("{}", rust_i18n::t!("monitor.file_queued", filename = filename));
                                            {
                                                let mut files = processing_files.lock().expect("Failed to lock processing files");
                                                files.insert(filename.clone());
                                            }

                                            let http_client_clone = http_client.clone();
                                            let data_access_clone = data_access.clone();
                                            let model_name_clone = model_name.to_string();
                                            let path_clone = path.to_path_buf();
                                            let filename_clone = filename.clone();
                                            let processing_files_clone = Arc::clone(&processing_files);
                                            let prompt_clone = prompt.to_string();
                                            let ollama_manager_clone = ollama_manager.clone();
                                            let llamacpp_manager_clone = llamacpp_manager.clone();
                                            let config_clone = config.clone();

                                            let file_processing_config = FileProcessingConfig {
                                                file_write_timeout: config.file_write_timeout,
                                                file_check_interval: config.file_check_interval,
                                                overwrite_existing: config.overwrite_existing,
                                                request_timeout: config.timeout,
                                                max_retries: config.max_retries,
                                                retry_delay_seconds: config.retry_delay_seconds,
                                            };

                                            tokio::spawn(async move {
                                                rust_i18n::set_locale(&config_clone.lang);
                                                let ctx = crate::config::ProcessingContext {
                                                    http_client: &http_client_clone,
                                                    data_access: &data_access_clone,
                                                    model_name: &model_name_clone,
                                                    prompt: &prompt_clone,
                                                    timeout: file_processing_config.request_timeout,
                                                    ollama_manager: ollama_manager_clone.as_ref(),
                                                    llamacpp_manager: llamacpp_manager_clone.as_ref(),
                                                    max_retries: file_processing_config.max_retries,
                                                    retry_delay: Duration::from_secs(file_processing_config.retry_delay_seconds),
                                                };
                                                let result = process_new_file(
                                                    &ctx,
                                                    &path_clone,
                                                    &file_processing_config,
                                                )
                                                .await;
                                                {
                                                    let mut files = processing_files_clone.lock().expect("Failed to lock processing files");
                                                    files.remove(&filename_clone);
                                                }
                                                if let Err(e) = result {
                                                    if let ImageAnalysisError::AlreadyProcessed { filename: _ } = e {
                                                        // Expected when ignoring existing files
                                                    } else {
                                                        error!(
                                                            "{}",
                                                            rust_i18n::t!("error.background_processing_error", filename = filename_clone)
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
                                        rust_i18n::t!("error.filesystem_monitoring_error_details", error = e.to_string())
                                    );
                                }
                            }
                        }
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
                        let assets_result = if is_first_poll {
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
                                if is_first_poll {
                                    for asset in &assets {
                                        known_assets.insert(asset.id);
                                    }
                                    is_first_poll = false;
                                    last_poll_time = Some(chrono::Utc::now());
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
                                            let processing = processing_assets.lock()
                                                .expect("Failed to lock processing assets");
                                            if processing.contains(&asset.id) {
                                                continue;
                                            }
                                        }

                                        known_assets.insert(asset.id);
                                        {
                                            let mut processing = processing_assets.lock()
                                                .expect("Failed to lock processing assets");
                                            processing.insert(asset.id);
                                        }

                                        println!("{}", rust_i18n::t!("monitor.api_asset_queued", asset_id = asset.id.to_string()));

                                        let http_client_clone = http_client.clone();
                                        let data_access_clone = data_access.clone();
                                        let model_name_clone = model_name.to_string();
                                        let asset_id = asset.id;
                                        let processing_assets_clone = Arc::clone(&processing_assets);
                                        let prompt_clone = prompt.to_string();
                                        let ollama_manager_clone = ollama_manager.clone();
                                        let llamacpp_manager_clone = llamacpp_manager.clone();
                                        let config_clone = config.clone();

                                        tokio::spawn(async move {
                                            rust_i18n::set_locale(&config_clone.lang);

                                            let preview_path = match data_access_clone.get_preview_path(&asset_id).await {
                                                Ok(path) => path,
                                                Err(e) => {
                                                    error!("Failed to get preview for asset {}: {}", asset_id, e);
                                                    let mut processing = processing_assets_clone.lock()
                                                        .expect("Failed to lock processing assets");
                                                    processing.remove(&asset_id);
                                                    return;
                                                }
                                            };

                                            let ctx = crate::config::ProcessingContext {
                                                http_client: &http_client_clone,
                                                data_access: &data_access_clone,
                                                model_name: &model_name_clone,
                                                prompt: &prompt_clone,
                                                timeout: config_clone.timeout,
                                                ollama_manager: ollama_manager_clone.as_ref(),
                                                llamacpp_manager: llamacpp_manager_clone.as_ref(),
                                                max_retries: config_clone.max_retries,
                                                retry_delay: Duration::from_secs(config_clone.retry_delay_seconds),
                                            };

                                            let file_processing_config = FileProcessingConfig {
                                                file_write_timeout: config_clone.file_write_timeout,
                                                file_check_interval: config_clone.file_check_interval,
                                                overwrite_existing: config_clone.overwrite_existing,
                                                request_timeout: config_clone.timeout,
                                                max_retries: config_clone.max_retries,
                                                retry_delay_seconds: config_clone.retry_delay_seconds,
                                            };

                                            let result = process_new_file(
                                                &ctx,
                                                &preview_path,
                                                &file_processing_config,
                                            )
                                            .await;

                                            let _ = data_access_clone.cleanup_preview(&preview_path).await;

                                            {
                                                let mut processing = processing_assets_clone.lock()
                                                    .expect("Failed to lock processing assets");
                                                processing.remove(&asset_id);
                                            }

                                            if let Err(e) = result {
                                                if let ImageAnalysisError::AlreadyProcessed { .. } = e {
                                                    // Expected when ignoring existing files
                                                } else {
                                                    error!(
                                                        "{}",
                                                        rust_i18n::t!("error.background_processing_error", filename = asset_id.to_string())
                                                    );
                                                }
                                            }
                                        });
                                    }
                                    last_poll_time = Some(chrono::Utc::now());
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
                }
            }
        }
    }
}
