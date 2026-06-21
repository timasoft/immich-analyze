use log::{error, info, warn};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

const ACTIVITY_TIMEOUT_SECS: u64 = 120;

static LAST_ACTIVITY: OnceLock<AtomicU64> = OnceLock::new();

fn timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn get_tracker() -> &'static AtomicU64 {
    LAST_ACTIVITY.get_or_init(|| AtomicU64::new(timestamp_secs()))
}

/// Notify the health server that the application is making progress.
/// Call this periodically from main processing loops (monitor, batch).
pub fn mark_activity() {
    get_tracker().store(timestamp_secs(), Ordering::Relaxed);
}

/// Start a lightweight health check HTTP server.
///
/// Responds with:
/// - `200 OK` if the app has shown activity within `ACTIVITY_TIMEOUT_SECS`
/// - `503 Service Unavailable` if the app appears hung (no recent activity)
///
/// Runs as a background task — if the port is already in use, logs a warning and exits silently.
pub async fn start_health_server(port: u16) {
    if port == 0 {
        return;
    }

    // Initialize the tracker so there's a valid baseline
    get_tracker();

    let addr = format!("127.0.0.1:{port}");
    let listener = match TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(err) => {
            warn!("Failed to start health server on {addr}: {err}");
            return;
        }
    };
    info!("Health server listening on {addr}");

    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                tokio::spawn(async move {
                    let mut buf = [0_u8; 512];
                    if let Ok(n) = stream.read(&mut buf).await {
                        let request_data = buf.get(..n).unwrap_or(&[]);
                        let request = String::from_utf8_lossy(request_data);

                        let now = timestamp_secs();
                        let last_activity = get_tracker().load(Ordering::Relaxed);
                        let is_healthy = now.saturating_sub(last_activity) < ACTIVITY_TIMEOUT_SECS;

                        let response = if request.starts_with("GET /health") && is_healthy {
                            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nOK"
                        } else if request.starts_with("GET /health") {
                            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n"
                        } else {
                            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        };
                        let _: std::io::Result<()> = stream.write_all(response.as_bytes()).await;
                    }
                });
            }
            Err(err) => {
                error!("Health server accept error: {err}");
            }
        }
    }
}
