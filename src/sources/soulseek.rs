use crate::models::Track;
use crate::sources::MusicSource;
use crate::taste::classify_moods_str;
use crate::store::Store;
use async_trait::async_trait;
use flate2::read::ZlibDecoder;
use md5::{Digest, Md5};
use std::collections::HashMap;
use std::io::Read;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify, RwLock, mpsc};
use tracing::{debug, info, warn};

const DEFAULT_SERVER: &str = "server.slsknet.org:2242";
const KV_USER: &str = "slsk:username";
const KV_PASS: &str = "slsk:password";
const KV_SERVER: &str = "slsk:server";

// Soulseek server message codes
const MSG_LOGIN: u32 = 1;
const MSG_SET_WAIT_PORT: u32 = 2;
const MSG_GET_PEER_ADDRESS: u32 = 3;
const MSG_CONNECT_TO_PEER: u32 = 18;
const MSG_FILE_SEARCH: u32 = 26;
const MSG_PING: u32 = 32;
const MSG_SHARED_FOLDERS_FILES: u32 = 35;
const MSG_HAVE_NO_PARENT: u32 = 71;
const MSG_ACCEPT_CHILDREN: u32 = 100;

// Soulseek peer init / message codes
const PEER_PIERCE_FIREWALL: u8 = 0;
const PEER_INIT: u8 = 1;
const PEER_MSG_SEARCH_RESPONSE: u32 = 9;
const PEER_MSG_TRANSFER_REQUEST: u32 = 40;
const PEER_MSG_TRANSFER_RESPONSE: u32 = 41;
const PEER_MSG_QUEUE_UPLOAD: u32 = 43;
const PEER_MSG_PLACE_IN_QUEUE_RESPONSE: u32 = 44;
const PEER_MSG_UPLOAD_FAILED: u32 = 46;
const PEER_MSG_UPLOAD_DENIED: u32 = 50;

struct SlskRuntime {
    connected: bool,
    connecting: bool,
    username: Option<String>,
    server: String,
    last_error: Option<String>,
    writer: Option<mpsc::Sender<Vec<u8>>>,
    session_id: u64,
}

#[derive(Clone)]
struct SlskSearchHit {
    track: Track,
    slot_free: bool,
    avg_speed: u32,
}

struct PendingTransfer {
    peer_lower: String,
    remote_path: String,
    remote_path_norm: String,
    upload_info: Mutex<Option<(u32, u64)>>,
    legacy_dl_token: u32,
    file_socket: Mutex<Option<(u32, TcpStream)>>,
    failed_reason: Mutex<Option<String>>,
    notify: Notify,
}

impl PendingTransfer {
    fn new(peer: &str, remote_path: &str, legacy_dl_token: u32) -> Self {
        Self {
            peer_lower: peer.to_lowercase(),
            remote_path: remote_path.to_string(),
            remote_path_norm: normalize_slsk_path(remote_path),
            upload_info: Mutex::new(None),
            legacy_dl_token,
            file_socket: Mutex::new(None),
            failed_reason: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    fn matches_path(&self, fname: &str) -> bool {
        self.remote_path == fname || self.remote_path_norm == normalize_slsk_path(fname)
    }
}

pub struct SoulseekSource {
    store: Store,
    download_dir: PathBuf,
    runtime: RwLock<SlskRuntime>,
    pending_searches: Mutex<HashMap<u32, mpsc::Sender<Vec<SlskSearchHit>>>>,
    peer_addrs: Mutex<HashMap<String, Option<(Ipv4Addr, u32)>>>,
    addr_notify: Notify,
    pending_transfers: Mutex<Vec<Arc<PendingTransfer>>>,
    pending_by_token: Mutex<HashMap<u32, Arc<PendingTransfer>>>,
    pending_pierce_peers: Mutex<HashMap<u32, (String, String)>>,
    in_flight_downloads: Mutex<HashMap<String, Arc<Notify>>>,
    alt_streams: Mutex<HashMap<String, Vec<String>>>,
    next_token: Mutex<u32>,
    self_weak: OnceLock<Weak<Self>>,
}

impl SoulseekSource {
    pub fn new(store: Store, download_dir: PathBuf) -> Arc<Self> {
        let saved_user = store.kv_get(KV_USER).filter(|s| !s.trim().is_empty());
        let saved_pass = store.kv_get(KV_PASS).filter(|s| !s.is_empty());
        let saved_server = store
            .kv_get(KV_SERVER)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SERVER.to_string());

        let src = Arc::new(Self {
            store,
            download_dir,
            runtime: RwLock::new(SlskRuntime {
                connected: false,
                connecting: false,
                username: saved_user.clone(),
                server: saved_server.clone(),
                last_error: None,
                writer: None,
                session_id: 0,
            }),
            pending_searches: Mutex::new(HashMap::new()),
            peer_addrs: Mutex::new(HashMap::new()),
            addr_notify: Notify::new(),
            pending_transfers: Mutex::new(Vec::new()),
            pending_by_token: Mutex::new(HashMap::new()),
            pending_pierce_peers: Mutex::new(HashMap::new()),
            in_flight_downloads: Mutex::new(HashMap::new()),
            alt_streams: Mutex::new(HashMap::new()),
            next_token: Mutex::new(1000),
            self_weak: OnceLock::new(),
        });
        let _ = src.self_weak.set(Arc::downgrade(&src));

        if let (Some(u), Some(p)) = (saved_user, saved_pass) {
            let s = Arc::clone(&src);
            tokio::spawn(async move {
                if let Err(e) = s.connect(&u, &p, Some(&saved_server)).await {
                    warn!(error = %e, "soulseek auto-connect failed");
                }
            });
        }

        src
    }

    pub async fn connect(
        self: &Arc<Self>,
        username: &str,
        password: &str,
        server: Option<&str>,
    ) -> Result<(), String> {
        let user = username.trim().to_string();
        let pass = password.to_string();
        if user.is_empty() || pass.is_empty() {
            return Err("Username and password are required".to_string());
        }
        let srv = server
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_SERVER)
            .to_string();

        let session_id = {
            let mut rt = self.runtime.write().await;
            rt.connected = false;
            rt.connecting = true;
            rt.writer = None;
            rt.username = Some(user.clone());
            rt.server = srv.clone();
            rt.last_error = None;
            rt.session_id = rt.session_id.wrapping_add(1);
            rt.session_id
        };

        let connect_res = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(&srv))
            .await
            .map_err(|_| format!("Connection to {} timed out", srv))?
            .map_err(|e| format!("Failed to connect to {}: {}", srv, e));

        let mut stream = match connect_res {
            Ok(s) => s,
            Err(e) => {
                let mut rt = self.runtime.write().await;
                if rt.session_id == session_id {
                    rt.connecting = false;
                    rt.last_error = Some(e.clone());
                }
                return Err(e);
            }
        };

        // Send Login (code 1)
        let login_frame = build_login_frame(&user, &pass);
        if let Err(e) = stream.write_all(&login_frame).await {
            let msg = format!("Failed to send login: {}", e);
            self.set_error(session_id, &msg).await;
            return Err(msg);
        }

        // Read Login response
        let (code, payload) = match tokio::time::timeout(
            Duration::from_secs(10),
            read_frame(&mut stream),
        )
        .await
        {
            Ok(Ok(frame)) => frame,
            Ok(Err(e)) => {
                let msg = format!("Login read error: {}", e);
                self.set_error(session_id, &msg).await;
                return Err(msg);
            }
            Err(_) => {
                let msg = "Login response timed out".to_string();
                self.set_error(session_id, &msg).await;
                return Err(msg);
            }
        };

        if code != MSG_LOGIN || payload.is_empty() {
            let msg = format!("Unexpected login response (code {})", code);
            self.set_error(session_id, &msg).await;
            return Err(msg);
        }

        if payload[0] == 0 {
            let mut cur = Cursor::new(&payload[1..]);
            let reason = cur.read_string().unwrap_or_else(|| "Invalid credentials".to_string());
            let msg = format!("Soulseek login rejected: {}", reason);
            self.set_error(session_id, &msg).await;
            return Err(msg);
        }

