mod engine;
mod models;
mod navidrome;
mod share_page;
mod sources;
mod stats;
mod store;
mod subsonic;
mod taste;
mod webapp;

use axum::extract::{Request, State};
use axum::http::{header, uri::Authority, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
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
use tracing::info;
use tracing_subscriber::EnvFilter;

fn web_ui_routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/", get(webapp::serve_index))
        .route("/web", get(webapp::serve_index))
        .route("/web/styles.css", get(webapp::serve_css))
        .route("/web/app.js", get(webapp::serve_js))
        .route("/web/favicon.svg", get(webapp::serve_favicon))
        .route("/favicon.svg", get(webapp::serve_favicon))
        .route("/manifest.webmanifest", get(webapp::serve_manifest))
        .route("/web/manifest.webmanifest", get(webapp::serve_manifest))
        .route("/sw.js", get(webapp::serve_sw))
        .route("/web/icon-192.png", get(webapp::serve_icon_192))
        .route("/web/icon-512.png", get(webapp::serve_icon_512))
        .route("/web/icon-maskable-512.png", get(webapp::serve_icon_maskable_512))
        .route("/web/apple-touch-icon.png", get(webapp::serve_apple_touch_icon))
        .route("/apple-touch-icon.png", get(webapp::serve_apple_touch_icon))
}

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
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(5.0);
    let music_dirs: Vec<PathBuf> = std::env::var_os("BEATLAB_MUSIC_DIRS")
        .map(|raw| std::env::split_paths(&raw).filter(|p| !p.as_os_str().is_empty()).collect())
        .unwrap_or_default();
    let ytdlp_override = std::env::var("BEATLAB_YTDLP").ok();
    let allowed_url_raw = std::env::var("URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let allowed_url = allowed_url_raw
        .as_deref()
        .map(AllowedUrl::parse)
        .transpose()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

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
            slsk.maintain_connection(u, p, srv).await;
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
        .merge(web_ui_routes())
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
        .route("/api/web/stats", get(webapp::stats))
        .route("/api/web/playlists", post(webapp::create_playlist))
        .route(
            "/api/web/playlist/{id}",
            get(webapp::get_playlist),
        )
        .route(
            "/api/web/playlist/{id}/recommendations",
            get(webapp::playlist_recommendations),
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
        .route("/api/web/not-interested", post(webapp::mark_not_interested))
        .route("/api/web/hidden", get(webapp::list_hidden))
        .route("/api/web/hidden/reset", post(webapp::reset_hidden))
        .route("/api/web/hidden/{id}", delete(webapp::unhide_track))
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
        .route("/api/web/share", post(webapp::create_share))
        // Public Share Pages & Discord-Compatible Embed Endpoints
        .route("/s/{id}", get(webapp::public_share_page))
        .route("/s/{id}/embed", get(webapp::public_share_embed_page))
        .route("/s/{id}/cover", get(webapp::public_share_cover))
        .route("/s/{id}/stream", get(webapp::public_share_stream_primary))
        .route("/s/{id}/stream/{track_id}", get(webapp::public_share_stream_track))
        .route("/s/{id}/embed.mp4", get(webapp::public_share_embed_mp4))
        .route("/s/{id}/oembed", get(webapp::public_share_oembed))
        .route(
            "/api/public/share/{id}/lyrics/{track_id}",
            get(webapp::public_share_lyrics),
        )
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

    let app = if let Some(ref allowed) = allowed_url {
        app.layer(middleware::from_fn_with_state(
            allowed.clone(),
            enforce_allowed_url,
        ))
    } else {
        app
    };

    // Mount /health outside enforce_allowed_url so container/platform health checks
    // (which hit 127.0.0.1 or private IPs without the public Host) still succeed.
    let app = app.route("/health", get(|| async { "ok" }));

    let addr: SocketAddr = addr_str.parse()?;
    if let Some(ref raw_url) = allowed_url_raw {
        info!(address = %addr, data_dir = %data_dir.display(), url = %raw_url, "Beatlab listening — open sign-up enabled (Subsonic, Navidrome & Web UI)");
    } else {
        info!(address = %addr, data_dir = %data_dir.display(), "Beatlab listening — open sign-up enabled (Subsonic, Navidrome & Web UI)");
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Allowed origin parsed from the `URL` environment variable.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AllowedUrl {
    scheme: Option<String>,
    host: String,
    port: Option<u16>,
}

impl AllowedUrl {
    fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("URL must not be empty".to_string());
        }

        if trimmed.contains("://") {
            let uri: Uri = trimmed
                .parse()
                .map_err(|e| format!("Invalid URL '{trimmed}': {e}"))?;
            let scheme = uri.scheme_str().map(|s| s.to_ascii_lowercase());
            let auth = uri
                .authority()
                .ok_or_else(|| format!("Invalid URL '{trimmed}': missing host"))?;
            let host = auth.host();
            if host.is_empty() {
                return Err(format!("Invalid URL '{trimmed}': missing host"));
            }
            let port = auth.port_u16().or_else(|| default_port(scheme.as_deref()));
            Ok(Self {
                scheme,
                host: host.to_ascii_lowercase(),
                port,
            })
        } else {
            let auth_part = trimmed.split('/').next().unwrap_or(trimmed);
            let auth: Authority = auth_part
                .parse()
                .map_err(|e| format!("Invalid URL '{trimmed}': {e}"))?;
            let host = auth.host();
            if host.is_empty() {
                return Err(format!("Invalid URL '{trimmed}': missing host"));
            }
            Ok(Self {
                scheme: None,
                host: host.to_ascii_lowercase(),
                port: auth.port_u16(),
            })
        }
    }

    fn matches(&self, req: &Request) -> bool {
        let Some(req_auth) = extract_request_authority(req) else {
            return false;
        };

        if !req_auth.host().eq_ignore_ascii_case(&self.host) {
            return false;
        }

        let req_scheme = extract_request_scheme(req);
        if let (Some(expected_scheme), Some(actual_scheme)) = (&self.scheme, &req_scheme) {
            if expected_scheme != actual_scheme {
                return false;
            }
        }

        let req_port = req_auth
            .port_u16()
            .or_else(|| default_port(req_scheme.as_deref()))
            .or_else(|| {
                if self.scheme.is_some() && req_scheme.is_none() {
                    default_port(self.scheme.as_deref())
                } else {
                    None
                }
            });

        req_port == self.port
    }
}

fn default_port(scheme: Option<&str>) -> Option<u16> {
    match scheme {
        Some("http") => Some(80),
        Some("https") => Some(443),
        _ => None,
    }
}

fn extract_request_authority(req: &Request) -> Option<Authority> {
    if let Some(raw) = req
        .headers()
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
    {
        let first = raw.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.parse::<Authority>().ok();
        }
    }

    if let Some(raw) = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.parse::<Authority>().ok();
        }
    }

    req.uri().authority().cloned()
}

