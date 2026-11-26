(() => {
  'use strict';

  const audio = document.getElementById('audio-el');

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
    lock: '<svg class="vis-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4.5" y="11" width="15" height="10" rx="2"/><path d="M8 11V7.5a4 4 0 0 1 8 0V11"/></svg>',
    globe: '<svg class="vis-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M3 12h18"/><path d="M12 3a14 14 0 0 1 0 18a14 14 0 0 1 0-18z"/></svg>',
    pin: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/></svg>',
    playlistAdd: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M11 12H3"/><path d="M16 6H3"/><path d="M16 18H3"/><path d="M18 9v6"/><path d="M21 12h-6"/></svg>',
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

  const RECENT_SEARCH_LIMIT = 6;

  function recentSearchesStorageKey() {
    return `beatlab_recent_searches_${state.user?.id || 'anon'}`;
  }

  function getRecentSearches() {
    try {
      const raw = JSON.parse(localStorage.getItem(recentSearchesStorageKey()) || '[]');
      return Array.isArray(raw) ? raw.filter((q) => typeof q === 'string' && q).slice(0, RECENT_SEARCH_LIMIT) : [];
    } catch {
      return [];
    }
  }

  function saveRecentSearches(list) {
    try {
      localStorage.setItem(recentSearchesStorageKey(), JSON.stringify(list.slice(0, RECENT_SEARCH_LIMIT)));
    } catch {}
  }

  function addRecentSearch(query) {
    const q = String(query || '').trim();
    if (!q) return;
    const lower = q.toLowerCase();
    saveRecentSearches([q, ...getRecentSearches().filter((s) => s.toLowerCase() !== lower)]);
  }

  function removeRecentSearch(query) {
    saveRecentSearches(getRecentSearches().filter((s) => s !== query));
  }

  function hideRecentSearches() {
    const box = document.getElementById('recent-searches');
    if (box) box.classList.add('hidden');
  }

  function showRecentSearches() {
    const box = document.getElementById('recent-searches');
    const input = document.getElementById('global-search');
    if (!box || !input) return;
    const list = getRecentSearches();
    if (!list.length || (input.value || '').trim()) {
      box.classList.add('hidden');
      return;
    }
    box.innerHTML =
      `<div class="recent-searches-head">Recent searches</div>` +
      list
        .map(
          (q, i) => `
        <div class="recent-search-item" role="option" data-idx="${i}">
          <svg class="recent-search-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></svg>
          <span class="recent-search-text">${esc(q)}</span>
          <button class="recent-search-remove" type="button" title="Remove" data-remove="${i}">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12"/></svg>
          </button>
        </div>`
        )
        .join('');
    box.classList.remove('hidden');
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

  const RIGHT_PANEL_CLOSED_KEY = 'beatlab_right_panel_closed';
  const MQ_MOBILE = window.matchMedia('(max-width: 780px)');
  const MQ_TABLET = window.matchMedia('(max-width: 1060px)');
  const isMobileViewport = () => MQ_MOBILE.matches;
  const isTabletOrMobileViewport = () => MQ_TABLET.matches;

  function loadDesktopRightPanelClosed() {
    try {
      return localStorage.getItem(RIGHT_PANEL_CLOSED_KEY) === 'true';
    } catch {
      return false;
    }
  }

  function saveDesktopRightPanelClosed(closed) {
    try {
      localStorage.setItem(RIGHT_PANEL_CLOSED_KEY, closed ? 'true' : 'false');
    } catch {}
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
    desktopRightPanelClosed: loadDesktopRightPanelClosed(),
    radioReqToken: 0,
    radioLoading: false,
    radioInFlight: null,
    radioError: false,
    queueRestoreFailed: false,
    compatAttemptedId: '',
    isSwitchingTrack: false,
    switchGuardUntil: 0,
    intentionalPause: false,
    hiddenWhilePlayingAt: 0,
    hiddenResumeAttempted: false,
    networkRetryTimer: null,
    networkRetryCount: 0,
    pendingNetworkResume: null,
    stallTimer: null,
    lastPositionStateAt: 0,
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
      // Full stats payload shown in the profile Stats tab (keyed by user + range).
      profileStatsData: null,
      profileStatsKey: '',
      profileStatsRange: 'all',
      profileStatsReq: 0,
      editColor: '',
      // null = untouched, a data URL = pending new picture, 'REMOVE' = clear.
      editAvatarData: null,
    },
    settingsTab: 'general',
    settingsData: {
      hidden: null,
      sessions: null,
      security: null,
      curator: null,
      curatorSaving: false,
      totpSetup: null,
      recoveryCodes: null,
      confirmResetHidden: false,
      confirmRevokeAll: false,
      confirmDisable2fa: false,
      confirmDelete: false,
      showPasswordForm: false,
    },
    authMode: 'signin',
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
    marqueeTimer: null,
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
    if (meta) meta.setAttribute('content', next === 'light' ? '#ffffff' : '#0b0b0d');
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
    if (state.user && state.queueRestoreFailed && state.queue.length === 0) {
      await restoreServerQueue();
    }
    resumePendingNetworkPlayback();
    ensureUpcomingRadio();
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
      checkServerHealth().then((ok) => {
        if (ok) {
          resumePendingNetworkPlayback();
          const cur = currentTrack();
          if (cur && !mediaSessionState.artwork) {
            syncMediaSession(cur, false);
          }
        }
      });
    });

    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'hidden') {
        const cur = currentTrack();
        if (cur && !audio.paused && !audio.ended) {
          state.hiddenWhilePlayingAt = Date.now();
          state.hiddenResumeAttempted = false;
          // Re-assert active MediaSession state right before Android backgrounds the PWA.
          setMediaPlaybackState('playing');
          updateMediaPositionState(true);
          if (state.queueSaveDirty) flushServerQueue(true);
        } else {
          state.hiddenWhilePlayingAt = 0;
        }
        return;
      }

      if (document.visibilityState === 'visible') {
        state.hiddenWhilePlayingAt = 0;
        state.hiddenResumeAttempted = false;
        checkServerHealth();
        if (state.pendingNetworkResume && navigator.onLine !== false) {
          resumePendingNetworkPlayback();
        }
        const cur = currentTrack();
        if (cur) {
          if (!audio.paused && !audio.ended) {
            updatePlayButton(true);
            setMediaPlaybackState('playing');
            updateMediaPositionState(true);
            syncActiveLyricLine((audio.currentTime || 0) * 1000);
          } else {
            // Android SystemUI can drop Chrome's idle media notification while backgrounded;
            // force the next play() to republish full metadata & handlers.
            mediaSessionState.publishedWhilePlaying = '';
          }
        }
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
      '/api/web/auth/captcha',
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
      const httpErr = new Error(data.error || `Request failed (${resp.status})`);
      httpErr.status = resp.status;
      throw httpErr;
    }
    return data;
  }

  // POSTs to an endpoint that may require a fresh second factor. When the
  // server answers 403 asking for one, prompt for an authenticator/recovery
  // code and retry once. Resolves to null if the user cancels the prompt.
  async function apiWithStepUp(path, promptText) {
    const first = securityCodeValue();
    try {
      return await api(path, {
        method: 'POST',
        body: JSON.stringify({ code: first || undefined }),
      });
    } catch (err) {
      if (err.status !== 403 || !/two-factor|recovery code/i.test(err.message || '')) throw err;
    }
    const code = await promptModal({
      title: 'Confirm your identity',
      message: promptText,
      label: '2FA or recovery code',
      placeholder: '000000',
      confirmLabel: 'Continue',
    });
    if (code === null) return null;
    return api(path, { method: 'POST', body: JSON.stringify({ code }) });
  }

  function cacheTracks(list) {
    if (!Array.isArray(list)) return [];
    for (const t of list) {
      if (t && t.id) {
        const prev = state.trackMap.get(t.id);
        if (prev) {
          if (!t.year && prev.year > 0) t.year = prev.year;
          if (!t.genre && prev.genre) t.genre = prev.genre;
        }
        state.trackMap.set(t.id, t);
      }
    }
    return list;
  }

  function currentTrack() {
    if (state.queueIndex >= 0 && state.queueIndex < state.queue.length) {
      return state.queue[state.queueIndex];
    }
    return null;
  }

  function timeOfDaySlot() {
    const h = new Date().getHours();
    if (h >= 5 && h < 12) return 'morning';
    if (h >= 12 && h < 17) return 'afternoon';
    if (h >= 17 && h < 22) return 'evening';
    return 'lateNight';
  }

  function greetingText() {
    const h = new Date().getHours();
    if (h < 12) return 'Good morning';
    if (h < 18) return 'Good afternoon';
    return 'Good evening';
  }

  function hasHardcodedClockHour(text) {
    return /\bo['’]?clock\b|\b(?:1[0-2]|0?[1-9])(?::\d{2})?\s*(?:a\.?m\.?|p\.?m\.?)\b/i.test(text);
  }

  function resolveHomeGreeting(llm) {
    const fallback = `${greetingText()}, ${state.user.username}`;
    if (!llm) return fallback;
    const slot = timeOfDaySlot();
    const slotted = llm.homeGreetings && llm.homeGreetings[slot]
      ? String(llm.homeGreetings[slot]).trim()
      : '';
    if (slotted && !hasHardcodedClockHour(slotted)) return slotted;
    const legacy = llm.homeGreeting ? String(llm.homeGreeting).trim() : '';
    if (legacy && !hasHardcodedClockHour(legacy)) return legacy;
    return fallback;
  }

  // ---------------------------------------------------------------------------
  // Playback output
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

  function updateMediaPositionState(force = false) {
    if (!('mediaSession' in navigator) || typeof navigator.mediaSession.setPositionState !== 'function') return;
    const nowMs = typeof performance !== 'undefined' ? performance.now() : Date.now();
    if (!force && nowMs - state.lastPositionStateAt < 4000) return;
    const cur = currentTrack();
    const duration = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : cur ? cur.duration : 0;
    const rawPos =
      Number.isFinite(audio.currentTime) && audio.currentTime > 0
        ? audio.currentTime
        : state.pendingSeekSec > 0
          ? state.pendingSeekSec
          : 0;
    const position = Math.max(0, rawPos);
    if (!Number.isFinite(duration) || duration <= 0) return;
    try {
      navigator.mediaSession.setPositionState({
        duration,
        playbackRate: Number.isFinite(audio.playbackRate) && audio.playbackRate > 0 ? audio.playbackRate : 1,
        position: Math.min(position, duration),
      });
      state.lastPositionStateAt = nowMs;
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
    if (next) {
      const cur = currentTrack();
      if (cur) {
        setMarqueeText($('fs-title'), cur.title);
        const fsNow = $('fs-queue-now-playing');
        if (fsNow) setMarqueeText(fsNow.querySelector('.q-item.active .q-item-title'), cur.title);
      }
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

  let rpContentRaf = 0;

  function scheduleRightPanelContentSync(scrollBehavior = 'smooth') {
    if (rpContentRaf) cancelAnimationFrame(rpContentRaf);
    rpContentRaf = requestAnimationFrame(() => {
      rpContentRaf = 0;
      const rp = $('right-panel');
      const shell = $('app-shell');
      if (!rp || rp.classList.contains('hidden') || !shell || shell.classList.contains('hidden')) return;
      const queueBody = $('rp-queue-body');
      if (queueBody && !queueBody.classList.contains('hidden')) {
        const cur = currentTrack();
        if (cur) {
          setMarqueeText($('queue-now-playing')?.querySelector('.q-item.active .q-item-title'), cur.title);
        }
      }
      const lyricsBody = $('rp-lyrics-body');
      if (lyricsBody && !lyricsBody.classList.contains('hidden') && state.activeLyricIdx >= 0) {
        const activeLine = $('lyrics-container')?.querySelector(`.lyric-line[data-lidx="${state.activeLyricIdx}"]`);
        if (activeLine) {
          activeLine.scrollIntoView({ behavior: scrollBehavior, block: 'center', inline: 'nearest' });
        }
      }
    });
  }

  function syncResponsiveLayout() {
    const isMobile = isMobileViewport();
    const isTabletOrMobile = isTabletOrMobileViewport();
    const searchInput = $('global-search');
    if (searchInput) {
      searchInput.placeholder = isMobile
        ? 'Search music'
        : 'Search songs, artists, albums... (Press /)';
    }
    const rp = $('right-panel');
    const backdrop = $('panel-backdrop');
    if (!rp) return;
    const wasHidden = rp.classList.contains('hidden');
    // Breakpoint transitions switch between docked grid column and fixed
    // overlay drawer; snap cleanly instead of animating across layout modes.
    document.body.classList.remove('rp-anim');
    if (isMobile) {
      delete rp.dataset.tabletToggled;
      rp.classList.add('hidden');
      if (backdrop) backdrop.classList.add('hidden');
    } else if (isTabletOrMobile) {
      if (!rp.dataset.tabletToggled) {
        rp.classList.add('hidden');
      }
      if (backdrop) backdrop.classList.toggle('hidden', rp.classList.contains('hidden'));
    } else {
      const keepTabletOpen = rp.dataset.tabletToggled === '1' && !wasHidden;
      delete rp.dataset.tabletToggled;
      if (backdrop) backdrop.classList.add('hidden');
      rp.classList.toggle('hidden', keepTabletOpen ? false : state.desktopRightPanelClosed);
    }
    syncRightPanelState();
    if (wasHidden && !rp.classList.contains('hidden')) {
      scheduleRightPanelContentSync('auto');
    }
  }

  function updateVolumeIcon() {
    const slider = $('pb-volume');
    if (slider) {
      const level = state.muted ? 0 : Math.max(0, Math.min(1, Number(state.volume) || 0));
      slider.style.setProperty('--pct', `${level * 100}%`);
    }
    const icon = $('vol-icon');
    if (!icon) return;
    icon.innerHTML = state.muted || state.volume === 0 ? ICONS.volMuted : ICONS.volOn;
  }

  // ---------------------------------------------------------------------------
  // Smart Proof-of-Work Captcha & Browser Integrity Check
  // ---------------------------------------------------------------------------
  const POW_SHA256_CORE = `
    const K = new Int32Array([
      0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
      0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
      0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
      0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
      0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
      0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
      0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
      0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
    ]);
    const W = new Int32Array(64);
    const BUF = new Uint8Array(192);
    function sha256FirstWordAscii(prefixBytes, prefixLen, nonceStr) {
      const nLen = nonceStr.length;
      const len = prefixLen + nLen;
      const total = ((len + 9 + 63) >> 6) << 6;
      for (let i = 0; i < prefixLen; i++) BUF[i] = prefixBytes[i];
      for (let i = 0; i < nLen; i++) BUF[prefixLen + i] = nonceStr.charCodeAt(i) & 0xff;
      BUF[len] = 0x80;
      for (let i = len + 1; i < total - 4; i++) BUF[i] = 0;
      const bitLen = len * 8;
      BUF[total - 4] = (bitLen >>> 24) & 0xff;
      BUF[total - 3] = (bitLen >>> 16) & 0xff;
      BUF[total - 2] = (bitLen >>> 8) & 0xff;
      BUF[total - 1] = bitLen & 0xff;

      let h0 = 0x6a09e667 | 0, h1 = 0xbb67ae85 | 0, h2 = 0x3c6ef372 | 0, h3 = 0xa54ff53a | 0;
      let h4 = 0x510e527f | 0, h5 = 0x9b05688c | 0, h6 = 0x1f83d9ab | 0, h7 = 0x5be0cd19 | 0;

      for (let off = 0; off < total; off += 64) {
        for (let i = 0; i < 16; i++) {
          const j = off + (i << 2);
          W[i] = (BUF[j] << 24) | (BUF[j + 1] << 16) | (BUF[j + 2] << 8) | BUF[j + 3];
        }
        for (let i = 16; i < 64; i++) {
          const w15 = W[i - 15], w2 = W[i - 2];
          const s0 = ((w15 >>> 7) | (w15 << 25)) ^ ((w15 >>> 18) | (w15 << 14)) ^ (w15 >>> 3);
          const s1 = ((w2 >>> 17) | (w2 << 15)) ^ ((w2 >>> 19) | (w2 << 13)) ^ (w2 >>> 10);
          W[i] = (W[i - 16] + s0 + W[i - 7] + s1) | 0;
        }
        let a = h0, b = h1, c = h2, d = h3, e = h4, f = h5, g = h6, h = h7;
        for (let i = 0; i < 64; i++) {
          const S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
          const ch = (e & f) ^ (~e & g);
          const t1 = (h + S1 + ch + K[i] + W[i]) | 0;
          const S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
          const maj = (a & b) ^ (a & c) ^ (b & c);
          const t2 = (S0 + maj) | 0;
          h = g; g = f; f = e; e = (d + t1) | 0;
          d = c; c = b; b = a; a = (t1 + t2) | 0;
        }
        h0 = (h0 + a) | 0; h1 = (h1 + b) | 0; h2 = (h2 + c) | 0; h3 = (h3 + d) | 0;
        h4 = (h4 + e) | 0; h5 = (h5 + f) | 0; h6 = (h6 + g) | 0; h7 = (h7 + h) | 0;
      }
      return h0 >>> 0;
    }
    function matchesDifficulty(h0, bits) {
      if (bits <= 0) return true;
      if (bits >= 32) return h0 === 0;
      return (h0 >>> (32 - bits)) === 0;
    }
  `;

  const captchaState = {
    enabled: true,
    status: 'idle', // idle | checking | interactive | solving | verified | error
    action: 'signin',
    challenge: null,
    solution: null,
    interacted: false,
    runId: 0,
    worker: null,
    workerUrl: '',
    fallbackTimer: null,
    expiryTimer: null,
    autoStartTimer: null,
    inFlightPromise: null,
    workerSupported: typeof Worker === 'function' && typeof Blob === 'function',
  };

  function computeBrowserProbe(probe) {
    if (!probe) return 0;
    return ((Math.imul(probe.a | 0, probe.b | 0) ^ (probe.c | 0)) >>> 0);
  }

  function detectAutomationGlobals() {
    try {
      const winKeys = [
        '__playwright',
        '__pw_manual',
        '__PW_inspect',
        '__nightmare',
        '_phantom',
        'callPhantom',
        'domAutomation',
        'domAutomationController',
        '__selenium_unwrapped',
        '__webdriver_evaluate',
        '__driver_evaluate',
      ];
      for (const k of winKeys) {
        if (k in window && window[k]) return true;
      }
      if (document) {
        for (const k of Object.keys(document)) {
          if (k.startsWith('$cdc_') || k.startsWith('$wdc_')) return true;
        }
      }
    } catch {}
    return false;
  }

  function checkCanvasRender() {
    try {
      const c = document.createElement('canvas');
      c.width = 16;
      c.height = 16;
      const ctx = c.getContext && c.getContext('2d');
      if (!ctx) return false;
      ctx.fillStyle = '#1ed760';
      ctx.fillRect(2, 2, 12, 12);
      const data = ctx.getImageData(4, 4, 1, 1).data;
      return data && data.length === 4 && data[3] > 0;
    } catch {
      return false;
    }
  }

  function collectBrowserSignals() {
    const ua = String(navigator.userAgent || '');
    const headlessUa = /HeadlessChrome|PhantomJS|Playwright|Puppeteer|Selenium/i.test(ua);
    const card = document.querySelector('.auth-card');
    const rect = card && card.getBoundingClientRect ? card.getBoundingClientRect() : null;
    const layoutOk = !!(rect && rect.width > 100 && rect.height > 100);
    return {
      webdriver: navigator.webdriver === true,
      automation: detectAutomationGlobals(),
      headlessUa,
      languages: Array.isArray(navigator.languages) ? navigator.languages.length : (navigator.language ? 1 : 0),
      screenW: (window.screen && window.screen.width) | 0,
      screenH: (window.screen && window.screen.height) | 0,
      innerW: window.innerWidth | 0,
      innerH: window.innerHeight | 0,
      cores: (navigator.hardwareConcurrency || 0) | 0,
      canvasOk: checkCanvasRender(),
      layoutOk,
      workerOk: captchaState.workerSupported,
      signalsPresent: true,
    };
  }

  function setCaptchaProgress(pct) {
    const fill = $('auth-captcha-progress-fill');
    if (!fill) return;
    const clamped = Math.max(0, Math.min(100, Number(pct) || 0));
    fill.style.width = `${clamped}%`;
  }

  function setCaptchaUi(status, label, sub, progressPct) {
    captchaState.status = status;
    const root = $('auth-captcha');
    const btn = $('auth-captcha-btn');
    const lblEl = $('auth-captcha-label');
    const subEl = $('auth-captcha-sub');
    if (root) {
      root.classList.toggle('hidden', !captchaState.enabled);
      root.dataset.state = status;
    }
    if (btn) {
      btn.setAttribute('aria-checked', status === 'verified' ? 'true' : 'false');
    }
    if (lblEl && label !== undefined) lblEl.textContent = label;
    if (subEl && sub !== undefined) subEl.textContent = sub;
    if (progressPct !== undefined) setCaptchaProgress(progressPct);
  }

  function cancelCaptchaWork() {
    captchaState.runId += 1;
    captchaState.inFlightPromise = null;
    if (captchaState.autoStartTimer) {
      clearTimeout(captchaState.autoStartTimer);
      captchaState.autoStartTimer = null;
    }
    if (captchaState.expiryTimer) {
      clearTimeout(captchaState.expiryTimer);
      captchaState.expiryTimer = null;
    }
    if (captchaState.fallbackTimer) {
      clearTimeout(captchaState.fallbackTimer);
      captchaState.fallbackTimer = null;
    }
    if (captchaState.worker) {
      try { captchaState.worker.terminate(); } catch {}
      captchaState.worker = null;
    }
    if (captchaState.workerUrl) {
      try { URL.revokeObjectURL(captchaState.workerUrl); } catch {}
      captchaState.workerUrl = '';
    }
  }

  function solvePowChallenge(challenge, probeAnswer, runId, onProgress) {
    const prefix = `${challenge.id}:${challenge.salt}:${probeAnswer}:`;
    const difficulty = challenge.difficulty | 0;
    const expectedTries = Math.max(1, Math.pow(2, difficulty));
    const startedAt = performance.now ? performance.now() : Date.now();

    const runMainThreadFallback = () =>
      new Promise((resolve, reject) => {
        const solverFn = new Function(
          'prefix',
          'difficulty',
          'startNonce',
          'batchSize',
          `${POW_SHA256_CORE}
          const prefixLen = prefix.length;
          const prefixBytes = new Uint8Array(prefixLen);
          for (let i = 0; i < prefixLen; i++) prefixBytes[i] = prefix.charCodeAt(i) & 0xff;
          for (let i = 0; i < batchSize; i++) {
            const nonce = startNonce + i;
            if (matchesDifficulty(sha256FirstWordAscii(prefixBytes, prefixLen, String(nonce)), difficulty)) {
              return nonce;
            }
          }
          return -1;`
        );
        let nonce = 0;
        const batchSize = 4096;
        const step = () => {
          if (runId !== captchaState.runId) {
            reject(new Error('cancelled'));
            return;
          }
          const found = solverFn(prefix, difficulty, nonce, batchSize);
          if (found >= 0) {
            const elapsed = Math.round((performance.now ? performance.now() : Date.now()) - startedAt);
            resolve({ nonce: found, elapsedMs: Math.max(1, elapsed) });
            return;
          }
          nonce += batchSize;
          if (onProgress) onProgress(Math.min(92, Math.round((1 - Math.exp(-nonce / expectedTries)) * 100)));
          captchaState.fallbackTimer = setTimeout(step, 0);
        };
        step();
      });

    if (!captchaState.workerSupported) {
      return runMainThreadFallback();
    }

    return new Promise((resolve, reject) => {
      try {
        const workerSrc = `
          ${POW_SHA256_CORE}
          self.onmessage = function(e) {
            const { prefix, difficulty } = e.data;
            const prefixLen = prefix.length;
            const prefixBytes = new Uint8Array(prefixLen);
            for (let i = 0; i < prefixLen; i++) prefixBytes[i] = prefix.charCodeAt(i) & 0xff;
            const t0 = performance.now();
            let nonce = 0;
            while (true) {
              for (let i = 0; i < 8192; i++) {
                const h0 = sha256FirstWordAscii(prefixBytes, prefixLen, String(nonce));
                if (matchesDifficulty(h0, difficulty)) {
                  self.postMessage({ type: 'done', nonce, elapsedMs: Math.max(1, Math.round(performance.now() - t0)) });
                  return;
                }
                nonce++;
              }
              self.postMessage({ type: 'progress', tried: nonce });
            }
          };
        `;
        const blob = new Blob([workerSrc], { type: 'application/javascript' });
        const url = URL.createObjectURL(blob);
        const worker = new Worker(url);
        captchaState.worker = worker;
        captchaState.workerUrl = url;

        const cleanup = () => {
          if (captchaState.worker === worker) {
            captchaState.worker = null;
          }
          try { worker.terminate(); } catch {}
          if (captchaState.workerUrl === url) {
            captchaState.workerUrl = '';
          }
          try { URL.revokeObjectURL(url); } catch {}
        };

        worker.onmessage = (ev) => {
          if (runId !== captchaState.runId) {
            cleanup();
            reject(new Error('cancelled'));
            return;
          }
          const msg = ev.data || {};
          if (msg.type === 'progress') {
            if (onProgress) {
              onProgress(Math.min(94, Math.round((1 - Math.exp(-msg.tried / expectedTries)) * 100)));
            }
          } else if (msg.type === 'done') {
            cleanup();
            resolve({ nonce: msg.nonce, elapsedMs: msg.elapsedMs });
          }
        };

        worker.onerror = () => {
          cleanup();
          captchaState.workerSupported = false;
          if (runId !== captchaState.runId) {
            reject(new Error('cancelled'));
            return;
          }
          runMainThreadFallback().then(resolve, reject);
        };

        worker.postMessage({ prefix, difficulty });
      } catch {
        captchaState.workerSupported = false;
        runMainThreadFallback().then(resolve, reject);
      }
    });
  }

  function captchaSolutionFresh(action) {
    if (!captchaState.enabled) return true;
    if (captchaState.status !== 'verified' || !captchaState.solution || !captchaState.challenge) {
      return false;
    }
    if (captchaState.action !== action) return false;
    const nowSec = Math.floor(Date.now() / 1000);
    return nowSec < (captchaState.challenge.expiresAt || 0) - 10;
  }

  async function solveCurrentChallenge(runId, userTriggered) {
    const ch = captchaState.challenge;
    if (!ch) return null;
    if (userTriggered) captchaState.interacted = true;
    setCaptchaUi('solving', 'Verifying browser…', 'Solving proof-of-work…', 18);
    const probeAnswer = computeBrowserProbe(ch.probe);
    const solved = await solvePowChallenge(ch, probeAnswer, runId, (pct) => {
      if (runId === captchaState.runId) {
        setCaptchaProgress(18 + Math.round(pct * 0.78));
      }
    });
    if (runId !== captchaState.runId) return null;

    captchaState.solution = {
      id: ch.id,
      nonce: solved.nonce,
      mac: ch.mac,
      probe: probeAnswer,
      interacted: captchaState.interacted,
      elapsedMs: solved.elapsedMs,
    };
    setCaptchaUi('verified', 'Verified', `Browser check passed · ${solved.elapsedMs} ms`, 100);

    const ttlMs = Math.max(15_000, ((ch.expiresAt || 0) - Math.floor(Date.now() / 1000) - 15) * 1000);
    captchaState.expiryTimer = setTimeout(() => {
      if (runId !== captchaState.runId) return;
      captchaState.solution = null;
      captchaState.challenge = null;
      setCaptchaUi('idle', "Verify you're human", 'Verification expired — click to refresh', 0);
    }, ttlMs);

    return captchaState.solution;
  }

  function startCaptchaFlow(action, { userTriggered = false } = {}) {
    cancelCaptchaWork();
    const runId = captchaState.runId;
    captchaState.action = action === 'signup' ? 'signup' : 'signin';
    captchaState.challenge = null;
    captchaState.solution = null;
    captchaState.interacted = Boolean(userTriggered);

    const promise = (async () => {
      setCaptchaUi('checking', 'Checking browser…', 'Inspecting browser environment', 10);
      try {
        const signals = collectBrowserSignals();
        const ch = await api('/api/web/auth/captcha', {
          method: 'POST',
          body: JSON.stringify({ action: captchaState.action, signals }),
        });
        if (runId !== captchaState.runId) return null;
        if (ch && ch.enabled === false) {
          captchaState.enabled = false;
          setCaptchaUi('verified', 'Verified', '', 100);
          return null;
        }
        captchaState.enabled = true;
        captchaState.challenge = ch;

        if (ch.interactive && !captchaState.interacted) {
          setCaptchaUi(
            'interactive',
            "Click to verify you're human",
            'Additional browser verification required',
            0
          );
          return null;
        }

        return await solveCurrentChallenge(runId, captchaState.interacted);
      } catch (err) {
        if (runId !== captchaState.runId) return null;
        const msg = err && err.message ? err.message : 'Could not reach verification service';
        setCaptchaUi('error', 'Verification failed', `${msg} — tap to retry`, 0);
        return null;
      } finally {
        if (captchaState.inFlightPromise === promise) {
          captchaState.inFlightPromise = null;
        }
      }
    })();

    captchaState.inFlightPromise = promise;
    return promise;
  }

  function resetCaptchaAndStart(action, delayMs = 120) {
    cancelCaptchaWork();
    captchaState.action = action === 'signup' ? 'signup' : 'signin';
    captchaState.challenge = null;
    captchaState.solution = null;
    captchaState.interacted = false;
    setCaptchaUi('idle', "Verify you're human", 'Browser & proof-of-work check', 0);
    const authScreen = $('auth-screen');
    if (!authScreen || authScreen.classList.contains('hidden')) return;
    const targetAction = captchaState.action;
    captchaState.autoStartTimer = setTimeout(() => {
      captchaState.autoStartTimer = null;
      if ($('auth-screen') && !$('auth-screen').classList.contains('hidden') && state.authMode === targetAction) {
        startCaptchaFlow(targetAction, { userTriggered: false });
      }
    }, delayMs);
  }

  async function onCaptchaButtonClick(e) {
    const trusted = Boolean(e && e.isTrusted);
    const action = state.authMode === 'signup' ? 'signup' : 'signin';
    if (captchaSolutionFresh(action)) return;
    if (captchaState.status === 'checking' || captchaState.status === 'solving') {
      if (trusted) captchaState.interacted = true;
      return;
    }
    if (captchaState.status === 'interactive' && captchaState.challenge) {
      const nowSec = Math.floor(Date.now() / 1000);
      if (nowSec < (captchaState.challenge.expiresAt || 0) - 15) {
        const runId = captchaState.runId;
        const promise = (async () => {
          try {
            return await solveCurrentChallenge(runId, trusted);
          } catch (err) {
            if (runId !== captchaState.runId) return null;
            setCaptchaUi('error', 'Verification failed', 'Tap to try again', 0);
            return null;
          } finally {
            if (captchaState.inFlightPromise === promise) {
              captchaState.inFlightPromise = null;
            }
          }
        })();
        captchaState.inFlightPromise = promise;
        await promise;
        return;
      }
    }
    await startCaptchaFlow(action, { userTriggered: trusted });
  }

  async function ensureCaptchaReady(action, userTriggered) {
    if (!captchaState.enabled) return null;
    if (captchaSolutionFresh(action)) return captchaState.solution;
    if (captchaState.status === 'interactive') {
      throw new Error('Please click the verification checkbox above to continue.');
    }
    if (captchaState.inFlightPromise) {
      await captchaState.inFlightPromise;
      if (!captchaState.enabled) return null;
      if (captchaSolutionFresh(action)) return captchaState.solution;
      if (captchaState.status === 'interactive') {
        throw new Error('Please click the verification checkbox above to continue.');
      }
    }
    await startCaptchaFlow(action, { userTriggered });
    if (!captchaState.enabled) return null;
    if (captchaSolutionFresh(action)) return captchaState.solution;
    if (captchaState.status === 'interactive') {
      throw new Error('Please click the verification checkbox above to continue.');
    }
    throw new Error('Human verification failed. Please click the verification box to try again.');
  }

  // ---------------------------------------------------------------------------
  // Authentication & Session Bootstrap
  // ---------------------------------------------------------------------------
  function setAuthMode(mode) {
    const signup = mode === 'signup';
    const nextAction = signup ? 'signup' : 'signin';
    const modeChanged = state.authMode !== nextAction;
    state.authMode = nextAction;
    const signinTab = $('auth-tab-signin');
    const signupTab = $('auth-tab-signup');
    if (signinTab) {
      signinTab.classList.toggle('active', !signup);
      signinTab.setAttribute('aria-selected', signup ? 'false' : 'true');
    }
    if (signupTab) {
      signupTab.classList.toggle('active', signup);
      signupTab.setAttribute('aria-selected', signup ? 'true' : 'false');
    }
    const confirmRow = $('auth-confirm-row');
    if (confirmRow) confirmRow.classList.toggle('hidden', !signup);
    const confirmInput = $('auth-confirm');
    if (confirmInput) {
      confirmInput.required = signup;
      if (!signup) confirmInput.value = '';
    }
    const passwordInput = $('auth-password');
    if (passwordInput) passwordInput.autocomplete = signup ? 'new-password' : 'current-password';
    const submit = $('auth-submit');
    if (submit) submit.textContent = signup ? 'Create account' : 'Sign in';
    const passkeyBtn = $('auth-passkey-btn');
    if (passkeyBtn) passkeyBtn.classList.toggle('hidden', signup);
    if (modeChanged || captchaState.action !== nextAction || captchaState.status === 'idle') {
      resetCaptchaAndStart(nextAction);
    }
  }

  function resetAuthForm() {
    const form = $('auth-form');
    if (form) {
      form.reset();
      form.classList.remove('hidden');
    }
    const passkeyBtn = $('auth-passkey-btn');
    if (passkeyBtn) passkeyBtn.classList.remove('hidden');
    setAuthMode('signin');
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
    for (const id of ['auth-username', 'auth-password', 'auth-confirm']) {
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
    const hp = $('auth-hp');
    if (hp) hp.value = '';
    resetCaptchaAndStart('signin');
  }

  function resetPlayerAndSessionUI() {
    clearPlaybackRecovery();
    state.intentionalPause = true;
    state.hiddenWhilePlayingAt = 0;
    state.hiddenResumeAttempted = false;
    state.switchGuardUntil = 0;
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
    state.radioError = false;
    state.queueRestoreFailed = false;
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
      hidden: null,
      sessions: null,
      security: null,
      curator: null,
      curatorSaving: false,
      totpSetup: null,
      recoveryCodes: null,
      confirmResetHidden: false,
      confirmRevokeAll: false,
      confirmDisable2fa: false,
      confirmDelete: false,
      showPasswordForm: false,
    };
    clearTimeout(state.searchTimer);
    state.searchTimer = null;
    state.searchComposing = false;
    state.ownSearchNavQuery = null;
    abortActiveViewRequest();
    state.fsMobileView = 'player';

    $('pb-cover').src = '/api/web/cover/default';
    setMarqueeText($('pb-title'), 'Ready to play');
    $('pb-artist').textContent = 'Select any track';
    $('pb-like').classList.remove('active');
    $('pb-quality').classList.add('hidden');
    $('pb-elapsed').textContent = '0:00';
    $('pb-duration').textContent = '0:00';
    $('pb-seek').value = '0';
    if ($('pb-mini-progress-fill')) $('pb-mini-progress-fill').style.width = '0%';

    $('fs-cover').src = '/api/web/cover/default';
    setMarqueeText($('fs-title'), 'Ready');
    $('fs-artist').textContent = 'Beatlab';
    setFsTint('');
    if ($('fs-mini-cover')) $('fs-mini-cover').src = '/api/web/cover/default';
    if ($('fs-mini-title')) $('fs-mini-title').textContent = 'Ready';
    if ($('fs-mini-artist')) $('fs-mini-artist').textContent = 'Beatlab';
    if ($('fs-like')) $('fs-like').classList.remove('active');
    if ($('fs-quality')) $('fs-quality').classList.add('hidden');
    if ($('fs-elapsed')) $('fs-elapsed').textContent = '0:00';
    if ($('fs-duration')) $('fs-duration').textContent = '0:00';
    if ($('fs-seek')) $('fs-seek').value = '0';
    paintSeekFill();
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
    resetMediaSessionState();
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
    if ($('right-panel') && isTabletOrMobileViewport()) {
      delete $('right-panel').dataset.tabletToggled;
      $('right-panel').classList.add('hidden');
    }
    if ($('panel-backdrop')) $('panel-backdrop').classList.add('hidden');
    document.body.classList.remove('rp-anim');
    syncRightPanelState();
    authScreen.classList.remove('hidden');
    $('app-shell').classList.add('hidden');
    $('player-bar').classList.add('hidden');
    if (wasHidden || forceReset) {
      resetAuthForm();
    }
  }

  function showAppShell() {
    cancelCaptchaWork();
    document.body.classList.remove('rp-anim');
    syncResponsiveLayout();
    syncRightPanelState();
    $('auth-screen').classList.add('hidden');
    $('app-shell').classList.remove('hidden');
    $('player-bar').classList.remove('hidden');
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
    // webauthn-rs wraps options in its own `publicKey` field; accept either
    // shape so an older server build still works during rolling deploys.
    let options = src || {};
    if (!options.challenge && options.publicKey && options.publicKey.challenge) {
      options = options.publicKey;
    }
    const out = Object.assign({}, options);
    out.challenge = base64urlToBytes(options.challenge);
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
        return !!target.closest('button, a, input, label, .pb-progress-row, .pb-vol-wrap');
      }
      return !!target.closest('button, a, input, label, .pb-title, .pb-artist, .pb-progress-row, .pb-vol-wrap');
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

  let syncRightPanelState = () => {};

  // Mirror the queue panel's visibility onto <body> so the edge pull-tab shows
  // whenever the panel is closed, whichever code path closed it.
  function watchRightPanel() {
    const rp = $('right-panel');
    if (!rp) return;
    let wasClosed = rp.classList.contains('hidden');
    const sync = () => {
      const closed = rp.classList.contains('hidden');
      wasClosed = closed;
      document.body.classList.toggle('rp-closed', closed);
    };
    const observer = new MutationObserver(() => {
      const closed = rp.classList.contains('hidden');
      if (closed === wasClosed) return;
      // Enable the open/close slide only on an actual visibility toggle while
      // the app is on screen, so the panel doesn't animate in on initial load.
      const shell = $('app-shell');
      if (shell && !shell.classList.contains('hidden')) document.body.classList.add('rp-anim');
      sync();
      if (!closed) scheduleRightPanelContentSync('smooth');
    });
    observer.observe(rp, { attributes: true, attributeFilter: ['class'] });
    syncRightPanelState = () => {
      observer.takeRecords();
      sync();
    };
    sync();
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
    watchRightPanel();
    syncResponsiveLayout();
    MQ_TABLET.addEventListener('change', syncResponsiveLayout);
    MQ_MOBILE.addEventListener('change', syncResponsiveLayout);
    window.addEventListener('storage', (e) => {
      if (e.key !== null && e.key !== RIGHT_PANEL_CLOSED_KEY) return;
      const nextClosed = loadDesktopRightPanelClosed();
      if (nextClosed === state.desktopRightPanelClosed) return;
      state.desktopRightPanelClosed = nextClosed;
      const rp = $('right-panel');
      const shell = $('app-shell');
      if (!rp || isTabletOrMobileViewport() || !shell || shell.classList.contains('hidden')) return;
      rp.classList.toggle('hidden', nextClosed);
      if ($('panel-backdrop')) $('panel-backdrop').classList.add('hidden');
    });
    window.addEventListener('resize', () => {
      clearTimeout(state.marqueeTimer);
      state.marqueeTimer = setTimeout(() => {
        const cur = currentTrack();
        if (!cur) return;
        setMarqueeText($('pb-title'), cur.title);
        const nowTitle = $('queue-now-playing').querySelector('.q-item.active .q-item-title');
        setMarqueeText(nowTitle, cur.title);
      }, 220);
    });
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

    const captchaBtn = $('auth-captcha-btn');
    if (captchaBtn) {
      captchaBtn.addEventListener('click', (e) => {
        const errBox = $('auth-error');
        if (errBox) errBox.classList.add('hidden');
        onCaptchaButtonClick(e);
      });
    }

    $('auth-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const errBox = $('auth-error');
      errBox.classList.add('hidden');
      const username = $('auth-username').value.trim();
      const password = $('auth-password').value;
      const signup = state.authMode === 'signup';
      const action = signup ? 'signup' : 'signin';
      const confirmPassword = signup ? $('auth-confirm').value : '';
      if (signup && password !== confirmPassword) {
        errBox.textContent = 'Passwords do not match';
        errBox.classList.remove('hidden');
        return;
      }
      const submitBtn = $('auth-submit');
      const prevSubmitText = submitBtn ? submitBtn.textContent : '';
      const setSubmitting = (busy, label) => {
        if (!submitBtn) return;
        submitBtn.disabled = busy;
        submitBtn.textContent = busy ? label : (signup ? 'Create account' : 'Sign in');
      };
      let captchaSolution = null;
      try {
        if (captchaState.enabled && !captchaSolutionFresh(action)) {
          setSubmitting(true, 'Verifying…');
        }
        captchaSolution = await ensureCaptchaReady(action, Boolean(e && e.isTrusted));
      } catch (err) {
        setSubmitting(false, prevSubmitText);
        errBox.textContent = err.message;
        errBox.classList.remove('hidden');
        if (captchaState.status === 'interactive' && $('auth-captcha-btn')) {
          $('auth-captcha-btn').focus();
        }
        return;
      }

      const hpToken = ($('auth-hp') && $('auth-hp').value) || '';
      setSubmitting(true, signup ? 'Creating account…' : 'Signing in…');
      try {
        const payload = signup
          ? { username, password, confirmPassword, captcha: captchaSolution, hpToken }
          : { username, password, captcha: captchaSolution, hpToken };
        const res = await api(signup ? '/api/web/auth/signup' : '/api/web/auth/signin', {
          method: 'POST',
          body: JSON.stringify(payload),
        });
        cancelCaptchaWork();
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
        // Server consumes each PoW token once; immediately prepare a fresh one for retry.
        resetCaptchaAndStart(action, 160);
      } finally {
        setSubmitting(false, prevSubmitText);
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
          // Clearing the box must never jump to a different tab.
          if (isSearchHash(location.hash.replace(/^#/, ''))) {
            if (state.searchState) {
              renderSearchView();
            } else {
              renderSearch('');
            }
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
      if ((e.target.value || '').trim()) hideRecentSearches();
      else showRecentSearches();
    });

    globalSearch.addEventListener('focus', () => showRecentSearches());

    globalSearch.addEventListener('blur', () => {
      hideRecentSearches();
      // Live search fires while typing; only remember the query the user
      // settled on (they left the box while its results were showing).
      const q = (globalSearch.value || '').trim();
      if (q && isSearchHash(location.hash.replace(/^#/, ''))) addRecentSearch(q);
    });

    globalSearch.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        hideRecentSearches();
        return;
      }
      if (e.key === 'Enter') {
        if (e.isComposing || e.keyCode === 229 || state.searchComposing) return;
        e.preventDefault();
        clearTimeout(state.searchTimer);
        state.searchTimer = null;
        addRecentSearch(e.target.value);
        hideRecentSearches();
        triggerSearchFromInput(e.target.value);
      }
    });

    const recentBox = $('recent-searches');
    // Keep focus in the input so blur doesn't close the dropdown mid-click.
    recentBox.addEventListener('mousedown', (e) => e.preventDefault());
    recentBox.addEventListener('click', (e) => {
      const list = getRecentSearches();
      const removeBtn = e.target.closest('[data-remove]');
      if (removeBtn) {
        e.stopPropagation();
        removeRecentSearch(list[Number(removeBtn.dataset.remove)]);
        showRecentSearches();
        return;
      }
      const item = e.target.closest('.recent-search-item');
      if (!item) return;
      const q = list[Number(item.dataset.idx)];
      if (!q) return;
      hideRecentSearches();
      runRefinedSearch(q);
    });

    wireAudioEvents();
    wireKeyboardShortcuts();
    wireTrackGestures();
    wireIntentWarm();
    wireStatsTooltips();
    wireOverflowTooltips();
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
      state.queueRestoreFailed = false;
      if (q.tracks && q.tracks.length > 0) {
        state.queue = cacheTracks(q.tracks);
        let idx = Math.max(0, Math.min(q.currentIndex || 0, state.queue.length - 1));
        if (q.currentId) {
          const byId = state.queue.findIndex((t) => t.id === q.currentId);
          if (byId >= 0) idx = byId;
        }
        state.queueIndex = idx;
        if (reconcileRestoredQueueShuffle()) saveServerQueue();
        const cur = currentTrack();
        const savedSec = q.positionMs > 1000 ? q.positionMs / 1000 : 0;
        state.pendingSeekSec = savedSec;
        updateNowPlayingUI(cur, false);
        syncPlayingTrackRows(cur ? cur.id : '');
        renderQueuePanel();
        if (cur) {
          audio.src = buildStreamUrl(cur);
          syncMediaSession(cur, true);
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
              paintSeekFill();
            }
          }
          loadLyricsForTrack(cur.id);
          // If the saved queue has no upcoming tracks (e.g. server crashed or tab closed
          // before radio finished populating), immediately auto-fill upcoming tracks.
          ensureUpcomingRadio();
        }
      }
    } catch {
      state.queueRestoreFailed = true;
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
    rememberQueueShuffleState();
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

  // The shuffle toggle is stored per browser while the queue is synced through the
  // server (and can be rewritten by other devices). Remember which exact order this
  // browser shuffled so a restored queue can be reconciled with it.
  const QUEUE_SHUFFLE_KEY = 'beatlab_queue_shuffle_state';

  function rememberQueueShuffleState() {
    try {
      if (state.shuffle && Array.isArray(state.unshuffledQueue) && state.queue.length) {
        localStorage.setItem(
          QUEUE_SHUFFLE_KEY,
          JSON.stringify({
            shuffled: state.queue.map((t) => t.id),
            original: state.unshuffledQueue.map((t) => t.id),
          }),
        );
      } else {
        localStorage.removeItem(QUEUE_SHUFFLE_KEY);
      }
    } catch {
      // storage unavailable; reconciliation falls back to reshuffling
    }
  }

  // Ensures a restored queue actually matches the shuffle button. Returns true when the
  // queue order changed and needs to be saved back.
  function reconcileRestoredQueueShuffle() {
    state.unshuffledQueue = null;
    if (!state.shuffle || state.queue.length < 2) return false;
    let saved = null;
    try {
      saved = JSON.parse(localStorage.getItem(QUEUE_SHUFFLE_KEY) || 'null');
    } catch {
      saved = null;
    }
    const ids = state.queue.map((t) => t.id);
    if (
      saved &&
      Array.isArray(saved.shuffled) &&
      Array.isArray(saved.original) &&
      saved.shuffled.length === ids.length &&
      saved.shuffled.every((id, i) => id === ids[i])
    ) {
      // Same shuffled queue this browser saved: restore the original order for un-shuffling.
      const byId = new Map(state.queue.map((t) => [t.id, t]));
      const original = saved.original.map((id) => byId.get(id)).filter(Boolean);
      if (original.length === state.queue.length) state.unshuffledQueue = original;
      return false;
    }
    // Queue came back in natural order (another device, or saved with shuffle off)
    // while shuffle is on here: shuffle what hasn't played yet.
    const splitIdx = state.queueIndex >= 0 ? state.queueIndex + 1 : 0;
    const upcoming = state.queue.slice(splitIdx);
    if (upcoming.length < 2) return false;
    state.unshuffledQueue = state.queue.slice();
    state.queue = state.queue.slice(0, splitIdx).concat(shuffleArray(upcoming));
    return true;
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
    document.body.dataset.route = hash.split(/[/?]/)[0];
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
    if (hash === 'profile' || hash.startsWith('profile/') || hash.startsWith('user/')) {
      const { reference, tab } = parseProfileHash(hash);
      return renderProfileView(reference, tab);
    }
    if (hash.startsWith('explore-mix/')) return renderExploreMix(safeDecodeUri(hash.slice(12)));
    if (hash === 'mix' || hash.startsWith('mix?')) {
      const qIdx = hash.indexOf('?');
      const params = qIdx >= 0 ? new URLSearchParams(hash.slice(qIdx + 1)) : null;
      const mixQ = params ? params.get('q') || '' : '';
      return renderInstantMixPage(mixQ);
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

  const QP_PER_PAGE = 9;
  const QP_MAX_PAGES = 4;

  function quickPicksSectionHtml(quickPicks, title = 'Quick Picks') {
    const pages = [];
    for (let i = 0; i < quickPicks.length; i += QP_PER_PAGE) pages.push(quickPicks.slice(i, i + QP_PER_PAGE));
    const multi = pages.length > 1;
    const chevron = (d) => `<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${d}"/></svg>`;
    const nav = multi ? `<div class="qp-nav">
        <button class="qp-arrow" id="qp-prev" title="Previous" aria-label="Previous quick picks" disabled>${chevron('M15 18l-6-6 6-6')}</button>
        <button class="qp-arrow" id="qp-next" title="Next" aria-label="Next quick picks">${chevron('M9 18l6-6-6-6')}</button>
      </div>` : '';
    const dots = multi ? `<div class="qp-dots" id="qp-dots">${pages.map((_, i) => `<button class="qp-dot${i === 0 ? ' active' : ''}" data-page="${i}" aria-label="Page ${i + 1}"></button>`).join('')}</div>` : '';
    return `<section class="section quick-picks-section">
      <div class="section-head"><h2>${esc(title)}</h2>${nav}</div>
      <div class="qp-pager" id="qp-pager">${pages.map((p) => `<div class="qp-page"><div class="quick-picks-grid">${p.map(renderQuickTile).join('')}</div></div>`).join('')}</div>
      ${dots}
    </section>`;
  }

  function bindQuickPicksPager() {
    const pager = $('qp-pager');
    if (!pager || pager.children.length < 2) return;
    const prev = $('qp-prev');
    const next = $('qp-next');
    const dots = document.querySelectorAll('#qp-dots .qp-dot');
    const count = pager.children.length;
    const current = () => Math.round(pager.scrollLeft / Math.max(1, pager.clientWidth));
    const goTo = (i) => {
      const idx = Math.max(0, Math.min(count - 1, i));
      pager.scrollTo({ left: idx * pager.clientWidth, behavior: 'smooth' });
    };
    const sync = () => {
      const i = current();
      if (prev) prev.disabled = i <= 0;
      if (next) next.disabled = i >= count - 1;
      dots.forEach((d, j) => d.classList.toggle('active', j === i));
    };
    if (prev) prev.onclick = () => goTo(current() - 1);
    if (next) next.onclick = () => goTo(current() + 1);
    dots.forEach((d) => { d.onclick = () => goTo(Number(d.dataset.page)); });
    let raf = 0;
    pager.addEventListener('scroll', () => {
      if (raf) return;
      raf = requestAnimationFrame(() => { raf = 0; sync(); });
    }, { passive: true });
    sync();
  }

  function newReleasesSectionHtml(newReleases, title = 'New Releases') {
    if (!newReleases.length) return '';
    return `<section class="section" id="new-releases-section">
      <div class="section-head">
        <h2>${esc(title)}</h2>
      </div>
      <div class="card-grid rail">${newReleases.slice(0, 8).map(renderNewReleaseCard).join('')}</div>
    </section>`;
  }

  async function renderHome() {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Loading...');
    try {
      const d = await api(`/api/web/home?tz=${encodeURIComponent(-new Date().getTimezoneOffset())}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const quickPicks = cacheTracks(d.quickPicks || []).slice(0, QP_PER_PAGE * QP_MAX_PAGES);
      const newReleases = d.newReleases || [];
      const trending = cacheTracks(d.trending || []);
      const recent = cacheTracks(d.recent || []);
      const relatedTracks = cacheTracks(d.relatedTracks || []);
      const mixes = d.madeForYou || [];
      const mixedForYou = d.mixedForYou || [];
      const homeCategories = d.homeCategories || [];
      const albums = d.albums || [];
      const artists = d.artists || [];
      const recommendedArtists = d.recommendedArtists || [];
      const moods = d.moods || [];
      const llm = d.llm || null;
      const sec = (llm && llm.sections) || {};
      const secTitle = (key, fallback) => (sec[key] && String(sec[key]).trim()) || fallback;
      if (llm && Array.isArray(llm.searchPrompts) && llm.searchPrompts.length) {
        state.llmSearchPrompts = llm.searchPrompts;
      }

      const greetingHeading = resolveHomeGreeting(llm);
      const hasHomeTagline = Boolean(llm && llm.homeTagline && String(llm.homeTagline).trim());
      let html = `<h1 class="page-greeting${hasHomeTagline ? ' has-edition' : ''}">${esc(greetingHeading)}</h1>`;
      if (hasHomeTagline) {
        html += `<div class="daily-edition-note">
          <span class="daily-edition-tagline">${esc(llm.homeTagline)}</span>
        </div>`;
      }

      if (quickPicks.length > 0) {
        html += quickPicksSectionHtml(quickPicks, secTitle('quickPicks', 'Quick Picks'));
      }

      // New releases from artists the user listens to sit right under Quick Picks.
      const newReleasesTitle = secTitle('newReleases', 'New Releases');
      html += `<div id="new-releases-slot">${newReleasesSectionHtml(newReleases, newReleasesTitle)}</div>`;

      if (mixedForYou.length > 0) {
        html += `<section class="section">
          <div class="section-head">
            <h2>${esc(secTitle('mixedForYou', 'Mixed for you'))}</h2>
          </div>
          <div class="mixed-for-you-grid rail">${mixedForYou.map(renderMixedForYouCard).join('')}</div>
        </section>`;
      }

      if (mixes.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('madeForYou', 'Made For You'))}</h2></div>
          <div class="card-grid rail">${mixes.map(renderPlaylistCard).join('')}</div>
        </section>`;
      }

      for (const cat of homeCategories) {
        const catMixes = Array.isArray(cat.mixes) ? cat.mixes : [];
        if (!catMixes.length) continue;
        html += `<section class="section llm-explore-section">
          <div class="section-head">
            <div class="llm-section-title-group">
              ${cat.kicker ? `<div class="llm-cat-kicker">${esc(cat.kicker)}</div>` : ''}
              <h2>${esc(cat.title)}</h2>
              ${cat.subtitle ? `<p class="llm-cat-sub">${esc(cat.subtitle)}</p>` : ''}
            </div>
            <button class="text-btn" onclick="App.navigate('explore')">See all</button>
          </div>
          <div class="explore-mix-grid">${catMixes.map(renderExploreMixCard).join('')}</div>
        </section>`;
      }

      if (recent.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('recent', 'Recently Played'))}</h2></div>
          <div class="card-grid rail">${recent.slice(0, 8).map(renderTrackCard).join('')}</div>
        </section>`;
      }

      if (relatedTracks.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('relatedTracks', 'Related Tracks For You'))}</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(relatedTracks.map((t) => t.id)))}, 0)">Play All</button></div>
          <div class="track-table">${renderTrackRows(relatedTracks.slice(0, 10), relatedTracks)}</div>
        </section>`;
      }

      if (recommendedArtists.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('recommendedArtists', 'Artists You Might Like'))}</h2></div>
          <div class="card-grid rail">${recommendedArtists.map(renderArtistCard).join('')}</div>
        </section>`;
      }

      if (moods.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('moods', 'Moods & Vibes'))}</h2><button class="text-btn" onclick="App.navigate('explore')">See all</button></div>
          <div class="mood-grid">${moods.slice(0, 5).map(renderMoodTile).join('')}</div>
        </section>`;
      }

      if (trending.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('trending', 'Trending Now'))}</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(trending.map((t) => t.id)))}, 0)">Play All</button></div>
          <div class="track-table">${renderTrackRows(trending.slice(0, 12))}</div>
        </section>`;
      }

      if (albums.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('albums', 'Popular Albums'))}</h2></div>
          <div class="card-grid rail">${albums.map(renderAlbumCard).join('')}</div>
        </section>`;
      }

      if (artists.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('artists', 'Top Artists'))}</h2></div>
          <div class="card-grid rail">${artists.map(renderArtistCard).join('')}</div>
        </section>`;
      }

      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
      bindQuickPicksPager();

      if (d.newReleasesPending && !newReleases.length) {
        (async () => {
          for (let attempt = 0; attempt < 3; attempt++) {
            try {
              const nr = await api('/api/web/home/new-releases', { signal });
              if (!isViewRequestCurrent(reqToken, signal)) return;
              const items = nr.newReleases || [];
              if (items.length > 0) {
                const slot = $('new-releases-slot');
                if (slot) slot.innerHTML = newReleasesSectionHtml(items, newReleasesTitle);
                return;
              }
              if (!nr.pending) return;
            } catch {
              return;
            }
          }
        })();
      }
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
    const kicker = item.kicker || (item.kind === 'llm' ? 'CURATED MIX' : 'ERA MIX');
    return `<div class="explore-mix-card" style="border-color:${esc(item.c1)}44" onclick="App.navigate('explore-mix/${encodeURIComponent(item.key)}')">
      <div class="explore-mix-art" style="background:${esc(item.c2)}">${artHtml}</div>
      <div class="explore-mix-body">
        <div class="explore-mix-kicker" style="color:${esc(item.c1)}">${esc(kicker)}</div>
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
        <button class="mixfor-play" title="Play ${esc(m.title)}" aria-label="Play ${esc(m.title)}" onclick="event.stopPropagation();App.playPlaylistNow(${esc(JSON.stringify(m.id))}, this)">${ICONS.play}</button>
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
      const customCategories = d.customCategories || [];
      const decades = d.decades || [];
      const artists = d.artists || [];
      const recommendedArtists = d.recommendedArtists || [];
      const curator = d.curator || null;
      const llm = d.llm || null;
      const sec = (llm && llm.sections) || {};
      const secTitle = (key, fallback) => (sec[key] && String(sec[key]).trim()) || fallback;
      if (llm && Array.isArray(llm.searchPrompts) && llm.searchPrompts.length) {
        state.llmSearchPrompts = llm.searchPrompts;
      }
      const exploreHeading = (llm && llm.exploreTitle && String(llm.exploreTitle).trim())
        ? String(llm.exploreTitle).trim()
        : 'Explore';
      const featuredGenres = (llm && Array.isArray(llm.featuredGenres)) ? llm.featuredGenres : [];
      const curatorBadge = curator
        ? `<span class="curator-pill" title="Server continuously curates moods, decades, and discovery pools in the background"><span class="curator-dot"></span>Live Curator · ${Number(curator.catalogTracks || 0).toLocaleString()} tracks · ${curator.readyMoods || 0}/${curator.totalMoods || 9} moods ready</span>`
        : '';

      let html = `<div class="section-head" style="margin-bottom:${llm && llm.exploreTagline ? '8px' : '18px'};gap:12px;flex-wrap:wrap">
        <div style="display:flex;align-items:center;gap:12px;flex-wrap:wrap">
          <h1 class="page-greeting" style="margin-bottom:0">${esc(exploreHeading)}</h1>
          ${curatorBadge}
        </div>
        <button class="btn-secondary" onclick="App.promptInstantMix()">Create Instant Mix</button>
      </div>`;

      if (llm && llm.exploreTagline) {
        html += `<p class="explore-daily-tagline">${esc(llm.exploreTagline)}</p>`;
      }

      for (const cat of customCategories) {
        const catMixes = Array.isArray(cat.mixes) ? cat.mixes : [];
        if (!catMixes.length) continue;
        html += `<section class="section llm-explore-section">
          <div class="section-head">
            <div class="llm-section-title-group">
              ${cat.kicker ? `<div class="llm-cat-kicker">${esc(cat.kicker)}</div>` : ''}
              <h2>${esc(cat.title)}</h2>
              ${cat.subtitle ? `<p class="llm-cat-sub">${esc(cat.subtitle)}</p>` : ''}
            </div>
          </div>
          <div class="explore-mix-grid">${catMixes.map(renderExploreMixCard).join('')}</div>
        </section>`;
      }

      html += `<section class="section">
        <div class="section-head"><h2>${esc(secTitle('moods', 'Moods & Vibes'))}</h2></div>
        <div class="mood-grid">${(d.moods || []).map(renderMoodTile).join('')}</div>
      </section>`;

      if (decades.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('decades', 'Decades & Time Machine'))}</h2></div>
          <div class="explore-mix-grid">${decades.map(renderExploreMixCard).join('')}</div>
        </section>`;
      }

      if (discoveryPicks.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('discovery', 'Fresh Discovery Picks'))}</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(discoveryPicks.map((t) => t.id)))}, 0)">Play All</button></div>
          <div class="track-table">${renderTrackRows(discoveryPicks)}</div>
        </section>`;
      }

      if (relatedTracks.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('relatedTracks', 'Related Tracks'))}</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(relatedTracks.map((t) => t.id)))}, 0)">Play All</button></div>
          <div class="track-table">${renderTrackRows(relatedTracks)}</div>
        </section>`;
      }

      if (recommendedArtists.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('recommendedArtists', 'Artists You Might Like'))}</h2></div>
          <div class="card-grid">${recommendedArtists.map(renderArtistCard).join('')}</div>
        </section>`;
      }

      if (artists.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('featuredArtists', 'Featured Artists'))}</h2></div>
          <div class="card-grid">${artists.map(renderArtistCard).join('')}</div>
        </section>`;
      }

      if (featuredGenres.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('genres', 'Scenes & Genres'))}</h2></div>
          <div class="llm-genre-grid">${featuredGenres.map((g) => `
            <div class="llm-genre-card" style="background:${esc(g.c2 || '#1e293b')};border-color:${esc(g.c1 || '#38bdf8')}55;border-left:4px solid ${esc(g.c1 || '#38bdf8')}" onclick="App.navigate('genre/${encodeURIComponent(g.name)}')">
              <div class="llm-genre-name">${esc(g.name)}</div>
              ${g.subtitle ? `<div class="llm-genre-sub">${esc(g.subtitle)}</div>` : ''}
            </div>`).join('')}</div>
        </section>`;
      } else if ((d.genres || []).length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('genres', 'Genres'))}</h2></div>
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
          <div class="section-head"><h2>${esc(secTitle('chartAlbums', 'Chart Albums'))}</h2></div>
          <div class="card-grid">${d.newAlbums.map(renderAlbumCard).join('')}</div>
        </section>`;
      }

      if (charts.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>${esc(secTitle('charts', 'Top 30 Charts'))}</h2><button class="mini-btn" onclick="App.playListByIds(${esc(JSON.stringify(charts.map((t) => t.id)))}, 0)">Play Charts</button></div>
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
      const mixKicker = m.kicker || (m.kind === 'llm' ? 'CURATED MIX' : 'ERA MIX');
      let html = `<div class="hero-header" style="background:${esc(m.c2)};border-color:${esc(m.c1)}55;border-left:4px solid ${esc(m.c1)};">
        ${artHtml}
        <div class="hero-info">
          <div class="hero-kicker" style="color:#fff">${esc(mixKicker)}</div>
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
      // An empty search stays on the Search tab with a neutral prompt instead
      // of bouncing the user over to Explore.
      state.searchState = null;
      state.searchFetchingQuery = '';
      state.searchLoadingUi = false;
      const prompts = Array.isArray(state.llmSearchPrompts) ? state.llmSearchPrompts : [];
      const promptsHtml = prompts.length
        ? `<div class="llm-search-prompts">
            <div class="llm-search-prompts-label">Curated vibes for today</div>
            <div class="llm-prompt-row">${prompts
              .map(
                (p) =>
                  `<button class="llm-prompt-chip" onclick="App.navigate('mix?q=${encodeURIComponent(p.query)}&mode=auto')">${esc(p.label)}</button>`
              )
              .join('')}</div>
          </div>`
        : '';
      $('main-view').innerHTML = `
        <div class="section-head" style="margin-bottom:18px">
          <h1 class="page-greeting" style="margin-bottom:0">Search</h1>
        </div>
        <div class="muted-center">
          <div style="font-size:15px;font-weight:700;color:var(--text);margin-bottom:6px">What do you want to hear?</div>
          <div>Search for artists, songs, albums, genres, moods, or people using the bar above.</div>
          <div style="margin-top:16px;display:flex;gap:8px;justify-content:center;flex-wrap:wrap">
            <button class="btn-secondary" onclick="App.promptInstantMix()">Create Instant Mix</button>
          </div>
          ${promptsHtml}
        </div>`;
      return;
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
    const intentTag = mix.aiInterpreted
      ? ' · AI VIBE MIX'
      : refinements?.detectedIntent
        ? ` · ${esc(refinements.detectedIntent)}`
        : '';

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
        ${renderTrackRows(mix.tracks.slice(0, previewCount), mix.tracks, null, false, query, false, true)}
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

  async function renderInstantMixPage(query) {
    const cleanQ = String(query || '').trim();
    const llmOn = Boolean(state.user?.llmEnabled);
    state.searchState = null;
    const placeholder = llmOn
      ? 'Describe a vibe, scene, artist, or era (e.g. rainy 2am Tokyo drive with warm synths, Daft Punk)...'
      : 'Enter an artist, song, genre, era, or mood (e.g. 80s hits, Daft Punk, chill)...';
    const builderHeader = (val) => `
      <div class="section-head" style="margin-bottom:14px">
        <h1 class="page-greeting" style="margin-bottom:0">Instant Mix</h1>
      </div>
      <form class="instant-mix-builder-form" onsubmit="App.submitInstantMixForm(event)">
        <input id="instant-mix-term-input" type="text" value="${esc(val)}" placeholder="${esc(placeholder)}" autocomplete="off" />
        <button type="submit" class="btn-primary">Create Instant Mix</button>
      </form>
    `;
    if (!cleanQ) {
      state.activeInstantMix = null;
      const dailyPrompts = Array.isArray(state.llmSearchPrompts) && state.llmSearchPrompts.length
        ? state.llmSearchPrompts.map((p) => ({
            label: p.label || p.query,
            query: p.query || p.label,
          }))
        : [];
      const fallbackPrompts = llmOn
        ? [
            { label: 'Rainy 2am Tokyo drive, warm synths', query: 'rainy 2am Tokyo drive with warm analog synths' },
            { label: 'Cozy Sunday morning acoustic coffee', query: 'cozy sunday morning acoustic songs for coffee' },
            { label: 'Upbeat indie rock for cleaning', query: 'upbeat indie rock like early strokes for cleaning' },
            { label: '80s hits', query: '80s hits' },
            { label: 'Late night chill', query: 'Late night chill' },
          ]
        : ['80s hits', '90s rock', 'Late night chill', 'Workout energy', 'Indie synthpop'].map((sg) => ({
            label: sg,
            query: sg,
          }));
      const chips = dailyPrompts.length ? dailyPrompts : fallbackPrompts;
      $('main-view').innerHTML = `
        ${builderHeader('')}
        <div class="instant-mix-suggestions">
          ${chips
            .map(
              (sg) =>
                `<button class="refine-query-pill" onclick="App.navigate('mix?q=${encodeURIComponent(sg.query)}')">${esc(sg.label)}</button>`
            )
            .join('')}
        </div>
      `;
      return;
    }

    const { reqToken, signal } = beginViewRequest();
    setMainLoading(`Creating instant mix for "${cleanQ}"...`);
    try {
      const d = await api(`/api/web/instant-mix?q=${encodeURIComponent(cleanQ)}`, { signal });
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
    const llmOn = Boolean(state.user?.llmEnabled);
    const hint = llmOn
      ? 'Describe any mood, scene, activity, artist, song, genre, or era.'
      : 'Generate an on-demand mix from any artist, song, genre, era, or mood.';
    const placeholder = llmOn
      ? 'e.g. rainy 2am Tokyo drive with warm synths, Daft Punk, 80s hits...'
      : 'e.g. 80s hits, Daft Punk, late night chill...';
    openModal(`
      <div class="modal-head"><h3>Create Instant Mix</h3><button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button></div>
      <form onsubmit="App.submitInstantMixModal(event)" style="display:flex;flex-direction:column;gap:12px">
        <p style="font-size:13px;color:var(--text-muted)">${esc(hint)}</p>
        <input id="instant-mix-modal-input" type="text" value="${esc(defaultVal)}" placeholder="${esc(placeholder)}" required autofocus />
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
            <div class="card-sub">${esc(topTrack.artist)} · ${esc(topTrack.album)}${yearTag(topTrack.year)}</div>
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
      let hintTrack = '';
      const cur = currentTrack();
      if (cur && cur.albumId === id) {
        hintName = cur.album || '';
        hintArtist = cur.artist || '';
        hintTrack = cur.title || '';
      } else {
        for (const t of state.trackMap.values()) {
          if (t.albumId === id) {
            hintName = t.album || '';
            hintArtist = t.artist || '';
            hintTrack = t.title || '';
            break;
          }
        }
      }
      const qs = new URLSearchParams();
      if (hintName) qs.set('name', hintName);
      if (hintArtist) qs.set('artist', hintArtist);
      if (hintTrack) qs.set('track', hintTrack);
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
            <button id="save-alb-btn" class="btn-secondary ${alb.isSaved ? 'active' : ''}" data-album-id="${esc(alb.id)}" data-saved="${alb.isSaved ? '1' : '0'}" onclick="App.toggleSaveAlbum(${esc(JSON.stringify(alb.id))})">${ICONS.heart} ${alb.isSaved ? 'Saved' : 'Save Album'}</button>
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
          html += renderTrackRows(tracks.filter((t) => (t.discNo || 1) === dn), tracks, null, false, '', true, false, false);
        }
      } else {
        html += renderTrackRows(tracks, null, null, false, '', true, false, false);
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
      const isSoloHint = (n) => {
        const s = String(n || '').trim();
        if (!s) return false;
        const low = s.toLowerCase();
        if (/\b(feat\.?|ft\.?|featuring|vs\.?)\b/.test(low)) return false;
        if (s.includes(' × ') || s.includes(' & ') || s.includes(' + ')) return false;
        const comma = s.indexOf(', ');
        if (comma > 0) {
          const right = s.slice(comma + 2).trim().toLowerCase();
          if (!right.startsWith('the ') && !/^(jr\.?|sr\.?|inc\.?|llc|ii|iii|iv)$/.test(right)) {
            return false;
          }
        }
        return true;
      };
      let hintName = '';
      const cur = currentTrack();
      if (cur && cur.artistId === id && cur.artist) {
        hintName = cur.artist;
      }
      if (!hintName || !isSoloHint(hintName)) {
        for (const t of state.trackMap.values()) {
          if (t.artistId === id && t.artist) {
            if (!hintName) hintName = t.artist;
            if (isSoloHint(t.artist)) {
              hintName = t.artist;
              break;
            }
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
            <button id="follow-art-btn" class="btn-secondary ${art.isFollowed ? 'active' : ''}" data-artist-id="${esc(art.id)}" data-followed="${art.isFollowed ? '1' : '0'}" onclick="App.toggleFollowArtist(${esc(JSON.stringify(art.id))})">${art.isFollowed ? '✓ Following' : '+ Follow'}</button>
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
          <button type="button" class="pl-visibility-opt ${isPublic ? '' : 'selected'}" data-vis="private" onclick="App.pickPlaylistVisibility('private')">${ICONS.lock}Private</button>
          <button type="button" class="pl-visibility-opt ${isPublic ? 'selected' : ''}" data-vis="public" onclick="App.pickPlaylistVisibility('public')">${ICONS.globe}Public</button>
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
          <h1 class="hero-title">${esc(pl.name)}<span class="pl-visibility-badge" title="${pl.public ? 'Anyone can view this playlist' : 'Only you can view this playlist'}">${pl.public ? `${ICONS.globe}Public` : `${ICONS.lock}Private`}</span></h1>
          <div class="hero-sub">By <strong>${esc(pl.ownerName)}</strong>${pl.comment ? ` · ${esc(pl.comment)}` : ''} · ${tracks.length} songs · ${fmtTotalDur(pl.duration)}</div>
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
          const cur = currentTrack();
          const isCur = cur && cur.id === t.id;
          const metaSub = t.genre ? `${t.artist} · ${t.genre}` : t.artist;
          return `
            <div class="track-row ${isCur ? 'playing' : ''}" data-tid="${esc(t.id)}">
              <div class="tr-idx" onclick="App.playListByIds(${listJson}, ${idx})">
                <span class="tr-num">${idx + 1}</span>
                <span class="tr-playing-icon">${ICONS.nowPlaying}</span>
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

  function isLibraryHash(hash) {
    return hash === 'library' || hash === 'liked' || hash === 'history';
  }

  async function renderLibrary(tab = 'playlists') {
    const { reqToken, signal } = beginViewRequest();
    state.libTab = tab;
    setMainLoading('Loading your library...');
    try {
      const lib = await api('/api/web/library', { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      state.playlists = lib.playlists || [];
      renderSidebarPlaylistsDom();
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
        html += state.playlists.length
          ? `<div class="card-grid">${state.playlists.map(renderPlaylistCard).join('')}</div>`
          : `<p class="muted-center">No playlists yet. Create one or save an Instant Mix!</p>`;
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

  /* ------------------------------------------------------------------
   * Overflow tooltips (desktop): text cut off by an ellipsis or a line
   * clamp reveals its full value in a floating box under the cursor,
   * the way native title tooltips do — but styled for this UI.
   * ---------------------------------------------------------------- */
  const OVF_SVG_NS = 'http://www.w3.org/2000/svg';
  const OVF_SHOW_DELAY = 220;
  const OVF_MAX_TEXT = 300;
  const OVF_MAX_DEPTH = 10;
  const OVF_MAX_CHILDREN = 6;
  const OVF_MAX_DESCENDANTS = 40;
  let ovfTip = null;
  let ovfTarget = null;
  let ovfText = '';
  let ovfTimer = 0;
  let ovfX = 0;
  let ovfY = 0;
  let ovfW = 0;
  let ovfH = 0;
  let ovfStrippedTitle = null;

  function ovfHideBox() {
    if (ovfTimer) {
      clearTimeout(ovfTimer);
      ovfTimer = 0;
    }
    // Hand the native title back so app logic that relies on it (marquee
    // titles) keeps working the way it did before this tooltip existed.
    if (ovfStrippedTitle && ovfTarget && ovfTarget.isConnected && !ovfTarget.hasAttribute('title')) {
      ovfTarget.setAttribute('title', ovfStrippedTitle);
    }
    ovfStrippedTitle = null;
    if (ovfTip) ovfTip.classList.add('hidden');
  }

  function ovfHide() {
    ovfHideBox();
    ovfTarget = null;
    ovfText = '';
  }

  function ovfPlace() {
    if (!ovfTip || ovfTip.classList.contains('hidden')) return;
    const pad = 10;
    const left = Math.max(pad, Math.min(window.innerWidth - ovfW - pad, ovfX - ovfW / 2));
    let top = ovfY + 22;
    if (top + ovfH > window.innerHeight - pad) top = ovfY - ovfH - 12;
    ovfTip.style.left = `${Math.round(left)}px`;
    ovfTip.style.top = `${Math.round(Math.max(pad, top))}px`;
  }

  function ovfSchedule(el) {
    if (ovfTimer) return;
    ovfTimer = setTimeout(() => {
      ovfTimer = 0;
      if (ovfTarget === el && el.isConnected) ovfShow(el);
    }, OVF_SHOW_DELAY);
  }

  function ovfShow(el) {
    if (!ovfTip || !ovfText) return;
    if (el.hasAttribute('title')) {
      // Suppress the browser's own box so only one tooltip is ever visible.
      ovfStrippedTitle = el.getAttribute('title');
      el.removeAttribute('title');
    }
    ovfTip.textContent = ovfText;
    ovfTip.classList.remove('hidden');
    ovfW = ovfTip.offsetWidth;
    ovfH = ovfTip.offsetHeight;
    ovfPlace();
  }

  // The full, uncut value of an element's text. Marquee titles duplicate
  // their text in the DOM, so read the first copy instead.
  function ovfTextOf(el) {
    let source = el;
    if (el.classList.contains('is-marquee')) {
      source = el.querySelector('.marquee-inner > span') || el;
    }
    let text = (source.textContent || '').replace(/\s+/g, ' ').trim();
    if (!text) text = (el.getAttribute('title') || '').replace(/\s+/g, ' ').trim();
    if (!text || text.length > OVF_MAX_TEXT) return null;
    return text;
  }

  // Is this element actually cutting its own content off? Scroll containers
  // and plain overflow are excluded: those can be scrolled or never clip.
  function ovfIsClipped(el) {
    if (!el.clientWidth || !el.clientHeight) return false;
    if (el.scrollWidth - el.clientWidth > 1) {
      const ox = getComputedStyle(el).overflowX;
      if (ox !== 'visible' && ox !== 'auto' && ox !== 'scroll') return true;
    }
    if (el.scrollHeight - el.clientHeight > 1) {
      const cs = getComputedStyle(el);
      const clamp = cs.getPropertyValue('-webkit-line-clamp');
      if (clamp && clamp !== 'none' && cs.overflowY !== 'auto' && cs.overflowY !== 'scroll') return true;
    }
    return false;
  }

  // Cards, rows and lists clip for layout reasons; only a text run should
  // become a tooltip target, so containers with clipped children are skipped.
  function ovfHasClippedDescendant(el) {
    const kids = el.querySelectorAll('*');
    const n = Math.min(kids.length, OVF_MAX_DESCENDANTS);
    for (let i = 0; i < n; i += 1) {
      if (ovfIsClipped(kids[i])) return true;
    }
    return false;
  }

  function ovfFindTarget(start) {
    let el = start;
    for (let depth = 0; el && el.nodeType === 1 && depth < OVF_MAX_DEPTH; depth += 1) {
      if (el === document.body || el === document.documentElement) break;
      if (el.id === 'text-tooltip') return null;
      if (el.hasAttribute('data-no-overflow-tip')) return null;
      if (typeof el.closest === 'function' && el.closest('.stats-tip-target')) return null;
      const tag = el.tagName;
      if (
        tag === 'INPUT' ||
        tag === 'TEXTAREA' ||
        tag === 'SELECT' ||
        el.namespaceURI === OVF_SVG_NS
      ) {
        el = el.parentElement;
        continue;
      }
      if (ovfIsClipped(el) && el.children.length <= OVF_MAX_CHILDREN && !ovfHasClippedDescendant(el)) {
        const text = ovfTextOf(el);
        return text ? { el, text } : null;
      }
      el = el.parentElement;
    }
    return null;
  }

  function wireOverflowTooltips() {
    // Pointer/fine devices only — touch devices reveal text by tapping.
    if (!window.matchMedia || !window.matchMedia('(hover: hover) and (pointer: fine)').matches) return;
    ovfTip = $('text-tooltip');
    if (!ovfTip) return;

    document.addEventListener(
      'pointerover',
      (e) => {
        const node = e.target;
        if (!node || node.nodeType !== 1) return;
        ovfX = e.clientX;
        ovfY = e.clientY;
        const hit = ovfFindTarget(node);
        const el = hit ? hit.el : null;
        if (el === ovfTarget) {
          if (!el) return;
          if (ovfTip.classList.contains('hidden')) ovfSchedule(el);
          else ovfPlace();
          return;
        }
        ovfHide();
        ovfTarget = el;
        ovfText = hit ? hit.text : '';
        if (el) ovfSchedule(el);
      },
      true
    );

    document.addEventListener(
      'pointermove',
      (e) => {
        ovfX = e.clientX;
        ovfY = e.clientY;
        if (ovfTimer) return;
        if (!ovfTarget) return;
        if (!ovfTarget.isConnected) {
          ovfHide();
          return;
        }
        if (ovfTip.classList.contains('hidden')) {
          // Re-arm after an explicit hide (e.g. a click) while still hovering.
          if (!ovfTarget.matches(':hover')) {
            ovfTarget = null;
            ovfText = '';
            return;
          }
          ovfSchedule(ovfTarget);
          return;
        }
        // The text under the cursor can change while it is showing (the
        // track advances, a list re-renders) — refresh instead of going stale.
        if (ovfTextOf(ovfTarget) !== ovfText) {
          const hit = ovfFindTarget(ovfTarget);
          ovfHide();
          if (hit) {
            ovfTarget = hit.el;
            ovfText = hit.text;
            ovfSchedule(hit.el);
          }
          return;
        }
        ovfPlace();
      },
      true
    );

    document.addEventListener(
      'pointerout',
      (e) => {
        if (!e.relatedTarget) ovfHide();
      },
      true
    );

    // Clicking opens menus/modals that would otherwise sit under the box.
    document.addEventListener('pointerdown', ovfHideBox, true);
    window.addEventListener('scroll', ovfHide, true);
    window.addEventListener('blur', ovfHide);
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
    let lastLabelCol = -3;
    const monthLabels = weeks
      .map((col, colIdx) => {
        const firstDay = col.find((c) => c.inRange && c.date.getUTCDate() <= 7) || col.find((c) => c.inRange);
        let label = '';
        if (firstDay) {
          const m = firstDay.date.getUTCMonth();
          const dayOfMonth = firstDay.date.getUTCDate();
          const hasRoom = colIdx >= lastLabelCol + 2;
          const showFirst = lastMonth === -1 && dayOfMonth <= 21;
          if (m !== lastMonth && hasRoom && (dayOfMonth <= 7 || showFirst)) {
            label = months[m];
            lastMonth = m;
            lastLabelCol = colIdx;
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
        <div class="stats-hm-inner" style="--hm-weeks:${weeks.length}">
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

  // Renders the full stats screen. With no `target` it is the personal stats
  // route in #main-view; with a `target` user it fills the Stats tab of that
  // user's profile (same layout, controls wired to the profile's handlers).
  function renderStatsView(opts = {}) {
    const target = opts.target || null;
    const d = target ? state.social.profileStatsData : state.statsData;
    if (!d) return;
    const host = target ? $('profile-tab-body') : $('main-view');
    if (!host) return;
    hideStatsTooltip();
    const isSelf = !target || d.isSelf;
    const ownerName = target ? target.displayName || target.username : state.user?.username || 'Your';
    const rangeFn = target ? 'App.setProfileStatsRange' : 'App.setStatsRange';
    const metricFn = target ? 'App.setProfileStatsMetric' : 'App.setStatsMetric';
    const refreshFn = target ? 'App.refreshProfileStats' : 'App.refreshStats';

    const s = d.summary || {};
    const streaks = d.streaks || {};
    const lib = d.library || {};
    const persona = d.persona || {};
    const portrait = d.tastePortrait || null;
    const hasPortrait = Boolean(
      portrait && (portrait.title || portrait.summary || (Array.isArray(portrait.traits) && portrait.traits.length))
    );
    const personaTitle = (hasPortrait && portrait.title) || persona.title || 'Music Listener';
    const personaBadges =
      hasPortrait && Array.isArray(portrait.traits) && portrait.traits.length
        ? portrait.traits
        : persona.badges || [];
    const personaHeadline = (hasPortrait && portrait.summary) || persona.headline || '';
    const metric = state.statsMetric === 'mins' ? 'mins' : 'streams';
    const ranges = [
      ['7d', '7 Days'],
      ['30d', '30 Days'],
      ['90d', '90 Days'],
      ['365d', '1 Year'],
      ['all', 'All Time'],
    ];
    let html = `<div class="stats-page${target ? ' stats-embedded' : ''}">`;

    // Hero Banner with Listener Persona + Controls
    html += `
      <section class="stats-hero">
        <div class="stats-hero-left">
          <div class="stats-hero-identity">
            <span class="avatar-dot large"${target?.avatarColor ? ` style="background:${esc(target.avatarColor)}"` : ''}>${avatarInner(target || selfAvatarUser())}</span>
            <div>
              <div class="hero-kicker">LISTENING ANALYTICS · ${esc(d.rangeLabel || 'All Time')}</div>
              <${target ? 'h2' : 'h1'} class="stats-hero-title">${esc(ownerName)}’s Listening Stats</${target ? 'h2' : 'h1'}>
              <div class="stats-hero-dates">
                Member since ${esc(fmtDateLabel(d.memberSince))}
                ${d.firstListenAt ? ` · First stream ${esc(fmtDateLabel(d.firstListenAt))}` : ''}
              </div>
            </div>
          </div>
          <div class="stats-persona-box">
            <div class="stats-persona-row">
              <span class="stats-persona-pill">${esc(personaTitle)}</span>
              ${hasPortrait ? `<span class="stats-portrait-badge" title="${esc(portrait.editionTitle || 'Daily Taste Portrait')}">Taste Portrait</span>` : ''}
              ${personaBadges.map((b) => `<span class="stats-vibe-chip">${esc(b)}</span>`).join('')}
            </div>
            <p class="stats-persona-headline">${esc(personaHeadline)}</p>
          </div>
        </div>
        <div class="stats-hero-controls">
          <div class="stats-range-pills" role="group" aria-label="Timeframe">
            ${ranges
              .map(
                ([k, lbl]) =>
                  `<button class="tab-chip ${d.range === k ? 'active' : ''}" onclick="${rangeFn}('${k}')">${esc(lbl)}</button>`
              )
              .join('')}
          </div>
          <div class="stats-ctrl-row">
            <div class="stats-seg-toggle" role="group" aria-label="Chart Metric">
              <button class="stats-seg-btn ${metric === 'streams' ? 'active' : ''}" onclick="${metricFn}('streams')">Streams</button>
              <button class="stats-seg-btn ${metric === 'mins' ? 'active' : ''}" onclick="${metricFn}('mins')">Minutes</button>
            </div>
            <button class="btn-secondary" onclick="${refreshFn}()" title="Refresh stats">Refresh</button>
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
          ${
            isSelf
              ? `<p>Play songs from Home, Explore, or Search to unlock your timeline graphs, 24-hour listening clock, mood radar, and top charts.</p>`
              : `<p>${esc(ownerName)} hasn’t played anything in this period yet.</p>`
          }
          <div class="hero-actions" style="justify-content:center;margin-top:14px">
            ${target ? '' : `<button class="btn-primary" onclick="App.navigate('explore')">Explore Music</button>
            <button class="btn-secondary" onclick="App.navigate('home')">Go Home</button>`}
            ${d.range !== 'all' ? `<button class="btn-secondary" onclick="${rangeFn}('all')">View All Time</button>` : ''}
          </div>
        </section>
      `;
      if ((d.recentEvents || []).length > 0) {
        html += renderStatsActivityCard(d.recentEvents);
      }
      html += `</div>`;
      host.innerHTML = html;
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
              <p class="stats-card-sub">${isSelf ? 'Your' : `${esc(ownerName)}’s`} emotional listening fingerprint across Beatlab’s 10 vibes</p>
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
                        <div class="stats-lb-title">${esc(a.name)} ${isSelf && a.isFollowed ? '<span class="stats-mini-tag">Following</span>' : ''}</div>
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
                        <div class="stats-lb-title">${esc(al.name)} ${isSelf && al.isSaved ? '<span class="stats-mini-tag">Saved</span>' : ''}</div>
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
              <p class="stats-card-sub">${isSelf ? 'Your' : `${esc(ownerName)}’s`} top ${topTracks.length} heavy rotation songs</p>
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
                      <span class="tr-num ${idx < 3 ? 'stats-medal' : ''}">${idx + 1}</span>
                      <span class="tr-playing-icon">${ICONS.nowPlaying}</span>
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

    // Other users' profiles don't expose the activity log, so milestones go full width.
    const showActivity = isSelf;
    html += `
      <section class="${showActivity ? 'stats-grid-2' : 'stats-section-gap'}">
        <div class="stats-card">
          <div class="stats-card-head">
            <div>
              <h2>Milestones &amp; Quirks</h2>
              <p class="stats-card-sub">Standout moments from ${isSelf ? 'your' : `${esc(ownerName)}’s`} listening history</p>
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

        ${showActivity ? renderStatsActivityCard(d.recentEvents || []) : ''}
      </section>
    `;

    html += `</div>`;
    host.innerHTML = html;

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
            <p class="stats-card-sub">Latest playback &amp; taste events</p>
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
  // Small "· 1999" marker for song surfaces. Empty when the year is unknown so
  // no stray separators ever show up.
  function yearTag(year) {
    const y = parseInt(year, 10);
    if (!Number.isFinite(y) || y < 1900 || y > 2100) return '';
    return `<span class="year-tag">&nbsp;&middot;&nbsp;${y}</span>`;
  }

  function renderQuickTile(t) {
    const cur = currentTrack();
    const playing = cur && cur.id === t.id ? ' playing' : '';
    return `<div class="qp-tile${playing}" data-tid="${esc(t.id)}" onclick="App.playSingle(${esc(JSON.stringify(t.id))})">
      <img src="${esc(t.coverUrl)}" loading="lazy" alt="" />
      <div class="qp-wave" aria-hidden="true"><span></span><span></span><span></span><span></span></div>
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
      <div class="card-sub">${esc(t.artist)}${yearTag(t.year)}</div>
    </div>`;
  }

  function renderAlbumCard(a) {
    const sub = `${a.artist}${a.year ? ` · ${a.year}` : ''}`;
    return `<div class="media-card" data-share-kind="album" data-share-id="${esc(a.id)}" data-share-title="${esc(a.name)}" data-share-sub="${esc(sub)}" data-share-cover="${esc(a.coverUrl)}" onclick="App.navigate('album/${encodeURIComponent(a.id)}')">
      <div class="card-art-wrap">
        <img src="${esc(a.coverUrl)}" loading="lazy" alt="" />
        <button class="card-play-fab" onclick="event.stopPropagation();App.playAlbumNow(${esc(JSON.stringify(a.id))}, this)">${ICONS.play}</button>
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
        <button class="card-play-fab" onclick="event.stopPropagation();App.playAlbumNow(${esc(JSON.stringify(a.id))}, this)">${ICONS.play}</button>
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
    const sub = p.isAuto && p.comment
      ? p.comment
      : `${p.trackCount} tracks · ${p.ownerName}`;
    const pinned = isPlaylistPinned(p.id);
    return `<div class="media-card" data-share-kind="playlist" data-share-id="${esc(p.id)}" data-share-title="${esc(p.name)}" data-share-sub="${esc(sub)}" data-share-cover="${esc(p.coverUrl)}" onclick="App.navigate('playlist/${encodeURIComponent(p.id)}')">
      <div class="card-art-wrap">
        <img src="${esc(p.coverUrl)}" loading="lazy" alt="" />
        <button class="card-pin-btn ${pinned ? 'pinned' : ''}" title="${pinned ? 'Unpin from sidebar' : 'Pin to sidebar'}" aria-label="${pinned ? 'Unpin' : 'Pin'} ${esc(p.name)}" onclick="event.stopPropagation();App.togglePinPlaylist(${esc(JSON.stringify(p.id))})">${ICONS.pin}</button>
        ${!p.isAuto ? `<button class="card-del-btn" title="Delete playlist" aria-label="Delete ${esc(p.name)}" onclick="event.stopPropagation();App.deletePlaylist(${esc(JSON.stringify(p.id))})">${ICONS.trash}</button>` : ''}
        <button class="card-play-fab" onclick="event.stopPropagation();App.playPlaylistNow(${esc(JSON.stringify(p.id))}, this)">${ICONS.play}</button>
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
    useTrackNo = false,
    useInstantMix = false,
    showYear = true
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
        if (useInstantMix) {
          playClick = `App.playInstantMixAt(${ctxIdx})`;
        } else if (searchQuery && singleRadio) {
          playClick = `App.playSearchTrack(${esc(JSON.stringify(t.id))})`;
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
          <div class="tr-idx" onclick="${playClick}">
            <span class="tr-num">${useTrackNo && t.trackNo > 0 ? t.trackNo : i + 1}</span>
            <span class="tr-playing-icon">${ICONS.nowPlaying}</span>
          </div>
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
                ${showYear ? yearTag(t.year) : ''}
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

  function buildStreamUrl(track, forceCompat = false, offsetSec = 0, retryAttempt = 0) {
    const trackId = typeof track === 'string' ? track : track.id;
    const needCompat = forceCompat || (typeof track === 'object' && track && !canBrowserPlayFormat(track.format));
    let url = `/api/web/stream/${encodeURIComponent(trackId)}?token=${encodeURIComponent(state.token)}`;
    if (needCompat) url += '&compat=1';
    if (offsetSec > 0.5) url += `&t=${encodeURIComponent(offsetSec.toFixed(2))}`;
    if (retryAttempt > 0) url += `&retry=${retryAttempt}`;
    return url;
  }

  function clearPlaybackRecovery() {
    if (state.networkRetryTimer) {
      clearTimeout(state.networkRetryTimer);
      state.networkRetryTimer = null;
    }
    if (state.stallTimer) {
      clearTimeout(state.stallTimer);
      state.stallTimer = null;
    }
    state.networkRetryCount = 0;
    state.pendingNetworkResume = null;
  }

  function scheduleNetworkRetry(errToken, track, resumePos = 0, fromStall = false) {
    if (!track || errToken !== state.activeAudioToken) return false;
    const cur = currentTrack();
    if (!cur || cur.id !== track.id) return false;
    const safePos = Number.isFinite(resumePos) && resumePos > 0.5 ? resumePos : 0;

    if (typeof navigator !== 'undefined' && navigator.onLine === false) {
      state.pendingNetworkResume = { trackId: track.id, pos: safePos, token: errToken };
      return true;
    }

    if (state.networkRetryCount >= 3) {
      state.pendingNetworkResume = { trackId: track.id, pos: safePos, token: errToken };
      return false;
    }

    if (state.networkRetryTimer) {
      clearTimeout(state.networkRetryTimer);
      state.networkRetryTimer = null;
    }

    const attempt = ++state.networkRetryCount;
    const delayMs = fromStall ? 400 : Math.min(8000, 1500 * Math.pow(2, attempt - 1));
    state.networkRetryTimer = setTimeout(() => {
      state.networkRetryTimer = null;
      if (errToken !== state.activeAudioToken) return;
      const nowCur = currentTrack();
      if (!nowCur || nowCur.id !== track.id) return;
      if (typeof navigator !== 'undefined' && navigator.onLine === false) {
        state.pendingNetworkResume = { trackId: track.id, pos: safePos, token: errToken };
        return;
      }
      state.pendingSeekSec = safePos;
      state.intentionalPause = false;
      state.isSwitchingTrack = true;
      state.switchGuardUntil = Date.now() + 800;
      try {
        audio.src = buildStreamUrl(nowCur, state.compatAttemptedId === nowCur.id, 0, attempt);
        audio.play().catch((e) => {
          if (errToken !== state.activeAudioToken) return;
          if (e && e.name === 'AbortError') return;
          if (e && e.name !== 'NotAllowedError') {
            scheduleNetworkRetry(errToken, nowCur, safePos, false);
          } else {
            updatePlayButton(false);
          }
        });
      } finally {
        state.isSwitchingTrack = false;
      }
    }, delayMs);
    return true;
  }

  function resumePendingNetworkPlayback() {
    const pending = state.pendingNetworkResume;
    if (!pending) return;
    if (pending.token !== state.activeAudioToken) {
      state.pendingNetworkResume = null;
      return;
    }
    const cur = currentTrack();
    if (!cur || cur.id !== pending.trackId) {
      state.pendingNetworkResume = null;
      return;
    }
    if (!audio.paused && !audio.ended && audio.readyState >= 3) {
      state.pendingNetworkResume = null;
      return;
    }
    state.pendingNetworkResume = null;
    state.networkRetryCount = 0;
    state.pendingSeekSec = pending.pos > 0.5 ? pending.pos : 0;
    state.intentionalPause = false;
    state.isSwitchingTrack = true;
    state.switchGuardUntil = Date.now() + 800;
    try {
      audio.src = buildStreamUrl(cur, state.compatAttemptedId === cur.id, 0, Date.now() % 10000);
      audio.play().catch((e) => {
        if (pending.token !== state.activeAudioToken) return;
        if (e && e.name === 'AbortError') return;
        updatePlayButton(false);
      });
    } finally {
      state.isSwitchingTrack = false;
    }
  }

  function cancelPendingPlaybackWork() {
    clearPlaybackRecovery();
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

    clearPlaybackRecovery();

    applyCommittedMediaUI(track);
    state.intentionalPause = false;
    state.hiddenResumeAttempted = false;
    state.isSwitchingTrack = true;
    state.switchGuardUntil = Date.now() + 800;
    try {
      audio.src = buildStreamUrl(track);
      audio.play().catch((e) => {
        if (loadToken !== state.activeAudioToken) return;
        if (e && e.name === 'AbortError') return;
        if (e && e.name !== 'NotAllowedError') {
          if (scheduleNetworkRetry(loadToken, track, 0, false)) return;
        }
        updatePlayButton(false);
        console.warn('Playback prevented or error:', e);
      });
    } finally {
      state.isSwitchingTrack = false;
    }

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

  function syncPlayingTrackRows(trackId) {
    document.querySelectorAll('.track-row, .qp-tile').forEach((row) => {
      row.classList.toggle('playing', !!trackId && row.dataset.tid === trackId);
    });
  }

  async function playQueueAt(index, { allowAutoRadio = true } = {}) {
    if (index < 0 || index >= state.queue.length) return;
    const prevTrackObj = currentTrack();
    const track = state.queue[index];
    if (prevTrackObj && prevTrackObj.id !== track.id) {
      recordRecentHeard(prevTrackObj);
    }
    recordRecentHeard(track);
    const now = typeof performance !== 'undefined' && performance.now ? performance.now() : Date.now();
    // Never defer audio commitment into a throttled setTimeout while backgrounded/locked.
    const isBurst =
      document.visibilityState === 'visible' &&
      state.lastPlaySwitchAt > 0 &&
      now - state.lastPlaySwitchAt < 280;
    state.lastPlaySwitchAt = now;

    cancelPendingPlaybackWork();
    const loadToken = ++state.activeAudioToken;
    state.queueIndex = index;
    state.pendingSeekSec = 0;
    state.lastScrobbleCompleteId = '';
    state.compatAttemptedId = '';

    const needsAutoRadio =
      allowAutoRadio &&
      !!state.autoplay &&
      !state.radioLoading &&
      !state.radioInFlight &&
      state.queue.length - 1 - index <= 2 &&
      (state.queue.length === 1 || state.repeat !== 'all');
    if (needsAutoRadio) {
      state.radioLoading = true;
    }

    setAudioLoading(true);
    updateNowPlayingUI(track, true, isBurst);
    renderQueuePanel();
    saveServerQueue();

    // Highlight playing track in current view if visible
    syncPlayingTrackRows(track.id);

    if (needsAutoRadio) {
      appendRadioTracks(track.id);
    }

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

  function setMediaMetadata(track, artwork) {
    navigator.mediaSession.metadata = new MediaMetadata({
      title: track.title || '',
      artist: track.artist || '',
      album: track.album || '',
      artwork,
    });
  }

  let mediaArtworkToken = 0;
  let mediaSessionState = {
    trackId: '',
    coverSrc: '',
    artwork: null,
    refiningKey: '',
    publishedWhilePlaying: '',
  };

  function resetMediaSessionState() {
    mediaArtworkToken++;
    mediaSessionState.trackId = '';
    mediaSessionState.coverSrc = '';
    mediaSessionState.artwork = null;
    mediaSessionState.refiningKey = '';
    mediaSessionState.publishedWhilePlaying = '';
  }

  function bindMediaSessionHandlers() {
    if (!('mediaSession' in navigator) || typeof navigator.mediaSession.setActionHandler !== 'function') return;
    const handlers = [
      ['play', () => playCurrent()],
      ['pause', () => pauseCurrent()],
      ['previoustrack', () => prevTrack()],
      ['nexttrack', () => nextTrack(true)],
      [
        'seekto',
        (details) => {
          if (details.seekTime != null && Number.isFinite(audio.duration)) {
            audio.currentTime = details.seekTime;
            updateMediaPositionState(true);
          }
        },
      ],
    ];
    for (const [action, handler] of handlers) {
      try {
        navigator.mediaSession.setActionHandler(action, handler);
      } catch {
        // unsupported action in this browser
      }
    }
  }

  // Lock-screen / notification players (Android especially) can't render SVG
  // placeholders and pick artwork by declared size/type. Load the cover once,
  // then publish either its real type + size or a rasterized PNG copy.
  async function refineMediaArtwork(track, coverSrc) {
    const key = `${track.id}\0${coverSrc}`;
    if (mediaSessionState.refiningKey === key) return;
    const token = ++mediaArtworkToken;
    mediaSessionState.refiningKey = key;
    const stillCurrent = () => token === mediaArtworkToken && currentTrack()?.id === track.id;
    try {
      const res = await fetch(coverSrc, { credentials: 'same-origin' });
      if (!res.ok) return;
      const type = (res.headers.get('content-type') || '').split(';')[0].trim().toLowerCase();
      const blob = await res.blob();
      const objectUrl = URL.createObjectURL(blob);
      let img;
      try {
        img = await new Promise((resolve, reject) => {
          const el = new Image();
          el.onload = () => resolve(el);
          el.onerror = reject;
          el.src = objectUrl;
        });
      } catch {
        URL.revokeObjectURL(objectUrl);
        return;
      }
      const w = img.naturalWidth || 512;
      const h = img.naturalHeight || 512;
      const rasterOk = ['image/jpeg', 'image/png', 'image/webp'].includes(type);
      let artwork;
      if (rasterOk && Math.max(w, h) >= 256) {
        artwork = [{ src: coverSrc, sizes: `${w}x${h}`, type }];
      } else {
        const size = 512;
        const canvas = document.createElement('canvas');
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d');
        if (!ctx) {
          URL.revokeObjectURL(objectUrl);
          return;
        }
        // Cover-fit the image into a square.
        const scale = Math.max(size / w, size / h);
        const dw = w * scale;
        const dh = h * scale;
        ctx.drawImage(img, (size - dw) / 2, (size - dh) / 2, dw, dh);
        artwork = [{ src: canvas.toDataURL('image/png'), sizes: `${size}x${size}`, type: 'image/png' }];
      }
      URL.revokeObjectURL(objectUrl);
      if (!stillCurrent() || !('mediaSession' in navigator)) return;
      mediaSessionState.trackId = track.id;
      mediaSessionState.coverSrc = coverSrc;
      mediaSessionState.artwork = artwork;
      setMediaMetadata(track, artwork);
    } catch {
      // keep the plain URL artwork already set
    } finally {
      if (token === mediaArtworkToken && mediaSessionState.refiningKey === key) {
        mediaSessionState.refiningKey = '';
      }
    }
  }

  function syncMediaSession(track, force = false) {
    if (!track || !('mediaSession' in navigator)) return;
    try {
      const coverSrc = new URL(track.coverUrl || '/api/web/cover/default', location.origin).href;
      const sameTrack = mediaSessionState.trackId === track.id && mediaSessionState.coverSrc === coverSrc;
      const hasMetadata = !!navigator.mediaSession.metadata;

      if (!sameTrack) {
        mediaArtworkToken++;
        mediaSessionState.trackId = track.id;
        mediaSessionState.coverSrc = coverSrc;
        mediaSessionState.artwork = null;
        mediaSessionState.refiningKey = '';
        mediaSessionState.publishedWhilePlaying = '';
      }

      if (force || !sameTrack || !hasMetadata) {
        if (sameTrack && mediaSessionState.artwork) {
          setMediaMetadata(track, mediaSessionState.artwork);
        } else {
          setMediaMetadata(track, [
            { src: coverSrc, sizes: '96x96' },
            { src: coverSrc, sizes: '256x256' },
            { src: coverSrc, sizes: '512x512' },
          ]);
        }
      }

      if (!mediaSessionState.artwork) {
        refineMediaArtwork(track, coverSrc);
      }

      bindMediaSessionHandlers();
      updateMediaPositionState(true);
    } catch {
      // unsupported in some browsers
    }
  }

  // Flat now-playing colour: one calm colour sampled from the cover art
  // (no blur). Saturated pixels are weighted up so colourful covers don't
  // average out to grey, then lightness is pinned to a readable range.
  // --fs-tint paints the expanded player; --np-tint / --np-tint-light (on the
  // root) paint the mobile mini player in the dark / light theme.
  let fsTintUrl = null;
  let fsTintToken = 0;
  function clearNowPlayingTint() {
    const fs = $('fullscreen-player');
    if (fs) fs.style.removeProperty('--fs-tint');
    const root = document.documentElement.style;
    root.removeProperty('--np-tint');
    root.removeProperty('--np-tint-light');
  }
  function setFsTint(url) {
    const fs = $('fullscreen-player');
    if (!fs || url === fsTintUrl) return;
    fsTintUrl = url;
    const token = ++fsTintToken;
    if (!url) {
      clearNowPlayingTint();
      return;
    }
    const img = new Image();
    img.decoding = 'async';
    img.onload = () => {
      if (token !== fsTintToken) return;
      try {
        const size = 16;
        const c = document.createElement('canvas');
        c.width = size;
        c.height = size;
        const ctx = c.getContext('2d', { willReadFrequently: true });
        ctx.drawImage(img, 0, 0, size, size);
        const d = ctx.getImageData(0, 0, size, size).data;
        let r = 0;
        let g = 0;
        let b = 0;
        let w = 0;
        for (let i = 0; i < d.length; i += 4) {
          const hi = Math.max(d[i], d[i + 1], d[i + 2]);
          const lo = Math.min(d[i], d[i + 1], d[i + 2]);
          const wt = 0.08 + ((hi - lo) / 255) ** 2 * 2;
          r += d[i] * wt;
          g += d[i + 1] * wt;
          b += d[i + 2] * wt;
          w += wt;
        }
        r /= w * 255;
        g /= w * 255;
        b /= w * 255;
        const hi = Math.max(r, g, b);
        const lo = Math.min(r, g, b);
        const l = (hi + lo) / 2;
        let h = 0;
        let sat = 0;
        if (hi !== lo) {
          const delta = hi - lo;
          sat = l > 0.5 ? delta / (2 - hi - lo) : delta / (hi + lo);
          if (hi === r) h = (g - b) / delta + (g < b ? 6 : 0);
          else if (hi === g) h = (b - r) / delta + 2;
          else h = (r - g) / delta + 4;
          h *= 60;
        }
        const hue = Math.round(h);
        const outS = Math.round(Math.min(sat, 0.5) * 100);
        const dark = `hsl(${hue} ${outS}% 16%)`;
        fs.style.setProperty('--fs-tint', dark);
        const root = document.documentElement.style;
        root.setProperty('--np-tint', `hsl(${hue} ${outS}% 22%)`);
        root.setProperty('--np-tint-light', `hsl(${hue} ${Math.min(outS, 45)}% 91%)`);
      } catch {
        clearNowPlayingTint();
      }
    };
    img.onerror = () => {
      if (token === fsTintToken) clearNowPlayingTint();
    };
    img.src = url;
  }

  function applyCommittedMediaUI(track) {
    if (!track) return;
    $('pb-cover').src = track.coverUrl;
    $('fs-cover').src = track.coverUrl;
    if ($('fs-mini-cover')) $('fs-mini-cover').src = track.coverUrl;
    setFsTint(track.coverUrl);

    syncMediaSession(track, true);
  }

  // Only long, clipped titles scroll. A duplicated copy makes the loop seamless,
  // and the duration scales with length so every title moves at the same slow pace.
  function setMarqueeText(el, text) {
    if (!el) return;
    const value = String(text || '');
    el.classList.remove('is-marquee');
    el.style.removeProperty('--marquee-duration');
    el.removeAttribute('title');
    el.textContent = value;
    if (value.length < 10 || el.scrollWidth <= el.clientWidth + 1 || !el.clientWidth) return;
    el.title = value;
    const safe = esc(value);
    el.innerHTML = `<span class="marquee-inner"><span>${safe}</span><span aria-hidden="true">${safe}</span></span>`;
    el.style.setProperty('--marquee-duration', `${Math.min(34, Math.max(12, value.length * 0.55)).toFixed(1)}s`);
    el.classList.add('is-marquee');
  }

  function updateNowPlayingUI(track, active, skipHeavy = false) {
    if (!track) return;
    if (!skipHeavy) {
      $('pb-cover').src = track.coverUrl;
      $('fs-cover').src = track.coverUrl;
      if ($('fs-mini-cover')) $('fs-mini-cover').src = track.coverUrl;
      setFsTint(track.coverUrl);
    }
    setMarqueeText($('pb-title'), track.title);
    $('pb-artist').innerHTML = `${esc(track.artist)}${yearTag(track.year)}`;
    $('pb-like').classList.toggle('active', !!track.isStarred);
    $('pb-quality').textContent = track.qualityBadge;
    $('pb-quality').classList.remove('hidden');
    $('pb-quality').classList.toggle('lossless', !!track.isLossless);
    $('pb-duration').textContent = fmtTime(track.duration);
    $('pb-elapsed').textContent = '0:00';
    $('pb-seek').value = '0';
    if ($('pb-mini-progress-fill')) $('pb-mini-progress-fill').style.width = '0%';

    setMarqueeText($('fs-title'), track.title);
    $('fs-artist').innerHTML = `<span class="clickable" onclick="App.goCurrentArtist()">${esc(track.artist)}</span>${track.album ? ` · <span class="clickable" onclick="App.goCurrentAlbum()">${esc(track.album)}</span>` : ''}${yearTag(track.year)}`;
    if ($('fs-mini-title')) $('fs-mini-title').textContent = track.title;
    if ($('fs-mini-artist')) $('fs-mini-artist').innerHTML = `${esc(track.artist)}${yearTag(track.year)}`;
    if ($('fs-like')) $('fs-like').classList.toggle('active', !!track.isStarred);
    if ($('fs-quality')) {
      $('fs-quality').textContent = track.qualityBadge;
      $('fs-quality').classList.remove('hidden');
      $('fs-quality').classList.toggle('lossless', !!track.isLossless);
    }
    if ($('fs-duration')) $('fs-duration').textContent = fmtTime(track.duration);
    if ($('fs-elapsed')) $('fs-elapsed').textContent = '0:00';
    if ($('fs-seek')) $('fs-seek').value = '0';
    paintSeekFill();

    document.title = `${track.title} · ${track.artist} — Beatlab`;
    updatePlayButton(active);
  }

  function paintSeekFill() {
    for (const id of ['pb-seek', 'fs-seek']) {
      const el = $(id);
      if (el) el.style.setProperty('--pct', `${parseFloat(el.value) || 0}%`);
    }
  }

  // Spinner while a tapped track is still connecting/buffering (e.g. slow mobile data).
  function setAudioLoading(on) {
    document.body.classList.toggle('audio-loading', !!on);
  }

  function updatePlayButton(playing) {
    if (!playing) setAudioLoading(false);
    const svgHtml = playing
      ? '<path d="M6.5 4.5h4v15h-4zm7 0h4v15h-4z"/>'
      : '<path d="M8 4.5v15l12-7.5z"/>';
    $('play-icon-svg').innerHTML = svgHtml;
    document.body.classList.toggle('audio-playing', !!playing);
    if ($('fs-play-icon-svg')) $('fs-play-icon-svg').innerHTML = svgHtml;
  }

  function applyTrackYearUpdate(trackId, patch) {
    const year = parseInt(patch?.year, 10);
    if (!trackId || !Number.isFinite(year) || year < 1900 || year > 2100) return;
    const genre = typeof patch?.genre === 'string' && patch.genre.trim() ? patch.genre : '';
    let changed = false;

    const patchObj = (t) => {
      if (!t || t.id !== trackId) return;
      if (!t.year || t.year <= 0) {
        t.year = year;
        changed = true;
      }
      if (!t.genre && genre) {
        t.genre = genre;
      }
    };

    patchObj(state.trackMap.get(trackId));
    let inQueue = false;
    for (const t of state.queue) {
      if (t && t.id === trackId) {
        patchObj(t);
        inQueue = true;
      }
    }
    if (Array.isArray(state.unshuffledQueue)) {
      for (const t of state.unshuffledQueue) {
        patchObj(t);
      }
    }
    if (!changed) return;

    const cur = currentTrack();
    if (cur && cur.id === trackId) {
      $('pb-artist').innerHTML = `${esc(cur.artist)}${yearTag(cur.year)}`;
      $('fs-artist').innerHTML = `<span class="clickable" onclick="App.goCurrentArtist()">${esc(cur.artist)}</span>${cur.album ? ` · <span class="clickable" onclick="App.goCurrentAlbum()">${esc(cur.album)}</span>` : ''}${yearTag(cur.year)}`;
      if ($('fs-mini-artist')) $('fs-mini-artist').innerHTML = `${esc(cur.artist)}${yearTag(cur.year)}`;
    }
    if (inQueue) {
      renderQueuePanel();
    }
  }

  function sendTrackEvent(trackId, kind, value = 1) {
    api('/api/web/event', {
      method: 'POST',
      body: JSON.stringify({ trackId, kind, value }),
    })
      .then((res) => {
        if (kind === 'play' && res && res.track && res.track.id === trackId) {
          applyTrackYearUpdate(trackId, res.track);
        }
      })
      .catch(() => {});
  }

  function playListByIds(ids, startIndex = 0) {
    const tracks = ids.map((id) => state.trackMap.get(id)).filter(Boolean);
    if (!tracks.length) return;
    const prev = currentTrack();
    if (prev) recordRecentHeard(prev);
    state.radioReqToken++;
    state.radioLoading = false;
    state.radioInFlight = null;
    state.radioError = false;
    state.queueRestoreFailed = false;
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
    state.radioError = false;
    state.queueRestoreFailed = false;
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

  let activeLoadingFab = null;

  function setFabLoading(btn) {
    if (activeLoadingFab && activeLoadingFab !== btn) {
      activeLoadingFab.classList.remove('fab-loading');
      activeLoadingFab.removeAttribute('aria-busy');
    }
    activeLoadingFab = btn || null;
    if (activeLoadingFab) {
      activeLoadingFab.classList.add('fab-loading');
      activeLoadingFab.setAttribute('aria-busy', 'true');
    }
  }

  function clearFabLoading(btn) {
    if (btn) {
      btn.classList.remove('fab-loading');
      btn.removeAttribute('aria-busy');
      if (activeLoadingFab === btn) activeLoadingFab = null;
    } else if (activeLoadingFab) {
      activeLoadingFab.classList.remove('fab-loading');
      activeLoadingFab.removeAttribute('aria-busy');
      activeLoadingFab = null;
    }
  }

  function findCardPlayBtn(kind, id) {
    for (const el of document.querySelectorAll(`[data-share-kind="${kind}"]`)) {
      if (el.dataset.shareId === id) {
        return el.querySelector('.card-play-fab, .mixfor-play');
      }
    }
    return null;
  }

  async function playAlbumNow(id, btn = null) {
    const targetBtn = btn || findCardPlayBtn('album', id);
    if (targetBtn && targetBtn.classList.contains('fab-loading')) return;
    setFabLoading(targetBtn);
    const { token, signal } = beginQueueFetch();
    try {
      const d = await api(`/api/web/album/${encodeURIComponent(id)}?fast=1`, { signal });
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
      } else {
        showToast('No playable tracks found for this album');
      }
    } catch (e) {
      if (e && e.name === 'AbortError') return;
      showToast(e.message);
    } finally {
      clearFabLoading(targetBtn);
    }
  }

  async function playPlaylistNow(id, btn = null) {
    const targetBtn = btn || findCardPlayBtn('playlist', id);
    if (targetBtn && targetBtn.classList.contains('fab-loading')) return;
    setFabLoading(targetBtn);
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
    } finally {
      clearFabLoading(targetBtn);
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
    addRecentSearch(q);
    $('global-search').value = q;
    $('clear-search-btn').classList.remove('hidden');
    navigate(`search?q=${encodeURIComponent(q)}`);
  }

  function needsUpcomingAutofill() {
    const cur = currentTrack();
    if (!cur || !state.autoplay) return false;
    if (state.queue.length - 1 > state.queueIndex) return false;
    return state.queue.length === 1 || state.repeat !== 'all';
  }

  function ensureUpcomingRadio() {
    const cur = currentTrack();
    if (!cur || state.radioInFlight || state.radioLoading) return null;
    if (!needsUpcomingAutofill()) return null;
    return appendRadioTracks(cur.id);
  }

  function appendRadioTracks(seedId, { retries = 2 } = {}) {
    if (state.radioInFlight) return state.radioInFlight;
    const reqToken = ++state.radioReqToken;
    state.radioLoading = true;
    state.radioError = false;
    renderQueuePanel();
    const run = (async () => {
      const fetchOnce = async (minimalExclude) => {
        const excludeIds = minimalExclude
          ? seedId || ''
          : state.queue
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
        return api(`/api/web/radio?${params.toString()}`);
      };

      const mergeTracks = (tracks) => {
        const recs = cacheTracks(tracks || []);
        const existing = new Set(state.queue.map((t) => t.id));
        let added = 0;
        for (const r of recs) {
          if (r && r.id && !existing.has(r.id)) {
            state.queue.push(r);
            if (Array.isArray(state.unshuffledQueue)) {
              state.unshuffledQueue.push(r);
            }
            existing.add(r.id);
            added++;
          }
        }
        return added;
      };

      let hadError = false;
      try {
        for (let attempt = 0; attempt <= retries; attempt++) {
          if (reqToken !== state.radioReqToken) return 0;
          try {
            let d = await fetchOnce(false);
            if (reqToken !== state.radioReqToken) return 0;
            let added = mergeTracks(d.tracks);
            if (added === 0 && state.queue.length > 1) {
              d = await fetchOnce(true);
              if (reqToken !== state.radioReqToken) return 0;
              added = mergeTracks(d.tracks);
            }
            if (added > 0) {
              state.radioError = false;
              saveServerQueue();
              scheduleWarmNextTrack();
              return added;
            }
            hadError = false;
          } catch {
            hadError = true;
          }
          if (attempt < retries) {
            await new Promise((r) => setTimeout(r, (attempt + 1) * 1500));
            if (reqToken !== state.radioReqToken) return 0;
            if (seedId && currentTrack()?.id && currentTrack().id !== seedId) return 0;
          }
        }
        if (reqToken === state.radioReqToken && state.queue.length - 1 <= state.queueIndex) {
          state.radioError = hadError || true;
        }
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
        playQueueAt(idx, { allowAutoRadio: false });
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
        playQueueAt(idx, { allowAutoRadio: false });
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
    const cur = currentTrack();
    const hasUpcoming = state.queue.length - 1 > state.queueIndex;
    if (cleanMode === state.upcomingMode && (hasUpcoming || state.radioLoading || !cur)) return;
    state.upcomingMode = cleanMode;
    try {
      localStorage.setItem('beatlab_upcoming_mode', cleanMode);
    } catch {}
    haptic('select');
    if (!cur && state.queue.length === 0) {
      renderQueuePanel();
      return;
    }
    if (state.upcomingModeAbortController) {
      try {
        state.upcomingModeAbortController.abort();
      } catch {}
    }
    const ctrl = typeof AbortController !== 'undefined' ? new AbortController() : null;
    state.upcomingModeAbortController = ctrl;
    const reqToken = ++state.radioReqToken;
    state.radioLoading = true;
    state.radioError = false;
    renderQueuePanel();
    const run = (async () => {
      try {
        const params = new URLSearchParams();
        if (state.radioPlaylistId) {
          params.set('playlist', state.radioPlaylistId);
        } else if (cur?.id) {
          params.set('seed', cur.id);
        }
        if (cur?.id) {
          params.set('exclude', cur.id);
        }
        params.set('mode', cleanMode);
        const recentParam = buildRecentRadioParam(cur?.id || '');
        if (recentParam) params.set('recent', recentParam);
        const d = await api(`/api/web/radio?${params.toString()}`, {
          signal: ctrl ? ctrl.signal : undefined,
        });
        if (reqToken !== state.radioReqToken) return;
        const recs = cacheTracks(d.tracks || []);
        const nowCur = currentTrack();
        const curId = nowCur?.id;
        const seenIds = new Set(curId ? [curId] : []);
        const filtered = [];
        for (const r of recs) {
          if (r && r.id && !seenIds.has(r.id)) {
            seenIds.add(r.id);
            filtered.push(r);
          }
        }
        if (!filtered.length) {
          if (state.queue.length - 1 <= state.queueIndex) {
            state.radioError = true;
          }
          showToast('No matching upcoming tracks found');
          return;
        }
        state.radioError = false;
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
          if (state.queue.length - 1 <= state.queueIndex) {
            state.radioError = true;
          }
          showToast(e.message);
        }
      } finally {
        if (state.upcomingModeAbortController === ctrl) {
          state.upcomingModeAbortController = null;
        }
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

  function playCurrent() {
    const cur = currentTrack();
    if (!cur) return;
    clearPlaybackRecovery();
    state.intentionalPause = false;
    state.hiddenResumeAttempted = false;
    syncMediaSession(cur, true);
    ensureUpcomingRadio();
    const needsReload =
      !audio.getAttribute('src') ||
      !!audio.error ||
      audio.networkState === HTMLMediaElement.NETWORK_NO_SOURCE;
    const playToken = state.activeAudioToken;
    const resumePos =
      Number.isFinite(audio.currentTime) && audio.currentTime > 0.5
        ? audio.currentTime
        : state.pendingSeekSec > 0.5
          ? state.pendingSeekSec
          : 0;
    if (needsReload) {
      state.pendingSeekSec = resumePos;
      state.isSwitchingTrack = true;
      state.switchGuardUntil = Date.now() + 800;
      try {
        audio.src = buildStreamUrl(cur, state.compatAttemptedId === cur.id, 0, Date.now() % 10000);
      } finally {
        state.isSwitchingTrack = false;
      }
    }
    if (audio.paused) {
      audio.play().catch((e) => {
        if (playToken !== state.activeAudioToken) return;
        if (e && e.name === 'AbortError') return;
        if (e && e.name !== 'NotAllowedError') {
          if (scheduleNetworkRetry(playToken, cur, resumePos, false)) return;
        }
        updatePlayButton(false);
      });
    }
  }

  function pauseCurrent() {
    state.intentionalPause = true;
    state.hiddenWhilePlayingAt = 0;
    clearPlaybackRecovery();
    if (!audio.paused) {
      audio.pause();
    }
  }

  function togglePlay() {
    const cur = currentTrack();
    if (!cur) return;
    haptic('medium');
    if (audio.paused) {
      playCurrent();
    } else {
      pauseCurrent();
    }
  }

  function prevTrack() {
    if (!state.queue.length) return;
    haptic('medium');
    if (audio.currentTime > 4 || (state.queueIndex === 0 && state.repeat !== 'all')) {
      state.intentionalPause = false;
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
    if (!state.queue.length) {
      updatePlayButton(false);
      setMediaPlaybackState('none');
      return;
    }

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
      const idxBefore = state.queueIndex;
      const inFlight = state.radioInFlight;
      if (inFlight) {
        await inFlight;
      } else {
        await appendRadioTracks(cur.id);
      }
      if (state.queueIndex === idxBefore && state.queueIndex + 1 < state.queue.length) {
        return playQueueAt(state.queueIndex + 1);
      }
      updatePlayButton(false);
      setMediaPlaybackState('none');
      return;
    }
    updatePlayButton(false);
    setMediaPlaybackState('none');
  }

  function wireAudioEvents() {
    const clearStallWatch = () => {
      if (state.stallTimer) {
        clearTimeout(state.stallTimer);
        state.stallTimer = null;
      }
    };
    const armStallWatch = () => {
      if (state.stallTimer || audio.paused || audio.ended || state.playCommitTimer || !audio.getAttribute('src')) {
        return;
      }
      const watchToken = state.activeAudioToken;
      const watchPos = audio.currentTime || 0;
      state.stallTimer = setTimeout(() => {
        state.stallTimer = null;
        if (watchToken !== state.activeAudioToken) return;
        if (audio.paused || audio.ended || state.playCommitTimer) return;
        const cur = currentTrack();
        if (!cur) return;
        const nowPos = audio.currentTime || 0;
        // Only intervene if playback genuinely hasn't progressed for 15s.
        if (Math.abs(nowPos - watchPos) < 0.25 && audio.readyState < 3) {
          scheduleNetworkRetry(watchToken, cur, nowPos, true);
        }
      }, 15000);
    };

    audio.addEventListener('play', () => {
      clearStallWatch();
      state.intentionalPause = false;
      updatePlayButton(true);
      syncMediaSession(currentTrack(), false);
      setMediaPlaybackState('playing');
      updateMediaPositionState(true);
    });
    audio.addEventListener('playing', () => {
      clearStallWatch();
      setAudioLoading(false);
      state.intentionalPause = false;
      state.hiddenResumeAttempted = false;
      state.networkRetryCount = 0;
      state.pendingNetworkResume = null;
      updatePlayButton(true);
      const cur = currentTrack();
      if (cur) {
        const needsPublish = mediaSessionState.publishedWhilePlaying !== cur.id;
        syncMediaSession(cur, needsPublish);
        mediaSessionState.publishedWhilePlaying = cur.id;
      }
      setMediaPlaybackState('playing');
      updateMediaPositionState(true);
    });
    audio.addEventListener('pause', () => {
      clearStallWatch();
      if (state.isSwitchingTrack || !audio.paused || audio.ended || state.playCommitTimer) return;
      if (Date.now() < state.switchGuardUntil && !state.intentionalPause) return;

      // Single-shot recovery if Android Chrome WebAPK fires an unsolicited pause
      // right during the visible -> hidden screen-lock transition. Never retries more
      // than once per hide cycle so legitimate interruptions (calls/headphone unplug)
      // are respected immediately.
      if (
        document.visibilityState === 'hidden' &&
        !state.intentionalPause &&
        !state.hiddenResumeAttempted &&
        state.hiddenWhilePlayingAt > 0 &&
        Date.now() - state.hiddenWhilePlayingAt < 1500 &&
        currentTrack() &&
        !audio.error
      ) {
        state.hiddenResumeAttempted = true;
        setMediaPlaybackState('playing');
        audio.play().catch(() => {
          mediaSessionState.publishedWhilePlaying = '';
          updatePlayButton(false);
          setMediaPlaybackState('paused');
          updateMediaPositionState(true);
          if (state.queue.length) saveServerQueue();
        });
        return;
      }

      mediaSessionState.publishedWhilePlaying = '';
      updatePlayButton(false);
      setMediaPlaybackState('paused');
      updateMediaPositionState(true);
      if (state.queue.length) saveServerQueue();
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
      updateMediaPositionState(true);
    });
    audio.addEventListener('seeked', () => {
      updateMediaPositionState(true);
    });
    audio.addEventListener('waiting', () => {
      if (!audio.paused) setAudioLoading(true);
      armStallWatch();
    });
    audio.addEventListener('stalled', armStallWatch);
    audio.addEventListener('error', () => {
      clearStallWatch();
      if (state.playCommitTimer) return;
      const cur = currentTrack();
      if (!cur || !audio.error || !audio.getAttribute('src')) return;
      const errToken = state.activeAudioToken;
      const code = audio.error.code;
      const pos = audio.currentTime || 0;
      const isOffline = typeof navigator !== 'undefined' && navigator.onLine === false;
      if (
        !isOffline &&
        (code === 3 || code === 4) &&
        state.compatAttemptedId !== cur.id &&
        !audio.src.includes('compat=1')
      ) {
        state.compatAttemptedId = cur.id;
        state.intentionalPause = false;
        state.isSwitchingTrack = true;
        state.switchGuardUntil = Date.now() + 800;
        try {
          audio.src = buildStreamUrl(cur, true, pos);
          audio.play().catch(() => {
            if (errToken !== state.activeAudioToken) return;
            updatePlayButton(false);
          });
        } finally {
          state.isSwitchingTrack = false;
        }
        return;
      }
      if (code === 2 || isOffline || code === 4) {
        if (scheduleNetworkRetry(errToken, cur, pos, false)) {
          checkServerHealth();
          return;
        }
      }
      updatePlayButton(false);
      checkServerHealth();
      showToast(`Unable to play "${cur.title}" right now`);
    });
    audio.addEventListener('ended', () => {
      clearStallWatch();
      if (state.playCommitTimer) return;
      const cur = currentTrack();
      if (state.repeat === 'one') {
        state.intentionalPause = false;
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
      clearStallWatch();
      if (!audio.paused && audio.readyState >= 3 && document.body.classList.contains('audio-loading')) {
        setAudioLoading(false);
      }
      const cur = currentTrack();
      const total = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : cur ? cur.duration : 0;
      const pos = audio.currentTime || 0;
      updateMediaPositionState(false);
      if (cur && total > 20 && pos / total >= 0.75 && state.lastScrobbleCompleteId !== cur.id) {
        state.lastScrobbleCompleteId = cur.id;
        sendTrackEvent(cur.id, 'complete', 1);
      }
      if (state.seeking || document.visibilityState === 'hidden') return;
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
        paintSeekFill();
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
        paintSeekFill();
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

    const applyVolumeValue = (val) => {
      const clamped = Math.max(0, Math.min(1, Number(val) || 0));
      state.volume = clamped;
      state.muted = clamped === 0;
      $('pb-volume').value = String(clamped);
      applyOutputVolume();
      localStorage.setItem('beatlab_vol', String(clamped));
      updateVolumeIcon();
    };

    $('pb-volume').addEventListener('input', (e) => {
      applyVolumeValue(parseFloat(e.target.value));
    });

    const volWrap = $('pb-volume').closest('.pb-vol-wrap');
    if (volWrap) {
      const setVolFromClientX = (clientX) => {
        const rect = $('pb-volume').getBoundingClientRect();
        if (rect.width <= 0) return;
        const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
        applyVolumeValue(Math.round(ratio * 100) / 100);
      };
      volWrap.addEventListener('pointerdown', (e) => {
        if (e.button !== 0 && e.pointerType === 'mouse') return;
        if (e.target === $('pb-volume')) return;
        e.preventDefault();
        setVolFromClientX(e.clientX);
        const onMove = (ev) => setVolFromClientX(ev.clientX);
        const onUp = () => {
          window.removeEventListener('pointermove', onMove);
          window.removeEventListener('pointerup', onUp);
          window.removeEventListener('pointercancel', onUp);
        };
        window.addEventListener('pointermove', onMove);
        window.addEventListener('pointerup', onUp);
        window.addEventListener('pointercancel', onUp);
      });
    }
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
        ? `<div class="queue-section-head">
            <div class="queue-section-label">Next Up (${upcoming.length})${state.radioLoading ? ' · Updating...' : ''}</div>
            <button type="button" class="queue-save-btn" onclick="App.promptSaveUpcoming()" title="Save upcoming as playlist" aria-label="Save upcoming as playlist">${ICONS.playlistAdd}<span>Save</span></button>
          </div>` +
          upcoming.map((t, i) => renderQItem(t, state.queueIndex + 1 + i, false)).join('')
        : state.radioLoading
          ? `<div class="queue-section-label">Next Up</div><p class="muted-center">Loading upcoming tracks...</p>`
          : cur && state.autoplay && (state.queue.length === 1 || state.repeat !== 'all')
            ? `<div class="queue-section-label">Next Up</div><div class="queue-empty-state"><p class="muted-center">${
                state.radioError ? 'Couldn\u2019t load upcoming tracks.' : 'No upcoming tracks loaded.'
              }</p><button type="button" class="btn-secondary queue-retry-btn" onclick="App.retryUpcoming()">Load Upcoming</button></div>`
            : '');

    $('queue-now-playing').innerHTML = nowHtml;
    $('queue-up-next').innerHTML = upNextHtml;
    if ($('fs-queue-now-playing')) $('fs-queue-now-playing').innerHTML = nowHtml;
    if ($('fs-queue-up-next')) $('fs-queue-up-next').innerHTML = upNextHtml;
    if (cur) {
      setMarqueeText($('queue-now-playing').querySelector('.q-item.active .q-item-title'), cur.title);
      const fsNow = $('fs-queue-now-playing');
      if (fsNow && fsNow.offsetParent) {
        setMarqueeText(fsNow.querySelector('.q-item.active .q-item-title'), cur.title);
      }
    }
  }

  function renderQItem(t, idx, active) {
    const queueAttr = !active ? ` data-queue-idx="${idx}"` : '';
    return `<div class="q-item ${active ? 'active' : ''}" data-tid="${esc(t.id)}"${queueAttr} onclick="App.playQueueAt(${idx})">
      <img src="${esc(t.coverUrl)}" alt="" />
      <div class="q-item-info">
        <div class="q-item-title">${esc(t.title)}</div>
        <div class="q-item-sub">${esc(t.artist)}${yearTag(t.year)}</div>
      </div>
    </div>`;
  }

  let saveQueueDraft = null;

  function defaultUpcomingPlaylistName(cur) {
    const mode = state.upcomingMode;
    const modeLabel = mode === 'discover' ? 'Discover' : mode === 'familiar' ? 'Familiar' : '';
    if (cur && cur.title) {
      return modeLabel ? `${cur.title} · ${modeLabel}` : `${cur.title} · Up Next`;
    }
    return modeLabel ? `Up Next · ${modeLabel}` : 'Up Next Queue';
  }

  function dedupeQueueTracks(list) {
    const seen = new Set();
    const out = [];
    for (const t of list) {
      if (!t || !t.id || seen.has(t.id)) continue;
      seen.add(t.id);
      out.push(t);
    }
    return out;
  }

  function saveQueueSelectedTracks(includeCurrentOverride = null) {
    if (!saveQueueDraft) return [];
    const inc = includeCurrentOverride === null ? saveQueueDraft.includeCurrent : includeCurrentOverride;
    const raw = inc && saveQueueDraft.cur
      ? [saveQueueDraft.cur, ...saveQueueDraft.upcoming]
      : [...saveQueueDraft.upcoming];
    return dedupeQueueTracks(raw);
  }

  function syncSaveQueueDraftFromDom() {
    if (!saveQueueDraft) return;
    const nameEl = $('save-queue-name');
    if (nameEl) saveQueueDraft.name = nameEl.value;
    const descEl = $('save-queue-desc');
    if (descEl) saveQueueDraft.comment = descEl.value;
    if (document.querySelector('.pl-visibility-opt.selected')) {
      saveQueueDraft.isPublic = selectedPlaylistVisibility();
    }
  }

  function renderSaveQueueModal(focusName = false) {
    if (!saveQueueDraft) return;
    const draft = saveQueueDraft;
    const tracks = saveQueueSelectedTracks();
    const withCurCount = saveQueueSelectedTracks(true).length;
    const upOnlyCount = saveQueueSelectedTracks(false).length;
    const totalSecs = tracks.reduce((sum, t) => sum + (Number(t.duration) || 0), 0);
    const durLabel = totalSecs > 0 ? ` · ${fmtTotalDur(totalSecs)}` : '';

    const coverUrls = [];
    for (const t of tracks) {
      if (t.coverUrl && !coverUrls.includes(t.coverUrl)) {
        coverUrls.push(t.coverUrl);
        if (coverUrls.length >= 4) break;
      }
    }
    if (!coverUrls.length) coverUrls.push('/api/web/cover/default');

    const artistNames = [];
    for (const t of tracks) {
      const a = (t.artist || '').trim();
      if (a && !artistNames.includes(a)) artistNames.push(a);
    }
    const artistPreview = artistNames.length > 3
      ? `${artistNames.slice(0, 3).join(', ')} & ${artistNames.length - 3} more`
      : artistNames.join(', ');

    const editablePlaylists = (state.playlists || []).filter((p) => !p.isAuto);
    if (draft.mode === 'existing' && !editablePlaylists.length) {
      draft.mode = 'new';
    }

    const scopeHtml =
      draft.cur && draft.upcoming.length && withCurCount !== upOnlyCount
        ? `<div class="save-queue-scope" role="group" aria-label="Tracks to include">
            <button type="button" class="save-queue-chip ${draft.includeCurrent ? 'active' : ''}" onclick="App.setSaveQueueIncludeCurrent(true)">Now Playing + Up Next (${withCurCount})</button>
            <button type="button" class="save-queue-chip ${!draft.includeCurrent ? 'active' : ''}" onclick="App.setSaveQueueIncludeCurrent(false)">Up Next only (${upOnlyCount})</button>
          </div>`
        : '';

    const modeTabsHtml = editablePlaylists.length
      ? `<div class="save-queue-tabs" role="tablist" aria-label="Save destination">
          <button type="button" role="tab" aria-selected="${draft.mode === 'new' ? 'true' : 'false'}" class="save-queue-tab ${draft.mode === 'new' ? 'active' : ''}" onclick="App.setSaveQueueMode('new')">New Playlist</button>
          <button type="button" role="tab" aria-selected="${draft.mode === 'existing' ? 'true' : 'false'}" class="save-queue-tab ${draft.mode === 'existing' ? 'active' : ''}" onclick="App.setSaveQueueMode('existing')">Add to Existing (${editablePlaylists.length})</button>
        </div>`
      : '';

    const bodyHtml =
      draft.mode === 'existing'
        ? `<div class="save-queue-pl-list">
            ${editablePlaylists
              .map(
                (p) => `
              <button type="button" class="save-queue-pl-row" onclick="App.submitSaveQueueToExisting(${esc(JSON.stringify(p.id))}, this)">
                <img src="${esc(p.coverUrl)}" alt="" />
                <span class="save-queue-pl-meta">
                  <span class="save-queue-pl-name">${esc(p.name)}</span>
                  <span class="save-queue-pl-count">${p.trackCount} track${p.trackCount === 1 ? '' : 's'}</span>
                </span>
                <span class="save-queue-pl-add">+ Add</span>
              </button>`
              )
              .join('')}
          </div>`
        : `<form onsubmit="App.submitSaveQueueNew(event)" style="display:flex;flex-direction:column;gap:12px">
            <input id="save-queue-name" type="text" value="${esc(draft.name)}" placeholder="Playlist name" required />
            <input id="save-queue-desc" type="text" value="${esc(draft.comment)}" placeholder="Description (optional)" />
            ${playlistVisibilityHtml(!!draft.isPublic)}
            <button id="save-queue-submit" type="submit" class="btn-primary">Create Playlist (${tracks.length} track${tracks.length === 1 ? '' : 's'})</button>
          </form>`;

    openModal(`
      <div class="modal-head"><h3>Save Queue to Playlist</h3><button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button></div>
      <div class="save-queue-preview">
        <div class="save-queue-covers">
          ${coverUrls.map((u) => `<img src="${esc(u)}" alt="" />`).join('')}
        </div>
        <div class="save-queue-summary">
          <div class="save-queue-count">${tracks.length} track${tracks.length === 1 ? '' : 's'}${durLabel}</div>
          ${artistPreview ? `<div class="save-queue-artists">${esc(artistPreview)}</div>` : ''}
        </div>
      </div>
      ${scopeHtml}
      ${modeTabsHtml}
      ${bodyHtml}
    `, 'save-queue-modal');

    if (focusName && draft.mode === 'new' && window.matchMedia && window.matchMedia('(hover: hover)').matches) {
      const inp = $('save-queue-name');
      if (inp) {
        inp.focus();
        inp.select();
      }
    }
  }

  function promptSaveUpcoming() {
    const cur = currentTrack();
    const upcoming = state.queue.slice(state.queueIndex + 1);
    if (!upcoming.length && !cur) {
      showToast('No upcoming tracks to save');
      return;
    }
    saveQueueDraft = {
      cur: cur ? { ...cur } : null,
      upcoming: upcoming.map((t) => ({ ...t })),
      includeCurrent: !!cur,
      mode: 'new',
      name: defaultUpcomingPlaylistName(cur),
      comment: '',
      isPublic: false,
    };
    renderSaveQueueModal(true);
  }

  function setSaveQueueIncludeCurrent(include) {
    if (!saveQueueDraft) return;
    syncSaveQueueDraftFromDom();
    saveQueueDraft.includeCurrent = !!include;
    renderSaveQueueModal(false);
  }

  function setSaveQueueMode(mode) {
    if (!saveQueueDraft) return;
    syncSaveQueueDraftFromDom();
    saveQueueDraft.mode = mode === 'existing' ? 'existing' : 'new';
    renderSaveQueueModal(false);
  }

  async function refreshCurrentPlaylistSurface(targetPid = null) {
    const curHash = location.hash.replace(/^#/, '');
    if (targetPid && (curHash === `playlist/${encodeURIComponent(targetPid)}` || curHash === `playlist/${targetPid}`)) {
      renderPlaylist(targetPid);
    } else if (curHash === 'playlists') {
      renderPlaylistsView();
    } else if (isLibraryHash(curHash)) {
      renderLibrary(state.libTab || 'playlists');
    }
  }

  async function submitSaveQueueNew(e) {
    e.preventDefault();
    if (!saveQueueDraft) {
      closeModal();
      return;
    }
    syncSaveQueueDraftFromDom();
    const name = (saveQueueDraft.name || '').trim();
    if (!name) {
      showToast('Please enter a playlist name');
      return;
    }
    const tracks = saveQueueSelectedTracks();
    const trackIds = tracks.map((t) => t.id);
    if (!trackIds.length) {
      showToast('No tracks to save');
      return;
    }
    const btn = $('save-queue-submit');
    if (btn) {
      btn.disabled = true;
      btn.textContent = 'Saving...';
    }
    try {
      const res = await api('/api/web/playlists', {
        method: 'POST',
        body: JSON.stringify({
          name,
          comment: (saveQueueDraft.comment || '').trim(),
          public: !!saveQueueDraft.isPublic,
          trackIds,
        }),
      });
      haptic('success');
      saveQueueDraft = null;
      closeModal();
      await refreshSidebarPlaylists();
      refreshCurrentPlaylistSurface(res.playlist?.id);
      showToast(`Saved "${name}" (${trackIds.length} track${trackIds.length === 1 ? '' : 's'})`);
    } catch (err) {
      if (btn) {
        btn.disabled = false;
        btn.textContent = `Create Playlist (${trackIds.length} track${trackIds.length === 1 ? '' : 's'})`;
      }
      showToast(err.message);
    }
  }

  async function submitSaveQueueToExisting(pid, rowBtn) {
    if (!saveQueueDraft) {
      closeModal();
      return;
    }
    const tracks = saveQueueSelectedTracks();
    const trackIds = tracks.map((t) => t.id);
    if (!trackIds.length) {
      showToast('No tracks to add');
      return;
    }
    const targetPl = (state.playlists || []).find((p) => p.id === pid);
    const plName = targetPl ? targetPl.name : 'playlist';
    if (rowBtn) rowBtn.disabled = true;
    try {
      const res = await api(`/api/web/playlists/${encodeURIComponent(pid)}/tracks`, {
        method: 'POST',
        body: JSON.stringify({ trackIds }),
      });
      haptic('success');
      saveQueueDraft = null;
      closeModal();
      await refreshSidebarPlaylists();
      refreshCurrentPlaylistSurface(pid);
      const added = Number(res.added) || 0;
      if (added === 0) {
        showToast(`Already in "${plName}"`);
      } else if (added < trackIds.length) {
        showToast(`Added ${added} new track${added === 1 ? '' : 's'} to "${plName}"`);
      } else {
        showToast(`Added ${added} track${added === 1 ? '' : 's'} to "${plName}"`);
      }
    } catch (err) {
      if (rowBtn) rowBtn.disabled = false;
      showToast(err.message);
    }
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
    if (document.visibilityState === 'hidden') return;
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

  function openTrackMenu(event, trackId, editablePlaylistId = null, playlistPos = -1, queueIdx = -1) {
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
    const canRemoveFromQueue =
      Number.isInteger(queueIdx) &&
      queueIdx > state.queueIndex &&
      queueIdx < state.queue.length &&
      state.queue[queueIdx]?.id === trackId;
    const queueActions = canRemoveFromQueue
      ? `<button class="ctx-item ctx-item-icon danger" onclick="App.closeTrackMenu();App.removeFromQueue(${queueIdx}, ${tidArg})">
          ${ICONS.close}
          <span>Remove from Queue</span>
        </button>
        <hr class="ctx-sep">`
      : '';
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
          <div class="ctx-sheet-sub">${esc(t.artist)}${yearTag(t.year)}</div>
        </div>
        <button class="icon-btn" title="Close" onclick="App.closeTrackMenu()">${ICONS.close}</button>
      </div>
      ${queueActions}
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

  // Pre-resolve a track's stream URL when the user shows intent to play it
  // (mouse resting on a row, or a finger going down on mobile), so the click
  // doesn't pay for the YouTube lookup. Server side this never downloads audio
  // or contacts Soulseek peers, and it's rate-limited here and there.
  function wireIntentWarm() {
    const recent = new Map(); // tid -> last warm time (ms)
    const TTL_MS = 10 * 60 * 1000;
    const MIN_GAP_MS = 300;
    let lastSent = 0;
    let hoverTimer = null;
    let hoverTid = null;

    const warm = (tid) => {
      if (!tid || !state.token) return;
      if (currentTrack()?.id === tid) return;
      const now = Date.now();
      const prev = recent.get(tid);
      if (prev && now - prev < TTL_MS) return;
      if (now - lastSent < MIN_GAP_MS) return;
      lastSent = now;
      recent.set(tid, now);
      if (recent.size > 500) {
        for (const [k, t] of recent) {
          if (now - t > TTL_MS) recent.delete(k);
        }
      }
      api(`/api/web/warm/${encodeURIComponent(tid)}?light=1`, { method: 'POST' }).catch(() => {});
    };

    const tidFrom = (target) => {
      const el = target && target.closest ? target.closest('[data-tid]') : null;
      return el ? el.dataset.tid : null;
    };

    document.addEventListener('pointerover', (e) => {
      if (e.pointerType !== 'mouse') return;
      const tid = tidFrom(e.target);
      if (tid === hoverTid) return;
      hoverTid = tid;
      if (hoverTimer) {
        clearTimeout(hoverTimer);
        hoverTimer = null;
      }
      if (!tid) return;
      hoverTimer = setTimeout(() => {
        hoverTimer = null;
        if (hoverTid === tid) warm(tid);
      }, 250);
    }, { passive: true });

    document.addEventListener('pointerdown', (e) => {
      if (e.pointerType === 'mouse') return;
      warm(tidFrom(e.target));
    }, { passive: true });
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
      if (!tid) return null;
      const queueIdx = row.dataset.queueIdx != null ? parseInt(row.dataset.queueIdx, 10) : -1;
      const validQueueIdx = Number.isFinite(queueIdx) ? queueIdx : -1;
      if (!state.trackMap.has(tid)) {
        const qTrack = validQueueIdx >= 0 ? state.queue[validQueueIdx] : state.queue.find((t) => t.id === tid);
        if (qTrack) state.trackMap.set(tid, qTrack);
      }
      if (!state.trackMap.has(tid)) return null;
      const plId = row.dataset.plId || null;
      const plPos = row.dataset.plPos != null ? parseInt(row.dataset.plPos, 10) : -1;
      return {
        type: 'track',
        tid,
        plId,
        plPos: Number.isFinite(plPos) ? plPos : -1,
        queueIdx: validQueueIdx,
      };
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
        openTrackMenu(evt, info.tid, info.plId, info.plPos, info.queueIdx);
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
          state.intentionalPause = true;
          state.hiddenWhilePlayingAt = 0;
          audio.pause();
          audio.removeAttribute('src');
          syncPlayingTrackRows('');
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
  let pendingPromptResolve = null;

  function settlePendingConfirm(result) {
    if (!pendingConfirmResolve) return;
    const resolve = pendingConfirmResolve;
    pendingConfirmResolve = null;
    resolve(result);
  }

  function settlePendingPrompt(value) {
    if (!pendingPromptResolve) return;
    const resolve = pendingPromptResolve;
    pendingPromptResolve = null;
    resolve(value);
  }

  function openModal(html, extraClass = '') {
    settlePendingConfirm(false);
    settlePendingPrompt(null);
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
      settlePendingPrompt(null);
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

  /// In-app replacement for the browser's native prompt() — resolves with the
  /// entered string, or null when cancelled.
  function promptModal({
    title = 'Enter a name',
    message = '',
    label = 'Name',
    placeholder = '',
    defaultValue = '',
    confirmLabel = 'Save',
  }) {
    return new Promise((resolve) => {
      openModal(`
        <div class="modal-head">
          <h3>${esc(title)}</h3>
          <button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button>
        </div>
        ${message ? `<p class="confirm-modal-message">${esc(message)}</p>` : ''}
        <form id="prompt-modal-form" style="display:flex;flex-direction:column;gap:10px">
          <label class="kv-label" for="prompt-modal-input">${esc(label)}</label>
          <input id="prompt-modal-input" type="text" maxlength="64" autocomplete="off"
                 placeholder="${esc(placeholder)}" value="${esc(defaultValue)}" />
          <div class="confirm-modal-actions">
            <button type="button" class="btn-secondary" id="prompt-modal-cancel">Cancel</button>
            <button type="submit" class="btn-primary">${esc(confirmLabel)}</button>
          </div>
        </form>
      `);
      pendingPromptResolve = resolve;
      const form = $('prompt-modal-form');
      const input = $('prompt-modal-input');
      $('prompt-modal-cancel')?.addEventListener('click', () => {
        settlePendingPrompt(null);
        closeModal();
      });
      form?.addEventListener('submit', (e) => {
        e.preventDefault();
        const value = input.value.trim();
        if (!value) {
          input.focus();
          return;
        }
        settlePendingPrompt(value);
        closeModal();
      });
      input?.focus();
      input?.select();
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
    const ua = s.userAgent || '';
    const mobile = /iPhone|iPad|Android|Mobile/i.test(ua);
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
              <div class="settings-row-label">Haptic Feedback</div>
              <div class="settings-row-desc">Subtle tactile vibrations on mobile taps, swipes, and controls (when supported by device)</div>
            </div>
            <label class="switch-toggle">
              <input type="checkbox" ${state.hapticsEnabled ? 'checked' : ''} onchange="App.toggleHaptics(this.checked)" />
              <span class="switch-track"></span>
            </label>
          </div>
        </div>
        ${renderCuratorSettingsSection()}
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
          <div class="settings-row-desc" style="margin-bottom:12px">Every device currently signed in to your account. Revoke anything you don't recognize.</div>
          ${
            sessions.length > 0
              ? `<div class="session-list">
                  ${sessions
                    .map((s) => {
                      const info = sessionInfo(s);
                      const isCurrent = s.current || s.id === state.token;
                      const ua = s.userAgent || '';
                      return `
                    <div class="session-card${isCurrent ? ' current' : ''}">
                      <span class="session-icon">${info.icon}</span>
                      <div class="session-meta">
                        <div class="session-title">
                          ${esc(info.title)}
                          ${isCurrent ? '<span class="session-current-pill">This device</span>' : ''}
                        </div>
                        <div class="session-sub">${esc(info.kindLabel)} &middot; Signed in ${esc(formatSessionDate(s.created))}</div>
                        <div class="session-sub">Last active ${esc(timeAgo(s.lastSeen || s.created))}</div>
                        ${s.ip ? `<div class="session-sub">IP ${esc(s.ip)}</div>` : ''}
                        ${ua ? `<div class="session-ua" title="${esc(ua)}">${esc(ua)}</div>` : ''}
                      </div>
                      <button class="mini-btn danger-btn session-revoke" title="Revoke this session" onclick="App.revokeSession(${esc(JSON.stringify(s.id))})">Revoke</button>
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
          ${
            sec.twoFactorEnabled
              ? confirmDisable
                ? `<div class="settings-confirm-bar">
                    <span>Disable all two-factor authentication? This removes your authenticator, passkeys and recovery codes.</span>
                    <input id="disable-code" type="text" inputmode="numeric" autocomplete="one-time-code" placeholder="2FA or recovery code (if required)" style="width:100%" />
                    <div class="settings-confirm-actions">
                      <button class="btn-secondary danger-btn" onclick="App.confirmDisable2fa()">Yes, disable 2FA</button>
                      <button class="mini-btn ghost" onclick="App.cancelDisable2fa()">Cancel</button>
                    </div>
                  </div>`
                : `<div class="settings-actions-row"><button class="btn-secondary danger-btn" onclick="App.promptDisable2fa()">Disable two-factor authentication</button></div>`
              : ''
          }
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Password</div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">Account password</div>
              <div class="settings-row-desc">Used to sign in to the web player. Music clients use your API key instead.${sec.twoFactorEnabled ? ' Your 2FA code is required to change it.' : ''}</div>
            </div>
          </div>
          ${
            state.settingsData.showPasswordForm
              ? `<form onsubmit="App.submitPasswordChange(event)" style="display:flex;flex-direction:column;gap:8px;margin-top:10px">
                  <input id="pw-cur" type="password" autocomplete="current-password" placeholder="Current password" required />
                  <input id="pw-new" type="password" autocomplete="new-password" placeholder="New password" required />
                  ${sec.twoFactorEnabled ? '<input id="pw-code" type="text" inputmode="numeric" autocomplete="one-time-code" placeholder="2FA or recovery code" required />' : ''}
                  <div class="settings-confirm-actions" style="justify-content:flex-start;margin-top:6px">
                    <button type="submit" class="btn-secondary">Update password</button>
                    <button type="button" class="mini-btn ghost" onclick="App.cancelPasswordChange()">Cancel</button>
                  </div>
                </form>`
              : `<div class="settings-actions-row"><button class="btn-secondary" onclick="App.showPasswordForm()">Change password</button></div>`
          }
        </div>

        ${
          sec.twoFactorEnabled
            ? `<div class="settings-section">
                <div class="settings-section-title">Recovery Codes</div>
                <div class="settings-row">
                  <div class="settings-row-info">
                    <div class="settings-row-label">${Math.max(0, sec.recoveryTotal - sec.recoveryUsed)} of ${sec.recoveryTotal} codes left</div>
                    <div class="settings-row-desc">Single-use backup codes for your second factor. Use one if you lose access to your authenticator or passkey. Store them somewhere safe.</div>
                  </div>
                </div>
                ${
                  codes
                    ? `<div class="recovery-codes-card">
                        <div class="kv-label">SAVE THESE NOW — THEY WILL NOT BE SHOWN AGAIN</div>
                        <div class="recovery-codes-grid">${codes.map((c) => `<code>${esc(c)}</code>`).join('')}</div>
                        <button class="btn-secondary" style="margin-top:10px" onclick="App.dismissRecoveryCodes()">I've saved them</button>
                      </div>`
                    : `<div class="settings-actions-row"><button class="btn-secondary" onclick="App.promptGenerateRecoveryCodes()">Generate new recovery codes</button></div>`
                }
              </div>`
            : ''
        }

        <div class="settings-section">
          <div class="settings-section-title">Passkeys</div>
          <div class="settings-row-desc" style="margin-bottom:12px">Passkeys let you sign in with Face ID, Windows Hello, a hardware key or a synced password key. ${!sec.passkeysAvailable ? 'Passkeys are not configured on this server (set BEATLAB_ORIGIN to your public HTTPS URL).' : ''}</div>
          <div class="hidden-track-list">${passkeyRows}</div>
          <div class="settings-actions-row">
            <button class="btn-secondary" ${!sec.passkeysAvailable || !passkeySupported() ? 'disabled' : ''} onclick="App.addPasskey()">Add a passkey</button>
          </div>
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Delete Account</div>
          <div class="settings-row">
            <div class="settings-row-info">
              <div class="settings-row-label">Delete my account</div>
              <div class="settings-row-desc">Signs you out of every device immediately. You have 7 days to restore it by signing back in — after that it is permanently removed.</div>
            </div>
          </div>
          ${
            confirmDelete
              ? `<div class="settings-confirm-bar">
                  <div style="display:flex;flex-direction:column;gap:8px;width:100%">
                    <span>Type your password${sec.twoFactorEnabled ? ' and a 2FA or recovery code' : ''} to confirm deletion.</span>
                    <input id="delete-password" type="password" autocomplete="current-password" placeholder="Account password" />
                    ${sec.twoFactorEnabled ? '<input id="delete-code" type="text" inputmode="numeric" autocomplete="one-time-code" placeholder="2FA or recovery code" />' : ''}
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
            <div class="settings-row-desc">Beatlab account</div>
          </div>
        </div>
        <button class="btn-secondary" style="margin-top:12px;width:100%" onclick="App.closeModal();App.openMyProfile()">View my public profile →</button>
      </div>
      <div class="settings-section">
        <div class="settings-section-title">Account Actions</div>
        <button class="btn-secondary" style="width:100%;margin-bottom:8px" onclick="App.setSettingsTab('security')">Security &amp; two-factor authentication &rarr;</button>
        <button class="btn-secondary" style="width:100%;margin-bottom:8px" onclick="App.setSettingsTab('sessions')">Manage active sessions &rarr;</button>
        ${state.user && state.user.isAdmin ? '<button class="btn-secondary" style="width:100%;margin-bottom:8px" onclick="window.location.href=\'/admin\'">Open admin panel &rarr;</button>' : ''}
        <button type="button" class="btn-secondary danger-btn" style="width:100%" onclick="App.signOut()">Sign out</button>
      </div>
    `;
  }

  function renderCuratorSettingsSection() {
    const cur = state.settingsData.curator;
    const enabled = Boolean(cur ? cur.enabled : state.user?.llmEnabled);
    if (!enabled) return '';
    const note = cur ? String(cur.tasteNote || '') : '';
    const maxChars = (cur && cur.maxChars) || 280;
    const saving = Boolean(state.settingsData.curatorSaving);
    const inFlight = Boolean(cur && cur.inFlight);
    const cooldownSecs = (cur && cur.cooldownSecs) || 0;
    const cooldownMins = cooldownSecs > 0 ? Math.ceil(cooldownSecs / 60) : 0;
    const editionMeta = cur && cur.editionTitle
      ? `Current edition: ${cur.editionTitle}${cur.dateLabel ? ` · ${cur.dateLabel}` : ''}`
      : 'Applied on your next overnight edition or when you refresh.';
    const statusText = inFlight
      ? 'Refreshing your daily edition in the background…'
      : cooldownMins > 0
        ? `${editionMeta} · Manual refresh available in ~${cooldownMins} min`
        : editionMeta;
    const refreshDisabled = saving || inFlight || cooldownSecs > 0;
    return `
      <div class="settings-section">
        <div class="settings-section-title">Daily Curator</div>
        <div class="curator-note-wrap">
          <div class="settings-row-label">Personal Curation Note</div>
          <div class="settings-row-desc">Steer your overnight Daily Mixes, Explore categories, custom moods, Taste Portrait, and AI Instant Mixes.</div>
          <textarea
            id="curator-taste-note"
            class="curator-note-input"
            rows="3"
            maxlength="${maxChars}"
            placeholder="e.g. More 90s jazz-rap and warm acoustic mornings; less anime OSTs on weekdays; road trip energy this week"
            oninput="App.onCuratorNoteInput(this)"
          >${esc(note)}</textarea>
          <div class="curator-note-meta">
            <span class="settings-row-desc">${esc(statusText)}</span>
            <span id="curator-note-count" class="curator-note-count">${note.length}/${maxChars}</span>
          </div>
          <div class="curator-note-actions">
            <button type="button" class="btn-primary" ${saving ? 'disabled' : ''} onclick="App.saveCuratorPrefs(false)">${saving ? 'Saving…' : 'Save Note'}</button>
            <button type="button" class="btn-secondary" ${refreshDisabled ? 'disabled' : ''} onclick="App.saveCuratorPrefs(true)">${inFlight ? 'Refreshing…' : 'Save & Refresh Edition'}</button>
          </div>
        </div>
      </div>
    `;
  }

  async function openSettingsModal(initialTab = 'general') {
    state.settingsTab = initialTab || 'general';
    state.settingsData.confirmResetHidden = false;
    state.settingsData.confirmRevokeAll = false;
    state.settingsData.showPasswordForm = false;
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
            <button class="settings-nav-btn" data-stab="account" onclick="App.setSettingsTab('account')">Account</button>
          </nav>
          <div id="settings-body" class="settings-body"></div>
        </div>
      </div>
    `, 'settings-modal');
    renderSettingsModalBody();

    try {
      const [hidRes, sessRes, secRes, curRes] = await Promise.all([
        api('/api/web/hidden').catch(() => null),
        api('/api/web/sessions').catch(() => null),
        api('/api/web/2fa/status').catch(() => null),
        api('/api/web/curator/prefs').catch(() => null),
      ]);
      if (hidRes) state.settingsData.hidden = hidRes;
      if (sessRes) state.settingsData.sessions = sessRes;
      if (secRes) state.settingsData.security = secRes;
      if (curRes) state.settingsData.curator = curRes;
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

  function shareProfile(username) {
    if (!username) return;
    haptic('success');
    const data = state.social.profileData;
    const u = data && data.user && data.user.username === username ? data.user : { username };
    const isSelf = !!(data && data.isSelf && data.user && data.user.username === username);
    const url = `${location.origin}/u/${encodeURIComponent(username)}`;
    state.currentShareUrl = url;
    const nativeShareBtn =
      typeof navigator.share === 'function'
        ? `<button class="btn-secondary" onclick="App.nativeShareCurrent()">Share via Device</button>`
        : '';
    openModal(`
      <div class="modal-head">
        <h3>Share ${isSelf ? 'your profile' : 'profile'}</h3>
        <button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button>
      </div>
      <div class="share-preview-card">
        <div class="profile-avatar" style="width:64px;height:64px;font-size:26px;flex-shrink:0;background:${esc(u.avatarColor || '#1ed760')}">${avatarInner(u)}</div>
        <div class="share-preview-meta">
          <span class="share-kind-pill">PUBLIC PROFILE LINK</span>
          <div class="share-preview-title">${esc(u.displayName || u.username)}</div>
          <div class="share-preview-sub">@${esc(u.username)}</div>
        </div>
      </div>
      <div class="share-url-row">
        <input id="share-url-input" type="text" readonly value="${esc(url)}" onclick="this.select()" />
        <button class="btn-primary" onclick="App.copyText(document.getElementById('share-url-input').value, 'Profile link copied!')">Copy Link</button>
      </div>
      <div class="share-modal-note">Anyone with this link can view the public profile — no sign-in needed. It has tabs for stats, public playlists, followers and following.</div>
      <div class="share-modal-actions">
        <a class="btn-secondary" href="${esc(url)}" target="_blank" rel="noopener">Open Public Page ↗</a>
        ${nativeShareBtn}
      </div>
    `);
  }

  function openMyProfile() {
    closeUserMenu();
    navigate('profile');
  }

  const PROFILE_TABS = ['stats', 'radio', 'playlists', 'followers', 'following'];

  // `profile[/tab]` and `user/<ref>[/tab]` → { reference, tab }.
  function parseProfileHash(hash) {
    if (hash === 'profile' || hash.startsWith('profile/')) {
      const tab = hash.slice('profile/'.length);
      return { reference: 'me', tab: PROFILE_TABS.includes(tab) ? tab : '' };
    }
    const [refEnc = '', tab = ''] = hash.slice('user/'.length).split('/');
    return { reference: safeDecodeUri(refEnc), tab: PROFILE_TABS.includes(tab) ? tab : '' };
  }

  // Keeps the address bar in sync with the open tab so it can be bookmarked,
  // shared and walked with back/forward. pushState doesn't fire `hashchange`,
  // so this never triggers a re-render by itself.
  function syncProfileTabUrl(data, tab) {
    const current = location.hash.replace(/^#/, '');
    const base = current === 'profile' || current.startsWith('profile/')
      ? 'profile'
      : `user/${encodeURIComponent(data.user.username)}`;
    const next = `${base}/${tab}`;
    if (current !== next) history.pushState(null, '', `#${next}`);
  }

  function setProfileTab(tab, { updateUrl = true } = {}) {
    haptic('tap');
    state.social.profileTab = tab;
    const data = state.social.profileData;
    if (!data) return;
    data.tab = tab;
    if (updateUrl) syncProfileTabUrl(data, tab);
    const body = $('profile-tab-body');
    if (body) {
      body.innerHTML = renderProfileTabBody(data);
      afterProfileTabRender(data);
      body.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }
    document.querySelectorAll('#profile-tabs .tab-chip').forEach((el) => {
      el.classList.toggle('active', el.dataset.ptab === tab);
    });
  }

  const PROFILE_STATS_LOADING = `
      <div class="main-loading-state" role="status" aria-live="polite">
        <div class="loading-spinner" aria-hidden="true"></div>
        <p class="loading-label">Crunching listening stats...</p>
      </div>`;

  function profileStatsKey(user, range) {
    return `${user.id}:${range}`;
  }

  // The Stats tab shows the same full stats screen as the personal stats page.
  // Its HTML is a loading placeholder until the payload arrives (or is cached).
  function afterProfileTabRender(data) {
    if ((data.tab || state.social.profileTab) !== 'stats') return;
    const s = state.social;
    if (s.profileStatsData && s.profileStatsKey === profileStatsKey(data.user, s.profileStatsRange)) {
      renderStatsView({ target: data.user });
      return;
    }
    loadProfileStats(s.profileStatsRange);
  }

  async function loadProfileStats(range = 'all', { keepContent = false } = {}) {
    const s = state.social;
    const data = s.profileData;
    if (!data) return;
    const user = data.user;
    const cleanRange = ['7d', '30d', '90d', '365d', 'all'].includes(range) ? range : 'all';
    s.profileStatsRange = cleanRange;
    const req = ++s.profileStatsReq;
    const body = $('profile-tab-body');
    if (body && !keepContent) {
      body.innerHTML = PROFILE_STATS_LOADING;
    } else if (body) {
      body.querySelector('.stats-page')?.classList.add('stats-refreshing');
    }
    try {
      const tz = -new Date().getTimezoneOffset();
      const d = await api(
        `/api/web/social/stats/${encodeURIComponent(user.username)}?range=${encodeURIComponent(cleanRange)}&tz=${encodeURIComponent(tz)}`
      );
      const cur = s.profileData;
      if (req !== s.profileStatsReq || !cur || cur.user.id !== user.id || cur.tab !== 'stats') return;
      cacheTracks((d.topTracks || []).map((x) => x.track));
      cacheTracks((d.mostSkipped || []).map((x) => x.track));
      if (d.highlights) {
        for (const k of ['longestTrack', 'shortestTrack', 'firstTrackEver', 'mostCompletedTrack']) {
          if (d.highlights[k]?.track) cacheTracks([d.highlights[k].track]);
        }
      }
      s.profileStatsData = d;
      s.profileStatsKey = profileStatsKey(user, cleanRange);
      renderStatsView({ target: user });
    } catch (e) {
      if (req !== s.profileStatsReq) return;
      const b = $('profile-tab-body');
      if (b && s.profileData?.tab === 'stats') {
        b.innerHTML = emptyState('Couldn’t load stats', e?.message || 'Please try again.');
      }
    }
  }

  function renderProfileTabBody(data) {
    const tab = data.tab || state.social.profileTab;
    if (tab === 'stats') {
      return PROFILE_STATS_LOADING;
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

  async function renderProfileView(reference, tab = '') {
    // A bare profile URL means the default tab.
    tab = tab || 'stats';
    // Back/forward between tabs of the profile already on screen: just swap
    // the tab body instead of refetching everything. Re-opening the tab that's
    // already showing still does a full refresh.
    const current = state.social.profileData;
    if (
      current &&
      current.tab !== tab &&
      $('profile-tab-body') &&
      (reference === 'me'
        ? current.isSelf
        : String(reference).toLowerCase() === String(current.user.username).toLowerCase())
    ) {
      setProfileTab(tab, { updateUrl: false });
      return;
    }
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
      const nowPlaying = profileRes.nowPlaying && profileRes.nowPlaying.track ? profileRes.nowPlaying : null;
      cacheTracks(radio.tracks || []);
      if (nowPlaying) cacheTracks([nowPlaying.track]);
      const isSelf = selfRef || !!u.isSelf;
      const defaultTab = 'stats';
      if (state.social.profileData && state.social.profileData.user.id !== u.id) {
        state.social.profileTab = defaultTab;
        state.social.profileStatsRange = 'all';
      }
      state.social.profileTab = tab;
      // Re-opening a profile always fetches fresh stats.
      state.social.profileStatsData = null;
      state.social.profileStatsKey = '';
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

      const nowPlayingHtml = nowPlaying
        ? (() => {
            const nt = nowPlaying.track;
            const npSub = nt.album
              ? `<span class="clickable" onclick="event.stopPropagation();App.navigate('artist/${encodeURIComponent(nt.artistId)}')">${esc(nt.artist)}</span> · <span class="clickable" onclick="event.stopPropagation();App.navigate('album/${encodeURIComponent(nt.albumId)}')">${esc(nt.album)}</span>`
              : `<span class="clickable" onclick="event.stopPropagation();App.navigate('artist/${encodeURIComponent(nt.artistId)}')">${esc(nt.artist)}</span>`;
            return `<div class="profile-now-playing" onclick="App.playSingle(${esc(JSON.stringify(nt.id))})" role="button" tabindex="0" aria-label="Play ${esc(nt.title)}">
              <img class="profile-np-art" src="${esc(nt.coverUrl)}" loading="lazy" alt="" />
              <div class="profile-np-info">
                <div class="profile-np-kicker"><span class="profile-np-eq" aria-hidden="true"><span></span><span></span><span></span></span>Currently listening</div>
                <div class="profile-np-title">${esc(nt.title)}</div>
                <div class="profile-np-sub">${npSub}</div>
              </div>
              <button class="icon-btn profile-np-play" title="Play track" onclick="event.stopPropagation();App.playSingle(${esc(JSON.stringify(nt.id))})">${ICONS.play}</button>
            </div>`;
          })()
        : '';

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
            ${data.profile.publicPage !== false ? `<button class="btn-secondary" onclick="App.shareProfile(${esc(JSON.stringify(u.username))})">🔗 Share profile</button>` : ''}
          </div>
        </div>
      </div>
      ${nowPlayingHtml}
      <div class="tabs-bar" id="profile-tabs">
        ${tabs.map(([key, label]) => `<button class="tab-chip ${data.tab === key ? 'active' : ''}" data-ptab="${key}" onclick="App.setProfileTab('${key}')">${label}</button>`).join('')}
      </div>
      <div id="profile-tab-body">${renderProfileTabBody(data)}</div>`;

      $('main-view').innerHTML = html;
      $('main-view').scrollTop = 0;
      afterProfileTabRender(data);
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
      <div class="social-edit-row">
        <label style="display:flex;align-items:flex-start;gap:10px;cursor:pointer">
          <input type="checkbox" id="prof-public-page" style="width:auto;margin-top:3px" ${p.publicPage !== false ? 'checked' : ''} />
          <span>Public profile link<br><small style="opacity:.7;font-weight:400">Let anyone with your link (${esc(location.origin)}/u/${esc(state.user.username)}) see your profile without signing in. Turning this off doesn't affect your profile inside Beatlab.</small></span>
        </label>
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
          publicPage: !!$('prof-public-page')?.checked,
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
    shareProfile,
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
      // Stay on the current tab. Only refresh the view when it is a search
      // page, so clearing the box never yanks the user to Explore.
      if (isSearchHash(location.hash.replace(/^#/, ''))) {
        navigate('search');
      }
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
    setAuthMode,
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
      const box = $('auth-mfa') && !$('auth-mfa').classList.contains('hidden') ? $('mfa-error') : $('auth-error');
      if (box) box.classList.add('hidden');
      try {
        const opts = await api('/api/web/auth/passkey/options', {
          method: 'POST',
          body: JSON.stringify(username ? { username } : {}),
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
        if (box) {
          box.textContent = err.message;
          box.classList.remove('hidden');
        }
      }
    },
    setLibTab: (tab) => {
      haptic('tap');
      const nextHash = tab === 'liked' ? '#liked' : tab === 'history' ? '#history' : '#library';
      if (location.hash !== nextHash && history.replaceState) {
        history.replaceState(null, '', nextHash);
      }
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
    setProfileStatsRange: (range) => {
      haptic('tap');
      loadProfileStats(range, { keepContent: true });
    },
    setProfileStatsMetric: (metric) => {
      haptic('tap');
      state.statsMetric = metric === 'mins' ? 'mins' : 'streams';
      const u = state.social.profileData?.user;
      if (u) renderStatsView({ target: u });
    },
    refreshProfileStats: () => loadProfileStats(state.social.profileStatsRange, { keepContent: true }),
    setRightTab: (tab) => {
      haptic('tap');
      $('tab-queue').classList.toggle('active', tab === 'queue');
      $('tab-lyrics').classList.toggle('active', tab === 'lyrics');
      $('rp-queue-body').classList.toggle('hidden', tab !== 'queue');
      $('rp-lyrics-body').classList.toggle('hidden', tab !== 'lyrics');
      scheduleRightPanelContentSync('smooth');
    },
    setFsMobileView,
    toggleFsMobileView: (view) => {
      setFsMobileView(state.fsMobileView === view ? 'player' : view);
    },
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
      if (isMobileViewport()) {
        delete rp.dataset.tabletToggled;
        rp.classList.add('hidden');
        if (backdrop) backdrop.classList.add('hidden');
        return;
      }
      const hide = show === false;
      const isDesktop = !isTabletOrMobileViewport();
      if (isDesktop) {
        state.desktopRightPanelClosed = hide;
        saveDesktopRightPanelClosed(hide);
      } else {
        rp.dataset.tabletToggled = '1';
      }
      rp.classList.toggle('hidden', hide);
      if (backdrop) {
        backdrop.classList.toggle('hidden', hide || isDesktop);
      }
      if (!hide) scheduleRightPanelContentSync('smooth');
    },
    openPanelTab: (tab) => {
      if (isMobileViewport()) {
        setExpandedPlayer(true);
        setFsMobileView(tab === 'lyrics' ? 'lyrics' : 'queue');
        return;
      }
      if ($('fullscreen-player').classList.contains('open')) {
        window.App.setFsTab(tab);
        return;
      }
      const rp = $('right-panel');
      const isDesktop = !isTabletOrMobileViewport();
      if (isDesktop) {
        state.desktopRightPanelClosed = false;
        saveDesktopRightPanelClosed(false);
      } else {
        rp.dataset.tabletToggled = '1';
      }
      rp.classList.remove('hidden');
      if ($('panel-backdrop')) {
        $('panel-backdrop').classList.toggle('hidden', isDesktop);
      }
      window.App.setRightTab(tab);
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
    removeFromQueue: (idx, expectedTrackId = null) => {
      let targetIdx = idx;
      if (
        expectedTrackId != null &&
        !(targetIdx > state.queueIndex && targetIdx < state.queue.length && state.queue[targetIdx]?.id === expectedTrackId)
      ) {
        targetIdx = state.queue.findIndex((t, i) => i > state.queueIndex && t.id === expectedTrackId);
      }
      if (targetIdx > state.queueIndex && targetIdx < state.queue.length) {
        haptic('tap');
        const [removed] = state.queue.splice(targetIdx, 1);
        if (removed && Array.isArray(state.unshuffledQueue)) {
          const uIdx = state.unshuffledQueue.findIndex((item) => item.id === removed.id);
          if (uIdx >= 0) state.unshuffledQueue.splice(uIdx, 1);
        }
        renderQueuePanel();
        saveServerQueue();
        if (targetIdx === state.queueIndex + 1) scheduleWarmNextTrack();
        showToast('Removed from queue');
      }
    },
    startRadioForId,
    setUpcomingMode,
    retryUpcoming: () => {
      const cur = currentTrack();
      if (!cur || state.radioLoading) return;
      state.radioError = false;
      appendRadioTracks(cur.id);
    },
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
        state.intentionalPause = false;
        audio.currentTime = ms / 1000;
        if (audio.paused) audio.play().catch(() => {});
      }
    },
    openTrackMenu,
    openCurrentTrackMenu: (event) => {
      if (event && typeof event.stopPropagation === 'function') {
        event.stopPropagation();
      }
      const cur = currentTrack();
      if (!cur) return showToast('Play a track first');
      if (!state.trackMap.has(cur.id)) state.trackMap.set(cur.id, cur);
      let posEvent = event;
      if (event && event.currentTarget && event.clientX === 0 && event.clientY === 0) {
        const r = event.currentTarget.getBoundingClientRect();
        posEvent = { clientX: r.left, clientY: r.top, stopPropagation: () => {} };
      }
      openTrackMenu(posEvent, cur.id);
    },
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
      state.settingsData.showPasswordForm = false;
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
    refreshSecurityStatus: async () => {
      try {
        state.settingsData.security = await api('/api/web/2fa/status');
      } catch {}
      if ($('settings-body') && state.settingsTab === 'security') renderSettingsModalBody();
    },
    startTotpSetup: async () => {
      try {
        const setup = await apiWithStepUp(
          '/api/web/2fa/totp/start',
          'Your account already has a passkey. Enter an unused recovery code (or verify with your passkey first) to add an authenticator app.'
        );
        if (setup) state.settingsData.totpSetup = setup;
      } catch (err) {
        showToast(err.message);
      }
      // A step-up prompt replaces the settings modal; reopen it if so.
      if ($('settings-body')) renderSettingsModalBody();
      else openSettingsModal('security');
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
    promptGenerateRecoveryCodes: async () => {
      const code = await promptModal({
        title: 'Confirm your identity',
        message: 'Enter a code from your authenticator app or an unused recovery code before generating a new set.',
        label: '2FA or recovery code',
        placeholder: '000000',
        confirmLabel: 'Generate codes',
      });
      if (code === null) {
        openSettingsModal('security');
        return;
      }
      await window.App.generateRecoveryCodes(code);
    },
    generateRecoveryCodes: async (code) => {
      try {
        const res = await api('/api/web/2fa/recovery', {
          method: 'POST',
          body: JSON.stringify({ code: code || securityCodeValue() || undefined }),
        });
        state.settingsData.recoveryCodes = res.recoveryCodes;
        openSettingsModal('security');
        showToast('New recovery codes generated');
      } catch (err) {
        openSettingsModal('security');
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
      const label = await promptModal({
        title: 'Name your passkey',
        message: 'Pick a name you will recognise later, like “This laptop” or “My phone”.',
        label: 'Passkey name',
        placeholder: 'e.g. This laptop',
        defaultValue: 'This device',
        confirmLabel: 'Continue',
      });
      if (label === null) {
        openSettingsModal('security');
        return;
      }
      try {
        const opts = await apiWithStepUp(
          '/api/web/2fa/passkey/options',
          'Two-factor authentication is on. Enter a code from your authenticator app or an unused recovery code to add another passkey.'
        );
        if (!opts) {
          openSettingsModal('security');
          return;
        }
        const credential = await navigator.credentials.create({
          publicKey: webauthnOptions(opts.publicKey, true),
        });
        if (!credential) throw new Error('Passkey registration was cancelled');
        const result = await api('/api/web/2fa/passkey/verify', {
          method: 'POST',
          body: JSON.stringify({
            challenge: opts.challenge,
            credential: webauthnCredentialToJson(credential),
            label,
          }),
        });
        if (result.recoveryCodes && result.recoveryCodes.length) {
          state.settingsData.recoveryCodes = result.recoveryCodes;
        }
        showToast('Passkey added');
        await window.App.refreshSecurityStatus();
        openSettingsModal('security');
      } catch (err) {
        showToast(err.message);
        openSettingsModal('security');
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
      const code = await promptModal({
        title: 'Confirm your identity',
        message: 'Enter a code from your authenticator app or an unused recovery code to remove this passkey.',
        label: '2FA or recovery code',
        placeholder: '000000',
        confirmLabel: 'Remove passkey',
      });
      if (code === null) {
        openSettingsModal('security');
        return;
      }
      try {
        const query = code ? `?code=${encodeURIComponent(code)}` : '';
        await api(`/api/web/2fa/passkey/${encodeURIComponent(id)}${query}`, { method: 'DELETE' });
        showToast('Passkey removed');
        await window.App.refreshSecurityStatus();
        openSettingsModal('security');
      } catch (err) {
        showToast(err.message);
        openSettingsModal('security');
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
      const codeInput = $('disable-code');
      const code = codeInput ? codeInput.value.trim() : '';
      try {
        await api('/api/web/2fa/disable', {
          method: 'POST',
          body: JSON.stringify({ code: code || undefined }),
        });
        if (state.user) {
          state.user.twoFactorEnabled = false;
          state.user.totpEnabled = false;
          state.user.passkeys = 0;
        }
        state.settingsData.confirmDisable2fa = false;
        state.settingsData.recoveryCodes = null;
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
      const codeInput = $('delete-code');
      if (codeInput && !codeInput.value.trim()) {
        showToast('Enter a 2FA or recovery code to confirm');
        return;
      }
      try {
        const res = await api('/api/web/account/delete', {
          method: 'POST',
          body: JSON.stringify({
            password: passwordInput.value,
            code: codeInput ? codeInput.value : undefined,
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
      const btn = $('save-alb-btn');
      const matchesBtn = btn && btn.dataset.albumId === id;
      if (matchesBtn && btn.dataset.busy === '1') return;
      const prev = matchesBtn ? btn.dataset.saved === '1' : !saved;
      const next = typeof saved === 'boolean' ? saved : !prev;
      const paintBtn = (isSaved) => {
        if (!matchesBtn) return;
        btn.dataset.saved = isSaved ? '1' : '0';
        btn.classList.toggle('active', isSaved);
        btn.innerHTML = `${ICONS.heart} ${isSaved ? 'Saved' : 'Save Album'}`;
      };
      haptic(next ? 'success' : 'tap');
      if (matchesBtn) btn.dataset.busy = '1';
      paintBtn(next);
      try {
        await api('/api/web/star', {
          method: 'POST',
          body: JSON.stringify({ id, starred: next, kind: 'album' }),
        });
        if (state.searchState?.albums) {
          for (const a of state.searchState.albums) {
            if (a.id === id) a.isSaved = next;
          }
        }
      } catch (e) {
        paintBtn(prev);
        showToast(e.message);
      } finally {
        if (matchesBtn) delete btn.dataset.busy;
      }
    },
    toggleFollowArtist: async (id, followed) => {
      const btn = $('follow-art-btn');
      const matchesBtn = btn && btn.dataset.artistId === id;
      if (matchesBtn && btn.dataset.busy === '1') return;
      const prev = matchesBtn ? btn.dataset.followed === '1' : !followed;
      const next = typeof followed === 'boolean' ? followed : !prev;
      const paintBtn = (isFollowed) => {
        if (!matchesBtn) return;
        btn.dataset.followed = isFollowed ? '1' : '0';
        btn.classList.toggle('active', isFollowed);
        btn.textContent = isFollowed ? '✓ Following' : '+ Follow';
      };
      haptic(next ? 'success' : 'tap');
      if (matchesBtn) btn.dataset.busy = '1';
      paintBtn(next);
      try {
        await api('/api/web/star', {
          method: 'POST',
          body: JSON.stringify({ id, starred: next, kind: 'artist' }),
        });
        if (state.searchState?.artists) {
          for (const a of state.searchState.artists) {
            if (a.id === id) a.isFollowed = next;
          }
        }
      } catch (e) {
        paintBtn(prev);
        showToast(e.message);
      } finally {
        if (matchesBtn) delete btn.dataset.busy;
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
      navigate(`mix?q=${encodeURIComponent(val)}`);
    },
    playInstantMix,
    playInstantMixAt,
    queueInstantMix,
    promptSaveUpcoming,
    setSaveQueueIncludeCurrent,
    setSaveQueueMode,
    submitSaveQueueNew,
    submitSaveQueueToExisting,
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
      } else if (isLibraryHash(curHash) && state.libTab === 'playlists') {
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
        state.playlists = (state.playlists || []).filter((p) => p.id !== pid);
        renderSidebarPlaylistsDom();
        document
          .querySelectorAll('#main-view .media-card[data-share-kind="playlist"]')
          .forEach((el) => {
            if (el.dataset.shareId === pid) el.remove();
          });
        if (state.activeInstantMix?.mix?.savedPlaylistId === pid) {
          delete state.activeInstantMix.mix.savedPlaylistId;
        }
        if (state.searchState?.instantMix?.savedPlaylistId === pid) {
          delete state.searchState.instantMix.savedPlaylistId;
        }
        refreshActiveInstantMixCard();
        showToast('Playlist deleted');
        const curHash = location.hash.replace(/^#/, '');
        if (curHash === `playlist/${encodeURIComponent(pid)}` || curHash === `playlist/${pid}`) {
          navigate('playlists');
          await refreshSidebarPlaylists();
        } else if (curHash === 'playlists') {
          await renderPlaylistsView();
        } else if (isLibraryHash(curHash)) {
          await renderLibrary(state.libTab || 'playlists');
        } else if (
          curHash === 'profile' ||
          curHash.startsWith('profile/') ||
          curHash.startsWith('user/')
        ) {
          renderCurrentHash();
          await refreshSidebarPlaylists();
        } else {
          await refreshSidebarPlaylists();
        }
      } catch (err) {
        showToast(err.message);
      }
    },
    closeModal,
    installPwa,
    onCuratorNoteInput: (el) => {
      const cnt = $('curator-note-count');
      const max = (state.settingsData.curator && state.settingsData.curator.maxChars) || 280;
      if (cnt && el) cnt.textContent = `${el.value.length}/${max}`;
    },
    saveCuratorPrefs: async (refreshNow = false) => {
      const input = $('curator-taste-note');
      const note = input ? input.value : '';
      state.settingsData.curatorSaving = true;
      renderSettingsModalBody();
      try {
        const res = await api('/api/web/curator/prefs', {
          method: 'POST',
          body: JSON.stringify({ tasteNote: note, refreshNow: Boolean(refreshNow) }),
        });
        state.settingsData.curator = {
          ...(state.settingsData.curator || {}),
          enabled: res.enabled,
          tasteNote: res.tasteNote || '',
          inFlight: Boolean(res.inFlight),
          cooldownSecs: res.cooldownSecs || 0,
        };
        if (refreshNow) {
          if (res.refreshStarted) {
            showToast('Curation note saved · Refreshing your daily edition in the background');
          } else if (res.cooldownSecs > 0) {
            showToast(`Curation note saved · Refresh available in ~${Math.ceil(res.cooldownSecs / 60)} min`);
          } else {
            showToast('Curation note saved · Edition refresh already running');
          }
        } else {
          showToast(res.tasteNote ? 'Personal curation note saved' : 'Personal curation note cleared');
        }
      } catch (err) {
        showToast(err.message || 'Could not save curation note');
      } finally {
        state.settingsData.curatorSaving = false;
        if ($('settings-body')) renderSettingsModalBody();
      }
    },
    showPasswordForm: () => {
      state.settingsData.showPasswordForm = true;
      renderSettingsModalBody();
      const first = $('pw-cur');
      if (first) first.focus();
    },
    cancelPasswordChange: () => {
      state.settingsData.showPasswordForm = false;
      renderSettingsModalBody();
    },
    submitPasswordChange: async (e) => {
      e.preventDefault();
      try {
        const res = await api('/api/web/account/password', {
          method: 'POST',
          body: JSON.stringify({
            currentPassword: $('pw-cur').value,
            newPassword: $('pw-new').value,
            code: $('pw-code') ? $('pw-code').value : undefined,
          }),
        });
        state.settingsData.showPasswordForm = false;
        const others = res && res.signedOut > 0 ? res.signedOut : 0;
        showToast(
          others
            ? `Password updated · signed out ${others} other ${others === 1 ? 'session' : 'sessions'}`
            : 'Password updated'
        );
        renderSettingsModalBody();
      } catch (err) {
        showToast(err.message);
      }
    },
  };

  init();
})();
