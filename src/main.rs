mod engine;
mod models;
mod navidrome;
mod neural;
mod sources;
mod store;
mod subsonic;
mod taste;
mod webui;

use crate::engine::Engine;
use crate::sources::SourceRegistry;
use crate::sources::archive::ArchiveSource;
use crate::sources::bandcamp::BandcampSource;
use crate::sources::local::LocalSource;
use crate::sources::ytdlp::{YtDlpMode, YtDlpSource};
use crate::store::Store;
use crate::subsonic::AppState;
use crate::taste::TasteEngine;
use axum::Router;
use axum::response::Json;
use axum::routing::{get, post};
use rand::RngCore;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;
use tracing::info;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn admin_password(data_dir: &Path) -> String {
    if let Ok(v) = std::env::var("BEATLAB_ADMIN_PASSWORD") {
        if !v.trim().is_empty() {
            return v;
        }
    }
    let path = data_dir.join("admin-password.txt");
    if let Ok(raw) = std::fs::read_to_string(&path) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let mut buf = [0u8; 9];
    rand::thread_rng().fill_bytes(&mut buf);
    let pw = hex::encode(buf);
    let _ = std::fs::create_dir_all(data_dir);
    if std::fs::write(&path, format!("{}\n", pw)).is_ok() {
        info!(path = %path.display(), "generated admin password");
    }
    pw
}

fn parse_music_dirs() -> Vec<PathBuf> {
    let raw = std::env::var("BEATLAB_MUSIC_DIRS").unwrap_or_default();
    raw.split([';', ','])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "beatlab=info,tower_http=info".into()),
        )
        .init();

    let addr = env_or("BEATLAB_ADDR", "0.0.0.0:4533");
    let data_dir = PathBuf::from(env_or("BEATLAB_DATA", "./beatlab-data"));
    let cache_gb: i64 = env_or("BEATLAB_CACHE_GB", "10")
        .parse()
        .unwrap_or(10)
        .max(1);

    let store = Store::open(&data_dir).expect("failed to open SQLite store");
    let password = admin_password(&data_dir);

    if store.user_count() == 0 {
        let username = env_or("BEATLAB_USER", "admin");
        store
            .create_user(&username, &password, true)
            .expect("failed to seed admin user");
        info!(username = %username, "seeded initial admin user");
    }

    let ytdlp_bin = YtDlpSource::find_binary(std::env::var("BEATLAB_YTDLP").ok().as_deref());
    let has_ytdlp = ytdlp_bin.is_some();

    let local = Arc::new(LocalSource::new(store.clone(), parse_music_dirs()));
    let archive = Arc::new(ArchiveSource::new());
    let bandcamp = Arc::new(BandcampSource::new(has_ytdlp));
    let soundcloud = Arc::new(YtDlpSource::new(ytdlp_bin.clone(), YtDlpMode::SoundCloud));
    let youtube = Arc::new(YtDlpSource::new(ytdlp_bin, YtDlpMode::YouTube));

    let registry = Arc::new(SourceRegistry::new(
        store.clone(),
        vec![
            local.clone(),
            archive.clone(),
            bandcamp.clone(),
            soundcloud.clone(),
            youtube.clone(),
        ],
    ));

    let engine = Arc::new(Engine::new(
        store.clone(),
        registry.clone(),
        &data_dir,
        cache_gb,
    ));
    let taste = Arc::new(TasteEngine::new(store.clone(), registry.clone()));

    let state = AppState {
        store: store.clone(),
        registry: registry.clone(),
        engine: engine.clone(),
        taste: taste.clone(),
        local: local.clone(),
        admin_password: password,
        now_playing: Arc::new(Mutex::new(HashMap::new())),
    };

    // Background local scan + catalog warming
    let local_bg = Arc::clone(&local);
    tokio::task::spawn_blocking(move || {
        local_bg.scan();
    });
    let taste_bg = Arc::clone(&taste);
    tokio::spawn(async move {
        taste_bg.warm_catalog().await;
    });

    let app = Router::new()
        .route("/", get(webui::index_page))
        .route("/admin", get(webui::index_page))
        .route("/player/api/home", get(webui::api_home))
        .route("/player/api/search", get(webui::api_search))
        .route("/player/api/mood/{slug}", get(webui::api_mood_radio))
        .route("/player/api/radio/track/{id}", get(webui::api_track_radio))
        .route("/player/api/stream/{id}", get(webui::api_stream))
        .route("/player/api/cover/{id}", get(webui::api_cover))
        .route("/player/api/event", post(webui::api_event))
        .route("/player/api/admin/status", get(webui::api_admin_status))
        .route("/player/api/admin/dirs", post(webui::api_admin_add_dir))
        .route("/player/api/admin/scan", post(webui::api_admin_scan))
        .route("/player/api/admin/cache/clear", post(webui::api_admin_clear_cache))
        .route("/rest/{action}", get(subsonic::dispatch).post(subsonic::dispatch))
        .route("/auth/login", post(navidrome::nd_login))
        .route("/api/song", get(navidrome::nd_song_list))
        .route("/api/playlist", get(navidrome::nd_playlist_list))
        .route("/api/playlist/{id}/tracks", get(navidrome::nd_playlist_tracks))
        .route("/api/radio", get(navidrome::nd_radio_list))
        .route(
            "/healthz",
            get(|| async { Json(json!({"ok": true, "version": "0.2.0-rust"})) }),
        )
        .layer(CorsLayer::permissive())
        .with_state(state);

    info!("Beatlab 0.2.0-rust starting");
    info!(data_dir = %data_dir.display(), "data directory");
    for s in registry.all() {
        info!(source = s.name(), status = %s.status(), "source");
    }
    info!(url = %format!("http://{}", addr), "Web Player & Subsonic API listening");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind address");
    axum::serve(listener, app).await.expect("server error");
}
