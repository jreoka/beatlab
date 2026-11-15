mod engine;
mod models;
mod navidrome;
mod neural;
mod sources;
mod store;
mod subsonic;
mod taste;
mod webapp;

use axum::http::header;
use axum::routing::{any, delete, get, post, put};
use axum::Router;
use engine::StreamEngine;
use sources::archive::ArchiveSource;
use sources::deezer::DeezerSource;
use sources::local::LocalSource;
use sources::soulseek::SoulseekSource;
use sources::ytm::YtmSource;
use sources::SourceRegistry;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use store::Store;
use subsonic::AppState;
use taste::TasteEngine;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> models::AppResult<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("beatlab=info,tower_http=warn")),
        )
        .init();

    let addr_str = std::env::var("BEATLAB_ADDR").unwrap_or_else(|_| "0.0.0.0:4533".to_string());
    let data_dir = PathBuf::from(
        std::env::var("BEATLAB_DATA").unwrap_or_else(|_| "./data".to_string()),
    );
    let max_cache_gb: f64 = std::env::var("BEATLAB_CACHE_GB")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5.0);
    let music_dirs: Vec<PathBuf> = std::env::var_os("BEATLAB_MUSIC_DIRS")
        .map(|raw| std::env::split_paths(&raw).filter(|p| !p.as_os_str().is_empty()).collect())
        .unwrap_or_default();
    let ytdlp_override = std::env::var("BEATLAB_YTDLP").ok();

    std::fs::create_dir_all(&data_dir)?;
    let store = Store::open(&data_dir.join("beatlab.db"))?;

    let deezer = DeezerSource::new();
    let ytm = YtmSource::new(ytdlp_override);
    let archive = ArchiveSource::new();
    let local = Arc::new(LocalSource::new(store.clone(), music_dirs));
    let soulseek = SoulseekSource::new(store.clone(), data_dir.join("cache").join("soulseek"));

    if let (Ok(u), Ok(p)) = (
        std::env::var("BEATLAB_SLSK_USER"),
        std::env::var("BEATLAB_SLSK_PASS"),
    ) {
        let srv = std::env::var("BEATLAB_SLSK_SERVER").ok();
        let slsk = soulseek.clone();
        tokio::spawn(async move {
            if let Err(e) = slsk.connect(&u, &p, srv.as_deref()).await {
                warn!(error = %e, "soulseek env login failed");
            }
        });
    }

    if !local.dirs().is_empty() {
        let l = local.clone();
        tokio::task::spawn_blocking(move || {
            l.scan();
        });
    }

    let sources = SourceRegistry::new(deezer, ytm, archive, local, soulseek);
    let engine = Arc::new(StreamEngine::new(
        store.clone(),
        sources.clone(),
        &data_dir,
        max_cache_gb,
    ));
    let taste = Arc::new(TasteEngine::new(store.clone(), sources.clone()));

    let state = AppState {
        store: store.clone(),
        sources,
        engine,
        taste: taste.clone(),
        now_playing: Arc::new(Mutex::new(HashMap::new())),
    };

    // Fast startup catalog warming + periodic refresh every 45 mins
    let warm_taste = taste.clone();
    tokio::spawn(async move {
        warm_taste.warm_catalog().await;
        let mut interval = tokio::time::interval(Duration::from_secs(45 * 60));
        interval.tick().await;
        loop {
            interval.tick().await;
            warm_taste.warm_catalog().await;
        }
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers([
            header::CONTENT_LENGTH,
            header::CONTENT_RANGE,
            header::ACCEPT_RANGES,
            header::CONTENT_TYPE,
            "x-total-count".parse().unwrap(),
        ]);

    let app = Router::new()
        // Embedded Web Player SPA
        .route("/", get(webapp::serve_index))
        .route("/web", get(webapp::serve_index))
        .route("/web/styles.css", get(webapp::serve_css))
        .route("/web/app.js", get(webapp::serve_js))
        // Web Player API (/api/web/*) — open account creation & full player features
        .route("/api/web/me", get(webapp::me))
        .route("/api/web/auth/signin", post(webapp::signin))
        .route("/api/web/auth/signup", post(webapp::signup))
        .route("/api/web/auth/signout", post(webapp::signout))
        .route("/api/web/account/apikey", post(webapp::regenerate_api_key))
        .route("/api/web/account/password", post(webapp::change_password))
        .route("/api/web/home", get(webapp::home))
        .route("/api/web/explore", get(webapp::explore))
        .route("/api/web/mood/{slug}", get(webapp::mood_mix))
        .route("/api/web/genre/{name}", get(webapp::genre_detail))
        .route("/api/web/search", get(webapp::search))
        .route("/api/web/album/{id}", get(webapp::album_detail))
        .route("/api/web/artist/{id}", get(webapp::artist_detail))
        .route("/api/web/library", get(webapp::library))
        .route("/api/web/playlists", post(webapp::create_playlist))
        .route(
            "/api/web/playlist/{id}",
            get(webapp::get_playlist),
        )
        .route(
            "/api/web/playlists/{id}",
            axum::routing::patch(webapp::update_playlist).delete(webapp::delete_playlist),
        )
        .route(
            "/api/web/playlists/{id}/tracks",
            post(webapp::add_playlist_tracks),
        )
        .route(
            "/api/web/playlists/{id}/tracks/{pos}",
            delete(webapp::remove_playlist_track),
        )
        .route(
            "/api/web/playlists/{id}/reorder",
            post(webapp::reorder_playlist_track),
        )
        .route("/api/web/star", post(webapp::toggle_star))
        .route("/api/web/rate", post(webapp::set_rating))
        .route("/api/web/event", post(webapp::record_event))
        .route("/api/web/lyrics/{id}", get(webapp::get_lyrics))
        .route("/api/web/radio", get(webapp::radio))
        .route(
            "/api/web/queue",
            get(webapp::get_queue).put(webapp::save_queue),
        )
        .route("/api/web/warm/{id}", post(webapp::warm_track))
        .route("/api/web/stream/{id}", get(webapp::stream_track))
        .route("/api/web/cover/{id}", get(webapp::cover_art))
        .route("/api/web/sources", get(webapp::sources_info))
        // Subsonic / OpenSubsonic REST API
        .route("/rest/{*action}", any(subsonic::dispatch))
        // Navidrome Auth & Native API (auto-creates new user accounts on first login!)
        .route("/auth/login", post(navidrome::login))
        .route("/api/auth/login", post(navidrome::login))
        .route("/auth/signup", post(navidrome::signup))
        .route("/api/auth/signup", post(navidrome::signup))
        .route("/auth/logout", post(navidrome::logout))
        .route("/api/song", get(navidrome::list_songs))
        .route("/api/song/{id}", get(navidrome::get_song))
        .route("/api/album", get(navidrome::list_albums))
        .route("/api/album/{id}", get(navidrome::get_album))
        .route("/api/artist", get(navidrome::list_artists))
        .route("/api/artist/{id}", get(navidrome::get_artist))
        .route("/api/genre", get(navidrome::list_genres))
        .route("/api/tag", get(navidrome::list_genres))
        .route(
            "/api/playlist",
            get(navidrome::list_playlists).post(navidrome::create_playlist),
        )
        .route(
            "/api/playlist/{id}",
            get(navidrome::get_playlist)
                .put(navidrome::update_playlist)
                .delete(navidrome::delete_playlist),
        )
        .route(
            "/api/playlist/{id}/tracks",
            get(navidrome::get_playlist_tracks)
                .post(navidrome::add_playlist_tracks)
                .delete(navidrome::remove_playlist_tracks),
        )
        .route(
            "/api/playlist/{id}/tracks/{pos}",
            put(navidrome::move_playlist_track),
        )
        .route(
            "/api/queue",
            get(navidrome::get_queue).post(navidrome::save_queue),
        )
        .route(
            "/api/radio",
            get(navidrome::list_radios).post(navidrome::create_radio),
        )
        .route(
            "/api/radio/{id}",
            put(navidrome::update_radio).delete(navidrome::delete_radio),
        )
        .route("/api/user", get(navidrome::list_users))
        .route("/api/user/{id}", put(navidrome::update_user))
        .route("/api/keepalive/keepalive", get(navidrome::keepalive))
        .layer(cors)
        .with_state(state);

    let addr: SocketAddr = addr_str.parse()?;
    info!(address = %addr, data_dir = %data_dir.display(), "Beatlab listening — open sign-up enabled (Subsonic, Navidrome & Web UI)");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
