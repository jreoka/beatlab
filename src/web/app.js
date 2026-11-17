(() => {
  'use strict';

  const audio = document.getElementById('audio-el');
  let rafVisualizer = null;
  let vizCtx = null;
  let vizAnalyser = null;
  let vizSource = null;
  let vizData = null;
  let vizStream = null;
  let vizBoundTrack = null;
  let vizMode = '';
  let vizUnavailable = false;

  const ICONS = {
    play: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M8 4.5v15l12-7.5z"/></svg>',
    heart: '<svg class="heart-svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/></svg>',
    close: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12"/></svg>',
    arrowUp: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5M5 12l7-7 7 7"/></svg>',
    more: '<svg viewBox="0 0 24 24" fill="currentColor"><circle cx="5" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="19" cy="12" r="2"/></svg>',
    eyeOff: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94"/><path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19"/><path d="M14.12 14.12a3 3 0 1 1-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>',
    share: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><line x1="8.59" y1="13.51" x2="15.42" y2="17.49"/><line x1="15.41" y1="6.51" x2="8.59" y2="10.49"/></svg>',
    nowPlaying: '<svg viewBox="0 0 24 24" fill="currentColor"><rect x="4" y="10" width="3" height="10" rx="1"/><rect x="10.5" y="4" width="3" height="16" rx="1"/><rect x="17" y="7" width="3" height="13" rx="1"/></svg>',
    volOn: '<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.5 8.5a5 5 0 0 1 0 7"/><path d="M19 5a10 10 0 0 1 0 14"/>',
    volMuted: '<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="22" y1="9" x2="16" y2="15"/><line x1="16" y1="9" x2="22" y2="15"/>',
    repeatAll: '<polyline points="17 2 21 6 17 10"/><path d="M3 12v-2a4 4 0 0 1 4-4h14"/><polyline points="7 22 3 18 7 14"/><path d="M21 12v2a4 4 0 0 1-4 4H3"/>',
    repeatOne: '<polyline points="17 2 21 6 17 10"/><path d="M3 12v-2a4 4 0 0 1 4-4h14"/><polyline points="7 22 3 18 7 14"/><path d="M21 12v2a4 4 0 0 1-4 4H3"/><path d="M11 10.2 12.4 9.2V15"/>',
    trash: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/></svg>',
    pin: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/></svg>',
  };

  function pinnedPlaylistsStorageKey() {
    return `beatlab_pinned_playlists_${state.user?.id || 'anon'}`;
  }

  function getPinnedPlaylistIds() {
    try {
      const raw = JSON.parse(localStorage.getItem(pinnedPlaylistsStorageKey()) || '[]');
      return Array.isArray(raw) ? raw.filter((id) => typeof id === 'string' && id) : [];
    } catch {
      return [];
    }
  }

  function savePinnedPlaylistIds(ids) {
    try {
      localStorage.setItem(pinnedPlaylistsStorageKey(), JSON.stringify([...new Set(ids)]));
    } catch {}
  }

  function isPlaylistPinned(pid) {
    return getPinnedPlaylistIds().includes(pid);
  }

  function renderSidebarPlaylistsDom() {
    const box = $('sidebar-playlists');
    if (!box) return;
    const pinnedSet = new Set(getPinnedPlaylistIds());
    const pinned = (state.playlists || []).filter((p) => pinnedSet.has(p.id));
    if (!pinned.length) {
      box.innerHTML = `<div class="sidebar-pin-empty">No pinned playlists yet.<br><button type="button" class="sidebar-pin-link" onclick="App.navigate('playlists')">Browse Playlists</button></div>`;
      return;
    }
    box.innerHTML = pinned
      .map(
        (p) => `
          <div class="pl-link-row">
            <button class="pl-link" data-share-kind="playlist" data-share-id="${esc(p.id)}" data-share-title="${esc(p.name)}" data-share-sub="${esc(`${p.trackCount} tracks · ${p.ownerName}`)}" data-share-cover="${esc(p.coverUrl)}" onclick="App.navigate('playlist/${encodeURIComponent(p.id)}')">
              <span class="pl-link-name">${esc(p.name)}</span>
              ${p.isAuto ? '<span class="pl-auto-pill">Mix</span>' : ''}
            </button>
            <button class="pl-pin-btn" title="Unpin from sidebar" aria-label="Unpin ${esc(p.name)}" onclick="event.stopPropagation();App.togglePinPlaylist(${esc(JSON.stringify(p.id))})">${ICONS.pin}</button>
            ${!p.isAuto ? `<button class="pl-del-btn" title="Delete playlist" aria-label="Delete ${esc(p.name)}" onclick="event.stopPropagation();App.deletePlaylist(${esc(JSON.stringify(p.id))})">${ICONS.trash}</button>` : ''}
          </div>`
      )
      .join('');
  }

  const state = {
    token: localStorage.getItem('beatlab_token') || '',
    user: null,
    playlists: [],
    trackMap: new Map(), // id -> track object for instant access
    queue: [],
    unshuffledQueue: null,
    queueIndex: -1,
    pendingSeekSec: 0,
    shuffle: localStorage.getItem('beatlab_shuffle') === 'true',
    repeat: 'off', // off | all | one
    autoplay: localStorage.getItem('beatlab_autoplay') !== 'false',
    hapticsEnabled: localStorage.getItem('beatlab_haptics') !== 'false',
    theme: localStorage.getItem('beatlab_theme') === 'light' ? 'light' : 'dark',
    radioReqToken: 0,
    radioLoading: false,
    radioInFlight: null,
    compatAttemptedId: '',
    volume: parseFloat(localStorage.getItem('beatlab_vol') || '0.9'),
    muted: false,
    seeking: false,
    lastScrobbleCompleteId: '',
    playCommitTimer: null,
    lastPlaySwitchAt: 0,
    lastCommittedTrackId: '',
    activeAudioToken: 0,
    lyricsAbortController: null,
    lyricsReqToken: 0,
    queueActionToken: 0,
    queueAbortController: null,
    queueSaveTimer: null,
    queueSaveDirty: false,
    queueWarmTimer: null,
    likeInFlight: new Set(),
    lastSkipKeyAt: 0,
    shareInFlight: false,
    fsTab: 'queue',
    fsMobileView: 'player',
    lyrics: { synced: false, plain: '', lines: [] },
    activeLyricIdx: -1,
    searchTimer: null,
    searchComposing: false,
    ownSearchNavQuery: null,
    searchFetchingQuery: '',
    searchLoadingUi: false,
    viewReqToken: 0,
    viewAbortController: null,
    libTab: 'playlists',
    statsRange: 'all',
    statsMetric: 'streams',
    statsData: null,
    currentMoodShare: null,
    currentGenreShare: null,
    currentShareUrl: '',
    searchState: null,
    activeInstantMix: null,
    social: {
      profileData: null,
      profileTab: 'radio',
      editColor: '',
      // null = untouched, a data URL = pending new picture, 'REMOVE' = clear.
      editAvatarData: null,
    },
    settingsTab: 'general',
    settingsData: {
      sources: null,
      hidden: null,
      sessions: null,
      security: null,
      totpSetup: null,
      recoveryCodes: null,
      confirmResetHidden: false,
      confirmRevokeAll: false,
      confirmDisable2fa: false,
      confirmDelete: false,
    },
    authChallenge: '',
    authFactor: 'totp',
    restoreChallenge: '',
    pendingPasskeyOptions: null,
    deferredInstallPrompt: null,
    isStandalone: false,
    hasViewNetworkError: false,
    networkMode: 'online',
    wasDisconnected: false,
    backOnlineTimer: null,
    reconnectTimer: null,
    healthHeartbeatTimer: null,
    healthCheckInFlight: null,
    recentHeard: loadRecentHeard(),
    upcomingMode: (() => {
      const m = localStorage.getItem('beatlab_upcoming_mode');
      return ['default', 'familiar', 'discover'].includes(m) ? m : 'default';
    })(),
    radioPlaylistId: null,
  };

  function applyTheme(theme) {
    const next = theme === 'light' ? 'light' : 'dark';
    state.theme = next;
    document.documentElement.setAttribute('data-theme', next);
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute('content', next === 'light' ? '#f8f9fb' : '#090a0d');
  }
  applyTheme(state.theme);

  const RECENT_HEARD_TTL_MS = 2 * 60 * 60 * 1000;
  const RECENT_HEARD_MAX = 50;

  function loadRecentHeard() {
    try {
      const parsed = JSON.parse(localStorage.getItem('beatlab_recent_heard') || '[]');
      if (!Array.isArray(parsed)) return [];
      const cutoff = Date.now() - 2 * 60 * 60 * 1000;
      return parsed
        .filter((e) => e && typeof e.id === 'string' && e.id && Number.isFinite(e.at) && e.at > cutoff)
        .slice(0, 50);
    } catch {
      return [];
    }
  }

  function pruneRecentHeard() {
    const cutoff = Date.now() - RECENT_HEARD_TTL_MS;
    state.recentHeard = (state.recentHeard || []).filter((e) => e && e.id && e.at > cutoff);
    if (state.recentHeard.length > RECENT_HEARD_MAX) {
      state.recentHeard = state.recentHeard.slice(0, RECENT_HEARD_MAX);
    }
  }

  function recordRecentHeard(track) {
    const id = typeof track === 'string' ? track : track && track.id;
    if (!id) return;
    pruneRecentHeard();
    state.recentHeard = state.recentHeard.filter((e) => e.id !== id);
    state.recentHeard.unshift({ id, at: Date.now() });
    if (state.recentHeard.length > RECENT_HEARD_MAX) {
      state.recentHeard = state.recentHeard.slice(0, RECENT_HEARD_MAX);
    }
    try {
      localStorage.setItem('beatlab_recent_heard', JSON.stringify(state.recentHeard));
    } catch {}
  }

  function buildRecentRadioParam(excludeId = '') {
    pruneRecentHeard();
    const now = Date.now();
    return state.recentHeard
      .filter((e) => e.id && e.id !== excludeId)
      .slice(0, 40)
      .map((e) => `${e.id}@${Math.max(0, Math.floor((now - e.at) / 1000))}`)
      .join(',');
  }

  function isStandalonePwa() {
    try {
      return (
        window.matchMedia('(display-mode: standalone)').matches ||
        window.matchMedia('(display-mode: minimal-ui)').matches ||
        window.navigator.standalone === true
      );
    } catch {
      return false;
    }
  }

  function isIosBrowser() {
    const ua = navigator.userAgent || '';
    const isIos = /iphone|ipad|ipod/i.test(ua) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
    return isIos && !isStandalonePwa();
  }

  const HAPTIC_PATTERNS = {
    tick: 6,
    tap: 9,
    select: 12,
    medium: 18,
    success: [10, 28, 14],
    warning: [18, 36, 18],
  };

  let lastHapticAt = 0;
  let lastTickHapticAt = 0;

  function haptic(kind = 'tap') {
    if (!state.hapticsEnabled) return;
    if (typeof navigator === 'undefined' || typeof navigator.vibrate !== 'function') return;
    const now = Date.now();
    if (kind === 'tick') {
      if (now - lastTickHapticAt < 55) return;
      lastTickHapticAt = now;
    } else if (now - lastHapticAt < 28) {
      return;
    } else {
      lastHapticAt = now;
    }
    try {
      navigator.vibrate(HAPTIC_PATTERNS[kind] || HAPTIC_PATTERNS.tap);
    } catch {}
  }

  function updateInstallUI() {
    state.isStandalone = isStandalonePwa();
    document.documentElement.classList.toggle('is-standalone', state.isStandalone);
    const btn = $('pwa-install-btn');
    if (btn) {
      btn.classList.toggle('hidden', state.isStandalone || !state.deferredInstallPrompt);
    }
    if ($('settings-body') && state.settingsTab === 'general') {
      renderSettingsModalBody();
    }
  }

  function renderConnectivityBanner(mode) {
    const pairs = [
      [$('offline-banner'), $('offline-banner-text')],
      [$('auth-offline-banner'), $('auth-offline-banner-text')],
    ];
    for (const [banner, textEl] of pairs) {
      if (!banner) continue;
      if (mode === 'online') {
        banner.classList.remove('is-online');
        banner.classList.add('hidden');
      } else if (mode === 'back-online') {
        if (textEl) textEl.textContent = 'Back online — connected to Beatlab server';
        banner.classList.add('is-online');
        banner.classList.remove('hidden');
      } else if (mode === 'server-offline') {
        if (textEl) textEl.textContent = 'Beatlab server is offline. Waiting to reconnect…';
        banner.classList.remove('is-online');
        banner.classList.remove('hidden');
      } else {
        if (textEl) textEl.textContent = 'You are offline. Waiting for internet connection…';
        banner.classList.remove('is-online');
        banner.classList.remove('hidden');
      }
    }
  }

  function stopReconnectPoll() {
    if (state.reconnectTimer) {
      clearTimeout(state.reconnectTimer);
      state.reconnectTimer = null;
    }
  }

  function scheduleReconnectPoll() {
    if (state.reconnectTimer || state.networkMode === 'online') return;
    state.reconnectTimer = setTimeout(async () => {
      state.reconnectTimer = null;
      if (state.networkMode === 'online') return;
      const ok = await checkServerHealth();
      if (!ok && state.networkMode !== 'online') {
        scheduleReconnectPoll();
      }
    }, 4000);
  }

  function setOfflineState(mode) {
    const next = mode === 'device-offline' || (typeof navigator !== 'undefined' && navigator.onLine === false)
      ? 'device-offline'
      : 'server-offline';
    if (state.backOnlineTimer) {
      clearTimeout(state.backOnlineTimer);
      state.backOnlineTimer = null;
    }
    state.wasDisconnected = true;
    state.networkMode = next;
    renderConnectivityBanner(next);
    scheduleReconnectPoll();
  }

  async function handleConnectivityRestored() {
    stopReconnectPoll();
    const shouldShowBackOnline = state.wasDisconnected || state.networkMode !== 'online';
    state.networkMode = 'online';
    state.wasDisconnected = false;

    if (shouldShowBackOnline) {
      if (state.backOnlineTimer) {
        clearTimeout(state.backOnlineTimer);
      }
      renderConnectivityBanner('back-online');
      state.backOnlineTimer = setTimeout(() => {
        state.backOnlineTimer = null;
        if (state.networkMode === 'online') {
          renderConnectivityBanner('online');
        }
      }, 3500);
    } else {
      renderConnectivityBanner('online');
    }

    if (state.token && !state.user) {
      try {
        const res = await api('/api/web/me');
        if (res.user) {
          state.user = res.user;
          await onSignedIn();
          return;
        }
      } catch {}
    } else if (state.user && state.hasViewNetworkError) {
      state.hasViewNetworkError = false;
      renderCurrentHash();
    }
  }

  function checkServerHealth() {
    if (typeof navigator !== 'undefined' && navigator.onLine === false) {
      setOfflineState('device-offline');
      return Promise.resolve(false);
    }
    if (state.healthCheckInFlight) return state.healthCheckInFlight;
    const run = (async () => {
      const ctrl = typeof AbortController !== 'undefined' ? new AbortController() : null;
      const tid = ctrl ? setTimeout(() => ctrl.abort(), 4000) : null;
      try {
        const resp = await fetch('/health', {
          method: 'GET',
          cache: 'no-store',
          signal: ctrl ? ctrl.signal : undefined,
        });
        if (resp.ok) {
          if (state.networkMode !== 'online' || state.wasDisconnected) {
            await handleConnectivityRestored();
          }
          return true;
        }
        setOfflineState('server-offline');
        return false;
      } catch {
        setOfflineState(
          typeof navigator !== 'undefined' && navigator.onLine === false
            ? 'device-offline'
            : 'server-offline'
        );
        return false;
      } finally {
        if (tid) clearTimeout(tid);
        if (state.healthCheckInFlight === run) {
          state.healthCheckInFlight = null;
        }
      }
    })();
    state.healthCheckInFlight = run;
    return run;
  }

  function syncOnlineStatus() {
    const online = typeof navigator === 'undefined' ? true : navigator.onLine !== false;
    if (!online) {
      setOfflineState('device-offline');
    } else if (state.networkMode === 'device-offline') {
      setOfflineState('server-offline');
      checkServerHealth();
    } else if (state.networkMode === 'online') {
      renderConnectivityBanner('online');
    }
  }

  function registerServiceWorker() {
    if (!('serviceWorker' in navigator)) return;
    const doRegister = () => {
      navigator.serviceWorker.register('/sw.js', { scope: '/' }).catch((err) => {
        console.warn('Service worker registration failed:', err);
      });
    };
    if (document.readyState === 'complete') {
      doRegister();
    } else {
      window.addEventListener('load', doRegister, { once: true });
    }
  }

  function wirePwaLifecycle() {
    updateInstallUI();
    syncOnlineStatus();
    registerServiceWorker();

    for (const mq of ['(display-mode: standalone)', '(display-mode: minimal-ui)']) {
      try {
        window.matchMedia(mq).addEventListener('change', updateInstallUI);
      } catch {}
    }

    window.addEventListener('beforeinstallprompt', (e) => {
      e.preventDefault();
      state.deferredInstallPrompt = e;
      updateInstallUI();
    });

    window.addEventListener('appinstalled', () => {
      state.deferredInstallPrompt = null;
      state.isStandalone = true;
      updateInstallUI();
      haptic('success');
      showToast('Beatlab installed!');
    });

    window.addEventListener('offline', () => {
      setOfflineState('device-offline');
    });

    window.addEventListener('online', () => {
      checkServerHealth();
    });

    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'visible') {
        checkServerHealth();
      }
    });

    window.addEventListener('focus', () => {
      if (state.networkMode !== 'online') {
        checkServerHealth();
      }
    });

    if (!state.healthHeartbeatTimer) {
      state.healthHeartbeatTimer = setInterval(() => {
        if (document.visibilityState === 'visible' && state.networkMode === 'online') {
          checkServerHealth();
        }
      }, 20000);
    }
  }

  async function installPwa() {
    const evt = state.deferredInstallPrompt;
    if (evt) {
      state.deferredInstallPrompt = null;
      updateInstallUI();
      try {
        await evt.prompt();
        const choice = await evt.userChoice;
        if (choice && choice.outcome === 'accepted') {
          showToast('Installing Beatlab...');
        }
      } catch (err) {
        console.warn('Install prompt failed:', err);
      }
      return;
    }
    if (isStandalonePwa()) {
      showToast('Beatlab is already installed');
    } else if (isIosBrowser()) {
      showToast('Tap Share in Safari, then "Add to Home Screen"');
    } else {
      showToast('Use your browser menu to install Beatlab');
    }
  }

  const $ = (id) => document.getElementById(id);

  function esc(str) {
    return String(str ?? '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function fmtTime(sec) {
    if (!Number.isFinite(sec) || sec < 0) return '0:00';
    const s = Math.floor(sec);
    const m = Math.floor(s / 60);
    const r = s % 60;
    return `${m}:${r < 10 ? '0' : ''}${r}`;
  }

  function fmtTotalDur(sec) {
    if (!sec || sec <= 0) return '0 min';
    const hrs = Math.floor(sec / 3600);
    const mins = Math.floor((sec % 3600) / 60);
    return hrs > 0 ? `${hrs} hr ${mins} min` : `${mins} min`;
  }

  function showToast(msg) {
    const el = $('toast');
    el.textContent = msg;
    el.classList.remove('hidden');
    clearTimeout(el._t);
    el._t = setTimeout(() => el.classList.add('hidden'), 2600);
  }

  async function api(path, opts = {}) {
    const headers = Object.assign({ 'Content-Type': 'application/json' }, opts.headers || {});
    if (state.token) {
      headers['Authorization'] = `Bearer ${state.token}`;
    }
    let resp;
    try {
      resp = await fetch(path, { ...opts, headers });
    } catch (err) {
      if (!err || err.name !== 'AbortError') {
        setOfflineState(
          typeof navigator !== 'undefined' && navigator.onLine === false
            ? 'device-offline'
            : 'server-offline'
        );
        if (err && typeof err === 'object') err.isNetworkError = true;
      }
      throw err;
    }
    if (resp.status === 502 || resp.status === 503 || resp.status === 504) {
      setOfflineState('server-offline');
      const netErr = new Error(`Beatlab server is unavailable (${resp.status})`);
      netErr.isNetworkError = true;
      throw netErr;
    }
    if (state.networkMode !== 'online' || state.wasDisconnected) {
      handleConnectivityRestored();
    }
    const inlineAuthPaths = [
      '/api/web/auth/signin',
      '/api/web/auth/signup',
      '/api/web/auth/mfa/verify',
      '/api/web/auth/passkey/options',
      '/api/web/auth/passkey/verify',
      '/api/web/account/restore',
    ];
    if (resp.status === 401 && !inlineAuthPaths.includes(path)) {
      showAuthScreen();
      throw new Error('Unauthorized');
    }
    const text = await resp.text();
    let data = {};
    try {
      data = text ? JSON.parse(text) : {};
    } catch {
      data = { error: text };
    }
    if (!resp.ok) {
      throw new Error(data.error || `Request failed (${resp.status})`);
    }
    return data;
  }

  function cacheTracks(list) {
    if (!Array.isArray(list)) return [];
    for (const t of list) {
      if (t && t.id) state.trackMap.set(t.id, t);
    }
    return list;
  }

  function currentTrack() {
    if (state.queueIndex >= 0 && state.queueIndex < state.queue.length) {
      return state.queue[state.queueIndex];
    }
    return null;
  }

  function greetingText() {
    const h = new Date().getHours();
    if (h < 12) return 'Good morning';
    if (h < 18) return 'Good afternoon';
    return 'Good evening';
  }

  // ---------------------------------------------------------------------------
  // Playback output & visualizer
  // ---------------------------------------------------------------------------
  function applyOutputVolume() {
    const effectiveVol = state.muted ? 0 : Math.max(0, Math.min(1, Number(state.volume) || 0));
    audio.muted = state.muted;
    audio.volume = effectiveVol;
  }

  function setMediaPlaybackState(value) {
    if (!('mediaSession' in navigator)) return;
    try {
      navigator.mediaSession.playbackState = value;
    } catch {}
  }

  function setExpandedPlayer(open) {
    const fs = $('fullscreen-player');
    if (!fs) return;
    const wasOpen = fs.classList.contains('open');
    const next = typeof open === 'boolean' ? open : !wasOpen;
    fs.classList.toggle('open', next);
    fs.setAttribute('aria-hidden', next ? 'false' : 'true');
    if (next !== wasOpen && window.innerWidth <= 780) {
      haptic(next ? 'select' : 'tap');
    }
    if (next && window.innerWidth <= 780) {
      setFsMobileView(state.fsMobileView || 'player', true);
    }
    const btn = $('btn-expand-player');
    if (btn) {
      btn.classList.toggle('active', next);
      btn.title = next ? 'Collapse player' : 'Expand player';
    }
    if (next && !rafVisualizer) {
      drawVisualizer();
    }
  }

  function setFsMobileView(view, silent = false) {
    const next = view === 'queue' || view === 'lyrics' ? view : 'player';
    const changed = state.fsMobileView !== next;
    state.fsMobileView = next;
    const fs = $('fullscreen-player');
    if (fs) {
      fs.classList.remove('fs-view-player', 'fs-view-queue', 'fs-view-lyrics');
      fs.classList.add(`fs-view-${next}`);
    }
    if (!silent && changed && window.innerWidth <= 780) {
      haptic('select');
    }
    if ($('fs-tab-player')) $('fs-tab-player').classList.toggle('active', next === 'player');
    if ($('fs-tab-mobile-queue')) $('fs-tab-mobile-queue').classList.toggle('active', next === 'queue');
    if ($('fs-tab-mobile-lyrics')) $('fs-tab-mobile-lyrics').classList.toggle('active', next === 'lyrics');
    if (next === 'queue' || next === 'lyrics') {
      window.App.setFsTab(next, true);
    }
  }

  function syncResponsiveLayout() {
    const isMobile = window.innerWidth <= 780;
    const isTabletOrMobile = window.innerWidth <= 1060;
    const searchInput = $('global-search');
    if (searchInput) {
      searchInput.placeholder = isMobile
        ? 'Search songs, artists, albums...'
        : 'Search songs, artists, albums... (Press /)';
    }
    const rp = $('right-panel');
    const backdrop = $('panel-backdrop');
    if (!rp) return;
    if (isMobile) {
      rp.classList.add('hidden');
      if (backdrop) backdrop.classList.add('hidden');
    } else if (isTabletOrMobile) {
      if (!rp.dataset.userToggled) {
        rp.classList.add('hidden');
      }
      if (backdrop) backdrop.classList.toggle('hidden', rp.classList.contains('hidden'));
    } else {
      if (backdrop) backdrop.classList.add('hidden');
      if (!rp.dataset.userToggled) {
        rp.classList.remove('hidden');
      }
    }
  }

  function updateVolumeIcon() {
    const icon = $('vol-icon');
    if (!icon) return;
    icon.innerHTML = state.muted || state.volume === 0 ? ICONS.volMuted : ICONS.volOn;
  }

  function ensureVisualizerAnalyser() {
    if (vizUnavailable || !audio) return;
    if (vizCtx && vizCtx.state === 'suspended' && !audio.paused) {
      vizCtx.resume().catch(() => {});
    }
    if (vizAnalyser && vizMode === 'element') {
      return;
    }
    const AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) {
      vizUnavailable = true;
      return;
    }
    const isMobileUa = /iPhone|iPad|iPod|Android/i.test(navigator.userAgent || '');
    try {
      if (!vizCtx) {
        vizCtx = new AC();
        audio.addEventListener('play', () => {
          if (vizCtx && vizCtx.state === 'suspended') {
            vizCtx.resume().catch(() => {});
          }
        });
      }
      if (vizCtx.state === 'suspended') {
        vizCtx.resume().catch(() => {});
      }
      if (!vizAnalyser) {
        vizAnalyser = vizCtx.createAnalyser();
        vizAnalyser.fftSize = 256;
        vizAnalyser.smoothingTimeConstant = 0.78;
        vizData = new Uint8Array(vizAnalyser.frequencyBinCount);
      }
      if (!isMobileUa && typeof vizCtx.createMediaElementSource === 'function') {
        vizSource = vizCtx.createMediaElementSource(audio);
        vizSource.connect(vizAnalyser);
        vizAnalyser.connect(vizCtx.destination);
        vizMode = 'element';
        return;
      }
      const capture =
        typeof audio.captureStream === 'function'
          ? () => audio.captureStream()
          : typeof audio.mozCaptureStream === 'function'
            ? () => audio.mozCaptureStream()
            : null;
      if (!capture) {
        if (typeof vizCtx.createMediaElementSource === 'function' && !vizSource) {
          vizSource = vizCtx.createMediaElementSource(audio);
          vizSource.connect(vizAnalyser);
          vizAnalyser.connect(vizCtx.destination);
          vizMode = 'element';
        } else {
          vizUnavailable = true;
        }
        return;
      }
      let liveTrack =
        vizStream && vizStream.getAudioTracks().find((t) => t.readyState === 'live');
      if (!liveTrack) {
        vizStream = capture();
        liveTrack =
          vizStream && vizStream.getAudioTracks().find((t) => t.readyState === 'live');
      }
      if (!liveTrack) {
        return;
      }
      if (vizMode === 'stream' && vizBoundTrack === liveTrack && vizSource) {
        return;
      }
      if (vizSource) {
        try {
          vizSource.disconnect();
        } catch {}
      }
      vizSource = vizCtx.createMediaStreamSource(vizStream);
      vizBoundTrack = liveTrack;
      vizMode = 'stream';
      const silentGain = vizCtx.createGain();
      silentGain.gain.value = 0;
      vizSource.connect(vizAnalyser);
      vizAnalyser.connect(silentGain);
      silentGain.connect(vizCtx.destination);
    } catch {
      vizUnavailable = true;
    }
  }

  function drawVisualizer() {
    const fs = $('fullscreen-player');
    if (!fs.classList.contains('open')) {
      rafVisualizer = null;
      return;
    }
    const canvas = $('fs-visualizer');
    if (canvas) {
      const ctx = canvas.getContext('2d');
      const bars = 40;
      const bw = canvas.width / bars;
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.fillStyle = '#1ed760';
      const playing = !!audio && !audio.paused && !audio.ended;
      if (playing && audio.readyState >= 2) {
        ensureVisualizerAnalyser();
      }
      if (playing && vizAnalyser && vizData) {
        vizAnalyser.getByteFrequencyData(vizData);
        const usableBins = Math.max(1, Math.floor(vizData.length * 0.65));
        for (let i = 0; i < bars; i++) {
          const bin = Math.min(
            vizData.length - 1,
            Math.floor(Math.pow(i / bars, 1.35) * usableBins)
          );
          const amp = vizData[bin] / 255;
          const h = Math.min(canvas.height, Math.max(3, (0.06 + amp * 0.9) * canvas.height));
          ctx.fillRect(i * bw + 2, canvas.height - h, Math.max(3, bw - 4), h);
        }
      } else {
        for (let i = 0; i < bars; i++) {
          const h = Math.max(3, canvas.height * 0.06);
          ctx.fillRect(i * bw + 2, canvas.height - h, Math.max(3, bw - 4), h);
        }
      }
    }
    rafVisualizer = requestAnimationFrame(drawVisualizer);
  }

  // ---------------------------------------------------------------------------
  // Authentication & Session Bootstrap
  // ---------------------------------------------------------------------------
  function resetAuthForm() {
    const form = $('auth-form');
    if (form) {
      form.reset();
      form.classList.remove('hidden');
    }
    const passkeyBtn = $('auth-passkey-btn');
    if (passkeyBtn) passkeyBtn.classList.remove('hidden');
    const mfa = $('auth-mfa');
    if (mfa) mfa.classList.add('hidden');
    const mfaForm = $('mfa-form');
    if (mfaForm) mfaForm.classList.remove('hidden');
    const mfaPasskey = $('mfa-passkey-btn');
    if (mfaPasskey) mfaPasskey.classList.add('hidden');
    const restore = $('auth-restore');
    if (restore) restore.classList.add('hidden');
    state.authChallenge = '';
    state.restoreChallenge = '';
    state.pendingPasskeyOptions = null;
    const errBox = $('auth-error');
    if (errBox) {
      errBox.textContent = '';
      errBox.classList.add('hidden');
    }
    for (const id of ['auth-username', 'auth-password']) {
      const el = $(id);
      if (!el) continue;
      if (document.activeElement === el) el.blur();
      el.value = '';
      el.defaultValue = '';
      // Replace input DOM element so mobile browsers don't retain prior autofill/session state
      if (el.parentNode) {
        const fresh = el.cloneNode(false);
        fresh.value = '';
        fresh.defaultValue = '';
        el.parentNode.replaceChild(fresh, el);
      }
    }
  }

  function resetPlayerAndSessionUI() {
    audio.pause();
    audio.removeAttribute('src');
    audio.load();

    state.playlists = [];
    state.trackMap.clear();
    state.queue = [];
    state.unshuffledQueue = null;
    state.queueIndex = -1;
    state.radioReqToken++;
    state.radioLoading = false;
    state.radioInFlight = null;
    state.radioPlaylistId = null;
    state.compatAttemptedId = '';
    state.lastScrobbleCompleteId = '';
    state.lyrics = { synced: false, plain: '', lines: [] };
    state.activeLyricIdx = -1;
    state.searchState = null;
    state.statsData = null;
    state.currentMoodShare = null;
    state.currentGenreShare = null;
    state.currentShareUrl = '';
    state.settingsData = {
      sources: null,
      hidden: null,
      sessions: null,
      security: null,
      totpSetup: null,
      recoveryCodes: null,
      confirmResetHidden: false,
      confirmRevokeAll: false,
      confirmDisable2fa: false,
      confirmDelete: false,
    };
    clearTimeout(state.searchTimer);
    state.searchTimer = null;
    state.searchComposing = false;
    state.ownSearchNavQuery = null;
    abortActiveViewRequest();
    state.fsMobileView = 'player';

    $('pb-cover').src = '/api/web/cover/default';
    $('pb-title').textContent = 'Ready to play';
    $('pb-artist').textContent = 'Select any track';
    $('pb-like').classList.remove('active');
    $('pb-quality').classList.add('hidden');
    $('pb-elapsed').textContent = '0:00';
    $('pb-duration').textContent = '0:00';
    $('pb-seek').value = '0';
    if ($('pb-mini-progress-fill')) $('pb-mini-progress-fill').style.width = '0%';

    $('fs-cover').src = '/api/web/cover/default';
    $('fs-title').textContent = 'Ready';
    $('fs-artist').textContent = 'Beatlab';
    $('fs-bg').style.backgroundImage = '';
    if ($('fs-like')) $('fs-like').classList.remove('active');
    if ($('fs-quality')) $('fs-quality').classList.add('hidden');
    if ($('fs-elapsed')) $('fs-elapsed').textContent = '0:00';
    if ($('fs-duration')) $('fs-duration').textContent = '0:00';
    if ($('fs-seek')) $('fs-seek').value = '0';
    updatePlayButton(false);

    $('main-view').innerHTML = '';
    $('sidebar-playlists').innerHTML = '';
    $('global-search').value = '';
    $('clear-search-btn').classList.add('hidden');
    $('queue-now-playing').innerHTML = '';
    $('queue-up-next').innerHTML = '';
    if ($('fs-queue-now-playing')) $('fs-queue-now-playing').innerHTML = '';
    if ($('fs-queue-up-next')) $('fs-queue-up-next').innerHTML = '';
    $('lyrics-container').innerHTML = '<p class="muted-center">Play a song to view synced lyrics</p>';
    if ($('fs-lyrics-body')) $('fs-lyrics-body').innerHTML = '<p class="muted-center">Play a song to view synced lyrics</p>';

    const toast = $('toast');
    if (toast) {
      clearTimeout(toast._t);
      toast.classList.add('hidden');
    }
    document.title = 'Beatlab — Stream Everything';
    setMediaPlaybackState('none');
    if ('mediaSession' in navigator) {
      try {
        navigator.mediaSession.metadata = null;
      } catch {}
    }
  }

  function showAuthScreen(forceReset = false) {
    const authScreen = $('auth-screen');
    const wasHidden = authScreen.classList.contains('hidden');
    setExpandedPlayer(false);
    closeModal();
    closeTrackMenu();
    if ($('right-panel') && window.innerWidth <= 1060) {
      $('right-panel').classList.add('hidden');
    }
    if ($('panel-backdrop')) $('panel-backdrop').classList.add('hidden');
    authScreen.classList.remove('hidden');
    $('app-shell').classList.add('hidden');
    $('player-bar').classList.add('hidden');
    if (wasHidden || forceReset) {
      resetAuthForm();
    }
  }

  function showAppShell() {
    $('auth-screen').classList.add('hidden');
    $('app-shell').classList.remove('hidden');
    $('player-bar').classList.remove('hidden');
    syncResponsiveLayout();
  }

  // WebAuthn's JavaScript API uses ArrayBuffers while the server speaks
  // base64url strings, so both directions need a small conversion layer.
  function base64urlToBytes(value) {
    let raw = String(value || '').replace(/-/g, '+').replace(/_/g, '/');
    while (raw.length % 4) raw += '=';
    const bin = atob(raw);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  function bytesToBase64url(buffer) {
    const bytes = new Uint8Array(buffer || []);
    let bin = '';
    for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
    return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  }

  function webauthnOptions(src, creating = false) {
    const out = Object.assign({}, src);
    out.challenge = base64urlToBytes(src.challenge);
    if (creating && out.user) {
      out.user = Object.assign({}, out.user, { id: base64urlToBytes(out.user.id) });
    }
    const listKey = creating ? 'excludeCredentials' : 'allowCredentials';
    if (Array.isArray(out[listKey])) {
      out[listKey] = out[listKey].map((c) => Object.assign({}, c, { id: base64urlToBytes(c.id) }));
    }
    return out;
  }

  function webauthnCredentialToJson(cred) {
    const response = cred.response || {};
    const encoded = {};
    for (const key of ['authenticatorData', 'clientDataJSON', 'signature', 'userHandle', 'attestationObject']) {
      if (response[key] != null) encoded[key] = bytesToBase64url(response[key]);
    }
    return {
      id: cred.id,
      rawId: bytesToBase64url(cred.rawId),
      type: cred.type || 'public-key',
      response: encoded,
      clientExtensionResults: typeof cred.getClientExtensionResults === 'function' ? cred.getClientExtensionResults() : {},
    };
  }

  function passkeySupported() {
    return !!(window.PublicKeyCredential && navigator.credentials && navigator.credentials.create);
  }

  function securityCodeValue() {
    const input = $('sec-code');
    return input ? input.value.trim() : '';
  }

  function wirePlayerBarToggle() {
    const bar = $('player-bar');
    if (!bar) return;
    let pointerDownIgnored = false;
    let suppressClick = false;
    let startX = 0;
    let startY = 0;
    let lastDx = 0;
    let lastDy = 0;
    let gestureAxis = '';
    let hapticFired = false;
    let tracking = false;
    const metaBox = bar.querySelector('.pb-left');

    const isInteractive = (target) => {
      if (window.innerWidth <= 780) {
        return !!target.closest('button, a, input, label, .pb-progress-row');
      }
      return !!target.closest('button, a, input, label, .pb-title, .pb-artist, .pb-progress-row');
    };

    bar.addEventListener('pointerdown', (e) => {
      pointerDownIgnored = isInteractive(e.target);
    });

    bar.addEventListener('touchstart', (e) => {
      if (window.innerWidth > 780 || e.touches.length !== 1) return;
      if (isInteractive(e.target)) return;
      startX = e.touches[0].clientX;
      startY = e.touches[0].clientY;
      lastDx = 0;
      lastDy = 0;
      gestureAxis = '';
      hapticFired = false;
      tracking = true;
      if (metaBox) metaBox.style.transition = 'none';
    }, { passive: true });

    bar.addEventListener('touchmove', (e) => {
      if (!tracking || e.touches.length !== 1) return;
      const dx = e.touches[0].clientX - startX;
      const dy = e.touches[0].clientY - startY;
      lastDx = dx;
      lastDy = dy;
      if (!gestureAxis && (Math.abs(dx) > 10 || Math.abs(dy) > 10)) {
        gestureAxis = Math.abs(dx) > Math.abs(dy) * 1.15 ? 'x' : 'y';
      }
      if (gestureAxis === 'x' && state.queue.length > 0 && metaBox) {
        const clamped = Math.max(-56, Math.min(56, dx * 0.55));
        metaBox.style.transform = `translateX(${clamped}px)`;
        metaBox.style.opacity = String(Math.max(0.55, 1 - Math.abs(clamped) / 110));
        if (!hapticFired && Math.abs(dx) >= 52) {
          hapticFired = true;
          haptic('tick');
        }
      } else if (gestureAxis === 'y' && dy < -36 && !hapticFired) {
        hapticFired = true;
        haptic('tick');
      }
    }, { passive: true });

    const finishSwipe = () => {
      if (!tracking) return;
      tracking = false;
      if (metaBox) {
        metaBox.style.transition = 'transform 0.18s ease, opacity 0.18s ease';
        metaBox.style.transform = '';
        metaBox.style.opacity = '';
      }
      if (gestureAxis === 'x' && Math.abs(lastDx) >= 52 && state.queue.length > 0) {
        suppressClick = true;
        if (lastDx < 0) {
          nextTrack(true);
        } else {
          haptic('medium');
          prevTrack();
        }
      } else if (gestureAxis === 'y' && lastDy < -36) {
        suppressClick = true;
        if (!$('fullscreen-player').classList.contains('open')) {
          setFsMobileView('player', true);
          setExpandedPlayer(true);
        }
      }
      gestureAxis = '';
      lastDx = 0;
      lastDy = 0;
    };

    bar.addEventListener('touchend', finishSwipe, { passive: true });
    bar.addEventListener('touchcancel', finishSwipe, { passive: true });

    bar.addEventListener('click', (e) => {
      if (suppressClick) {
        suppressClick = false;
        return;
      }
      const titleEl = e.target.closest('#pb-title');
      const artistEl = e.target.closest('#pb-artist');
      if (window.innerWidth > 780 && titleEl) {
        App.goCurrentAlbum();
        return;
      }
      if (window.innerWidth > 780 && artistEl) {
        App.goCurrentArtist();
        return;
      }
      const ignored = pointerDownIgnored || isInteractive(e.target);
      pointerDownIgnored = false;
      if (ignored) return;
      if (window.innerWidth <= 780 && !$('fullscreen-player').classList.contains('open')) {
        setFsMobileView('player', true);
      }
      setExpandedPlayer();
    });
  }

  function wireFsSwipeClose() {
    const fs = $('fullscreen-player');
    if (!fs) return;
    const cover = $('fs-cover');
    let startY = 0;
    let startX = 0;
    let tracking = false;
    let axis = '';
    let canSwipeArt = false;
    let deltaY = 0;
    let deltaX = 0;
    let closeHapticFired = false;
    let skipHapticFired = false;

    fs.addEventListener('touchstart', (e) => {
      if (window.innerWidth > 780 || !fs.classList.contains('open')) return;
      if (e.touches.length !== 1) return;
      if (e.target.closest('input, button, a, .fs-panel-body, .fs-mobile-tabs')) return;
      startY = e.touches[0].clientY;
      startX = e.touches[0].clientX;
      deltaY = 0;
      deltaX = 0;
      axis = '';
      canSwipeArt = state.fsMobileView === 'player' && !!e.target.closest('.fs-art-col') && state.queue.length > 0;
      closeHapticFired = false;
      skipHapticFired = false;
      tracking = true;
      fs.style.transition = 'none';
      if (cover) cover.style.transition = 'none';
    }, { passive: true });

    fs.addEventListener('touchmove', (e) => {
      if (!tracking || e.touches.length !== 1) return;
      const dy = e.touches[0].clientY - startY;
      const dx = e.touches[0].clientX - startX;
      if (!axis && (Math.abs(dy) > 10 || Math.abs(dx) > 10)) {
        axis = Math.abs(dx) > Math.abs(dy) * 1.2 && canSwipeArt ? 'x' : 'y';
      }
      if (axis === 'y' && dy > 8 && Math.abs(dy) > Math.abs(dx) * 1.15) {
        deltaY = dy;
        fs.style.transform = `translateY(${dy}px)`;
        if (!closeHapticFired && dy > 90) {
          closeHapticFired = true;
          haptic('tick');
        }
      } else if (axis === 'x' && canSwipeArt && cover) {
        deltaX = dx;
        const clamped = Math.max(-72, Math.min(72, dx * 0.48));
        cover.style.transform = `translateX(${clamped}px) scale(${Math.max(0.94, 1 - Math.abs(clamped) / 720)})`;
        cover.style.opacity = String(Math.max(0.6, 1 - Math.abs(clamped) / 150));
        if (!skipHapticFired && Math.abs(dx) >= 64) {
          skipHapticFired = true;
          haptic('tick');
        }
      }
    }, { passive: true });

    const finish = () => {
      if (!tracking) return;
      tracking = false;
      fs.style.transition = '';
      fs.style.transform = '';
      if (cover) {
        cover.style.transition = 'transform 0.2s cubic-bezier(0.22, 1, 0.36, 1), opacity 0.2s ease';
        cover.style.transform = '';
        cover.style.opacity = '';
      }
      if (axis === 'y' && deltaY > 90) {
        setExpandedPlayer(false);
      } else if (axis === 'x' && Math.abs(deltaX) >= 64 && canSwipeArt) {
        if (deltaX < 0) {
          nextTrack(true);
        } else {
          haptic('medium');
          prevTrack();
        }
      }
      axis = '';
      deltaY = 0;
      deltaX = 0;
    };
    fs.addEventListener('touchend', finish, { passive: true });
    fs.addEventListener('touchcancel', finish, { passive: true });
  }

  function wireSheetAndPtrGestures() {
    // Swipe down on mobile bottom context sheet to dismiss
    const menu = $('context-menu');
    if (menu) {
      let startY = 0;
      let startX = 0;
      let dy = 0;
      let tracking = false;
      let detent = false;

      menu.addEventListener('touchstart', (e) => {
        if (window.innerWidth > 780 || menu.classList.contains('hidden')) return;
        if (e.touches.length !== 1) return;
        if (menu.scrollTop > 2 && !e.target.closest('.ctx-sheet-head')) return;
        startY = e.touches[0].clientY;
        startX = e.touches[0].clientX;
        dy = 0;
        detent = false;
        tracking = true;
        menu.style.transition = 'none';
      }, { passive: true });

      menu.addEventListener('touchmove', (e) => {
        if (!tracking || e.touches.length !== 1) return;
        const nextDy = e.touches[0].clientY - startY;
        const dx = e.touches[0].clientX - startX;
        if (nextDy > 8 && Math.abs(nextDy) > Math.abs(dx) * 1.15 && menu.scrollTop <= 2) {
          dy = nextDy;
          menu.style.transform = `translateY(${nextDy}px)`;
          if (!detent && nextDy > 72) {
            detent = true;
            haptic('tick');
          }
        }
      }, { passive: true });

      const finishSheet = () => {
        if (!tracking) return;
        tracking = false;
        menu.style.transition = '';
        menu.style.transform = '';
        if (dy > 72) {
          haptic('tap');
          closeTrackMenu();
        }
        dy = 0;
      };
      menu.addEventListener('touchend', finishSheet, { passive: true });
      menu.addEventListener('touchcancel', finishSheet, { passive: true });
    }

    // Mobile pull-to-refresh inside main scrollable view
    const main = $('main-view');
    const ptr = $('ptr-indicator');
    const ptrText = $('ptr-text');
    if (main && ptr) {
      let startY = 0;
      let startX = 0;
      let pullPx = 0;
      let active = false;
      let refreshing = false;
      let primed = false;

      main.addEventListener('touchstart', (e) => {
        if (window.innerWidth > 780 || refreshing || e.touches.length !== 1) return;
        if (main.scrollTop > 0) return;
        if (e.target.closest('input, textarea, select, .tabs-bar, .stats-range-pills')) return;
        startY = e.touches[0].clientY;
        startX = e.touches[0].clientX;
        pullPx = 0;
        primed = false;
        active = true;
      }, { passive: true });

      main.addEventListener('touchmove', (e) => {
        if (!active || refreshing || e.touches.length !== 1) return;
        if (main.scrollTop > 0) {
          active = false;
          ptr.classList.add('hidden');
          ptr.style.transform = '';
          return;
        }
        const dy = e.touches[0].clientY - startY;
        const dx = e.touches[0].clientX - startX;
        if (dy <= 14 || Math.abs(dx) > Math.abs(dy)) {
          if (dy < 0) active = false;
          return;
        }
        pullPx = Math.min(84, (dy - 14) * 0.5);
        ptr.classList.remove('hidden', 'ptr-spinning');
        ptr.classList.toggle('ptr-ready', pullPx >= 48);
        ptr.style.transform = `translate(-50%, ${Math.round(pullPx - 36)}px)`;
        ptr.style.opacity = String(Math.min(1, pullPx / 36));
        if (ptrText) ptrText.textContent = pullPx >= 48 ? 'Release to refresh' : 'Pull to refresh';
        if (!primed && pullPx >= 48) {
          primed = true;
          haptic('tick');
        }
      }, { passive: true });

      const finishPtr = async () => {
        if (!active) return;
        active = false;
        const shouldRefresh = pullPx >= 48 && !refreshing;
        pullPx = 0;
        if (shouldRefresh) {
          refreshing = true;
          haptic('medium');
          ptr.classList.remove('ptr-ready');
          ptr.classList.add('ptr-spinning');
          ptr.style.transform = 'translate(-50%, 10px)';
          ptr.style.opacity = '1';
          if (ptrText) ptrText.textContent = 'Refreshing...';
          try {
            await renderCurrentHash();
          } finally {
            setTimeout(() => {
              ptr.classList.add('hidden');
              ptr.classList.remove('ptr-spinning', 'ptr-ready');
              ptr.style.transform = '';
              ptr.style.opacity = '';
              refreshing = false;
            }, 220);
          }
        } else {
          ptr.classList.add('hidden');
          ptr.classList.remove('ptr-ready', 'ptr-spinning');
          ptr.style.transform = '';
          ptr.style.opacity = '';
        }
      };

      main.addEventListener('touchend', finishPtr, { passive: true });
      main.addEventListener('touchcancel', finishPtr, { passive: true });
    }
  }

  async function init() {
    applyOutputVolume();
    $('pb-volume').value = String(state.volume);
    updateVolumeIcon();
    syncShuffleUI();
    localStorage.removeItem('beatlab_quality');
    wirePlayerBarToggle();
    wireFsSwipeClose();
    wireSheetAndPtrGestures();
    wirePwaLifecycle();
    syncResponsiveLayout();
    window.matchMedia('(max-width: 1060px)').addEventListener('change', syncResponsiveLayout);
    window.matchMedia('(max-width: 780px)').addEventListener('change', syncResponsiveLayout);
    window.addEventListener('pageshow', (e) => {
      if (e.persisted && !state.user) {
        resetAuthForm();
      }
    });
    const flushOnExit = () => {
      if (state.queueSaveDirty) flushServerQueue(true);
    };
    window.addEventListener('pagehide', flushOnExit);
    window.addEventListener('beforeunload', flushOnExit);

    $('auth-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const errBox = $('auth-error');
      errBox.classList.add('hidden');
      const username = $('auth-username').value.trim();
      const password = $('auth-password').value;
      try {
        const res = await api('/api/web/auth/signin', {
          method: 'POST',
          body: JSON.stringify({ username, password }),
        });
        if (res.mfaRequired) {
          state.authChallenge = res.challenge;
          state.authFactor = res.totp ? 'totp' : 'passkey';
          $('auth-form').classList.add('hidden');
          $('auth-passkey-btn').classList.add('hidden');
          $('auth-mfa').classList.remove('hidden');
          $('auth-mfa-hint').textContent = res.totp
            ? 'Enter the 6-digit code from your authenticator app, or one of your recovery codes.'
            : 'This account signs in with passkeys. Verify one to continue.';
          const mfaForm = $('mfa-form');
          const passkeyBtn = $('mfa-passkey-btn');
          if (mfaForm) mfaForm.classList.toggle('hidden', !res.totp);
          if (passkeyBtn) passkeyBtn.classList.toggle('hidden', !!res.totp);
          if (res.totp) $('mfa-code').focus();
          return;
        }
        if (res.restoreRequired) {
          state.restoreChallenge = res.challenge;
          $('auth-form').classList.add('hidden');
          $('auth-passkey-btn').classList.add('hidden');
          $('auth-restore').classList.remove('hidden');
          return;
        }
        state.token = res.token;
        state.user = res.user;
        localStorage.setItem('beatlab_token', res.token);
        showToast(res.created ? `Welcome, ${res.user.username}! Account created.` : `Welcome back, ${res.user.username}`);
        await onSignedIn();
      } catch (err) {
        errBox.textContent = err.message;
        errBox.classList.remove('hidden');
      }
    });

    const mfaForm = $('mfa-form');
    if (mfaForm) {
      mfaForm.addEventListener('submit', async (e) => {
        e.preventDefault();
        const errBox = $('mfa-error');
        errBox.classList.add('hidden');
        try {
          const res = await api('/api/web/auth/mfa/verify', {
            method: 'POST',
            body: JSON.stringify({ challenge: state.authChallenge, code: $('mfa-code').value }),
          });
          state.token = res.token;
          state.user = res.user;
          state.authChallenge = '';
          localStorage.setItem('beatlab_token', res.token);
          showToast(`Welcome back, ${res.user.username}`);
          await onSignedIn();
        } catch (err) {
          errBox.textContent = err.message;
          errBox.classList.remove('hidden');
        }
      });
    }

    const globalSearch = $('global-search');
    const triggerSearchFromInput = (rawVal) => {
      const q = String(rawVal !== undefined ? rawVal : globalSearch.value || '').trim();
      if (!q) return;
      state.ownSearchNavQuery = q;
      navigate(`search?q=${encodeURIComponent(q)}`);
    };

    const handleSearchBoxEdit = (rawVal) => {
      const val = String(rawVal || '');
      $('clear-search-btn').classList.toggle('hidden', !val);
      clearTimeout(state.searchTimer);
      state.searchTimer = null;

      const trimmed = val.trim();
      if (!trimmed) {
        if (state.searchFetchingQuery || state.searchLoadingUi) {
          abortActiveViewRequest();
          if (state.searchState) {
            renderSearchView();
          } else {
            renderExplore();
          }
        }
        return;
      }

      if (state.searchFetchingQuery && state.searchFetchingQuery !== trimmed) {
        abortActiveViewRequest();
        state.searchLoadingUi = true;
        setMainLoading(`Searching "${trimmed}"...`);
      } else if (!state.searchComposing) {
        state.searchLoadingUi = true;
        setMainLoading(`Searching "${trimmed}"...`);
      }

      state.searchTimer = setTimeout(() => {
        state.searchTimer = null;
        const liveQ = (globalSearch.value || '').trim();
        if (!liveQ) return;
        const currentHash = location.hash.replace(/^#/, '');
        const targetHash = `search?q=${encodeURIComponent(liveQ)}`;
        if (
          currentHash === targetHash &&
          (state.searchFetchingQuery === liveQ || (!state.searchLoadingUi && state.searchState?.query === liveQ))
        ) {
          return;
        }
        triggerSearchFromInput(liveQ);
      }, 320);
    };

    globalSearch.addEventListener('compositionstart', () => {
      state.searchComposing = true;
    });

    globalSearch.addEventListener('compositionend', (e) => {
      state.searchComposing = false;
      handleSearchBoxEdit(e.target.value);
    });

    globalSearch.addEventListener('input', (e) => {
      handleSearchBoxEdit(e.target.value);
    });

    globalSearch.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        if (e.isComposing || e.keyCode === 229 || state.searchComposing) return;
        e.preventDefault();
        clearTimeout(state.searchTimer);
        state.searchTimer = null;
        triggerSearchFromInput(e.target.value);
      }
    });

    wireAudioEvents();
    wireKeyboardShortcuts();
    wireTrackGestures();
    wireStatsTooltips();
    window.addEventListener('hashchange', () => renderCurrentHash());

    if (state.token) {
      try {
        const res = await api('/api/web/me');
        if (res.user) {
          state.user = res.user;
          await onSignedIn();
          return;
        }
      } catch (err) {
        if (err && err.isNetworkError) {
          showAuthScreen();
          return;
        }
      }
    }
    showAuthScreen();
    checkServerHealth();
  }

  async function onSignedIn() {
    resetAuthForm();
    showAppShell();
    $('user-name').textContent = state.user.username;
    $('user-avatar').innerHTML = avatarInner(selfAvatarUser());
    await Promise.all([refreshSidebarPlaylists(), restoreServerQueue()]);
    renderCurrentHash();
  }

  async function refreshSidebarPlaylists() {
    try {
      const lib = await api('/api/web/library');
      state.playlists = lib.playlists || [];
      renderSidebarPlaylistsDom();
    } catch {
      // ignore background error
    }
  }

  async function restoreServerQueue() {
    try {
      const q = await api('/api/web/queue');
      if (q.tracks && q.tracks.length > 0) {
        state.queue = cacheTracks(q.tracks);
        let idx = Math.max(0, Math.min(q.currentIndex || 0, state.queue.length - 1));
        if (q.currentId) {
          const byId = state.queue.findIndex((t) => t.id === q.currentId);
          if (byId >= 0) idx = byId;
        }
        state.queueIndex = idx;
        const cur = currentTrack();
        const savedSec = q.positionMs > 1000 ? q.positionMs / 1000 : 0;
        state.pendingSeekSec = savedSec;
        updateNowPlayingUI(cur, false);
        renderQueuePanel();
        if (cur) {
          audio.src = buildStreamUrl(cur);
          if (savedSec > 0) {
            const total = cur.duration > 0 ? cur.duration : savedSec;
            const clamped = Math.min(savedSec, total);
            const el = fmtTime(clamped);
            $('pb-elapsed').textContent = el;
            if ($('fs-elapsed')) $('fs-elapsed').textContent = el;
            if (cur.duration > 0) {
              const pct = Math.min(100, (clamped / cur.duration) * 100);
              $('pb-seek').value = String(pct);
              if ($('fs-seek')) $('fs-seek').value = String(pct);
              if ($('pb-mini-progress-fill')) $('pb-mini-progress-fill').style.width = `${pct.toFixed(2)}%`;
            }
          }
          loadLyricsForTrack(cur.id);
        }
      }
    } catch {
      // optional
    }
  }

  function flushServerQueue(keepalive = false) {
    if (state.queueSaveTimer) {
      clearTimeout(state.queueSaveTimer);
      state.queueSaveTimer = null;
    }
    state.queueSaveDirty = false;
    if (!state.token) return;
    const cur = currentTrack();
    api('/api/web/queue', {
      method: 'PUT',
      keepalive: !!keepalive,
      body: JSON.stringify({
        currentId: cur ? cur.id : '',
        currentIndex: Math.max(0, state.queueIndex),
        positionMs: Math.floor((audio.currentTime || 0) * 1000),
        trackIds: state.queue.map((t) => t.id),
      }),
    }).catch(() => {});
  }

  function saveServerQueue(immediate = false) {
    if (!state.token) return;
    state.queueSaveDirty = true;
    if (state.queueSaveTimer) {
      clearTimeout(state.queueSaveTimer);
      state.queueSaveTimer = null;
    }
    if (immediate) {
      flushServerQueue(false);
      return;
    }
    state.queueSaveTimer = setTimeout(() => {
      flushServerQueue(false);
    }, 650);
  }

  // ---------------------------------------------------------------------------
  // Routing & Views
  // ---------------------------------------------------------------------------
  function safeDecodeUri(str) {
    try {
      return decodeURIComponent(str);
    } catch {
      return str;
    }
  }

  function isSearchHash(hash) {
    return hash === 'search' || hash.startsWith('search?') || hash.startsWith('search/');
  }

  function extractSearchQueryFromHash(hash) {
    if (hash.startsWith('search/')) {
      return safeDecodeUri(hash.slice(7));
    }
    const qIdx = hash.indexOf('?');
    if (qIdx < 0) return '';
    return new URLSearchParams(hash.slice(qIdx + 1)).get('q') || '';
  }

  function abortActiveViewRequest() {
    state.viewReqToken += 1;
    if (state.viewAbortController) {
      state.viewAbortController.abort();
      state.viewAbortController = null;
    }
    state.searchFetchingQuery = '';
    state.searchLoadingUi = false;
  }

  function beginViewRequest() {
    abortActiveViewRequest();
    state.hasViewNetworkError = false;
    const controller = typeof AbortController !== 'undefined' ? new AbortController() : null;
    state.viewAbortController = controller;
    return {
      reqToken: state.viewReqToken,
      signal: controller ? controller.signal : undefined,
    };
  }

  function isViewRequestCurrent(reqToken, signal) {
    return reqToken === state.viewReqToken && (!signal || !signal.aborted);
  }

  function handleViewFetchError(err, reqToken, signal) {
    if (!isViewRequestCurrent(reqToken, signal) || (err && err.name === 'AbortError')) {
      return;
    }
    state.searchFetchingQuery = '';
    state.searchLoadingUi = false;
    state.hasViewNetworkError = true;
    $('main-view').innerHTML = `<p class="muted-center">${esc(err.message)}</p>`;
  }

  function navigate(route) {
    haptic('tap');
    setExpandedPlayer(false);
    closeTrackMenu();
    hideStatsTooltip();
    closeUserMenu();
    const clean = route.replace(/^#/, '');
    if (!isSearchHash(clean)) {
      clearTimeout(state.searchTimer);
      state.searchTimer = null;
      state.ownSearchNavQuery = null;
    }
    if (location.hash.replace(/^#/, '') === clean) {
      renderCurrentHash();
    } else {
      location.hash = `#${clean}`;
    }
  }

  function renderCurrentHash() {
    if (!state.user) return;
    const ownSearchQuery = state.ownSearchNavQuery;
    state.ownSearchNavQuery = null;

    const hash = location.hash.replace(/^#/, '') || 'home';
    document.querySelectorAll('.nav-item').forEach((el) => {
      const r = el.dataset.route;
      const match =
        hash === r ||
        hash.startsWith(`${r}?`) ||
        hash.startsWith(`${r}/`) ||
        (r === 'playlists' && hash.startsWith('playlist/')) ||
        (r === 'profile' && hash.startsWith('user/')) ||
        (r === 'library' && (hash === 'liked' || hash === 'history'));
      el.classList.toggle('active', match);
    });

    if (hash === 'home') return renderHome();
    if (hash === 'explore') return renderExplore();
    if (hash === 'community') return renderCommunity();
    if (hash === 'profile') return renderProfileView('me');
    if (hash.startsWith('user/')) return renderProfileView(safeDecodeUri(hash.slice(5)));
    if (hash.startsWith('explore-mix/')) return renderExploreMix(safeDecodeUri(hash.slice(12)));
    if (hash === 'mix' || hash.startsWith('mix?')) {
      const qIdx = hash.indexOf('?');
      const params = qIdx >= 0 ? new URLSearchParams(hash.slice(qIdx + 1)) : null;
      const mixQ = params ? params.get('q') || '' : '';
      const mixMode = params ? params.get('mode') || 'auto' : 'auto';
      return renderInstantMixPage(mixQ, mixMode);
    }
    if (isSearchHash(hash)) {
      const q = extractSearchQueryFromHash(hash);
      const searchInput = $('global-search');
      const liveTrimmed = searchInput ? (searchInput.value || '').trim() : '';

      if (ownSearchQuery !== null && ownSearchQuery === q) {
        if (searchInput && liveTrimmed !== q) {
          return;
        }
      } else if (searchInput && !state.searchComposing && !state.searchTimer) {
        if (searchInput.value !== q) {
          searchInput.value = q;
          $('clear-search-btn').classList.toggle('hidden', !q);
        }
      }

      if (q && state.searchFetchingQuery === q) {
        return;
      }
      return renderSearch(q);
    }
    if (hash.startsWith('album/')) return renderAlbum(safeDecodeUri(hash.slice(6)));
    if (hash.startsWith('artist/')) return renderArtist(safeDecodeUri(hash.slice(7)));
    if (hash.startsWith('playlist/')) return renderPlaylist(safeDecodeUri(hash.slice(9)));
    if (hash.startsWith('mood/')) return renderMood(safeDecodeUri(hash.slice(5)));
    if (hash.startsWith('genre/')) return renderGenre(safeDecodeUri(hash.slice(6)));
    if (hash === 'playlists') return renderPlaylistsView();
    if (hash === 'library') return renderLibrary(state.libTab);
    if (hash === 'liked') return renderLibrary('liked');
    if (hash === 'history') return renderLibrary('history');
    if (hash === 'stats' || hash.startsWith('stats?')) {
      const qIdx = hash.indexOf('?');
      const rangeParam = qIdx >= 0 ? new URLSearchParams(hash.slice(qIdx + 1)).get('range') : '';
      return renderStats(rangeParam || state.statsRange || 'all');
    }
    return renderHome();
  }

  function setMainLoading(label = 'Loading...', showSpinner = true) {
    $('main-view').innerHTML = `
      <div class="main-loading-state" role="status" aria-live="polite">
        ${showSpinner ? '<div class="loading-spinner" aria-hidden="true"></div>' : ''}
        <p class="loading-label">${esc(label || 'Loading...')}</p>
      </div>
    `;
  }

  async function renderHome() {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Loading...');
    try {
      const d = await api('/api/web/home', { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const quickPicks = cacheTracks(d.quickPicks || []).slice(0, 6);
      const newReleases = d.newReleases || [];
      const trending = cacheTracks(d.trending || []);
      const recent = cacheTracks(d.recent || []);
      const relatedTracks = cacheTracks(d.relatedTracks || []);
      const mixes = d.madeForYou || [];
      const mixedForYou = d.mixedForYou || [];
      const albums = d.albums || [];
      const artists = d.artists || [];
      const recommendedArtists = d.recommendedArtists || [];
      const moods = d.moods || [];

      let html = `<h1 class="page-greeting">${greetingText()}, ${esc(state.user.username)}</h1>`;

      if (quickPicks.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Quick Picks</h2></div>
          <div class="quick-picks-grid">${quickPicks.map(renderQuickTile).join('')}</div>
        </section>`;
      }

      if (mixedForYou.length > 0) {
        html += `<section class="section">
          <div class="section-head">
            <h2>Mixed for you</h2>
          </div>
          <div class="mixed-for-you-grid">${mixedForYou.map(renderMixedForYouCard).join('')}</div>
        </section>`;
      }

      if (newReleases.length > 0) {
        html += `<section class="section">
          <div class="section-head">
            <h2>New Releases</h2>
            <span class="section-hint">Fresh drops from artists you follow &amp; listen to</span>
          </div>
          <div class="card-grid">${newReleases.map(renderNewReleaseCard).join('')}</div>
        </section>`;
      }

      if (mixes.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Made For You</h2></div>
          <div class="card-grid">${mixes.map(renderPlaylistCard).join('')}</div>
        </section>`;
      }

      if (recent.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Recently Played</h2></div>
          <div class="card-grid">${recent.slice(0, 8).map(renderTrackCard).join('')}</div>
        </section>`;
      }

      if (relatedTracks.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Related Tracks For You</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(relatedTracks.map((t) => t.id)))}, 0)">Play All</button></div>
          <div class="track-table">${renderTrackRows(relatedTracks.slice(0, 10), relatedTracks)}</div>
        </section>`;
      }

      if (recommendedArtists.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Artists You Might Like</h2></div>
          <div class="card-grid">${recommendedArtists.map(renderArtistCard).join('')}</div>
        </section>`;
      }

      if (moods.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Moods &amp; Vibes</h2><button class="text-btn" onclick="App.navigate('explore')">See all</button></div>
          <div class="mood-grid">${moods.slice(0, 5).map(renderMoodTile).join('')}</div>
        </section>`;
      }

      if (trending.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Trending Now</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(trending.map((t) => t.id)))}, 0)">Play All</button></div>
          <div class="track-table">${renderTrackRows(trending.slice(0, 12))}</div>
        </section>`;
      }

      if (albums.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Popular Albums</h2></div>
          <div class="card-grid">${albums.map(renderAlbumCard).join('')}</div>
        </section>`;
      }

      if (artists.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Top Artists</h2></div>
          <div class="card-grid">${artists.map(renderArtistCard).join('')}</div>
        </section>`;
      }

      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  function renderExploreMixCard(item) {
    const covers = Array.isArray(item.covers) ? item.covers.filter(Boolean) : [];
    const artHtml = covers.length >= 4
      ? `<div class="explore-collage">${covers.slice(0, 4).map((c) => `<img src="${esc(c)}" loading="lazy" alt="" />`).join('')}</div>`
      : covers.length >= 1
        ? `<img class="explore-single-art" src="${esc(covers[0])}" loading="lazy" alt="" />`
        : `<div class="explore-art-fallback" style="background:${esc(item.c2)};box-shadow:inset 0 -3px 0 ${esc(item.c1)}"><span>${esc(item.title.slice(0, 2).toUpperCase())}</span></div>`;
    const countLabel = item.trackCount > 0 ? `${item.trackCount} curated tracks` : 'Curating on demand';
    return `<div class="explore-mix-card" style="border-color:${esc(item.c1)}44" onclick="App.navigate('explore-mix/${encodeURIComponent(item.key)}')">
      <div class="explore-mix-art" style="background:${esc(item.c2)}">${artHtml}</div>
      <div class="explore-mix-body">
        <div class="explore-mix-kicker" style="color:${esc(item.c1)}">ERA MIX</div>
        <h3>${esc(item.title)}</h3>
        <p>${esc(item.subtitle)}</p>
        <div class="explore-mix-meta">${esc(countLabel)}</div>
      </div>
    </div>`;
  }

  function renderMixedForYouCard(m) {
    const covers = Array.isArray(m.covers) ? m.covers.filter(Boolean) : [];
    const artHtml = covers.length
      ? `<div class="mixfor-collage${covers.length < 4 ? ' sparse' : ''}">${covers.slice(0, 4).map((c) => `<img src="${esc(c)}" loading="lazy" alt="" />`).join('')}</div>`
      : `<div class="mixfor-fallback"></div>`;
    const sub = `${m.trackCount || 0} tracks · ${m.subtitle || ''}`;
    return `<div class="mixfor-card" data-share-kind="playlist" data-share-id="${esc(m.id)}" data-share-title="${esc(m.title)}" data-share-sub="${esc(sub)}" onclick="App.navigate('playlist/${encodeURIComponent(m.id)}')">
      <div class="mixfor-art" style="--mix-c1:${esc(m.c1)};--mix-c2:${esc(m.c2)}">
        ${artHtml}
        <div class="mixfor-overlay"></div>
        <div class="mixfor-art-title"><span>${esc(m.art_title)}</span>${m.is_super ? '' : `<span class="mixfor-num">${esc(m.number || '')}</span>`}</div>
        <button class="mixfor-play" title="Play ${esc(m.title)}" aria-label="Play ${esc(m.title)}" onclick="event.stopPropagation();App.playPlaylistNow(${esc(JSON.stringify(m.id))})">${ICONS.play}</button>
      </div>
      <div class="mixfor-info">
        <div class="mixfor-name">${esc(m.title)}</div>
        <div class="mixfor-artists">${esc(m.subtitle || '')}</div>
      </div>
    </div>`;
  }

  async function renderExplore() {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Exploring charts, moods & genres...');
    try {
      const d = await api('/api/web/explore', { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const charts = cacheTracks(d.charts || []);
      const discoveryPicks = cacheTracks(d.discoveryPicks || []);
      const relatedTracks = cacheTracks(d.relatedTracks || []);
      const decades = d.decades || [];
      const artists = d.artists || [];
      const recommendedArtists = d.recommendedArtists || [];
      const curator = d.curator || null;
      const curatorBadge = curator
        ? `<span class="curator-pill" title="Server continuously curates moods, decades, and discovery pools in the background"><span class="curator-dot"></span>Live Curator · ${Number(curator.catalogTracks || 0).toLocaleString()} tracks · ${curator.readyMoods || 0}/${curator.totalMoods || 10} moods ready</span>`
        : '';

      let html = `<div class="section-head" style="margin-bottom:18px;gap:12px;flex-wrap:wrap">
        <div style="display:flex;align-items:center;gap:12px;flex-wrap:wrap">
          <h1 class="page-greeting" style="margin-bottom:0">Explore</h1>
          ${curatorBadge}
        </div>
        <button class="btn-secondary" onclick="App.promptInstantMix()">Create Instant Mix</button>
      </div>`;

      html += `<section class="section">
        <div class="section-head"><h2>Moods &amp; Vibes</h2></div>
        <div class="mood-grid">${(d.moods || []).map(renderMoodTile).join('')}</div>
      </section>`;

      if (decades.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Decades &amp; Time Machine</h2></div>
          <div class="explore-mix-grid">${decades.map(renderExploreMixCard).join('')}</div>
        </section>`;
      }

      if (discoveryPicks.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Fresh Discovery Picks</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(discoveryPicks.map((t) => t.id)))}, 0)">Play All</button></div>
          <div class="track-table">${renderTrackRows(discoveryPicks)}</div>
        </section>`;
      }

      if (relatedTracks.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Related Tracks</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(relatedTracks.map((t) => t.id)))}, 0)">Play All</button></div>
          <div class="track-table">${renderTrackRows(relatedTracks)}</div>
        </section>`;
      }

      if (recommendedArtists.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Artists You Might Like</h2></div>
          <div class="card-grid">${recommendedArtists.map(renderArtistCard).join('')}</div>
        </section>`;
      }

      if (artists.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Featured Artists</h2></div>
          <div class="card-grid">${artists.map(renderArtistCard).join('')}</div>
        </section>`;
      }

      if ((d.genres || []).length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Genres</h2></div>
          <div class="card-grid">${d.genres.slice(0, 24).map((g) => `
            <div class="media-card" onclick="App.navigate('genre/${encodeURIComponent(g.name)}')">
              <div class="card-art-wrap"><img src="${esc(g.coverUrl)}" loading="lazy" alt="${esc(g.name)}" /></div>
              <div class="card-title">${esc(g.name)}</div>
              <div class="card-sub">Genre Mix</div>
            </div>`).join('')}</div>
        </section>`;
      }

      if ((d.newAlbums || []).length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Chart Albums</h2></div>
          <div class="card-grid">${d.newAlbums.map(renderAlbumCard).join('')}</div>
        </section>`;
      }

      if (charts.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Top 30 Charts</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(charts.map((t) => t.id)))}, 0)">Play Charts</button></div>
          <div class="track-table">${renderTrackRows(charts)}</div>
        </section>`;
      }
      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  async function renderExploreMix(key) {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Preparing curated mix...');
    try {
      const d = await api(`/api/web/explore/mix/${encodeURIComponent(key)}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const m = d.mix;
      const tracks = cacheTracks(d.tracks || []);
      state.currentMoodShare = {
        kind: 'mix',
        targetId: `explore:${m.key}`,
        title: m.title,
        subtitle: m.subtitle || 'Curated Mix',
        trackIds: tracks.map((t) => t.id),
        coverUrl: tracks[0]?.coverUrl || '/api/web/cover/default',
      };
      const idsJson = esc(JSON.stringify(tracks.map((t) => t.id)));
      const covers = [...new Set(tracks.map((t) => t.coverUrl).filter(Boolean))].slice(0, 4);
      const artHtml = covers.length >= 4
        ? `<div class="hero-collage">${covers.map((c) => `<img src="${esc(c)}" alt="" />`).join('')}</div>`
        : covers.length >= 1
          ? `<img class="hero-art" src="${esc(covers[0])}" alt="" />`
          : '';
      let html = `<div class="hero-header" style="background:${esc(m.c2)};border-color:${esc(m.c1)}55;border-left:4px solid ${esc(m.c1)};">
        ${artHtml}
        <div class="hero-info">
          <div class="hero-kicker" style="color:#fff">ERA MIX</div>
          <h1 class="hero-title">${esc(m.title)}</h1>
          <div class="hero-sub" style="color:rgba(255,255,255,0.88)">${esc(m.subtitle)} · ${tracks.length} tracks · ${fmtTotalDur(m.totalDuration || 0)}</div>
          <div class="hero-actions">
            ${tracks.length ? `<button class="btn-primary" onclick="App.playListByIds(${idsJson}, 0)">${ICONS.play} Play Mix</button>` : ''}
            ${tracks.length ? `<button class="btn-secondary" onclick="App.playShuffledIds(${idsJson})">Shuffle</button>` : ''}
            ${tracks.length ? `<button class="btn-secondary" onclick="App.shareCurrentMoodMix()">${ICONS.share} Share Mix</button>` : ''}
            <button class="btn-secondary" onclick="App.navigate('explore')">Back to Explore</button>
          </div>
        </div>
      </div>
      ${tracks.length ? `<div class="track-table">${renderTrackRows(tracks, tracks, null, false, 'explore')}</div>` : `<p class="muted-center">No tracks available for this mix yet. Check back shortly while the curator refreshes.</p>`}`;
      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  async function renderSearch(query) {
    const cleanQ = String(query || '').trim();
    if (!cleanQ) {
      return renderExplore();
    }
    const { reqToken, signal } = beginViewRequest();
    const personQuery = cleanQ.startsWith('@');
    state.searchFetchingQuery = cleanQ;
    state.searchLoadingUi = true;
    setMainLoading(personQuery ? `Finding people for "${cleanQ}"...` : `Searching "${cleanQ}"...`);
    try {
      const [d, social] = await Promise.all([
        personQuery
          ? Promise.resolve({ tracks: [], albums: [], artists: [], refinements: null })
          : api(`/api/web/search?q=${encodeURIComponent(cleanQ)}&phase=fast`, { signal }),
        api(`/api/web/social/search?q=${encodeURIComponent(cleanQ)}&limit=20`, { signal }).catch(() => ({ users: [] })),
      ]);
      if (!isViewRequestCurrent(reqToken, signal)) return;
      state.searchFetchingQuery = '';
      state.searchLoadingUi = false;

      const tracks = cacheTracks(d.tracks || []);
      const albums = d.albums || [];
      const artists = d.artists || [];
      const users = social.users || [];
      const refinements = d.refinements || null;

      state.searchState = {
        query: (!tracks.length && !albums.length && !artists.length && !users.length) ? cleanQ : query,
        tracks,
        albums,
        artists,
        users,
        instantMix: null,
        instantMixLoading: false,
        instantMixError: '',
        refinements,
        activeFilter: 'all',
        mixExpanded: false,
      };
      state.activeInstantMix = null;
      renderSearchView();
      $('main-view').scrollTop = 0;

      if (!personQuery) {
        api(`/api/web/search?q=${encodeURIComponent(cleanQ)}&phase=slow`, { signal })
          .then((slowData) => {
            if (!isViewRequestCurrent(reqToken, signal) || !state.searchState) return;
            const extra = cacheTracks(slowData?.tracks || []);
            if (!extra.length) return;
            const existing = state.searchState.tracks || [];
            const seenIds = new Set(existing.map((t) => t.id));
            const byKey = new Map();
            existing.forEach((t, idx) => {
              const k = `${String(t.title || '').toLowerCase()}::${String(t.artist || '').toLowerCase()}`;
              if (!byKey.has(k)) byKey.set(k, idx);
            });
            const merged = existing.slice();
            let changed = false;
            for (const t of extra) {
              if (seenIds.has(t.id)) continue;
              const k = `${String(t.title || '').toLowerCase()}::${String(t.artist || '').toLowerCase()}`;
              const idx = byKey.get(k);
              if (idx !== undefined) {
                const cur = merged[idx];
                const curFmt = String(cur.format || '').toLowerCase();
                const newFmt = String(t.format || '').toLowerCase();
                const curLossless = curFmt === 'flac' || curFmt === 'alac' || curFmt === 'wav' || curFmt === 'aiff';
                const newLossless = newFmt === 'flac' || newFmt === 'alac' || newFmt === 'wav' || newFmt === 'aiff';
                if (newLossless && !curLossless) {
                  merged[idx] = t;
                  seenIds.add(t.id);
                  changed = true;
                }
              } else {
                byKey.set(k, merged.length);
                merged.push(t);
                seenIds.add(t.id);
                changed = true;
              }
            }
            if (changed) {
              state.searchState.tracks = merged;
              renderSearchView();
            }
          })
          .catch(() => {});
      }
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  function applySearchFilter(tracks, filterKey) {
    if (!filterKey || filterKey === 'all') return tracks;
    if (filterKey === 'taste') return tracks.filter((t) => t.isTasteMatch);
    if (filterKey === 'hits') return tracks.filter((t) => t.isPopular);
    if (filterKey === 'lossless') return tracks.filter((t) => t.isLossless || t.source === 'local');
    if (filterKey.startsWith('mood:')) {
      const want = filterKey.slice(5);
      return tracks.filter((t) => String(t.moods || '').split(',').includes(want));
    }
    return tracks;
  }

  function renderInstantMixCard(mix, refinements, query, expanded) {
    const covers = [];
    for (const t of mix.tracks) {
      if (t.coverUrl && !covers.includes(t.coverUrl)) {
        covers.push(t.coverUrl);
        if (covers.length === 4) break;
      }
    }
    const artHtml =
      covers.length >= 4
        ? `<div class="mix-mosaic">${covers.map((c) => `<img src="${esc(c)}" loading="lazy" alt="" />`).join('')}</div>`
        : `<img class="mix-single-art" src="${esc(covers[0] || '')}" alt="" />`;
    const previewCount = expanded ? mix.tracks.length : Math.min(6, mix.tracks.length);
    const intentTag = refinements?.detectedIntent ? ` · ${esc(refinements.detectedIntent)}` : '';

    const isProminent = !!(mix.prominent || mix.isThemeMix);
    return `<section class="instant-mix-card ${isProminent ? 'prominent' : ''}" data-share-kind="mix" data-share-id="${esc(`mix:${mix.title || query}`)}" data-share-title="${esc(mix.title)}" data-share-sub="${esc(mix.subtitle || '')}" data-share-cover="${esc(covers[0] || '')}">
      <div class="instant-mix-head">
        ${artHtml}
        <div class="instant-mix-info">
          <div class="hero-kicker">INSTANT CUSTOM MIX · EPHEMERAL${intentTag}</div>
          <h2 class="instant-mix-title">${esc(mix.title)}</h2>
          <p class="instant-mix-sub">${esc(mix.subtitle)} · ${mix.tracks.length} tracks · ${fmtTotalDur(mix.totalDuration)}</p>
          <div class="hero-actions">
            <button class="btn-primary" onclick="App.playInstantMix(false, 0)">${ICONS.play} Play Mix</button>
            <button class="btn-secondary" onclick="App.playInstantMix(true, 0)">Shuffle</button>
            <button class="btn-secondary" onclick="App.queueInstantMix()">Add to Queue</button>
            ${
              mix.savedPlaylistId
                ? `<button class="btn-secondary" onclick="App.navigate('playlist/${encodeURIComponent(mix.savedPlaylistId)}')">Saved ✓</button>
                   <button class="btn-secondary" style="color:#fda4af" onclick="App.deletePlaylist(${esc(JSON.stringify(mix.savedPlaylistId))})">Delete Playlist</button>`
                : `<button class="btn-secondary" onclick="App.promptSaveInstantMix()">+ Save to Playlists</button>`
            }
            <button class="btn-secondary" onclick="App.shareCurrentInstantMix()">${ICONS.share} Share Mix</button>
          </div>
        </div>
      </div>
      <div class="track-table instant-mix-tracks">
        ${renderTrackRows(mix.tracks.slice(0, previewCount), mix.tracks, null, false, query)}
      </div>
      ${
        mix.tracks.length > 6
          ? `<div class="instant-mix-footer">
              <button class="mini-btn" onclick="App.toggleMixExpand()">${
                expanded ? 'Show fewer tracks' : `Show all ${mix.tracks.length} tracks in mix`
              }</button>
            </div>`
          : ''
      }
    </section>`;
  }

  async function createInstantMixForSearch(customTerm) {
    const s = state.searchState;
    const term = String(customTerm ?? (s ? s.query : '')).trim();
    if (!term) {
      return promptInstantMix();
    }
    if (!s || (customTerm != null && term !== s.query)) {
      return navigate(`mix?q=${encodeURIComponent(term)}`);
    }
    if (s.instantMixLoading) return;
    s.instantMixLoading = true;
    s.instantMixError = '';
    renderSearchView();
    try {
      const d = await api(`/api/web/instant-mix?q=${encodeURIComponent(term)}`);
      if (state.searchState !== s) return;
      s.instantMixLoading = false;
      if (d.instantMix && Array.isArray(d.instantMix.tracks) && d.instantMix.tracks.length > 0) {
        const mix = {
          ...d.instantMix,
          tracks: cacheTracks(d.instantMix.tracks || []),
        };
        s.instantMix = mix;
        s.mixExpanded = false;
        state.activeInstantMix = { mix, query: term, refinements: d.refinements || s.refinements, expanded: false };
      } else {
        s.instantMixError = `Couldn't generate an instant mix for "${term}". Try a different artist, song, genre, or mood.`;
        showToast('No instant mix found for that term');
      }
      renderSearchView();
    } catch (e) {
      if (state.searchState === s) {
        s.instantMixLoading = false;
        s.instantMixError = e.message || 'Could not create instant mix right now.';
        renderSearchView();
      }
    }
  }

  async function renderInstantMixPage(query, mode = 'auto') {
    const cleanQ = String(query || '').trim();
    const cleanMode = ['topic', 'entity'].includes(mode) ? mode : 'auto';
    state.instantMixMode = cleanMode;
    state.searchState = null;
    const modeOptions = [
      ['auto', 'Auto'],
      ['topic', 'Genre & Vibe'],
      ['entity', 'Artist or Song'],
    ];
    const builderHeader = (val) => `
      <div class="section-head" style="margin-bottom:14px">
        <h1 class="page-greeting" style="margin-bottom:0">Instant Mix</h1>
      </div>
      <form class="instant-mix-builder-form" onsubmit="App.submitInstantMixForm(event)">
        <input id="instant-mix-term-input" type="text" value="${esc(val)}" placeholder="Enter an artist, song, genre, era, or mood (e.g. 80s hits, Daft Punk, chill)..." autocomplete="off" />
        <button type="submit" class="btn-primary">Create Instant Mix</button>
      </form>
      <div class="instant-mix-mode-row" role="group" aria-label="Mix mode">
        ${modeOptions
          .map(
            ([key, label]) =>
              `<button type="button" class="refine-query-pill${cleanMode === key ? ' active' : ''}" onclick="App.setInstantMixMode('${key}')">${esc(label)}</button>`
          )
          .join('')}
      </div>
    `;
    if (!cleanQ) {
      state.activeInstantMix = null;
      $('main-view').innerHTML = `
        ${builderHeader('')}
        <div class="instant-mix-suggestions">
          ${['80s hits', '90s rock', 'Late night chill', 'Workout energy', 'Indie synthpop']
            .map((sg) => `<button class="refine-query-pill" onclick="App.navigate('mix?q=${encodeURIComponent(sg)}')">${esc(sg)}</button>`)
            .join('')}
        </div>
      `;
      return;
    }

    const { reqToken, signal } = beginViewRequest();
    setMainLoading(`Creating instant mix for "${cleanQ}"...`);
    try {
      const modeParam = cleanMode !== 'auto' ? `&mode=${encodeURIComponent(cleanMode)}` : '';
      const d = await api(`/api/web/instant-mix?q=${encodeURIComponent(cleanQ)}${modeParam}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      if (!d.instantMix || !Array.isArray(d.instantMix.tracks) || !d.instantMix.tracks.length) {
        state.activeInstantMix = null;
        $('main-view').innerHTML = `
          ${builderHeader(cleanQ)}
          <p class="muted-center">Couldn't build an instant mix for "${esc(cleanQ)}". Try another artist, song, era, or vibe.</p>
        `;
        return;
      }
      const mix = {
        ...d.instantMix,
        tracks: cacheTracks(d.instantMix.tracks || []),
      };
      state.activeInstantMix = {
        mix,
        query: cleanQ,
        refinements: d.refinements || null,
        expanded: true,
      };
      $('main-view').innerHTML = `
        ${builderHeader(cleanQ)}
        ${renderInstantMixCard(mix, d.refinements || null, cleanQ, true)}
      `;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  function promptInstantMix(initialQuery = '') {
    const defaultVal = String(initialQuery || (state.searchState ? state.searchState.query : '') || '').trim();
    openModal(`
      <div class="modal-head"><h3>Create Instant Mix</h3><button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button></div>
      <form onsubmit="App.submitInstantMixModal(event)" style="display:flex;flex-direction:column;gap:12px">
        <p style="font-size:13px;color:var(--text-muted)">Generate an on-demand mix from any artist, song, genre, era, or mood.</p>
        <input id="instant-mix-modal-input" type="text" value="${esc(defaultVal)}" placeholder="e.g. 80s hits, Daft Punk, late night chill..." required autofocus />
        <div class="instant-mix-suggestions">
          ${['80s hits', '90s rock', 'Chill synthwave', 'Workout energy']
            .map((sg) => `<button type="button" class="refine-query-pill" onclick="App.closeModal();App.navigate('mix?q=${encodeURIComponent(sg)}')">${esc(sg)}</button>`)
            .join('')}
        </div>
        <button type="submit" class="btn-primary">Create Instant Mix</button>
      </form>
    `);
  }

  function renderSearchView() {
    const s = state.searchState;
    if (!s) return;
    const { query, albums, artists, users, instantMix, refinements, activeFilter, mixExpanded } = s;
    const tracks = applySearchFilter(s.tracks, activeFilter);

    // "@username" searches render people directly in the main results view.
    if (String(query || '').trim().startsWith('@')) {
      const term = String(query).trim();
      const people = users || [];
      let personHtml = `<div class="people-search-head">
        <div class="hero-kicker">PEOPLE</div>
        <h1 class="hero-title">Listeners matching “${esc(term)}”</h1>
        <p class="hero-sub">Follow people to hear their auto-updating radios in your Community feed.</p>
      </div>`;
      personHtml += people.length
        ? `<div class="people-search-grid">${people.map(personCardHtml).join('')}</div>`
        : emptyState('No listeners found', `Nobody matched “${esc(term)}”. Try a different name or a partial username.`);
      $('main-view').innerHTML = personHtml;
      $('main-view').scrollTop = 0;
      return;
    }

    let html = '';

    // Search Refinement & Sub-Query Bar
    const chips = refinements?.filterChips || [];
    const suggestions = refinements?.querySuggestions || [];
    if (chips.length > 0 || suggestions.length > 0) {
      html += `<div class="search-refine-bar">`;
      if (chips.length > 0) {
        html += `<div class="refine-group">
          <button class="tab-chip ${activeFilter === 'all' ? 'active' : ''}" onclick="App.setSearchFilter('all')">All (${s.tracks.length})</button>
          ${chips
            .map(
              (c) =>
                `<button class="tab-chip ${activeFilter === c.key ? 'active' : ''}" onclick="App.setSearchFilter(${esc(JSON.stringify(c.key))})">${esc(c.label)} (${c.count})</button>`
            )
            .join('')}
        </div>`;
      }
      if (suggestions.length > 0) {
        html += `<div class="refine-group refine-suggestions">
          <span class="refine-label">Related:</span>
          ${suggestions
            .map(
              (sg) =>
                `<button class="refine-query-pill" onclick="App.runRefinedSearch(${esc(JSON.stringify(sg))})">${esc(sg)}</button>`
            )
            .join('')}
        </div>`;
      }
      html += `</div>`;
    }

    if (instantMix && activeFilter === 'all') {
      html += renderInstantMixCard(instantMix, refinements, query, mixExpanded);
    }

    if (activeFilter === 'all' && users && users.length) {
      html += `<section class="section">
        <div class="section-head"><h2>People</h2><span style="color:var(--text-dim);font-size:12px">Matching “@${esc(query)}”</span></div>
        <div class="user-grid">${users.map(userCardHtml).join('')}</div>
      </section>`;
    }

    if (!s.tracks.length && !albums.length && !artists.length && !(users && users.length)) {
      html += `<p class="muted-center">No matches found for "${esc(query)}".</p>`;
      $('main-view').innerHTML = html;
      return;
    }

    const topTrack = tracks[0];
    if (tracks.length > 0) {
      html += `<div class="search-hero-row">`;
      if (topTrack) {
        html += `<section>
          <div class="section-head"><h2>Top Result</h2></div>
          <div class="top-result-card" data-tid="${esc(topTrack.id)}" onclick="App.playSearchTrack(${esc(JSON.stringify(topTrack.id))})">
            <img src="${esc(topTrack.coverUrl)}" alt="" />
            <div class="top-result-title">${esc(topTrack.title)}</div>
            <div class="card-sub">${esc(topTrack.artist)} · ${esc(topTrack.album)}</div>
            <div style="display:flex;gap:6px;flex-wrap:wrap;align-items:center">
              <span class="top-result-type">SONG · ${esc(topTrack.qualityBadge)}</span>
              ${topTrack.isTasteMatch ? `<span class="taste-pill">For You</span>` : ''}
            </div>
            <button class="card-play-fab" style="opacity:1;transform:none;" onclick="event.stopPropagation();App.playSearchTrack(${esc(JSON.stringify(topTrack.id))})">${ICONS.play}</button>
          </div>
        </section>`;
      } else {
        html += `<div></div>`;
      }

      html += `<section>
        <div class="section-head"><h2>${activeFilter === 'all' ? 'Songs' : 'Refined Songs'}</h2>${
        tracks.length ? `<button class="mini-btn" onclick="App.startRadioForId(${esc(JSON.stringify(tracks[0].id))})">Start Radio</button>` : ''
      }</div>
        <div class="track-table">${renderTrackRows(tracks.slice(0, 8), null, null, true, query)}</div>
      </section></div>`;
    } else if (activeFilter !== 'all') {
      html += `<p class="muted-center" style="margin:24px 0">No tracks matched this filter. <button class="mini-btn" onclick="App.setSearchFilter('all')">Reset filter</button></p>`;
    }

    if (artists.length > 0 && activeFilter === 'all') {
      html += `<section class="section">
        <div class="section-head"><h2>Artists</h2></div>
        <div class="card-grid">${artists.map(renderArtistCard).join('')}</div>
      </section>`;
    }
    if (albums.length > 0 && activeFilter === 'all') {
      html += `<section class="section">
        <div class="section-head"><h2>Albums</h2></div>
        <div class="card-grid">${albums.map(renderAlbumCard).join('')}</div>
      </section>`;
    }
    if (tracks.length > 8) {
      html += `<section class="section">
        <div class="section-head"><h2>More Tracks</h2></div>
        <div class="track-table">${renderTrackRows(tracks.slice(8), null, null, true, query)}</div>
      </section>`;
    }

    $('main-view').innerHTML = html;
  }

  async function renderAlbum(id) {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Loading album & tracklist...');
    try {
      let hintName = '';
      let hintArtist = '';
      const cur = currentTrack();
      if (cur && cur.albumId === id) {
        hintName = cur.album || '';
        hintArtist = cur.artist || '';
      } else {
        for (const t of state.trackMap.values()) {
          if (t.albumId === id) {
            hintName = t.album || '';
            hintArtist = t.artist || '';
            break;
          }
        }
      }
      const qs = new URLSearchParams();
      if (hintName) qs.set('name', hintName);
      if (hintArtist) qs.set('artist', hintArtist);
      const qStr = qs.toString() ? `?${qs.toString()}` : '';
      const d = await api(`/api/web/album/${encodeURIComponent(id)}${qStr}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const alb = d.album;
      const tracks = cacheTracks(d.tracks || []);
      const idsJson = esc(JSON.stringify(tracks.map((t) => t.id)));

      let html = `<div class="hero-header">
        <img class="hero-art" src="${esc(alb.coverUrl)}" alt="${esc(alb.name)}" />
        <div class="hero-info">
          <div class="hero-kicker">ALBUM</div>
          <h1 class="hero-title">${esc(alb.name)}</h1>
          <div class="hero-sub">
            <a href="#artist/${encodeURIComponent(alb.artistId)}"><strong>${esc(alb.artist)}</strong></a>
            ${alb.year ? ` · ${alb.year}` : ''}
            ${alb.genre ? ` · ${esc(alb.genre)}` : ''}
            · ${tracks.length} songs · ${fmtTotalDur(alb.duration)}
            ${alb.label ? `<br><span style="opacity:.75">${esc(alb.label)}</span>` : ''}
          </div>
          <div class="hero-actions">
            <button class="btn-primary" onclick="App.playListByIds(${idsJson}, 0)">${ICONS.play} Play</button>
            <button class="btn-secondary" onclick="App.playShuffledIds(${idsJson})">Shuffle</button>
            <button id="save-alb-btn" class="btn-secondary ${alb.isSaved ? 'active' : ''}" onclick="App.toggleSaveAlbum(${esc(JSON.stringify(alb.id))}, ${!alb.isSaved})">${ICONS.heart} ${alb.isSaved ? 'Saved' : 'Save Album'}</button>
            <button class="btn-secondary" onclick="App.shareAlbum(${esc(JSON.stringify(alb.id))}, ${esc(JSON.stringify(alb.name))}, ${esc(JSON.stringify(alb.artist))}, ${esc(JSON.stringify(alb.coverUrl))})">${ICONS.share} Share</button>
          </div>
        </div>
      </div>`;

      // Group multi-disc albums cleanly
      const discs = [...new Set(tracks.map((t) => t.discNo || 1))].sort((a, b) => a - b);
      html += `<div class="track-table">`;
      if (discs.length > 1) {
        for (const dn of discs) {
          html += `<div class="disc-divider">Disc ${dn}</div>`;
          html += renderTrackRows(tracks.filter((t) => (t.discNo || 1) === dn), tracks, null, false, '', true);
        }
      } else {
        html += renderTrackRows(tracks, null, null, false, '', true);
      }
      html += `</div>`;
      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  async function renderArtist(id) {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Loading artist profile...');
    try {
      let hintName = '';
      const cur = currentTrack();
      if (cur && cur.artistId === id) {
        hintName = cur.artist || '';
      } else {
        for (const t of state.trackMap.values()) {
          if (t.artistId === id) {
            hintName = t.artist || '';
            break;
          }
        }
      }
      const qStr = hintName ? `?name=${encodeURIComponent(hintName)}` : '';
      const d = await api(`/api/web/artist/${encodeURIComponent(id)}${qStr}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const art = d.artist;
      const top = cacheTracks(d.topTracks || []);
      const albums = d.albums || [];
      const related = d.related || [];
      const relatedTracks = cacheTracks(d.relatedTracks || []);
      const idsJson = esc(JSON.stringify(top.map((t) => t.id)));
      state.artistView = { top, expanded: false };

      let html = `<div class="hero-header">
        <img class="hero-art round" src="${esc(art.coverUrl)}" alt="${esc(art.name)}" />
        <div class="hero-info">
          <div class="hero-kicker">ARTIST</div>
          <h1 class="hero-title">${esc(art.name)}</h1>
          <div class="hero-sub">${art.bio ? esc(art.bio) : `${albums.length} albums · ${top.length} popular tracks`}</div>
          <div class="hero-actions">
            ${top.length ? `<button class="btn-primary" onclick="App.playListByIds(${idsJson}, 0)">${ICONS.play} Play Top Tracks</button>` : ''}
            ${top[0] ? `<button class="btn-secondary" onclick="App.startRadioForId(${esc(JSON.stringify(top[0].id))})">Artist Radio</button>` : ''}
            <button class="btn-secondary ${art.isFollowed ? 'active' : ''}" onclick="App.toggleFollowArtist(${esc(JSON.stringify(art.id))}, ${!art.isFollowed})">${art.isFollowed ? '✓ Following' : '+ Follow'}</button>
            <button class="btn-secondary" onclick="App.shareArtist(${esc(JSON.stringify(art.id))}, ${esc(JSON.stringify(art.name))}, ${esc(JSON.stringify(art.coverUrl))})">${ICONS.share} Share</button>
          </div>
        </div>
      </div>`;

      if (top.length > 0) {
        const showMore = top.length > 15
          ? `<button id="artist-tracks-toggle" class="mini-btn" onclick="App.toggleArtistTracks()">Show all (${top.length})</button>`
          : '';
        html += `<section class="section">
          <div class="section-head"><h2>Popular</h2>${showMore}</div>
          <div id="artist-popular-table" class="track-table">${renderTrackRows(top.slice(0, 15), top)}</div>
        </section>`;
      }
      if (albums.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Discography</h2></div>
          <div class="card-grid">${albums.map(renderAlbumCard).join('')}</div>
        </section>`;
      }
      if (relatedTracks.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Related Tracks</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(relatedTracks.map((t) => t.id)))}, 0)">Play All</button></div>
          <div class="track-table">${renderTrackRows(relatedTracks)}</div>
        </section>`;
      }
      if (related.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Fans Also Like</h2></div>
          <div class="card-grid">${related.map(renderArtistCard).join('')}</div>
        </section>`;
      }

      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  function playlistVisibilityHtml(isPublic) {
    return `
      <div class="social-edit-row">
        <label>Visibility</label>
        <div class="pl-visibility-row">
          <button type="button" class="pl-visibility-opt ${isPublic ? '' : 'selected'}" data-vis="private" onclick="App.pickPlaylistVisibility('private')">🔒 Private</button>
          <button type="button" class="pl-visibility-opt ${isPublic ? 'selected' : ''}" data-vis="public" onclick="App.pickPlaylistVisibility('public')">🌐 Public</button>
        </div>
        <span class="pl-visibility-hint">${isPublic
          ? 'Anyone on this instance can find and view this playlist.'
          : 'Only you can see this playlist. You can change this anytime.'}</span>
      </div>`;
  }

  function pickPlaylistVisibility(value) {
    document.querySelectorAll('.pl-visibility-opt').forEach((el) => {
      el.classList.toggle('selected', el.dataset.vis === value);
    });
    const hint = document.querySelector('.pl-visibility-hint');
    if (hint) {
      hint.textContent = value === 'public'
        ? 'Anyone on this instance can find and view this playlist.'
        : 'Only you can see this playlist. You can change this anytime.';
    }
  }

  function selectedPlaylistVisibility() {
    return document.querySelector('.pl-visibility-opt.selected')?.dataset.vis === 'public';
  }

  async function renderPlaylist(id) {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Loading playlist...');
    try {
      const d = await api(`/api/web/playlist/${encodeURIComponent(id)}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const pl = d.playlist;
      const tracks = cacheTracks(d.tracks || []);
      const idsJson = esc(JSON.stringify(tracks.map((t) => t.id)));

      let html = `<div class="hero-header">
        <img class="hero-art" src="${esc(pl.coverUrl)}" alt="${esc(pl.name)}" />
        <div class="hero-info">
          <div class="hero-kicker">${pl.isAuto ? 'MADE FOR YOU' : 'PLAYLIST'}</div>
          <h1 class="hero-title">${esc(pl.name)}</h1>
          <div class="hero-sub">By <strong>${esc(pl.ownerName)}</strong>${pl.comment ? ` · ${esc(pl.comment)}` : ''} · ${tracks.length} songs · ${fmtTotalDur(pl.duration)}
            <span class="pl-visibility-badge" title="${pl.public ? 'Anyone can view this playlist' : 'Only you can view this playlist'}">${pl.public ? '🌐 Public' : '🔒 Private'}</span>
          </div>
          <div class="hero-actions">
            ${tracks.length ? `<button class="btn-primary" onclick="App.playListByIds(${idsJson}, 0)">${ICONS.play} Play</button>` : ''}
            ${tracks.length > 1 ? `<button class="btn-secondary" onclick="App.playShuffledIds(${idsJson})">Shuffle</button>` : ''}
            ${tracks.length ? `<button class="btn-secondary" onclick="App.startPlaylistRadio(${esc(JSON.stringify(pl.id))})">Start Radio</button>` : ''}
            ${tracks.length && pl.public ? `<button class="btn-secondary" onclick="App.sharePlaylist(${esc(JSON.stringify(pl.id))}, ${esc(JSON.stringify(pl.name))}, ${esc(JSON.stringify(pl.ownerName))}, ${esc(JSON.stringify(pl.coverUrl))})">${ICONS.share} Share</button>` : ''}
            ${d.isOwner && !pl.isAuto ? `<button class="btn-secondary" onclick="App.promptEditPlaylist(${esc(JSON.stringify(pl.id))}, ${esc(JSON.stringify(pl.name))}, ${esc(JSON.stringify(pl.comment || ''))}, ${!!pl.public})">Edit</button>` : ''}
            <button class="btn-secondary" id="pl-hero-pin-btn" onclick="App.togglePinPlaylist(${esc(JSON.stringify(pl.id))})">${ICONS.pin} ${isPlaylistPinned(pl.id) ? 'Pinned' : 'Pin to Sidebar'}</button>
            ${d.isOwner && !pl.isAuto ? `<button class="btn-secondary" style="color:#fda4af" onclick="App.deletePlaylist(${esc(JSON.stringify(pl.id))})">Delete</button>` : ''}
          </div>
        </div>
      </div>`;

      html += tracks.length
        ? `<div class="track-table">${renderTrackRows(tracks, tracks, d.isOwner && !pl.isAuto ? pl.id : null)}</div>`
        : `<p class="muted-center">This playlist is empty. Click "..." on any song or pick from recommendations below to start curating.</p>`;

      if (d.isOwner && !pl.isAuto) {
        html += `<section class="section pl-recs-section" id="pl-recs-section">
          <div class="section-head">
            <div>
              <h2>Recommended for this playlist</h2>
              <p style="font-size:12.5px;color:var(--text-muted);margin-top:2px">Matched to this playlist's sound and your listening taste</p>
            </div>
            <button class="btn-secondary" onclick="App.loadPlaylistRecs(${esc(JSON.stringify(pl.id))})">Refresh</button>
          </div>
          <div id="pl-recs-body"><p style="color:var(--text-muted);font-size:13px;padding:8px 0">Finding matching tracks...</p></div>
        </section>`;
      }

      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;

      if (d.isOwner && !pl.isAuto) {
        loadPlaylistRecs(pl.id);
      }
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  async function loadPlaylistRecs(pid) {
    const box = $('pl-recs-body');
    if (!box) return;
    const signal = state.viewAbortController ? state.viewAbortController.signal : undefined;
    box.innerHTML = `<p style="color:var(--text-muted);font-size:13px;padding:8px 0">Finding matching tracks...</p>`;
    try {
      const d = await api(`/api/web/playlist/${encodeURIComponent(pid)}/recommendations`, { signal });
      const targetBox = $('pl-recs-body');
      const curPlHash = location.hash.startsWith('#playlist/')
        ? safeDecodeUri(location.hash.slice('#playlist/'.length))
        : '';
      if (!targetBox || curPlHash !== pid || (signal && signal.aborted)) return;
      const recs = cacheTracks(d.tracks || []);
      if (!recs.length) {
        targetBox.innerHTML = `<p style="color:var(--text-muted);font-size:13px;padding:8px 0">No extra matches found yet. Try searching or playing a few songs first.</p>`;
        return;
      }
      const listJson = esc(JSON.stringify(recs.map((t) => t.id)));
      targetBox.innerHTML = `<div class="track-table">${recs
        .map((t, idx) => {
          const metaSub = t.genre ? `${t.artist} · ${t.genre}` : t.artist;
          return `
            <div class="track-row" data-tid="${esc(t.id)}">
              <div class="tr-idx" onclick="App.playListByIds(${listJson}, ${idx})">
                <span class="tr-num">${idx + 1}</span>
                <span class="tr-play-icon">${ICONS.play}</span>
              </div>
              <div class="tr-main" onclick="App.playListByIds(${listJson}, ${idx})">
                <img class="tr-art" src="${esc(t.coverUrl)}" loading="lazy" alt="" />
                <div class="tr-meta">
                  <div class="tr-title">${esc(t.title)}</div>
                  <div class="tr-sub">${esc(metaSub)}</div>
                </div>
              </div>
              <div class="tr-album clickable" onclick="App.navigate('album/${encodeURIComponent(t.albumId)}')">${esc(t.album)}</div>
              <div class="tr-actions">
                <span class="tr-dur">${fmtTime(t.duration)}</span>
                <button class="rec-add-btn" onclick="event.stopPropagation();App.quickAddToPlaylist(${esc(JSON.stringify(pid))}, ${esc(JSON.stringify(t.id))}, this)">+ Add</button>
              </div>
            </div>`;
        })
        .join('')}</div>`;
    } catch (e) {
      if ((e && e.name === 'AbortError') || (signal && signal.aborted)) return;
      const targetBox = $('pl-recs-body');
      if (targetBox) {
        targetBox.innerHTML = `<p style="color:var(--text-muted);font-size:13px;padding:8px 0">Couldn't load recommendations right now.</p>`;
      }
    }
  }

  async function renderMood(slug) {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Building mood mix...');
    try {
      const d = await api(`/api/web/mood/${encodeURIComponent(slug)}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const m = d.mood;
      const tracks = cacheTracks(d.tracks || []);
      state.currentMoodShare = {
        kind: 'mix',
        targetId: `mood:${m.slug}`,
        title: `${m.label} Mix`,
        subtitle: m.subtitle || 'Mood Mix',
        trackIds: tracks.map((t) => t.id),
        coverUrl: tracks[0]?.coverUrl || '/api/web/cover/default',
      };
      const idsJson = esc(JSON.stringify(tracks.map((t) => t.id)));
      const covers = [...new Set(tracks.map((t) => t.coverUrl).filter(Boolean))].slice(0, 4);
      const artHtml = covers.length >= 4
        ? `<div class="hero-collage">${covers.map((c) => `<img src="${esc(c)}" alt="" />`).join('')}</div>`
        : covers.length >= 1
          ? `<img class="hero-art" src="${esc(covers[0])}" alt="" />`
          : '';
      let html = `<div class="hero-header" style="background:${esc(m.c2)};border-color:${esc(m.c1)}55;border-left:4px solid ${esc(m.c1)};">
        ${artHtml}
        <div class="hero-info">
          <div class="hero-kicker" style="color:#fff">MOOD MIX</div>
          <h1 class="hero-title">${esc(m.label)}</h1>
          <div class="hero-sub" style="color:rgba(255,255,255,0.88)">${esc(m.subtitle)} · ${tracks.length} track${tracks.length === 1 ? '' : 's'}</div>
          <div class="hero-actions">
            ${tracks.length ? `<button class="btn-primary" onclick="App.playListByIds(${idsJson}, 0)">${ICONS.play} Play Mix</button>` : `<button class="btn-primary" onclick="App.navigate('mood/${encodeURIComponent(m.slug)}')">Refresh Mix</button>`}
            ${tracks.length ? `<button class="btn-secondary" onclick="App.playShuffledIds(${idsJson})">Shuffle</button>` : ''}
            ${tracks.length ? `<button class="btn-secondary" onclick="App.shareCurrentMoodMix()">${ICONS.share} Share Mix</button>` : ''}
          </div>
        </div>
      </div>
      ${tracks.length ? `<div class="track-table">${renderTrackRows(tracks, tracks, null, false, 'explore')}</div>` : `<p class="muted-center">Still curating tracks for ${esc(m.label)} — try refreshing in a moment.</p>`}`;
      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  async function renderGenre(name) {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading(`Loading ${name}...`);
    try {
      const d = await api(`/api/web/genre/${encodeURIComponent(name)}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const tracks = cacheTracks(d.tracks || []);
      const artists = d.artists || [];
      state.currentGenreShare = {
        kind: 'mix',
        targetId: `genre:${d.genre}`,
        title: `${d.genre} Mix`,
        subtitle: `Genre Mix · ${tracks.length} tracks`,
        trackIds: tracks.map((t) => t.id),
        coverUrl: tracks[0]?.coverUrl || '/api/web/cover/default',
      };
      const idsJson = esc(JSON.stringify(tracks.map((t) => t.id)));
      let html = `<div class="hero-header">
        <div class="hero-info">
          <div class="hero-kicker">GENRE</div>
          <h1 class="hero-title">${esc(d.genre)}</h1>
          <div class="hero-sub">${tracks.length} tracks ready to stream</div>
          <div class="hero-actions">
            ${tracks.length ? `<button class="btn-primary" onclick="App.playListByIds(${idsJson}, 0)">${ICONS.play} Play ${esc(d.genre)}</button>` : ''}
            ${tracks.length ? `<button class="btn-secondary" onclick="App.shareCurrentGenreMix()">${ICONS.share} Share Mix</button>` : ''}
          </div>
        </div>
      </div>`;
      if (artists.length) {
        html += `<section class="section">
          <div class="section-head"><h2>Featured Artists</h2></div>
          <div class="card-grid">${artists.map(renderArtistCard).join('')}</div>
        </section>`;
      }
      html += `<section class="section">
        <div class="section-head"><h2>Tracks</h2></div>
        <div class="track-table">${renderTrackRows(tracks)}</div>
      </section>`;
      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  async function renderPlaylistsView() {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Loading playlists...');
    try {
      const lib = await api('/api/web/library', { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      state.playlists = lib.playlists || [];
      renderSidebarPlaylistsDom();
      const pinnedSet = new Set(getPinnedPlaylistIds());
      const pinned = state.playlists.filter((p) => pinnedSet.has(p.id));
      const allHtml = state.playlists.length
        ? `<div class="card-grid">${state.playlists.map(renderPlaylistCard).join('')}</div>`
        : `<p class="muted-center">No playlists yet. Create one or save an Instant Mix!</p>`;
      const pinnedHtml = pinned.length
        ? `<section class="section"><div class="section-head"><h2>Pinned to Sidebar (${pinned.length})</h2></div><div class="card-grid">${pinned.map(renderPlaylistCard).join('')}</div></section>`
        : '';
      $('main-view').innerHTML = `
        <div class="section-head">
          <div>
            <h1>Playlists</h1>
            <p style="font-size:13px;color:var(--text-muted);margin-top:4px">Pin your favorites to keep them handy in the sidebar</p>
          </div>
          <div style="display:flex;gap:8px;flex-wrap:wrap">
            <button class="btn-secondary" onclick="App.promptInstantMix()">Instant Mix</button>
            <button class="btn-primary" onclick="App.promptCreatePlaylist()">+ New Playlist</button>
          </div>
        </div>
        ${pinnedHtml}
        <section class="section">
          <div class="section-head"><h2>All Playlists (${state.playlists.length})</h2></div>
          ${allHtml}
        </section>`;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  async function renderLibrary(tab = 'playlists') {
    const { reqToken, signal } = beginViewRequest();
    state.libTab = tab;
    setMainLoading('Loading your library...');
    try {
      const lib = await api('/api/web/library', { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      state.playlists = lib.playlists || [];
      const liked = cacheTracks(lib.likedTracks || []);
      const recent = cacheTracks(lib.recentTracks || []);
      const frequent = cacheTracks(lib.frequentTracks || []);
      const albums = lib.savedAlbums || [];
      const artists = lib.followedArtists || [];

      const tabs = [
        ['playlists', `Playlists (${state.playlists.length})`],
        ['liked', `Liked Songs (${liked.length})`],
        ['albums', `Albums (${albums.length})`],
        ['artists', `Artists (${artists.length})`],
        ['history', `History (${recent.length})`],
        ['frequent', `Most Played`],
      ];

      let html = `<div class="section-head"><h1>Your Library</h1><div style="display:flex;gap:8px;flex-wrap:wrap"><button class="btn-secondary" onclick="App.navigate('stats')">Listening Stats</button><button class="btn-primary" onclick="App.promptCreatePlaylist()">+ New Playlist</button></div></div>`;
      html += `<div class="tabs-bar">${tabs
        .map(
          ([k, lbl]) =>
            `<button class="tab-chip ${tab === k ? 'active' : ''}" onclick="App.setLibTab('${k}')">${esc(lbl)}</button>`
        )
        .join('')}</div>`;

      if (tab === 'playlists') {
        html += `<div class="card-grid">${state.playlists.map(renderPlaylistCard).join('')}</div>`;
      } else if (tab === 'liked') {
        html += liked.length
          ? `<div style="margin-bottom:14px"><button class="btn-primary" onclick="App.playListByIds(${esc(JSON.stringify(liked.map((t) => t.id)))}, 0)">${ICONS.play} Play Liked Songs</button></div><div class="track-table">${renderTrackRows(liked)}</div>`
          : `<p class="muted-center">No liked songs yet. Tap the heart icon on any track!</p>`;
      } else if (tab === 'albums') {
        html += albums.length
          ? `<div class="card-grid">${albums.map(renderAlbumCard).join('')}</div>`
          : `<p class="muted-center">No saved albums yet.</p>`;
      } else if (tab === 'artists') {
        html += artists.length
          ? `<div class="card-grid">${artists.map(renderArtistCard).join('')}</div>`
          : `<p class="muted-center">Not following any artists yet.</p>`;
      } else if (tab === 'history') {
        html += recent.length
          ? `<div class="track-table">${renderTrackRows(recent, null, null, true)}</div>`
          : `<p class="muted-center">Your listening history will appear here.</p>`;
      } else if (tab === 'frequent') {
        html += frequent.length
          ? `<div class="track-table">${renderTrackRows(frequent, null, null, true)}</div>`
          : `<p class="muted-center">Play tracks to see your most-spun favorites.</p>`;
      }

      $('main-view').innerHTML = html;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  // ---------------------------------------------------------------------------
  // Detailed Listening Stats & Graphs
  // ---------------------------------------------------------------------------
  function fmtStatsDuration(sec) {
    if (!Number.isFinite(sec) || sec <= 0) return '0m';
    const s = Math.floor(sec);
    if (s < 60) return `${s}s`;
    const mins = Math.floor(s / 60);
    const hrs = Math.floor(mins / 60);
    const remM = mins % 60;
    if (hrs >= 100) {
      const days = Math.floor(hrs / 24);
      const remH = hrs % 24;
      return `${days}d ${remH}h`;
    }
    return hrs > 0 ? `${hrs}h ${remM}m` : `${mins}m`;
  }

  function fmtDateLabel(ts) {
    if (!ts) return '—';
    try {
      return new Date(ts * 1000).toLocaleDateString(undefined, {
        month: 'short',
        day: 'numeric',
        year: 'numeric',
      });
    } catch {
      return '—';
    }
  }

  function fmtShortDayIso(iso) {
    if (!iso) return '';
    try {
      const [y, m, d] = iso.split('-').map(Number);
      const dt = new Date(Date.UTC(y, (m || 1) - 1, d || 1));
      return dt.toLocaleDateString(undefined, {
        timeZone: 'UTC',
        month: 'short',
        day: 'numeric',
        year: 'numeric',
      });
    } catch {
      return iso;
    }
  }

  function fmtRelativeTime(ts) {
    if (!ts) return '';
    const diff = Math.max(0, Math.floor(Date.now() / 1000) - ts);
    if (diff < 60) return 'just now';
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    if (diff < 7 * 86400) return `${Math.floor(diff / 86400)}d ago`;
    return fmtDateLabel(ts);
  }

  function fmtHourLongJs(h) {
    if (h === 0) return '12 AM';
    if (h < 12) return `${h} AM`;
    if (h === 12) return '12 PM';
    return `${h - 12} PM`;
  }

  function hideStatsTooltip() {
    const tip = $('stats-tooltip');
    if (tip) tip.classList.add('hidden');
  }

  function showStatsTooltip(target, clientX, clientY) {
    const tip = $('stats-tooltip');
    if (!tip || !target) return;
    const title = target.getAttribute('data-tip-title') || '';
    const body = target.getAttribute('data-tip-body') || '';
    if (!title && !body) {
      hideStatsTooltip();
      return;
    }
    tip.innerHTML = `${title ? `<div class="stats-tip-title">${esc(title)}</div>` : ''}${body ? `<div class="stats-tip-body">${esc(body)}</div>` : ''}`;
    tip.classList.remove('hidden');

    const rect = tip.getBoundingClientRect();
    let x = clientX;
    let y = clientY;
    if (!Number.isFinite(x) || !Number.isFinite(y) || (x === 0 && y === 0)) {
      const tr = target.getBoundingClientRect();
      x = tr.left + tr.width / 2;
      y = tr.top;
    }
    const pad = 10;
    const left = Math.max(pad, Math.min(window.innerWidth - rect.width - pad, x - rect.width / 2));
    let top = y - rect.height - 14;
    if (top < pad) top = y + 18;
    top = Math.max(pad, Math.min(window.innerHeight - rect.height - pad, top));
    tip.style.left = `${Math.round(left)}px`;
    tip.style.top = `${Math.round(top)}px`;
  }

  function wireStatsTooltips() {
    const mv = $('main-view');
    if (!mv) return;
    mv.addEventListener('pointermove', (e) => {
      const t = e.target && e.target.closest ? e.target.closest('.stats-tip-target') : null;
      if (t) {
        showStatsTooltip(t, e.clientX, e.clientY);
      } else {
        hideStatsTooltip();
      }
    });
    mv.addEventListener('pointerleave', hideStatsTooltip);
    mv.addEventListener('scroll', hideStatsTooltip, { passive: true });
    mv.addEventListener('click', (e) => {
      const t = e.target && e.target.closest ? e.target.closest('.stats-tip-target') : null;
      if (t && window.matchMedia && window.matchMedia('(pointer: coarse)').matches) {
        showStatsTooltip(t, e.clientX, e.clientY);
      }
    });
  }

  function svgDonutSlicePath(cx, cy, rOuter, rInner, startDeg, endDeg) {
    const rad = Math.PI / 180;
    const s = startDeg * rad;
    const e = endDeg * rad;
    const large = endDeg - startDeg > 180 ? 1 : 0;
    const x1 = cx + rOuter * Math.cos(s);
    const y1 = cy + rOuter * Math.sin(s);
    const x2 = cx + rOuter * Math.cos(e);
    const y2 = cy + rOuter * Math.sin(e);
    const x3 = cx + rInner * Math.cos(e);
    const y3 = cy + rInner * Math.sin(e);
    const x4 = cx + rInner * Math.cos(s);
    const y4 = cy + rInner * Math.sin(s);
    return `M ${x1.toFixed(2)} ${y1.toFixed(2)} A ${rOuter} ${rOuter} 0 ${large} 1 ${x2.toFixed(2)} ${y2.toFixed(2)} L ${x3.toFixed(2)} ${y3.toFixed(2)} A ${rInner} ${rInner} 0 ${large} 0 ${x4.toFixed(2)} ${y4.toFixed(2)} Z`;
  }

  function renderStatsTimelineSvg(timeline, metric) {
    const points = (timeline && timeline.points) || [];
    if (!points.length) {
      return '<p class="muted-center">No timeline points yet.</p>';
    }
    const vals = points.map((p) => (metric === 'mins' ? Math.round((p.secs || 0) / 60) : (p.streams || 0)));
    const maxVal = Math.max(1, ...vals);
    const W = 760;
    const H = 228;
    const padL = 42;
    const padR = 14;
    const padT = 18;
    const padB = 30;
    const plotW = W - padL - padR;
    const plotH = H - padT - padB;
    const baseY = padT + plotH;
    const n = points.length;
    const slotW = plotW / n;
    const barW = Math.max(3, Math.min(28, slotW * 0.54));

    const coords = points.map((p, i) => {
      const x = padL + (i + 0.5) * slotW;
      const y = baseY - (vals[i] / maxVal) * plotH;
      return { x, y, v: vals[i], p };
    });

    // Horizontal grid lines & Y axis labels
    const ticks = [0, 0.5, 1];
    const gridSvg = ticks
      .map((t) => {
        const y = baseY - t * plotH;
        const val = Math.round(t * maxVal);
        return `<line x1="${padL}" y1="${y.toFixed(1)}" x2="${W - padR}" y2="${y.toFixed(1)}" stroke="#232834" stroke-dasharray="${t === 0 ? '0' : '3 4'}" stroke-width="1" />
          <text x="${padL - 8}" y="${(y + 4).toFixed(1)}" text-anchor="end" fill="#6b7485" font-size="10.5">${val}${metric === 'mins' ? 'm' : ''}</text>`;
      })
      .join('');

    // Smooth curve & gradient area
    let linePath = '';
    let areaPath = '';
    if (coords.length === 1) {
      linePath = `M ${coords[0].x.toFixed(1)} ${coords[0].y.toFixed(1)}`;
    } else {
      linePath = coords
        .map((c, i) => {
          if (i === 0) return `M ${c.x.toFixed(1)} ${c.y.toFixed(1)}`;
          const prev = coords[i - 1];
          const midX = ((prev.x + c.x) / 2).toFixed(1);
          return `C ${midX} ${prev.y.toFixed(1)}, ${midX} ${c.y.toFixed(1)}, ${c.x.toFixed(1)} ${c.y.toFixed(1)}`;
        })
        .join(' ');
      areaPath = `${linePath} L ${coords[coords.length - 1].x.toFixed(1)} ${baseY} L ${coords[0].x.toFixed(1)} ${baseY} Z`;
    }

    const peakVal = Math.max(...vals);
    const labelStep = Math.max(1, Math.ceil(n / 8));

    const barsAndHits = coords
      .map((c, i) => {
        const barH = Math.max(c.v > 0 ? 3 : 0, baseY - c.y);
        const bx = c.x - barW / 2;
        const isPeak = peakVal > 0 && c.v === peakVal;
        const tipTitle = c.p.label || c.p.key;
        const tipBody = `${c.p.streams} stream${c.p.streams === 1 ? '' : 's'} · ${fmtStatsDuration(c.p.secs)} (${c.p.completed} completed, ${c.p.skipped} skipped)`;
        const xLabel =
          i % labelStep === 0 || i === n - 1
            ? `<text x="${c.x.toFixed(1)}" y="${H - 8}" text-anchor="middle" fill="#6b7485" font-size="10.5">${esc(c.p.shortLabel)}</text>`
            : '';
        const peakDot = isPeak
          ? `<circle cx="${c.x.toFixed(1)}" cy="${c.y.toFixed(1)}" r="4.2" fill="#1ed760" stroke="#0c0e13" stroke-width="2" />`
          : '';
        return `
          <rect x="${bx.toFixed(1)}" y="${c.y.toFixed(1)}" width="${barW.toFixed(1)}" height="${barH.toFixed(1)}" rx="2" fill="${isPeak ? '#1ed760' : 'rgba(56, 189, 248, 0.38)'}" />
          ${peakDot}
          ${xLabel}
          <rect class="stats-hit-col stats-tip-target" x="${(padL + i * slotW).toFixed(1)}" y="${padT}" width="${slotW.toFixed(1)}" height="${plotH}" fill="transparent" data-tip-title="${esc(tipTitle)}" data-tip-body="${esc(tipBody)}" />
        `;
      })
      .join('');

    return `<svg class="stats-svg-chart" viewBox="0 0 ${W} ${H}" preserveAspectRatio="xMidYMid meet">
      <defs>
        <linearGradient id="statsTimelineGrad" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stop-color="#1ed760" stop-opacity="0.28" />
          <stop offset="100%" stop-color="#1ed760" stop-opacity="0.01" />
        </linearGradient>
      </defs>
      ${gridSvg}
      ${areaPath ? `<path d="${areaPath}" fill="url(#statsTimelineGrad)" />` : ''}
      ${linePath ? `<path d="${linePath}" fill="none" stroke="#1ed760" stroke-width="2.2" stroke-linecap="round" />` : ''}
      ${barsAndHits}
    </svg>`;
  }

  function renderStatsClockSvg(hourly, metric, peakHour) {
    const hrs = hourly || [];
    const vals = hrs.map((h) => (metric === 'mins' ? Math.round((h.secs || 0) / 60) : (h.streams || 0)));
    const maxVal = Math.max(1, ...vals);
    const W = 280;
    const H = 280;
    const cx = 140;
    const cy = 140;
    const rInner = 46;
    const rMax = 112;

    const wedges = hrs
      .map((h, i) => {
        const startDeg = i * 15 - 90 + 1.1;
        const endDeg = (i + 1) * 15 - 90 - 1.1;
        const v = vals[i] || 0;
        const ratio = v / maxVal;
        const rOuter = v > 0 ? rInner + 10 + ratio * (rMax - rInner - 10) : rInner + 4;
        let fill = '#232834';
        let opacity = 0.85;
        if (v > 0) {
          if (i === peakHour) {
            fill = '#1ed760';
            opacity = 1;
          } else if (i >= 22 || i <= 4) {
            fill = '#38bdf8';
            opacity = 0.35 + 0.65 * ratio;
          } else if (i >= 17 && i <= 21) {
            fill = '#a855f7';
            opacity = 0.35 + 0.65 * ratio;
          } else {
            fill = '#1ed760';
            opacity = 0.3 + 0.6 * ratio;
          }
        }
        const tipTitle = `${fmtHourLongJs(i)} – ${fmtHourLongJs((i + 1) % 24)}`;
        const tipBody = `${h.streams} stream${h.streams === 1 ? '' : 's'} (${h.pct}%) · ${fmtStatsDuration(h.secs)}`;
        return `
          <path d="${svgDonutSlicePath(cx, cy, rOuter, rInner, startDeg, endDeg)}" fill="${fill}" fill-opacity="${opacity.toFixed(2)}" />
          <path class="stats-clock-hit stats-tip-target" d="${svgDonutSlicePath(cx, cy, rMax + 4, rInner, startDeg, endDeg)}" fill="transparent" data-tip-title="${esc(tipTitle)}" data-tip-body="${esc(tipBody)}" />
        `;
      })
      .join('');

    const peakLabel = peakHour != null ? fmtHourLongJs(peakHour) : '—';
    const peakPct = peakHour != null && hrs[peakHour] ? `${hrs[peakHour].pct}% of plays` : 'No streams';

    return `<svg class="stats-clock-svg" viewBox="0 0 ${W} ${H}">
      <circle cx="${cx}" cy="${cy}" r="${rMax}" fill="none" stroke="#232834" stroke-dasharray="3 4" />
      <circle cx="${cx}" cy="${cy}" r="${((rInner + rMax) / 2).toFixed(1)}" fill="none" stroke="#1e2330" stroke-dasharray="2 4" />
      <text x="${cx}" y="16" text-anchor="middle" fill="#98a2b3" font-size="11" font-weight="700">12 AM</text>
      <text x="${W - 8}" y="${cy + 4}" text-anchor="end" fill="#98a2b3" font-size="11" font-weight="700">6 AM</text>
      <text x="${cx}" y="${H - 8}" text-anchor="middle" fill="#98a2b3" font-size="11" font-weight="700">12 PM</text>
      <text x="8" y="${cy + 4}" text-anchor="start" fill="#98a2b3" font-size="11" font-weight="700">6 PM</text>
      ${wedges}
      <circle cx="${cx}" cy="${cy}" r="${rInner - 4}" fill="#12151c" stroke="#232834" />
      <text x="${cx}" y="${cy - 3}" text-anchor="middle" fill="#f3f5f8" font-size="15" font-weight="800">${esc(peakLabel)}</text>
      <text x="${cx}" y="${cy + 14}" text-anchor="middle" fill="#98a2b3" font-size="10">${esc(peakPct)}</text>
    </svg>`;
  }

  function renderStatsHeatmap(heatmap) {
    if (!heatmap || !heatmap.startDate || !heatmap.endDate) return '';
    const dayMap = new Map();
    for (const d of heatmap.days || []) {
      dayMap.set(d.date, d);
    }
    const maxStreams = Math.max(1, heatmap.maxStreams || 0);
    const [sy, sm, sd] = heatmap.startDate.split('-').map(Number);
    const [ey, em, ed] = heatmap.endDate.split('-').map(Number);
    const startUtc = new Date(Date.UTC(sy, sm - 1, sd));
    const endUtc = new Date(Date.UTC(ey, em - 1, ed));

    // Align grid start back to Monday (0=Mon..6=Sun)
    const startDow = (startUtc.getUTCDay() + 6) % 7;
    const gridStart = new Date(startUtc.getTime() - startDow * 86400000);

    const weeks = [];
    let cur = new Date(gridStart.getTime());
    while (cur <= endUtc) {
      const col = [];
      for (let r = 0; r < 7; r++) {
        const d = new Date(cur.getTime() + r * 86400000);
        const iso = d.toISOString().slice(0, 10);
        const inRange = d >= startUtc && d <= endUtc;
        const info = inRange ? dayMap.get(iso) : null;
        col.push({ iso, date: d, inRange, streams: info ? info.streams : 0, secs: info ? info.secs : 0 });
      }
      weeks.push(col);
      cur = new Date(cur.getTime() + 7 * 86400000);
    }

    const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
    let lastMonth = -1;
    const monthLabels = weeks
      .map((col) => {
        const firstDay = col.find((c) => c.inRange && c.date.getUTCDate() <= 7) || col.find((c) => c.inRange);
        let label = '';
        if (firstDay) {
          const m = firstDay.date.getUTCMonth();
          if (m !== lastMonth && (firstDay.date.getUTCDate() <= 7 || lastMonth === -1)) {
            label = months[m];
            lastMonth = m;
          }
        }
        return `<div class="stats-hm-month">${label}</div>`;
      })
      .join('');

    const colsHtml = weeks
      .map((col) => {
        const cells = col
          .map((c) => {
            if (!c.inRange) {
              return '<div class="stats-hm-cell empty"></div>';
            }
            let lvl = 0;
            if (c.streams > 0) {
              const ratio = c.streams / maxStreams;
              lvl = ratio <= 0.25 ? 1 : ratio <= 0.5 ? 2 : ratio <= 0.75 ? 3 : 4;
            }
            const tipTitle = fmtShortDayIso(c.iso);
            const tipBody = c.streams > 0
              ? `${c.streams} stream${c.streams === 1 ? '' : 's'} · ${fmtStatsDuration(c.secs)}`
              : 'No streams logged';
            return `<div class="stats-hm-cell lv-${lvl} stats-tip-target" data-tip-title="${esc(tipTitle)}" data-tip-body="${esc(tipBody)}"></div>`;
          })
          .join('');
        return `<div class="stats-hm-col">${cells}</div>`;
      })
      .join('');

    return `
      <div class="stats-hm-scroll" id="stats-hm-scroll">
        <div class="stats-hm-inner">
          <div class="stats-hm-months-row">
            <div class="stats-hm-wd-spacer"></div>
            <div class="stats-hm-months">${monthLabels}</div>
          </div>
          <div class="stats-hm-body">
            <div class="stats-hm-weekdays">
              <span>Mon</span><span></span><span>Wed</span><span></span><span>Fri</span><span></span><span></span>
            </div>
            <div class="stats-hm-grid">${colsHtml}</div>
          </div>
        </div>
      </div>
      <div class="stats-hm-legend">
        <span>Less</span>
        <span class="stats-hm-cell lv-0"></span>
        <span class="stats-hm-cell lv-1"></span>
        <span class="stats-hm-cell lv-2"></span>
        <span class="stats-hm-cell lv-3"></span>
        <span class="stats-hm-cell lv-4"></span>
        <span>More</span>
      </div>
    `;
  }

  function renderStatsMoodRadarSvg(moods) {
    const list = moods || [];
    if (!list.length) return '';
    const W = 340;
    const H = 286;
    const cx = 170;
    const cy = 143;
    const rMax = 88;
    const n = list.length;
    const maxStreams = Math.max(1, ...list.map((m) => m.streams || 0));
    const hasAny = list.some((m) => m.streams > 0);

    const pointAt = (idx, radius) => {
      const ang = -Math.PI / 2 + (idx * 2 * Math.PI) / n;
      return [cx + radius * Math.cos(ang), cy + radius * Math.sin(ang), ang];
    };

    const rings = [0.25, 0.5, 0.75, 1]
      .map((frac) => {
        const pts = list.map((_, i) => pointAt(i, rMax * frac).slice(0, 2).map((v) => v.toFixed(1)).join(',')).join(' ');
        return `<polygon points="${pts}" fill="none" stroke="#232834" stroke-width="1" />`;
      })
      .join('');

    const spokes = list
      .map((_, i) => {
        const [x, y] = pointAt(i, rMax);
        return `<line x1="${cx}" y1="${cy}" x2="${x.toFixed(1)}" y2="${y.toFixed(1)}" stroke="#1e2330" stroke-width="1" />`;
      })
      .join('');

    const dataPts = list.map((m, i) => {
      const ratio = hasAny && m.streams > 0 ? Math.max(0.14, m.streams / maxStreams) : 0.05;
      return pointAt(i, rMax * ratio);
    });
    const polyStr = dataPts.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(' ');

    const labelsAndDots = list
      .map((m, i) => {
        const [dx, dy] = dataPts[i];
        const [lx, ly, ang] = pointAt(i, rMax + 18);
        const cos = Math.cos(ang);
        const anchor = Math.abs(cos) < 0.25 ? 'middle' : cos > 0 ? 'start' : 'end';
        const shortLbl = m.label.replace(' & Lo-Fi', '').replace(' & Dance', '').replace(' & Indie', '');
        const tipBody = `${m.streams} stream${m.streams === 1 ? '' : 's'} (${m.pct}%) · ${fmtStatsDuration(m.secs)}`;
        return `
          <circle class="stats-tip-target" cx="${dx.toFixed(1)}" cy="${dy.toFixed(1)}" r="${m.streams > 0 ? 4.2 : 2.5}" fill="${m.streams > 0 ? esc(m.c1) : '#475569'}" stroke="#0c0e13" stroke-width="1.5" data-tip-title="${esc(m.label)}" data-tip-body="${esc(tipBody)}" />
          <text class="stats-radar-lbl" x="${lx.toFixed(1)}" y="${(ly + 3.5).toFixed(1)}" text-anchor="${anchor}" fill="${m.streams > 0 ? '#f3f5f8' : '#6b7485'}" font-size="10.5" font-weight="600" onclick="App.navigate('mood/${encodeURIComponent(m.slug)}')">${esc(shortLbl)}</text>
        `;
      })
      .join('');

    return `<svg class="stats-radar-svg" viewBox="0 0 ${W} ${H}">
      ${rings}
      ${spokes}
      ${hasAny ? `<polygon points="${polyStr}" fill="rgba(30, 215, 96, 0.22)" stroke="#1ed760" stroke-width="2.1" stroke-linejoin="round" />` : ''}
      ${labelsAndDots}
    </svg>`;
  }

  async function renderStats(range = 'all') {
    const { reqToken, signal } = beginViewRequest();
    const cleanRange = ['7d', '30d', '90d', '365d', 'all'].includes(range) ? range : 'all';
    state.statsRange = cleanRange;
    hideStatsTooltip();
    setMainLoading('Crunching your listening stats...');
    try {
      const tzOffsetMin = -new Date().getTimezoneOffset();
      const d = await api(`/api/web/stats?range=${encodeURIComponent(cleanRange)}&tz=${encodeURIComponent(tzOffsetMin)}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      state.statsData = d;
      cacheTracks((d.topTracks || []).map((x) => x.track));
      cacheTracks((d.mostSkipped || []).map((x) => x.track));
      cacheTracks((d.recentEvents || []).map((x) => x.track));
      if (d.highlights) {
        for (const k of ['longestTrack', 'shortestTrack', 'firstTrackEver', 'mostCompletedTrack']) {
          if (d.highlights[k]?.track) cacheTracks([d.highlights[k].track]);
        }
      }
      renderStatsView();
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  function renderStatsView() {
    const d = state.statsData;
    if (!d) return;
    hideStatsTooltip();

    const s = d.summary || {};
    const streaks = d.streaks || {};
    const lib = d.library || {};
    const persona = d.persona || {};
    const metric = state.statsMetric === 'mins' ? 'mins' : 'streams';
    const ranges = [
      ['7d', '7 Days'],
      ['30d', '30 Days'],
      ['90d', '90 Days'],
      ['365d', '1 Year'],
      ['all', 'All Time'],
    ];
    let html = `<div class="stats-page">`;

    // Hero Banner with Listener Persona + Controls
    html += `
      <section class="stats-hero">
        <div class="stats-hero-left">
          <div class="stats-hero-identity">
            <span class="avatar-dot large">${avatarInner(selfAvatarUser())}</span>
            <div>
              <div class="hero-kicker">LISTENING ANALYTICS · ${esc(d.rangeLabel || 'All Time')}</div>
              <h1 class="stats-hero-title">${esc(state.user?.username || 'Your')}’s Listening Stats</h1>
              <div class="stats-hero-dates">
                Member since ${esc(fmtDateLabel(d.memberSince))}
                ${d.firstListenAt ? ` · First stream ${esc(fmtDateLabel(d.firstListenAt))}` : ''}
              </div>
            </div>
          </div>
          <div class="stats-persona-box">
            <div class="stats-persona-row">
              <span class="stats-persona-pill">${esc(persona.title || 'Music Listener')}</span>
              ${(persona.badges || []).map((b) => `<span class="stats-vibe-chip">${esc(b)}</span>`).join('')}
            </div>
            <p class="stats-persona-headline">${esc(persona.headline || '')}</p>
          </div>
        </div>
        <div class="stats-hero-controls">
          <div class="stats-range-pills" role="group" aria-label="Timeframe">
            ${ranges
              .map(
                ([k, lbl]) =>
                  `<button class="tab-chip ${d.range === k ? 'active' : ''}" onclick="App.setStatsRange('${k}')">${esc(lbl)}</button>`
              )
              .join('')}
          </div>
          <div class="stats-ctrl-row">
            <div class="stats-seg-toggle" role="group" aria-label="Chart Metric">
              <button class="stats-seg-btn ${metric === 'streams' ? 'active' : ''}" onclick="App.setStatsMetric('streams')">Streams</button>
              <button class="stats-seg-btn ${metric === 'mins' ? 'active' : ''}" onclick="App.setStatsMetric('mins')">Minutes</button>
            </div>
            <button class="btn-secondary" onclick="App.refreshStats()" title="Refresh stats">Refresh</button>
          </div>
        </div>
      </section>
    `;

    // 8-Card KPI Grid (renders cleanly even at 0 streams)
    const activeTodayDot = streaks.streakActiveToday
      ? '<span class="stats-live-dot" title="Active today"></span>'
      : '';
    html += `
      <section class="stats-kpi-grid">
        <div class="stats-kpi-card accent-green">
          <div class="stats-kpi-label">Listening Time</div>
          <div class="stats-kpi-val">${esc(fmtStatsDuration(s.totalListeningSecs || 0))}</div>
          <div class="stats-kpi-sub">${s.activeDays ? `${fmtStatsDuration(s.avgDailySecs)}/active day · ${fmtTime(s.avgTrackDurationSecs)} avg track` : 'Start spinning tracks'}</div>
        </div>
        <div class="stats-kpi-card">
          <div class="stats-kpi-label">Total Streams</div>
          <div class="stats-kpi-val">${s.totalStreams || 0}</div>
          <div class="stats-kpi-sub">${s.completedStreams || 0} completed (${s.completionRate || 0}%)</div>
          <div class="stats-kpi-bar"><span style="width:${Math.min(100, s.completionRate || 0)}%"></span></div>
        </div>
        <div class="stats-kpi-card accent-blue">
          <div class="stats-kpi-label">Unique Songs</div>
          <div class="stats-kpi-val">${s.uniqueTracks || 0}</div>
          <div class="stats-kpi-sub">${s.repeatRatio || 0}× avg replay · ${s.newTracksDiscovered || 0} new (${s.discoveryRatePct || 0}%)</div>
        </div>
        <div class="stats-kpi-card">
          <div class="stats-kpi-label">Artists &amp; Albums</div>
          <div class="stats-kpi-val">${s.uniqueArtists || 0} <span class="stats-kpi-unit">artists</span></div>
          <div class="stats-kpi-sub">${s.uniqueAlbums || 0} albums · ${s.newArtistsDiscovered || 0} newly discovered</div>
        </div>
        <div class="stats-kpi-card accent-amber">
          <div class="stats-kpi-label">Daily Streak ${activeTodayDot}</div>
          <div class="stats-kpi-val">${streaks.currentDays || 0}d</div>
          <div class="stats-kpi-sub">Best streak: ${streaks.longestDays || 0}d · ${streaks.activeDaysAllTime || 0} days all-time</div>
        </div>
        <div class="stats-kpi-card">
          <div class="stats-kpi-label">Completion vs Skips</div>
          <div class="stats-kpi-val">${s.skipRate || 0}% <span class="stats-kpi-unit">skip</span></div>
          <div class="stats-kpi-sub">${s.skippedStreams || 0} skipped · ${s.partialStreams || 0} partial plays</div>
        </div>
        <div class="stats-kpi-card accent-purple">
          <div class="stats-kpi-label">Audio Fidelity</div>
          <div class="stats-kpi-val">${s.losslessPct || 0}% <span class="stats-kpi-unit">lossless</span></div>
          <div class="stats-kpi-sub">${s.totalStreams ? `${s.avgBitrate ? `${s.avgBitrate} kbps avg · ` : ''}${s.losslessStreams || 0} of ${s.totalStreams} streams` : 'No streams logged'}</div>
        </div>
        <div class="stats-kpi-card">
          <div class="stats-kpi-label">Library &amp; Taste</div>
          <div class="stats-kpi-val">${lib.likedTracks || 0} <span class="stats-kpi-unit">likes</span></div>
          <div class="stats-kpi-sub">${lib.manualPlaylists || 0} playlists · ${lib.followedArtists || 0} followed${lib.ratedTracks ? ` · ${lib.avgRating}★` : ''}</div>
        </div>
      </section>
    `;

    if (!s.totalStreams) {
      html += `
        <section class="stats-card stats-empty-card">
          <h2>No Streams Recorded in ${esc(d.rangeLabel || 'This Period')}</h2>
          <p>Play songs from Home, Explore, or Search (or scrobble via Subsonic/Navidrome clients) to unlock your timeline graphs, 24-hour listening clock, mood radar, and top charts.</p>
          <div class="hero-actions" style="justify-content:center;margin-top:14px">
            <button class="btn-primary" onclick="App.navigate('explore')">Explore Music</button>
            <button class="btn-secondary" onclick="App.navigate('home')">Go Home</button>
            ${d.range !== 'all' ? `<button class="btn-secondary" onclick="App.setStatsRange('all')">View All Time</button>` : ''}
          </div>
        </section>
      `;
      if ((d.recentEvents || []).length > 0) {
        html += renderStatsActivityCard(d.recentEvents);
      }
      html += `</div>`;
      $('main-view').innerHTML = html;
      return;
    }

    // Row 1: Listening Timeline + 24-Hour Listening Clock
    const granLabel =
      d.timeline?.granularity === 'month'
        ? 'Monthly'
        : d.timeline?.granularity === 'week'
          ? 'Weekly'
          : 'Daily';
    html += `
      <section class="stats-grid-hero-charts">
        <div class="stats-card">
          <div class="stats-card-head">
            <div>
              <h2>Listening Activity</h2>
              <p class="stats-card-sub">${granLabel} ${metric === 'mins' ? 'minutes listened' : 'streams'} · ${s.activeDays} active day${s.activeDays === 1 ? '' : 's'} (${s.avgStreamsPerActiveDay}/day avg)</p>
            </div>
            <span class="stats-badge">${esc(granLabel)}</span>
          </div>
          <div class="stats-chart-wrap">
            ${renderStatsTimelineSvg(d.timeline, metric)}
          </div>
        </div>
        <div class="stats-card">
          <div class="stats-card-head">
            <div>
              <h2>24-Hour Clock</h2>
              <p class="stats-card-sub">${esc(persona.chronotypeDesc || 'Hourly distribution')}</p>
            </div>
            <span class="stats-badge green">${esc(persona.chronotype || '')}</span>
          </div>
          <div class="stats-clock-wrap">
            ${renderStatsClockSvg(d.hourly, metric, d.peakHour)}
          </div>
        </div>
      </section>
    `;

    // 365-Day Activity Heatmap
    const bestDayText = streaks.bestDay
      ? `Best day: ${fmtShortDayIso(streaks.bestDay.date)} · ${streaks.bestDay.streams} streams (${fmtStatsDuration(streaks.bestDay.secs)})`
      : 'Consistency calendar';
    html += `
      <section class="stats-card stats-section-gap">
        <div class="stats-card-head">
          <div>
            <h2>Listening Consistency (Past 365 Days)</h2>
            <p class="stats-card-sub">${esc(bestDayText)}</p>
          </div>
          <div class="stats-inline-chips">
            <span class="stats-badge">Current: ${streaks.currentDays || 0}d</span>
            <span class="stats-badge">Best: ${streaks.longestDays || 0}d</span>
          </div>
        </div>
        ${renderStatsHeatmap(d.heatmap)}
      </section>
    `;

    // Row 2: Mood DNA Radar + Weekday & Decade Charts
    const activeMoods = [...(d.moods || [])].filter((m) => m.streams > 0).sort((a, b) => b.streams - a.streams);
    const weekdays = d.weekdays || [];
    const maxWdVal = Math.max(1, ...weekdays.map((w) => (metric === 'mins' ? Math.round((w.secs || 0) / 60) : (w.streams || 0))));
    const decades = d.decades || [];
    const maxDecVal = Math.max(1, ...decades.map((x) => (metric === 'mins' ? Math.round((x.secs || 0) / 60) : (x.streams || 0))));

    html += `
      <section class="stats-grid-2">
        <div class="stats-card">
          <div class="stats-card-head">
            <div>
              <h2>Mood &amp; Vibe DNA</h2>
              <p class="stats-card-sub">Your emotional listening fingerprint across Beatlab’s 10 vibes</p>
            </div>
          </div>
          <div class="stats-mood-split">
            <div class="stats-radar-wrap">${renderStatsMoodRadarSvg(d.moods)}</div>
            <div class="stats-mood-bars">
              ${
                activeMoods.length
                  ? activeMoods
                      .slice(0, 6)
                      .map(
                        (m) => `
                    <div class="stats-hbar-row clickable" onclick="App.navigate('mood/${encodeURIComponent(m.slug)}')">
                      <div class="stats-hbar-top">
                        <span class="stats-hbar-name">${esc(m.label)}</span>
                        <span class="stats-hbar-val">${m.streams} · ${m.pct}%</span>
                      </div>
                      <div class="stats-hbar-track">
                        <div class="stats-hbar-fill" style="width:${Math.max(4, m.pct)}%;background:${esc(m.c1)}"></div>
                      </div>
                    </div>`
                      )
                      .join('')
                  : '<p class="muted-center">Mood breakdown appears as tracks are analyzed.</p>'
              }
            </div>
          </div>
        </div>

        <div class="stats-card">
          <div class="stats-card-head">
            <div>
              <h2>Weekly Rhythm &amp; Eras</h2>
              <p class="stats-card-sub">${d.peakWeekday ? `Peak day: ${esc(d.peakWeekday)}` : 'Day-of-week & release decades'}</p>
            </div>
          </div>
          <div class="stats-subblock">
            <div class="stats-subblock-title">Day of Week (${metric === 'mins' ? 'Minutes' : 'Streams'})</div>
            <div class="stats-vbar-chart">
              ${weekdays
                .map((w) => {
                  const v = metric === 'mins' ? Math.round((w.secs || 0) / 60) : w.streams;
                  const hPct = v > 0 ? Math.max(8, (v / maxWdVal) * 100) : 3;
                  const isPeak = d.peakWeekday && w.fullDay === d.peakWeekday && w.streams > 0;
                  const tipBody = `${w.streams} streams (${w.pct}%) · ${fmtStatsDuration(w.secs)}`;
                  return `
                    <div class="stats-vbar-col stats-tip-target" data-tip-title="${esc(w.fullDay)}" data-tip-body="${esc(tipBody)}">
                      <div class="stats-vbar-num">${v > 0 ? v : ''}</div>
                      <div class="stats-vbar-track">
                        <div class="stats-vbar-fill ${isPeak ? 'peak' : ''}" style="height:${hPct.toFixed(1)}%"></div>
                      </div>
                      <div class="stats-vbar-lbl">${esc(w.day)}</div>
                    </div>`;
                })
                .join('')}
            </div>
          </div>

          ${
            decades.length > 0
              ? `<div class="stats-subblock" style="margin-top:16px">
                  <div class="stats-subblock-title">Music Eras &amp; Release Decades</div>
                  <div class="stats-vbar-chart compact">
                    ${decades
                      .map((dec) => {
                        const v = metric === 'mins' ? Math.round((dec.secs || 0) / 60) : dec.streams;
                        const hPct = v > 0 ? Math.max(10, (v / maxDecVal) * 100) : 4;
                        const tipBody = `${dec.streams} streams (${dec.pct}%) · ${fmtStatsDuration(dec.secs)}`;
                        return `
                          <div class="stats-vbar-col stats-tip-target" data-tip-title="${esc(dec.label)}" data-tip-body="${esc(tipBody)}">
                            <div class="stats-vbar-num">${dec.pct}%</div>
                            <div class="stats-vbar-track">
                              <div class="stats-vbar-fill blue" style="height:${hPct.toFixed(1)}%"></div>
                            </div>
                            <div class="stats-vbar-lbl">${esc(dec.label)}</div>
                          </div>`;
                      })
                      .join('')}
                  </div>
                  ${
                    d.oldestTrack || d.newestTrack
                      ? `<div class="stats-era-footnotes">
                          ${d.oldestTrack ? `<span>Oldest: <strong>${d.oldestTrack.year}</strong> (${esc(d.oldestTrack.title)})</span>` : ''}
                          ${d.newestTrack ? `<span>Newest: <strong>${d.newestTrack.year}</strong> (${esc(d.newestTrack.title)})</span>` : ''}
                        </div>`
                      : ''
                  }
                </div>`
              : ''
          }
        </div>
      </section>
    `;

    // Row 3: Top Genres
    const genres = d.genres || [];
    const maxGenreStreams = Math.max(1, ...genres.map((g) => g.streams || 0));

    html += `
      <section class="stats-card stats-section-gap">
        <div class="stats-card-head">
          <div>
            <h2>Top Genres</h2>
            <p class="stats-card-sub">${genres.length ? 'Click any genre to explore & play' : 'Genre tags from streamed tracks'}</p>
          </div>
        </div>
        ${
          genres.length
            ? `<div class="stats-genre-list">
                ${genres
                  .slice(0, 10)
                  .map((g, idx) => {
                    const wPct = Math.max(6, (g.streams / maxGenreStreams) * 100);
                    return `
                      <div class="stats-hbar-row clickable" onclick="App.navigate('genre/${encodeURIComponent(g.name)}')">
                        <div class="stats-hbar-top">
                          <span class="stats-hbar-name"><span class="stats-rank-num">#${idx + 1}</span> ${esc(g.name)}</span>
                          <span class="stats-hbar-val">${g.streams} · ${fmtStatsDuration(g.secs)} (${g.pct}%)</span>
                        </div>
                        <div class="stats-hbar-track">
                          <div class="stats-hbar-fill" style="width:${wPct.toFixed(1)}%;background:${idx === 0 ? '#1ed760' : idx < 3 ? '#38bdf8' : '#a855f7'}"></div>
                        </div>
                      </div>`;
                  })
                  .join('')}
              </div>`
            : '<p class="muted-center">No genre tags logged yet.</p>'
        }
      </section>
    `;

    // Row 4: Top Artists & Top Albums Leaderboards
    const topArtists = d.topArtists || [];
    const maxArtistStreams = Math.max(1, ...topArtists.map((a) => a.streams || 0));
    const topAlbums = d.topAlbums || [];
    const maxAlbumStreams = Math.max(1, ...topAlbums.map((a) => a.streams || 0));

    html += `
      <section class="stats-grid-2">
        <div class="stats-card">
          <div class="stats-card-head">
            <div>
              <h2>Top Artists</h2>
              <p class="stats-card-sub">Ranked by stream count in ${esc(d.rangeLabel || 'All Time')}</p>
            </div>
          </div>
          ${
            topArtists.length
              ? `<div class="stats-lb-list">
                  ${topArtists
                    .slice(0, 10)
                    .map(
                      (a, idx) => `
                    <div class="stats-lb-row" onclick="App.navigate('artist/${encodeURIComponent(a.id)}')">
                      <span class="stats-lb-rank ${idx < 3 ? 'top' : ''}">${idx + 1}</span>
                      <img class="stats-lb-img round" src="${esc(a.coverUrl)}" loading="lazy" alt="" />
                      <div class="stats-lb-info">
                        <div class="stats-lb-title">${esc(a.name)} ${a.isFollowed ? '<span class="stats-mini-tag">Following</span>' : ''}</div>
                        <div class="stats-lb-sub">${a.uniqueTracks} track${a.uniqueTracks === 1 ? '' : 's'} · ${fmtStatsDuration(a.listenedSecs)}</div>
                        <div class="stats-lb-meter"><span style="width:${Math.max(4, (a.streams / maxArtistStreams) * 100).toFixed(1)}%"></span></div>
                      </div>
                      <div class="stats-lb-count"><strong>${a.streams}</strong><span>plays</span></div>
                    </div>`
                    )
                    .join('')}
                </div>`
              : '<p class="muted-center">No artist plays yet.</p>'
          }
        </div>

        <div class="stats-card">
          <div class="stats-card-head">
            <div>
              <h2>Top Albums</h2>
              <p class="stats-card-sub">Most spun records in ${esc(d.rangeLabel || 'All Time')}</p>
            </div>
          </div>
          ${
            topAlbums.length
              ? `<div class="stats-lb-list">
                  ${topAlbums
                    .slice(0, 10)
                    .map(
                      (al, idx) => `
                    <div class="stats-lb-row" onclick="App.navigate('album/${encodeURIComponent(al.id)}')">
                      <span class="stats-lb-rank ${idx < 3 ? 'top' : ''}">${idx + 1}</span>
                      <img class="stats-lb-img" src="${esc(al.coverUrl)}" loading="lazy" alt="" />
                      <div class="stats-lb-info">
                        <div class="stats-lb-title">${esc(al.name)} ${al.isSaved ? '<span class="stats-mini-tag">Saved</span>' : ''}</div>
                        <div class="stats-lb-sub">${esc(al.artist)}${al.year ? ` · ${al.year}` : ''} · ${fmtStatsDuration(al.listenedSecs)}</div>
                        <div class="stats-lb-meter"><span style="width:${Math.max(4, (al.streams / maxAlbumStreams) * 100).toFixed(1)}%;background:var(--accent-blue)"></span></div>
                      </div>
                      <div class="stats-lb-count"><strong>${al.streams}</strong><span>plays</span></div>
                    </div>`
                    )
                    .join('')}
                </div>`
              : '<p class="muted-center">No album plays yet.</p>'
          }
        </div>
      </section>
    `;

    // Top Tracks Playable Leaderboard
    const topTracks = d.topTracks || [];
    if (topTracks.length > 0) {
      const topTrackObjs = topTracks.map((x) => x.track);
      const topIdsJson = esc(JSON.stringify(topTrackObjs.map((t) => t.id)));
      html += `
        <section class="stats-card stats-section-gap">
          <div class="stats-card-head">
            <div>
              <h2>Most Played Tracks</h2>
              <p class="stats-card-sub">Your top ${topTracks.length} heavy rotation songs</p>
            </div>
            <div style="display:flex;gap:8px;flex-wrap:wrap">
              <button class="btn-primary" onclick="App.playListByIds(${topIdsJson}, 0)">${ICONS.play} Play Top Tracks</button>
              <button class="btn-secondary" onclick="App.playShuffledIds(${topIdsJson})">Shuffle</button>
            </div>
          </div>
          <div class="track-table">
            ${topTracks
              .map((item, idx) => {
                const t = item.track;
                const cur = currentTrack();
                const isCur = cur && cur.id === t.id;
                return `
                  <div class="track-row stats-top-track-row ${isCur ? 'playing' : ''}" data-tid="${esc(t.id)}">
                    <div class="tr-idx" onclick="App.playListByIds(${topIdsJson}, ${idx})">
                      <span class="tr-num ${idx < 3 ? 'stats-medal' : ''}">${isCur ? ICONS.nowPlaying : idx + 1}</span>
                      <span class="tr-play-icon">${ICONS.play}</span>
                    </div>
                    <div class="tr-main" onclick="App.playListByIds(${topIdsJson}, ${idx})">
                      <img class="tr-thumb" src="${esc(t.coverUrl)}" loading="lazy" alt="" />
                      <div class="tr-text">
                        <div class="tr-title-row">
                          <div class="tr-title">${esc(t.title)}</div>
                        </div>
                        <div class="tr-sub">
                          <span class="clickable" onclick="event.stopPropagation();App.navigate('artist/${encodeURIComponent(t.artistId)}')">${esc(t.artist)}</span>
                          ${t.album ? ` · <span class="clickable" onclick="event.stopPropagation();App.navigate('album/${encodeURIComponent(t.albumId)}')">${esc(t.album)}</span>` : ''}
                        </div>
                      </div>
                    </div>
                    <span class="stats-play-pill">${item.streams} play${item.streams === 1 ? '' : 's'} · ${fmtStatsDuration(item.listenedSecs)}</span>
                    <span class="quality-badge ${t.isLossless ? 'lossless' : ''}">${esc(t.qualityBadge)}</span>
                    <div class="tr-dur">${fmtTime(t.duration)}</div>
                    <div class="tr-actions">
                      <button class="icon-btn tr-like-btn ${t.isStarred ? 'active' : ''}" title="Like" onclick="App.toggleTrackLike(${esc(JSON.stringify(t.id))}, this)">${ICONS.heart}</button>
                      <button class="icon-btn" title="More options" onclick="App.openTrackMenu(event, ${esc(JSON.stringify(t.id))})">${ICONS.more}</button>
                    </div>
                  </div>`;
              })
              .join('')}
          </div>
        </section>
      `;
    }

    // Bottom Row: Milestones, Ratings & Most Skipped + Recent Activity Feed
    const hl = d.highlights || {};
    const mostSkipped = d.mostSkipped || [];
    const ratingsDist = lib.ratingsDist || [0, 0, 0, 0, 0];
    const maxRatingCnt = Math.max(1, ...ratingsDist);

    html += `
      <section class="stats-grid-2">
        <div class="stats-card">
          <div class="stats-card-head">
            <div>
              <h2>Milestones &amp; Quirks</h2>
              <p class="stats-card-sub">Standout moments from your listening history</p>
            </div>
          </div>
          <div class="stats-milestones-grid">
            ${
              hl.mostCompletedTrack
                ? `<div class="stats-ms-tile clickable" onclick="App.playSingle(${esc(JSON.stringify(hl.mostCompletedTrack.track.id))})">
                    <div class="stats-ms-kicker">MOST COMPLETED</div>
                    <div class="stats-ms-title">${esc(hl.mostCompletedTrack.track.title)}</div>
                    <div class="stats-ms-sub">${esc(hl.mostCompletedTrack.track.artist)} · ${hl.mostCompletedTrack.completed} full listens</div>
                  </div>`
                : ''
            }
            ${
              hl.longestTrack
                ? `<div class="stats-ms-tile clickable" onclick="App.playSingle(${esc(JSON.stringify(hl.longestTrack.track.id))})">
                    <div class="stats-ms-kicker">LONGEST TRACK</div>
                    <div class="stats-ms-title">${esc(hl.longestTrack.track.title)}</div>
                    <div class="stats-ms-sub">${esc(hl.longestTrack.track.artist)} · ${fmtTime(hl.longestTrack.duration)}</div>
                  </div>`
                : ''
            }
            ${
              hl.shortestTrack
                ? `<div class="stats-ms-tile clickable" onclick="App.playSingle(${esc(JSON.stringify(hl.shortestTrack.track.id))})">
                    <div class="stats-ms-kicker">SHORTEST TRACK</div>
                    <div class="stats-ms-title">${esc(hl.shortestTrack.track.title)}</div>
                    <div class="stats-ms-sub">${esc(hl.shortestTrack.track.artist)} · ${fmtTime(hl.shortestTrack.duration)}</div>
                  </div>`
                : ''
            }
            ${
              hl.firstTrackEver
                ? `<div class="stats-ms-tile clickable" onclick="App.playSingle(${esc(JSON.stringify(hl.firstTrackEver.track.id))})">
                    <div class="stats-ms-kicker">FIRST TRACK LOGGED</div>
                    <div class="stats-ms-title">${esc(hl.firstTrackEver.track.title)}</div>
                    <div class="stats-ms-sub">${esc(hl.firstTrackEver.track.artist)} · ${esc(fmtDateLabel(hl.firstTrackEver.at))}</div>
                  </div>`
                : ''
            }
          </div>

          ${
            lib.ratedTracks > 0
              ? `<div class="stats-subblock" style="margin-top:16px">
                  <div class="stats-subblock-title">Star Ratings Distribution (${lib.ratedTracks} rated · ${lib.avgRating}★ avg)</div>
                  <div class="stats-ratings-bars">
                    ${[5, 4, 3, 2, 1]
                      .map((star) => {
                        const cnt = ratingsDist[star - 1] || 0;
                        const w = cnt > 0 ? Math.max(4, (cnt / maxRatingCnt) * 100) : 0;
                        return `
                          <div class="stats-rating-row">
                            <span>${star}★</span>
                            <div class="stats-hbar-track"><div class="stats-hbar-fill" style="width:${w.toFixed(1)}%;background:#fbbf24"></div></div>
                            <span>${cnt}</span>
                          </div>`;
                      })
                      .join('')}
                  </div>
                </div>`
              : ''
          }

          ${
            mostSkipped.length > 0
              ? `<div class="stats-subblock" style="margin-top:16px">
                  <div class="stats-subblock-title">Most Skipped Tracks</div>
                  <div class="stats-mini-list">
                    ${mostSkipped
                      .slice(0, 5)
                      .map(
                        (ms) => `
                      <div class="stats-mini-row" data-tid="${esc(ms.track.id)}" onclick="App.playSingle(${esc(JSON.stringify(ms.track.id))})">
                        <img src="${esc(ms.track.coverUrl)}" loading="lazy" alt="" />
                        <div class="stats-mini-meta">
                          <div class="stats-mini-title">${esc(ms.track.title)}</div>
                          <div class="stats-mini-sub">${esc(ms.track.artist)}</div>
                        </div>
                        <span class="stats-skip-pill">${ms.skips} skip${ms.skips === 1 ? '' : 's'}</span>
                      </div>`
                      )
                      .join('')}
                  </div>
                </div>`
              : ''
          }
        </div>

        ${renderStatsActivityCard(d.recentEvents || [])}
      </section>
    `;

    html += `</div>`;
    $('main-view').innerHTML = html;

    // Auto-scroll 365-day heatmap to the latest week on narrow viewports
    const hmScroll = $('stats-hm-scroll');
    if (hmScroll) {
      hmScroll.scrollLeft = hmScroll.scrollWidth;
    }
  }

  function renderStatsActivityCard(recentEvents) {
    return `
      <div class="stats-card">
        <div class="stats-card-head">
          <div>
            <h2>Recent Activity Log</h2>
            <p class="stats-card-sub">Latest playback &amp; taste events across web &amp; Subsonic</p>
          </div>
        </div>
        ${
          recentEvents.length
            ? `<div class="stats-activity-list">
                ${recentEvents
                  .map(
                    (ev) => `
                  <div class="stats-act-row" data-tid="${esc(ev.track.id)}" onclick="App.playSingle(${esc(JSON.stringify(ev.track.id))})">
                    <img src="${esc(ev.track.coverUrl)}" loading="lazy" alt="" />
                    <div class="stats-act-meta">
                      <div class="stats-act-title">${esc(ev.track.title)}</div>
                      <div class="stats-act-sub">${esc(ev.track.artist)}</div>
                    </div>
                    <span class="stats-act-kind kind-${esc(ev.kind)}">${esc(ev.label)}</span>
                    <span class="stats-act-time">${esc(fmtRelativeTime(ev.at))}</span>
                  </div>`
                  )
                  .join('')}
              </div>`
            : '<p class="muted-center">No recent events recorded yet.</p>'
        }
      </div>
    `;
  }

  // ---------------------------------------------------------------------------
  // Reusable UI Renderers
  // ---------------------------------------------------------------------------
  function renderQuickTile(t) {
    return `<div class="qp-tile" data-tid="${esc(t.id)}" onclick="App.playSingle(${esc(JSON.stringify(t.id))})">
      <img src="${esc(t.coverUrl)}" loading="lazy" alt="" />
      <div class="qp-info">
        <div class="qp-title">${esc(t.title)}</div>
        <div class="qp-sub">${esc(t.artist)}</div>
      </div>
      <div class="qp-play">${ICONS.play}</div>
    </div>`;
  }

  function renderTrackCard(t) {
    return `<div class="media-card" data-tid="${esc(t.id)}" onclick="App.playSingle(${esc(JSON.stringify(t.id))})">
      <div class="card-art-wrap">
        <img src="${esc(t.coverUrl)}" loading="lazy" alt="" />
        <button class="card-play-fab">${ICONS.play}</button>
      </div>
      <div class="card-title">${esc(t.title)}</div>
      <div class="card-sub">${esc(t.artist)}</div>
    </div>`;
  }

  function renderAlbumCard(a) {
    const sub = `${a.artist}${a.year ? ` · ${a.year}` : ''}`;
    return `<div class="media-card" data-share-kind="album" data-share-id="${esc(a.id)}" data-share-title="${esc(a.name)}" data-share-sub="${esc(sub)}" data-share-cover="${esc(a.coverUrl)}" onclick="App.navigate('album/${encodeURIComponent(a.id)}')">
      <div class="card-art-wrap">
        <img src="${esc(a.coverUrl)}" loading="lazy" alt="" />
        <button class="card-play-fab" onclick="event.stopPropagation();App.playAlbumNow(${esc(JSON.stringify(a.id))})">${ICONS.play}</button>
      </div>
      <div class="card-title">${esc(a.name)}</div>
      <div class="card-sub">${esc(sub)}</div>
    </div>`;
  }

  function renderNewReleaseCard(a) {
    const sub = `${a.artist}${a.year ? ` · ${a.year}` : ''}`;
    return `<div class="media-card new-release-card" data-share-kind="album" data-share-id="${esc(a.id)}" data-share-title="${esc(a.name)}" data-share-sub="${esc(sub)}" data-share-cover="${esc(a.coverUrl)}" onclick="App.navigate('album/${encodeURIComponent(a.id)}')">
      <div class="card-art-wrap">
        <img src="${esc(a.coverUrl)}" loading="lazy" alt="" />
        <span class="new-release-badge">NEW</span>
        <button class="card-play-fab" onclick="event.stopPropagation();App.playAlbumNow(${esc(JSON.stringify(a.id))})">${ICONS.play}</button>
      </div>
      <div class="card-title">${esc(a.name)}</div>
      <div class="card-sub">${esc(sub)}</div>
    </div>`;
  }

  function renderArtistCard(a) {
    return `<div class="media-card" data-share-kind="artist" data-share-id="${esc(a.id)}" data-share-title="${esc(a.name)}" data-share-sub="Artist" data-share-cover="${esc(a.coverUrl)}" onclick="App.navigate('artist/${encodeURIComponent(a.id)}')">
      <div class="card-art-wrap round">
        <img src="${esc(a.coverUrl)}" loading="lazy" alt="" />
      </div>
      <div class="card-title" style="text-align:center">${esc(a.name)}</div>
      <div class="card-sub" style="text-align:center">Artist</div>
    </div>`;
  }

  function renderPlaylistCard(p) {
    const sub = `${p.trackCount} tracks · ${p.ownerName}`;
    const pinned = isPlaylistPinned(p.id);
    return `<div class="media-card" data-share-kind="playlist" data-share-id="${esc(p.id)}" data-share-title="${esc(p.name)}" data-share-sub="${esc(sub)}" data-share-cover="${esc(p.coverUrl)}" onclick="App.navigate('playlist/${encodeURIComponent(p.id)}')">
      <div class="card-art-wrap">
        <img src="${esc(p.coverUrl)}" loading="lazy" alt="" />
        <button class="card-pin-btn ${pinned ? 'pinned' : ''}" title="${pinned ? 'Unpin from sidebar' : 'Pin to sidebar'}" aria-label="${pinned ? 'Unpin' : 'Pin'} ${esc(p.name)}" onclick="event.stopPropagation();App.togglePinPlaylist(${esc(JSON.stringify(p.id))})">${ICONS.pin}</button>
        ${!p.isAuto ? `<button class="card-del-btn" title="Delete playlist" aria-label="Delete ${esc(p.name)}" onclick="event.stopPropagation();App.deletePlaylist(${esc(JSON.stringify(p.id))})">${ICONS.trash}</button>` : ''}
        <button class="card-play-fab" onclick="event.stopPropagation();App.playPlaylistNow(${esc(JSON.stringify(p.id))})">${ICONS.play}</button>
      </div>
      <div class="card-title">${esc(p.name)}</div>
      <div class="card-sub">${esc(sub)}</div>
    </div>`;
  }

  function renderMoodTile(m) {
    const covers = Array.isArray(m.covers) ? m.covers.filter(Boolean).slice(0, 3) : [];
    const thumbs = covers.length
      ? `<div class="mood-tile-thumbs">${covers.map((c) => `<img src="${esc(c)}" loading="lazy" alt="" />`).join('')}</div>`
      : '';
    const countTag = m.trackCount > 0 ? `<span class="mood-tile-count">${m.trackCount} track${m.trackCount === 1 ? '' : 's'}</span>` : '';
    return `<div class="mood-tile" style="background:${esc(m.c2)};border-color:${esc(m.c1)}55;border-left:4px solid ${esc(m.c1)}" onclick="App.navigate('mood/${encodeURIComponent(m.slug)}')">
      <div class="mood-tile-top">
        <h3>${esc(m.label)}</h3>
        ${countTag}
      </div>
      <p>${esc(m.subtitle)}</p>
      ${thumbs}
    </div>`;
  }

  function renderTrackRows(
    tracks,
    fullContext = null,
    editablePlaylistId = null,
    singleRadio = false,
    searchQuery = '',
    useTrackNo = false
  ) {
    const queueList = fullContext || tracks;
    const idsJson = singleRadio ? '' : esc(JSON.stringify(queueList.map((t) => t.id)));
    const cur = currentTrack();

    return tracks
      .map((t, i) => {
        const ctxIdx =
          fullContext && fullContext !== tracks
            ? Math.max(0, queueList.findIndex((x) => x.id === t.id))
            : i;
        const isCur = cur && cur.id === t.id;
        let playClick;
        if (searchQuery) {
          playClick = singleRadio
            ? `App.playSearchTrack(${esc(JSON.stringify(t.id))})`
            : `App.playInstantMixAt(${ctxIdx})`;
        } else {
          playClick = singleRadio
            ? `App.playSingle(${esc(JSON.stringify(t.id))})`
            : `App.playListByIds(${idsJson}, ${ctxIdx})`;
        }
        const editArgs = editablePlaylistId
          ? `, ${esc(JSON.stringify(editablePlaylistId))}, ${i}`
          : '';
        const plDataAttrs = editablePlaylistId
          ? ` data-pl-id="${esc(editablePlaylistId)}" data-pl-pos="${i}"`
          : '';
        return `<div class="track-row ${isCur ? 'playing' : ''}" data-tid="${esc(t.id)}"${plDataAttrs}>
          <div class="tr-idx" onclick="${playClick}">${isCur ? ICONS.nowPlaying : useTrackNo && t.trackNo > 0 ? t.trackNo : i + 1}</div>
          <div class="tr-main" onclick="${playClick}">
            <img class="tr-thumb" src="${esc(t.coverUrl)}" loading="lazy" alt="" />
            <div class="tr-text">
              <div class="tr-title-row">
                <div class="tr-title">${esc(t.title)}</div>
                ${searchQuery && t.isTasteMatch ? '<span class="taste-pill">For You</span>' : ''}
              </div>
              <div class="tr-sub">
                <span class="clickable" onclick="event.stopPropagation();App.navigate('artist/${encodeURIComponent(t.artistId)}')">${esc(t.artist)}</span>
                ${t.album ? ` · <span class="clickable" onclick="event.stopPropagation();App.navigate('album/${encodeURIComponent(t.albumId)}')">${esc(t.album)}</span>` : ''}
              </div>
            </div>
          </div>
          <span class="quality-badge ${t.isLossless ? 'lossless' : ''}">${esc(t.qualityBadge)}</span>
          <div class="tr-dur">${fmtTime(t.duration)}</div>
          <div class="tr-actions">
            <button class="icon-btn tr-like-btn ${t.isStarred ? 'active' : ''}" title="Like" onclick="App.toggleTrackLike(${esc(JSON.stringify(t.id))}, this)">
              ${ICONS.heart}
            </button>
            ${
              editablePlaylistId
                ? `<button class="icon-btn tr-edit-btn" title="Move Up" onclick="App.movePlaylistItem(${esc(JSON.stringify(editablePlaylistId))}, ${i}, ${Math.max(0, i - 1)})">${ICONS.arrowUp}</button>
                   <button class="icon-btn tr-edit-btn" title="Remove" onclick="App.removePlaylistItem(${esc(JSON.stringify(editablePlaylistId))}, ${i})">${ICONS.close}</button>`
                : ''
            }
            <button class="icon-btn" title="More options" onclick="App.openTrackMenu(event, ${esc(JSON.stringify(t.id))}${editArgs})">${ICONS.more}</button>
          </div>
        </div>`;
      })
      .join('');
  }

  // ---------------------------------------------------------------------------
  // Playback, Queue & Autoplay Engine
  // ---------------------------------------------------------------------------
  function canBrowserPlayFormat(fmt) {
    const f = String(fmt || '').trim().toLowerCase();
    if (!f) return true;
    const probes = {
      mp3: ['audio/mpeg'],
      flac: ['audio/flac', 'audio/x-flac'],
      wav: ['audio/wav', 'audio/wave', 'audio/x-wav'],
      m4a: ['audio/mp4; codecs="mp4a.40.2"', 'audio/mp4'],
      mp4: ['audio/mp4; codecs="mp4a.40.2"', 'audio/mp4'],
      aac: ['audio/aac', 'audio/mp4; codecs="mp4a.40.2"'],
      ogg: ['audio/ogg; codecs="vorbis"', 'audio/ogg; codecs="opus"', 'audio/ogg'],
      oga: ['audio/ogg; codecs="vorbis"', 'audio/ogg; codecs="opus"', 'audio/ogg'],
      opus: ['audio/ogg; codecs="opus"', 'audio/webm; codecs="opus"', 'audio/opus'],
      webm: ['audio/webm; codecs="opus"', 'audio/webm; codecs="vorbis"', 'audio/webm'],
    };
    const mimes = probes[f];
    if (!mimes) return false;
    return mimes.some((m) => audio.canPlayType(m) !== '');
  }

  function buildStreamUrl(track, forceCompat = false, offsetSec = 0) {
    const trackId = typeof track === 'string' ? track : track.id;
    const needCompat = forceCompat || (typeof track === 'object' && track && !canBrowserPlayFormat(track.format));
    let url = `/api/web/stream/${encodeURIComponent(trackId)}?token=${encodeURIComponent(state.token)}`;
    if (needCompat) url += '&compat=1';
    if (offsetSec > 0.5) url += `&t=${encodeURIComponent(offsetSec.toFixed(2))}`;
    return url;
  }

  function cancelPendingPlaybackWork() {
    if (state.playCommitTimer) {
      clearTimeout(state.playCommitTimer);
      state.playCommitTimer = null;
    }
    if (state.queueWarmTimer) {
      clearTimeout(state.queueWarmTimer);
      state.queueWarmTimer = null;
    }
    if (state.lyricsAbortController) {
      try {
        state.lyricsAbortController.abort();
      } catch {}
      state.lyricsAbortController = null;
    }
    state.lyricsReqToken++;
  }

  function beginQueueFetch() {
    if (state.queueAbortController) {
      try {
        state.queueAbortController.abort();
      } catch {}
    }
    const ctrl = typeof AbortController !== 'undefined' ? new AbortController() : null;
    state.queueAbortController = ctrl;
    return {
      token: ++state.queueActionToken,
      signal: ctrl ? ctrl.signal : undefined,
    };
  }

  function commitTrackLoad(loadToken, track) {
    if (loadToken !== state.activeAudioToken) return;
    const cur = currentTrack();
    if (!cur || cur.id !== track.id) return;

    applyCommittedMediaUI(track);
    try {
      if (!audio.paused) audio.pause();
    } catch {}
    audio.src = buildStreamUrl(track);
    audio.play().catch((e) => {
      if (loadToken !== state.activeAudioToken) return;
      if (e && e.name === 'AbortError') return;
      updatePlayButton(false);
      console.warn('Playback prevented or error:', e);
    });

    loadLyricsForTrack(track.id);
    state.lastCommittedTrackId = track.id;
    sendTrackEvent(track.id, 'play', 1);

    // Pre-warm next track
    scheduleWarmNextTrack(loadToken);
  }

  function syncShuffleUI() {
    $('btn-shuffle').classList.toggle('active', state.shuffle);
    if ($('fs-shuffle')) $('fs-shuffle').classList.toggle('active', state.shuffle);
  }

  function shuffleArray(items) {
    const copy = items.slice();
    for (let i = copy.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [copy[i], copy[j]] = [copy[j], copy[i]];
    }
    return copy;
  }

  function scheduleWarmNextTrack(loadToken = state.activeAudioToken) {
    if (state.queueWarmTimer) {
      clearTimeout(state.queueWarmTimer);
      state.queueWarmTimer = null;
    }
    const next = state.queue[state.queueIndex + 1];
    const next2 = state.queue[state.queueIndex + 2];
    if (!next) return;
    state.queueWarmTimer = setTimeout(() => {
      state.queueWarmTimer = null;
      if (loadToken !== state.activeAudioToken) return;
      api(`/api/web/warm/${encodeURIComponent(next.id)}`, { method: 'POST' }).catch(() => {});
      if (next2) {
        setTimeout(() => {
          if (loadToken !== state.activeAudioToken) return;
          api(`/api/web/warm/${encodeURIComponent(next2.id)}`, { method: 'POST' }).catch(() => {});
        }, 1050);
      }
    }, 150);
  }

  function restoreUpcomingOrder(upcoming, original, cur) {
    const len = original.length;
    if (!len || upcoming.length <= 1) return upcoming.slice();

    const curOrigIdx = cur ? original.findIndex((t) => t.id === cur.id) : -1;
    const posQueues = new Map();
    original.forEach((t, idx) => {
      if (!posQueues.has(t.id)) posQueues.set(t.id, []);
      posQueues.get(t.id).push(idx);
    });

    if (curOrigIdx >= 0 && cur) {
      const q = posQueues.get(cur.id);
      if (q) {
        const hit = q.indexOf(curOrigIdx);
        if (hit >= 0) q.splice(hit, 1);
      }
    }

    const known = [];
    const extras = [];
    for (const t of upcoming) {
      const q = posQueues.get(t.id);
      const origIdx = q && q.length ? q.shift() : -1;
      if (origIdx >= 0) {
        const sortKey = curOrigIdx >= 0 ? (origIdx - curOrigIdx - 1 + len) % len : origIdx;
        known.push({ track: t, sortKey });
      } else {
        extras.push(t);
      }
    }
    known.sort((a, b) => a.sortKey - b.sortKey);
    return known.map((x) => x.track).concat(extras);
  }

  function setQueueWithShuffle(tracks, startIndex = 0, forceShuffle = false) {
    state.radioPlaylistId = null;
    const ordered = tracks.slice();
    if (!ordered.length) {
      state.unshuffledQueue = null;
      state.queue = [];
      return -1;
    }
    if (forceShuffle) {
      state.shuffle = true;
      localStorage.setItem('beatlab_shuffle', 'true');
      syncShuffleUI();
      state.unshuffledQueue = ordered;
      state.queue = shuffleArray(ordered);
      return 0;
    }
    if (state.shuffle && ordered.length > 1) {
      const idx = Math.max(0, Math.min(startIndex, ordered.length - 1));
      state.unshuffledQueue = ordered;
      const pinned = ordered[idx];
      const rest = ordered.filter((_, i) => i !== idx);
      state.queue = [pinned, ...shuffleArray(rest)];
      return 0;
    }
    state.unshuffledQueue = null;
    state.queue = ordered;
    return Math.max(0, Math.min(startIndex, ordered.length - 1));
  }

  async function playQueueAt(index) {
    if (index < 0 || index >= state.queue.length) return;
    const prevTrackObj = currentTrack();
    const track = state.queue[index];
    if (prevTrackObj && prevTrackObj.id !== track.id) {
      recordRecentHeard(prevTrackObj);
    }
    recordRecentHeard(track);
    const now = typeof performance !== 'undefined' && performance.now ? performance.now() : Date.now();
    const isBurst = state.lastPlaySwitchAt > 0 && now - state.lastPlaySwitchAt < 280;
    state.lastPlaySwitchAt = now;

    cancelPendingPlaybackWork();
    const loadToken = ++state.activeAudioToken;
    state.queueIndex = index;
    state.pendingSeekSec = 0;
    state.lastScrobbleCompleteId = '';
    state.compatAttemptedId = '';

    updateNowPlayingUI(track, true, isBurst);
    renderQueuePanel();
    saveServerQueue();

    // Highlight playing track in current view if visible
    document.querySelectorAll('.track-row').forEach((row) => {
      row.classList.toggle('playing', row.dataset.tid === track.id);
    });

    if (!isBurst) {
      commitTrackLoad(loadToken, track);
      return;
    }

    // During rapid skip bursts, pause current stream immediately and coalesce the actual
    // audio/network/lyrics/warm commitment until the user stops skipping for 220ms.
    try {
      if (!audio.paused) audio.pause();
    } catch {}
    state.playCommitTimer = setTimeout(() => {
      state.playCommitTimer = null;
      if (loadToken === state.activeAudioToken) {
        commitTrackLoad(loadToken, track);
      }
    }, 220);
  }

  function applyCommittedMediaUI(track) {
    if (!track) return;
    $('pb-cover').src = track.coverUrl;
    $('fs-cover').src = track.coverUrl;
    $('fs-bg').style.backgroundImage = `url('${track.coverUrl}')`;

    if ('mediaSession' in navigator) {
      try {
        navigator.mediaSession.metadata = new MediaMetadata({
          title: track.title,
          artist: track.artist,
          album: track.album,
          artwork: [{ src: `${location.origin}${track.coverUrl}`, sizes: '512x512', type: 'image/jpeg' }],
        });
        navigator.mediaSession.setActionHandler('play', () => togglePlay());
        navigator.mediaSession.setActionHandler('pause', () => togglePlay());
        navigator.mediaSession.setActionHandler('previoustrack', () => prevTrack());
        navigator.mediaSession.setActionHandler('nexttrack', () => nextTrack(true));
        navigator.mediaSession.setActionHandler('seekto', (details) => {
          if (details.seekTime != null && Number.isFinite(audio.duration)) {
            audio.currentTime = details.seekTime;
          }
        });
      } catch {
        // unsupported in some browsers
      }
    }
  }

  function updateNowPlayingUI(track, active, skipHeavy = false) {
    if (!track) return;
    if (!skipHeavy) {
      $('pb-cover').src = track.coverUrl;
      $('fs-cover').src = track.coverUrl;
      $('fs-bg').style.backgroundImage = `url('${track.coverUrl}')`;
    }
    $('pb-title').textContent = track.title;
    $('pb-artist').textContent = track.artist;
    $('pb-like').classList.toggle('active', !!track.isStarred);
    $('pb-quality').textContent = track.qualityBadge;
    $('pb-quality').classList.remove('hidden');
    $('pb-quality').classList.toggle('lossless', !!track.isLossless);
    $('pb-duration').textContent = fmtTime(track.duration);
    $('pb-elapsed').textContent = '0:00';
    $('pb-seek').value = '0';
    if ($('pb-mini-progress-fill')) $('pb-mini-progress-fill').style.width = '0%';

    $('fs-title').textContent = track.title;
    $('fs-artist').innerHTML = `<span class="clickable" onclick="App.goCurrentArtist()">${esc(track.artist)}</span>${track.album ? ` · <span class="clickable" onclick="App.goCurrentAlbum()">${esc(track.album)}</span>` : ''}`;
    if ($('fs-like')) $('fs-like').classList.toggle('active', !!track.isStarred);
    if ($('fs-quality')) {
      $('fs-quality').textContent = track.qualityBadge;
      $('fs-quality').classList.remove('hidden');
      $('fs-quality').classList.toggle('lossless', !!track.isLossless);
    }
    if ($('fs-duration')) $('fs-duration').textContent = fmtTime(track.duration);
    if ($('fs-elapsed')) $('fs-elapsed').textContent = '0:00';
    if ($('fs-seek')) $('fs-seek').value = '0';

    document.title = `${track.title} · ${track.artist} — Beatlab`;
    updatePlayButton(active);
  }

  function updatePlayButton(playing) {
    const svgHtml = playing
      ? '<path d="M6.5 4.5h4v15h-4zm7 0h4v15h-4z"/>'
      : '<path d="M8 4.5v15l12-7.5z"/>';
    $('play-icon-svg').innerHTML = svgHtml;
    if ($('fs-play-icon-svg')) $('fs-play-icon-svg').innerHTML = svgHtml;
  }

  function sendTrackEvent(trackId, kind, value = 1) {
    api('/api/web/event', {
      method: 'POST',
      body: JSON.stringify({ trackId, kind, value }),
    }).catch(() => {});
  }

  function playListByIds(ids, startIndex = 0) {
    const tracks = ids.map((id) => state.trackMap.get(id)).filter(Boolean);
    if (!tracks.length) return;
    const prev = currentTrack();
    if (prev) recordRecentHeard(prev);
    state.radioReqToken++;
    state.radioLoading = false;
    state.radioInFlight = null;
    state.radioPlaylistId = null;
    const idx = setQueueWithShuffle(tracks, startIndex, false);
    playQueueAt(idx);
  }

  function playShuffledIds(ids) {
    const tracks = ids.map((id) => state.trackMap.get(id)).filter(Boolean);
    if (!tracks.length) return;
    const prev = currentTrack();
    if (prev) recordRecentHeard(prev);
    state.radioReqToken++;
    state.radioLoading = false;
    state.radioInFlight = null;
    state.radioPlaylistId = null;
    const idx = setQueueWithShuffle(tracks, 0, true);
    playQueueAt(idx);
  }

  function playSingle(id) {
    const t = state.trackMap.get(id);
    if (!t) return;
    const prev = currentTrack();
    if (prev) recordRecentHeard(prev);
    recordRecentHeard(t);
    state.radioReqToken++;
    state.radioLoading = !!state.autoplay;
    state.radioInFlight = null;
    state.radioPlaylistId = null;
    state.unshuffledQueue = null;
    state.queue = [t];
    playQueueAt(0);
    if (state.autoplay) {
      appendRadioTracks(t.id);
    }
  }

  async function playAlbumNow(id) {
    const { token, signal } = beginQueueFetch();
    try {
      const d = await api(`/api/web/album/${encodeURIComponent(id)}`, { signal });
      if (token !== state.queueActionToken) return;
      const tracks = cacheTracks(d.tracks || []);
      if (tracks.length) {
        const prev = currentTrack();
        if (prev) recordRecentHeard(prev);
        state.radioReqToken++;
        state.radioLoading = false;
        state.radioInFlight = null;
        const idx = setQueueWithShuffle(tracks, 0, false);
        playQueueAt(idx);
      }
    } catch (e) {
      if (e && e.name === 'AbortError') return;
      showToast(e.message);
    }
  }

  async function playPlaylistNow(id) {
    const { token, signal } = beginQueueFetch();
    try {
      const d = await api(`/api/web/playlist/${encodeURIComponent(id)}`, { signal });
      if (token !== state.queueActionToken) return;
      const tracks = cacheTracks(d.tracks || []);
      if (tracks.length) {
        const prev = currentTrack();
        if (prev) recordRecentHeard(prev);
        state.radioReqToken++;
        state.radioLoading = false;
        state.radioInFlight = null;
        const idx = setQueueWithShuffle(tracks, 0, false);
        playQueueAt(idx);
      }
    } catch (e) {
      if (e && e.name === 'AbortError') return;
      showToast(e.message);
    }
  }

  function playSearchTrack(id) {
    sendTrackEvent(id, 'search_pick', 1);
    playSingle(id);
  }

  function getActiveInstantMix() {
    if (state.searchState && state.searchState.instantMix) {
      return {
        mix: state.searchState.instantMix,
        query: state.searchState.query,
        inSearch: true,
      };
    }
    if (state.activeInstantMix && state.activeInstantMix.mix) {
      return {
        mix: state.activeInstantMix.mix,
        query: state.activeInstantMix.query,
        inSearch: false,
      };
    }
    return null;
  }

  function refreshActiveInstantMixCard() {
    const card = document.querySelector('#main-view .instant-mix-card');
    if (!card) return;
    if (state.searchState && state.searchState.instantMix) {
      const s = state.searchState;
      card.outerHTML = renderInstantMixCard(s.instantMix, s.refinements, s.query, !!s.mixExpanded);
    } else if (state.activeInstantMix && state.activeInstantMix.mix) {
      const { mix, query, refinements, expanded } = state.activeInstantMix;
      card.outerHTML = renderInstantMixCard(mix, refinements, query, expanded);
    }
  }

  function playInstantMix(shuffled = false, startIndex = 0) {
    const active = getActiveInstantMix();
    const mix = active && active.mix;
    if (!mix || !mix.tracks || !mix.tracks.length) return;
    const ids = mix.tracks.map((t) => t.id);
    const idx = shuffled ? 0 : Math.max(0, Math.min(startIndex, ids.length - 1));
    const pickedId = ids[idx];
    if (pickedId) sendTrackEvent(pickedId, 'mix_play', 1);
    if (shuffled) {
      playShuffledIds(ids);
    } else {
      playListByIds(ids, idx);
    }
  }

  function playInstantMixAt(idx) {
    playInstantMix(false, idx);
  }

  function queueInstantMix() {
    const active = getActiveInstantMix();
    const mix = active && active.mix;
    if (!mix || !mix.tracks || !mix.tracks.length) return;
    const tracks = mix.tracks.map((t) => state.trackMap.get(t.id) || t);
    if (!state.queue.length) {
      const idx = setQueueWithShuffle(tracks, 0, false);
      playQueueAt(idx);
    } else {
      const toAdd = state.shuffle ? shuffleArray(tracks) : tracks;
      state.queue.push(...toAdd);
      if (Array.isArray(state.unshuffledQueue)) {
        state.unshuffledQueue.push(...tracks);
      }
      renderQueuePanel();
      saveServerQueue();
    }
    sendTrackEvent(tracks[0].id, 'mix_play', 1);
    showToast(`Added ${tracks.length} tracks from "${mix.title}"`);
  }

  function setSearchFilter(key) {
    if (!state.searchState) return;
    state.searchState.activeFilter = key || 'all';
    renderSearchView();
  }

  function toggleMixExpand() {
    if (state.searchState && state.searchState.instantMix) {
      state.searchState.mixExpanded = !state.searchState.mixExpanded;
      renderSearchView();
      return;
    }
    if (state.activeInstantMix && state.activeInstantMix.mix) {
      state.activeInstantMix.expanded = !state.activeInstantMix.expanded;
      const { mix, query, refinements, expanded } = state.activeInstantMix;
      const card = document.querySelector('#main-view .instant-mix-card');
      if (card) {
        card.outerHTML = renderInstantMixCard(mix, refinements, query, expanded);
      }
    }
  }

  function runRefinedSearch(query) {
    const q = String(query || '').trim();
    if (!q) return;
    clearTimeout(state.searchTimer);
    state.searchTimer = null;
    state.searchComposing = false;
    state.ownSearchNavQuery = q;
    $('global-search').value = q;
    $('clear-search-btn').classList.remove('hidden');
    navigate(`search?q=${encodeURIComponent(q)}`);
  }

  function appendRadioTracks(seedId) {
    if (state.radioInFlight) return state.radioInFlight;
    const reqToken = ++state.radioReqToken;
    state.radioLoading = true;
    renderQueuePanel();
    const run = (async () => {
      try {
        const excludeIds = state.queue
          .slice(-80)
          .map((t) => t.id)
          .filter(Boolean)
          .join(',');
        const recentParam = buildRecentRadioParam(seedId);
        const params = new URLSearchParams();
        if (state.radioPlaylistId) {
          params.set('playlist', state.radioPlaylistId);
        } else if (seedId) {
          params.set('seed', seedId);
        }
        if (excludeIds) params.set('exclude', excludeIds);
        if (recentParam) params.set('recent', recentParam);
        if (state.upcomingMode && state.upcomingMode !== 'default') {
          params.set('mode', state.upcomingMode);
        }
        const d = await api(`/api/web/radio?${params.toString()}`);
        if (reqToken !== state.radioReqToken) return 0;
        const recs = cacheTracks(d.tracks || []);
        const existing = new Set(state.queue.map((t) => t.id));
        let added = 0;
        for (const r of recs) {
          if (!existing.has(r.id)) {
            state.queue.push(r);
            if (Array.isArray(state.unshuffledQueue)) {
              state.unshuffledQueue.push(r);
            }
            existing.add(r.id);
            added++;
          }
        }
        if (added > 0) {
          saveServerQueue();
        }
        return added;
      } catch {
        return 0;
      } finally {
        if (state.radioInFlight === run) {
          state.radioInFlight = null;
        }
        if (reqToken === state.radioReqToken) {
          state.radioLoading = false;
          renderQueuePanel();
        }
      }
    })();
    state.radioInFlight = run;
    return run;
  }

  async function startPlaylistRadio(pid) {
    const { token, signal } = beginQueueFetch();
    const reqToken = ++state.radioReqToken;
    const prev = currentTrack();
    if (prev) recordRecentHeard(prev);
    state.radioLoading = true;
    state.radioInFlight = null;
    showToast('Starting playlist radio...');
    try {
      const params = new URLSearchParams();
      params.set('playlist', pid || '');
      const recentParam = buildRecentRadioParam();
      if (recentParam) params.set('recent', recentParam);
      if (state.upcomingMode && state.upcomingMode !== 'default') {
        params.set('mode', state.upcomingMode);
      }
      const d = await api(`/api/web/radio?${params.toString()}`, { signal });
      if (token !== state.queueActionToken || reqToken !== state.radioReqToken) return;
      state.radioLoading = false;
      const tracks = cacheTracks(d.tracks || []);
      if (tracks.length) {
        const idx = setQueueWithShuffle(tracks, 0, false);
        state.radioPlaylistId = pid || null;
        playQueueAt(idx);
      } else {
        showToast('No radio tracks found for this playlist yet');
      }
    } catch (e) {
      if (reqToken === state.radioReqToken) {
        state.radioLoading = false;
      }
      if (e && e.name === 'AbortError') return;
      showToast(e.message);
    }
  }

  async function startRadioForId(seedId) {
    const { token, signal } = beginQueueFetch();
    const seed = state.trackMap.get(seedId);
    const reqToken = ++state.radioReqToken;
    const cur = currentTrack();
    const keepingCurrent = !!(cur && cur.id === seedId);
    if (cur) recordRecentHeard(cur);
    if (seed) recordRecentHeard(seed);
    state.radioPlaylistId = null;
    state.radioLoading = true;
    state.radioInFlight = null;
    if (keepingCurrent) renderQueuePanel();
    showToast('Starting track radio...');
    try {
      const params = new URLSearchParams();
      params.set('seed', seedId || '');
      const recentParam = buildRecentRadioParam(seedId);
      if (recentParam) params.set('recent', recentParam);
      if (state.upcomingMode && state.upcomingMode !== 'default') {
        params.set('mode', state.upcomingMode);
      }
      const d = await api(`/api/web/radio?${params.toString()}`, { signal });
      if (token !== state.queueActionToken || reqToken !== state.radioReqToken) return;
      state.radioLoading = false;
      const recs = cacheTracks(d.tracks || []);
      const filtered = recs.filter((r) => r.id !== seedId);
      if (keepingCurrent && currentTrack()?.id === seedId) {
        const unshuffled = [cur, ...filtered];
        const upcoming = state.shuffle ? shuffleArray(filtered) : filtered;
        state.unshuffledQueue = state.shuffle ? unshuffled : null;
        state.queue = [cur, ...upcoming];
        state.queueIndex = 0;
        renderQueuePanel();
        saveServerQueue();
        scheduleWarmNextTrack();
        return;
      }
      const queue = seed ? [seed, ...filtered] : filtered;
      if (queue.length) {
        const idx = setQueueWithShuffle(queue, 0, false);
        playQueueAt(idx);
      } else {
        renderQueuePanel();
      }
    } catch (e) {
      if (reqToken === state.radioReqToken) {
        state.radioLoading = false;
        renderQueuePanel();
      }
      if (e && e.name === 'AbortError') return;
      showToast(e.message);
    }
  }

  async function setUpcomingMode(mode) {
    const cleanMode = ['default', 'familiar', 'discover'].includes(mode) ? mode : 'default';
    if (cleanMode === state.upcomingMode) return;
    state.upcomingMode = cleanMode;
    try {
      localStorage.setItem('beatlab_upcoming_mode', cleanMode);
    } catch {}
    haptic('select');
    const cur = currentTrack();
    if (!cur && state.queue.length === 0) {
      renderQueuePanel();
      return;
    }
    const reqToken = ++state.radioReqToken;
    state.radioLoading = true;
    renderQueuePanel();
    const run = (async () => {
      try {
        const params = new URLSearchParams();
        if (state.radioPlaylistId) {
          params.set('playlist', state.radioPlaylistId);
        } else if (cur?.id) {
          params.set('seed', cur.id);
        }
        params.set('mode', cleanMode);
        const recentParam = buildRecentRadioParam(cur?.id || '');
        if (recentParam) params.set('recent', recentParam);
        const d = await api(`/api/web/radio?${params.toString()}`);
        if (reqToken !== state.radioReqToken) return;
        const recs = cacheTracks(d.tracks || []);
        const nowCur = currentTrack();
        const curId = nowCur?.id;
        const filtered = curId ? recs.filter((r) => r.id !== curId) : recs;
        if (!filtered.length) {
          showToast('No matching upcoming tracks found');
          return;
        }
        const upcoming = state.shuffle ? shuffleArray(filtered) : filtered;
        const keepLen = nowCur ? state.queueIndex + 1 : 0;
        if (Array.isArray(state.unshuffledQueue) && nowCur) {
          const curUnshuffledIdx = state.unshuffledQueue.findIndex((t) => t.id === nowCur.id);
          const baseUnshuffled =
            curUnshuffledIdx >= 0 ? state.unshuffledQueue.slice(0, curUnshuffledIdx + 1) : [nowCur];
          state.unshuffledQueue = baseUnshuffled.concat(filtered);
        }
        state.queue = state.queue.slice(0, keepLen).concat(upcoming);
        saveServerQueue();
        scheduleWarmNextTrack();
      } catch (e) {
        if (reqToken === state.radioReqToken && e && e.name !== 'AbortError') {
          showToast(e.message);
        }
      } finally {
        if (state.radioInFlight === run) {
          state.radioInFlight = null;
        }
        if (reqToken === state.radioReqToken) {
          state.radioLoading = false;
          renderQueuePanel();
        }
      }
    })();
    state.radioInFlight = run;
    return run;
  }

  function togglePlay() {
    const cur = currentTrack();
    if (!cur) return;
    haptic('medium');
    if (!audio.getAttribute('src')) {
      audio.src = buildStreamUrl(cur);
    }
    if (audio.paused) {
      audio.play().catch(() => {
        updatePlayButton(false);
      });
    } else {
      audio.pause();
    }
  }

  function prevTrack() {
    if (!state.queue.length) return;
    haptic('medium');
    if (audio.currentTime > 4 || (state.queueIndex === 0 && state.repeat !== 'all')) {
      audio.currentTime = 0;
      if (audio.paused) audio.play().catch(() => {});
      return;
    }
    const prev = state.queueIndex > 0 ? state.queueIndex - 1 : state.queue.length - 1;
    playQueueAt(prev);
  }

  async function nextTrack(manual = false) {
    if (manual) haptic('medium');
    const cur = currentTrack();
    if (
      manual &&
      cur &&
      cur.id === state.lastCommittedTrackId &&
      audio.currentTime < 25 &&
      state.lastScrobbleCompleteId !== cur.id
    ) {
      state.lastCommittedTrackId = '';
      sendTrackEvent(cur.id, 'skip', 1);
    }
    if (!state.queue.length) return;

    if (state.queueIndex + 1 < state.queue.length) {
      const next = state.queueIndex + 1;
      if (state.autoplay && !state.radioLoading && state.queue.length - next <= 2 && cur) {
        appendRadioTracks(cur.id);
      }
      return playQueueAt(next);
    }
    if (state.repeat === 'all') {
      if (state.shuffle && state.queue.length > 1) {
        const reshuffled = shuffleArray(state.queue);
        if (cur && reshuffled[0].id === cur.id) {
          const swapIdx = 1 + Math.floor(Math.random() * (reshuffled.length - 1));
          [reshuffled[0], reshuffled[swapIdx]] = [reshuffled[swapIdx], reshuffled[0]];
        }
        state.queue = reshuffled;
      }
      return playQueueAt(0);
    }
    if (state.autoplay && cur) {
      if (state.radioLoading || state.radioInFlight) return;
      const idxBefore = state.queueIndex;
      const added = await appendRadioTracks(cur.id);
      if (added > 0 && state.queueIndex === idxBefore && state.queueIndex + 1 < state.queue.length) {
        return playQueueAt(state.queueIndex + 1);
      }
      return;
    }
    updatePlayButton(false);
  }

  function wireAudioEvents() {
    audio.addEventListener('play', () => {
      updatePlayButton(true);
      setMediaPlaybackState('playing');
    });
    audio.addEventListener('pause', () => {
      updatePlayButton(false);
      setMediaPlaybackState(audio.ended ? 'none' : 'paused');
      if (!audio.ended && state.queue.length) saveServerQueue();
    });
    audio.addEventListener('loadedmetadata', () => {
      if (state.pendingSeekSec > 0) {
        const target = state.pendingSeekSec;
        state.pendingSeekSec = 0;
        const maxDur = Number.isFinite(audio.duration) && audio.duration > 1 ? audio.duration - 1 : target;
        try {
          audio.currentTime = Math.max(0, Math.min(target, maxDur));
        } catch {}
      }
    });
    audio.addEventListener('error', () => {
      if (state.playCommitTimer) return;
      const cur = currentTrack();
      if (!cur || !audio.error || !audio.getAttribute('src')) return;
      const errToken = state.activeAudioToken;
      const code = audio.error.code;
      if (
        (code === 3 || code === 4) &&
        state.compatAttemptedId !== cur.id &&
        !audio.src.includes('compat=1')
      ) {
        state.compatAttemptedId = cur.id;
        const pos = audio.currentTime || 0;
        audio.src = buildStreamUrl(cur, true, pos);
        audio.play().catch(() => {
          if (errToken !== state.activeAudioToken) return;
          updatePlayButton(false);
        });
        return;
      }
      updatePlayButton(false);
      checkServerHealth();
      showToast(`Unable to play "${cur.title}" right now`);
    });
    audio.addEventListener('ended', () => {
      setMediaPlaybackState('none');
      if (state.playCommitTimer) return;
      const cur = currentTrack();
      if (state.repeat === 'one') {
        audio.currentTime = 0;
        audio.play().catch(() => {});
        return;
      }
      if (cur && state.lastScrobbleCompleteId !== cur.id) {
        state.lastScrobbleCompleteId = cur.id;
        sendTrackEvent(cur.id, 'complete', 1);
      }
      nextTrack(false);
    });

    audio.addEventListener('timeupdate', () => {
      if (state.seeking) return;
      const cur = currentTrack();
      const total = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : cur ? cur.duration : 0;
      const pos = audio.currentTime || 0;
      const elapsedStr = fmtTime(pos);
      $('pb-elapsed').textContent = elapsedStr;
      if ($('fs-elapsed')) $('fs-elapsed').textContent = elapsedStr;
      if (total > 0) {
        const durStr = fmtTime(total);
        const pct = Math.min(100, (pos / total) * 100);
        $('pb-duration').textContent = durStr;
        $('pb-seek').value = String(pct);
        if ($('fs-duration')) $('fs-duration').textContent = durStr;
        if ($('fs-seek')) $('fs-seek').value = String(pct);
        if ($('pb-mini-progress-fill')) $('pb-mini-progress-fill').style.width = `${pct.toFixed(2)}%`;
      }
      if (cur && total > 20 && pos / total >= 0.75 && state.lastScrobbleCompleteId !== cur.id) {
        state.lastScrobbleCompleteId = cur.id;
        sendTrackEvent(cur.id, 'complete', 1);
      }
      syncActiveLyricLine(pos * 1000);
    });

    const wireSeekSlider = (el) => {
      if (!el) return;
      el.addEventListener('input', (e) => {
        state.seeking = true;
        const val = parseFloat(e.target.value) || 0;
        const cur = currentTrack();
        const total = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : cur ? cur.duration : 0;
        if ($('pb-seek') && e.target !== $('pb-seek')) $('pb-seek').value = String(val);
        if ($('fs-seek') && e.target !== $('fs-seek')) $('fs-seek').value = String(val);
        if (total > 0) {
          const preview = fmtTime((val / 100) * total);
          $('pb-elapsed').textContent = preview;
          if ($('fs-elapsed')) $('fs-elapsed').textContent = preview;
        }
      });
      el.addEventListener('change', (e) => {
        state.seeking = false;
        const cur = currentTrack();
        const total = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : cur ? cur.duration : 0;
        if (total > 0) {
          audio.currentTime = (parseFloat(e.target.value) / 100) * total;
          haptic('tap');
          if (state.queue.length) saveServerQueue();
        }
      });
    };
    wireSeekSlider($('pb-seek'));
    wireSeekSlider($('fs-seek'));

    $('pb-volume').addEventListener('input', (e) => {
      state.volume = parseFloat(e.target.value);
      state.muted = state.volume === 0;
      applyOutputVolume();
      localStorage.setItem('beatlab_vol', String(state.volume));
      updateVolumeIcon();
    });
  }

  // ---------------------------------------------------------------------------
  // Queue Panel & Synced Lyrics
  // ---------------------------------------------------------------------------
  function renderQueuePanel() {
    const cur = currentTrack();
    const nowHtml = cur
      ? `<div class="queue-section-label">Now Playing</div>${renderQItem(cur, state.queueIndex, true)}`
      : `<p class="muted-center">Queue is empty</p>`;

    const upcoming = state.queue.slice(state.queueIndex + 1);
    const modeTabsHtml =
      cur || upcoming.length || state.radioLoading
        ? `<div class="upcoming-mode-tabs" role="tablist" aria-label="Upcoming mode">
            <button type="button" role="tab" aria-selected="${state.upcomingMode === 'default' ? 'true' : 'false'}" class="upcoming-mode-tab ${state.upcomingMode === 'default' ? 'active' : ''}" onclick="App.setUpcomingMode('default')">Default</button>
            <button type="button" role="tab" aria-selected="${state.upcomingMode === 'familiar' ? 'true' : 'false'}" class="upcoming-mode-tab ${state.upcomingMode === 'familiar' ? 'active' : ''}" onclick="App.setUpcomingMode('familiar')">Familiar</button>
            <button type="button" role="tab" aria-selected="${state.upcomingMode === 'discover' ? 'true' : 'false'}" class="upcoming-mode-tab ${state.upcomingMode === 'discover' ? 'active' : ''}" onclick="App.setUpcomingMode('discover')">Discover</button>
          </div>`
        : '';
    const upNextHtml =
      modeTabsHtml +
      (upcoming.length
        ? `<div class="queue-section-label">Next Up (${upcoming.length})${state.radioLoading ? ' · Updating...' : ''}</div>` +
          upcoming.map((t, i) => renderQItem(t, state.queueIndex + 1 + i, false)).join('')
        : state.radioLoading
          ? `<div class="queue-section-label">Next Up</div><p class="muted-center">Loading upcoming tracks...</p>`
          : '');

    $('queue-now-playing').innerHTML = nowHtml;
    $('queue-up-next').innerHTML = upNextHtml;
    if ($('fs-queue-now-playing')) $('fs-queue-now-playing').innerHTML = nowHtml;
    if ($('fs-queue-up-next')) $('fs-queue-up-next').innerHTML = upNextHtml;
  }

  function renderQItem(t, idx, active) {
    return `<div class="q-item ${active ? 'active' : ''}" data-tid="${esc(t.id)}" onclick="App.playQueueAt(${idx})">
      <img src="${esc(t.coverUrl)}" alt="" />
      <div class="q-item-info">
        <div class="q-item-title">${esc(t.title)}</div>
        <div class="q-item-sub">${esc(t.artist)}</div>
      </div>
      ${!active ? `<button class="icon-btn" title="Remove from queue" onclick="event.stopPropagation();App.removeFromQueue(${idx})">${ICONS.close}</button>` : ''}
    </div>`;
  }

  async function loadLyricsForTrack(id) {
    if (state.lyricsAbortController) {
      try {
        state.lyricsAbortController.abort();
      } catch {}
    }
    const ctrl = typeof AbortController !== 'undefined' ? new AbortController() : null;
    state.lyricsAbortController = ctrl;
    const reqToken = ++state.lyricsReqToken;
    $('lyrics-container').innerHTML = `<p class="muted-center">Fetching lyrics...</p>`;
    $('fs-lyrics-body').innerHTML = `<p class="muted-center">Fetching lyrics...</p>`;
    state.lyrics = { synced: false, plain: '', lines: [] };
    state.activeLyricIdx = -1;
    try {
      const data = await api(`/api/web/lyrics/${encodeURIComponent(id)}`, {
        signal: ctrl ? ctrl.signal : undefined,
      });
      if (reqToken !== state.lyricsReqToken || currentTrack()?.id !== id) return;
      state.lyrics = data;
      const lines = data.lines || [];
      const html = lines.length
        ? lines
            .map(
              (l, idx) =>
                `<div class="lyric-line ${l.start_ms != null ? 'synced' : ''}" data-lidx="${idx}" ${
                  l.start_ms != null ? `onclick="App.seekMs(${l.start_ms})"` : ''
                }>${esc(l.text || '♪')}</div>`
            )
            .join('')
        : `<p class="muted-center">No lyrics available for this track.</p>`;
      $('lyrics-container').innerHTML = html;
      $('fs-lyrics-body').innerHTML = html;
    } catch (e) {
      if ((e && e.name === 'AbortError') || reqToken !== state.lyricsReqToken) return;
      $('lyrics-container').innerHTML = `<p class="muted-center">No lyrics available.</p>`;
      $('fs-lyrics-body').innerHTML = `<p class="muted-center">No lyrics available.</p>`;
    } finally {
      if (state.lyricsAbortController === ctrl) {
        state.lyricsAbortController = null;
      }
    }
  }

  function syncActiveLyricLine(posMs) {
    if (!state.lyrics.synced || !state.lyrics.lines.length) return;
    let active = -1;
    for (let i = 0; i < state.lyrics.lines.length; i++) {
      const ms = state.lyrics.lines[i].start_ms;
      if (ms != null && ms <= posMs + 150) {
        active = i;
      }
    }
    if (active === state.activeLyricIdx) return;
    state.activeLyricIdx = active;

    for (const containerId of ['lyrics-container', 'fs-lyrics-body']) {
      const box = $(containerId);
      if (!box) continue;
      const isVisible =
        containerId === 'fs-lyrics-body'
          ? $('fullscreen-player').classList.contains('open') && state.fsTab === 'lyrics'
          : box.offsetParent !== null;
      box.querySelectorAll('.lyric-line').forEach((el, idx) => {
        el.classList.toggle('active', idx === active);
        if (idx === active && isVisible) {
          el.scrollIntoView({ behavior: 'smooth', block: 'center' });
        }
      });
    }
  }

  // ---------------------------------------------------------------------------
  // User Actions: Likes, Playlists, Context Menu, Modals
  // ---------------------------------------------------------------------------
  async function toggleTrackLike(id, btnEl) {
    const t = state.trackMap.get(id);
    if (!t || state.likeInFlight.has(id)) return;
    state.likeInFlight.add(id);
    const next = !t.isStarred;
    haptic(next ? 'success' : 'tap');
    try {
      await api('/api/web/star', {
        method: 'POST',
        body: JSON.stringify({ id, starred: next, kind: 'track' }),
      });
      t.isStarred = next;
      if (btnEl) {
        btnEl.classList.toggle('active', next);
      }
      document.querySelectorAll('.track-row').forEach((row) => {
        if (row.dataset.tid === id) {
          const b = row.querySelector('.tr-like-btn');
          if (b) b.classList.toggle('active', next);
        }
      });
      const cur = currentTrack();
      if (cur && cur.id === id) {
        $('pb-like').classList.toggle('active', next);
        if ($('fs-like')) $('fs-like').classList.toggle('active', next);
      }
      showToast(next ? 'Added to Liked Songs' : 'Removed from Liked Songs');
    } catch (e) {
      showToast(e.message);
    } finally {
      state.likeInFlight.delete(id);
    }
  }

  let trackMenuCloser = null;

  function closeTrackMenu() {
    const menu = $('context-menu');
    const backdrop = $('ctx-backdrop');
    if (menu) menu.classList.add('hidden');
    if (backdrop) backdrop.classList.add('hidden');
    if (trackMenuCloser) {
      window.removeEventListener('click', trackMenuCloser);
      trackMenuCloser = null;
    }
  }

  function positionContextMenu(menu, backdrop, event) {
    haptic('select');
    menu.classList.remove('hidden');
    if (window.innerWidth <= 780) {
      menu.style.left = '';
      menu.style.top = '';
      if (backdrop) backdrop.classList.remove('hidden');
    } else {
      if (backdrop) backdrop.classList.add('hidden');
      const maxH = Math.min(440, window.innerHeight - 24);
      const rect = menu.getBoundingClientRect();
      const menuW = Math.max(220, rect.width || 220);
      const menuH = Math.min(maxH, rect.height || 320);
      const cx = event && Number.isFinite(event.clientX) ? event.clientX : window.innerWidth / 2;
      const cy = event && Number.isFinite(event.clientY) ? event.clientY : window.innerHeight / 2;
      const x = Math.min(window.innerWidth - menuW - 10, cx);
      const y = Math.min(window.innerHeight - menuH - 10, cy);
      menu.style.left = `${Math.max(8, x)}px`;
      menu.style.top = `${Math.max(8, y)}px`;

      trackMenuCloser = (e) => {
        if (!menu.contains(e.target)) closeTrackMenu();
      };
      setTimeout(() => {
        if (trackMenuCloser) window.addEventListener('click', trackMenuCloser);
      }, 10);
    }
  }

  function openTrackMenu(event, trackId, editablePlaylistId = null, playlistPos = -1) {
    if (event && typeof event.stopPropagation === 'function') {
      event.stopPropagation();
    }
    const t = state.trackMap.get(trackId);
    if (!t) return;
    closeTrackMenu();
    const menu = $('context-menu');
    const backdrop = $('ctx-backdrop');
    const userPls = state.playlists.filter((p) => !p.isAuto);
    const tidArg = esc(JSON.stringify(trackId));
    const plActions =
      editablePlaylistId != null && playlistPos >= 0
        ? `${
            playlistPos > 0
              ? `<button class="ctx-item" onclick="App.closeTrackMenu();App.movePlaylistItem(${esc(JSON.stringify(editablePlaylistId))}, ${playlistPos}, ${playlistPos - 1})">Move Up in Playlist</button>`
              : ''
          }
          <button class="ctx-item danger" onclick="App.closeTrackMenu();App.removePlaylistItem(${esc(JSON.stringify(editablePlaylistId))}, ${playlistPos})">Remove from Playlist</button>
          <hr class="ctx-sep">`
        : '';

    menu.innerHTML = `
      <div class="ctx-sheet-head">
        <img src="${esc(t.coverUrl)}" alt="" />
        <div class="ctx-sheet-meta">
          <div class="ctx-sheet-title">${esc(t.title)}</div>
          <div class="ctx-sheet-sub">${esc(t.artist)}</div>
        </div>
        <button class="icon-btn" title="Close" onclick="App.closeTrackMenu()">${ICONS.close}</button>
      </div>
      ${plActions}
      <button class="ctx-item ctx-item-icon accent" onclick="App.closeTrackMenu();App.shareTrack(${tidArg})">
        ${ICONS.share}
        <span>Share Song Link</span>
      </button>
      <button class="ctx-item" onclick="App.closeTrackMenu();App.playNext(${tidArg})">Play Next</button>
      <button class="ctx-item" onclick="App.closeTrackMenu();App.addToQueue(${tidArg})">Add to Queue</button>
      <button class="ctx-item" onclick="App.closeTrackMenu();App.startRadioForId(${tidArg})">Start Song Radio</button>
      <button class="ctx-item" onclick="App.closeTrackMenu();App.navigate('album/${encodeURIComponent(t.albumId)}')">Go to Album</button>
      <button class="ctx-item" onclick="App.closeTrackMenu();App.navigate('artist/${encodeURIComponent(t.artistId)}')">Go to Artist</button>
      <hr class="ctx-sep">
      <button class="ctx-item ctx-item-icon danger" onclick="App.closeTrackMenu();App.markNotInterested(${tidArg})">
        ${ICONS.eyeOff}
        <span>Not Interested</span>
      </button>
      <hr class="ctx-sep">
      <button class="ctx-item" onclick="App.closeTrackMenu();App.promptAddToNewPlaylist(${tidArg})">+ New Playlist with Song</button>
      ${userPls
        .map(
          (p) =>
            `<button class="ctx-item" onclick="App.closeTrackMenu();App.addTrackToPlaylist(${esc(JSON.stringify(p.id))}, ${tidArg})">Add to: ${esc(p.name)}</button>`
        )
        .join('')}
    `;
    positionContextMenu(menu, backdrop, event);
  }

  function openEntityMenu(event, info) {
    if (event && typeof event.stopPropagation === 'function') {
      event.stopPropagation();
    }
    if (!info || !info.kind || !info.id) return;
    closeTrackMenu();
    const menu = $('context-menu');
    const backdrop = $('ctx-backdrop');
    const kindArg = esc(JSON.stringify(info.kind));
    const idArg = esc(JSON.stringify(info.id));
    const titleArg = esc(JSON.stringify(info.title || ''));
    const subArg = esc(JSON.stringify(info.sub || ''));
    const coverArg = esc(JSON.stringify(info.cover || ''));
    const shareLabel =
      info.kind === 'mix'
        ? 'Share Instant Mix'
        : info.kind === 'playlist'
          ? 'Share Playlist'
          : info.kind === 'album'
            ? 'Share Album'
            : 'Share Artist';

    let navAction = '';
    let playAction = '';
    if (info.kind === 'playlist') {
      playAction = `<button class="ctx-item" onclick="App.closeTrackMenu();App.playPlaylistNow(${idArg})">Play Playlist</button>`;
      navAction = `<button class="ctx-item" onclick="App.closeTrackMenu();App.navigate('playlist/${encodeURIComponent(info.id)}')">Open Playlist</button>`;
    } else if (info.kind === 'album') {
      playAction = `<button class="ctx-item" onclick="App.closeTrackMenu();App.playAlbumNow(${idArg})">Play Album</button>`;
      navAction = `<button class="ctx-item" onclick="App.closeTrackMenu();App.navigate('album/${encodeURIComponent(info.id)}')">Open Album</button>`;
    } else if (info.kind === 'artist') {
      navAction = `<button class="ctx-item" onclick="App.closeTrackMenu();App.navigate('artist/${encodeURIComponent(info.id)}')">Open Artist</button>`;
    } else if (info.kind === 'mix') {
      playAction = `<button class="ctx-item" onclick="App.closeTrackMenu();App.playInstantMix(false, 0)">Play Mix</button>`;
    }

    const shareAction =
      info.kind === 'mix'
        ? `App.shareCurrentInstantMix()`
        : `App.shareEntity({ kind: ${kindArg}, targetId: ${idArg}, title: ${titleArg}, subtitle: ${subArg}, coverUrl: ${coverArg} })`;

    menu.innerHTML = `
      <div class="ctx-sheet-head">
        ${info.cover ? `<img src="${esc(info.cover)}" alt="" />` : ''}
        <div class="ctx-sheet-meta">
          <div class="ctx-sheet-title">${esc(info.title || 'Share')}</div>
          <div class="ctx-sheet-sub">${esc(info.sub || '')}</div>
        </div>
        <button class="icon-btn" title="Close" onclick="App.closeTrackMenu()">${ICONS.close}</button>
      </div>
      <button class="ctx-item ctx-item-icon accent" onclick="App.closeTrackMenu();${shareAction}">
        ${ICONS.share}
        <span>${shareLabel}</span>
      </button>
      ${playAction || navAction ? '<hr class="ctx-sep">' : ''}
      ${playAction}
      ${navAction}
    `;
    positionContextMenu(menu, backdrop, event);
  }

  function wireTrackGestures() {
    let holdTimer = null;
    let startX = 0;
    let startY = 0;
    let longPressFiredAt = 0;
    let suppressNextClick = false;

    const clearHold = () => {
      if (holdTimer) {
        clearTimeout(holdTimer);
        holdTimer = null;
      }
    };

    const getRowArgs = (target) => {
      const row = target.closest('[data-tid]');
      if (!row || target.closest('input, textarea, select')) return null;
      const tid = row.dataset.tid;
      if (!tid || !state.trackMap.has(tid)) return null;
      const plId = row.dataset.plId || null;
      const plPos = row.dataset.plPos != null ? parseInt(row.dataset.plPos, 10) : -1;
      return { type: 'track', tid, plId, plPos: Number.isFinite(plPos) ? plPos : -1 };
    };

    const getEntityArgs = (target) => {
      const card = target.closest('[data-share-kind]');
      if (!card || target.closest('input, textarea, select')) return null;
      const kind = card.dataset.shareKind;
      const id = card.dataset.shareId;
      if (!kind || !id) return null;
      return {
        type: 'entity',
        kind,
        id,
        title: card.dataset.shareTitle || '',
        sub: card.dataset.shareSub || '',
        cover: card.dataset.shareCover || '',
      };
    };

    const getTargetArgs = (target) => getRowArgs(target) || getEntityArgs(target);

    const openTargetMenu = (evt, info) => {
      if (info.type === 'track') {
        openTrackMenu(evt, info.tid, info.plId, info.plPos);
      } else {
        openEntityMenu(evt, info);
      }
    };

    document.addEventListener('contextmenu', (e) => {
      const info = getTargetArgs(e.target);
      if (!info) return;
      e.preventDefault();
      if (Date.now() - longPressFiredAt < 700) return;
      openTargetMenu(e, info);
    });

    document.addEventListener('pointerdown', (e) => {
      if (e.pointerType !== 'touch' && e.pointerType !== 'pen') return;
      if (e.target.closest('button:not(.pl-link), a, input, select, textarea, .tr-actions')) return;
      const info = getTargetArgs(e.target);
      if (!info) return;
      clearHold();
      startX = e.clientX;
      startY = e.clientY;
      holdTimer = setTimeout(() => {
        holdTimer = null;
        longPressFiredAt = Date.now();
        suppressNextClick = true;
        haptic('medium');
        openTargetMenu({ clientX: startX, clientY: startY }, info);
      }, 480);
    });

    document.addEventListener('pointermove', (e) => {
      if (!holdTimer) return;
      if (Math.hypot(e.clientX - startX, e.clientY - startY) > 10) {
        clearHold();
      }
    });

    document.addEventListener('pointerup', clearHold);
    document.addEventListener('pointercancel', clearHold);
    document.addEventListener('scroll', clearHold, true);

    document.addEventListener(
      'click',
      (e) => {
        if (suppressNextClick) {
          suppressNextClick = false;
          if (Date.now() - longPressFiredAt < 1200) {
            e.preventDefault();
            e.stopPropagation();
            return;
          }
        }
        if (e.target.closest('.tr-like-btn, .tr-more-btn, input, select, textarea')) return;
        if (e.target.closest('.track-row, .q-item, .qp-tile, .media-card, .mood-tile, .top-result-card, .tab-chip, .pill-btn')) {
          haptic('tap');
        }
      },
      true
    );
  }

  async function shareEntity({ kind, targetId, title = '', subtitle = '', trackIds = [], coverUrl = '' }) {
    if (state.shareInFlight) return;
    haptic('success');
    state.shareInFlight = true;
    try {
      const res = await api('/api/web/share', {
        method: 'POST',
        body: JSON.stringify({
          kind,
          targetId,
          title,
          subtitle,
          trackIds,
        }),
      });
      const fullUrl = res.url || `${location.origin}${res.path}`;
      state.currentShareUrl = fullUrl;
      let autoCopied = false;
      try {
        if (navigator.clipboard && navigator.clipboard.writeText) {
          await navigator.clipboard.writeText(fullUrl);
          autoCopied = true;
        }
      } catch {}

      const art = coverUrl || `/s/${encodeURIComponent(res.id)}/cover`;
      const kindBadge =
        res.kind === 'track'
          ? 'SONG SHARE'
          : res.kind === 'mix'
            ? 'INSTANT MIX SHARE'
            : res.kind === 'playlist'
              ? 'PLAYLIST SHARE'
              : res.kind === 'album'
                ? 'ALBUM SHARE'
                : 'ARTIST SHARE';

      const nativeShareBtn =
        typeof navigator.share === 'function'
          ? `<button class="btn-secondary" onclick="App.nativeShareCurrent()">Share via Device</button>`
          : '';

      openModal(`
        <div class="modal-head">
          <h3>Share ${esc(res.kind === 'track' ? 'Song' : res.kind === 'mix' ? 'Instant Mix' : res.kind.charAt(0).toUpperCase() + res.kind.slice(1))}</h3>
          <button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button>
        </div>
        <div class="share-preview-card">
          <img src="${esc(art)}" alt="" />
          <div class="share-preview-meta">
            <span class="share-kind-pill">${kindBadge} · DISCORD EMBED READY</span>
            <div class="share-preview-title">${esc(res.title || title)}</div>
            <div class="share-preview-sub">${esc(res.subtitle || subtitle)}${res.trackCount > 1 ? ` · ${res.trackCount} tracks` : ''}</div>
          </div>
        </div>
        <div class="share-url-row">
          <input id="share-url-input" type="text" readonly value="${esc(fullUrl)}" onclick="this.select()" />
          <button class="btn-primary" onclick="App.copyText(document.getElementById('share-url-input').value, 'Share link copied!')">Copy Link</button>
        </div>
        <div class="share-modal-note">
          ${autoCopied ? '✓ Copied to clipboard! ' : ''}Paste into Discord for a rich playable embed, or open in any browser — anyone can listen without signing in.
        </div>
        <div class="share-modal-actions">
          <a class="btn-secondary" href="${esc(fullUrl)}" target="_blank" rel="noopener">Open Public Player ↗</a>
          ${nativeShareBtn}
        </div>
      `);
    } catch (e) {
      showToast(e.message || 'Could not create share link');
    } finally {
      state.shareInFlight = false;
    }
  }

  async function markNotInterested(trackId) {
    if (!trackId) return;
    haptic('warning');
    try {
      await api('/api/web/not-interested', {
        method: 'POST',
        body: JSON.stringify({ trackId }),
      });
      if (state.searchState) {
        state.searchState.tracks = (state.searchState.tracks || []).filter((t) => t.id !== trackId);
        if (state.searchState.instantMix && Array.isArray(state.searchState.instantMix.tracks)) {
          state.searchState.instantMix.tracks = state.searchState.instantMix.tracks.filter(
            (t) => t.id !== trackId
          );
        }
        renderSearchView();
      } else if (state.activeInstantMix && state.activeInstantMix.mix && document.querySelector('#main-view .instant-mix-card')) {
        state.activeInstantMix.mix.tracks = (state.activeInstantMix.mix.tracks || []).filter(
          (t) => t.id !== trackId
        );
        const { mix, query, refinements, expanded } = state.activeInstantMix;
        const card = document.querySelector('#main-view .instant-mix-card');
        if (card) {
          card.outerHTML = renderInstantMixCard(mix, refinements, query, expanded);
        }
      }
      document.querySelectorAll('#main-view [data-tid]').forEach((el) => {
        if (el.dataset.tid === trackId && !el.classList.contains('q-item')) {
          el.remove();
        }
      });

      const cur = currentTrack();
      const wasCur = !!(cur && cur.id === trackId);
      if (Array.isArray(state.unshuffledQueue)) {
        state.unshuffledQueue = state.unshuffledQueue.filter((item) => item.id !== trackId);
      }
      const oldIdx = state.queueIndex;
      const newQueue = [];
      let newIdx = -1;
      state.queue.forEach((item, idx) => {
        if (item.id === trackId) return;
        if (idx <= oldIdx) newIdx = newQueue.length;
        newQueue.push(item);
      });

      if (wasCur) {
        const nextIdx = newIdx + 1;
        state.queue = newQueue;
        if (newQueue.length > 0 && nextIdx < newQueue.length) {
          playQueueAt(nextIdx);
        } else if (state.autoplay && newQueue.length > 0) {
          state.queueIndex = newQueue.length - 1;
          await nextTrack(false);
        } else {
          cancelPendingPlaybackWork();
          state.activeAudioToken++;
          state.queueIndex = -1;
          audio.pause();
          audio.removeAttribute('src');
          renderQueuePanel();
          saveServerQueue();
        }
      } else if (newQueue.length !== state.queue.length) {
        state.queue = newQueue;
        state.queueIndex = newIdx;
        renderQueuePanel();
        saveServerQueue();
      }

      showToast('Marked as Not Interested — song hidden');
    } catch (e) {
      showToast(e.message);
    }
  }

  let pendingConfirmResolve = null;

  function settlePendingConfirm(result) {
    if (!pendingConfirmResolve) return;
    const resolve = pendingConfirmResolve;
    pendingConfirmResolve = null;
    resolve(result);
  }

  function openModal(html, extraClass = '') {
    settlePendingConfirm(false);
    haptic('tap');
    const box = $('modal-box');
    box.className = extraClass ? `modal-box ${extraClass}` : 'modal-box';
    box.innerHTML = html;
    $('modal-backdrop').classList.remove('hidden');
  }

  function closeModal(event) {
    if (!event || event.target === $('modal-backdrop')) {
      $('modal-backdrop').classList.add('hidden');
      settlePendingConfirm(false);
    }
  }

  function confirmModal({
    title = 'Confirm',
    message,
    confirmLabel = 'Confirm',
    cancelLabel = 'Cancel',
    danger = false,
  }) {
    return new Promise((resolve) => {
      openModal(`
        <div class="modal-head">
          <h3>${esc(title)}</h3>
          <button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button>
        </div>
        <p class="confirm-modal-message">${esc(message)}</p>
        <div class="confirm-modal-actions">
          <button type="button" class="btn-secondary" id="confirm-modal-cancel">${esc(cancelLabel)}</button>
          <button type="button" class="${danger ? 'btn-secondary danger-btn' : 'btn-primary'}" id="confirm-modal-ok">${esc(confirmLabel)}</button>
        </div>
      `);
      pendingConfirmResolve = resolve;
      const cancelBtn = $('confirm-modal-cancel');
      const okBtn = $('confirm-modal-ok');
      cancelBtn?.addEventListener('click', () => {
        settlePendingConfirm(false);
        closeModal();
      });
      okBtn?.addEventListener('click', () => {
        settlePendingConfirm(true);
        closeModal();
      });
      cancelBtn?.focus();
    });
  }

  const SESSION_ICON_WEB = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>';
  const SESSION_ICON_APP = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="2" width="14" height="20" rx="2"/><line x1="12" y1="18" x2="12.01" y2="18"/></svg>';

  function formatSessionDate(ts) {
    try {
      return new Date(ts * 1000).toLocaleString(undefined, {
        month: 'short', day: 'numeric', year: 'numeric',
        hour: 'numeric', minute: '2-digit',
      });
    } catch {
      return 'Unknown';
    }
  }

  function timeAgo(ts) {
    if (!ts) return 'unknown';
    const diff = Date.now() - ts * 1000;
    if (diff < 60000) return 'just now';
    if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`;
    if (diff < 86400000) return `${Math.floor(diff / 3600000)}h ago`;
    if (diff < 2592000000) return `${Math.floor(diff / 86400000)}d ago`;
    return formatSessionDate(ts);
  }

  function parseUserAgent(ua) {
    if (!ua) return 'Unknown device';
    let browser = '';
    if (/Edg\//i.test(ua)) browser = 'Edge';
    else if (/OPR\/|Opera/i.test(ua)) browser = 'Opera';
    else if (/CriOS\//i.test(ua)) browser = 'Chrome';
    else if (/FxiOS\//i.test(ua)) browser = 'Firefox';
    else if (/Firefox\//i.test(ua)) browser = 'Firefox';
    else if (/Chrome\/|Chromium\//i.test(ua)) browser = 'Chrome';
    else if (/Safari\//i.test(ua)) browser = 'Safari';
    if (!browser) {
      const token = ua.split(/[\s/]/)[0];
      browser = token ? token.slice(0, 32) : 'Unknown device';
    }

    let os = '';
    if (/Windows NT 10|Windows NT 11|Windows/i.test(ua)) os = 'Windows';
    else if (/iPad/i.test(ua)) os = 'iPadOS';
    else if (/iPhone|iPod/i.test(ua)) os = 'iOS';
    else if (/Android/i.test(ua)) os = 'Android';
    else if (/CrOS/i.test(ua)) os = 'ChromeOS';
    else if (/Mac OS X|Macintosh/i.test(ua)) os = 'macOS';
    else if (/Linux/i.test(ua)) os = 'Linux';
    return os ? `${browser} on ${os}` : browser;
  }

  function sessionInfo(s) {
    const client = (s.client || '').trim();
    const ua = s.userAgent || '';
    const mobile = /iPhone|iPad|Android|Mobile/i.test(ua);
    if (s.kind === 'subsonic') {
      return {
        title: client || 'Subsonic client',
        kindLabel: 'Subsonic / Navidrome client',
        icon: SESSION_ICON_APP,
      };
    }
    if (s.kind === 'nd') {
      return {
        title: client || parseUserAgent(ua),
        kindLabel: 'Navidrome login',
        icon: mobile ? SESSION_ICON_APP : SESSION_ICON_WEB,
      };
    }
    return {
      title: parseUserAgent(ua),
      kindLabel: 'Web browser session',
      icon: mobile ? SESSION_ICON_APP : SESSION_ICON_WEB,
    };
  }

  function renderSettingsModalBody() {
    const body = $('settings-body');
    if (!body) return;
    const tab = state.settingsTab || 'general';
    const srcData = state.settingsData.sources || { sources: [], totalTracks: 0, cacheMb: 0, ffmpeg: false };
    const hiddenData = state.settingsData.hidden || { count: 0, tracks: [] };
    const confirmReset = !!state.settingsData.confirmResetHidden;

    document.querySelectorAll('.settings-nav-btn').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.stab === tab);
    });

    if (tab === 'general') {
      const standalone = isStandalonePwa();
      const installControl = standalone
        ? `<span class="pwa-status-pill">Installed</span>`
        : state.deferredInstallPrompt
          ? `<button class="btn-primary" onclick="App.installPwa()">Install App</button>`
          : isIosBrowser()
            ? `<span class="settings-row-desc" style="text-align:right;max-width:220px">In Safari, tap Share then &ldquo;Add to Home Screen&rdquo;</span>`
            : !window.isSecureContext
              ? `<span class="settings-row-desc" style="text-align:right;max-width:220px">Requires HTTPS or localhost</span>`
              : `<span class="settings-row-desc" style="text-align:right;max-width:220px">Available via browser menu</span>`;
      body.innerHTML = `
        <div class="settings-section">
          <div class="settings-section-title">Appearance</div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">Theme</div>
              <div class="settings-row-desc">Switch between flat dark mode and clean light mode</div>
            </div>
            <div class="theme-toggle-group" role="group" aria-label="Color theme">
              <button type="button" class="theme-toggle-opt${state.theme !== 'light' ? ' active' : ''}" onclick="App.setTheme('dark')">Dark</button>
              <button type="button" class="theme-toggle-opt${state.theme === 'light' ? ' active' : ''}" onclick="App.setTheme('light')">Light</button>
            </div>
          </div>
        </div>

        <div class="settings-section">
          <div class="settings-section-title">App &amp; Installation</div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">Install Beatlab App</div>
              <div class="settings-row-desc">${standalone ? 'Running as an installed standalone Progressive Web App' : 'Install Beatlab on your desktop or home screen for standalone playback'}</div>
            </div>
            ${installControl}
          </div>
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Playback &amp; Audio</div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">Autoplay Radio</div>
              <div class="settings-row-desc">Automatically append related songs when your queue ends</div>
            </div>
            <label class="switch-toggle">
              <input type="checkbox" ${state.autoplay ? 'checked' : ''} onchange="App.toggleAutoplay(this.checked)" />
              <span class="switch-track"></span>
            </label>
          </div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">Haptic Feedback</div>
              <div class="settings-row-desc">Subtle tactile vibrations on mobile taps, swipes, and controls (when supported by device)</div>
            </div>
            <label class="switch-toggle">
              <input type="checkbox" ${state.hapticsEnabled ? 'checked' : ''} onchange="App.toggleHaptics(this.checked)" />
              <span class="switch-track"></span>
            </label>
          </div>
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Music Sources &amp; Engine</div>
          <div class="settings-metrics">
            <div class="settings-metric"><span>Catalog</span><strong>${srcData.totalTracks} tracks</strong></div>
            <div class="settings-metric"><span>Audio Cache</span><strong>${srcData.cacheMb} MB</strong></div>
            <div class="settings-metric"><span>FFmpeg</span><strong>${srcData.ffmpeg ? 'Ready' : 'Off'}</strong></div>
          </div>
        </div>
      `;
      return;
    }

    if (tab === 'hidden') {
      const count = hiddenData.count || 0;
      const tracks = hiddenData.tracks || [];
      body.innerHTML = `
        <div class="settings-section">
          <div class="settings-section-title">Hidden Songs &amp; Not Interested</div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">Hide Data (${count} ${count === 1 ? 'song' : 'songs'})</div>
              <div class="settings-row-desc">Songs marked Not Interested are excluded from mixes, recommendations, search, charts, and autoplay radio.</div>
            </div>
          </div>
          ${
            confirmReset
              ? `<div class="settings-confirm-bar">
                  <span>Restore all ${count} hidden ${count === 1 ? 'song' : 'songs'} and clear Not Interested history?</span>
                  <div class="settings-confirm-actions">
                    <button class="btn-secondary danger-btn" onclick="App.confirmResetHidden()">Confirm Reset</button>
                    <button class="mini-btn ghost" onclick="App.cancelResetHidden()">Cancel</button>
                  </div>
                </div>`
              : `<div class="settings-actions-row">
                  <button class="btn-secondary danger-btn" ${count === 0 ? 'disabled' : ''} onclick="App.promptResetHidden()">Reset Hide Data</button>
                </div>`
          }
          ${
            tracks.length > 0
              ? `<div class="hidden-track-list">
                  ${tracks
                    .map(
                      (t) => `
                    <div class="hidden-track-row">
                      <img src="${esc(t.coverUrl)}" alt="" />
                      <div class="hidden-track-meta">
                        <div class="hidden-track-title">${esc(t.title)}</div>
                        <div class="hidden-track-sub">${esc(t.artist)}</div>
                      </div>
                      <button class="mini-btn" onclick="App.unhideSong(${esc(JSON.stringify(t.id))})">Unhide</button>
                    </div>`
                    )
                    .join('')}
                </div>`
              : `<div class="settings-empty">No hidden songs yet. Right-click or long-hold any song and select <strong>Not Interested</strong>.</div>`
          }
        </div>
      `;
      return;
    }

    if (tab === 'sessions') {
      const data = state.settingsData.sessions || { count: 0, sessions: [] };
      const sessions = data.sessions || [];
      const confirmAll = !!state.settingsData.confirmRevokeAll;
      body.innerHTML = `
        <div class="settings-section">
          <div class="settings-section-title">Active Sessions</div>
          <div class="settings-row-desc" style="margin-bottom:12px">Every device and app currently connected to your account &mdash; web logins, Navidrome sign-ins, and Subsonic clients. Revoke anything you don't recognize.</div>
          ${
            sessions.length > 0
              ? `<div class="session-list">
                  ${sessions
                    .map((s) => {
                      const info = sessionInfo(s);
                      const isCurrent = s.current || s.id === state.token;
                      const revoked = !!s.revoked;
                      const ua = s.userAgent || '';
                      return `
                    <div class="session-card${isCurrent ? ' current' : ''}${revoked ? ' revoked' : ''}">
                      <span class="session-icon">${info.icon}</span>
                      <div class="session-meta">
                        <div class="session-title">
                          ${esc(info.title)}
                          ${isCurrent ? '<span class="session-current-pill">This device</span>' : ''}
                          ${revoked ? '<span class="session-revoked-pill">Revoked</span>' : ''}
                        </div>
                        <div class="session-sub">${esc(info.kindLabel)} &middot; Signed in ${esc(formatSessionDate(s.created))}</div>
                        <div class="session-sub">Last active ${esc(timeAgo(s.lastSeen || s.created))}</div>
                        ${s.ip ? `<div class="session-sub">IP ${esc(s.ip)}</div>` : ''}
                        ${ua ? `<div class="session-ua" title="${esc(ua)}">${esc(ua)}</div>` : ''}
                      </div>
                      ${
                        revoked
                          ? `<button class="mini-btn session-revoke" title="Allow this client again" onclick="App.allowSession(${esc(JSON.stringify(s.id))})">Allow</button>`
                          : `<button class="mini-btn danger-btn session-revoke" title="Revoke this session" onclick="App.revokeSession(${esc(JSON.stringify(s.id))})">Revoke</button>`
                      }
                    </div>`;
                    })
                    .join('')}
                </div>`
              : `<div class="settings-empty">No active sessions found.</div>`
          }
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Sign Out Everywhere</div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">Log out of all sessions</div>
              <div class="settings-row-desc">Revoke every active session, including this one. You'll need to sign in again on all your devices.</div>
            </div>
          </div>
          ${
            confirmAll
              ? `<div class="settings-confirm-bar">
                  <span>Are you sure? Every device will be signed out immediately and you'll need to sign in again.</span>
                  <div class="settings-confirm-actions">
                    <button class="btn-secondary danger-btn" onclick="App.confirmRevokeAll()">Yes, sign out everywhere</button>
                    <button class="mini-btn ghost" onclick="App.cancelRevokeAll()">Cancel</button>
                  </div>
                </div>`
              : `<div class="settings-actions-row">
                  <button class="btn-secondary danger-btn" onclick="App.promptRevokeAll()">Sign out of every session</button>
                </div>`
          }
        </div>
      `;
      return;
    }

    if (tab === 'apps') {
      const clients = ['Symfonium', 'Feishin', 'Supersonic', 'Tempo', 'DSub', 'Amperfy'];
      body.innerHTML = `
        <div class="settings-section">
          <div class="settings-section-title">Subsonic &amp; Navidrome Connect</div>
          <div class="kv-card">
            <div class="kv-label">SERVER URL</div>
            <div class="kv-row">
              <span class="kv-val">${esc(location.origin)}</span>
              <button class="mini-btn" onclick="App.copyText(${esc(JSON.stringify(location.origin))}, 'Server URL copied')">Copy</button>
            </div>
          </div>
          <div class="kv-card">
            <div class="kv-label">USERNAME</div>
            <div class="kv-val">${esc(state.user.username)}</div>
          </div>
          <div class="settings-row-desc" style="margin-bottom:12px">Music clients can no longer sign in with your account password. When your client asks for a password, paste the API key below instead &mdash; or use its <code>apiKey</code> field. Regenerating the key signs every connected client out until you update it.</div>
          <div class="kv-card">
            <div class="kv-label">OPENSUBSONIC / NAVIDROME API KEY</div>
            <div class="kv-row">
              <span id="modal-apikey" class="kv-val">${esc(state.user.apiKey || '')}</span>
              <div style="display:flex;gap:6px;flex-shrink:0">
                <button class="mini-btn" onclick="App.copyText(document.getElementById('modal-apikey').textContent, 'API key copied')">Copy</button>
                <button class="mini-btn" onclick="App.regenApiKey()">Regenerate</button>
              </div>
            </div>
            ${state.user && state.user.twoFactorEnabled ? '<input id="apikey-code" type="text" inputmode="numeric" autocomplete="one-time-code" placeholder="2FA or recovery code to regenerate" style="width:100%;margin-top:8px" />' : ''}
          </div>
          <div class="settings-clients">
            <div class="kv-label">COMPATIBLE CLIENTS</div>
            <div class="settings-client-chips">
              ${clients.map((c) => `<span class="settings-client-chip">${c}</span>`).join('')}
            </div>
          </div>
        </div>
      `;
      return;
    }

    if (tab === 'security') {
      const sec = state.settingsData.security || {
        twoFactorEnabled: false, totpEnabled: false, passkeys: [], events: [],
        recoveryTotal: 0, recoveryUsed: 0, passkeysAvailable: false,
      };
      const setup = state.settingsData.totpSetup;
      const codes = state.settingsData.recoveryCodes;
      const confirmDisable = !!state.settingsData.confirmDisable2fa;
      const confirmDelete = !!state.settingsData.confirmDelete;
      const methods = [
        sec.totpEnabled ? 'Authenticator app' : '',
        sec.passkeys.length ? 'Passkeys' : '',
      ].filter(Boolean).join(' + ') || 'None';
      const passkeyRows = sec.passkeys.length
        ? sec.passkeys.map((p) => `
            <div class="hidden-track-row">
              <span class="session-icon">${ICONS.key || ICONS.lock || ''}</span>
              <div class="hidden-track-meta">
                <div class="hidden-track-title">${esc(p.label || 'Passkey')}</div>
                <div class="hidden-track-sub">Added ${esc(timeAgo(p.created))}${p.lastUsed ? ` &middot; Last used ${esc(timeAgo(p.lastUsed))}` : ''}</div>
              </div>
              <button class="mini-btn danger-btn" onclick="App.removePasskey(${esc(JSON.stringify(p.id))})">Remove</button>
            </div>`).join('')
        : `<div class="settings-empty">No passkeys registered yet.</div>`;
      const eventRows = sec.events.length
        ? sec.events.map((e) => `
            <div class="hidden-track-row">
              <div class="hidden-track-meta">
                <div class="hidden-track-title">${esc(e.detail || e.kind)}</div>
                <div class="hidden-track-sub">${esc(timeAgo(e.created))}${e.ip ? ` &middot; ${esc(e.ip)}` : ''}</div>
              </div>
            </div>`).join('')
        : `<div class="settings-empty">No security events recorded yet.</div>`;
      body.innerHTML = `
        <div class="settings-section">
          <div class="settings-section-title">Two-Factor Authentication</div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">Account protection</div>
              <div class="settings-row-desc">${sec.twoFactorEnabled ? `Active: ${esc(methods)}` : 'Add an authenticator app or passkey to protect your account'}</div>
            </div>
            <span class="pwa-status-pill">${sec.twoFactorEnabled ? 'Enabled' : 'Off'}</span>
          </div>
          ${
            sec.totpEnabled
              ? `<div class="settings-row"><div class="settings-row-info"><div class="settings-row-label">Authenticator app</div><div class="settings-row-desc">Codes are verified with your app. Recovery codes can also sign you in.</div></div><span class="pwa-status-pill">Enabled</span></div>`
              : setup
                ? `<div class="totp-setup-card">
                    <div class="kv-label">1. SCAN THIS QR CODE</div>
                    <div class="totp-qr">${setup.qrSvg || ''}</div>
                    <div class="kv-label">OR ENTER THIS KEY MANUALLY</div>
                    <div class="kv-row"><span id="totp-secret" class="kv-val totp-secret">${esc(setup.secret)}</span><button class="mini-btn" onclick="App.copyText(document.getElementById('totp-secret').textContent, 'Secret copied')">Copy</button></div>
                    <div class="kv-label" style="margin-top:12px">2. ENTER THE 6-DIGIT CODE TO CONFIRM</div>
                    <form onsubmit="App.enableTotp(event)" style="display:flex;gap:8px;flex-wrap:wrap">
                      <input id="totp-code" type="text" inputmode="numeric" autocomplete="one-time-code" placeholder="000000" maxlength="6" required style="flex:1;min-width:140px" />
                      <button type="submit" class="btn-secondary">Enable 2FA</button>
                      <button type="button" class="mini-btn ghost" onclick="App.cancelTotpSetup()">Cancel</button>
                    </form>
                  </div>`
                : `<div class="settings-actions-row"><button class="btn-secondary" onclick="App.startTotpSetup()">Set up authenticator app</button></div>`
          }
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Recovery Codes</div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">${Math.max(0, sec.recoveryTotal - sec.recoveryUsed)} of ${sec.recoveryTotal} codes left</div>
              <div class="settings-row-desc">Single-use codes that work when you lose your authenticator. Store them somewhere safe.</div>
            </div>
          </div>
          ${
            codes
              ? `<div class="recovery-codes-card">
                  <div class="kv-label">SAVE THESE NOW — THEY WILL NOT BE SHOWN AGAIN</div>
                  <div class="recovery-codes-grid">${codes.map((c) => `<code>${esc(c)}</code>`).join('')}</div>
                  <button class="btn-secondary" style="margin-top:10px" onclick="App.dismissRecoveryCodes()">I've saved them</button>
                </div>`
              : `<div class="settings-actions-row"><button class="btn-secondary" onclick="App.generateRecoveryCodes()">Generate new recovery codes</button></div>`
          }
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Passkeys</div>
          <div class="settings-row-desc" style="margin-bottom:12px">Passkeys let you sign in with Face ID, Windows Hello, a hardware key or a synced password key. ${!sec.passkeysAvailable ? 'Passkeys are not configured on this server (set BEATLAB_ORIGIN to your public HTTPS URL).' : ''}</div>
          <div class="hidden-track-list">${passkeyRows}</div>
          <div class="settings-actions-row">
            <button class="btn-secondary" ${!sec.passkeysAvailable || !passkeySupported() ? 'disabled' : ''} onclick="App.addPasskey()">Add a passkey</button>
          </div>
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Sensitive Actions</div>
          <div class="settings-row-desc" style="margin-bottom:10px">${sec.twoFactorEnabled ? 'Enter an authenticator code, a recovery code, or authenticate with a passkey in this session.' : '2FA is off, so no code is required yet.'}</div>
          <input id="sec-code" type="text" inputmode="numeric" autocomplete="one-time-code" placeholder="${sec.twoFactorEnabled ? '2FA or recovery code' : '2FA code (not required)'}" style="width:100%;margin-bottom:8px" />
          ${sec.passkeys.length && sec.passkeysAvailable ? '<button class="mini-btn" style="margin-bottom:12px" onclick="App.stepUpWithPasskey()">Verify with a passkey instead</button>' : ''}
          ${
            confirmDisable
              ? `<div class="settings-confirm-bar">
                  <span>Disable all two-factor authentication? This removes your authenticator, passkeys and recovery codes.</span>
                  <div class="settings-confirm-actions">
                    <button class="btn-secondary danger-btn" onclick="App.confirmDisable2fa()">Yes, disable 2FA</button>
                    <button class="mini-btn ghost" onclick="App.cancelDisable2fa()">Cancel</button>
                  </div>
                </div>`
              : `<div class="settings-actions-row"><button class="btn-secondary danger-btn" ${!sec.twoFactorEnabled ? 'disabled' : ''} onclick="App.promptDisable2fa()">Disable two-factor authentication</button></div>`
          }
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Delete Account</div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">Delete my account</div>
              <div class="settings-row-desc">Signs you out of every device and music app immediately. You have 7 days to restore it by signing back in — after that it is permanently removed.</div>
            </div>
          </div>
          ${
            confirmDelete
              ? `<div class="settings-confirm-bar">
                  <div style="display:flex;flex-direction:column;gap:8px;width:100%">
                    <span>Type your password to confirm deletion.</span>
                    <input id="delete-password" type="password" autocomplete="current-password" placeholder="Account password" />
                    <div class="settings-confirm-actions">
                      <button class="btn-secondary danger-btn" onclick="App.confirmDeleteAccount()">Delete my account</button>
                      <button class="mini-btn ghost" onclick="App.cancelDeleteAccount()">Cancel</button>
                    </div>
                  </div>
                </div>`
              : `<div class="settings-actions-row"><button class="btn-secondary danger-btn" onclick="App.promptDeleteAccount()">Delete account</button></div>`
          }
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Security History</div>
          <div class="hidden-track-list">${eventRows}</div>
        </div>
      `;
      return;
    }

    body.innerHTML = `
      <div class="settings-section">
        <div class="settings-section-title">Account Profile</div>
        <div class="settings-profile-card">
          <span class="avatar-dot large">${avatarInner(selfAvatarUser())}</span>
          <div>
            <div class="settings-profile-name">${esc(state.user.username)}</div>
            <div class="settings-row-desc">Web, OpenSubsonic &amp; Navidrome account</div>
          </div>
        </div>
        <button class="btn-secondary" style="margin-top:12px;width:100%" onclick="App.closeModal();App.openMyProfile()">View my public profile →</button>
      </div>
      <div class="settings-section">
        <div class="settings-section-title">Security</div>
        <form onsubmit="App.submitPasswordChange(event)" style="display:flex;flex-direction:column;gap:8px">
          <div class="kv-label">CHANGE PASSWORD</div>
          <input id="pw-cur" type="password" autocomplete="current-password" placeholder="Current password" required />
          <input id="pw-new" type="password" autocomplete="new-password" placeholder="New password" required />
          ${state.user && state.user.twoFactorEnabled ? '<input id="pw-code" type="text" inputmode="numeric" autocomplete="one-time-code" placeholder="2FA or recovery code" required />' : ''}
          <div style="display:flex;gap:8px;flex-wrap:wrap;justify-content:space-between;align-items:center;margin-top:6px">
            <button type="submit" class="btn-secondary">Update Password</button>
            <button type="button" class="btn-secondary danger-btn" onclick="App.signOut()">Sign out</button>
          </div>
        </form>
        <button class="btn-secondary" style="margin-top:10px;width:100%" onclick="App.setSettingsTab('sessions')">Manage active sessions &rarr;</button>
      </div>
    `;
  }

  async function openSettingsModal(initialTab = 'general') {
    state.settingsTab = initialTab || 'general';
    state.settingsData.confirmResetHidden = false;
    state.settingsData.confirmRevokeAll = false;
    openModal(`
      <div class="settings-shell">
        <div class="modal-head settings-modal-head">
          <h3>Settings</h3>
          <button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button>
        </div>
        <div class="settings-layout">
          <nav class="settings-nav" aria-label="Settings sections">
            <button class="settings-nav-btn" data-stab="general" onclick="App.setSettingsTab('general')">General</button>
            <button class="settings-nav-btn" data-stab="hidden" onclick="App.setSettingsTab('hidden')">Hidden Songs</button>
            <button class="settings-nav-btn" data-stab="sessions" onclick="App.setSettingsTab('sessions')">Sessions</button>
            <button class="settings-nav-btn" data-stab="security" onclick="App.setSettingsTab('security')">Security &amp; 2FA</button>
            <button class="settings-nav-btn" data-stab="apps" onclick="App.setSettingsTab('apps')">Apps &amp; API</button>
            <button class="settings-nav-btn" data-stab="account" onclick="App.setSettingsTab('account')">Account</button>
          </nav>
          <div id="settings-body" class="settings-body"></div>
        </div>
      </div>
    `, 'settings-modal');
    renderSettingsModalBody();

    try {
      const [srcRes, hidRes, sessRes, secRes] = await Promise.all([
        api('/api/web/sources').catch(() => null),
        api('/api/web/hidden').catch(() => null),
        api('/api/web/sessions').catch(() => null),
        api('/api/web/2fa/status').catch(() => null),
      ]);
      if (srcRes) state.settingsData.sources = srcRes;
      if (hidRes) state.settingsData.hidden = hidRes;
      if (sessRes) state.settingsData.sessions = sessRes;
      if (secRes) state.settingsData.security = secRes;
      if ($('settings-body')) {
        renderSettingsModalBody();
      }
    } catch {}
  }

  function wireKeyboardShortcuts() {
    window.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        if ($('context-menu') && !$('context-menu').classList.contains('hidden')) {
          closeTrackMenu();
          return;
        }
        if (!$('modal-backdrop').classList.contains('hidden')) {
          closeModal();
          return;
        }
        if ($('fullscreen-player').classList.contains('open')) {
          setExpandedPlayer(false);
          return;
        }
      }
      const tag = (e.target && e.target.tagName) || '';
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
      if (e.code === 'Space') {
        if (e.repeat) return;
        e.preventDefault();
        togglePlay();
      } else if (e.key === '/') {
        if (e.repeat) return;
        e.preventDefault();
        setExpandedPlayer(false);
        $('global-search').focus();
      } else if (e.key === 'ArrowRight' && e.shiftKey) {
        const now = Date.now();
        if (e.repeat && now - state.lastSkipKeyAt < 75) return;
        state.lastSkipKeyAt = now;
        nextTrack(true);
      } else if (e.key === 'ArrowLeft' && e.shiftKey) {
        const now = Date.now();
        if (e.repeat && now - state.lastSkipKeyAt < 75) return;
        state.lastSkipKeyAt = now;
        prevTrack();
      } else if (e.key === 'ArrowRight') {
        audio.currentTime = Math.min(audio.duration || 9999, (audio.currentTime || 0) + 5);
      } else if (e.key === 'ArrowLeft') {
        audio.currentTime = Math.max(0, (audio.currentTime || 0) - 5);
      } else if (e.key.toLowerCase() === 'm') {
        if (e.repeat) return;
        App.toggleMute();
      } else if (e.key.toLowerCase() === 'l') {
        if (e.repeat) return;
        App.toggleCurrentLike();
      }
    });
  }

  // ---------------------------------------------------------------------------
  // SOCIAL — @mentions, following, public profiles & auto-updating user radio
  // ---------------------------------------------------------------------------
  const SOCIAL_COLORS = [
    '#f43f5e', '#8b5cf6', '#3b82f6', '#06b6d4', '#10b981',
    '#f59e0b', '#ec4899', '#6366f1', '#14b8a6', '#f97316',
  ];

  // Users with an uploaded picture render an <img>; everyone else keeps the
  // generated initial/color avatar. The container supplies size + background.
  function avatarInner(u) {
    if (u && u.avatarUrl) {
      return `<img class="avatar-img" src="${esc(u.avatarUrl)}" alt="" />`;
    }
    return esc((u && u.avatarLetter) || '?');
  }

  function selfAvatarUser() {
    return {
      avatarUrl: state.user?.avatarUrl || '',
      avatarLetter: (state.user?.username?.[0] || 'B').toUpperCase(),
    };
  }

  function timeAgo(ts) {
    const secs = Math.floor(Date.now() / 1000 - Number(ts || 0));
    if (!Number.isFinite(secs) || secs < 0) return '';
    if (secs < 60) return 'just now';
    if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
    if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
    if (secs < 2592000) return `${Math.floor(secs / 86400)}d ago`;
    return new Date(Number(ts) * 1000).toLocaleDateString();
  }

  function joinedText(ts) {
    try {
      return new Date(Number(ts) * 1000).toLocaleDateString(undefined, {
        month: 'long', year: 'numeric',
      });
    } catch {
      return '';
    }
  }

  function personCardHtml(u) {
    const ref = esc(JSON.stringify(u.username));
    const following = !!u.isFollowing;
    return `<div class="person-card">
      <div class="person-card-top">
        <span class="person-avatar" style="background:${esc(u.avatarColor)}" onclick="App.openUserProfile(${ref})">${avatarInner(u)}</span>
        <div class="person-card-name-wrap" onclick="App.openUserProfile(${ref})">
          <div class="person-card-name">${esc(u.displayName || u.username)}</div>
          <div class="person-card-handle">@${esc(u.username)}</div>
        </div>
        ${u.isSelf
          ? '<span class="mention-you">YOU</span>'
          : `<button class="btn-follow ${following ? 'following' : ''}" data-follow-ref="${esc(u.username)}" onclick="App.toggleFollowUser(${ref}, ${!following})">${following ? '✓ Following' : '+ Follow'}</button>`}
      </div>
      ${u.bio ? `<div class="person-card-bio">${esc(u.bio)}</div>` : ''}
      <div class="person-card-stats">
        <span onclick="App.openUserProfile(${ref})"><b>${u.followersCount}</b> followers</span>
        <span onclick="App.openUserProfile(${ref})"><b>${u.followingCount}</b> following</span>
        <span onclick="App.playUserRadio(${ref})">▶ Play radio</span>
      </div>
    </div>`;
  }

  function userCardHtml(u) {
    const ref = esc(JSON.stringify(u.username));
    const following = !!u.isFollowing;
    return `<div class="user-card">
      <span class="mention-avatar" style="background:${esc(u.avatarColor)}" onclick="App.openUserProfile(${ref})">${avatarInner(u)}</span>
      <div class="user-card-info" onclick="App.openUserProfile(${ref})">
        <div class="user-card-name">${esc(u.displayName || u.username)}</div>
        <div class="user-card-handle">@${esc(u.username)} · ${u.followersCount} followers</div>
      </div>
      ${u.isSelf ? '<span class="mention-you">YOU</span>' :
        `<button class="mention-follow ${following ? 'following' : ''}" data-follow-ref="${esc(u.username)}" onclick="App.toggleFollowUser(${ref}, ${!following})">${following ? 'Following' : 'Follow'}</button>`}
    </div>`;
  }

  function emptyState(title, msg) {
    return `<div class="social-empty"><h3>${esc(title)}</h3><div>${esc(msg)}</div></div>`;
  }

  function radioPanelHtml(data) {
    const radio = data.radio || {};
    const u = data.user;
    const ref = esc(JSON.stringify(u.username));
    const name = u.displayName || u.username;
    const tracks = cacheTracks(radio.tracks || []);
    const idsJson = esc(JSON.stringify(tracks.map((t) => t.id)));
    const saved = !!radio.saved;
    return `<div class="radio-panel">
      <div class="radio-hero">
        <div class="radio-art">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="7" width="19" height="13" rx="2.5"/><path d="m7 7 11.5-4.2a1 1 0 0 1 1.3.9V7"/><circle cx="8" cy="13.5" r="2.6"/><path d="M15 11h3.5M15 15h3.5"/></svg>
        </div>
        <div class="radio-copy">
          <div class="hero-kicker">USER RADIO · AUTO-UPDATING</div>
          <h2>${esc(name)}'s Radio</h2>
          <p>An evolving mix of what ${esc(name)} actually listens to. Beatlab quietly rebuilds it as their taste changes.</p>
          <div class="hero-actions">
            <button class="btn-primary" onclick="App.playUserRadio(${ref})">${ICONS.play} Play Radio</button>
            ${tracks.length > 1 ? `<button class="btn-secondary" onclick="App.playShuffledIds(${idsJson})">Shuffle</button>` : ''}
            <button class="btn-secondary" onclick="App.refreshUserRadio(${ref})">↻ Refresh</button>
            <button class="btn-secondary ${saved ? 'active' : ''}" onclick="App.toggleSaveRadio(${ref}, ${!saved})">${saved ? '✓ In your library' : '+ Save to library'}</button>
          </div>
        </div>
      </div>
      ${tracks.length
        ? `<section class="section">
            <div class="section-head"><h2>On the air</h2><span style="color:var(--text-dim);font-size:12px">${tracks.length} tracks · updates automatically</span></div>
            <div class="track-table">${renderTrackRows(tracks)}</div>
          </section>`
        : emptyState('Silent for now', 'This listener has not built up enough listening history for a radio yet.')}
    </div>`;
  }

  function playUserRadio(reference) {
    const tracks = (state.social.profileData?.radio?.tracks || []).filter(Boolean);
    playUserRadioFromData(tracks.map((t) => t.id), reference);
  }

  function playUserRadioFromData(ids, reference) {
    const trackIds = Array.isArray(ids) ? ids.filter(Boolean) : [];
    if (!trackIds.length) {
      showToast('This radio has no tracks yet');
      return;
    }
    playListByIds(trackIds, 0);
    showToast(`Playing @${reference}'s Radio`);
  }

  async function refreshUserRadio(reference) {
    await loadUserRadio(reference, true);
  }

  async function loadUserRadio(reference, announce = false) {
    try {
      const d = await api(`/api/web/social/radio/${encodeURIComponent(reference)}`);
      cacheTracks(d.tracks || []);
      if (state.social.profileData) state.social.profileData.radio = d;
      const body = $('profile-tab-body');
      if (body && state.social.profileData) {
        body.innerHTML = renderProfileTabBody(state.social.profileData);
      }
      if (announce) showToast(`Radio refreshed — ${(d.tracks || []).length} tracks`);
    } catch (e) {
      showToast(e.message);
    }
  }

  async function toggleSaveRadio(reference, save) {
    try {
      const res = await api(`/api/web/social/radio/${encodeURIComponent(reference)}/save`, {
        method: save ? 'POST' : 'DELETE',
      });
      showToast(
        save
          ? `Saved ${res.playlist?.name || 'radio'} to your library`
          : 'Radio removed from your library'
      );
      if (state.social.profileData && state.social.profileData.radio) {
        state.social.profileData.radio.saved = !!res.saved;
      }
      renderCurrentHash();
    } catch (e) {
      showToast(e.message);
    }
  }

  function openUserProfile(reference) {
    closeUserMenu();
    navigate(`user/${encodeURIComponent(reference)}`);
  }

  function openMyProfile() {
    closeUserMenu();
    navigate('profile');
  }

  function setProfileTab(tab) {
    haptic('tap');
    state.social.profileTab = tab;
    const data = state.social.profileData;
    if (!data) return;
    data.tab = tab;
    const body = $('profile-tab-body');
    if (body) {
      body.innerHTML = renderProfileTabBody(data);
      body.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }
    document.querySelectorAll('#profile-tabs .tab-chip').forEach((el) => {
      el.classList.toggle('active', el.dataset.ptab === tab);
    });
  }

  function publicStatsHtml(stats) {
    if (!stats || !stats.totalStreams) {
      return emptyState('No listening stats yet', 'These numbers appear once this listener starts playing music on Beatlab.');
    }
    const topTracks = stats.topTracks || [];
    const cards = [
      ['Streams', stats.totalStreams.toLocaleString(), `${(stats.completedStreams || 0).toLocaleString()} completed`],
      ['Listening time', fmtStatsDuration(stats.totalListeningSecs || 0), 'all time'],
      ['Songs', (stats.uniqueTracks || 0).toLocaleString(), `${stats.uniqueAlbums || 0} albums`],
      ['Artists', (stats.uniqueArtists || 0).toLocaleString(), 'explored'],
      ['Best streak', `${stats.longestStreak || 0}d`, 'in a row'],
    ];
    return `
      <section class="public-stats">
        <div class="stats-kpi-grid">
          ${cards
            .map(
              ([label, val, sub], i) => `
            <div class="stats-kpi-card ${i === 0 ? 'accent-green' : ''}">
              <div class="stats-kpi-label">${esc(label)}</div>
              <div class="stats-kpi-val">${esc(val)}</div>
              <div class="stats-kpi-sub">${esc(sub)}</div>
            </div>`
            )
            .join('')}
        </div>
        ${
          stats.topArtist || stats.topGenre
            ? `<div class="public-stats-highlights">
                 ${stats.topArtist ? `<div class="public-stat-highlight"><span>Top artist</span><strong>${esc(stats.topArtist)}</strong></div>` : ''}
                 ${stats.topGenre ? `<div class="public-stat-highlight"><span>Top genre</span><strong>${esc(stats.topGenre)}</strong></div>` : ''}
               </div>`
            : ''
        }
        ${
          topTracks.length
            ? `<section class="section">
                 <div class="section-head"><h2>Top tracks</h2><span style="color:var(--text-dim);font-size:12px">Most played</span></div>
                 <div class="public-top-tracks">
                   ${topTracks
                     .map(
                       (t, i) => `
                     <div class="public-top-track">
                       <span class="public-track-rank">${i + 1}</span>
                       <div class="public-track-meta"><strong>${esc(t.title)}</strong><span>${esc(t.artist)}</span></div>
                       <span class="public-track-plays">${(t.streams || 0).toLocaleString()} plays</span>
                     </div>`
                     )
                     .join('')}
                 </div>
               </section>`
            : ''
        }
        <p class="public-stats-foot">Member since ${esc(fmtDateLabel(stats.memberSince))}</p>
      </section>`;
  }

  function renderProfileTabBody(data) {
    const tab = data.tab || state.social.profileTab;
    if (tab === 'stats') {
      return publicStatsHtml(data.stats);
    }
    if (tab === 'radio') {
      return radioPanelHtml(data);
    }
    if (tab === 'playlists') {
      const pls = data.playlists || [];
      return pls.length
        ? `<div class="card-grid">${pls.map(renderPlaylistCard).join('')}</div>`
        : emptyState('No public playlists', 'Public playlists will appear here.');
    }
    const people = tab === 'following' ? data.following || [] : data.followers || [];
    return people.length
      ? `<div class="user-grid">${people.map(userCardHtml).join('')}</div>`
      : emptyState(tab === 'following' ? 'Not following anyone' : 'No followers yet', tab === 'following' ? 'Find people with @mentions in the search bar.' : 'Share your profile to gain followers.');
  }

  async function renderProfileView(reference) {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Loading profile...');
    const selfRef = reference === 'me';
    const ref = selfRef ? (state.user && state.user.id) : reference;
    const root = encodeURIComponent(String(ref));
    try {
      const emptyRadio = { tracks: [], saved: false, playlist: null, user: null };
      const results = await Promise.all([
        api(`/api/web/social/profile/${root}`, { signal }),
        api(`/api/web/social/followers/${root}`, { signal }).catch(() => ({ users: [] })),
        api(`/api/web/social/following/${root}`, { signal }).catch(() => ({ users: [] })),
        api(`/api/web/social/radio/${root}`, { signal }).catch(() => emptyRadio),
      ]);
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const profileRes = results[0];
      const u = profileRes.user;
      const pstats = profileRes.stats || {};
      const radio = results[3] || emptyRadio;
      cacheTracks(radio.tracks || []);
      const isSelf = selfRef || !!u.isSelf;
      const defaultTab = 'stats';
      if (state.social.profileData && state.social.profileData.user.id !== u.id) {
        state.social.profileTab = defaultTab;
      }
      const data = {
        user: u,
        profile: profileRes.profile || {},
        stats: profileRes.stats || null,
        playlists: profileRes.playlists || [],
        radio,
        followers: results[1]?.users || [],
        following: results[2]?.users || [],
        isSelf,
        tab: state.social.profileTab || defaultTab,
      };
      state.social.profileData = data;

      const color = u.avatarColor || '#1ed760';
      const tabs = [
        ['stats', 'Stats'],
        ['radio', isSelf ? 'Your Radio' : 'Radio'],
        ['playlists', 'Playlists'],
        ['followers', 'Followers'],
        ['following', 'Following'],
      ];
      const name = u.displayName || u.username;

      let html = `<div class="hero-header" style="border-color:${esc(color)}55;background:${esc(color)}14">
        <div class="profile-avatar" style="background:${esc(color)}">${avatarInner(u)}</div>
        <div class="hero-info">
          <div class="profile-verified">${isSelf ? 'Your public profile' : 'Beatlab listener'}</div>
          <h1 class="hero-title">${esc(name)}</h1>
          <div class="hero-sub"><span class="profile-handle">@${esc(u.username)}</span> · Joined ${esc(joinedText(u.created))}</div>
          ${u.bio ? `<div class="profile-bio">${esc(u.bio)}</div>` : ''}
          ${u.location ? `<div class="profile-location">📍 ${esc(u.location)}</div>` : ''}
          <div class="profile-stats">
            <button class="profile-stat profile-stat-btn" onclick="App.setProfileTab('followers')"><strong>${u.followersCount}</strong><span>Followers</span></button>
            <button class="profile-stat profile-stat-btn" onclick="App.setProfileTab('following')"><strong>${u.followingCount}</strong><span>Following</span></button>
            <button class="profile-stat profile-stat-btn" onclick="App.setProfileTab('radio')"><strong>${(radio.tracks || []).length}</strong><span>Radio tracks</span></button>
            ${pstats.totalStreams ? `<button class="profile-stat profile-stat-btn" onclick="App.setProfileTab('stats')"><strong>${pstats.totalStreams.toLocaleString()}</strong><span>Streams</span></button>` : ''}
            ${pstats.totalStreams ? `<button class="profile-stat profile-stat-btn" onclick="App.setProfileTab('stats')"><strong>${esc(fmtStatsDuration(pstats.totalListeningSecs || 0))}</strong><span>Listening</span></button>` : ''}
          </div>
          <div class="hero-actions">
            <button class="btn-primary" onclick="App.playUserRadio(${esc(JSON.stringify(u.username))})">${ICONS.play} Play ${isSelf ? 'My' : '@' + u.username + "'s"} Radio</button>
            ${isSelf
              ? `<button class="btn-secondary" onclick="App.openProfileEditor()">✎ Edit profile</button>`
              : `<button class="btn-follow ${u.isFollowing ? 'following' : ''}" data-follow-ref="${esc(u.username)}" onclick="App.toggleFollowUser(${esc(JSON.stringify(u.username))}, ${!u.isFollowing})">${u.isFollowing ? '✓ Following' : '+ Follow'}</button>`}
          </div>
        </div>
      </div>
      <div class="tabs-bar" id="profile-tabs">
        ${tabs.map(([key, label]) => `<button class="tab-chip ${data.tab === key ? 'active' : ''}" data-ptab="${key}" onclick="App.setProfileTab('${key}')">${label}</button>`).join('')}
      </div>
      <div id="profile-tab-body">${renderProfileTabBody(data)}</div>`;

      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  function syncFollowState(reference, updated, following) {
    const ref = String(reference || '').toLowerCase();
    // Flip every visible follow button for this user, whichever view is open.
    document.querySelectorAll('[data-follow-ref]').forEach((btn) => {
      if (String(btn.dataset.followRef || '').toLowerCase() !== ref) return;
      btn.classList.toggle('following', following);
      btn.textContent = btn.classList.contains('btn-follow')
        ? following ? '✓ Following' : '+ Follow'
        : following ? 'Following' : 'Follow';
    });
    // Keep cached user objects consistent so later renders are correct too.
    const patch = (u) => {
      if (u && String(u.username || '').toLowerCase() === ref) {
        u.isFollowing = following;
        if (updated && typeof updated.followersCount === 'number') {
          u.followersCount = updated.followersCount;
        }
      }
    };
    (state.searchState?.users || []).forEach(patch);
    const pdata = state.social.profileData;
    if (pdata) {
      patch(pdata.user);
      (pdata.followers || []).forEach(patch);
      (pdata.following || []).forEach(patch);
      if (pdata.user && String(pdata.user.username || '').toLowerCase() === ref && updated) {
        document.querySelectorAll('.profile-stats .profile-stat').forEach((stat) => {
          const label = stat.querySelector('span');
          if (label && label.textContent.trim() === 'Followers') {
            const strong = stat.querySelector('strong');
            if (strong) strong.textContent = updated.followersCount;
          }
        });
      }
    }
  }

  async function toggleFollowUser(reference, following) {
    haptic('select');
    try {
      const res = await api(`/api/web/social/follow/${encodeURIComponent(reference)}`, {
        method: following ? 'POST' : 'DELETE',
      });
      const updated = res.user;
      showToast(following ? `Now following @${updated?.username || reference}` : `Unfollowed @${reference}`);
      syncFollowState(reference, updated, following);
    } catch (e) {
      showToast(e.message);
    }
  }

  function avatarInitial(u) {
    const src = (u && (u.displayName || u.username)) || state.user?.username || '?';
    return (String(src).trim()[0] || '?').toUpperCase();
  }

  function renderAvatarPreview() {
    const el = $('prof-avatar-preview');
    if (!el) return;
    const pending = state.social.editAvatarData;
    const u = state.social.profileData?.user;
    el.style.background = u?.avatarColor || '#1ed760';
    if (pending && pending !== 'REMOVE') {
      el.innerHTML = `<img class="avatar-img" src="${esc(pending)}" alt="" />`;
    } else if (pending === 'REMOVE') {
      el.innerHTML = esc(avatarInitial(u));
    } else if (u?.avatarUrl) {
      el.innerHTML = `<img class="avatar-img" src="${esc(u.avatarUrl)}" alt="" />`;
    } else {
      el.innerHTML = esc(avatarInitial(u));
    }
    const removeBtn = $('prof-avatar-remove');
    if (removeBtn) {
      const hasPicture = pending ? pending !== 'REMOVE' : !!u?.avatarUrl;
      removeBtn.classList.toggle('hidden', !hasPicture);
    }
  }

  function base64ToBlob(base64, mime) {
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return new Blob([bytes], { type: mime });
  }

  function pickAvatarFile() {
    const input = $('prof-avatar-file');
    if (input) input.click();
  }

  async function onAvatarFileSelected(event) {
    const file = event?.target?.files?.[0];
    if (!file) return;
    if (!file.type.startsWith('image/')) {
      showToast('Please choose an image file');
      return;
    }
    if (file.size > 8 * 1024 * 1024) {
      showToast('Image is too large (max 8 MB)');
      return;
    }
    try {
      state.social.editAvatarData = await cropAvatarFile(file);
      renderAvatarPreview();
    } catch (err) {
      showToast(err?.message || 'Could not read that image');
    } finally {
      if (event?.target) event.target.value = '';
    }
  }

  // Center-crop to a square and downscale before upload. Keeps uploads small
  // and makes every profile picture render consistently across the UI.
  function cropAvatarFile(file) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onerror = () => reject(new Error('Could not read that image'));
      reader.onload = () => {
        const img = new Image();
        img.onerror = () => reject(new Error('Unsupported image'));
        img.onload = () => {
          try {
            const size = 256;
            const canvas = document.createElement('canvas');
            canvas.width = size;
            canvas.height = size;
            const ctx = canvas.getContext('2d');
            const side = Math.min(img.width, img.height);
            const sx = (img.width - side) / 2;
            const sy = (img.height - side) / 2;
            ctx.drawImage(img, sx, sy, side, side, 0, 0, size, size);
            resolve(canvas.toDataURL('image/jpeg', 0.88));
          } catch (e) {
            reject(e);
          }
        };
        img.src = reader.result;
      };
      reader.readAsDataURL(file);
    });
  }

  function removeAvatar() {
    state.social.editAvatarData = 'REMOVE';
    renderAvatarPreview();
  }

  function openProfileEditor() {
    const p = state.social.profileData?.profile || {};
    const user = state.social.profileData?.user;
    state.social.editColor = p.avatarColor || '';
    state.social.editAvatarData = null;
    const hasPicture = !!user?.avatarUrl;
    openModal(`
      <div class="modal-head">
        <h3>Edit your profile</h3>
        <button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button>
      </div>
      <div class="social-edit-row">
        <label>Profile picture</label>
        <div class="social-avatar-editor">
          <div class="social-avatar-preview" id="prof-avatar-preview" style="background:${esc(user?.avatarColor || '#1ed760')}">${avatarInner(user || selfAvatarUser())}</div>
          <div class="social-avatar-actions">
            <input type="file" id="prof-avatar-file" accept="image/*" class="hidden" onchange="App.onAvatarFileSelected(event)" />
            <button type="button" class="btn-secondary" onclick="App.pickAvatarFile()">Choose image…</button>
            <button type="button" id="prof-avatar-remove" class="social-avatar-remove ${hasPicture ? '' : 'hidden'}" onclick="App.removeAvatar()">Remove picture</button>
            <span class="social-avatar-hint">Square crop · up to 8 MB</span>
          </div>
        </div>
      </div>
      <div class="social-edit-row">
        <label>Display name</label>
        <input id="prof-name" maxlength="40" value="${esc(p.displayName || state.user.username)}" />
      </div>
      <div class="social-edit-row">
        <label>Bio</label>
        <textarea id="prof-bio" maxlength="280" rows="3" placeholder="Tell people about your taste...">${esc(p.bio || '')}</textarea>
      </div>
      <div class="social-edit-row">
        <label>Location</label>
        <input id="prof-location" maxlength="80" value="${esc(p.location || '')}" placeholder="Where are you listening from?" />
      </div>
      <div class="social-edit-row">
        <label>Avatar color</label>
        <div class="social-color-row">
          ${SOCIAL_COLORS.map((c) => `<button type="button" class="social-color-swatch ${c === state.social.editColor ? 'selected' : ''}" style="background:${c}" onclick="App.pickProfileColor('${c}')" title="${c}"></button>`).join('')}
        </div>
      </div>
      <button class="btn-primary" style="width:100%;margin-top:6px" onclick="App.submitProfileUpdate(event)">Save profile</button>
    `);
  }

  function pickProfileColor(color) {
    state.social.editColor = color;
    document.querySelectorAll('.social-color-swatch').forEach((el) => {
      el.classList.toggle('selected', el.style.background.includes(color) || el.getAttribute('title') === color);
    });
  }

  async function submitProfileUpdate(e) {
    if (e) e.preventDefault();
    const btn = e?.target;
    try {
      if (btn) { btn.disabled = true; btn.textContent = 'Saving…'; }
      const pending = state.social.editAvatarData;
      if (pending === 'REMOVE') {
        await api('/api/web/social/avatar', { method: 'DELETE' });
      } else if (pending) {
        const base64 = pending.slice(pending.indexOf(',') + 1);
        await api('/api/web/social/avatar', {
          method: 'POST',
          headers: { 'Content-Type': 'image/jpeg' },
          body: base64ToBlob(base64, 'image/jpeg'),
        });
      }
      const res = await api('/api/web/social/me', {
        method: 'POST',
        body: JSON.stringify({
          displayName: $('prof-name')?.value || '',
          bio: $('prof-bio')?.value || '',
          location: $('prof-location')?.value || '',
          avatarColor: state.social.editColor || '',
        }),
      });
      if (typeof res.user?.avatarUrl === 'string') state.user.avatarUrl = res.user.avatarUrl;
      state.social.editAvatarData = null;
      closeModal();
      showToast('Profile updated');
      $('user-avatar').innerHTML = avatarInner(selfAvatarUser());
      renderCurrentHash();
    } catch (err) {
      showToast(err.message);
    } finally {
      if (btn) { btn.disabled = false; btn.textContent = 'Save profile'; }
    }
  }

  async function renderCommunity() {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Loading community...');
    try {
      const [feedRes, sugRes] = await Promise.all([
        api('/api/web/social/feed', { signal }),
        api('/api/web/social/suggestions', { signal }),
      ]);
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const radios = feedRes.radios || [];
      radios.forEach((r) => cacheTracks(r.tracks || []));
      const suggestions = sugRes.users || [];
      let html = `<div class="community-intro">
        <div>
          <h2>Your community</h2>
          <p>Live radios from the people you follow. Each one is an evolving mix of what that listener actually plays, refreshed automatically as their taste changes.</p>
        </div>
        <button class="btn-primary" onclick="App.openMyProfile()">${ICONS.play} My Radio</button>
      </div>`;
      html += `<section class="section">
        <div class="section-head"><h2>People to follow</h2><button class="mini-btn" onclick="App.focusSearch()">Find people with @</button></div>
        ${suggestions.length ? `<div class="user-grid">${suggestions.map(userCardHtml).join('')}</div>` : emptyState('No suggestions yet', 'Invite friends or search with @ to find listeners.')}
      </section>`;
      html += `<section class="section">
        <div class="section-head"><h2>Radios from people you follow</h2></div>
        ${radios.length
          ? `<div class="community-radios">${radios.map((r) => {
              const u = r.user;
              const ref = esc(JSON.stringify(u.username));
              const tracks = r.tracks || [];
              const idsJson = esc(JSON.stringify(tracks.map((t) => t.id)));
              return `<div class="community-radio-card">
                <div class="community-radio-head">
                  <span class="person-avatar small" style="background:${esc(u.avatarColor)}" onclick="App.openUserProfile(${ref})">${avatarInner(u)}</span>
                  <div class="community-radio-meta" onclick="App.openUserProfile(${ref})">
                    <b>${esc(u.displayName || u.username)}'s Radio</b>
                    <span>@${esc(u.username)} · ${typeof r.trackCount === 'number' ? r.trackCount : tracks.length} tracks · updates automatically</span>
                  </div>
                  <div class="community-radio-actions">
                    <button class="btn-secondary" onclick="App.playUserRadioFromData(${idsJson}, ${ref})">Play</button>
                    ${tracks.length > 1 ? `<button class="btn-secondary" onclick="App.playShuffledIds(${idsJson})">Shuffle</button>` : ''}
                  </div>
                </div>
                ${tracks.length ? `<div class="track-table compact">${renderTrackRows(tracks.slice(0, 5))}</div>` : ''}
              </div>`;
            }).join('')}</div>`
          : emptyState('No radios yet', 'Follow some listeners to see their radios here.')}
      </section>`;
      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
    } catch (e) {
      handleViewFetchError(e, reqToken, signal);
    }
  }

  let userMenuDocWired = false;
  function toggleUserMenu(event) {
    if (event && typeof event.stopPropagation === 'function') event.stopPropagation();
    const menu = $('user-menu');
    if (!menu) return;
    haptic('tap');
    const opening = menu.classList.contains('hidden');
    menu.classList.toggle('hidden', !opening);
    if (opening) {
      const name = state.user ? state.user.username : 'User';
      $('user-menu-name').textContent = name;
      $('user-menu-avatar').innerHTML = avatarInner(selfAvatarUser());
    }
    if (!userMenuDocWired) {
      userMenuDocWired = true;
      document.addEventListener('click', (e) => {
        if (!e.target.closest('.user-menu-wrap')) closeUserMenu();
      });
      document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') closeUserMenu();
      });
    }
  }

  function closeUserMenu() {
    const menu = $('user-menu');
    if (menu) menu.classList.add('hidden');
  }

  function goCommunity() {
    closeUserMenu();
    navigate('community');
  }

  function goSettings() {
    closeUserMenu();
    openSettingsModal();
  }

  // Expose public handlers for template onclick attributes
  window.App = {
    navigate,
    // Social
    openUserProfile,
    openMyProfile,
    setProfileTab,
    toggleFollowUser,
    playUserRadio,
    playUserRadioFromData,
    refreshUserRadio,
    loadUserRadio,
    toggleSaveRadio,
    openProfileEditor,
    pickProfileColor,
    pickAvatarFile,
    onAvatarFileSelected,
    removeAvatar,
    submitProfileUpdate,
    pickPlaylistVisibility,
    renderCommunity,
    toggleUserMenu,
    closeUserMenu,
    goCommunity,
    goSettings,
    focusSearch: () => {
      haptic('tap');
      setExpandedPlayer(false);
      closeTrackMenu();
      const input = $('global-search');
      if (input) {
        input.focus();
        if (input.value) input.select();
      }
    },
    clearSearch: () => {
      clearTimeout(state.searchTimer);
      state.searchTimer = null;
      state.searchComposing = false;
      state.ownSearchNavQuery = null;
      abortActiveViewRequest();
      $('global-search').value = '';
      $('clear-search-btn').classList.add('hidden');
      navigate('explore');
    },
    signOut: async () => {
      const oldToken = state.token;
      localStorage.removeItem('beatlab_token');
      state.token = '';
      state.user = null;
      resetPlayerAndSessionUI();
      showAuthScreen(true);
      if (navigator.credentials && navigator.credentials.preventSilentAccess) {
        navigator.credentials.preventSilentAccess().catch(() => {});
      }
      if (oldToken) {
        fetch('/api/web/auth/signout', {
          method: 'POST',
          headers: { Authorization: `Bearer ${oldToken}` },
        }).catch(() => {});
      }
    },
    backToSignIn: () => {
      resetAuthForm();
    },
    restoreAccount: async () => {
      const errBox = $('restore-error');
      if (errBox) errBox.classList.add('hidden');
      try {
        const res = await api('/api/web/account/restore', {
          method: 'POST',
          body: JSON.stringify({ challenge: state.restoreChallenge }),
        });
        state.token = res.token;
        state.user = res.user;
        state.restoreChallenge = '';
        localStorage.setItem('beatlab_token', res.token);
        showToast('Account restored — welcome back!');
        await onSignedIn();
      } catch (err) {
        if (errBox) {
          errBox.textContent = err.message;
          errBox.classList.remove('hidden');
        }
      }
    },
    signInWithPasskey: async () => {
      if (!passkeySupported()) {
        showToast('Passkeys are not supported by this browser');
        return;
      }
      const username = ($('auth-username') && $('auth-username').value || '').trim();
      if (!username) {
        const box = $('auth-error');
        if (box) {
          box.textContent = 'Enter your username first, then sign in with a passkey.';
          box.classList.remove('hidden');
        }
        return;
      }
      try {
        const opts = await api('/api/web/auth/passkey/options', {
          method: 'POST',
          body: JSON.stringify({ username }),
        });
        state.authChallenge = opts.challenge;
        const credential = await navigator.credentials.get({
          publicKey: webauthnOptions(opts.publicKey, false),
        });
        if (!credential) throw new Error('Passkey sign-in was cancelled');
        const res = await api('/api/web/auth/passkey/verify', {
          method: 'POST',
          body: JSON.stringify({
            challenge: opts.challenge,
            credential: webauthnCredentialToJson(credential),
          }),
        });
        state.token = res.token;
        state.user = res.user;
        state.authChallenge = '';
        localStorage.setItem('beatlab_token', res.token);
        showToast(`Welcome back, ${res.user.username}`);
        await onSignedIn();
      } catch (err) {
        const box = $('auth-mfa') && !$('auth-mfa').classList.contains('hidden') ? $('mfa-error') : $('auth-error');
        if (box) {
          box.textContent = err.message;
          box.classList.remove('hidden');
        }
      }
    },
    setLibTab: (tab) => {
      haptic('tap');
      return renderLibrary(tab);
    },
    setStatsRange: (range) => {
      haptic('tap');
      state.statsRange = range || 'all';
      navigate(state.statsRange === 'all' ? 'stats' : `stats?range=${encodeURIComponent(state.statsRange)}`);
    },
    setStatsMetric: (metric) => {
      haptic('tap');
      state.statsMetric = metric === 'mins' ? 'mins' : 'streams';
      renderStatsView();
    },
    refreshStats: () => renderStats(state.statsRange || 'all'),
    setRightTab: (tab) => {
      haptic('tap');
      $('tab-queue').classList.toggle('active', tab === 'queue');
      $('tab-lyrics').classList.toggle('active', tab === 'lyrics');
      $('rp-queue-body').classList.toggle('hidden', tab !== 'queue');
      $('rp-lyrics-body').classList.toggle('hidden', tab !== 'lyrics');
    },
    setFsMobileView,
    setFsTab: (tab, silent = false) => {
      if (!silent && state.fsTab !== tab) haptic('tap');
      state.fsTab = tab;
      $('fs-tab-queue').classList.toggle('active', tab === 'queue');
      $('fs-tab-lyrics').classList.toggle('active', tab === 'lyrics');
      $('fs-queue-panel').classList.toggle('hidden', tab !== 'queue');
      $('fs-lyrics-panel').classList.toggle('hidden', tab !== 'lyrics');
      if ($('fs-tab-mobile-queue')) $('fs-tab-mobile-queue').classList.toggle('active', state.fsMobileView === 'queue');
      if ($('fs-tab-mobile-lyrics')) $('fs-tab-mobile-lyrics').classList.toggle('active', state.fsMobileView === 'lyrics');
      if (tab === 'lyrics' && state.activeLyricIdx >= 0) {
        const activeLine = $('fs-lyrics-body')?.querySelector(`.lyric-line[data-lidx="${state.activeLyricIdx}"]`);
        if (activeLine) activeLine.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }
    },
    toggleRightPanel: (show) => {
      const rp = $('right-panel');
      const backdrop = $('panel-backdrop');
      if (!rp) return;
      if (window.innerWidth <= 780) {
        rp.classList.add('hidden');
        if (backdrop) backdrop.classList.add('hidden');
        return;
      }
      rp.dataset.userToggled = '1';
      const hide = show === false;
      rp.classList.toggle('hidden', hide);
      if (backdrop) {
        backdrop.classList.toggle('hidden', hide || window.innerWidth > 1060);
      }
    },
    openPanelTab: (tab) => {
      if (window.innerWidth <= 780) {
        setExpandedPlayer(true);
        setFsMobileView(tab === 'lyrics' ? 'lyrics' : 'queue');
        return;
      }
      if ($('fullscreen-player').classList.contains('open')) {
        window.App.setFsTab(tab);
        return;
      }
      const rp = $('right-panel');
      rp.dataset.userToggled = '1';
      rp.classList.remove('hidden');
      if ($('panel-backdrop')) {
        $('panel-backdrop').classList.toggle('hidden', window.innerWidth > 1060);
      }
      window.App.setRightTab(tab);
    },
    toggleAutoplay: (v) => {
      haptic('select');
      state.autoplay = !!v;
      localStorage.setItem('beatlab_autoplay', state.autoplay ? 'true' : 'false');
    },
    toggleHaptics: (v) => {
      state.hapticsEnabled = !!v;
      localStorage.setItem('beatlab_haptics', state.hapticsEnabled ? 'true' : 'false');
      if (state.hapticsEnabled) haptic('success');
    },
    setTheme: (mode) => {
      haptic('select');
      const next = mode === 'light' ? 'light' : 'dark';
      localStorage.setItem('beatlab_theme', next);
      applyTheme(next);
      renderSettingsModalBody();
    },
    toggleTheme: (isLight) => {
      window.App.setTheme(isLight ? 'light' : 'dark');
    },
    removeFromQueue: (idx) => {
      if (idx > state.queueIndex && idx < state.queue.length) {
        haptic('tap');
        state.queue.splice(idx, 1);
        renderQueuePanel();
        saveServerQueue();
        if (idx === state.queueIndex + 1) scheduleWarmNextTrack();
      }
    },
    startRadioForId,
    setUpcomingMode,
    playListByIds,
    playShuffledIds,
    playSingle,
    playQueueAt,
    playAlbumNow,
    playPlaylistNow,
    playNext: (tid) => {
      const t = state.trackMap.get(tid);
      if (!t) return;
      haptic('success');
      if (state.queue.length === 0) return playSingle(tid);
      state.queue.splice(state.queueIndex + 1, 0, t);
      if (Array.isArray(state.unshuffledQueue)) {
        const cur = currentTrack();
        const curPos = cur ? state.unshuffledQueue.findIndex((item) => item.id === cur.id) : -1;
        state.unshuffledQueue.splice(curPos >= 0 ? curPos + 1 : 0, 0, t);
      }
      renderQueuePanel();
      saveServerQueue();
      scheduleWarmNextTrack();
      showToast('Will play next');
    },
    addToQueue: (tid) => {
      const t = state.trackMap.get(tid);
      if (!t) return;
      haptic('success');
      if (state.queue.length === 0) return playSingle(tid);
      state.queue.push(t);
      if (Array.isArray(state.unshuffledQueue)) {
        state.unshuffledQueue.push(t);
      }
      renderQueuePanel();
      saveServerQueue();
      scheduleWarmNextTrack();
      showToast('Added to queue');
    },
    togglePlay,
    prevTrack,
    nextTrack,
    toggleShuffle: () => {
      haptic('select');
      state.shuffle = !state.shuffle;
      localStorage.setItem('beatlab_shuffle', state.shuffle ? 'true' : 'false');
      syncShuffleUI();
      if (state.queue.length > 0) {
        const splitIdx = state.queueIndex >= 0 ? state.queueIndex + 1 : 0;
        const played = state.queue.slice(0, splitIdx);
        const upcoming = state.queue.slice(splitIdx);
        if (state.shuffle) {
          state.unshuffledQueue = state.queue.slice();
          if (upcoming.length > 1) {
            state.queue = played.concat(shuffleArray(upcoming));
          }
        } else {
          if (Array.isArray(state.unshuffledQueue) && state.unshuffledQueue.length > 0 && upcoming.length > 1) {
            state.queue = played.concat(restoreUpcomingOrder(upcoming, state.unshuffledQueue, currentTrack()));
          }
          state.unshuffledQueue = null;
        }
        renderQueuePanel();
        saveServerQueue();
        scheduleWarmNextTrack();
      } else {
        state.unshuffledQueue = null;
      }
    },
    cycleRepeat: () => {
      haptic('select');
      const modes = ['off', 'all', 'one'];
      state.repeat = modes[(modes.indexOf(state.repeat) + 1) % modes.length];
      for (const btnId of ['btn-repeat', 'fs-repeat']) {
        const btn = $(btnId);
        if (!btn) continue;
        btn.classList.toggle('active', state.repeat !== 'off');
        btn.title = `Repeat: ${state.repeat}`;
        const svg = btn.querySelector('svg');
        if (svg) svg.innerHTML = state.repeat === 'one' ? ICONS.repeatOne : ICONS.repeatAll;
      }
      showToast(`Repeat: ${state.repeat}`);
    },
    toggleMute: () => {
      state.muted = !state.muted;
      applyOutputVolume();
      updateVolumeIcon();
    },
    toggleCurrentLike: () => {
      const cur = currentTrack();
      if (cur) toggleTrackLike(cur.id, $('pb-like'));
    },
    toggleTrackLike,
    goCurrentAlbum: () => {
      const cur = currentTrack();
      if (cur) navigate(`album/${encodeURIComponent(cur.albumId)}`);
    },
    goCurrentArtist: () => {
      const cur = currentTrack();
      if (cur) navigate(`artist/${encodeURIComponent(cur.artistId)}`);
    },
    toggleFullscreen: (open) => {
      setExpandedPlayer(open);
    },
    seekMs: (ms) => {
      if (Number.isFinite(ms)) {
        haptic('tap');
        audio.currentTime = ms / 1000;
        if (audio.paused) audio.play().catch(() => {});
      }
    },
    openTrackMenu,
    closeTrackMenu,
    shareEntity,
    shareTrack: (tid) => {
      const t = state.trackMap.get(tid);
      if (!t) return;
      shareEntity({
        kind: 'track',
        targetId: t.id,
        title: t.title,
        subtitle: t.artist,
        coverUrl: t.coverUrl,
      });
    },
    shareCurrentTrack: () => {
      const cur = currentTrack();
      if (!cur) return showToast('Play a track first to share');
      window.App.shareTrack(cur.id);
    },
    sharePlaylist: (id, title, ownerName, coverUrl = '') => {
      shareEntity({
        kind: 'playlist',
        targetId: id,
        title,
        subtitle: ownerName ? `Playlist by ${ownerName}` : 'Playlist',
        coverUrl,
      });
    },
    shareAlbum: (id, title, artist, coverUrl = '') => {
      shareEntity({
        kind: 'album',
        targetId: id,
        title,
        subtitle: artist || 'Album',
        coverUrl,
      });
    },
    shareArtist: (id, name, coverUrl = '') => {
      shareEntity({
        kind: 'artist',
        targetId: id,
        title: name,
        subtitle: 'Artist',
        coverUrl,
      });
    },
    shareCurrentInstantMix: () => {
      const active = getActiveInstantMix();
      const mix = active && active.mix;
      if (!mix || !mix.tracks || !mix.tracks.length) return;
      shareEntity({
        kind: 'mix',
        targetId: `mix:${mix.title || active.query || 'custom'}`,
        title: mix.title,
        subtitle: mix.subtitle || 'Instant Custom Mix',
        trackIds: mix.tracks.map((t) => t.id),
        coverUrl: mix.tracks[0]?.coverUrl || '',
      });
    },
    shareCurrentMoodMix: () => {
      if (!state.currentMoodShare) return;
      shareEntity(state.currentMoodShare);
    },
    shareCurrentGenreMix: () => {
      if (!state.currentGenreShare) return;
      shareEntity(state.currentGenreShare);
    },
    nativeShareCurrent: async () => {
      if (!state.currentShareUrl || !navigator.share) return;
      try {
        await navigator.share({ title: 'Beatlab', url: state.currentShareUrl });
      } catch {}
    },
    markNotInterested,
    openSettingsModal,
    setSettingsTab: (tab) => {
      haptic('tap');
      state.settingsTab = tab;
      state.settingsData.confirmResetHidden = false;
      state.settingsData.confirmRevokeAll = false;
      state.settingsData.confirmDisable2fa = false;
      state.settingsData.confirmDelete = false;
      if (tab === 'security') {
        api('/api/web/2fa/status').then((res) => {
          state.settingsData.security = res;
          if ($('settings-body') && state.settingsTab === 'security') renderSettingsModalBody();
        }).catch(() => {});
      }
      renderSettingsModalBody();
    },
    promptResetHidden: () => {
      state.settingsData.confirmResetHidden = true;
      renderSettingsModalBody();
    },
    cancelResetHidden: () => {
      state.settingsData.confirmResetHidden = false;
      renderSettingsModalBody();
    },
    confirmResetHidden: async () => {
      try {
        const res = await api('/api/web/hidden/reset', { method: 'POST' });
        state.settingsData.hidden = { count: 0, tracks: [] };
        state.settingsData.confirmResetHidden = false;
        renderSettingsModalBody();
        showToast(
          res.cleared > 0
            ? `Reset hide data (${res.cleared} ${res.cleared === 1 ? 'song' : 'songs'} restored)`
            : 'Hide data cleared'
        );
        renderCurrentHash();
      } catch (err) {
        showToast(err.message);
      }
    },
    unhideSong: async (trackId) => {
      try {
        await api(`/api/web/hidden/${encodeURIComponent(trackId)}`, { method: 'DELETE' });
        const hidRes = await api('/api/web/hidden');
        state.settingsData.hidden = hidRes;
        state.settingsData.confirmResetHidden = false;
        renderSettingsModalBody();
        showToast('Song restored');
        renderCurrentHash();
      } catch (err) {
        showToast(err.message);
      }
    },
    revokeSession: async (id) => {
      try {
        await api('/api/web/sessions/revoke', { method: 'POST', body: JSON.stringify({ id }) });
        const wasCurrent = id === state.token;
        const sessRes = await api('/api/web/sessions').catch(() => null);
        if (sessRes) state.settingsData.sessions = sessRes;
        state.settingsData.confirmRevokeAll = false;
        showToast('Session revoked');
        if (wasCurrent) {
          window.App.signOut();
        } else {
          renderSettingsModalBody();
        }
      } catch (err) {
        showToast(err.message);
      }
    },
    promptRevokeAll: () => {
      state.settingsData.confirmRevokeAll = true;
      renderSettingsModalBody();
    },
    cancelRevokeAll: () => {
      state.settingsData.confirmRevokeAll = false;
      renderSettingsModalBody();
    },
    confirmRevokeAll: async () => {
      try {
        const res = await api('/api/web/sessions/revoke-all', { method: 'POST' });
        state.settingsData.sessions = { count: 0, sessions: [] };
        state.settingsData.confirmRevokeAll = false;
        showToast(
          res && res.revoked > 0
            ? `Signed out of ${res.revoked} ${res.revoked === 1 ? 'session' : 'sessions'}`
            : 'All sessions cleared'
        );
        window.App.signOut();
      } catch (err) {
        showToast(err.message);
      }
    },
    allowSession: async (id) => {
      try {
        await api('/api/web/sessions/allow', { method: 'POST', body: JSON.stringify({ id }) });
        const sessRes = await api('/api/web/sessions').catch(() => null);
        if (sessRes) state.settingsData.sessions = sessRes;
        renderSettingsModalBody();
        showToast('Client allowed again');
      } catch (err) {
        showToast(err.message);
      }
    },
    refreshSecurityStatus: async () => {
      try {
        state.settingsData.security = await api('/api/web/2fa/status');
      } catch {}
      if ($('settings-body') && state.settingsTab === 'security') renderSettingsModalBody();
    },
    startTotpSetup: async () => {
      try {
        state.settingsData.totpSetup = await api('/api/web/2fa/totp/start', { method: 'POST' });
        renderSettingsModalBody();
      } catch (err) {
        showToast(err.message);
      }
    },
    cancelTotpSetup: () => {
      state.settingsData.totpSetup = null;
      renderSettingsModalBody();
    },
    enableTotp: async (e) => {
      if (e) e.preventDefault();
      try {
        const res = await api('/api/web/2fa/totp/enable', {
          method: 'POST',
          body: JSON.stringify({ code: $('totp-code').value }),
        });
        state.settingsData.totpSetup = null;
        state.settingsData.recoveryCodes = res.recoveryCodes;
        if (state.user) state.user.twoFactorEnabled = true;
        showToast('Two-factor authentication enabled');
        await window.App.refreshSecurityStatus();
      } catch (err) {
        showToast(err.message);
      }
    },
    generateRecoveryCodes: async () => {
      try {
        const res = await api('/api/web/2fa/recovery', {
          method: 'POST',
          body: JSON.stringify({ code: securityCodeValue() || undefined }),
        });
        state.settingsData.recoveryCodes = res.recoveryCodes;
        renderSettingsModalBody();
        showToast('New recovery codes generated');
      } catch (err) {
        showToast(err.message);
      }
    },
    dismissRecoveryCodes: async () => {
      state.settingsData.recoveryCodes = null;
      await window.App.refreshSecurityStatus();
    },
    addPasskey: async () => {
      if (!passkeySupported()) {
        showToast('Passkeys are not supported by this browser');
        return;
      }
      try {
        const opts = await api('/api/web/2fa/passkey/options', { method: 'POST' });
        const credential = await navigator.credentials.create({
          publicKey: webauthnOptions(opts.publicKey, true),
        });
        if (!credential) throw new Error('Passkey registration was cancelled');
        const label = window.prompt('Name this passkey', 'This device') || 'Passkey';
        await api('/api/web/2fa/passkey/verify', {
          method: 'POST',
          body: JSON.stringify({
            challenge: opts.challenge,
            credential: webauthnCredentialToJson(credential),
            label,
          }),
        });
        showToast('Passkey added');
        await window.App.refreshSecurityStatus();
      } catch (err) {
        showToast(err.message);
      }
    },
    stepUpWithPasskey: async () => {
      if (!passkeySupported()) {
        showToast('Passkeys are not supported by this browser');
        return;
      }
      try {
        const opts = await api('/api/web/2fa/passkey/step-up/options', { method: 'POST' });
        const credential = await navigator.credentials.get({
          publicKey: webauthnOptions(opts.publicKey, false),
        });
        if (!credential) throw new Error('Passkey verification was cancelled');
        await api('/api/web/2fa/passkey/step-up/verify', {
          method: 'POST',
          body: JSON.stringify({ challenge: opts.challenge, credential: webauthnCredentialToJson(credential) }),
        });
        showToast('Verified with your passkey');
        const input = $('sec-code');
        if (input) input.value = '';
      } catch (err) {
        showToast(err.message);
      }
    },
    removePasskey: async (id) => {
      try {
        const code = securityCodeValue();
        const query = code ? `?code=${encodeURIComponent(code)}` : '';
        await api(`/api/web/2fa/passkey/${encodeURIComponent(id)}${query}`, { method: 'DELETE' });
        showToast('Passkey removed');
        await window.App.refreshSecurityStatus();
      } catch (err) {
        showToast(err.message);
      }
    },
    promptDisable2fa: () => {
      state.settingsData.confirmDisable2fa = true;
      renderSettingsModalBody();
    },
    cancelDisable2fa: () => {
      state.settingsData.confirmDisable2fa = false;
      renderSettingsModalBody();
    },
    confirmDisable2fa: async () => {
      try {
        await api('/api/web/2fa/disable', {
          method: 'POST',
          body: JSON.stringify({ code: securityCodeValue() || undefined }),
        });
        if (state.user) {
          state.user.twoFactorEnabled = false;
          state.user.totpEnabled = false;
          state.user.passkeys = 0;
        }
        state.settingsData.confirmDisable2fa = false;
        showToast('Two-factor authentication disabled');
        await window.App.refreshSecurityStatus();
      } catch (err) {
        showToast(err.message);
      }
    },
    promptDeleteAccount: () => {
      state.settingsData.confirmDelete = true;
      renderSettingsModalBody();
    },
    cancelDeleteAccount: () => {
      state.settingsData.confirmDelete = false;
      renderSettingsModalBody();
    },
    confirmDeleteAccount: async () => {
      const passwordInput = $('delete-password');
      if (!passwordInput || !passwordInput.value) {
        showToast('Enter your password to confirm');
        return;
      }
      try {
        const res = await api('/api/web/account/delete', {
          method: 'POST',
          body: JSON.stringify({
            password: passwordInput.value,
            code: securityCodeValue() || undefined,
          }),
        });
        state.settingsData.confirmDelete = false;
        showToast('Account scheduled for deletion. Sign in within 7 days to restore it.');
        await window.App.signOut();
      } catch (err) {
        showToast(err.message);
      }
    },
    copyText: async (text, label = 'Copied') => {
      try {
        if (navigator.clipboard && navigator.clipboard.writeText) {
          await navigator.clipboard.writeText(text);
          haptic('success');
          showToast(label);
          return;
        }
      } catch {}
      showToast('Select and copy manually');
    },
    toggleSaveAlbum: async (id, saved) => {
      haptic(saved ? 'success' : 'tap');
      try {
        await api('/api/web/star', {
          method: 'POST',
          body: JSON.stringify({ id, starred: saved, kind: 'album' }),
        });
        renderAlbum(id);
      } catch (e) {
        showToast(e.message);
      }
    },
    toggleFollowArtist: async (id, followed) => {
      haptic(followed ? 'success' : 'tap');
      try {
        await api('/api/web/star', {
          method: 'POST',
          body: JSON.stringify({ id, starred: followed, kind: 'artist' }),
        });
        renderArtist(id);
      } catch (e) {
        showToast(e.message);
      }
    },
    promptCreatePlaylist: (initialTrackId = null) => {
      openModal(`
        <div class="modal-head"><h3>Create Playlist</h3><button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button></div>
        <form onsubmit="App.submitCreatePlaylist(event, ${initialTrackId ? esc(JSON.stringify(initialTrackId)) : 'null'})" style="display:flex;flex-direction:column;gap:12px">
          <input id="new-pl-name" type="text" placeholder="Playlist name" required autofocus />
          <input id="new-pl-desc" type="text" placeholder="Description (optional)" />
          ${playlistVisibilityHtml(false)}
          <button type="submit" class="btn-primary">Create</button>
        </form>
      `);
    },
    promptAddToNewPlaylist: (tid) => window.App.promptCreatePlaylist(tid),
    submitCreatePlaylist: async (e, tid) => {
      e.preventDefault();
      const name = $('new-pl-name').value.trim();
      if (!name) {
        showToast('Please enter a playlist name');
        return;
      }
      const comment = $('new-pl-desc').value.trim();
      try {
        const res = await api('/api/web/playlists', {
          method: 'POST',
          body: JSON.stringify({ name, comment, public: selectedPlaylistVisibility(), trackIds: tid ? [tid] : [] }),
        });
        haptic('success');
        closeModal();
        await refreshSidebarPlaylists();
        showToast(`Created "${name}"`);
        navigate(`playlist/${encodeURIComponent(res.playlist.id)}`);
      } catch (err) {
        showToast(err.message);
      }
    },
    addTrackToPlaylist: async (pid, tid) => {
      try {
        const res = await api(`/api/web/playlists/${encodeURIComponent(pid)}/tracks`, {
          method: 'POST',
          body: JSON.stringify({ trackIds: [tid] }),
        });
        haptic('success');
        refreshSidebarPlaylists();
        showToast(res.added === 0 ? 'Already in playlist' : 'Added to playlist');
        const curPlHash = location.hash.startsWith('#playlist/')
          ? decodeURIComponent(location.hash.slice('#playlist/'.length))
          : '';
        if (curPlHash === pid) {
          renderPlaylist(pid);
        }
      } catch (err) {
        showToast(err.message);
      }
    },
    quickAddToPlaylist: async (pid, tid, btn) => {
      if (btn) btn.disabled = true;
      try {
        const res = await api(`/api/web/playlists/${encodeURIComponent(pid)}/tracks`, {
          method: 'POST',
          body: JSON.stringify({ trackIds: [tid] }),
        });
        haptic('success');
        refreshSidebarPlaylists();
        showToast(res.added === 0 ? 'Already in playlist' : 'Added to playlist');
        renderPlaylist(pid);
      } catch (e) {
        if (btn) btn.disabled = false;
        showToast(e.message);
      }
    },
    loadPlaylistRecs,
    startPlaylistRadio,
    setSearchFilter,
    toggleMixExpand,
    toggleArtistTracks: () => {
      if (!state.artistView || !Array.isArray(state.artistView.top)) return;
      state.artistView.expanded = !state.artistView.expanded;
      const { top, expanded } = state.artistView;
      const tbl = $('artist-popular-table');
      if (tbl) {
        tbl.innerHTML = renderTrackRows(expanded ? top : top.slice(0, 15), top);
      }
      const btn = $('artist-tracks-toggle');
      if (btn) {
        btn.textContent = expanded ? 'Show fewer' : `Show all (${top.length})`;
      }
    },
    runRefinedSearch,
    playSearchTrack,
    createInstantMixForSearch,
    promptInstantMix,
    submitInstantMixModal: (e) => {
      e.preventDefault();
      const val = ($('instant-mix-modal-input')?.value || '').trim();
      if (!val) return;
      closeModal();
      navigate(`mix?q=${encodeURIComponent(val)}`);
    },
    submitInstantMixForm: (e) => {
      e.preventDefault();
      const val = ($('instant-mix-term-input')?.value || '').trim();
      if (!val) return;
      const mode = state.instantMixMode && state.instantMixMode !== 'auto' ? `&mode=${encodeURIComponent(state.instantMixMode)}` : '';
      navigate(`mix?q=${encodeURIComponent(val)}${mode}`);
    },
    setInstantMixMode: (mode) => {
      const cleanMode = ['topic', 'entity'].includes(mode) ? mode : 'auto';
      state.instantMixMode = cleanMode;
      const val = ($('instant-mix-term-input')?.value || '').trim();
      if (val) {
        const modeParam = cleanMode !== 'auto' ? `&mode=${encodeURIComponent(cleanMode)}` : '';
        navigate(`mix?q=${encodeURIComponent(val)}${modeParam}`);
      } else {
        document.querySelectorAll('.instant-mix-mode-row .refine-query-pill').forEach((btn, idx) => {
          const keys = ['auto', 'topic', 'entity'];
          btn.classList.toggle('active', keys[idx] === cleanMode);
        });
      }
    },
    playInstantMix,
    playInstantMixAt,
    queueInstantMix,
    promptSaveInstantMix: () => {
      const active = getActiveInstantMix();
      if (!active || !active.mix || !Array.isArray(active.mix.tracks) || !active.mix.tracks.length) {
        showToast('No instant mix tracks to save');
        return;
      }
      const mix = active.mix;
      openModal(`
        <div class="modal-head"><h3>Save Instant Mix to Playlists</h3><button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button></div>
        <form onsubmit="App.submitSaveInstantMix(event)" style="display:flex;flex-direction:column;gap:12px">
          <input id="save-mix-name" type="text" value="${esc(mix.title || active.query || 'Instant Mix')}" placeholder="Playlist name" required autofocus />
          <input id="save-mix-desc" type="text" value="${esc(mix.subtitle || '')}" placeholder="Description (optional)" />
          <button type="submit" class="btn-primary">Save Playlist (${mix.tracks.length} tracks)</button>
        </form>
      `);
    },
    submitSaveInstantMix: async (e) => {
      e.preventDefault();
      const active = getActiveInstantMix();
      if (!active || !active.mix || !Array.isArray(active.mix.tracks) || !active.mix.tracks.length) {
        closeModal();
        showToast('No instant mix tracks to save');
        return;
      }
      const name = ($('save-mix-name')?.value || '').trim();
      if (!name) {
        showToast('Please enter a playlist name');
        return;
      }
      const comment = ($('save-mix-desc')?.value || '').trim();
      const trackIds = [...new Set(active.mix.tracks.map((t) => t.id).filter(Boolean))];
      try {
        const res = await api('/api/web/playlists', {
          method: 'POST',
          body: JSON.stringify({ name, comment, trackIds }),
        });
        active.mix.savedPlaylistId = res.playlist.id;
        haptic('success');
        closeModal();
        await refreshSidebarPlaylists();
        refreshActiveInstantMixCard();
        showToast(`Saved "${name}" to playlists`);
      } catch (err) {
        showToast(err.message);
      }
    },
    removePlaylistItem: async (pid, pos) => {
      try {
        await api(`/api/web/playlists/${encodeURIComponent(pid)}/tracks/${pos}`, { method: 'DELETE' });
        renderPlaylist(pid);
      } catch (err) {
        showToast(err.message);
      }
    },
    movePlaylistItem: async (pid, from, to) => {
      try {
        await api(`/api/web/playlists/${encodeURIComponent(pid)}/reorder`, {
          method: 'POST',
          body: JSON.stringify({ from, to }),
        });
        renderPlaylist(pid);
      } catch (err) {
        showToast(err.message);
      }
    },
    promptEditPlaylist: (pid, curName, curComment = '', curPublic = false) => {
      openModal(`
        <div class="modal-head"><h3>Edit Playlist</h3><button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button></div>
        <form onsubmit="App.submitEditPlaylist(event, ${esc(JSON.stringify(pid))})" style="display:flex;flex-direction:column;gap:12px">
          <input id="edit-pl-name" type="text" value="${esc(curName)}" placeholder="Playlist name" required />
          <input id="edit-pl-desc" type="text" value="${esc(curComment || '')}" placeholder="Description (optional)" />
          ${playlistVisibilityHtml(!!curPublic)}
          <button type="submit" class="btn-primary">Save changes</button>
        </form>
      `);
    },
    // Kept as an alias for any older callers.
    promptRenamePlaylist: (pid, curName) => window.App.promptEditPlaylist(pid, curName),
    submitEditPlaylist: async (e, pid) => {
      e.preventDefault();
      const name = $('edit-pl-name')?.value.trim();
      if (!name) {
        showToast('Please enter a playlist name');
        return;
      }
      try {
        await api(`/api/web/playlists/${encodeURIComponent(pid)}`, {
          method: 'PATCH',
          body: JSON.stringify({
            name,
            comment: $('edit-pl-desc')?.value.trim() || '',
            public: selectedPlaylistVisibility(),
          }),
        });
        closeModal();
        await refreshSidebarPlaylists();
        renderPlaylist(pid);
        showToast('Playlist updated');
      } catch (err) {
        showToast(err.message);
      }
    },
    submitRenamePlaylist: async (e, pid) => window.App.submitEditPlaylist(e, pid),
    togglePinPlaylist: (pid) => {
      const ids = getPinnedPlaylistIds();
      const idx = ids.indexOf(pid);
      let pinnedNow = false;
      if (idx >= 0) {
        ids.splice(idx, 1);
      } else {
        ids.push(pid);
        pinnedNow = true;
      }
      savePinnedPlaylistIds(ids);
      renderSidebarPlaylistsDom();
      const heroBtn = $('pl-hero-pin-btn');
      if (heroBtn) {
        heroBtn.innerHTML = `${ICONS.pin} ${pinnedNow ? 'Pinned' : 'Pin to Sidebar'}`;
      }
      const curHash = location.hash.replace(/^#/, '');
      if (curHash === 'playlists') {
        renderPlaylistsView();
      } else if (curHash === 'library' && state.libTab === 'playlists') {
        renderLibrary('playlists');
      }
      showToast(pinnedNow ? 'Pinned to sidebar' : 'Unpinned from sidebar');
    },
    deletePlaylist: async (pid) => {
      const pl = state.playlists?.find((p) => p.id === pid);
      const msg = pl?.name
        ? `Are you sure you want to delete "${pl.name}"?`
        : 'Are you sure you want to delete this playlist?';
      const confirmed = await confirmModal({
        title: 'Delete Playlist',
        message: msg,
        confirmLabel: 'Delete',
        danger: true,
      });
      if (!confirmed) return;
      try {
        await api(`/api/web/playlists/${encodeURIComponent(pid)}`, { method: 'DELETE' });
        haptic('warning');
        const pinnedIds = getPinnedPlaylistIds().filter((id) => id !== pid);
        savePinnedPlaylistIds(pinnedIds);
        if (state.activeInstantMix?.mix?.savedPlaylistId === pid) {
          delete state.activeInstantMix.mix.savedPlaylistId;
        }
        if (state.searchState?.instantMix?.savedPlaylistId === pid) {
          delete state.searchState.instantMix.savedPlaylistId;
        }
        refreshActiveInstantMixCard();
        await refreshSidebarPlaylists();
        showToast('Playlist deleted');
        const curHash = location.hash.replace(/^#/, '');
        if (curHash === `playlist/${encodeURIComponent(pid)}` || curHash === `playlist/${pid}`) {
          navigate('playlists');
        } else if (curHash === 'playlists') {
          renderPlaylistsView();
        } else if (curHash === 'library' || curHash === '') {
          renderLibrary(state.libTab);
        }
      } catch (err) {
        showToast(err.message);
      }
    },
    closeModal,
    installPwa,
    regenApiKey: async () => {
      try {
        const codeInput = $('apikey-code');
        const res = await api('/api/web/account/apikey', {
          method: 'POST',
          body: JSON.stringify({ code: codeInput ? codeInput.value : undefined }),
        });
        state.user.apiKey = res.apiKey;
        const el = $('modal-apikey');
        if (el) el.textContent = res.apiKey;
        showToast('API Key regenerated');
      } catch (err) {
        showToast(err.message);
      }
    },
    submitPasswordChange: async (e) => {
      e.preventDefault();
      try {
        await api('/api/web/account/password', {
          method: 'POST',
          body: JSON.stringify({
            currentPassword: $('pw-cur').value,
            newPassword: $('pw-new').value,
            code: $('pw-code') ? $('pw-code').value : undefined,
          }),
        });
        showToast('Password updated');
        closeModal();
      } catch (err) {
        showToast(err.message);
      }
    },
  };

  init();
})();
