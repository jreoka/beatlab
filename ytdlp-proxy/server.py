import asyncio
import base64
import hmac
import json
import logging
import os
import re
import secrets
import sys
import tempfile
import time
from dataclasses import dataclass
from typing import Dict, List, Optional, Tuple
from urllib.parse import parse_qs, urlparse

from aiohttp import ClientSession, ClientTimeout, TCPConnector, web

logging.basicConfig(
    level=os.environ.get("LOG_LEVEL", "INFO").upper(),
    format="%(asctime)s %(levelname)s %(name)s: %(message)s",
)
log = logging.getLogger("ytdlp-proxy")

YTDLP_BIN = os.environ.get("YTDLP_BIN", "yt-dlp")
MAX_CONCURRENT = max(1, int(os.environ.get("MAX_CONCURRENT_YTDLP", "4")))
STREAM_CACHE_MAX = max(64, int(os.environ.get("STREAM_CACHE_MAX", "2048")))
RESOLVE_TIMEOUT_SECS = float(os.environ.get("RESOLVE_TIMEOUT_SECS", "25"))
SEARCH_TIMEOUT_SECS = float(os.environ.get("SEARCH_TIMEOUT_SECS", "15"))

ALLOWED_YT_HOSTS = {
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtu.be",
    "www.youtu.be",
}

YTSEARCH_RE = re.compile(r"^ytsearch([1-9]|10):.+$")


@dataclass
class CachedStream:
    url: str
    headers: Dict[str, str]
    expires_at: float


class ProxyApp:
    def __init__(self, token: str, cookies_path: Optional[str]) -> None:
        self.token = token
        self.cookies_path = cookies_path
        self.sem = asyncio.Semaphore(MAX_CONCURRENT)
        self.streams: Dict[str, CachedStream] = {}
        self.ytdlp_version = "unknown"
        self.http: Optional[ClientSession] = None

    def check_auth(self, request: web.Request) -> bool:
        provided = ""
        auth_hdr = request.headers.get("Authorization", "").strip()
        if auth_hdr.lower().startswith("bearer "):
            provided = auth_hdr[7:].strip()
        if not provided:
            provided = request.headers.get("X-Proxy-Token", "").strip()
        if not provided:
            return False
        return hmac.compare_digest(provided.encode("utf-8"), self.token.encode("utf-8"))

    def prune_streams(self, now: Optional[float] = None) -> None:
        now = time.time() if now is None else now
        expired = [k for k, v in self.streams.items() if v.expires_at <= now]
        for k in expired:
            self.streams.pop(k, None)
        if len(self.streams) > STREAM_CACHE_MAX:
            oldest = sorted(self.streams.items(), key=lambda kv: kv[1].expires_at)
            for k, _ in oldest[: len(self.streams) - STREAM_CACHE_MAX]:
                self.streams.pop(k, None)

    def base_ytdlp_args(self) -> List[str]:
        args: List[str] = []
        if self.cookies_path and os.path.isfile(self.cookies_path):
            args.extend(["--cookies", self.cookies_path])
        return args


def validate_youtube_url(raw: str) -> Optional[str]:
    u = raw.strip()
    if not u or len(u) > 512:
        return None
    parsed = urlparse(u)
    if parsed.scheme not in ("http", "https"):
        return None
    host = (parsed.hostname or "").lower()
    if host not in ALLOWED_YT_HOSTS:
        return None
    return u


def parse_upstream_expiry(url: str, now: float) -> float:
    parsed = urlparse(url)
    qs = parse_qs(parsed.query)
    if "expire" in qs and qs["expire"]:
        try:
            exp = float(qs["expire"][0])
            if exp > now + 30:
                return exp
        except ValueError:
            pass
    m = re.search(r"/expire/(\d+)", parsed.path)
    if m:
        try:
            exp = float(m.group(1))
            if exp > now + 30:
                return exp
        except ValueError:
            pass
    return now + 18000.0


def sanitize_extractor_args(raw: str) -> Optional[str]:
    val = raw.strip()
    if not val:
        return ""
    if len(val) > 256 or "\n" in val or "\r" in val or "\x00" in val:
        return None
    return val


def sanitize_format_sel(raw: str) -> Optional[str]:
    val = raw.strip()
    if not val:
        return "bestaudio[vcodec=none][protocol!=m3u8][protocol!=m3u8_native]/bestaudio/best"
    if len(val) > 256 or "\n" in val or "\r" in val or "\x00" in val:
        return None
    return val


