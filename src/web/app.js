(() => {
  'use strict';

  const EQ_FREQS = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
  const EQ_LABELS = ['32', '64', '125', '250', '500', '1k', '2k', '4k', '8k', '16k'];
  const EQ_PRESETS = {
    Flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    'Bass Boost': [6, 5, 4, 2, 0, 0, 0, 1, 2, 2],
    Vocal: [-2, -1, 0, 2, 4, 5, 4, 2, 1, 0],
    Electronic: [5, 4, 1, -2, -1, 1, 2, 4, 5, 4],
    Rock: [4, 3, 1, -1, -2, 0, 2, 4, 5, 4],
    'Late Night': [3, 2, 1, 0, 0, -1, -1, 0, 1, 2],
  };

  const audio = document.getElementById('audio-el');
  let audioCtx = null;
  let audioSource = null;
  let eqFilters = [];
  let analyser = null;
  let rafVisualizer = null;

  const ICONS = {
    play: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M8 4.5v15l12-7.5z"/></svg>',
    heart: '<svg class="heart-svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/></svg>',
    close: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12"/></svg>',
    arrowUp: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5M5 12l7-7 7 7"/></svg>',
    more: '<svg viewBox="0 0 24 24" fill="currentColor"><circle cx="5" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="19" cy="12" r="2"/></svg>',
    eyeOff: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94"/><path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19"/><path d="M14.12 14.12a3 3 0 1 1-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>',
    nowPlaying: '<svg viewBox="0 0 24 24" fill="currentColor"><rect x="4" y="10" width="3" height="10" rx="1"/><rect x="10.5" y="4" width="3" height="16" rx="1"/><rect x="17" y="7" width="3" height="13" rx="1"/></svg>',
    volOn: '<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.5 8.5a5 5 0 0 1 0 7"/><path d="M19 5a10 10 0 0 1 0 14"/>',
    volMuted: '<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="22" y1="9" x2="16" y2="15"/><line x1="16" y1="9" x2="22" y2="15"/>',
    repeatAll: '<polyline points="17 2 21 6 17 10"/><path d="M3 12v-2a4 4 0 0 1 4-4h14"/><polyline points="7 22 3 18 7 14"/><path d="M21 12v2a4 4 0 0 1-4 4H3"/>',
    repeatOne: '<polyline points="17 2 21 6 17 10"/><path d="M3 12v-2a4 4 0 0 1 4-4h14"/><polyline points="7 22 3 18 7 14"/><path d="M21 12v2a4 4 0 0 1-4 4H3"/><path d="M11 10.2 12.4 9.2V15"/>',
  };

  function loadEqGains() {
    const flat = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    try {
      const parsed = JSON.parse(localStorage.getItem('beatlab_eq_gains') || 'null');
      if (Array.isArray(parsed) && parsed.length === EQ_FREQS.length) {
        return parsed.map((v) => {
          const n = Number(v);
          return Number.isFinite(n) ? Math.max(-12, Math.min(12, n)) : 0;
        });
      }
    } catch {}
    return flat;
  }

  const state = {
    token: localStorage.getItem('beatlab_token') || '',
    user: null,
    playlists: [],
    trackMap: new Map(), // id -> track object for instant access
    queue: [],
    queueIndex: -1,
    pendingSeekSec: 0,
    shuffle: false,
    repeat: 'off', // off | all | one
    autoplay: localStorage.getItem('beatlab_autoplay') !== 'false',
    radioReqToken: 0,
    radioLoading: false,
    compatAttemptedId: '',
    volume: parseFloat(localStorage.getItem('beatlab_vol') || '0.9'),
    muted: false,
    seeking: false,
    lastScrobbleCompleteId: '',
    fsTab: 'queue',
    fsMobileView: 'player',
    lyrics: { synced: false, plain: '', lines: [] },
    activeLyricIdx: -1,
    eqEnabled: localStorage.getItem('beatlab_eq_on') === 'true',
    eqGains: loadEqGains(),
    searchTimer: null,
    searchComposing: false,
    ownSearchNavQuery: null,
    searchFetchingQuery: '',
    searchLoadingUi: false,
    viewReqToken: 0,
    viewAbortController: null,
    libTab: 'playlists',
    searchState: null,
    settingsTab: 'general',
    settingsData: {
      sources: null,
      hidden: null,
      confirmResetHidden: false,
    },
    deferredInstallPrompt: null,
    isStandalone: false,
    hasViewNetworkError: false,
  };

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

  function updateInstallUI() {
    state.isStandalone = isStandalonePwa();
    const btn = $('pwa-install-btn');
    if (btn) {
      btn.classList.toggle('hidden', state.isStandalone || !state.deferredInstallPrompt);
    }
    if ($('settings-body') && state.settingsTab === 'general') {
      renderSettingsModalBody();
    }
  }

  function syncOnlineStatus() {
    const online = typeof navigator === 'undefined' ? true : navigator.onLine !== false;
    const banner = $('offline-banner');
    if (banner) {
      banner.classList.toggle('hidden', online);
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
    state.isStandalone = isStandalonePwa();
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
      showToast('Beatlab installed!');
    });

    window.addEventListener('offline', () => {
      syncOnlineStatus();
      showToast('You are offline');
    });

    window.addEventListener('online', async () => {
      syncOnlineStatus();
      if (state.token && !state.user) {
        try {
          const res = await api('/api/web/me');
          if (res.user) {
            state.user = res.user;
            await onSignedIn();
            showToast('Back online');
            return;
          }
        } catch {}
      } else if (state.user && state.hasViewNetworkError) {
        state.hasViewNetworkError = false;
        showToast('Back online');
        renderCurrentHash();
      }
    });
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
    const resp = await fetch(path, { ...opts, headers });
    if (resp.status === 401 && path !== '/api/web/auth/signin' && path !== '/api/web/auth/signup') {
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
  // Web Audio Equalizer & Visualizer
  // ---------------------------------------------------------------------------
  function ensureAudioGraph() {
    if (audioCtx) {
      if (audioCtx.state === 'suspended') {
        audioCtx.resume().catch(() => {});
      }
      return;
    }
    try {
      const AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) return;
      audioCtx = new AC();
      audioSource = audioCtx.createMediaElementSource(audio);
      analyser = audioCtx.createAnalyser();
      analyser.fftSize = 128;

      eqFilters = EQ_FREQS.map((freq, idx) => {
        const f = audioCtx.createBiquadFilter();
        f.type = idx === 0 ? 'lowshelf' : idx === EQ_FREQS.length - 1 ? 'highshelf' : 'peaking';
        f.frequency.value = freq;
        f.Q.value = 1.2;
        f.gain.value = state.eqEnabled ? (state.eqGains[idx] || 0) : 0;
        return f;
      });

      let node = audioSource;
      for (const f of eqFilters) {
        node.connect(f);
        node = f;
      }
      node.connect(analyser);
      analyser.connect(audioCtx.destination);
    } catch (e) {
      console.warn('WebAudio init skipped:', e);
    }
  }

  function applyEqGains() {
    localStorage.setItem('beatlab_eq_on', state.eqEnabled ? 'true' : 'false');
    localStorage.setItem('beatlab_eq_gains', JSON.stringify(state.eqGains));
    eqFilters.forEach((f, idx) => {
      if (f) f.gain.value = state.eqEnabled ? (state.eqGains[idx] || 0) : 0;
    });
  }

  function setExpandedPlayer(open) {
    const fs = $('fullscreen-player');
    if (!fs) return;
    const next = typeof open === 'boolean' ? open : !fs.classList.contains('open');
    fs.classList.toggle('open', next);
    fs.setAttribute('aria-hidden', next ? 'false' : 'true');
    if (next && window.innerWidth <= 780) {
      setFsMobileView(state.fsMobileView || 'player');
    }
    const btn = $('btn-expand-player');
    if (btn) {
      btn.classList.toggle('active', next);
      btn.title = next ? 'Collapse player' : 'Expand player';
    }
    if (next && !rafVisualizer) {
      ensureAudioGraph();
      drawVisualizer();
    }
  }

  function setFsMobileView(view) {
    const next = view === 'queue' || view === 'lyrics' ? view : 'player';
    state.fsMobileView = next;
    const fs = $('fullscreen-player');
    if (fs) {
      fs.classList.remove('fs-view-player', 'fs-view-queue', 'fs-view-lyrics');
      fs.classList.add(`fs-view-${next}`);
    }
    if ($('fs-tab-player')) $('fs-tab-player').classList.toggle('active', next === 'player');
    if ($('fs-tab-mobile-queue')) $('fs-tab-mobile-queue').classList.toggle('active', next === 'queue');
    if ($('fs-tab-mobile-lyrics')) $('fs-tab-mobile-lyrics').classList.toggle('active', next === 'lyrics');
    if (next === 'queue' || next === 'lyrics') {
      window.App.setFsTab(next);
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

  function drawVisualizer() {
    const fs = $('fullscreen-player');
    if (!fs.classList.contains('open')) {
      rafVisualizer = null;
      return;
    }
    const canvas = $('fs-visualizer');
    if (canvas && analyser) {
      const ctx = canvas.getContext('2d');
      const data = new Uint8Array(analyser.frequencyBinCount);
      analyser.getByteFrequencyData(data);
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      const bars = 40;
      const bw = canvas.width / bars;
      ctx.fillStyle = '#1ed760';
      for (let i = 0; i < bars; i++) {
        const val = data[i] / 255;
        const h = Math.max(4, val * canvas.height);
        const x = i * bw + 2;
        const y = canvas.height - h;
        ctx.fillRect(x, y, Math.max(3, bw - 4), h);
      }
    }
    rafVisualizer = requestAnimationFrame(drawVisualizer);
  }

  // ---------------------------------------------------------------------------
  // Authentication & Session Bootstrap
  // ---------------------------------------------------------------------------
  function resetAuthForm() {
    const form = $('auth-form');
    if (form) form.reset();
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
    state.queueIndex = -1;
    state.radioReqToken++;
    state.radioLoading = false;
    state.compatAttemptedId = '';
    state.lastScrobbleCompleteId = '';
    state.lyrics = { synced: false, plain: '', lines: [] };
    state.activeLyricIdx = -1;
    state.searchState = null;
    state.settingsData = { sources: null, hidden: null, confirmResetHidden: false };
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

  function wirePlayerBarToggle() {
    const bar = $('player-bar');
    if (!bar) return;
    let pointerDownIgnored = false;
    const isInteractive = (target) => {
      if (window.innerWidth <= 780) {
        return !!target.closest('button, a, input, label, .pb-progress-row');
      }
      return !!target.closest('button, a, input, label, .pb-title, .pb-artist, .pb-progress-row');
    };
    bar.addEventListener('pointerdown', (e) => {
      pointerDownIgnored = isInteractive(e.target);
    });
    bar.addEventListener('click', (e) => {
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
        setFsMobileView('player');
      }
      setExpandedPlayer();
    });
  }

  function wireFsSwipeClose() {
    const fs = $('fullscreen-player');
    if (!fs) return;
    let startY = 0;
    let startX = 0;
    let tracking = false;
    let deltaY = 0;

    fs.addEventListener('touchstart', (e) => {
      if (window.innerWidth > 780 || !fs.classList.contains('open')) return;
      if (e.touches.length !== 1) return;
      if (e.target.closest('input, button, a, .fs-panel-body')) return;
      startY = e.touches[0].clientY;
      startX = e.touches[0].clientX;
      deltaY = 0;
      tracking = true;
      fs.style.transition = 'none';
    }, { passive: true });

    fs.addEventListener('touchmove', (e) => {
      if (!tracking || e.touches.length !== 1) return;
      const dy = e.touches[0].clientY - startY;
      const dx = e.touches[0].clientX - startX;
      if (dy > 8 && Math.abs(dy) > Math.abs(dx) * 1.2) {
        deltaY = dy;
        fs.style.transform = `translateY(${dy}px)`;
      }
    }, { passive: true });

    const finish = () => {
      if (!tracking) return;
      tracking = false;
      fs.style.transition = '';
      fs.style.transform = '';
      if (deltaY > 90) {
        setExpandedPlayer(false);
      }
      deltaY = 0;
    };
    fs.addEventListener('touchend', finish, { passive: true });
    fs.addEventListener('touchcancel', finish, { passive: true });
  }

  async function init() {
    audio.volume = state.volume;
    $('pb-volume').value = String(state.volume);
    updateVolumeIcon();
    localStorage.removeItem('beatlab_quality');
    $('autoplay-toggle').checked = state.autoplay;
    $('fs-autoplay-toggle').checked = state.autoplay;
    wirePlayerBarToggle();
    wireFsSwipeClose();
    wirePwaLifecycle();
    syncResponsiveLayout();
    window.matchMedia('(max-width: 1060px)').addEventListener('change', syncResponsiveLayout);
    window.matchMedia('(max-width: 780px)').addEventListener('change', syncResponsiveLayout);
    window.addEventListener('pageshow', (e) => {
      if (e.persisted && !state.user) {
        resetAuthForm();
      }
    });

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
            setMainLoading('Search songs, artists, or albums...');
          }
        }
        return;
      }

      if (state.searchFetchingQuery && state.searchFetchingQuery !== trimmed) {
        abortActiveViewRequest();
        state.searchLoadingUi = true;
        setMainLoading(`Searching for "${trimmed}" across all sources...`);
      } else if (state.searchLoadingUi) {
        setMainLoading(`Searching for "${trimmed}" across all sources...`);
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
        if (err && err.message !== 'Unauthorized' && !navigator.onLine) {
          showToast('You are offline — waiting to reconnect');
        }
      }
    }
    showAuthScreen();
  }

  async function onSignedIn() {
    resetAuthForm();
    showAppShell();
    $('user-name').textContent = state.user.username;
    $('user-avatar').textContent = (state.user.username[0] || 'B').toUpperCase();
    await Promise.all([refreshSidebarPlaylists(), restoreServerQueue()]);
    renderCurrentHash();
  }

  async function refreshSidebarPlaylists() {
    try {
      const lib = await api('/api/web/library');
      state.playlists = lib.playlists || [];
      const box = $('sidebar-playlists');
      box.innerHTML = state.playlists
        .map(
          (p) => `
          <button class="pl-link" onclick="App.navigate('playlist/${encodeURIComponent(p.id)}')">
            <span class="pl-link-name">${esc(p.name)}</span>
            ${p.isAuto ? '<span class="pl-auto-pill">Mix</span>' : ''}
          </button>`
        )
        .join('');
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

  function saveServerQueue() {
    if (!state.token) return;
    const cur = currentTrack();
    api('/api/web/queue', {
      method: 'PUT',
      body: JSON.stringify({
        currentId: cur ? cur.id : '',
        currentIndex: Math.max(0, state.queueIndex),
        positionMs: Math.floor((audio.currentTime || 0) * 1000),
        trackIds: state.queue.map((t) => t.id),
      }),
    }).catch(() => {});
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
    setExpandedPlayer(false);
    closeTrackMenu();
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
        (r === 'library' && (hash === 'liked' || hash === 'history'));
      el.classList.toggle('active', match);
    });

    if (hash === 'home') return renderHome();
    if (hash === 'explore') return renderExplore();
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
    if (hash === 'library') return renderLibrary(state.libTab);
    if (hash === 'liked') return renderLibrary('liked');
    if (hash === 'history') return renderLibrary('history');
    return renderHome();
  }

  function setMainLoading(label = 'Loading...') {
    $('main-view').innerHTML = `<p class="muted-center">${esc(label)}</p>`;
  }

  async function renderHome() {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Curating your feed...');
    try {
      const d = await api('/api/web/home', { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const quickPicks = cacheTracks(d.quickPicks || []).slice(0, 6);
      const trending = cacheTracks(d.trending || []);
      const recent = cacheTracks(d.recent || []);
      const mixes = d.madeForYou || [];
      const albums = d.albums || [];
      const artists = d.artists || [];
      const moods = d.moods || [];

      let html = `<h1 class="page-greeting">${greetingText()}, ${esc(state.user.username)}</h1>`;

      if (quickPicks.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Quick Picks</h2></div>
          <div class="quick-picks-grid">${quickPicks.map(renderQuickTile).join('')}</div>
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

  async function renderExplore() {
    const { reqToken, signal } = beginViewRequest();
    setMainLoading('Exploring charts & genres...');
    try {
      const d = await api('/api/web/explore', { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const charts = cacheTracks(d.charts || []);
      let html = `<h1 class="page-greeting">Explore</h1>`;
      html += `<section class="section">
        <div class="section-head"><h2>Moods &amp; Moments</h2></div>
        <div class="mood-grid">${(d.moods || []).map(renderMoodTile).join('')}</div>
      </section>`;
      if ((d.genres || []).length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Genres</h2></div>
          <div class="card-grid">${d.genres.slice(0, 18).map((g) => `
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

  async function renderSearch(query) {
    if (!query) {
      return renderExplore();
    }
    const { reqToken, signal } = beginViewRequest();
    state.searchFetchingQuery = query;
    state.searchLoadingUi = true;
    setMainLoading(`Searching for "${query}" across all sources...`);
    try {
      const d = await api(`/api/web/search?q=${encodeURIComponent(query)}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      state.searchFetchingQuery = '';
      state.searchLoadingUi = false;

      const tracks = cacheTracks(d.tracks || []);
      const albums = d.albums || [];
      const artists = d.artists || [];
      const instantMix = d.instantMix
        ? {
            ...d.instantMix,
            tracks: cacheTracks(d.instantMix.tracks || []),
          }
        : null;
      const refinements = d.refinements || null;

      if (!tracks.length && !albums.length && !artists.length && !instantMix) {
        state.searchState = null;
        $('main-view').innerHTML = `<p class="muted-center">No matches found for "${esc(query)}".</p>`;
        return;
      }

      state.searchState = {
        query,
        tracks,
        albums,
        artists,
        instantMix,
        refinements,
        activeFilter: 'all',
        mixExpanded: false,
      };
      renderSearchView();
      $('main-view').scrollTop = 0;
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
    return `<section class="instant-mix-card ${isProminent ? 'prominent' : ''}">
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

  function renderSearchView() {
    const s = state.searchState;
    if (!s) return;
    const { query, albums, artists, instantMix, refinements, activeFilter, mixExpanded } = s;
    const tracks = applySearchFilter(s.tracks, activeFilter);

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
          <span class="refine-label">Refine mix:</span>
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

    const mixIsProminent = !!(instantMix && (instantMix.prominent || instantMix.isThemeMix));
    if (instantMix && mixIsProminent && activeFilter === 'all') {
      html += renderInstantMixCard(instantMix, refinements, query, mixExpanded);
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

    if (instantMix && !mixIsProminent && activeFilter === 'all') {
      html += renderInstantMixCard(instantMix, refinements, query, mixExpanded);
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
      const d = await api(`/api/web/album/${encodeURIComponent(id)}`, { signal });
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
      const d = await api(`/api/web/artist/${encodeURIComponent(id)}`, { signal });
      if (!isViewRequestCurrent(reqToken, signal)) return;
      const art = d.artist;
      const top = cacheTracks(d.topTracks || []);
      const albums = d.albums || [];
      const related = d.related || [];
      const idsJson = esc(JSON.stringify(top.map((t) => t.id)));

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
          </div>
        </div>
      </div>`;

      if (top.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Popular</h2></div>
          <div class="track-table">${renderTrackRows(top.slice(0, 10), top)}</div>
        </section>`;
      }
      if (albums.length > 0) {
        html += `<section class="section">
          <div class="section-head"><h2>Discography</h2></div>
          <div class="card-grid">${albums.map(renderAlbumCard).join('')}</div>
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
          <div class="hero-sub">By <strong>${esc(pl.ownerName)}</strong>${pl.comment ? ` · ${esc(pl.comment)}` : ''} · ${tracks.length} songs · ${fmtTotalDur(pl.duration)}</div>
          <div class="hero-actions">
            ${tracks.length ? `<button class="btn-primary" onclick="App.playListByIds(${idsJson}, 0)">${ICONS.play} Play</button>` : ''}
            ${tracks.length > 1 ? `<button class="btn-secondary" onclick="App.playShuffledIds(${idsJson})">Shuffle</button>` : ''}
            ${tracks.length ? `<button class="btn-secondary" onclick="App.startPlaylistRadio(${esc(JSON.stringify(pl.id))})">Start Radio</button>` : ''}
            ${d.isOwner && !pl.isAuto ? `<button class="btn-secondary" onclick="App.promptRenamePlaylist(${esc(JSON.stringify(pl.id))}, ${esc(JSON.stringify(pl.name))})">Rename</button>` : ''}
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
      const idsJson = esc(JSON.stringify(tracks.map((t) => t.id)));
      let html = `<div class="hero-header" style="background:${esc(m.c2)};border-color:${esc(m.c1)}55;border-left:4px solid ${esc(m.c1)};">
        <div class="hero-info">
          <div class="hero-kicker" style="color:#fff">MOOD MIX</div>
          <h1 class="hero-title">${esc(m.label)}</h1>
          <div class="hero-sub" style="color:rgba(255,255,255,0.88)">${esc(m.subtitle)} · ${tracks.length} tracks</div>
          <div class="hero-actions">
            <button class="btn-primary" onclick="App.playListByIds(${idsJson}, 0)">${ICONS.play} Play Mix</button>
            <button class="btn-secondary" onclick="App.playShuffledIds(${idsJson})">Shuffle</button>
          </div>
        </div>
      </div>
      <div class="track-table">${renderTrackRows(tracks)}</div>`;
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
      const idsJson = esc(JSON.stringify(tracks.map((t) => t.id)));
      let html = `<div class="hero-header">
        <div class="hero-info">
          <div class="hero-kicker">GENRE</div>
          <h1 class="hero-title">${esc(d.genre)}</h1>
          <div class="hero-sub">${tracks.length} tracks ready to stream</div>
          <div class="hero-actions">
            ${tracks.length ? `<button class="btn-primary" onclick="App.playListByIds(${idsJson}, 0)">${ICONS.play} Play ${esc(d.genre)}</button>` : ''}
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

      let html = `<div class="section-head"><h1>Your Library</h1><button class="btn-primary" onclick="App.promptCreatePlaylist()">+ New Playlist</button></div>`;
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
    return `<div class="media-card" onclick="App.navigate('album/${encodeURIComponent(a.id)}')">
      <div class="card-art-wrap">
        <img src="${esc(a.coverUrl)}" loading="lazy" alt="" />
        <button class="card-play-fab" onclick="event.stopPropagation();App.playAlbumNow(${esc(JSON.stringify(a.id))})">${ICONS.play}</button>
      </div>
      <div class="card-title">${esc(a.name)}</div>
      <div class="card-sub">${esc(a.artist)}${a.year ? ` · ${a.year}` : ''}</div>
    </div>`;
  }

  function renderArtistCard(a) {
    return `<div class="media-card" onclick="App.navigate('artist/${encodeURIComponent(a.id)}')">
      <div class="card-art-wrap round">
        <img src="${esc(a.coverUrl)}" loading="lazy" alt="" />
      </div>
      <div class="card-title" style="text-align:center">${esc(a.name)}</div>
      <div class="card-sub" style="text-align:center">Artist</div>
    </div>`;
  }

  function renderPlaylistCard(p) {
    return `<div class="media-card" onclick="App.navigate('playlist/${encodeURIComponent(p.id)}')">
      <div class="card-art-wrap">
        <img src="${esc(p.coverUrl)}" loading="lazy" alt="" />
        <button class="card-play-fab" onclick="event.stopPropagation();App.playPlaylistNow(${esc(JSON.stringify(p.id))})">${ICONS.play}</button>
      </div>
      <div class="card-title">${esc(p.name)}</div>
      <div class="card-sub">${p.trackCount} tracks · ${esc(p.ownerName)}</div>
    </div>`;
  }

  function renderMoodTile(m) {
    return `<div class="mood-tile" style="background:${esc(m.c2)};border-color:${esc(m.c1)}55;border-left:4px solid ${esc(m.c1)}" onclick="App.navigate('mood/${encodeURIComponent(m.slug)}')">
      <h3>${esc(m.label)}</h3>
      <p>${esc(m.subtitle)}</p>
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

  async function playQueueAt(index) {
    if (index < 0 || index >= state.queue.length) return;
    state.queueIndex = index;
    state.pendingSeekSec = 0;
    const track = state.queue[index];
    state.lastScrobbleCompleteId = '';
    state.compatAttemptedId = '';

    ensureAudioGraph();
    audio.src = buildStreamUrl(track);
    audio.play().catch((e) => {
      updatePlayButton(false);
      console.warn('Playback prevented or error:', e);
    });

    updateNowPlayingUI(track, true);
    renderQueuePanel();
    saveServerQueue();
    loadLyricsForTrack(track.id);

    sendTrackEvent(track.id, 'play', 1);

    // Pre-warm next track so transitions feel instant!
    const next = state.queue[index + 1];
    if (next) {
      api(`/api/web/warm/${encodeURIComponent(next.id)}`, { method: 'POST' }).catch(() => {});
    }

    // Highlight playing track in current view if visible
    document.querySelectorAll('.track-row').forEach((row) => {
      row.classList.toggle('playing', row.dataset.tid === track.id);
    });
  }

  function updateNowPlayingUI(track, active) {
    if (!track) return;
    $('pb-cover').src = track.coverUrl;
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

    $('fs-cover').src = track.coverUrl;
    $('fs-title').textContent = track.title;
    $('fs-artist').innerHTML = `<span class="clickable" onclick="App.goCurrentArtist()">${esc(track.artist)}</span>${track.album ? ` · <span class="clickable" onclick="App.goCurrentAlbum()">${esc(track.album)}</span>` : ''}`;
    $('fs-bg').style.backgroundImage = `url('${track.coverUrl}')`;
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
    state.radioReqToken++;
    state.radioLoading = false;
    state.queue = tracks;
    playQueueAt(Math.max(0, Math.min(startIndex, tracks.length - 1)));
  }

  function playShuffledIds(ids) {
    const tracks = ids.map((id) => state.trackMap.get(id)).filter(Boolean);
    if (!tracks.length) return;
    for (let i = tracks.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [tracks[i], tracks[j]] = [tracks[j], tracks[i]];
    }
    state.radioReqToken++;
    state.radioLoading = false;
    state.queue = tracks;
    playQueueAt(0);
  }

  function playSingle(id) {
    const t = state.trackMap.get(id);
    if (!t) return;
    state.radioReqToken++;
    state.radioLoading = !!state.autoplay;
    state.queue = [t];
    playQueueAt(0);
    if (state.autoplay) {
      appendRadioTracks(t.id);
    }
  }

  async function playAlbumNow(id) {
    try {
      const d = await api(`/api/web/album/${encodeURIComponent(id)}`);
      const tracks = cacheTracks(d.tracks || []);
      if (tracks.length) {
        state.radioReqToken++;
        state.radioLoading = false;
        state.queue = tracks;
        playQueueAt(0);
      }
    } catch (e) {
      showToast(e.message);
    }
  }

  async function playPlaylistNow(id) {
    try {
      const d = await api(`/api/web/playlist/${encodeURIComponent(id)}`);
      const tracks = cacheTracks(d.tracks || []);
      if (tracks.length) {
        state.radioReqToken++;
        state.radioLoading = false;
        state.queue = tracks;
        playQueueAt(0);
      }
    } catch (e) {
      showToast(e.message);
    }
  }

  function playSearchTrack(id) {
    sendTrackEvent(id, 'search_pick', 1);
    playSingle(id);
  }

  function playInstantMix(shuffled = false, startIndex = 0) {
    const s = state.searchState;
    const mix = s && s.instantMix;
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
    const s = state.searchState;
    const mix = s && s.instantMix;
    if (!mix || !mix.tracks || !mix.tracks.length) return;
    const tracks = mix.tracks.map((t) => state.trackMap.get(t.id) || t);
    if (!state.queue.length) {
      state.queue = tracks;
      playQueueAt(0);
    } else {
      state.queue.push(...tracks);
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
    if (!state.searchState) return;
    state.searchState.mixExpanded = !state.searchState.mixExpanded;
    renderSearchView();
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

  async function appendRadioTracks(seedId) {
    const reqToken = ++state.radioReqToken;
    state.radioLoading = true;
    renderQueuePanel();
    try {
      const excludeIds = state.queue
        .slice(-80)
        .map((t) => t.id)
        .filter(Boolean)
        .join(',');
      const params = new URLSearchParams();
      if (seedId) params.set('seed', seedId);
      if (excludeIds) params.set('exclude', excludeIds);
      const d = await api(`/api/web/radio?${params.toString()}`);
      if (reqToken !== state.radioReqToken) return 0;
      const recs = cacheTracks(d.tracks || []);
      const existing = new Set(state.queue.map((t) => t.id));
      let added = 0;
      for (const r of recs) {
        if (!existing.has(r.id)) {
          state.queue.push(r);
          existing.add(r.id);
          added++;
        }
      }
      state.radioLoading = false;
      renderQueuePanel();
      if (added > 0) {
        saveServerQueue();
      }
      return added;
    } catch {
      if (reqToken === state.radioReqToken) {
        state.radioLoading = false;
        renderQueuePanel();
      }
      return 0;
    }
  }

  async function startPlaylistRadio(pid) {
    const reqToken = ++state.radioReqToken;
    state.radioLoading = true;
    showToast('Starting playlist radio...');
    try {
      const d = await api(`/api/web/radio?playlist=${encodeURIComponent(pid || '')}`);
      if (reqToken !== state.radioReqToken) return;
      state.radioLoading = false;
      const tracks = cacheTracks(d.tracks || []);
      if (tracks.length) {
        state.queue = tracks;
        playQueueAt(0);
      } else {
        showToast('No radio tracks found for this playlist yet');
      }
    } catch (e) {
      if (reqToken === state.radioReqToken) {
        state.radioLoading = false;
      }
      showToast(e.message);
    }
  }

  async function startRadioForId(seedId) {
    const seed = state.trackMap.get(seedId);
    const reqToken = ++state.radioReqToken;
    const cur = currentTrack();
    const keepingCurrent = !!(cur && cur.id === seedId);
    state.radioLoading = true;
    if (keepingCurrent) renderQueuePanel();
    showToast('Starting track radio...');
    try {
      const d = await api(`/api/web/radio?seed=${encodeURIComponent(seedId || '')}`);
      if (reqToken !== state.radioReqToken) return;
      state.radioLoading = false;
      const recs = cacheTracks(d.tracks || []);
      const filtered = recs.filter((r) => r.id !== seedId);
      if (keepingCurrent && currentTrack()?.id === seedId) {
        state.queue = [cur, ...filtered];
        state.queueIndex = 0;
        renderQueuePanel();
        saveServerQueue();
        if (filtered[0]) {
          api(`/api/web/warm/${encodeURIComponent(filtered[0].id)}`, { method: 'POST' }).catch(() => {});
        }
        return;
      }
      const queue = seed ? [seed, ...filtered] : filtered;
      if (queue.length) {
        state.queue = queue;
        playQueueAt(0);
      } else {
        renderQueuePanel();
      }
    } catch (e) {
      if (reqToken === state.radioReqToken) {
        state.radioLoading = false;
        renderQueuePanel();
      }
      showToast(e.message);
    }
  }

  function togglePlay() {
    const cur = currentTrack();
    if (!cur) return;
    ensureAudioGraph();
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
    if (audio.currentTime > 4 || (state.queueIndex === 0 && state.repeat !== 'all')) {
      audio.currentTime = 0;
      if (audio.paused) audio.play().catch(() => {});
      return;
    }
    const prev = state.queueIndex > 0 ? state.queueIndex - 1 : state.queue.length - 1;
    playQueueAt(prev);
  }

  async function nextTrack(manual = false) {
    const cur = currentTrack();
    if (manual && cur && audio.currentTime < 25 && state.lastScrobbleCompleteId !== cur.id) {
      sendTrackEvent(cur.id, 'skip', 1);
    }
    if (!state.queue.length) return;

    if (state.shuffle && state.queue.length > 1) {
      let nextIdx = Math.floor(Math.random() * (state.queue.length - 1));
      if (nextIdx >= state.queueIndex) nextIdx += 1;
      return playQueueAt(nextIdx);
    }
    if (state.queueIndex + 1 < state.queue.length) {
      const next = state.queueIndex + 1;
      if (state.autoplay && !state.radioLoading && state.queue.length - next <= 2 && cur) {
        appendRadioTracks(cur.id);
      }
      return playQueueAt(next);
    }
    if (state.repeat === 'all') {
      return playQueueAt(0);
    }
    if (state.autoplay && cur) {
      const added = await appendRadioTracks(cur.id);
      if (added > 0 && state.queueIndex + 1 < state.queue.length) {
        return playQueueAt(state.queueIndex + 1);
      }
    }
    updatePlayButton(false);
  }

  function wireAudioEvents() {
    audio.addEventListener('play', () => updatePlayButton(true));
    audio.addEventListener('pause', () => {
      updatePlayButton(false);
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
      const cur = currentTrack();
      if (!cur || !audio.error || !audio.getAttribute('src')) return;
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
          updatePlayButton(false);
        });
        return;
      }
      updatePlayButton(false);
      showToast(`Unable to play "${cur.title}" right now`);
    });
    audio.addEventListener('ended', () => {
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
          if (state.queue.length) saveServerQueue();
        }
      });
    };
    wireSeekSlider($('pb-seek'));
    wireSeekSlider($('fs-seek'));

    $('pb-volume').addEventListener('input', (e) => {
      state.volume = parseFloat(e.target.value);
      state.muted = state.volume === 0;
      audio.muted = state.muted;
      audio.volume = state.volume;
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
    const upNextHtml = upcoming.length
      ? `<div class="queue-section-label">Next Up (${upcoming.length})</div>` +
        upcoming.map((t, i) => renderQItem(t, state.queueIndex + 1 + i, false)).join('')
      : state.radioLoading
        ? `<div class="queue-section-label">Next Up</div><p class="muted-center">Loading track radio...</p>`
        : '';

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
    $('lyrics-container').innerHTML = `<p class="muted-center">Fetching lyrics...</p>`;
    $('fs-lyrics-body').innerHTML = `<p class="muted-center">Fetching lyrics...</p>`;
    state.lyrics = { synced: false, plain: '', lines: [] };
    state.activeLyricIdx = -1;
    try {
      const data = await api(`/api/web/lyrics/${encodeURIComponent(id)}`);
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
    } catch {
      $('lyrics-container').innerHTML = `<p class="muted-center">No lyrics available.</p>`;
      $('fs-lyrics-body').innerHTML = `<p class="muted-center">No lyrics available.</p>`;
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
    if (!t) return;
    const next = !t.isStarred;
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
      return { tid, plId, plPos: Number.isFinite(plPos) ? plPos : -1 };
    };

    document.addEventListener('contextmenu', (e) => {
      const info = getRowArgs(e.target);
      if (!info) return;
      e.preventDefault();
      if (Date.now() - longPressFiredAt < 700) return;
      openTrackMenu(e, info.tid, info.plId, info.plPos);
    });

    document.addEventListener('pointerdown', (e) => {
      if (e.pointerType !== 'touch' && e.pointerType !== 'pen') return;
      if (e.target.closest('button, a, input, select, textarea, .tr-actions')) return;
      const info = getRowArgs(e.target);
      if (!info) return;
      clearHold();
      startX = e.clientX;
      startY = e.clientY;
      holdTimer = setTimeout(() => {
        holdTimer = null;
        longPressFiredAt = Date.now();
        suppressNextClick = true;
        if (navigator.vibrate) {
          try {
            navigator.vibrate(15);
          } catch {}
        }
        openTrackMenu({ clientX: startX, clientY: startY }, info.tid, info.plId, info.plPos);
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
          }
        }
      },
      true
    );
  }

  async function markNotInterested(trackId) {
    if (!trackId) return;
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
      } else {
        document.querySelectorAll('[data-tid]').forEach((el) => {
          if (el.dataset.tid === trackId && !el.classList.contains('q-item')) {
            el.remove();
          }
        });
      }

      const cur = currentTrack();
      const wasCur = !!(cur && cur.id === trackId);
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

  function openModal(html, extraClass = '') {
    const box = $('modal-box');
    box.className = extraClass ? `modal-box ${extraClass}` : 'modal-box';
    box.innerHTML = html;
    $('modal-backdrop').classList.remove('hidden');
  }

  function closeModal(event) {
    if (!event || event.target === $('modal-backdrop')) {
      $('modal-backdrop').classList.add('hidden');
    }
  }

  function openEqModal() {
    ensureAudioGraph();
    openModal(`
      <div class="modal-head">
        <h3>10-Band Audio Equalizer</h3>
        <button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button>
      </div>
      <div style="display:flex;justify-content:space-between;align-items:center;gap:10px;flex-wrap:wrap">
        <label class="toggle-row">
          <input type="checkbox" ${state.eqEnabled ? 'checked' : ''} onchange="App.toggleEq(this.checked)" />
          <span><strong>Enable Equalizer</strong></span>
        </label>
        <div style="display:flex;gap:6px;flex-wrap:wrap">
          ${Object.keys(EQ_PRESETS)
            .map((name) => `<button class="mini-btn" onclick="App.applyEqPreset('${name}')">${name}</button>`)
            .join('')}
        </div>
      </div>
      <div class="eq-bands">
        ${EQ_LABELS.map(
          (lbl, i) => `
          <div class="eq-col">
            <span id="eq-val-${i}">${state.eqGains[i] > 0 ? '+' : ''}${state.eqGains[i]}dB</span>
            <input type="range" min="-12" max="12" step="1" value="${state.eqGains[i]}" oninput="App.setEqBand(${i}, this.value)" />
            <span>${lbl}</span>
          </div>`
        ).join('')}
      </div>
    `);
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
      const eqActive = state.eqEnabled || state.eqGains.some((v) => v !== 0);
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
              <div class="settings-row-label">10-Band Audio Equalizer</div>
              <div class="settings-row-desc">${eqActive ? 'Custom EQ filter active' : 'Flat response (off)'}</div>
            </div>
            <button class="btn-secondary" onclick="App.openEqModal()">Configure EQ</button>
          </div>
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Music Sources &amp; Engine</div>
          <div class="settings-metrics">
            <div class="settings-metric"><span>Catalog</span><strong>${srcData.totalTracks} tracks</strong></div>
            <div class="settings-metric"><span>Audio Cache</span><strong>${srcData.cacheMb} MB</strong></div>
            <div class="settings-metric"><span>FFmpeg</span><strong>${srcData.ffmpeg ? 'Ready' : 'Off'}</strong></div>
          </div>
          <div class="settings-source-list">
            ${(srcData.sources || [])
              .map(
                (s) => `
              <div class="settings-source-item">
                <div class="settings-source-head">
                  <span class="status-dot ${s.available ? 'on' : 'off'}"></span>
                  <strong>${esc(s.name)}</strong>
                </div>
                <div class="settings-source-status">${esc(s.status)}</div>
              </div>`
              )
              .join('') || '<p class="settings-empty">Loading source status...</p>'}
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
          <div class="kv-card">
            <div class="kv-label">OPENSUBSONIC API KEY</div>
            <div class="kv-row">
              <span id="modal-apikey" class="kv-val">${esc(state.user.apiKey || '')}</span>
              <div style="display:flex;gap:6px;flex-shrink:0">
                <button class="mini-btn" onclick="App.copyText(document.getElementById('modal-apikey').textContent, 'API key copied')">Copy</button>
                <button class="mini-btn" onclick="App.regenApiKey()">Regenerate</button>
              </div>
            </div>
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

    const initial = (state.user.username[0] || 'B').toUpperCase();
    body.innerHTML = `
      <div class="settings-section">
        <div class="settings-section-title">Account Profile</div>
        <div class="settings-profile-card">
          <span class="avatar-dot large">${esc(initial)}</span>
          <div>
            <div class="settings-profile-name">${esc(state.user.username)}</div>
            <div class="settings-row-desc">Web, OpenSubsonic &amp; Navidrome account</div>
          </div>
        </div>
      </div>
      <div class="settings-section">
        <div class="settings-section-title">Security</div>
        <form onsubmit="App.submitPasswordChange(event)" style="display:flex;flex-direction:column;gap:8px">
          <div class="kv-label">CHANGE PASSWORD</div>
          <input id="pw-cur" type="password" autocomplete="current-password" placeholder="Current password" required />
          <input id="pw-new" type="password" autocomplete="new-password" placeholder="New password" required />
          <div style="display:flex;gap:8px;flex-wrap:wrap;justify-content:space-between;align-items:center;margin-top:6px">
            <button type="submit" class="btn-secondary">Update Password</button>
            <button type="button" class="btn-secondary danger-btn" onclick="App.signOut()">Sign out</button>
          </div>
        </form>
      </div>
    `;
  }

  async function openSettingsModal(initialTab = 'general') {
    state.settingsTab = initialTab || 'general';
    state.settingsData.confirmResetHidden = false;
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
            <button class="settings-nav-btn" data-stab="apps" onclick="App.setSettingsTab('apps')">Apps &amp; API</button>
            <button class="settings-nav-btn" data-stab="account" onclick="App.setSettingsTab('account')">Account</button>
          </nav>
          <div id="settings-body" class="settings-body"></div>
        </div>
      </div>
    `, 'settings-modal');
    renderSettingsModalBody();

    try {
      const [srcRes, hidRes] = await Promise.all([
        api('/api/web/sources').catch(() => null),
        api('/api/web/hidden').catch(() => null),
      ]);
      if (srcRes) state.settingsData.sources = srcRes;
      if (hidRes) state.settingsData.hidden = hidRes;
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
        e.preventDefault();
        togglePlay();
      } else if (e.key === '/') {
        e.preventDefault();
        setExpandedPlayer(false);
        $('global-search').focus();
      } else if (e.key === 'ArrowRight' && e.shiftKey) {
        nextTrack(true);
      } else if (e.key === 'ArrowLeft' && e.shiftKey) {
        prevTrack();
      } else if (e.key === 'ArrowRight') {
        audio.currentTime = Math.min(audio.duration || 9999, (audio.currentTime || 0) + 5);
      } else if (e.key === 'ArrowLeft') {
        audio.currentTime = Math.max(0, (audio.currentTime || 0) - 5);
      } else if (e.key.toLowerCase() === 'm') {
        App.toggleMute();
      } else if (e.key.toLowerCase() === 'l') {
        App.toggleCurrentLike();
      }
    });
  }

  // Expose public handlers for template onclick attributes
  window.App = {
    navigate,
    focusSearch: () => {
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
    setLibTab: (tab) => renderLibrary(tab),
    setRightTab: (tab) => {
      $('tab-queue').classList.toggle('active', tab === 'queue');
      $('tab-lyrics').classList.toggle('active', tab === 'lyrics');
      $('rp-queue-body').classList.toggle('hidden', tab !== 'queue');
      $('rp-lyrics-body').classList.toggle('hidden', tab !== 'lyrics');
    },
    setFsMobileView,
    setFsTab: (tab) => {
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
      state.autoplay = !!v;
      $('autoplay-toggle').checked = state.autoplay;
      if ($('fs-autoplay-toggle')) $('fs-autoplay-toggle').checked = state.autoplay;
      localStorage.setItem('beatlab_autoplay', state.autoplay ? 'true' : 'false');
    },
    clearQueue: () => {
      const cur = currentTrack();
      state.queue = cur ? [cur] : [];
      state.queueIndex = cur ? 0 : -1;
      renderQueuePanel();
      saveServerQueue();
    },
    removeFromQueue: (idx) => {
      if (idx > state.queueIndex && idx < state.queue.length) {
        state.queue.splice(idx, 1);
        renderQueuePanel();
        saveServerQueue();
      }
    },
    startRadioFromCurrent: () => {
      const cur = currentTrack();
      startRadioForId(cur ? cur.id : '');
    },
    startRadioForId,
    playListByIds,
    playShuffledIds,
    playSingle,
    playQueueAt,
    playAlbumNow,
    playPlaylistNow,
    playNext: (tid) => {
      const t = state.trackMap.get(tid);
      if (!t) return;
      if (state.queue.length === 0) return playSingle(tid);
      state.queue.splice(state.queueIndex + 1, 0, t);
      renderQueuePanel();
      saveServerQueue();
      showToast('Will play next');
    },
    addToQueue: (tid) => {
      const t = state.trackMap.get(tid);
      if (!t) return;
      if (state.queue.length === 0) return playSingle(tid);
      state.queue.push(t);
      renderQueuePanel();
      saveServerQueue();
      showToast('Added to queue');
    },
    togglePlay,
    prevTrack,
    nextTrack,
    toggleShuffle: () => {
      state.shuffle = !state.shuffle;
      $('btn-shuffle').classList.toggle('active', state.shuffle);
      if ($('fs-shuffle')) $('fs-shuffle').classList.toggle('active', state.shuffle);
    },
    cycleRepeat: () => {
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
      audio.muted = state.muted;
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
        audio.currentTime = ms / 1000;
        if (audio.paused) audio.play().catch(() => {});
      }
    },
    openTrackMenu,
    closeTrackMenu,
    markNotInterested,
    openSettingsModal,
    setSettingsTab: (tab) => {
      state.settingsTab = tab;
      state.settingsData.confirmResetHidden = false;
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
    copyText: async (text, label = 'Copied') => {
      try {
        if (navigator.clipboard && navigator.clipboard.writeText) {
          await navigator.clipboard.writeText(text);
          showToast(label);
          return;
        }
      } catch {}
      showToast('Select and copy manually');
    },
    toggleSaveAlbum: async (id, saved) => {
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
          body: JSON.stringify({ name, comment, trackIds: tid ? [tid] : [] }),
        });
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
    runRefinedSearch,
    playSearchTrack,
    playInstantMix,
    playInstantMixAt,
    queueInstantMix,
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
    promptRenamePlaylist: (pid, curName) => {
      openModal(`
        <div class="modal-head"><h3>Rename Playlist</h3><button class="icon-btn" title="Close" onclick="App.closeModal()">${ICONS.close}</button></div>
        <form onsubmit="App.submitRenamePlaylist(event, ${esc(JSON.stringify(pid))})" style="display:flex;flex-direction:column;gap:12px">
          <input id="rename-pl-input" type="text" value="${esc(curName)}" required />
          <button type="submit" class="btn-primary">Save</button>
        </form>
      `);
    },
    submitRenamePlaylist: async (e, pid) => {
      e.preventDefault();
      const name = $('rename-pl-input').value.trim();
      if (!name) {
        showToast('Please enter a playlist name');
        return;
      }
      try {
        await api(`/api/web/playlists/${encodeURIComponent(pid)}`, {
          method: 'PATCH',
          body: JSON.stringify({ name }),
        });
        closeModal();
        await refreshSidebarPlaylists();
        renderPlaylist(pid);
      } catch (err) {
        showToast(err.message);
      }
    },
    deletePlaylist: async (pid) => {
      try {
        await api(`/api/web/playlists/${encodeURIComponent(pid)}`, { method: 'DELETE' });
        await refreshSidebarPlaylists();
        showToast('Playlist deleted');
        navigate('library');
      } catch (err) {
        showToast(err.message);
      }
    },
    openEqModal,
    closeModal,
    installPwa,
    toggleEq: (on) => {
      state.eqEnabled = on;
      applyEqGains();
    },
    setEqBand: (idx, val) => {
      const n = parseInt(val, 10) || 0;
      state.eqGains[idx] = n;
      const lbl = $(`eq-val-${idx}`);
      if (lbl) lbl.textContent = `${n > 0 ? '+' : ''}${n}dB`;
      applyEqGains();
    },
    applyEqPreset: (name) => {
      const vals = EQ_PRESETS[name];
      if (!vals) return;
      state.eqGains = [...vals];
      state.eqEnabled = name !== 'Flat';
      applyEqGains();
      openEqModal();
    },
    regenApiKey: async () => {
      try {
        const res = await api('/api/web/account/apikey', { method: 'POST' });
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