        // Bind local peer listener (respect optional BEATLAB_SLSK_PORT or bind ephemeral port)
        let bind_addr = std::env::var("BEATLAB_SLSK_PORT")
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .filter(|&p| p > 0)
            .map(|p| format!("0.0.0.0:{}", p))
            .unwrap_or_else(|| "0.0.0.0:0".to_string());
        let listener = match TcpListener::bind(&bind_addr).await {
            Ok(l) => Some(l),
            Err(_) => TcpListener::bind("0.0.0.0:0").await.ok(),
        };
        let listen_port = listener
            .as_ref()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.port() as u32)
            .unwrap_or(2234);

        let (mut rd, mut wr) = stream.into_split();
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);

        // Persist credentials on successful login
        self.store.kv_set(KV_USER, &user);
        self.store.kv_set(KV_PASS, &pass);
        self.store.kv_set(KV_SERVER, &srv);

        {
            let mut rt = self.runtime.write().await;
            if rt.session_id != session_id {
                return Err("Superseded by newer connection".to_string());
            }
            rt.connected = true;
            rt.connecting = false;
            rt.username = Some(user.clone());
            rt.server = srv.clone();
            rt.last_error = None;
            rt.writer = Some(tx.clone());
        }

        info!(username = %user, server = %srv, port = listen_port, "soulseek connected");

        // Send initial session setup frames
        let _ = tx.send(build_u32_frame(MSG_SET_WAIT_PORT, listen_port)).await;
        let _ = tx.send(build_shared_folders_frame(0, 0)).await;
        let _ = tx.send(build_u8_frame(MSG_HAVE_NO_PARENT, 1)).await;
        let _ = tx.send(build_u8_frame(MSG_ACCEPT_CHILDREN, 0)).await;

        // Writer task
        tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if wr.write_all(&frame).await.is_err() {
                    break;
                }
            }
        });

        // Reader task
        let self_rd = Arc::clone(self);
        let tx_rd = tx.clone();
        tokio::spawn(async move {
            loop {
                match read_frame(&mut rd).await {
                    Ok((msg_code, data)) => {
                        self_rd
                            .handle_server_message(msg_code, &data, &tx_rd)
                            .await;
                    }
                    Err(e) => {
                        let mut rt = self_rd.runtime.write().await;
                        if rt.session_id == session_id {
                            rt.connected = false;
                            rt.connecting = false;
                            rt.writer = None;
                            rt.last_error = Some(format!("Disconnected: {}", e));
                        }
                        break;
                    }
                }
            }
        });

        // Peer listener task
        if let Some(listener) = listener {
            let self_peer = Arc::clone(self);
            tokio::spawn(async move {
                loop {
                    {
                        let rt = self_peer.runtime.read().await;
                        if !rt.connected || rt.session_id != session_id {
                            break;
                        }
                    }
                    match listener.accept().await {
                        Ok((sock, _)) => {
                            let sp = Arc::clone(&self_peer);
                            tokio::spawn(async move {
                                sp.handle_incoming_peer(sock).await;
                            });
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        Ok(())
    }

    async fn set_error(&self, session_id: u64, msg: &str) {
        let mut rt = self.runtime.write().await;
        if rt.session_id == session_id {
            rt.connected = false;
            rt.connecting = false;
            rt.writer = None;
            rt.last_error = Some(msg.to_string());
        }
    }

    async fn handle_server_message(
        self: &Arc<Self>,
        code: u32,
        data: &[u8],
        tx: &mpsc::Sender<Vec<u8>>,
    ) {
        match code {
            MSG_PING => {
                let _ = tx.send(build_empty_frame(MSG_PING)).await;
            }
            MSG_GET_PEER_ADDRESS => {
                let mut cur = Cursor::new(data);
                if let (Some(peer), Some(ip_raw), Some(port)) =
                    (cur.read_string(), cur.read_u32(), cur.read_u32())
                {
                    let addr = (ip_raw != 0 && port > 0 && port <= 65535)
                        .then(|| (Ipv4Addr::from(ip_raw), port));
                    self.cache_peer_addr(peer, addr).await;
                }
            }
            MSG_CONNECT_TO_PEER => {
                let mut cur = Cursor::new(data);
                if let (Some(peer), Some(conn_type), Some(ip_raw), Some(port), Some(token)) = (
                    cur.read_string(),
                    cur.read_string(),
                    cur.read_u32(),
                    cur.read_u32(),
                    cur.read_u32(),
                ) {
                    let valid = ip_raw != 0 && port > 0 && port <= 65535;
                    let ip = Ipv4Addr::from(ip_raw);
                    if valid {
                        self.cache_peer_addr(peer.clone(), Some((ip, port))).await;
                    }
                    if valid && conn_type == "P" {
                        let sp = Arc::clone(self);
                        tokio::spawn(async move {
                            sp.connect_peer_pierce(&peer, ip, port, token).await;
                        });
                    } else if valid && conn_type == "F" {
                        let sp = Arc::clone(self);
                        tokio::spawn(async move {
                            sp.connect_peer_pierce_file(ip, port, token).await;
                        });
                    }
                }
            }
            _ => {
                debug!(code, len = data.len(), "ignored soulseek server message");
            }
        }
    }

    async fn connect_peer_pierce(
        self: &Arc<Self>,
        peer: &str,
        ip: Ipv4Addr,
        port: u32,
        token: u32,
    ) {
        let addr = format!("{}:{}", ip, port);
        let Ok(Ok(mut sock)) =
            tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&addr)).await
        else {
            return;
        };
        let mut init_payload = Vec::with_capacity(5);
        init_payload.push(PEER_PIERCE_FIREWALL);
        init_payload.extend_from_slice(&token.to_le_bytes());
        let frame = wrap_len(&init_payload);
        if sock.write_all(&frame).await.is_err() {
            return;
        }
        self.read_peer_messages(&mut sock, Some(peer.to_string()))
            .await;
    }

    async fn connect_peer_pierce_file(&self, ip: Ipv4Addr, port: u32, token: u32) {
        let addr = format!("{}:{}", ip, port);
        let Ok(Ok(mut sock)) =
            tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&addr)).await
        else {
            return;
        };
        let mut init_payload = Vec::with_capacity(5);
        init_payload.push(PEER_PIERCE_FIREWALL);
        init_payload.extend_from_slice(&token.to_le_bytes());
        let frame = wrap_len(&init_payload);
        if sock.write_all(&frame).await.is_err() {
            return;
        }
        self.handle_file_socket(sock).await;
    }

    async fn cache_peer_addr(&self, peer: String, addr: Option<(Ipv4Addr, u32)>) {
        {
            let mut guard = self.peer_addrs.lock().await;
            if guard.len() >= 4096 {
                guard.clear();
            }
            guard.insert(peer, addr);
        }
        self.addr_notify.notify_waiters();
    }

    async fn handle_incoming_peer(self: &Arc<Self>, mut sock: TcpStream) {
        let Ok(Ok((first_byte, rest))) =
            tokio::time::timeout(Duration::from_secs(5), read_raw_peer_init(&mut sock)).await
        else {
            return;
        };
        if first_byte == PEER_INIT {
            let mut cur = Cursor::new(&rest);
            let peer_name = cur.read_string();
            let conn_type = cur.read_string();
            if conn_type.as_deref() == Some("F") {
                self.handle_file_socket(sock).await;
                return;
            }
            self.read_peer_messages(&mut sock, peer_name).await;
        } else if first_byte == PEER_PIERCE_FIREWALL {
            let mut cur = Cursor::new(&rest);
            if let Some(tok) = cur.read_u32() {
                if let Some((peer, remote_path)) =
                    self.pending_pierce_peers.lock().await.remove(&tok)
                {
                    let q = build_queue_upload_frame(&remote_path);
                    if sock.write_all(&q).await.is_ok() {
                        self.read_peer_messages(&mut sock, Some(peer)).await;
                    }
                    return;
                }
            }
            self.read_peer_messages(&mut sock, None).await;
        }
    }

    async fn handle_file_socket(&self, mut sock: TcpStream) {
        let mut tok_buf = [0u8; 4];
        let Ok(Ok(_)) =
            tokio::time::timeout(Duration::from_secs(5), sock.read_exact(&mut tok_buf)).await
        else {
            return;
        };
        let upload_token = u32::from_le_bytes(tok_buf);
        let pending = self.pending_by_token.lock().await.get(&upload_token).cloned();
        if let Some(p) = pending {
            *p.file_socket.lock().await = Some((upload_token, sock));
            p.notify.notify_waiters();
        }
    }

    async fn find_pending_transfer(
        &self,
        peer_hint: Option<&str>,
        filename: &str,
    ) -> Option<Arc<PendingTransfer>> {
        let guard = self.pending_transfers.lock().await;
        if let Some(peer) = peer_hint {
            let peer_low = peer.to_lowercase();
            if let Some(hit) = guard
                .iter()
                .find(|p| p.peer_lower == peer_low && p.matches_path(filename))
            {
                return Some(Arc::clone(hit));
            }
        }
        guard
            .iter()
            .find(|p| p.matches_path(filename))
            .cloned()
    }

    async fn read_peer_messages(&self, sock: &mut TcpStream, peer_hint: Option<String>) {
        loop {
            let Ok(Ok((code, payload))) =
                tokio::time::timeout(Duration::from_secs(15), read_frame(sock)).await
            else {
                break;
            };
            match code {
                PEER_MSG_SEARCH_RESPONSE => {
                    if let Some((token, hits)) =
                        parse_search_response(&payload, peer_hint.as_deref())
                    {
                        let sender = self.pending_searches.lock().await.get(&token).cloned();
                        if let Some(tx) = sender {
                            let _ = tx.send(hits).await;
                        }
                    }
                }
                PEER_MSG_TRANSFER_REQUEST => {
                    let mut cur = Cursor::new(&payload);
                    let Some(direction) = cur.read_u32() else {
                        continue;
                    };
                    let Some(req_tok) = cur.read_u32() else {
                        continue;
                    };
                    let Some(filename) = cur.read_string() else {
                        continue;
                    };
                    let size = if direction == 1 {
                        cur.read_u64().unwrap_or(0)
                    } else {
                        0
                    };
                    if direction == 1 {
                        if let Some(p) = self
                            .find_pending_transfer(peer_hint.as_deref(), &filename)
                            .await
                        {
                            if size > 4096 && size < 250 * 1024 * 1024 {
                                self.pending_by_token
                                    .lock()
                                    .await
                                    .insert(req_tok, Arc::clone(&p));
                                *p.upload_info.lock().await = Some((req_tok, size));
                                let resp = build_transfer_response_frame(req_tok, true, None);
                                if sock.write_all(&resp).await.is_err() {
                                    self.pending_by_token.lock().await.remove(&req_tok);
                                    *p.failed_reason.lock().await =
                                        Some("Failed to send transfer acceptance".to_string());
                                    p.notify.notify_waiters();
                                    break;
                                }
                                p.notify.notify_waiters();
                                continue;
                            }
                        }
                        let deny = build_transfer_response_frame(req_tok, false, Some("Cancelled"));
                        let _ = sock.write_all(&deny).await;
                    } else {
                        let deny = build_transfer_response_frame(req_tok, false, Some("Cancelled"));
                        let _ = sock.write_all(&deny).await;
                    }
                }
                PEER_MSG_TRANSFER_RESPONSE => {
                    let mut cur = Cursor::new(&payload);
                    if let (Some(resp_tok), Some(allowed)) = (cur.read_u32(), cur.read_u8()) {
                        let pending = {
                            let guard = self.pending_transfers.lock().await;
                            guard
                                .iter()
                                .find(|p| p.legacy_dl_token == resp_tok)
                                .cloned()
                        };
                        if let Some(p) = pending {
                            if allowed != 0 {
                                let size = cur.read_u64().unwrap_or(0);
                                if size > 4096 && size < 250 * 1024 * 1024 {
                                    self.pending_by_token
                                        .lock()
                                        .await
                                        .insert(resp_tok, Arc::clone(&p));
                                    *p.upload_info.lock().await = Some((resp_tok, size));
                                    p.notify.notify_waiters();
                                }
                            } else {
                                let reason = cur.read_string().unwrap_or_default();
                                if !reason.eq_ignore_ascii_case("Queued") {
                                    *p.failed_reason.lock().await = Some(if reason.is_empty() {
                                        "Transfer rejected".to_string()
                                    } else {
                                        reason
                                    });
                                    p.notify.notify_waiters();
                                }
                            }
                        }
                    }
                }
                PEER_MSG_PLACE_IN_QUEUE_RESPONSE => {
                    let mut cur = Cursor::new(&payload);
                    if let (Some(filename), Some(place)) = (cur.read_string(), cur.read_u32()) {
                        if place > 5 {
                            if let Some(p) = self
                                .find_pending_transfer(peer_hint.as_deref(), &filename)
                                .await
                            {
                                *p.failed_reason.lock().await =
                                    Some(format!("Queued at position {}", place));
                                p.notify.notify_waiters();
                            }
                        }
                    }
                }
                PEER_MSG_UPLOAD_DENIED => {
                    let mut cur = Cursor::new(&payload);
                    if let Some(filename) = cur.read_string() {
                        let reason = cur.read_string().unwrap_or_else(|| "Denied".to_string());
                        if let Some(p) = self
                            .find_pending_transfer(peer_hint.as_deref(), &filename)
                            .await
                        {
                            *p.failed_reason.lock().await = Some(reason);
                            p.notify.notify_waiters();
                        }
                    }
                }
                PEER_MSG_UPLOAD_FAILED => {
                    let mut cur = Cursor::new(&payload);
                    if let Some(filename) = cur.read_string() {
                        if let Some(p) = self
                            .find_pending_transfer(peer_hint.as_deref(), &filename)
                            .await
                        {
                            *p.failed_reason.lock().await = Some("Upload failed".to_string());
                            p.notify.notify_waiters();
                        }
                    }
                }
                _ => {}
            }
        }
    }

    async fn next_search_token(&self) -> u32 {
        let mut tok = self.next_token.lock().await;
        *tok = tok.wrapping_add(1);
        *tok
    }

    async fn resolve_peer_addr(&self, peer: &str) -> Option<(Ipv4Addr, u32)> {
        if let Some(cached) = self.peer_addrs.lock().await.get(peer).copied() {
            return cached;
        }
        let writer = self.runtime.read().await.writer.clone()?;
        let _ = writer
            .send(build_string_frame(MSG_GET_PEER_ADDRESS, peer))
            .await;

        let deadline = tokio::time::Instant::now() + Duration::from_millis(1600);
        loop {
            let notified = self.addr_notify.notified();
            if let Some(cached) = self.peer_addrs.lock().await.get(peer).copied() {
                return cached;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let _ = tokio::time::timeout(deadline - now, notified).await;
            if let Some(cached) = self.peer_addrs.lock().await.get(peer).copied() {
                return cached;
            }
        }
    }

    /// Tries the primary `track.stream_url` first, then up to 2 cached alternate Soulseek peers
    /// for the same `song_key` if the primary peer is unavailable, firewalled, or queued.
    pub async fn resolve_track(self: &Arc<Self>, track: &Track) -> Option<String> {
        if let Some(path) = self.resolve_stream(&track.stream_url).await {
            return Some(path);
        }
        let key = track.song_key();
        let alts = {
            let guard = self.alt_streams.lock().await;
            guard.get(&key).cloned().unwrap_or_default()
        };
        let mut tried = 0usize;
        for alt_url in alts {
            if alt_url == track.stream_url {
                continue;
            }
            tried += 1;
            if let Some(path) = self.resolve_stream(&alt_url).await {
                return Some(path);
            }
            if tried >= 2 {
                break;
            }
        }
        None
    }

    async fn resolve_stream_singleflight(self: &Arc<Self>, stream_ref: &str) -> Option<String> {
        let rest = stream_ref.strip_prefix("slsk://")?;
        let (peer, remote_path) = rest.split_once('/')?;

        let _ = std::fs::create_dir_all(&self.download_dir);
        let ext = sanitize_slsk_ext(remote_path);
        let base_hash = hash_str(stream_ref);
        let local_path = self.download_dir.join(format!("{}.{}", base_hash, ext));
        if tokio::fs::metadata(&local_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0)
            > 4096
        {
            return Some(format!("file:{}", local_path.display()));
        }

        loop {
            let (notify, is_leader) = {
                let mut map = self.in_flight_downloads.lock().await;
                if let Some(existing) = map.get(stream_ref) {
                    (Arc::clone(existing), false)
                } else {
                    let n = Arc::new(Notify::new());
                    map.insert(stream_ref.to_string(), Arc::clone(&n));
                    (n, true)
                }
            };

            if !is_leader {
                notify.notified().await;
                if tokio::fs::metadata(&local_path)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0)
                    > 4096
                {
                    return Some(format!("file:{}", local_path.display()));
                }
                return None;
            }

            let res = self
                .download_from_peer(peer, remote_path, &local_path, &base_hash)
                .await;
            {
                let mut map = self.in_flight_downloads.lock().await;
                map.remove(stream_ref);
            }
            notify.notify_waiters();
            return res;
        }
    }

    async fn download_from_peer(
        self: &Arc<Self>,
        peer: &str,
        remote_path: &str,
        local_path: &Path,
        base_hash: &str,
    ) -> Option<String> {
        // Wait briefly only if auto-connect is actively in progress; otherwise fail fast when offline.
        for _ in 0..8 {
            let (connected, connecting) = {
                let rt = self.runtime.read().await;
                (rt.connected, rt.connecting)
            };
            if connected {
                break;
            }
            if !connecting {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if !self.runtime.read().await.connected {
            return None;
        }

        let part_path = self.download_dir.join(format!("{}.part", base_hash));
        let token = self.next_search_token().await;
        let pending = Arc::new(PendingTransfer::new(peer, remote_path, token));
        self.pending_transfers.lock().await.push(Arc::clone(&pending));

        let res = self
            .execute_peer_transfer(peer, remote_path, local_path, &part_path, token, &pending)
            .await;

        // Clean up transfer coordination state cleanly on any exit path.
        {
            let mut list = self.pending_transfers.lock().await;
            list.retain(|p| !Arc::ptr_eq(p, &pending));
        }
        if let Some((upload_tok, _)) = *pending.upload_info.lock().await {
            self.pending_by_token.lock().await.remove(&upload_tok);
        }
        self.pending_pierce_peers.lock().await.remove(&token);

        res
    }

    async fn execute_peer_transfer(
        self: &Arc<Self>,
        peer: &str,
        remote_path: &str,
        local_path: &Path,
        part_path: &Path,
        token: u32,
        pending: &Arc<PendingTransfer>,
    ) -> Option<String> {
        let my_user = self
            .runtime
            .read()
            .await
            .username
            .clone()
            .unwrap_or_default();
        let peer_addr = self.resolve_peer_addr(peer).await;
        let mut connected_direct = false;

        if let Some((ip, port)) = peer_addr {
            let addr = format!("{}:{}", ip, port);
            if let Ok(Ok(mut sock)) =
                tokio::time::timeout(Duration::from_millis(2500), TcpStream::connect(&addr)).await
            {
                let mut init = Vec::new();
                init.push(PEER_INIT);
                write_str(&mut init, &my_user);
                write_str(&mut init, "P");
                init.extend_from_slice(&token.to_le_bytes());
                if sock.write_all(&wrap_len(&init)).await.is_ok()
                    && sock
                        .write_all(&build_queue_upload_frame(remote_path))
                        .await
                        .is_ok()
                {
                    connected_direct = true;
                    let sp = Arc::clone(self);
                    let peer_owned = peer.to_string();
                    tokio::spawn(async move {
                        sp.read_peer_messages(&mut sock, Some(peer_owned)).await;
                    });
                }
            }
        }

        if !connected_direct {
            // Peer may be firewalled; ask server to tell peer to pierce back to our listener.
            let Some(writer) = self.runtime.read().await.writer.clone() else {
                return None;
            };
            self.pending_pierce_peers
                .lock()
                .await
                .insert(token, (peer.to_string(), remote_path.to_string()));
            let _ = writer
                .send(build_connect_to_peer_frame(token, peer, "P"))
                .await;
        }

        // Wait for TransferRequest(dir=1) -> TransferResponse(allowed=1), or fail fast on denial/deep queue.
        let req_timeout = if connected_direct {
            Duration::from_millis(4500)
        } else {
            Duration::from_millis(2500)
        };
        let req_deadline = tokio::time::Instant::now() + req_timeout;
        let (_upload_tok, size) = loop {
            let notified = pending.notify.notified();
            if let Some(info) = *pending.upload_info.lock().await {
                break info;
            }
            if let Some(ref reason) = *pending.failed_reason.lock().await {
                debug!(peer, reason, "soulseek transfer declined before start");
                return None;
            }
            let now = tokio::time::Instant::now();
            if now >= req_deadline {
                return None;
            }
            let _ = tokio::time::timeout(req_deadline - now, notified).await;
        };

        // Wait for incoming "F" socket (direct or via MSG_CONNECT_TO_PEER("F") -> PierceFirewall).
        let sock_deadline = tokio::time::Instant::now() + Duration::from_secs(6);
        let (_ftok, mut fsock) = loop {
            let notified = pending.notify.notified();
            if let Some(sock_pair) = pending.file_socket.lock().await.take() {
                break sock_pair;
            }
            if let Some(ref reason) = *pending.failed_reason.lock().await {
                debug!(peer, reason, "soulseek transfer failed waiting for F socket");
                return None;
            }
            let now = tokio::time::Instant::now();
            if now >= sock_deadline {
                return None;
            }
            let _ = tokio::time::timeout(sock_deadline - now, notified).await;
        };

        // Send FileOffset (8-byte LE start offset = 0) and stream file bytes.
        fsock.write_all(&0u64.to_le_bytes()).await.ok()?;

        let mut file = tokio::fs::File::create(part_path).await.ok()?;
        let mut chunk = [0u8; 32 * 1024];
        let mut written = 0u64;
        let xfer_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while written < size {
            let now = tokio::time::Instant::now();
            if now >= xfer_deadline {
                break;
            }
            let need = ((size - written) as usize).min(chunk.len());
            let step_timeout = (xfer_deadline - now).min(Duration::from_secs(8));
            let read_res = tokio::time::timeout(step_timeout, fsock.read(&mut chunk[..need])).await;
            match read_res {
                Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
                Ok(Ok(n)) => {
                    if file.write_all(&chunk[..n]).await.is_err() {
                        written = 0;
                        break;
                    }
                    written += n as u64;
                }
            }
        }
        let _ = file.flush().await;
        drop(file);
        if written != size {
            let _ = tokio::fs::remove_file(part_path).await;
            return None;
        }
        if tokio::fs::rename(part_path, local_path).await.is_err() {
            let _ = tokio::fs::remove_file(part_path).await;
            return None;
        }
        Some(format!("file:{}", local_path.display()))
    }
}

#[async_trait]
impl MusicSource for SoulseekSource {
    fn name(&self) -> &'static str {
        "soulseek"
    }

    fn available(&self) -> bool {
        self.runtime
            .try_read()
            .map(|r| r.connected)
            .unwrap_or(false)
    }

    fn status(&self) -> String {
        if let Ok(rt) = self.runtime.try_read() {
            if rt.connected {
                let u = rt.username.as_deref().unwrap_or("user");
                return format!("connected as {} ({})", u, rt.server);
            }
            if rt.connecting {
                return "connecting...".to_string();
            }
            if let Some(err) = &rt.last_error {
                return format!("disconnected ({})", err);
            }
        }
        "not signed in (optional; configure via BEATLAB_SLSK_USER/BEATLAB_SLSK_PASS env vars)".to_string()
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<Track> {
        let writer = {
            let rt = self.runtime.read().await;
            if !rt.connected {
                return vec![];
            }
            rt.writer.clone()
        };
        let Some(writer) = writer else {
            return vec![];
        };

        let token = self.next_search_token().await;
        let (tx, mut rx) = mpsc::channel::<Vec<SlskSearchHit>>(32);
        self.pending_searches.lock().await.insert(token, tx);

        let frame = build_search_frame(token, query);
        if writer.send(frame).await.is_err() {
            self.pending_searches.lock().await.remove(&token);
            return vec![];
        }

        let mut collected: Vec<SlskSearchHit> = Vec::new();
        let deadline = tokio::time::sleep(Duration::from_millis(4500));
        tokio::pin!(deadline);

        loop {
            tokio::select! {
                Some(batch) = rx.recv() => {
                    collected.extend(batch);
                    if collected.len() >= limit * 4 {
                        break;
                    }
                }
                _ = &mut deadline => break,
            }
        }

        self.pending_searches.lock().await.remove(&token);
        // Group hits by song_key, prefer peers with a free slot and higher upload speed,
        // and retain up to 4 alternate peer URLs per song_key for download fallback.
        let mut by_key: HashMap<String, Vec<SlskSearchHit>> = HashMap::new();
        for hit in collected {
            by_key.entry(hit.track.song_key()).or_default().push(hit);
        }

        let mut out: Vec<SlskSearchHit> = Vec::with_capacity(by_key.len());
        let mut alt_updates: Vec<(String, Vec<String>)> = Vec::new();
        for (key, mut hits) in by_key {
            hits.sort_by_key(|h| {
                std::cmp::Reverse((h.slot_free as u8, h.track.quality_score(), h.avg_speed))
            });
            let urls: Vec<String> = hits
                .iter()
                .map(|h| h.track.stream_url.clone())
                .take(4)
                .collect();
            if urls.len() > 1 {
                alt_updates.push((key, urls));
            }
            if let Some(best) = hits.into_iter().next() {
                out.push(best);
            }
        }

        if !alt_updates.is_empty() {
            let mut alts = self.alt_streams.lock().await;
            if alts.len() >= 2048 {
                alts.clear();
            }
            for (k, urls) in alt_updates {
                alts.insert(k, urls);
            }
        }

        out.sort_by_key(|h| {
            std::cmp::Reverse((h.slot_free as u8, h.track.quality_score(), h.avg_speed))
        });
        out.into_iter().take(limit).map(|h| h.track).collect()
    }

    async fn resolve_stream(&self, stream_ref: &str) -> Option<String> {
        let arc = self.self_weak.get().and_then(|w| w.upgrade())?;
        arc.resolve_stream_singleflight(stream_ref).await
    }
}

// --- Binary protocol helpers ---

fn normalize_slsk_path(path: &str) -> String {
    path.trim().replace('\\', "/").to_lowercase()
}

fn build_queue_upload_frame(remote_path: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(8 + remote_path.len());
    p.extend_from_slice(&PEER_MSG_QUEUE_UPLOAD.to_le_bytes());
    write_str(&mut p, remote_path);
    wrap_len(&p)
}

fn build_transfer_response_frame(token: u32, allowed: bool, reason: Option<&str>) -> Vec<u8> {
    let mut p = Vec::with_capacity(16);
    p.extend_from_slice(&PEER_MSG_TRANSFER_RESPONSE.to_le_bytes());
    p.extend_from_slice(&token.to_le_bytes());
    p.push(if allowed { 1 } else { 0 });
    if !allowed {
        write_str(&mut p, reason.unwrap_or("Cancelled"));
    }
    wrap_len(&p)
}

fn build_connect_to_peer_frame(token: u32, username: &str, conn_type: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(16 + username.len() + conn_type.len());
    p.extend_from_slice(&MSG_CONNECT_TO_PEER.to_le_bytes());
    p.extend_from_slice(&token.to_le_bytes());
    write_str(&mut p, username);
    write_str(&mut p, conn_type);
    wrap_len(&p)
}

fn sanitize_slsk_ext(remote_path: &str) -> String {
    let raw = remote_path
        .rsplit(['/', '\\'])
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext)
        .unwrap_or("mp3");
    let clean: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>()
        .to_lowercase();
    if clean.is_empty() {
        "mp3".to_string()
    } else {
        clean
    }
}

fn hash_str(s: &str) -> String {
    let mut h = Md5::new();
    h.update(s.as_bytes());
    hex::encode(&h.finalize()[..10])
}

fn wrap_len(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn write_str(buf: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
    buf.extend_from_slice(b);
}

fn build_login_frame(username: &str, password: &str) -> Vec<u8> {
    let mut hasher = Md5::new();
    hasher.update(format!("{}{}", username, password).as_bytes());
    let md5_hex = hex::encode(hasher.finalize());

    let mut p = Vec::new();
    p.extend_from_slice(&MSG_LOGIN.to_le_bytes());
    write_str(&mut p, username);
    write_str(&mut p, password);
    p.extend_from_slice(&160u32.to_le_bytes());
    write_str(&mut p, &md5_hex);
    p.extend_from_slice(&1u32.to_le_bytes());
    wrap_len(&p)
}

fn build_empty_frame(code: u32) -> Vec<u8> {
    wrap_len(&code.to_le_bytes())
}

fn build_u32_frame(code: u32, val: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(8);
    p.extend_from_slice(&code.to_le_bytes());
    p.extend_from_slice(&val.to_le_bytes());
    wrap_len(&p)
}

fn build_u8_frame(code: u32, val: u8) -> Vec<u8> {
    let mut p = Vec::with_capacity(5);
    p.extend_from_slice(&code.to_le_bytes());
    p.push(val);
    wrap_len(&p)
}

fn build_string_frame(code: u32, s: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(8 + s.len());
    p.extend_from_slice(&code.to_le_bytes());
    write_str(&mut p, s);
    wrap_len(&p)
}

fn build_shared_folders_frame(dirs: u32, files: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&MSG_SHARED_FOLDERS_FILES.to_le_bytes());
    p.extend_from_slice(&dirs.to_le_bytes());
    p.extend_from_slice(&files.to_le_bytes());
    wrap_len(&p)
}

fn build_search_frame(token: u32, query: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(12 + query.len());
    p.extend_from_slice(&MSG_FILE_SEARCH.to_le_bytes());
    p.extend_from_slice(&token.to_le_bytes());
    write_str(&mut p, query);
    wrap_len(&p)
}

async fn read_frame<R: AsyncReadExt + Unpin>(rd: &mut R) -> Result<(u32, Vec<u8>), String> {
    let mut len_buf = [0u8; 4];
    rd.read_exact(&mut len_buf)
        .await
        .map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if !(4..=32 * 1024 * 1024).contains(&len) {
        return Err(format!("invalid frame length {}", len));
    }
    let mut body = vec![0u8; len];
    rd.read_exact(&mut body).await.map_err(|e| e.to_string())?;
    let code = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    Ok((code, body[4..].to_vec()))
}

async fn read_raw_peer_init(sock: &mut TcpStream) -> Result<(u8, Vec<u8>), String> {
    let mut len_buf = [0u8; 4];
    sock.read_exact(&mut len_buf)
        .await
        .map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if !(1..=1024 * 1024).contains(&len) {
        return Err("invalid peer init length".to_string());
    }
    let mut body = vec![0u8; len];
    sock.read_exact(&mut body)
        .await
        .map_err(|e| e.to_string())?;
    Ok((body[0], body[1..].to_vec()))
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn read_u8(&mut self) -> Option<u8> {
        let b = *self.buf.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn read_u32(&mut self) -> Option<u32> {
        let s = self.buf.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn read_u64(&mut self) -> Option<u64> {
        let s = self.buf.get(self.pos..self.pos + 8)?;
        self.pos += 8;
        Some(u64::from_le_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]))
    }

    fn read_string(&mut self) -> Option<String> {
        let len = self.read_u32()? as usize;
        let s = self.buf.get(self.pos..self.pos + len)?;
        self.pos += len;
        Some(String::from_utf8_lossy(s).to_string())
    }
}

fn parse_search_response(compressed: &[u8], peer_hint: Option<&str>) -> Option<(u32, Vec<SlskSearchHit>)> {
    let decoder = ZlibDecoder::new(compressed);
    let mut raw = Vec::new();
    decoder
        .take(16 * 1024 * 1024)
        .read_to_end(&mut raw)
        .ok()?;

    let mut cur = Cursor::new(&raw);
    let peer = cur
        .read_string()
        .filter(|s| !s.is_empty())
        .or_else(|| peer_hint.map(|s| s.to_string()))?;
    let token = cur.read_u32()?;
    let count = cur.read_u32()? as usize;

    let mut parsed_tracks = Vec::new();
    for _ in 0..count.min(200) {
        let _code = cur.read_u8()?;
        let filename = cur.read_string()?;
        let size = cur.read_u64()?;
        let ext_raw = cur.read_string()?;
        let attr_count = cur.read_u32()? as usize;

        let mut bitrate = 0i32;
        let mut duration = 0i32;
        let mut sample_rate = 44100i32;
        let mut bit_depth = 16i32;

        for _ in 0..attr_count.min(16) {
            let k = cur.read_u32()?;
            let v = cur.read_u32()? as i32;
            match k {
                0 => bitrate = v,
                1 => duration = v,
                4 if v > 0 => {
                    sample_rate = v;
                }
                5 if v > 0 => {
                    bit_depth = v;
                }
                _ => {}
            }
        }

        if duration > 0 && duration < 45 {
            continue;
        }

        let ext = ext_raw
            .trim_start_matches('.')
            .to_lowercase()
            .if_empty_from_path(&filename);

        if !matches!(
            ext.as_str(),
            "flac" | "wav" | "aiff" | "alac" | "mp3" | "ogg" | "opus" | "m4a" | "aac"
        ) {
            continue;
        }

        let (artist, album, title, track_no) = parse_slsk_path(&filename);
        let stream_url = format!("slsk://{}/{}", peer, filename);
        let id = format!("soulseek:{}", hash_str(&stream_url));
        let moods = classify_moods_str(&title, &album, "", "");

        parsed_tracks.push(Track {
            id,
            source: "soulseek".to_string(),
            title,
            artist,
            album,
            genre: String::new(),
            moods,
            year: 0,
            track_no,
            disc_no: 1,
            duration,
            bitrate,
            sample_rate,
            bit_depth,
            format: ext,
            size: size as i64,
            stream_url,
            cover_url: String::new(),
            meta: filename,
            deezer_id: String::new(),
            created: chrono::Utc::now().timestamp(),
        });
    }

    let slot_free = cur.read_u8().unwrap_or(0) > 0;
    let avg_speed = cur.read_u32().unwrap_or(0);
    let hits = parsed_tracks
        .into_iter()
        .map(|track| SlskSearchHit {
            track,
            slot_free,
            avg_speed,
        })
        .collect();

    Some((token, hits))
}

trait StrExt {
    fn if_empty_from_path(self, path: &str) -> String;
}

impl StrExt for String {
    fn if_empty_from_path(self, path: &str) -> String {
        if !self.is_empty() {
            return self;
        }
        Path::new(&path.replace('\\', "/"))
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp3")
            .to_lowercase()
    }
}

fn is_disc_folder(s: &str) -> bool {
    let low = s.trim().to_ascii_lowercase();
    for p in ["disc", "disk", "cd"] {
        if let Some(rest) = low.strip_prefix(p) {
            let t = rest.trim();
            if !t.is_empty() && t.len() <= 3 && t.chars().all(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

fn normalize_slsk_separators(s: &str) -> String {
    let mut work = s.replace('_', " ");
    let trimmed = work.trim();
    if !trimmed.contains(' ') && trimmed.contains('.') {
        let segs: Vec<&str> = trimmed.split('.').filter(|p| !p.is_empty()).collect();
        let has_single_letter_initial = segs
            .iter()
            .any(|p| p.chars().count() == 1 && p.chars().all(|c| c.is_alphanumeric()));
        let has_abbr = segs.first().is_some_and(|p| {
            matches!(
                p.to_ascii_lowercase().as_str(),
                "mr" | "ms" | "mrs" | "dr" | "st" | "jr" | "sr" | "vs" | "ft" | "pt" | "vol"
            )
        });
        if segs.len() >= 2 && !has_single_letter_initial && !has_abbr {
            work = trimmed.replace('.', " ");
        }
    }
    work.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_leading_track_no(stem: &str) -> (i32, &str) {
    let mut s = stem.trim();
    // Handle optional "Track 02 - ..." or "Tr. 02 ..." prefix
    let low = s.to_ascii_lowercase();
    for prefix in ["track ", "tr. ", "tr "] {
        if low.starts_with(prefix) && s.is_char_boundary(prefix.len()) {
            let after = s[prefix.len()..].trim_start();
            if after.bytes().next().is_some_and(|b| b.is_ascii_digit()) {
                s = after;
                break;
            }
        }
    }
    // Handle disc-track like "1-02 Track" or "01 - Track" or "01. Track" or "01 Track"
    let bytes = s.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx > 0 && idx <= 3 && idx < bytes.len() {
        // Optional disc prefix like "1-02"
        if (bytes[idx] == b'-' || bytes[idx] == b'.')
            && bytes.get(idx + 1).is_some_and(|b| b.is_ascii_digit())
        {
            let mut j = idx + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j - (idx + 1) <= 3 && j < bytes.len() {
                if let Some(&sep) = bytes.get(j) {
                    if matches!(sep, b'.' | b'-' | b'_' | b' ') {
                        let tno = s[idx + 1..j].parse::<i32>().unwrap_or(0);
                        let rest = s[j..].trim_start_matches(['.', '-', '_', ' ']).trim();
                        if !rest.is_empty() {
                            return (tno, rest);
                        }
                    }
                }
            }
        }
        if matches!(bytes[idx], b'.' | b'-' | b'_' | b' ') {
            let tno = s[..idx].parse::<i32>().unwrap_or(0);
            let rest = s[idx..].trim_start_matches(['.', '-', '_', ' ']).trim();
            if !rest.is_empty() {
                return (tno, rest);
            }
        }
    }
    (0, stem.trim())
}

fn strip_leading_label_prefix(title: &str, label: &str) -> Option<String> {
    let lbl = label.trim();
    if lbl.is_empty() || title.len() <= lbl.len() {
        return None;
    }
    let t_low = title.to_lowercase();
    let l_low = lbl.to_lowercase();
    if !t_low.starts_with(&l_low) || !title.is_char_boundary(lbl.len()) {
        return None;
    }
    let after = &title[lbl.len()..];
    let first = after.chars().next()?;
    if !matches!(first, ' ' | '-' | '_' | ':' | '|') {
        return None;
    }
    let rest = after
        .trim_start_matches([' ', '-', '_', ':', '|', '\u{2013}', '\u{2014}'])
        .trim();
    if rest.is_empty() {
        return None;
    }
    Some(rest.to_string())
}

/// Cleans Soulseek-style prefixes (`Artist - ...`, `Album - ...`, `Track 02 - ...`, `02 - ...`,
/// underscores/dots) from `(artist, album, title)` for display and fallback queries.
pub fn clean_slsk_title_and_artist(
    raw_artist: &str,
    raw_album: &str,
    raw_title: &str,
) -> (String, String, i32) {
    let mut artist = crate::models::primary_artist(&normalize_slsk_separators(raw_artist));
    if crate::models::is_generic_folder_name(&artist) {
        artist.clear();
    }
    let album = crate::models::strip_release_tags(&normalize_slsk_separators(raw_album));
    let mut title = crate::models::strip_release_tags(&normalize_slsk_separators(raw_title));
    if title.is_empty() {
        title = normalize_slsk_separators(raw_title);
    }

    let mut track_no = 0i32;
    for _ in 0..3 {
        let prev = title.clone();
        if track_no == 0 {
            let (tno, rest) = split_leading_track_no(&title);
            if tno > 0 {
                track_no = tno;
                let stripped = crate::models::strip_release_tags(rest);
                let next = if stripped.is_empty() {
                    rest.to_string()
                } else {
                    stripped
                };
                title = next;
            }
        }
        if let Some((left, right)) = title.split_once(" - ") {
            let la = crate::models::primary_artist(&normalize_slsk_separators(left));
            let rt = crate::models::strip_release_tags(right);
            if !la.is_empty()
                && !rt.is_empty()
                && !crate::models::is_generic_folder_name(&la)
                && (artist.is_empty() || artist.eq_ignore_ascii_case(&la))
            {
                artist = la;
                title = rt;
            } else if !album.is_empty() && !rt.is_empty() && left.trim().eq_ignore_ascii_case(&album)
            {
                title = rt;
            }
        }
        if !artist.is_empty() {
            if let Some(rest) = strip_leading_label_prefix(&title, &artist) {
                title = rest;
            }
        }
        if !album.is_empty() {
            if let Some(rest) = strip_leading_label_prefix(&title, &album) {
                title = rest;
            }
        }
        if title == prev {
            break;
        }
    }

    if track_no == 0 {
        let (tno, rest) = split_leading_track_no(&title);
        if tno > 0 {
            track_no = tno;
            title = rest.to_string();
        }
    }

    if artist.is_empty() {
        artist = "Unknown Artist".to_string();
    }
    (artist, title, track_no)
}

/// Parses a Soulseek virtual path into `(artist, album, title, track_no)`, cleaning share
/// bucket folders (`complete`, `music`), release tags (`[2020.07.22]`, `[FLAC]`, `[9a149]`),
/// and disc subfolders (`CD1`, `Disc 1`).
pub fn parse_slsk_path(raw: &str) -> (String, String, String, i32) {
    let norm = raw.replace('\\', "/");
    let mut dirs: Vec<&str> = norm
        .split('/')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && !s.starts_with("@@"))
        .collect();
    let fname = dirs.pop().unwrap_or("Unknown Track");
    let stem = Path::new(fname)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(fname)
        .trim();

    while dirs.last().is_some_and(|d| is_disc_folder(d)) {
        dirs.pop();
    }
    while dirs
        .first()
        .is_some_and(|d| crate::models::is_generic_folder_name(d))
    {
        dirs.remove(0);
    }

    let raw_album = dirs.pop().unwrap_or("");
    let raw_artist = dirs
        .iter()
        .rev()
        .find(|d| !crate::models::is_generic_folder_name(d))
        .copied()
        .unwrap_or("");

    let mut artist = crate::models::primary_artist(&normalize_slsk_separators(raw_artist));
    if crate::models::is_generic_folder_name(&artist) {
        artist.clear();
    }

    let mut album = crate::models::strip_release_tags(&normalize_slsk_separators(raw_album));

    // Extract "Artist | Album" or "Artist - Album" from folder name when appropriate
    if let Some((left, right)) = album.split_once(" | ").or_else(|| album.split_once(" - ")) {
        let la = crate::models::primary_artist(left);
        let ra = crate::models::strip_release_tags(right);
        if !la.is_empty()
            && !ra.is_empty()
            && !crate::models::is_generic_folder_name(&la)
            && !la.eq_ignore_ascii_case("cd")
            && (artist.is_empty() || artist.eq_ignore_ascii_case(&la))
        {
            artist = la;
            album = ra;
        }
    }

    let (clean_artist, title, track_no) = clean_slsk_title_and_artist(&artist, &album, stem);
    if album.is_empty() {
        album = title.clone();
    }

    (clean_artist, album, title, track_no)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    #[test]
    fn login_frame_has_expected_structure() {
        let frame = build_login_frame("alice", "secret");
        let len = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(len + 4, frame.len());
        let code = u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
        assert_eq!(code, MSG_LOGIN);
    }

    #[test]
    fn parse_search_response_extracts_flac_track() {
        let mut raw = Vec::new();
        write_str(&mut raw, "peer1");
        raw.extend_from_slice(&42u32.to_le_bytes()); // token
        raw.extend_from_slice(&1u32.to_le_bytes()); // 1 file
        raw.push(1);
        write_str(&mut raw, "@@peer1\\Music\\Miles Davis\\Kind of Blue\\01 - So What.flac");
        raw.extend_from_slice(&35_000_000u64.to_le_bytes());
        write_str(&mut raw, "flac");
        raw.extend_from_slice(&3u32.to_le_bytes()); // 3 attrs
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(&1411u32.to_le_bytes());
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.extend_from_slice(&562u32.to_le_bytes());
        raw.extend_from_slice(&4u32.to_le_bytes());
        raw.extend_from_slice(&96000u32.to_le_bytes());

        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&raw).unwrap();
        let compressed = enc.finish().unwrap();

        let (tok, hits) = parse_search_response(&compressed, None).unwrap();
        assert_eq!(tok, 42);
        assert_eq!(hits.len(), 1);
        let t = &hits[0].track;
        assert_eq!(t.source, "soulseek");
        assert_eq!(t.artist, "Miles Davis");
        assert_eq!(t.album, "Kind of Blue");
        assert_eq!(t.title, "So What");
        assert_eq!(t.track_no, 1);
        assert_eq!(t.format, "flac");
        assert_eq!(t.sample_rate, 96000);
        assert!(t.is_lossless());
    }

    #[test]
    fn parse_slsk_path_strips_bucket_folders_and_release_tags() {
        let (a1, alb1, t1, n1) = parse_slsk_path(
            "@@peer\\complete\\[2020.07.22] ReoNa | ANIMA [FLAC]\\02 - ANIMA.flac",
        );
        assert_eq!(a1, "ReoNa");
        assert_eq!(alb1, "ANIMA");
        assert_eq!(t1, "ANIMA");
        assert_eq!(n1, 2);

        let (a2, alb2, t2, n2) = parse_slsk_path(
            "@@peer\\ReoNa [9a149]\\[2023][XW] HUMAN (SAO) [24bit-48kHz]\\CD1\\01. HUMAN.flac",
        );
        assert_eq!(a2, "ReoNa");
        assert_eq!(alb2, "HUMAN (SAO)");
        assert_eq!(t2, "HUMAN");
        assert_eq!(n2, 1);
    }

    #[test]
    fn parse_slsk_path_normalizes_messy_album_track_and_dot_variants() {
        let cases = [
            "@@p1\\Music\\Porter Robinson\\Nurture\\Nurture - Look at the Sky.flac",
            "@@p2\\Music\\Porter Robinson\\Nurture\\Track 02 - Look at the Sky.flac",
            "@@p3\\Music\\Porter.Robinson\\Nurture\\Porter.Robinson.-.02.-.Look.at.the.Sky.flac",
            "@@p4\\Music\\Porter Robinson\\Nurture\\Porter Robinson_Nurture_02_Look at the Sky.flac",
            "@@p5\\Music\\Porter Robinson\\Nurture\\Nurture - 02 Look at the Sky.flac",
        ];
        for p in cases {
            let (art, alb, title, _) = parse_slsk_path(p);
            assert_eq!(art, "Porter Robinson", "path={}", p);
            assert_eq!(alb, "Nurture", "path={}", p);
            assert_eq!(title, "Look at the Sky", "path={}", p);
        }

        // Preserve dot initials like R.E.M.
        let (rem_art, rem_alb, rem_title, rem_no) = parse_slsk_path(
            "@@p6\\Music\\R.E.M.\\Automatic for the People\\03 - Everybody Hurts.flac",
        );
        assert_eq!(rem_art, "R.E.M.");
        assert_eq!(rem_alb, "Automatic for the People");
        assert_eq!(rem_title, "Everybody Hurts");
        assert_eq!(rem_no, 3);
    }

    #[test]
    fn sanitize_slsk_ext_strips_traversal_and_punctuation() {
        assert_eq!(sanitize_slsk_ext("@@peer\\song.FLAC"), "flac");
        assert_eq!(sanitize_slsk_ext("folder/track.mp3?.."), "mp3");
        assert_eq!(sanitize_slsk_ext("no_extension"), "mp3");
        assert_eq!(sanitize_slsk_ext("song.???"), "mp3");
    }

    #[tokio::test]
    async fn execute_transfer_supports_queue_upload_transfer_request_dir1_and_f_socket() {
        let dir = std::env::temp_dir().join(format!("beatlab_slsk_xfer_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open_memory().unwrap();
        let slsk = SoulseekSource::new(store, dir.join("dl"));

        // Spin up a mock Soulseek peer that speaks the modern QueueUpload -> TransferRequest(dir=1) protocol.
        let peer_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer_port = peer_listener.local_addr().unwrap().port() as u32;
        let payload_bytes = vec![0x66u8; 8192]; // fake FLAC payload > 4096 bytes
        let expected_bytes = payload_bytes.clone();
        let remote_path = "@@mockpeer\\Music\\Porter Robinson\\Nurture\\02 - Look at the Sky.flac".to_string();

        {
            let mut rt = slsk.runtime.write().await;
            rt.connected = true;
            rt.username = Some("beatlab_test".to_string());
        }
        slsk.cache_peer_addr("mockpeer".to_string(), Some((Ipv4Addr::LOCALHOST, peer_port)))
            .await;

        let slsk_srv = Arc::clone(&slsk);
        let remote_clone = remote_path.clone();
        let peer_task = tokio::spawn(async move {
            let (mut psock, _) = peer_listener.accept().await.unwrap();
            let (first, _rest) = read_raw_peer_init(&mut psock).await.unwrap();
            assert_eq!(first, PEER_INIT);

            // Expect QueueUpload (43)
            let (code, q_payload) = read_frame(&mut psock).await.unwrap();
            assert_eq!(code, PEER_MSG_QUEUE_UPLOAD);
            let mut cur = Cursor::new(&q_payload);
            assert_eq!(cur.read_string().as_deref(), Some(remote_clone.as_str()));

            // Reply with TransferRequest(direction=1, upload_token=777, filename, size)
            let upload_token = 777u32;
            let mut tr = Vec::new();
            tr.extend_from_slice(&PEER_MSG_TRANSFER_REQUEST.to_le_bytes());
            tr.extend_from_slice(&1u32.to_le_bytes());
            tr.extend_from_slice(&upload_token.to_le_bytes());
            write_str(&mut tr, &remote_clone);
            tr.extend_from_slice(&(expected_bytes.len() as u64).to_le_bytes());
            psock.write_all(&wrap_len(&tr)).await.unwrap();

            // Expect TransferResponse(41, upload_token, allowed=1)
            let (resp_code, resp_payload) = read_frame(&mut psock).await.unwrap();
            assert_eq!(resp_code, PEER_MSG_TRANSFER_RESPONSE);
            let mut rcur = Cursor::new(&resp_payload);
            assert_eq!(rcur.read_u32(), Some(upload_token));
            assert_eq!(rcur.read_u8(), Some(1));

            // Simulate uploader connecting "F" (or PierceFirewall "F") and handing socket to slsk
            let f_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let f_addr = f_listener.local_addr().unwrap();
            let uploader_f = tokio::spawn(async move {
                let (mut fsock, _) = f_listener.accept().await.unwrap();
                let mut init = Vec::new();
                init.push(PEER_INIT);
                write_str(&mut init, "mockpeer");
                write_str(&mut init, "F");
                init.extend_from_slice(&0u32.to_le_bytes());
                fsock.write_all(&wrap_len(&init)).await.unwrap();
                fsock.write_all(&upload_token.to_le_bytes()).await.unwrap();
                let mut off = [0u8; 8];
                fsock.read_exact(&mut off).await.unwrap();
                assert_eq!(u64::from_le_bytes(off), 0);
                fsock.write_all(&expected_bytes).await.unwrap();
            });
            let incoming = TcpStream::connect(f_addr).await.unwrap();
            slsk_srv.handle_incoming_peer(incoming).await;
            uploader_f.await.unwrap();
        });

        let stream_ref = format!("slsk://mockpeer/{}", remote_path);
        let resolved = slsk.resolve_stream(&stream_ref).await.expect("expected download to succeed");
        let saved_path = PathBuf::from(resolved.strip_prefix("file:").unwrap());
        assert_eq!(std::fs::read(&saved_path).unwrap(), payload_bytes);
        peer_task.await.unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn pierce_firewall_file_transfer_works_behind_nat() {
        let dir = std::env::temp_dir().join(format!("beatlab_slsk_pierce_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open_memory().unwrap();
        let slsk = SoulseekSource::new(store, dir.join("dl"));

        let peer_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer_port = peer_listener.local_addr().unwrap().port() as u32;
        let payload_bytes = vec![0x42u8; 6144];
        let expected_bytes = payload_bytes.clone();
        let remote_path = "@@natpeer\\Music\\Porter Robinson\\Nurture\\02 - Look at the Sky.flac".to_string();

        {
            let mut rt = slsk.runtime.write().await;
            rt.connected = true;
            rt.username = Some("beatlab_nat".to_string());
        }
        slsk.cache_peer_addr("natpeer".to_string(), Some((Ipv4Addr::LOCALHOST, peer_port)))
            .await;

        let slsk_srv = Arc::clone(&slsk);
        let remote_clone = remote_path.clone();
        let peer_task = tokio::spawn(async move {
            let (mut psock, _) = peer_listener.accept().await.unwrap();
            let _ = read_raw_peer_init(&mut psock).await.unwrap();
            let (code, _) = read_frame(&mut psock).await.unwrap();
            assert_eq!(code, PEER_MSG_QUEUE_UPLOAD);

            let upload_token = 991u32;
            let mut tr = Vec::new();
            tr.extend_from_slice(&PEER_MSG_TRANSFER_REQUEST.to_le_bytes());
            tr.extend_from_slice(&1u32.to_le_bytes());
            tr.extend_from_slice(&upload_token.to_le_bytes());
            write_str(&mut tr, &remote_clone);
            tr.extend_from_slice(&(expected_bytes.len() as u64).to_le_bytes());
            psock.write_all(&wrap_len(&tr)).await.unwrap();

            let (resp_code, _) = read_frame(&mut psock).await.unwrap();
            assert_eq!(resp_code, PEER_MSG_TRANSFER_RESPONSE);

            // Simulate downloader behind NAT: uploader listens on f_listener and server sends
            // MSG_CONNECT_TO_PEER("F") with pierce_token = 555.
            let f_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let f_port = f_listener.local_addr().unwrap().port() as u32;
            let pierce_token = 555u32;

            let mut msg = Vec::new();
            write_str(&mut msg, "natpeer");
            write_str(&mut msg, "F");
            msg.extend_from_slice(&u32::from(Ipv4Addr::LOCALHOST).to_le_bytes());
            msg.extend_from_slice(&f_port.to_le_bytes());
            msg.extend_from_slice(&pierce_token.to_le_bytes());
            let (dummy_tx, _dummy_rx) = mpsc::channel(4);
            slsk_srv
                .handle_server_message(MSG_CONNECT_TO_PEER, &msg, &dummy_tx)
                .await;

            let (mut fsock, _) = f_listener.accept().await.unwrap();
            let (pfirst, prest) = read_raw_peer_init(&mut fsock).await.unwrap();
            assert_eq!(pfirst, PEER_PIERCE_FIREWALL);
            let mut pcur = Cursor::new(&prest);
            assert_eq!(pcur.read_u32(), Some(pierce_token));

            fsock.write_all(&upload_token.to_le_bytes()).await.unwrap();
            let mut off = [0u8; 8];
            fsock.read_exact(&mut off).await.unwrap();
            assert_eq!(u64::from_le_bytes(off), 0);
            fsock.write_all(&expected_bytes).await.unwrap();
        });

        let stream_ref = format!("slsk://natpeer/{}", remote_path);
        let resolved = slsk.resolve_stream(&stream_ref).await.expect("expected pierce download to succeed");
        let saved_path = PathBuf::from(resolved.strip_prefix("file:").unwrap());
        assert_eq!(std::fs::read(&saved_path).unwrap(), payload_bytes);
        peer_task.await.unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
