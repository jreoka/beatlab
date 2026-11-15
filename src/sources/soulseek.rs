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
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock, mpsc};
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

pub struct SoulseekSource {
    store: Store,
    download_dir: PathBuf,
    runtime: RwLock<SlskRuntime>,
    pending_searches: Mutex<HashMap<u32, mpsc::Sender<Vec<SlskSearchHit>>>>,
    peer_addrs: Mutex<HashMap<String, (Ipv4Addr, u32)>>,
    next_token: Mutex<u32>,
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
            next_token: Mutex::new(1000),
        });

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

    pub fn connection_info(&self) -> serde_json::Value {
        if let Ok(rt) = self.runtime.try_read() {
            serde_json::json!({
                "connected": rt.connected,
                "connecting": rt.connecting,
                "username": rt.username,
                "server": rt.server,
                "last_error": rt.last_error,
            })
        } else {
            serde_json::json!({
                "connected": false,
                "connecting": true,
                "username": null,
                "server": DEFAULT_SERVER,
                "last_error": null,
            })
        }
    }

    pub async fn disconnect(&self) {
        let mut rt = self.runtime.write().await;
        rt.connected = false;
        rt.connecting = false;
        rt.writer = None;
        rt.last_error = None;
        rt.session_id = rt.session_id.wrapping_add(1);
        self.store.kv_set(KV_PASS, "");
        info!("soulseek disconnected");
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

        // Bind local peer listener on ephemeral port
        let listener = TcpListener::bind("0.0.0.0:0").await.ok();
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
                    let ip = Ipv4Addr::from(ip_raw);
                    self.cache_peer_addr(peer, ip, port).await;
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
                    let ip = Ipv4Addr::from(ip_raw);
                    self.cache_peer_addr(peer.clone(), ip, port).await;
                    if conn_type == "P" {
                        let sp = Arc::clone(self);
                        tokio::spawn(async move {
                            sp.connect_peer_pierce(&peer, ip, port, token).await;
                        });
                    }
                }
            }
            _ => {
                debug!(code, len = data.len(), "ignored soulseek server message");
            }
        }
    }

    async fn connect_peer_pierce(&self, peer: &str, ip: Ipv4Addr, port: u32, token: u32) {
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

    async fn cache_peer_addr(&self, peer: String, ip: Ipv4Addr, port: u32) {
        let mut guard = self.peer_addrs.lock().await;
        if guard.len() >= 4096 {
            guard.clear();
        }
        guard.insert(peer, (ip, port));
    }

    async fn handle_incoming_peer(&self, mut sock: TcpStream) {
        let Ok(Ok((first_byte, rest))) =
            tokio::time::timeout(Duration::from_secs(5), read_raw_peer_init(&mut sock)).await
        else {
            return;
        };
        let peer_name = if first_byte == PEER_INIT {
            let mut cur = Cursor::new(&rest);
            cur.read_string()
        } else {
            None
        };
        self.read_peer_messages(&mut sock, peer_name).await;
    }

    async fn read_peer_messages(&self, sock: &mut TcpStream, peer_hint: Option<String>) {
        loop {
            let Ok(Ok((code, payload))) =
                tokio::time::timeout(Duration::from_secs(10), read_frame(sock)).await
            else {
                break;
            };
            if code == PEER_MSG_SEARCH_RESPONSE {
                if let Some((token, hits)) =
                    parse_search_response(&payload, peer_hint.as_deref())
                {
                    let sender = self.pending_searches.lock().await.get(&token).cloned();
                    if let Some(tx) = sender {
                        let _ = tx.send(hits).await;
                    }
                }
            }
        }
    }

    async fn next_search_token(&self) -> u32 {
        let mut tok = self.next_token.lock().await;
        *tok = tok.wrapping_add(1);
        *tok
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
        "not signed in (configure in /admin)".to_string()
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
        // Prefer peers with a free upload slot and higher upload speed when deduping by song_key.
        let mut best_by_key: HashMap<String, SlskSearchHit> = HashMap::new();
        for hit in collected {
            let key = hit.track.song_key();
            let hit_rank = (hit.slot_free as u8, hit.track.quality_score(), hit.avg_speed);
            match best_by_key.get(&key) {
                Some(cur)
                    if (cur.slot_free as u8, cur.track.quality_score(), cur.avg_speed) >= hit_rank => {}
                _ => {
                    best_by_key.insert(key, hit);
                }
            }
        }
        let mut out: Vec<SlskSearchHit> = best_by_key.into_values().collect();
        out.sort_by_key(|h| {
            std::cmp::Reverse((h.slot_free as u8, h.track.quality_score(), h.avg_speed))
        });
        out.into_iter().take(limit).map(|h| h.track).collect()
    }

    async fn resolve_stream(&self, stream_ref: &str) -> Option<String> {
        let rest = stream_ref.strip_prefix("slsk://")?;
        let (peer, remote_path) = rest.split_once('/')?;

        // Request peer address from server if not cached
        if !self.peer_addrs.lock().await.contains_key(peer) {
            if let Some(writer) = self.runtime.read().await.writer.clone() {
                let _ = writer
                    .send(build_string_frame(MSG_GET_PEER_ADDRESS, peer))
                    .await;
                for _ in 0..10 {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    if self.peer_addrs.lock().await.contains_key(peer) {
                        break;
                    }
                }
            }
        }

        let (ip, port) = self.peer_addrs.lock().await.get(peer).copied()?;
        let addr = format!("{}:{}", ip, port);
        let mut sock = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(&addr))
            .await
            .ok()?
            .ok()?;

        let my_user = self
            .runtime
            .read()
            .await
            .username
            .clone()
            .unwrap_or_default();
        let token = self.next_search_token().await;

        // Send PeerInit ("P")
        let mut init = Vec::new();
        init.push(PEER_INIT);
        write_str(&mut init, &my_user);
        write_str(&mut init, "P");
        init.extend_from_slice(&token.to_le_bytes());
        sock.write_all(&wrap_len(&init)).await.ok()?;

        // Send QueueUpload (code 43) then TransferRequest (code 40)
        let mut q = Vec::new();
        q.extend_from_slice(&PEER_MSG_QUEUE_UPLOAD.to_le_bytes());
        write_str(&mut q, remote_path);
        sock.write_all(&wrap_len(&q)).await.ok()?;

        let mut tr = Vec::new();
        tr.extend_from_slice(&PEER_MSG_TRANSFER_REQUEST.to_le_bytes());
        tr.extend_from_slice(&0u32.to_le_bytes()); // direction: 0 = download
        tr.extend_from_slice(&token.to_le_bytes());
        write_str(&mut tr, remote_path);
        sock.write_all(&wrap_len(&tr)).await.ok()?;

        // Wait for TransferResponse (code 41); fail fast if peer queues/denies so fallback stays responsive.
        let (code, payload) = tokio::time::timeout(Duration::from_millis(3500), read_frame(&mut sock))
            .await
            .ok()?
            .ok()?;
        if code != PEER_MSG_TRANSFER_RESPONSE {
            return None;
        }
        let mut cur = Cursor::new(&payload);
        let _resp_tok = cur.read_u32()?;
        let allowed = cur.read_u8()?;
        if allowed == 0 {
            return None;
        }
        let size = cur.read_u64().unwrap_or(0);
        if size == 0 || size >= 250 * 1024 * 1024 {
            return None;
        }

        // Open dedicated "F" (file transfer) connection to peer
        let mut fsock = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(&addr))
            .await
            .ok()?
            .ok()?;
        let mut finit = Vec::new();
        finit.push(PEER_INIT);
        write_str(&mut finit, &my_user);
        write_str(&mut finit, "F");
        finit.extend_from_slice(&0u32.to_le_bytes());
        fsock.write_all(&wrap_len(&finit)).await.ok()?;
        fsock.write_all(&token.to_le_bytes()).await.ok()?;
        fsock.write_all(&0u64.to_le_bytes()).await.ok()?; // start offset 0

        let _ = std::fs::create_dir_all(&self.download_dir);
        let ext = Path::new(remote_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp3");
        let base_hash = hash_str(stream_ref);
        let local_path = self.download_dir.join(format!("{}.{}", base_hash, ext));
        let part_path = self.download_dir.join(format!("{}.part", base_hash));

        // Stream directly to a .part file with per-chunk idle timeout and verify complete length.
        let mut file = tokio::fs::File::create(&part_path).await.ok()?;
        let mut chunk = [0u8; 32 * 1024];
        let mut written = 0u64;
        while written < size {
            let need = ((size - written) as usize).min(chunk.len());
            let read_res = tokio::time::timeout(
                Duration::from_secs(6),
                fsock.read(&mut chunk[..need]),
            )
            .await;
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
        drop(file);
        if written != size {
            let _ = tokio::fs::remove_file(&part_path).await;
            return None;
        }
        if tokio::fs::rename(&part_path, &local_path).await.is_err() {
            let _ = tokio::fs::remove_file(&part_path).await;
            return None;
        }
        Some(format!("file:{}", local_path.display()))
    }
}

