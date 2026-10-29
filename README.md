# Beatlab

**Bitlab for music.** A single-binary, self-hosted music streaming
server. It speaks the **Subsonic / OpenSubsonic API** (the protocol
Navidrome implements), so existing clients — Feishin, Symfonium,
play:Sub, Substreamer, … — connect to it directly. Instead of a
fixed local library, Beatlab **fetches music on demand from
multiple sources**, caches what you play to disk, and **learns your
taste** from every play, skip, star and rating to power similar-song
lists, recommendations and radio auto-mixes.

Not on GitHub yet — local project.

## Run

```powershell
.\beatlab.exe
```

or on Linux: `./beatlab-linux`. Then:

- **Admin console:** http://localhost:4533/admin — the admin
  password is generated on first run and saved to
  `beatlab-data/admin-password.txt` (or set `BEATLAB_ADMIN_PASSWORD`).
- **Clients:** point any Subsonic client at `http://<host>:4533`
  with username `admin` and the admin password (seeded on first
  run). Create more users in the console. Clients with a
  **Navidrome** server mode (e.g. Feishin) work too — Beatlab
  implements Navidrome's native `POST /auth/login` and accepts the
  session token on API calls.

## Config (environment)

| Var | Default | Meaning |
|---|---|---|
| `BEATLAB_ADDR` | `:4533` | listen address |
| `BEATLAB_DATA` | `./beatlab-data` | database, cache, password file |
| `BEATLAB_CACHE_GB` | `10` | audio cache budget (LRU eviction) |
| `BEATLAB_ADMIN_PASSWORD` | *(generated)* | console password; also seeds the first API user |
| `BEATLAB_USER` | `admin` | first API username |
| `BEATLAB_YTDLP` | *(PATH lookup)* | path to yt-dlp; enables the YouTube source |

## Sources

| Source | Status |
|---|---|
| **archive.org** | ✅ live — phrase search over the Internet Archive audio catalog, per-file ID3 metadata, direct range-capable stream URLs, no key needed |
| **YouTube via yt-dlp** | ✅ live when a `yt-dlp` binary is present (search via `ytsearch`, stream URLs resolved on demand and cached ~25 min) |
| **Torrents (Torznab)** | 🚧 phase 2 — the Bitlab analog: Torznab indexer search + anacrolix swarm streaming. The source registers and reports its status; the engine lands next |

Search results **warm into the local catalog** (SQLite) as they're
found, Bitlab-style, so your library grows out of what you look for
and listen to. Completed streams are cached to disk under the
budget; partial/range requests proxy without caching.

## Cold start

A fresh server **warms its catalog in the background** from a
curated set of popular queries (the heavily-downloaded music
archive.org actually has — jazz greats, the Dead, classical,
Marley, Phish…), and fresh accounts get **trending-pool radio
mixes** until their own listening gives the taste engine
something to work with. Spoken-word collections (LibriVox
audiobooks, podcasts, old-time radio) and unattributed uploads
never enter the catalog, and browse lists hide "Unknown Artist"
one-offs.

## The learning layer

Every **scrobble** (start + completion), **skip** (starting a new
track before a third of the current one elapsed), **star** and
**rating** becomes an event. The taste engine folds events into
per-artist and per-genre affinity scores with a 90-day half-life:

- `getSimilarSongs2` — same artist/genre, affinity-weighted, with
  live source expansion when the local catalog is thin
- **Radio playlists** — *Beatlab Radio — Personal* (affinity mix
  + discovery slice), *— Discovery* (new music from your top
  genres/artists), *— Favorites* (starred + most played) appear as
  normal Subsonic playlists and refresh every 6 h or on demand
- `getArtistInfo2` similar artists and the console's "What you
  love" view are driven by the same profile

## API surface

Implemented: ping, getLicense, getOpenSubsonicExtensions,
getMusicFolders, getUser(s), getIndexes, getMusicDirectory,
getArtists, getArtist, getAlbum, getSong, search2/3, getAlbumList(2)
(recent/newest/frequent/random/starred/byGenre/byYear/alpha),
getRandomSongs, getSongsByGenre, getGenres, getStarred(2), star,
unstar, setRating, scrobble, getNowPlaying, playlists CRUD,
getSimilarSongs(2), getArtistInfo(2), getTopSongs, getLyrics
(empty), getInternetRadioStations (empty), getScanStatus, startScan,
savePlayQueue/getPlayQueue, stream, download, getCoverArt.
Auth: password (plain + `enc:`), token `md5(password+salt)`, and
`apiKey` (= account password in this build) — plus Navidrome
native login at `POST /auth/login` returning
`{id, username, token}` for clients like Feishin, with the token
accepted via the `X-ND-Authorization` header. Responses in XML or
JSON (`f=json`).

## Notes & tradeoffs

- **API passwords are stored recoverably in the local SQLite DB.**
  Subsonic token auth (`md5(password+salt)`) cannot be verified
  against a one-way hash; like the original Subsonic server,
  Beatlab keeps the password retrievable. The DB lives in your data
  dir — treat it like Bitlab's admin password file.
- Artists/albums are *virtual*, derived from track metadata —
  there's no folder library to scan; the catalog is whatever the
  sources have surfaced.
- Long-form audio (>20 min) is filtered out of archive.org results
  to keep radio shows/audiobooks out of music search.

## Layout

```
main.go                 wiring, config, seeding
internal/store          SQLite: users, tracks, events, stars, playlists, cache index
internal/sources        Source interface + archive.org / yt-dlp / torrent stub
internal/engine         stream proxy + disk cache (LRU) + cover art
internal/taste          affinity learning, similar songs, radio mixes
internal/subsonic       the Subsonic/OpenSubsonic REST API
internal/webui          embedded admin console
```

## Roadmap

1. Torrent source (Torznab + swarm streaming) — full Bitlab parity
2. SoundCloud / Bandcamp sources; MusicBrainz metadata enrichment
3. Movement-style "radio" endpoint with live skip feedback in-queue
4. Multi-user taste separation polish; shared/public radio mixes