def parse_has_video(v: dict) -> bool:
    vc = v.get("vcodec")
    if isinstance(vc, str):
        s = vc.strip()
        return bool(s) and s.lower() != "none"
    return False


async def run_subprocess(args: List[str], timeout_secs: float) -> Tuple[int, bytes, bytes]:
    proc = await asyncio.create_subprocess_exec(
        *args,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    try:
        stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=timeout_secs)
        return proc.returncode or 0, stdout, stderr
    except asyncio.TimeoutError:
        try:
            proc.kill()
        except ProcessLookupError:
            pass
        try:
            await asyncio.wait_for(proc.wait(), timeout=3.0)
        except Exception:
            pass
        raise


@web.middleware
async def auth_middleware(request: web.Request, handler):
    if request.path == "/health":
        return await handler(request)
    app_state: ProxyApp = request.app["proxy"]
    if not app_state.check_auth(request):
        return web.json_response({"error": "Unauthorized"}, status=401)
    return await handler(request)


async def handle_health(request: web.Request) -> web.Response:
    app_state: ProxyApp = request.app["proxy"]
    return web.json_response({"ok": True, "ytdlp": app_state.ytdlp_version})


async def handle_resolve(request: web.Request) -> web.Response:
    app_state: ProxyApp = request.app["proxy"]
    try:
        body = await request.json()
    except Exception:
        return web.json_response({"error": "Invalid JSON body"}, status=400)

    if not isinstance(body, dict):
        return web.json_response({"error": "JSON object required"}, status=400)

    watch_url = validate_youtube_url(str(body.get("url", "")))
    if not watch_url:
        return web.json_response({"error": "Invalid or unsupported URL"}, status=400)

    extractor_args = sanitize_extractor_args(str(body.get("extractor_args", "")))
    if extractor_args is None:
        return web.json_response({"error": "Invalid extractor_args"}, status=400)

    format_sel = sanitize_format_sel(str(body.get("format", "")))
    if format_sel is None:
        return web.json_response({"error": "Invalid format selector"}, status=400)

    args = [
        YTDLP_BIN,
        *app_state.base_ytdlp_args(),
        "-j",
        "--no-playlist",
        "--no-warnings",
    ]
    if extractor_args:
        args.extend(["--extractor-args", extractor_args])
    args.extend(["-f", format_sel, "--", watch_url])

    try:
        await asyncio.wait_for(app_state.sem.acquire(), timeout=12.0)
    except asyncio.TimeoutError:
        return web.json_response({"error": "yt-dlp concurrency limit reached"}, status=503)

    try:
        rc, stdout, stderr = await run_subprocess(args, RESOLVE_TIMEOUT_SECS)
    except asyncio.TimeoutError:
        return web.json_response({"error": "yt-dlp timed out"}, status=504)
    except Exception as exc:
        log.exception("Failed running yt-dlp resolve")
        return web.json_response({"error": f"yt-dlp execution error: {exc}"}, status=502)
    finally:
        app_state.sem.release()

    if rc != 0:
        err = stderr.decode("utf-8", errors="replace").strip()
        log.warning("yt-dlp resolve failed (rc=%d): %s", rc, err[:300])
        return web.json_response(
            {"error": err or "yt-dlp returned non-zero status"},
            status=502,
        )

    text = stdout.decode("utf-8", errors="replace")
    first_json = next((ln for ln in text.splitlines() if ln.lstrip().startswith("{")), None)
    if not first_json:
        return web.json_response({"error": "yt-dlp produced empty JSON"}, status=502)

    try:
        info = json.loads(first_json)
    except json.JSONDecodeError as exc:
        return web.json_response({"error": f"Invalid yt-dlp JSON: {exc}"}, status=502)

    upstream_url = str(info.get("url", "")).strip()
    if not upstream_url or not (
        upstream_url.startswith("http://") or upstream_url.startswith("https://")
    ):
        return web.json_response({"error": "yt-dlp returned non-HTTP stream URL"}, status=502)

    headers: Dict[str, str] = {}
    raw_headers = info.get("http_headers")
    if isinstance(raw_headers, dict):
        for k, v in raw_headers.items():
            if isinstance(k, str) and isinstance(v, str):
                headers[k] = v

    ext = str(info.get("ext") or "m4a").strip().lower() or "m4a"
    has_video = parse_has_video(info)

    now = time.time()
    expires_at = parse_upstream_expiry(upstream_url, now)
    stream_id = secrets.token_urlsafe(18)

    app_state.prune_streams(now)
    app_state.streams[stream_id] = CachedStream(
        url=upstream_url,
        headers=headers,
        expires_at=expires_at,
    )

    expire_int = int(expires_at)
    stream_path = f"/ytdlp/stream/{stream_id}?expire={expire_int}"
    return web.json_response(
        {
            "url": upstream_url,
            "stream_path": stream_path,
            "headers": [[k, v] for k, v in headers.items()],
            "ext": ext,
            "has_video": has_video,
            "expires_at": expire_int,
        }
    )