fn extract_request_scheme(req: &Request) -> Option<String> {
    for header_name in ["x-forwarded-proto", "x-forwarded-scheme"] {
        if let Some(raw) = req.headers().get(header_name).and_then(|v| v.to_str().ok()) {
            let first = raw.split(',').next().unwrap_or("").trim();
            if !first.is_empty() {
                return Some(first.to_ascii_lowercase());
            }
        }
    }

    req.uri().scheme_str().map(|s| s.to_ascii_lowercase())
}

async fn enforce_allowed_url(
    State(allowed): State<AllowedUrl>,
    req: Request,
    next: Next,
) -> Response {
    if allowed.matches(&req) {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "Forbidden").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    #[test]
    fn allowed_url_parses_schemes_hosts_and_ports() {
        let https = AllowedUrl::parse("https://music.example.com/").unwrap();
        assert_eq!(https.scheme.as_deref(), Some("https"));
        assert_eq!(https.host, "music.example.com");
        assert_eq!(https.port, Some(443));

        let custom_port = AllowedUrl::parse("http://localhost:4533").unwrap();
        assert_eq!(custom_port.scheme.as_deref(), Some("http"));
        assert_eq!(custom_port.host, "localhost");
        assert_eq!(custom_port.port, Some(4533));

        let host_only = AllowedUrl::parse("music.example.com").unwrap();
        assert_eq!(host_only.scheme, None);
        assert_eq!(host_only.host, "music.example.com");
        assert_eq!(host_only.port, None);

        assert!(AllowedUrl::parse("").is_err());
        assert!(AllowedUrl::parse("https://").is_err());
    }

    #[test]
    fn allowed_url_matches_expected_requests_and_rejects_others() {
        let allowed = AllowedUrl::parse("https://music.example.com").unwrap();

        // Matching Host (plain HTTP behind reverse proxy without X-Forwarded-Proto)
        let req = Request::builder()
            .uri("/")
            .header(header::HOST, "music.example.com")
            .body(Body::empty())
            .unwrap();
        assert!(allowed.matches(&req));

        // Matching X-Forwarded-Host and X-Forwarded-Proto
        let req = Request::builder()
            .uri("/")
            .header(header::HOST, "127.0.0.1:4533")
            .header("x-forwarded-host", "Music.Example.com:443")
            .header("x-forwarded-proto", "https")
            .body(Body::empty())
            .unwrap();
        assert!(allowed.matches(&req));

        // Different host -> rejected
        let req = Request::builder()
            .uri("/")
            .header(header::HOST, "127.0.0.1:4533")
            .body(Body::empty())
            .unwrap();
        assert!(!allowed.matches(&req));

        // Wrong port -> rejected
        let req = Request::builder()
            .uri("/")
            .header(header::HOST, "music.example.com:4533")
            .body(Body::empty())
            .unwrap();
        assert!(!allowed.matches(&req));

        // Wrong scheme via X-Forwarded-Proto -> rejected
        let req = Request::builder()
            .uri("/")
            .header(header::HOST, "music.example.com")
            .header("x-forwarded-proto", "http")
            .body(Body::empty())
            .unwrap();
        assert!(!allowed.matches(&req));

        // Missing Host -> rejected
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        assert!(!allowed.matches(&req));
    }

    #[test]
    fn allowed_url_checks_explicit_non_default_port() {
        let allowed = AllowedUrl::parse("http://localhost:4533").unwrap();

        let ok_req = Request::builder()
            .uri("/")
            .header(header::HOST, "localhost:4533")
            .body(Body::empty())
            .unwrap();
        assert!(allowed.matches(&ok_req));

        let missing_port_req = Request::builder()
            .uri("/")
            .header(header::HOST, "localhost")
            .body(Body::empty())
            .unwrap();
        assert!(!allowed.matches(&missing_port_req));
    }

    #[tokio::test]
    async fn enforce_allowed_url_middleware_returns_403_for_other_urls() {
        let allowed = AllowedUrl::parse("https://music.example.com").unwrap();
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(allowed, enforce_allowed_url))
            .route("/health", get(|| async { "ok" }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = reqwest::Client::new();
        let base_url = format!("http://{addr}/");

        // Direct access with 127.0.0.1:<port> Host -> 403 Forbidden
        let forbidden_resp = client.get(&base_url).send().await.unwrap();
        assert_eq!(forbidden_resp.status(), reqwest::StatusCode::FORBIDDEN);

        // Health check on /health bypasses AllowedUrl -> 200 OK
        let health_resp = client
            .get(format!("http://{addr}/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(health_resp.status(), reqwest::StatusCode::OK);

        // Access matching URL via forwarded headers -> 200 OK
        let ok_resp = client
            .get(&base_url)
            .header("x-forwarded-host", "music.example.com")
            .header("x-forwarded-proto", "https")
            .send()
            .await
            .unwrap();
        assert_eq!(ok_resp.status(), reqwest::StatusCode::OK);

        server.abort();
    }

    #[tokio::test]
    async fn web_ui_routes_serve_pwa_assets() {
        let app: Router<()> = web_ui_routes();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = reqwest::Client::new();
        for (path, expected_ct) in [
            ("/", "text/html"),
            ("/manifest.webmanifest", "application/manifest+json"),
            ("/web/manifest.webmanifest", "application/manifest+json"),
            ("/sw.js", "application/javascript"),
            ("/web/icon-192.png", "image/png"),
            ("/web/icon-512.png", "image/png"),
            ("/web/icon-maskable-512.png", "image/png"),
            ("/web/apple-touch-icon.png", "image/png"),
            ("/apple-touch-icon.png", "image/png"),
        ] {
            let resp = client.get(format!("http://{addr}{path}")).send().await.unwrap();
            assert_eq!(resp.status(), reqwest::StatusCode::OK, "{path}");
            let ct = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap();
            assert!(ct.starts_with(expected_ct), "{path} content-type: {ct}");
        }

        server.abort();
    }
}