// --- Binary protocol helpers ---

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
    let mut decoder = ZlibDecoder::new(compressed);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).ok()?;

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
            duration,
            bitrate,
            sample_rate,
            bit_depth,
            format: ext,
            size: size as i64,
            stream_url,
            cover_url: String::new(),
            meta: filename,
            play_count: 0,
            last_played: 0,
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

fn split_leading_track_no(stem: &str) -> (i32, &str) {
    let s = stem.trim();
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
    (0, s)
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

    let (track_no, raw_title) = split_leading_track_no(stem);
    let mut title = crate::models::strip_release_tags(raw_title);
    if title.is_empty() {
        title = raw_title.to_string();
    }

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

    let mut artist = crate::models::primary_artist(raw_artist);
    if crate::models::is_generic_folder_name(&artist) {
        artist.clear();
    }

    let mut album = crate::models::strip_release_tags(raw_album);

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

    // Extract "Artist - Title" from filename stem if present
    if let Some((left, right)) = title.split_once(" - ") {
        let la = crate::models::primary_artist(left);
        let rt = crate::models::strip_release_tags(right);
        if !la.is_empty()
            && !rt.is_empty()
            && !crate::models::is_generic_folder_name(&la)
            && (artist.is_empty() || artist.eq_ignore_ascii_case(&la))
        {
            artist = la;
            title = rt;
        }
    }

    if artist.is_empty() {
        artist = "Unknown Artist".to_string();
    }
    if album.is_empty() {
        album = title.clone();
    }

    (artist, album, title, track_no)
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
}