async def handle_stream(request: web.Request) -> web.StreamResponse:
    app_state: ProxyApp = request.app["proxy"]
    stream_id = request.match_info.get("stream_id", "").strip()
    entry = app_state.streams.get(stream_id)
    now = time.time()
    if entry is None or entry.expires_at <= now:
        app_state.streams.pop(stream_id, None)
        return web.json_response({"error": "Stream expired or not found"}, status=410)

    req_headers = dict(entry.headers)
    range_hdr = request.headers.get("Range")
    if range_hdr:
        req_headers["Range"] = range_hdr

    assert app_state.http is not None
    try:
        async with app_state.http.get(entry.url, headers=req_headers) as upstream:
            if upstream.status in (403, 410):
                app_state.streams.pop(stream_id, None)
                return web.Response(status=upstream.status, text="Upstream stream expired")
            if upstream.status not in (200, 206):
                return web.Response(
                    status=upstream.status,
                    text=f"Upstream returned {upstream.status}",
                )

            resp_headers = {
                "Accept-Ranges": upstream.headers.get("Accept-Ranges", "bytes"),
                "Cache-Control": "private, max-age=3600",
            }
            for hdr_name in ("Content-Type", "Content-Length", "Content-Range"):
                if hdr_name in upstream.headers:
                    resp_headers[hdr_name] = upstream.headers[hdr_name]

            resp = web.StreamResponse(status=upstream.status, headers=resp_headers)
            await resp.prepare(request)
            try:
                async for chunk in upstream.content.iter_chunked(65536):
                    if chunk:
                        await resp.write(chunk)
                await resp.write_eof()
            except (ConnectionResetError, asyncio.CancelledError):
                pass
            return resp
    except asyncio.CancelledError:
        raise
    except Exception as exc:
        log.warning("Upstream stream proxy error for %s: %s", stream_id, exc)
        return web.json_response({"error": "Upstream stream failed"}, status=502)


async def handle_search(request: web.Request) -> web.Response:
    app_state: ProxyApp = request.app["proxy"]
    try:
        body = await request.json()
    except Exception:
        return web.json_response({"error": "Invalid JSON body"}, status=400)

    if not isinstance(body, dict):
        return web.json_response({"error": "JSON object required"}, status=400)

    query = str(body.get("query", "")).strip()
    if not query or len(query) > 300 or not YTSEARCH_RE.match(query):
        return web.json_response({"error": "Invalid ytsearch query"}, status=400)

    args = [
        YTDLP_BIN,
        *app_state.base_ytdlp_args(),
        "-j",
        "--flat-playlist",
        "--no-warnings",
        "--no-check-certificates",
        "--",
        query,
    ]

    try:
        await asyncio.wait_for(app_state.sem.acquire(), timeout=10.0)
    except asyncio.TimeoutError:
        return web.json_response({"error": "yt-dlp concurrency limit reached"}, status=503)

    try:
        rc, stdout, stderr = await run_subprocess(args, SEARCH_TIMEOUT_SECS)
    except asyncio.TimeoutError:
        return web.json_response({"error": "yt-dlp search timed out"}, status=504)
    except Exception as exc:
        log.exception("Failed running yt-dlp search")
        return web.json_response({"error": f"yt-dlp search error: {exc}"}, status=502)
    finally:
        app_state.sem.release()

    if rc != 0:
        err = stderr.decode("utf-8", errors="replace").strip()
        return web.json_response(
            {"error": err or "yt-dlp search failed"},
            status=502,
        )

    entries = []
    for line in stdout.decode("utf-8", errors="replace").splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            entries.append(json.loads(line))
        except json.JSONDecodeError:
            continue

    return web.json_response({"entries": entries})


