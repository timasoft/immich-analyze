use crate::{
    args::Interface,
    config::MonitorConfig,
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
use tokio_postgres::Client as PgClient;

/// Process new file with stability checking
pub async fn process_new_file(
    ctx: &crate::config::ProcessingContext<'_>,
    image_path: &Path,
    config: &crate::config::FileProcessingConfig,
) -> Result<(), ImageAnalysisError> {
    let http_client = ctx.http_client;
    let pg_client = ctx.pg_client;
    let model_name = ctx.model_name;
    let prompt = ctx.prompt;
    let ollama_manager = ctx.ollama_manager;
    let llamacpp_manager = ctx.llamacpp_manager;

    let filename = image_path
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
        if let Ok(metadata) = std::fs::metadata(image_path) {
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
    if !config.overwrite_existing
        && crate::database::asset_has_description(pg_client, asset_id).await?
    {
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
                image_path,
                model_name,
                prompt,
                config.request_timeout,
                manager,
            )
            .await
        }
        (_, Some(manager)) => {
            llamacpp_analyze_image(
                http_client,
                image_path,
                model_name,
                prompt,
                config.request_timeout,
                manager,
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
            crate::database::update_or_create_asset_description(
                pg_client,
                analysis.asset_id,
                &analysis.description,
            )
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

/// Monitor folder for new files in Immich thumbs structure
pub async fn monitor_folder(
    immich_root: &Path,
    model_name: &str,
    pg_client: Arc<PgClient>,
    prompt: &str,
    config: &MonitorConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    rust_i18n::set_locale(&config.lang);
    let http_client = Client::builder()
        .timeout(Duration::from_secs(config.timeout))
        .build()?;
    let thumbs_dir = immich_root.join("thumbs");
    if !thumbs_dir.exists() {
        return Err(Box::new(ImageAnalysisError::InvalidImmichStructure {
            error: format!(
                "{}",
                rust_i18n::t!(
                    "error.thumbs_directory_not_found",
                    path = thumbs_dir.display().to_string()
                )
            ),
        }));
    }
    println!(
        "{}",
        rust_i18n::t!("monitor.postgres_connected", url = "Immich database")
    );
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
                    println!(
                        "{}",
                        rust_i18n::t!("monitor.stop_signal_received", signal = "SIGTERM")
                    );
                }
                _ = sigint.recv() => {
                    println!(
                        "{}",
                        rust_i18n::t!("monitor.stop_signal_received", signal = "SIGINT")
                    );
                }
            }
            let _ = stop_tx.send(()).await;
        }
    });

    // Create host managers once for monitor session to preserve unavailable host state
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
                                && let Some(path) = event.paths.first() {
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
                                        let pg_client_clone = Arc::clone(&pg_client);
                                        let model_name_clone = model_name.to_string();
                                        let path_clone = path.to_path_buf();
                                        let filename_clone = filename.clone();
                                        let processing_files_clone = Arc::clone(&processing_files);
                                        let prompt_clone = prompt.to_string();
                                        let ollama_manager_clone = ollama_manager.clone();
                                        let llamacpp_manager_clone = llamacpp_manager.clone();
                                        let config_clone = config.clone();

                                        let file_processing_config = crate::config::FileProcessingConfig {
                                            file_write_timeout: config.file_write_timeout,
                                            file_check_interval: config.file_check_interval,
                                            overwrite_existing: config.overwrite_existing,
                                            request_timeout: config.timeout,
                                        };

                                        tokio::spawn(async move {
                                            rust_i18n::set_locale(&config_clone.lang);

                                        let ctx = crate::config::ProcessingContext {
                                            http_client: &http_client_clone,
                                            pg_client: &pg_client_clone,
                                            model_name: &model_name_clone,
                                            prompt: &prompt_clone,
                                            timeout: file_processing_config.request_timeout,
                                            ollama_manager: ollama_manager_clone.as_ref(),
                                            llamacpp_manager: llamacpp_manager_clone.as_ref(),
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