async def handle_pipe(request: web.Request) -> web.StreamResponse:
    app_state: ProxyApp = request.app["proxy"]
    watch_url = validate_youtube_url(request.query.get("url", ""))
    if not watch_url:
        return web.json_response({"error": "Invalid or unsupported URL"}, status=400)

    try:
        await asyncio.wait_for(app_state.sem.acquire(), timeout=5.0)
    except asyncio.TimeoutError:
        return web.json_response({"error": "Stream processor busy"}, status=503)

    args = [
        YTDLP_BIN,
        *app_state.base_ytdlp_args(),
        "--no-playlist",
        "--no-warnings",
        "--extractor-args",
        "youtube:player_client=android",
        "-f",
        "bestaudio[vcodec=none][ext=m4a][protocol!=m3u8][protocol!=m3u8_native]/bestaudio[vcodec=none][protocol!=m3u8][protocol!=m3u8_native]/bestaudio[vcodec=none]/bestaudio/best",
        "-o",
        "-",
        "--",
        watch_url,
    ]

    proc = None
    try:
        proc = await asyncio.create_subprocess_exec(
            *args,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
        )
        assert proc.stdout is not None
        first_chunk = await asyncio.wait_for(proc.stdout.read(32768), timeout=20.0)
        if not first_chunk:
            return web.json_response({"error": "yt-dlp produced no audio"}, status=502)

        resp = web.StreamResponse(
            status=200,
            headers={
                "Content-Type": "audio/mp4",
                "Cache-Control": "no-store",
            },
        )
        await resp.prepare(request)
        await resp.write(first_chunk)
        while True:
            chunk = await proc.stdout.read(65536)
            if not chunk:
                break
            await resp.write(chunk)
        await resp.write_eof()
        return resp
    except (ConnectionResetError, asyncio.CancelledError):
        return web.Response(status=499)
    except asyncio.TimeoutError:
        return web.json_response({"error": "yt-dlp pipe timed out"}, status=504)
    except Exception as exc:
        log.warning("yt-dlp pipe error: %s", exc)
        return web.json_response({"error": "yt-dlp pipe failed"}, status=502)
    finally:
        if proc is not None and proc.returncode is None:
            try:
                proc.kill()
            except ProcessLookupError:
                pass
            try:
                await asyncio.wait_for(proc.wait(), timeout=3.0)
            except Exception:
                pass
        app_state.sem.release()


async def on_startup(app: web.Application) -> None:
    app_state: ProxyApp = app["proxy"]
    connector = TCPConnector(limit=64, ttl_dns_cache=300)
    timeout = ClientTimeout(total=None, connect=15, sock_read=45)
    app_state.http = ClientSession(connector=connector, timeout=timeout)
    try:
        rc, stdout, _ = await run_subprocess([YTDLP_BIN, "--version"], 5.0)
        if rc == 0:
            app_state.ytdlp_version = stdout.decode("utf-8", errors="replace").strip()
    except Exception as exc:
        log.warning("Could not query yt-dlp version: %s", exc)
    log.info(
        "ytdlp-proxy ready (yt-dlp=%s, max_concurrent=%d, cookies=%s)",
        app_state.ytdlp_version,
        MAX_CONCURRENT,
        bool(app_state.cookies_path),
    )


async def on_cleanup(app: web.Application) -> None:
    app_state: ProxyApp = app["proxy"]
    if app_state.http is not None:
        await app_state.http.close()


def prepare_cookies_file() -> Optional[str]:
    raw_b64 = os.environ.get("YTDLP_COOKIES_B64", "").strip()
    if not raw_b64:
        return None
    try:
        decoded = base64.b64decode(raw_b64)
        fd, path = tempfile.mkstemp(prefix="ytdlp-cookies-", suffix=".txt")
        with os.fdopen(fd, "wb") as f:
            f.write(decoded)
        os.chmod(path, 0o600)
        return path
    except Exception as exc:
        log.error("Failed to decode YTDLP_COOKIES_B64: %s", exc)
        return None


def main() -> None:
    token = os.environ.get("YTDLP_PROXY_TOKEN", "").strip()
    if len(token) < 16:
        log.error("YTDLP_PROXY_TOKEN must be set (at least 16 characters); refusing to start.")
        sys.exit(1)

    cookies_path = prepare_cookies_file()
    proxy_state = ProxyApp(token=token, cookies_path=cookies_path)

    app = web.Application(middlewares=[auth_middleware])
    app["proxy"] = proxy_state
    app.on_startup.append(on_startup)
    app.on_cleanup.append(on_cleanup)

    app.router.add_get("/health", handle_health)
    app.router.add_post("/ytdlp/resolve", handle_resolve)
    app.router.add_get("/ytdlp/stream/{stream_id}", handle_stream)
    app.router.add_post("/ytdlp/search", handle_search)
    app.router.add_get("/ytdlp/pipe", handle_pipe)

    host = os.environ.get("HOST", "0.0.0.0")
    port = int(os.environ.get("PORT", "8080"))
    web.run_app(app, host=host, port=port)


if __name__ == "__main__":
    main()
