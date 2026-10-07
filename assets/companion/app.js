/*
 * The openOMSI companion on a phone or tablet: Omsi-Hub's phone (telefoon.tsx and its
 * device page) and the game's navigator, talking to the game's own little server
 * (crates/omsi-app/src/companion).
 *
 * The order is the whole point, as at a depot: pair with the code the game shows (or scan
 * its QR code), sign on with your personnel number and code, sign the duty order (or pick a
 * duty in the duty menu), see what goes into the IBIS - and only then the navigator: the
 * map with the route, the next turn and stop, the duty board, the sheet, the break, the
 * bus's screens. Until then each step has the whole screen.
 *
 * Nothing is decided here. The state is the game's (the navigator in the game sees the same
 * driver signed on), the number and code are checked by the game, the route, the turns and
 * the board are the game's navigator's, and a tap on a screen only says where on it it went.
 *
 * A tablet held across shows the map with a column beside it (as the game's city map); a
 * phone shows one thing at a time, with a dock to change. The size of everything is this
 * device's own choice (the Device tab), and so are the stop signs, if it wants others.
 */
'use strict';

(function () {
  var STORE = 'openomsi-companion-key';
  var PREFS = 'openomsi-companion-prefs';
  var appEl = document.getElementById('app');
  var screenEl = document.getElementById('screen');
  var pillEl = document.getElementById('pill');
  var modalEl = document.getElementById('modal');

  var key = readKey();
  var prefs = readPrefs();
  var texts = {};
  /* The pairing code's digits: nine through the Cloudflare tunnel (the game says, with the
     texts), else six. */
  var pairLen = 6;
  var textsLang = null;
  var state = null;
  var version = 0;
  var lastSignature = '';
  var queue = Promise.resolve();

  /* What the page itself keeps: where you are in it, and what you are typing. */
  var ui = {
    tab: 'nav',
    menu: false,
    lines: null,
    line: null,
    tours: null,
    note: '',
    busy: false,
    signOn: { step: 'number', number: '', typed: '', wrong: false, busy: false },
    pair: { typed: '', error: '', busy: false },
    screen: null,
    qr: null,
    /* until when the QR code is uncovered (streaming) */
    qrShown: 0,
    sheetAt: null,
    /* the Company tab: what came last (and when), an order on its way, what the last one did */
    company: null,
    companyAt: 0,
    companyBusy: false,
    companySending: false,
    companyNote: null
  };

  /* A pairing code in the address (the game's QR code): paired with at once. */
  var pairParam = (function () {
    try {
      var u = new URL(window.location.href);
      var c = u.searchParams.get('pair');
      if (c === null) return null;
      // (out of the address: it is used once, and a page put on the home screen would keep it)
      u.searchParams.delete('pair');
      window.history.replaceState(null, '', u.pathname + u.search + u.hash);
      return /^\d{1,12}$/.test(c) ? c : null;
    } catch (e) {
      return null;
    }
  })();

  // ------------------------------------------------------------------ small helpers

  function readKey() {
    try {
      return localStorage.getItem(STORE) || '';
    } catch (e) {
      return '';
    }
  }

  function writeKey(v) {
    try {
      if (v) localStorage.setItem(STORE, v);
      else localStorage.removeItem(STORE);
    } catch (e) {
      /* a private window: paired until the page closes */
    }
  }

  /* This device's own choices: the size, the stop signs, the map flat or tilted, the board
     open, the duties whose IBIS codes were seen, the last trip report seen. */
  function readPrefs() {
    var p = {};
    try {
      p = JSON.parse(localStorage.getItem(PREFS) || '{}') || {};
    } catch (e) {
      p = {};
    }
    return {
      size: clampSize(Number(p.size) || 1),
      stops: ['game', 'de', 'uk', 'fr'].indexOf(p.stops) >= 0 ? p.stops : 'game',
      flat: !!p.flat,
      board: p.board !== false,
      ibis: p.ibis && typeof p.ibis === 'object' ? p.ibis : {},
      report: typeof p.report === 'string' ? p.report : ''
    };
  }

  function savePrefs() {
    try {
      localStorage.setItem(PREFS, JSON.stringify(prefs));
    } catch (e) {
      /* kept until the page closes */
    }
  }

  function clampSize(s) {
    return Math.min(1.6, Math.max(0.8, Math.round(s * 10) / 10));
  }

  function applySize() {
    document.documentElement.style.fontSize = 16 * prefs.size + 'px';
    if (mapView) mapView.measure();
  }

  function t(k, vars) {
    var s = texts[k] || k;
    if (vars) {
      Object.keys(vars).forEach(function (n) {
        s = s.split('%{' + n + '}').join(String(vars[n]));
      });
    }
    return s;
  }

  /* An element: h('div', { class: 'x', onclick: f }, child, [children], 'text'). */
  function h(tag, attrs) {
    var el = document.createElement(tag);
    var a = attrs || {};
    Object.keys(a).forEach(function (n) {
      var v = a[n];
      if (v === undefined || v === null || v === false) return;
      if (n === 'class') el.className = v;
      else if (n === 'style') {
        Object.keys(v).forEach(function (p) {
          if (p.indexOf('--') === 0) el.style.setProperty(p, v[p]);
          else el.style[p] = v[p];
        });
      } else if (n.indexOf('on') === 0 && typeof v === 'function') el.addEventListener(n.slice(2), v);
      else if (v === true) el.setAttribute(n, '');
      else el.setAttribute(n, String(v));
    });
    for (var i = 2; i < arguments.length; i++) add(el, arguments[i]);
    return el;
  }

  function add(el, c) {
    if (c === null || c === undefined || c === false) return;
    if (Array.isArray(c)) c.forEach(function (x) { add(el, x); });
    else if (typeof c === 'string' || typeof c === 'number') el.appendChild(document.createTextNode(String(c)));
    else el.appendChild(c);
  }

  /* An icon: a few strokes in a 24 x 24 box (the game's navigator draws the same ones). */
  var ICONS = {
    turn_left: 'M17 20v-7a4 4 0 0 0-4-4H5M9 5 5 9l4 4',
    turn_right: 'M7 20v-7a4 4 0 0 1 4-4h8M15 5l4 4-4 4',
    turn_slight_left: 'M15 20v-7.5L7.5 5M7 11V5h6',
    turn_slight_right: 'M9 20v-7.5L16.5 5M17 11V5h-6',
    u_turn_left: 'M17 20V10a5 5 0 0 0-10 0v4M4 11l3 3 3-3',
    straight: 'M12 20V4M7 9l5-5 5 5',
    plus: 'M12 5v14M5 12h14',
    minus: 'M5 12h14',
    locate: 'M12 2v3M12 19v3M2 12h3M19 12h3M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10Z',
    map: 'M9 4 3 6v14l6-2 6 2 6-2V4l-6 2-6-2ZM9 4v14M15 6v14',
    duty: 'M8 4h8M6 7h12v13H6zM9 11h6M9 15h4',
    pause: 'M9 5v14M15 5v14',
    play: 'M8 5l11 7-11 7Z',
    screen: 'M4 5h16v11H4zM9 20h6M12 16v4',
    device: 'M8 3h8a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1ZM11 18h2',
    check: 'M5 12l5 5 9-10',
    chevron_down: 'M6 9l6 6 6-6',
    chevron_up: 'M6 15l6-6 6 6',
    chevron_left: 'M15 6l-6 6 6 6',
    bell: 'M6 16V11a6 6 0 0 1 12 0v5l2 2H4ZM10 20a2 2 0 0 0 4 0',
    refresh: 'M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6',
    logout: 'M15 4h4v16h-4M10 8l-4 4 4 4M6 12h10',
    flag: 'M5 21V4M5 4h11l-2 4 2 4H5',
    lock: 'M6 11h12v9H6zM8.5 11V8a3.5 3.5 0 0 1 7 0v3',
    eye: 'M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12ZM12 9.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5Z',
    company: 'M3 20V9l9-5 9 5v11M7 20v-7h10v7M7 16.5h10'
  };

  function icon(name, cls) {
    var ns = 'http://www.w3.org/2000/svg';
    var svg = document.createElementNS(ns, 'svg');
    svg.setAttribute('viewBox', '0 0 24 24');
    svg.setAttribute('aria-hidden', 'true');
    svg.setAttribute('class', 'icon' + (cls ? ' ' + cls : ''));
    var p = document.createElementNS(ns, 'path');
    p.setAttribute('d', ICONS[name] || '');
    svg.appendChild(p);
    return svg;
  }

  function pad(n) {
    return (n < 10 ? '0' : '') + n;
  }

  /* Seconds of the day as HH:MM (a night tour's times go on past 24:00). */
  function hhmm(sec) {
    if (sec === null || sec === undefined || !isFinite(sec)) return '--:--';
    var m = Math.floor(sec / 60);
    return pad(Math.floor(m / 60) % 24) + ':' + pad(((m % 60) + 60) % 60);
  }

  /* How far off the timetable, as the navigator writes it: +3:12, −2:30. */
  function offset(sec) {
    var s = Math.round(Math.abs(sec || 0));
    return (sec < 0 ? '−' : '+') + Math.floor(s / 60) + ':' + pad(s % 60);
  }

  /* Whole minutes, rounded up (a break with 20 s left is not over). */
  function minutesUp(s) {
    return Math.max(0, Math.ceil((s || 0) / 60));
  }

  function distance(m) {
    if (m === null || m === undefined) return '';
    return m >= 1000 ? (m / 1000).toFixed(1) + ' km' : Math.max(0, Math.round(m / 10) * 10) + ' m';
  }

  /* The day of the week in the page's language (0 = Monday). */
  function weekday(i) {
    try {
      // (1 January 2024 was a Monday)
      return new Intl.DateTimeFormat(document.documentElement.lang || 'en', { weekday: 'short' }).format(new Date(2024, 0, 1 + ((i % 7) + 7) % 7));
    } catch (e) {
      return '';
    }
  }

  function sleep(ms) {
    return new Promise(function (done) { setTimeout(done, ms); });
  }

  function pill(text) {
    pillEl.textContent = text || '';
    pillEl.hidden = !text;
  }

  function plate(line) {
    return line ? h('span', { class: 'plate' }, line) : h('span', { class: 'plate empty' }, t('Empty run'));
  }

  function punctualityClass(p) {
    return p === 'late' ? 'late' : p === 'early' ? 'early' : 'ontime';
  }

  // ------------------------------------------------------------------ the game

  function call(path, body) {
    var opts = { method: body ? 'POST' : 'GET', headers: {}, cache: 'no-store' };
    if (key) opts.headers['X-Companion-Key'] = key;
    if (body) {
      opts.headers['Content-Type'] = 'application/json';
      opts.body = JSON.stringify(body);
    }
    return fetch(path, opts).then(function (r) {
      if (r.status === 403 && key && path.indexOf('api/pair') < 0) {
        return r.json().catch(function () { return {}; }).then(function (j) {
          if (j.error === 'not_paired') {
            unpair();
            throw new Error('not_paired');
          }
          return r;
        });
      }
      return r;
    });
  }

  /* A command for the game; its answer (or { error } when the game did not answer). */
  function command(body) {
    return call('api/do', body)
      .then(function (r) {
        return r.json().catch(function () { return {}; }).then(function (j) {
          if (r.status === 503) j.error = 'busy';
          else if (r.status !== 200 && !j.error) j.error = 'failed';
          return j;
        });
      })
      .catch(function () { return { error: 'failed' }; });
  }

  /* Commands one after the other: a key let go never overtakes its press. */
  function send(body) {
    queue = queue.then(function () { return command(body); });
    return queue;
  }

  function failed(j) {
    return j && j.error === 'busy' ? t('The game is busy. Try again in a moment.') : t('That did not work. Try again.');
  }

  function unpair() {
    key = '';
    writeKey('');
    state = null;
    version = 0;
    closeScreen();
    render(true);
    // (the address had a code in it: the device paired anew with that)
    if (pairParam) {
      var c = pairParam;
      pairParam = null;
      pair(c);
    }
  }

  function loadTexts(lang) {
    var q = lang ? '?lang=' + encodeURIComponent(lang) : '';
    return fetch('api/texts' + q, { cache: 'no-store' })
      .then(function (r) { return r.json(); })
      .then(function (j) {
        texts = j.texts || {};
        if (j.pair_len >= 4 && j.pair_len <= 12) pairLen = j.pair_len;
        textsLang = lang === undefined ? j.lang : lang;
        document.documentElement.lang = j.lang || 'en';
        render(true);
        if (mapView) mapView.update();
      })
      .catch(function () {});
  }

  /* The interface's accent as the game has it (`state.accent`): the page's colours and the
   * map's route. */
  var accentNow = '';
  function accent(a) {
    if (!a || !a.base || a.base === accentNow) return;
    accentNow = a.base;
    var r = document.documentElement.style;
    r.setProperty('--route', a.base);
    r.setProperty('--route-deep', a.deep);
    r.setProperty('--route-rgb', a.rgb);
    r.setProperty('--on-route', a.on);
    if (window.OmsiMap && window.OmsiMap.setAccent) window.OmsiMap.setAccent(a.base, a.casing, a.rgb);
  }

  /* The state, by long polling: the game answers as soon as something changed. */
  function poll() {
    if (!key) {
      return sleep(400).then(poll);
    }
    return call('api/state?after=' + version)
      .then(function (r) {
        if (r.status !== 200) throw new Error(String(r.status));
        return r.json();
      })
      .then(function (j) {
        version = j.v;
        state = j.state;
        accent(state && state.accent);
        pill('');
        if (state && state.lang !== undefined && state.lang !== textsLang) loadTexts(state.lang);
        if (ui.menu && state && state.stage === 'sign_on') ui.menu = false;
        if (state && state.stage === 'sign_on' && ui.lines !== null) ui.lines = null;
        /* another bus: its screens are other ones */
        if (shown && !(state && state.stage === 'on_duty' && (state.screens || []).some(function (x) { return x.id === shown.id; }))) closeScreen();
        /* (the device's form made again - the bus looked at once more: drawn anew) */
        if (shown && shown.form) {
          var now = (state.screens || []).filter(function (x) { return x.id === shown.id; })[0];
          if (now && now.form !== shown.form) openScreen(shown.id);
        }
        render(false);
        layoutScreen();
        report();
        if (mapView) mapView.update();
        if (ui.tab === 'more' && ui.qr && ui.qr.when && Date.now() - ui.qr.when > 20000) loadQr();
        if (ui.tab === 'company' && !ui.companyBusy && Date.now() - ui.companyAt > 10000) loadCompany();
      })
      .catch(function (e) {
        if (e && e.message === 'not_paired') return;
        pill(t('Connection lost. Trying again…'));
        return sleep(2000);
      })
      .then(poll);
  }

  // ------------------------------------------------------------------ drawing

  /* Whether the screen is wide enough for the map with a column beside it (a tablet held
     across, a laptop). */
  function wide() {
    var w = window.innerWidth, hh = window.innerHeight;
    return w >= 52 * 16 * prefs.size && w > hh * 1.05;
  }

  /*
   * Draw the page again, but only when what it shows changed: the state comes once a
   * second (the clock moves), and drawing a list anew threw away how far it was scrolled.
   * The map is drawn by itself, five times a second, and is kept from one drawing to the
   * next (see `mapView`).
   */
  function render(force) {
    var signature = JSON.stringify([key ? 1 : 0, ui.tab, ui.menu, ui.lines, ui.line, ui.tours, ui.note, ui.busy, ui.signOn, ui.pair, ui.qr, prefs, wide(), view(), ui.tab === 'company' ? [ui.company, ui.companySending, ui.companyNote] : null]);
    if (!force && signature === lastSignature) return;
    lastSignature = signature;
    var kept = {};
    appEl.querySelectorAll('[data-keep]').forEach(function (el) { kept[el.getAttribute('data-keep')] = el.scrollTop; });
    appEl.textContent = '';
    appEl.appendChild(draw());
    appEl.querySelectorAll('[data-keep]').forEach(function (el) {
      var top = kept[el.getAttribute('data-keep')];
      if (top) el.scrollTop = top;
    });
    followSheet();
    if (mapView) mapView.placed();
    startNav();
  }

  /* The part of the state the page shows now (not the clock, unless the break needs it). */
  function view() {
    if (!state) return null;
    var s = state;
    var d = s.duty;
    var v = { stage: s.stage, driver: s.driver, free: s.free, screens: s.screens, bus: s.bus, style: s.stop_style };
    if (d) v.duty = [d.line, d.tour, d.start, d.end, d.trips.length, d.trip, d.next_stop, d.board, d.ibis, d.focus];
    if (ui.tab === 'break' || ui.tab === 'duty') v.clock = [Math.floor((s.clock || 0) / 60), s.break_since];
    if (ui.tab === 'duty' && d) v.sheet = d.sheet;
    return v;
  }

  function draw() {
    if (!key) return pairing();
    if (!state) return h('div', { class: 'phone' }, h('p', { class: 'empty' }, t('Connecting…')));
    if (state.stage === 'sign_on') return signOn();
    if (ui.menu || state.stage === 'duty_menu') return dutyMenu();
    if (state.stage === 'duty_order') return dutyOrder();
    if (ibisDue()) return ibisPage();
    return navigator();
  }

  /* A row of boxes, one a digit: you see how far you are. The code is typed blind. */
  function boxes(n, typed, wrong, hide) {
    var out = [];
    for (var i = 0; i < n; i++) {
      out.push(h('span', { class: i < typed.length ? 'full' : '' }, i < typed.length ? (hide ? '•' : typed[i]) : ''));
    }
    return h('div', { class: 'boxes' + (wrong ? ' wrong' : '') }, out);
  }

  function keypad(onDigit, onClear, onBack) {
    var keys = ['1', '2', '3', '4', '5', '6', '7', '8', '9'].map(function (d) {
      return h('button', { type: 'button', onclick: function () { onDigit(d); } }, d);
    });
    keys.push(h('button', { type: 'button', class: 'quiet', onclick: onClear }, t('Clear')));
    keys.push(h('button', { type: 'button', onclick: function () { onDigit('0'); } }, '0'));
    keys.push(h('button', { type: 'button', class: 'quiet', 'aria-label': '←', onclick: onBack }, '←'));
    return h('div', { class: 'keypad' }, keys);
  }

  /* A step that has the whole screen (pairing, signing on, the duty order, the IBIS). */
  function stepPage(cls, children) {
    return h('div', { class: 'phone step' }, h('div', { class: 'page ' + (cls || '') }, h('div', { class: 'step-card' }, children)));
  }

  // ---- pairing

  function pairing() {
    var p = ui.pair;
    var digit = function (d) {
      if (p.busy || p.typed.length >= pairLen) return;
      p.typed += d;
      p.error = '';
      render(true);
      if (p.typed.length === pairLen) pair(p.typed);
    };
    return stepPage('', [
      h(
        'div',
        { class: 'center' },
        h('img', { class: 'logo', src: 'icon.svg', alt: '' }),
        h('p', { class: 'kicker' }, 'openOMSI'),
        h('h1', null, t('Connect to openOMSI')),
        h('p', { class: 'explain' }, p.busy ? t('Pairing…') : t('Enter the pairing code the game shows.')),
        h('p', { class: 'explain small' }, t("Or scan the QR code on the navigator's sign-on page: the camera pairs at once."))
      ),
      boxes(pairLen, p.typed, !!p.error, false),
      p.error ? h('p', { class: 'error' }, p.error) : null,
      keypad(digit, function () { p.typed = ''; render(true); }, function () { p.typed = p.typed.slice(0, -1); render(true); })
    ]);
  }

  function pair(code) {
    var p = ui.pair;
    p.busy = true;
    p.typed = code.length <= pairLen ? code : '';
    render(true);
    fetch('api/pair', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ code: code }), cache: 'no-store' })
      .then(function (r) {
        return r.json().catch(function () { return {}; }).then(function (j) {
          if (r.status === 200 && j.key) {
            key = j.key;
            writeKey(key);
            p.error = '';
          } else if (r.status === 429) {
            p.error = pairLen > 6 ? t('Too many tries. Wait a few minutes.') : t('Too many tries. Wait a minute.');
          } else {
            p.error = t('That code is not right. The game shows the current one.');
          }
        });
      })
      .catch(function () { p.error = t('That did not work. Try again.'); })
      .then(function () {
        p.busy = false;
        p.typed = '';
        render(true);
      });
  }

  // ---- signing on (Omsi-Hub's AanmeldPaneel)

  function signOn() {
    var s = ui.signOn;
    var d = state.driver || { number_len: 6, code_len: 4, name: '' };
    var length = s.step === 'number' ? d.number_len : d.code_len;
    var digit = function (x) {
      if (s.busy || s.typed.length >= length) return;
      s.typed += x;
      s.wrong = false;
      render(true);
      /* no key to confirm: at the right length it is checked (one hand on the wheel) */
      if (s.typed.length === length) check();
    };
    return stepPage('', [
      h('p', { class: 'kicker' }, t('Sign on')),
      d.name ? h('p', { class: 'driver' }, d.name) : null,
      h('p', { class: 'explain' }, s.step === 'number' ? t('Your personnel number') : t('Your code')),
      boxes(length, s.typed, s.wrong, s.step === 'code'),
      s.wrong ? h('p', { class: 'error' }, ui.note || t("Not known here. Your number and code are in the game's duty menu (Esc, Line and tour).")) : null,
      keypad(digit, function () { s.typed = ''; render(true); }, function () { s.typed = s.typed.slice(0, -1); render(true); })
    ]);
  }

  function check() {
    var s = ui.signOn;
    var typed = s.typed;
    s.busy = true;
    var body = s.step === 'number' ? { do: 'sign_on', number: typed } : { do: 'sign_on', number: s.number, code: typed };
    send(body).then(function (j) {
      s.busy = false;
      s.typed = '';
      ui.note = j.error ? failed(j) : '';
      if (j.result === 'number') {
        s.number = typed;
        s.step = 'code';
        s.wrong = false;
      } else if (j.result === 'signed_on') {
        s.step = 'number';
        s.number = '';
        s.wrong = false;
        ui.tab = 'nav';
      } else {
        /* the boxes empty and red, on the same step (as Omsi-Hub's keypad) */
        s.wrong = true;
      }
      render(true);
    });
  }

  // ---- the duty menu: the game's "Line and tour..."

  function personnelLine() {
    var d = state.driver;
    if (!d) return null;
    return h('div', { class: 'who' }, h('p', { class: 'driver' }, d.name), d.number ? h('p', { class: 'explain' }, t('Personnel no. %{number}', { number: d.number })) : null);
  }

  function openMenu() {
    ui.menu = true;
    ui.line = null;
    ui.tours = null;
    ui.lines = null;
    ui.note = '';
    render(true);
  }

  var loadingLines = false;

  function dutyMenu() {
    /* the lines are asked for when the menu first shows (after signing on, or opened) */
    if (ui.lines === null && !loadingLines) {
      loadingLines = true;
      send({ do: 'lines' }).then(function (j) {
        loadingLines = false;
        ui.lines = j.lines || [];
        if (j.error) ui.note = failed(j);
        render(true);
      });
    }
    var body;
    if (ui.line === null) {
      body = ui.lines === null
        ? h('p', { class: 'empty' }, t('Loading…'))
        : ui.lines.length === 0
          ? h('p', { class: 'empty' }, t('No timetable on this map'))
          : h('ul', { class: 'list' }, ui.lines.map(function (l) {
              return h('li', null, h('button', { type: 'button', class: 'row', onclick: function () { openLine(l); } },
                h('span', { class: 'plate' }, l.name), h('span', null, h('b', null, l.label)), h('span', { class: 'chev' }, '›')));
            }));
    } else {
      body = [
        h('button', { type: 'button', class: 'action', onclick: function () { ui.line = null; ui.tours = null; render(true); } }, '‹ ' + t('Back')),
        h('p', { class: 'explain' }, t('Tap the tour you drive: you take it on at once, and its codes go to the IBIS.')),
        ui.tours === null
          ? h('p', { class: 'empty' }, t('Loading…'))
          : h('ul', { class: 'list' }, ui.tours.map(function (tour) {
              return h('li', null, h('button', { type: 'button', class: 'row', disabled: ui.busy, onclick: function () { pick(tour); } },
                h('span', { class: 'time' }, tour.when || ''), h('span', null, h('b', null, tour.what || (t('Tour') + ' ' + tour.tour))), h('span', { class: 'chev' }, '›')));
            }))
      ];
    }
    var foot = [];
    if (state.stage === 'duty_menu') foot.push(h('button', { type: 'button', class: 'action', onclick: driveFree }, t('Drive without a duty')));
    if (ui.menu) foot.push(h('button', { type: 'button', class: 'action', onclick: function () { ui.menu = false; render(true); } }, t('Back')));
    /* (signing on not asked for in the game's settings: nothing to sign off from) */
    if (!state.auto_sign_on) foot.push(h('button', { type: 'button', class: 'action quiet', onclick: signOff }, t('Sign off')));
    return h(
      'div',
      { class: 'phone step' },
      h('div', { class: 'page', 'data-keep': 'menu' + (ui.line === null ? '' : ui.line.nr) },
        h('div', { class: 'step-card wide-card' },
          h('p', { class: 'kicker' }, t('Choose a duty')),
          personnelLine(),
          ui.line !== null ? h('div', null, h('span', { class: 'plate' }, ui.line.name)) : null,
          body,
          ui.note ? h('p', { class: 'error' }, ui.note) : null,
          foot))
    );
  }

  function openLine(l) {
    ui.line = l;
    ui.tours = null;
    ui.note = '';
    render(true);
    send({ do: 'tours', line: l.nr }).then(function (j) {
      ui.tours = j.tours || [];
      if (j.error) ui.note = failed(j);
      render(true);
    });
  }

  function pick(tour) {
    ui.busy = true;
    render(true);
    send({ do: 'pick', line: ui.line.nr, tour: tour.nr }).then(function (j) {
      ui.busy = false;
      if (j.ok) {
        ui.menu = false;
        ui.line = null;
        ui.tours = null;
        ui.lines = null;
        ui.note = '';
        ui.tab = 'nav';
      } else {
        ui.note = failed(j);
      }
      render(true);
    });
  }

  function driveFree() {
    send({ do: 'free' }).then(function (j) {
      if (!j.ok) ui.note = failed(j);
      ui.tab = 'nav';
      render(true);
    });
  }

  function signOff() {
    send({ do: 'sign_off' }).then(function () {
      ui.menu = false;
      render(true);
    });
  }

  // ---- the duty order (Omsi-Hub's DienstOpdracht)

  function dutyOrder() {
    var d = state.duty;
    var field = function (label, value) { return h('div', null, h('dt', null, label), h('dd', null, value)); };
    return stepPage('order', [
      h('p', { class: 'kicker' }, t('Duty assignment')),
      state.driver ? h('p', { class: 'driver' }, state.driver.name) : null,
      h('dl', null,
        field(t('Line'), d.lines || d.line || '—'),
        field(t('Tour'), d.tour || '—'),
        field(t('Departure'), hhmm(d.start)),
        field(t('Back at'), hhmm(d.end)),
        field(t('Trips'), String(d.trips.length))),
      h('p', { class: 'explain' }, t("The duty's codes go to the IBIS as soon as you accept it.")),
      ui.note ? h('p', { class: 'error' }, ui.note) : null,
      h('button', { type: 'button', class: 'action main', disabled: ui.busy, onclick: accept }, t('Accept duty')),
      h('button', { type: 'button', class: 'action', onclick: openMenu }, t('Choose another duty')),
      state.auto_sign_on ? null : h('button', { type: 'button', class: 'action quiet', onclick: signOff }, t('Sign off'))
    ]);
  }

  function accept() {
    ui.busy = true;
    render(true);
    send({ do: 'accept' }).then(function (j) {
      ui.busy = false;
      ui.note = j.ok ? '' : failed(j);
      if (j.ok) ui.tab = 'nav';
      render(true);
    });
  }

  // ---- the IBIS: what the driver types, once the duty is signed

  function dutyKey(d) {
    return [d.line, d.tour, d.start].join('|');
  }

  /* The IBIS codes have the whole screen once per duty, right after signing it (on its
     first trip: a device that comes later finds the duty under way). */
  function ibisDue() {
    var d = state && state.duty;
    if (!d || !d.ibis || state.stage !== 'on_duty') return false;
    var k = dutyKey(d);
    if (prefs.ibis[k]) return false;
    if (d.trip > 0) {
      prefs.ibis[k] = 1;
      savePrefs();
      return false;
    }
    return true;
  }

  function ibisSeen() {
    var d = state && state.duty;
    if (d) {
      // (the last few duties are enough to remember)
      var keys = Object.keys(prefs.ibis);
      if (keys.length > 20) keys.slice(0, keys.length - 20).forEach(function (k) { delete prefs.ibis[k]; });
      prefs.ibis[dutyKey(d)] = 1;
      savePrefs();
    }
    ui.tab = 'nav';
    render(true);
  }

  /* The codes, large: line, route and destination as the IBIS takes them, with what they
     mean under them. Omsi-Hub wrote the route in full with its head dimmed: the IBIS wants
     the whole number, the eye wants its last two digits. */
  function ibisCard(ib, big) {
    var tile = function (label, value, sub, cls) {
      return h('div', { class: 'code' + (cls ? ' ' + cls : '') }, h('small', null, label), h('b', null, value || '—'), sub ? h('span', null, sub) : null);
    };
    var route = ib.route_full ? [h('span', { class: 'dim' }, ib.route_full.slice(0, ib.route_full.length - 2)), ib.route_full.slice(-2)] : null;
    var stateText = ib.state === 'typed' ? t('Typed into the IBIS by the game') : ib.state === 'typing' ? t('The game is typing it into the IBIS…') : t('Type these into the IBIS');
    var codes = ib.known
      ? h('div', { class: 'codes' },
          tile(t('Line'), ib.code_line + (ib.suffix ? '·' + ib.suffix : ''), String(Number(ib.code_line)) !== ib.line ? ib.line : null),
          tile(t('Route'), route, ib.route ? t('route %{route}', { route: ib.route }) : t('no route: by destination')),
          tile(t('Destination'), ib.dest, ib.dest_typed ? t('typed after the line') : t('comes with the route'), ib.dest_typed ? '' : 'soft'))
      : h('div', { class: 'codes' }, tile(t('Line'), ib.line || '—', null));
    return h('div', { class: 'ibis-card' + (big ? ' big' : '') },
      h('div', { class: 'ibis-head' },
        h('b', null, 'IBIS'),
        h('span', { class: 'pill-state ' + ib.state }, ib.state === 'typed' ? icon('check') : null, stateText)),
      codes,
      h('p', { class: 'ibis-to' }, h('span', null, t('to')), ' ', h('b', null, ib.dest_text || ib.terminus || '—')),
      h('div', { class: 'ibis-facts' },
        ib.tour ? h('span', null, t('Course'), ' ', h('b', null, ib.tour)) : null,
        ib.stop ? h('span', null, t('Stop no.'), ' ', h('b', null, String(ib.stop)), ib.stop_name ? ' · ' + ib.stop_name : '') : null,
        ib.depot ? h('span', null, t('Depot file'), ' ', h('b', null, ib.depot)) : null),
      ib.known ? null : h('p', { class: 'explain' }, t("This bus's depot file has no codes for this trip: set the destination by hand.")));
  }

  function ibisPage() {
    var d = state.duty;
    return stepPage('ibis-step', [
      h('p', { class: 'kicker' }, t('Into the IBIS')),
      h('h1', null, h('span', { class: 'plate' }, d.ibis.line || d.line || '—'), ' ', d.ibis.dest_text || d.ibis.terminus || ''),
      ibisCard(d.ibis, true),
      h('button', { type: 'button', class: 'action main', onclick: ibisSeen }, t('Start driving'))
    ]);
  }

  // ---- the navigator: the map, and the duty beside it or under it

  function navigator() {
    var tabs = [['duty', 'duty', t('Duty')], ['break', 'pause', t('Break')], ['screens', 'screen', t('Screens')], ['company', 'company', t('Company')], ['more', 'device', t('Device')]];
    var tab = function (id, ic, label) {
      var on = state.break_since !== null && state.break_since !== undefined;
      return h('button', { type: 'button', 'aria-pressed': ui.tab === id ? 'true' : 'false', onclick: function () { ui.tab = id; render(true); } },
        icon(ic), h('span', null, label), id === 'break' && on ? h('i') : null);
    };
    if (wide()) {
      if (ui.tab === 'nav') ui.tab = 'duty';
      return h('div', { class: 'shell wide' },
        h('aside', { class: 'side' },
          h('nav', { class: 'seg' }, tabs.map(function (x) { return tab(x[0], x[1], x[2]); })),
          h('div', { class: 'side-page', 'data-keep': 'side-' + ui.tab }, page(ui.tab))),
        mapPane());
    }
    var body = ui.tab === 'nav'
      ? h('div', { class: 'navpane' }, mapPane(), boardView())
      : h('div', { class: 'page', 'data-keep': ui.tab }, page(ui.tab));
    return h('div', { class: 'shell narrow' }, body,
      h('nav', { class: 'dock' }, [tab('nav', 'map', t('Map'))].concat(tabs.map(function (x) { return tab(x[0], x[1], x[2]); }))));
  }

  function page(id) {
    if (id === 'break') return breakApp();
    if (id === 'screens') return screensApp();
    if (id === 'company') return companyApp();
    if (id === 'more') return deviceApp();
    return dutyApp();
  }

  // ---- the company: the bus company the launcher has open (its money, today, the depot)

  function loadCompany() {
    ui.companyBusy = true;
    return call('api/company')
      .then(function (r) { return r.status === 200 ? r.json() : { failed: true }; })
      .catch(function () { return { failed: true }; })
      .then(function (j) {
        ui.companyBusy = false;
        ui.companyAt = Date.now();
        /* (a moment without the game keeps what was shown) */
        if (!(j && j.failed && ui.company && !ui.company.failed)) ui.company = j;
        if (ui.companyNote && Date.now() - ui.companyNote.when > 8000) ui.companyNote = null;
        render(false);
      });
  }

  function money(c) {
    if (c === null || c === undefined) return '—';
    try {
      return new Intl.NumberFormat(document.documentElement.lang || 'en', { style: 'currency', currency: 'EUR', maximumFractionDigits: 0 }).format(c / 100);
    } catch (e) {
      return '€' + Math.round(c / 100);
    }
  }

  function dayText(d) {
    var p = /^(\d{4})-(\d{2})-(\d{2})$/.exec(d || '');
    if (!p) return d || '';
    return new Intl.DateTimeFormat(document.documentElement.lang || 'en', { weekday: 'short', day: 'numeric', month: 'short' }).format(new Date(+p[1], +p[2] - 1, +p[3]));
  }

  function monthText(m) {
    var p = /^(\d{4})-(\d{2})$/.exec(m || '');
    if (!p) return m || '';
    return new Intl.DateTimeFormat(document.documentElement.lang || 'en', { month: 'short' }).format(new Date(+p[1], +p[2] - 1, 1));
  }

  /* An order for the company: checked by the game, carried out by the launcher. */
  function companyOrder(o) {
    ui.companySending = true;
    render(true);
    call('api/company', o)
      .then(function (r) { return r.json().catch(function () { return { error: 'bad_order' }; }); })
      .catch(function () { return { error: 'lost' }; })
      .then(function (j) {
        ui.companySending = false;
        var why = j && j.error;
        ui.companyNote = j && j.ok
          ? { text: t('Sent: the launcher carries it out in a moment.'), bad: false, when: Date.now() }
          : { text: why === 'bad_order' ? t('The order was not understood.') : why === 'lost' ? t('Connection lost. Trying again…') : t(String(why)), bad: true, when: Date.now() };
        return loadCompany();
      });
  }

  function companyAlert(a) {
    switch (a.kind) {
      case 'no_lines': return t('The company runs no line yet.');
      case 'no_buses': return t('The company has no bus yet.');
      case 'no_drivers': return t('The company has no driver yet.');
      case 'uncovered': return t('%{n} tours today have no bus or no driver.', { n: a.n });
      case 'low_cash': return t('Less cash than a month of wages.');
      case 'service_due': return t('%{n} buses are due for their service.', { n: a.n });
      case 'unhappy': return t('%{n} people are unhappy.', { n: a.n });
      case 'going_back': return t('Bus %{n} goes back on %{date}.', { n: a.number, date: dayText(a.until) });
    }
    return '';
  }

  function companyApp() {
    var co = ui.company;
    if (!co) {
      if (!ui.companyBusy) loadCompany();
      return [h('p', { class: 'empty' }, t('Loading…'))];
    }
    if (co.failed) return [h('p', { class: 'empty' }, t('The company cannot be read now.'))];
    if (co.none) return [h('p', { class: 'empty' }, t('No company is open in the launcher.'))];
    var sending = ui.companySending;
    var m = co.month || {};
    var waiting = (co.pending || []).length;
    var head = h('div', { class: 'co-head' },
      h('span', { class: 'co-mark', style: { background: co.colour || '#f28c28' } }, co.short || ''),
      h('div', null, h('h2', null, co.name), h('small', null, t('Company day') + ' · ' + dayText(co.date))));
    var note = ui.companyNote ? h('p', { class: 'co-note' + (ui.companyNote.bad ? ' bad' : '') }, ui.companyNote.text) : null;
    var tiles = h('div', { class: 'tiles three' },
      h('div', null, h('b', { class: co.cash < 0 ? 'co-bad' : '' }, money(co.cash)), h('span', null, t('Cash'))),
      h('div', null, h('b', { class: m.result < 0 ? 'co-bad' : 'co-good' }, money(m.result)), h('span', null, t('This month'))),
      h('div', null, h('b', null, String(co.reputation)), h('span', null, t('Reputation'))));
    var alerts = (co.alerts || []).map(companyAlert).filter(function (x) { return x; });
    var alertCard = alerts.length ? h('section', { class: 'card co-card co-alerts' }, alerts.map(function (a) { return h('p', null, icon('bell'), h('span', null, a)); })) : null;

    var td = co.today;
    var today = h('section', { class: 'card co-card' },
      h('h2', null, t('Today')),
      td ? h('p', { class: 'co-big' }, t('%{c} of %{n} tours covered', { c: td.covered, n: td.tours })) : h('p', { class: 'explain small' }, t('The timetable of the day cannot be read.')),
      td && td.open.length ? h('ul', { class: 'co-list' }, td.open.map(function (o) {
        return h('li', null, h('span', { class: 'plate' }, o.line), h('span', null, t('Tour') + ' ' + o.tour + ' · ' + o.from + '–' + o.to), h('em', { class: 'co-bad' }, o.why === 'bus' ? t('no bus') : t('no driver')));
      })) : null,
      (co.breakdowns || []).length ? h('ul', { class: 'co-list' }, co.breakdowns.map(function (b) {
        return h('li', null, h('b', null, b.number), h('span', null, t('in the workshop until %{date}', { date: dayText(b.until) })));
      })) : null);

    var months = co.months || [];
    var top = months.reduce(function (a, x) { return Math.max(a, x.income, x.expenses); }, 1);
    var bar = function (cls, v) { return h('i', { class: cls, style: { height: Math.max(2, Math.round(v / top * 100)) + '%' } }); };
    var money6 = h('section', { class: 'card co-card' },
      h('h2', null, t('Finances')),
      months.length ? h('div', { class: 'co-bars' }, months.map(function (x) {
        return h('div', { class: 'co-bar' },
          h('div', { class: 'co-pair' }, bar('in', x.income), bar('out', x.expenses)),
          h('b', { class: x.result < 0 ? 'co-bad' : 'co-good' }, money(x.result)),
          h('span', null, monthText(x.month)));
      })) : h('p', { class: 'explain small' }, t('No month closed yet.')),
      h('div', { class: 'co-legend' },
        h('span', null, h('i', { class: 'in' }), t('Income')),
        h('span', null, h('i', { class: 'out' }), t('Expenses')),
        co.debt ? h('span', null, t('Debt') + ' ' + money(co.debt)) : null));

    var fleet = co.fleet || {};
    var depot = h('section', { class: 'card co-card' },
      h('h2', null, t('Depot')),
      h('p', { class: 'explain small' }, t('%{b} buses on %{s} spaces · cleanliness %{c} % · upkeep %{u} a month', { b: fleet.buses, s: fleet.spaces, c: co.clean, u: money(co.upkeep) })),
      h('ul', { class: 'co-rows' }, (co.areas || []).map(function (a) {
        var right;
        if (a.building_until) right = h('em', { class: 'co-warn' }, t('ready %{date}', { date: dayText(a.building_until) }));
        else if (a.cost === null || a.cost === undefined) right = h('em', { class: 'co-good' }, t('complete'));
        else right = h('button', { type: 'button', class: 'action co-small', disabled: !a.can || !a.allowed || sending, onclick: function () { companyOrder({ do: 'build', area: a.key }); } }, money(a.cost));
        return h('li', null, h('div', null, h('b', null, t(a.label)), h('small', null, t('level %{n} of %{m}', { n: a.level, m: a.max }) + (a.days && !a.building_until ? ' · ' + t('%{d} days of work', { d: a.days }) : ''))), right);
      })),
      waiting ? h('p', { class: 'explain small' }, t('%{n} orders wait for the launcher.', { n: waiting })) : null);

    var shop = h('section', { class: 'card co-card' },
      h('h2', null, t('Workshop')),
      (co.jobs || []).length ? h('ul', { class: 'co-list' }, co.jobs.map(function (j) {
        return h('li', null, h('b', null, j.number), h('span', null, t(j.job)), h('em', null, j.started ? t('until %{date}', { date: dayText(j.until) }) : t('waiting for a bay')));
      })) : h('p', { class: 'explain small' }, t('No bus is in the workshop.')),
      (co.buses || []).length ? h('ul', { class: 'co-rows' }, co.buses.map(function (b) {
        return h('li', null,
          h('div', null, h('b', null, b.number + ' ' + b.name), h('small', { class: b.due ? 'co-warn' : '' }, b.due ? t('service due') : t('condition %{n} %', { n: b.condition }))),
          h('button', { type: 'button', class: 'action co-small', disabled: sending, onclick: function () { companyOrder({ do: 'job', bus: b.id, job: b.job }); } }, t(b.job_label)));
      })) : null);

    return [head, note, tiles, alertCard, today, money6, depot, shop,
      h('p', { class: 'explain small' }, t('The company is the launcher’s: what you order here is carried out there.'))];
  }

  // ---- the map and what lies over it (as the game's small navigator)

  var mapView = null;

  function mapPane() {
    if (!mapView) mapView = makeMapView();
    return mapView.root;
  }

  function makeMapView() {
    var map = window.OmsiMap({
      wantRoads: function (x, y, r, tol, done) {
        call('api/roads?x=' + x + '&y=' + y + '&r=' + r + '&tol=' + tol)
          .then(function (res) { return res.status === 200 ? res.json() : null; })
          .then(function (j) {
            if (j) done(j);
            else sleep(3000).then(function () { done(null); });
          })
          .catch(function () { sleep(3000).then(function () { done(null); }); });
      },
      onMode: function () { mv.update(); }
    });
    map.setFlat(prefs.flat);
    var top = {
      speed: h('b', null, '0'),
      limit: h('i', { class: 'limit', hidden: true }),
      mid: h('span', { class: 'mid' }),
      day: h('small', null),
      clock: h('b', null, '--:--')
    };
    var turn = { el: h('div', { class: 'turn', hidden: true }), icon: null, dist: h('b'), street: h('span') };
    turn.el.appendChild(h('i', { class: 'turn-icon' }));
    turn.el.appendChild(turn.dist);
    turn.el.appendChild(turn.street);
    var street = h('div', { class: 'street', hidden: true });
    var next = { name: h('b', { class: 'name' }), req: h('span', { class: 'req', hidden: true }, icon('bell'), 'STOP'), facts: h('span', { class: 'facts' }), chip: h('span', { class: 'chip', hidden: true }) };
    var follow = h('button', { type: 'button', class: 'mapbtn', 'aria-label': t('Follow the bus'), onclick: function () { map.follow(); } }, icon('locate'));
    var flat = h('button', { type: 'button', class: 'mapbtn txt', onclick: function () {
      prefs.flat = !prefs.flat;
      savePrefs();
      map.setFlat(prefs.flat);
      map.follow();
      mv.update();
    } }, prefs.flat ? '2D' : '3D');
    var header = h('header', { class: 'nav-top' },
      h('div', { class: 'speed' }, top.speed, h('small', null, 'km/h'), top.limit),
      top.mid,
      h('div', { class: 'when' }, top.day, top.clock));
    // (the street the bus is on stands just above the next stop, in the middle)
    var footer = h('footer', { class: 'nav-next' },
      street,
      h('div', { class: 'l1' }, next.req, next.name),
      h('div', { class: 'l2' }, next.facts, next.chip));
    var buttons = h('div', { class: 'mapbtns' },
      h('button', { type: 'button', class: 'mapbtn', 'aria-label': '+', onclick: function () { map.zoom(0.7); } }, icon('plus')),
      h('button', { type: 'button', class: 'mapbtn', 'aria-label': '−', onclick: function () { map.zoom(1 / 0.7); } }, icon('minus')),
      flat,
      follow);
    var root = h('section', { class: 'navmap' }, map.el, header, turn.el, buttons, footer);
    var nav = null;
    var shownTurn = '';

    var mv = {
      root: root,
      map: map,
      setNav: function (n) {
        nav = n;
        map.setNav(n);
        mv.update();
      },
      /* The words over the map, from the live picture and the state. */
      update: function () {
        var style = prefs.stops === 'game' ? (state && state.stop_style) || 'de' : prefs.stops;
        map.setStyle(style);
        follow.hidden = map.mode() === 'follow';
        flat.textContent = prefs.flat ? '2D' : '3D';
        flat.setAttribute('aria-label', prefs.flat ? t('Tilt the map') : t('Lay the map flat'));
        if (!nav) {
          top.mid.textContent = '';
          next.name.textContent = t('Waiting for the game…');
          next.facts.textContent = '';
          next.chip.hidden = true;
          return;
        }
        top.speed.textContent = String(Math.round(Math.abs(nav.v || 0)));
        top.limit.hidden = !nav.limit;
        top.limit.textContent = nav.limit ? String(nav.limit) : '';
        top.limit.className = 'limit' + (nav.limit >= 100 ? ' three' : '');
        var temps = 'EXT ' + nav.temps[0] + '°C · INT ' + nav.temps[1] + '°C';
        top.mid.textContent = nav.line ? temps + ' · ' + nav.line : temps;
        top.day.textContent = weekday(nav.wd);
        top.clock.textContent = hhmm(nav.t);
        // the next turn
        var tr = nav.turn;
        turn.el.hidden = !tr;
        if (tr) {
          var name = tr.dir === 2 ? 'u_turn_left' : tr.dir < 0 ? (tr.deg < 60 ? 'turn_slight_left' : 'turn_left') : (tr.deg < 60 ? 'turn_slight_right' : 'turn_right');
          if (name !== shownTurn) {
            shownTurn = name;
            var holder = turn.el.firstChild;
            holder.textContent = '';
            holder.appendChild(icon(name));
          }
          turn.dist.textContent = distance(tr.dist);
          turn.street.textContent = tr.street || '';
          turn.street.hidden = !tr.street;
        }
        street.hidden = !nav.street;
        street.textContent = nav.street || '';
        // the next stop: its distance, the time to it, its time; how the bus stands
        next.req.hidden = !nav.req;
        var nx = nav.next;
        // the player's own pins (set on the game's city map): a free drive's destination
        // where the next stop goes, a diversion's next via beside the stop
        var pins = nav.pins || [];
        var dest = !nav.diversion && pins.length && pins[pins.length - 1].via === 0 ? pins[pins.length - 1] : null;
        var minutes = function (s) { return s < 60 ? '<1 min' : t('%{minutes} min', { minutes: Math.round(s / 60) }); };
        var via = function (p) { return t('Via %{n}', { n: p.via }) + (p.dist !== null && p.dist !== undefined ? ' · ' + distance(p.dist) : ''); };
        var pinNote = !nav.pin_note ? '' : nav.pin_note.k === 'arrived' ? t('You have arrived') : nav.pin_note.k === 'reached' ? t('Via %{n} reached', { n: nav.pin_note.n }) : '';
        if (nx) {
          next.name.textContent = nx.last ? nx.name + ' · ' + t('Final stop') : nx.name;
          var parts = [];
          if (nav.diversion && pins.length) {
            parts.push(t('Diversion'));
            parts.push(via(pins[0]));
          } else if (nx.dist !== null && nx.dist !== undefined) {
            parts.push(distance(nx.dist));
            parts.push(minutes(nx.eta));
          }
          parts.push(hhmm(nx.arr));
          var line2 = parts.join('  ·  ');
          next.facts.className = nav.diversion ? 'facts warn' : 'facts';
          if (pinNote) {
            line2 = pinNote;
            next.facts.className = 'facts ontime';
          } else if (nav.note) {
            line2 = nav.note === 'recalculated' ? t('Route recalculated') : nav.note === 'rerouting' ? t('Recalculating route') : t('Off route');
            next.facts.className = 'facts ' + (nav.note === 'recalculated' ? 'ontime' : 'warn');
          } else if (nav.jam) {
            line2 += '  ·  ' + (nav.jam >= 60 ? t('Traffic jam') : t('Slow traffic')) + ' +' + Math.max(1, Math.round(nav.jam / 60)) + ' min';
            next.facts.className = 'facts ' + (nav.jam >= 60 ? 'late' : 'warn');
          }
          next.facts.textContent = line2;
        } else if (dest) {
          next.name.textContent = dest.name;
          var facts = [];
          if (dest.dist !== null && dest.dist !== undefined) {
            facts.push(distance(dest.dist));
            facts.push(minutes(dest.eta));
            facts.push(hhmm(nav.t + dest.eta));
          }
          if (pins.length > 1) facts.push(via(pins[0]));
          next.facts.className = pinNote ? 'facts ontime' : 'facts';
          next.facts.textContent = pinNote || (nav.arrived ? t('You have arrived') : facts.join('  ·  '));
        } else {
          next.name.textContent = nav.terminus || (state && state.duty ? '' : t('Driving without a duty'));
          next.facts.textContent = '';
        }
        next.chip.hidden = nav.punctuality === null || nav.punctuality === undefined || !nx;
        if (!next.chip.hidden) {
          next.chip.className = 'chip ' + punctualityClass(nav.punctuality);
          next.chip.textContent = nav.punctuality === 'on_time' ? t('on time') : offset(nav.delay);
        }
        mv.measure();
      },
      /* What lies over the map, for the map to keep the bus in the free part. */
      measure: function () {
        var r = root.getBoundingClientRect();
        if (!r.height) return;
        var top0 = header.getBoundingClientRect();
        var bottom0 = footer.getBoundingClientRect();
        map.setInsets(Math.max(0, top0.bottom - r.top), Math.max(0, r.bottom - bottom0.top), 0, 0, prefs.size);
        var box = function (el) {
          var b = el.getBoundingClientRect();
          return b.width ? [b.left - r.left - 4, b.top - r.top - 4, b.width + 8, b.height + 8] : null;
        };
        map.setAvoid([box(turn.el), box(buttons), box(street)].filter(Boolean));
      },
      placed: function () {
        if (root.isConnected) {
          map.resize();
          mv.measure();
        }
      }
    };
    return mv;
  }

  // the live picture, by long polling while the map shows; the trip when it changed
  var navOn = false;
  var navVersion = 0;
  var tripVersion = -1;
  var tripLoading = false;

  function mapShown() {
    return !!(mapView && mapView.root.isConnected && !document.hidden);
  }

  function startNav() {
    if (navOn || !key || !mapShown()) return;
    navOn = true;
    navLoop();
  }

  function navLoop() {
    if (!key || !mapShown()) {
      navOn = false;
      return;
    }
    call('api/nav?after=' + navVersion)
      .then(function (r) {
        if (r.status !== 200) throw new Error(String(r.status));
        return r.json();
      })
      .then(function (j) {
        navVersion = j.v;
        if (j.nav) {
          mapView.setNav(j.nav);
          if (j.nav.trip !== tripVersion) loadTrip();
        }
      })
      .catch(function () { return sleep(1500); })
      .then(navLoop);
  }

  function loadTrip() {
    if (tripLoading) return;
    tripLoading = true;
    call('api/trip')
      .then(function (r) { return r.status === 200 ? r.json() : null; })
      .then(function (j) {
        tripLoading = false;
        if (!j) return;
        tripVersion = j.v;
        mapView.map.setTrip(j);
      })
      .catch(function () {
        tripLoading = false;
      });
  }

  document.addEventListener('visibilitychange', startNav);

  // ---- the duty board under the map (the game's small navigator, Shift+N)

  function statusLabel(st) {
    if (!st) return '';
    if (st.k === 'departs') return t('leaves in %{m} min', { m: Math.max(1, minutesUp(st.s)) });
    if (st.k === 'running') return st.p === 'on_time' ? t('on time') : offset(st.s);
    if (st.k === 'break') return st.s >= 0 ? t('break, %{m} min left', { m: minutesUp(st.s) }) : t('%{m} min over', { m: Math.max(1, minutesUp(-st.s)) });
    return t('Duty finished');
  }

  function statusClass(st) {
    if (!st) return '';
    if (st.k === 'running') return punctualityClass(st.p);
    if (st.k === 'break' && st.s < 0) return 'late';
    if (st.k === 'finished') return 'ontime';
    return 'plain';
  }

  function chip(st) {
    return h('span', { class: 'chip fill ' + statusClass(st) }, statusLabel(st));
  }

  function stopRow(r, idx, rows) {
    var above = idx > 0 && rows[idx - 1].row === 'stop';
    var below = idx + 1 < rows.length && rows[idx + 1].row === 'stop';
    var exp = r.expected !== null && r.expected !== undefined
      ? h('span', { class: 'exp ' + punctualityClass(r.expected - r.planned > 180 ? 'late' : r.expected - r.planned < -120 ? 'early' : 'on_time') }, hhmm(r.expected))
      : null;
    return h('div', { class: 'srow ' + r.state + (above ? ' above' : '') + (below ? ' below' : '') },
      h('i', { class: 'pin' }),
      h('span', { class: 'sname' }, r.name),
      r.last ? h('span', { class: 'tag' }, t('terminus')) : null,
      exp,
      h('span', { class: 'stime' }, hhmm(r.planned)));
  }

  function boardRows(rows) {
    return rows.map(function (r, idx) {
      if (r.row === 'head') {
        var sub = t('trip %{k} of %{n}', { k: r.index, n: r.count }) + '  ·  ' + (r.status && r.status.k === 'break' ? t('leaves %{time}', { time: hhmm(r.time) }) : t('arrives %{time}', { time: hhmm(r.time) }));
        return h('div', { class: 'bhead' }, plate(r.line), h('div', { class: 'grow' }, h('b', null, r.terminus), h('small', null, sub)), chip(r.status));
      }
      if (r.row === 'stop') return stopRow(r, idx, rows);
      if (r.row === 'more') return h('p', { class: 'more' }, t('and %{n} more to %{terminus}', { n: r.count, terminus: r.terminus }));
      if (r.row === 'next') {
        var notes = [];
        if (r.pause > 0) notes.push(t('%{m} min break', { m: r.pause }));
        if (r.change) notes.push(t('Continue on line %{line}, tour %{tour}', { line: r.change[0], tour: r.change[1] }));
        return h('div', { class: 'advice' },
          h('div', { class: 'aline' }, h('b', null, t('Next %{time}', { time: hhmm(r.departure) })), plate(r.line), h('span', { class: 'grow' }, r.terminus)),
          notes.length ? h('small', null, notes.join('  ·  ')) : null);
      }
      if (r.row === 'change') return h('div', { class: 'advice' }, h('small', null, t('Continue on line %{line}, tour %{tour}', { line: r.line, tour: r.tour })));
      if (r.row === 'finished') return h('p', { class: 'done-row' }, icon('check'), t('Duty finished'));
      return h('p', { class: 'more' }, t('No duty running'));
    });
  }

  function boardView() {
    var d = state.duty;
    var rows = d && d.board ? d.board : [{ row: 'no_duty' }];
    var open = prefs.board;
    var shownRows = open ? rows : rows.filter(function (r) { return r.row === 'head' || r.row === 'no_duty'; });
    return h('div', { class: 'board' + (open ? '' : ' shut') },
      h('button', { type: 'button', class: 'handle', 'aria-label': open ? t('Fewer') : t('More'), onclick: function () { prefs.board = !prefs.board; savePrefs(); render(true); } }, icon(open ? 'chevron_down' : 'chevron_up')),
      boardRows(shownRows));
  }

  // ---- the duty: the IBIS codes and the sheet (the game's city map column)

  function dutyApp() {
    var d = state.duty;
    var foot = h('div', { class: 'foot' },
      h('button', { type: 'button', class: 'action', onclick: openMenu }, d ? t('Choose another duty') : t('Choose a duty')),
      state.auto_sign_on ? null : h('button', { type: 'button', class: 'action quiet', onclick: signOff }, icon('logout'), t('Sign off')));

    if (!d) {
      return [personnelLine(), h('p', { class: 'empty' }, state.free ? t('Driving without a duty') : t('No duty running.')), foot];
    }
    var rows = d.sheet || [];
    var head = rows.length ? sheetHead(d) : null;
    return [
      head,
      d.ibis ? ibisCard(d.ibis, false) : null,
      h('div', { class: 'sheet' }, sheetRows(rows)),
      foot
    ];
  }

  function sheetHead(d) {
    var head = (d.board || []).filter(function (r) { return r.row === 'head'; })[0];
    var st = head ? head.status : null;
    var span = (d.trips.length === 1 ? t('%{n} trip', { n: 1 }) : t('%{n} trips', { n: d.trips.length })) + '  ·  ' + hhmm(d.start) + ' – ' + hhmm(d.end);
    var delay = st && st.k === 'running' ? h('b', { class: punctualityClass(st.p) }, offset(st.s)) : h('b', { class: 'soft' }, '–');
    return h('div', { class: 'sheet-head' },
      h('div', { class: 'sh-title' }, h('div', null, h('h2', null, t('Duty')), h('small', null, span)), st ? chip(st) : null),
      h('div', { class: 'tiles three' },
        h('div', null, h('b', null, hhmm(state.clock)), h('span', null, t('In the game'))),
        h('div', null, delay, h('span', null, t('Delay'))),
        h('div', null, h('b', null, ((d.focus || 0) + 1) + ' / ' + d.trips.length), h('span', null, t('Trip')))));
  }

  function sheetRows(rows) {
    return rows.map(function (r, idx) {
      if (r.row === 'trip') {
        var stops = r.stops === 1 ? t('1 stop') : t('%{n} stops', { n: r.stops });
        return h('div', { class: 'trow ' + r.state, 'data-now': r.state === 'now' ? 'trip' : null },
          h('span', { class: 'ttime' }, hhmm(r.departure)),
          plate(r.line),
          h('span', { class: 'grow' }, h('b', null, r.terminus), h('small', null, stops)),
          h('span', { class: 'tarr' }, hhmm(r.arrival)));
      }
      if (r.row === 'stop') {
        var el = stopRow(r, idx, rows);
        if (r.state === 'now') el.setAttribute('data-now', 'stop');
        return el;
      }
      if (r.row === 'pause') return h('p', { class: 'pause-row' }, icon('pause'), t('%{m} min break', { m: r.minutes }));
      if (r.row === 'change') return h('p', { class: 'pause-row' }, t('Continue on line %{line}, tour %{tour}', { line: r.line, tour: r.tour }));
      return null;
    });
  }

  /* The sheet keeps the stop the bus heads for in view, when that changes. */
  function followSheet() {
    var d = state && state.duty;
    if (!d || ui.tab !== 'duty') return;
    var at = [d.focus, d.next_stop].join('|');
    if (ui.sheetAt === at) return;
    var el = appEl.querySelector('[data-now="stop"]') || appEl.querySelector('[data-now="trip"]');
    var box = el && el.closest('[data-keep]');
    if (!el || !box) return;
    ui.sheetAt = at;
    var r = el.getBoundingClientRect(), b = box.getBoundingClientRect();
    box.scrollTop += r.top - b.top - b.height * 0.35;
  }

  // ---- the break (Omsi-Hub's PauzeApp): in the game's time, against the layover

  function breakApp() {
    var d = state.duty;
    var clock = state.clock || 0;
    var since = state.break_since;
    var next = d ? d.trips[d.trip + 1] : null;
    var planned = d ? d.break_planned || 0 : 0;
    var running = since !== null && since !== undefined;
    var spent = running ? Math.max(0, Math.floor((((clock - since) % 86400) + 86400) % 86400 / 60)) : null;
    var left = running ? planned - spent : null;
    var part = planned > 0 && running ? Math.min(1, spent / planned) : running ? 1 : 0;
    var klass = !running ? '' : next && left < 0 ? 'late' : 'ontime';
    var label = !running
      ? (planned > 0 ? t('Scheduled break at the terminus') : '')
      : !next ? t('On break') : left >= 0 ? t('%{minutes} min left', { minutes: left }) : t('%{minutes} min over', { minutes: -left });
    return [
      h('div', { class: 'ring ' + klass, style: { '--part': String(part) } }, h('b', null, String(running ? spent : planned)), h('span', null, t('%{minutes} min', { minutes: '' }).trim())),
      h('p', { class: 'label' }, label),
      running
        ? h('button', { type: 'button', class: 'action', onclick: function () { send({ do: 'break', on: false }); } }, icon('play'), t('End break'))
        : h('button', { type: 'button', class: 'action main', onclick: function () { send({ do: 'break', on: true }); } }, icon('pause'), t('Start break')),
      next ? [
        h('p', { class: 'kicker' }, t('Next trip')),
        h('div', { class: 'sheet' }, h('div', { class: 'trow ahead' },
          h('span', { class: 'ttime' }, hhmm(next.dep)),
          plate(next.line),
          h('span', { class: 'grow' }, h('b', null, next.terminus), h('small', null, next.stops === 1 ? t('1 stop') : t('%{n} stops', { n: next.stops }))),
          h('span', { class: 'tarr' }, hhmm(next.arr))))
      ] : null
    ];
  }

  // ---- the bus's screens

  function screensApp() {
    var list = state.screens || [];
    return [
      state.bus ? h('p', { class: 'kicker' }, state.bus) : null,
      list.length === 0
        ? h('p', { class: 'empty' }, state.bus ? t('This bus has no screens to show here.') : t('Waiting for the game…'))
        : [
            h('p', { class: 'explain' }, t('Tap a screen to use it here.')),
            h('ul', { class: 'list' }, list.map(function (s) {
              var what = s.html || (s.panel && s.fields) ? t('Touchscreen') : s.keys.length ? t('%{count} keys', { count: s.keys.length }) : '';
              return h('li', null, h('button', { type: 'button', class: 'row', onclick: function () { openScreen(s.id); } },
                icon('screen', 'lead'), h('span', null, h('b', null, s.name), what ? h('small', null, what) : null), h('span', { class: 'chev' }, '›')));
            }))
          ]
    ];
  }

  // ---- this device: its size, its map, pairing another

  function deviceApp() {
    var seg = function (options, value, onPick) {
      return h('div', { class: 'segbtns' }, options.map(function (o) {
        return h('button', { type: 'button', 'aria-pressed': o[0] === value ? 'true' : 'false', onclick: function () { onPick(o[0]); } }, o[2] || null, o[1]);
      }));
    };
    var sign = function (s) {
      var c = h('canvas', { class: 'sign', width: 48, height: 48 });
      var g = c.getContext('2d');
      window.OmsiMap.stopSign(g, s === 'game' ? (state.stop_style || 'de') : s, 24, 24, 40);
      return c;
    };
    if (ui.qr === null) loadQr();
    return [
      h('section', { class: 'card' },
        h('h2', null, t('This device')),
        h('div', { class: 'setting' },
          h('span', null, t('Size')),
          h('div', { class: 'stepper' },
            h('button', { type: 'button', 'aria-label': t('Smaller'), disabled: prefs.size <= 0.8, onclick: function () { prefs.size = clampSize(prefs.size - 0.1); savePrefs(); applySize(); render(true); } }, 'A−'),
            h('b', null, Math.round(prefs.size * 100) + ' %'),
            h('button', { type: 'button', 'aria-label': t('Larger'), disabled: prefs.size >= 1.6, onclick: function () { prefs.size = clampSize(prefs.size + 0.1); savePrefs(); applySize(); render(true); } }, 'A+'))),
        h('div', { class: 'setting col' },
          h('span', null, t('Bus stop signs')),
          seg([['game', t('As in the game'), sign('game')], ['de', 'H', sign('de')], ['uk', 'UK', sign('uk')], ['fr', 'FR', sign('fr')]], prefs.stops, function (v) {
            prefs.stops = v;
            savePrefs();
            render(true);
            if (mapView) mapView.update();
          })),
        h('div', { class: 'setting col' },
          h('span', null, t('Map')),
          seg([[false, t('Tilted (3D)')], [true, t('Flat (2D)')]], prefs.flat, function (v) {
            prefs.flat = v;
            savePrefs();
            if (mapView) {
              mapView.map.setFlat(v);
              mapView.update();
            }
            render(true);
          }))),
      h('section', { class: 'card' },
        h('h2', null, t('Pair another device')),
        h('p', { class: 'explain' }, t('Scan this code with the other phone or tablet: it opens this page and pairs at once.')),
        qrView(),
        h('button', { type: 'button', class: 'action quiet', onclick: function () { ui.qr = null; render(true); } }, icon('refresh'), t('New QR code'))),
      h('section', { class: 'card' },
        personnelLine(),
        h('button', { type: 'button', class: 'action quiet', onclick: function () { if (window.confirm(t('Disconnect this device? It has to be paired again.'))) unpair(); } }, t('Disconnect this device')))
    ];
  }

  function loadQr() {
    ui.qr = { loading: true };
    call('api/qr')
      .then(function (r) { return r.status === 200 ? r.json() : null; })
      .then(function (j) {
        ui.qr = j || { failed: true };
        ui.qr.when = Date.now();
        render(true);
      })
      .catch(function () {
        ui.qr = { failed: true };
        render(true);
      });
  }

  /* The QR code as an SVG drawing: dark runs of each row as one rectangle, with the quiet
     zone of four modules round it that readers want. */
  function qrView() {
    var q = ui.qr;
    if (!q || q.loading) return h('p', { class: 'empty' }, t('Loading…'));
    if (!q.url || !q.bits) return h('p', { class: 'explain' }, t('The game has no network address for the QR code yet.'));
    /* covered while streaming (the game's setting), until "Show" uncovers it for a while */
    if (state && state.hide && !(ui.qrShown > Date.now())) {
      return h('div', { class: 'qr-box' },
        h('div', { class: 'covered' }, icon('lock'), h('b', null, t('Hidden while streaming'))),
        h('button', { type: 'button', class: 'action quiet', onclick: function () {
          ui.qrShown = Date.now() + 30000;
          render(true);
          setTimeout(function () { render(true); }, 30500);
        } }, icon('eye'), t('Show for 30 seconds')));
    }
    var ns = 'http://www.w3.org/2000/svg';
    var n = q.size, quiet = 4;
    var svg = document.createElementNS(ns, 'svg');
    svg.setAttribute('viewBox', '0 0 ' + (n + quiet * 2) + ' ' + (n + quiet * 2));
    svg.setAttribute('class', 'qr');
    svg.setAttribute('role', 'img');
    svg.setAttribute('aria-label', q.url);
    var bg = document.createElementNS(ns, 'rect');
    bg.setAttribute('width', n + quiet * 2);
    bg.setAttribute('height', n + quiet * 2);
    bg.setAttribute('fill', '#fff');
    svg.appendChild(bg);
    var d = '';
    for (var y = 0; y < n; y++) {
      var x = 0;
      while (x < n) {
        if (q.bits.charAt(y * n + x) === '1') {
          var s = x;
          while (x < n && q.bits.charAt(y * n + x) === '1') x++;
          d += 'M' + (s + quiet) + ' ' + (y + quiet) + 'h' + (x - s) + 'v1h-' + (x - s) + 'z';
        } else {
          x++;
        }
      }
    }
    var p = document.createElementNS(ns, 'path');
    p.setAttribute('d', d);
    p.setAttribute('fill', '#0e1522');
    svg.appendChild(p);
    return h('div', { class: 'qr-box' }, svg,
      h('p', { class: 'qr-url' }, q.url.replace(/^http:\/\//, '').replace(/\/\?pair=.*$/, '')),
      h('p', { class: 'explain' }, t('or enter the pairing code'), ' ', h('b', { class: 'pcode' }, q.code)));
  }

  // ------------------------------------------------------------------ the trip's report

  /* At the end of a trip the game sends what it was: shown once on each device, while it
     is recent (within a quarter of an hour of the game's time). */
  function report() {
    var r = state && state.report;
    if (!r || !modalEl.hidden) return;
    // (known by what it is about: the count starts again when the game does)
    var id = [r.seq, r.index, r.terminus, r.departure].join('|');
    if (id === prefs.report) return;
    var age = r.at !== undefined && r.at !== null ? ((((state.clock || 0) - r.at) % 86400) + 86400) % 86400 : 0;
    prefs.report = id;
    savePrefs();
    if (age > 900) return;
    var served = Math.max(0, r.served || 0);
    var part = function (n) { return served > 0 ? (100 * n / served).toFixed(2) + '%' : '0'; };
    var arrival = r.arrival !== null && r.arrival !== undefined
      ? h('div', { class: 'big-offset ' + punctualityClass(r.arrival_p) }, h('b', null, r.arrival_p === 'on_time' ? t('on time') : offset(r.arrival)), h('span', null, t('at the last stop')))
      : null;
    var close = function () {
      modalEl.hidden = true;
      modalEl.textContent = '';
    };
    modalEl.textContent = '';
    modalEl.appendChild(h('div', { class: 'modal-card', role: 'dialog', 'aria-modal': 'true' },
      h('p', { class: 'kicker' }, t('Trip finished')),
      h('div', { class: 'bhead' }, plate(r.line || null), h('div', { class: 'grow' }, h('b', null, r.terminus), h('small', null, t('trip %{k} of %{n}', { k: r.index, n: r.count }) + '  ·  ' + hhmm(r.departure) + ' – ' + hhmm(r.end)))),
      arrival,
      h('div', { class: 'split' },
        h('i', { class: 'ontime', style: { width: part(r.on_time) } }),
        h('i', { class: 'early', style: { width: part(r.early) } }),
        h('i', { class: 'late', style: { width: part(r.late) } })),
      h('div', { class: 'tiles three' },
        h('div', null, h('b', { class: 'ontime' }, String(r.on_time)), h('span', null, t('on time'))),
        h('div', null, h('b', { class: 'early' }, String(r.early)), h('span', null, t('too early'))),
        h('div', null, h('b', { class: 'late' }, String(r.late)), h('span', null, t('too late')))),
      h('p', { class: 'explain' }, t('%{n} stops served. Saved in your personnel file with the session.', { n: served })),
      h('button', { type: 'button', class: 'action main', onclick: close }, t('OK'))));
    modalEl.hidden = false;
  }

  // ------------------------------------------------------------------ a screen, full size

  /*
   * A screen of the bus: its display, large, and the device it belongs to (the display with
   * its keys) as the game photographs it in the cab, a few times a second. A tap on the
   * display goes to the display (a touchscreen, a touch field before it); a tap on the
   * device's picture is clicked into the cab where the picture shows it, so every key works
   * as it does with the mouse. The two lie side by side or one above the other, whichever
   * gives them more room, and never over each other.
   *
   * A device the game photographs (`view`: nearly all of them - an IBIS, a ticket table, an
   * ALMEX made of pages) is shown as that picture alone: a straight scan of its face, on the
   * whole screen as large as it fits (black beside it), with only a small back button over
   * it; turned a quarter when that makes it larger (a wide IBIS on a phone held upright).
   */
  var shown = null;

  function openScreen(id) {
    ui.screen = id;
    var s = (state.screens || []).filter(function (x) { return x.id === id; })[0];
    if (!s) return;
    /* (drawn here from its form; a device without one as the game's picture of it) */
    if (s.form) return openForm(s);
    if (s.view) return openPanel(s);
    var pane = function (cls, img, waitText) {
      var waiting = h('p', { class: 'waiting' }, waitText);
      var el = h('figure', { class: 'pane ' + cls }, img, waiting);
      return { el: el, img: img, waiting: waiting, url: null, aspect: 0 };
    };
    var disp = pane('disp', h('img', { class: 'pic' + (id.charAt(0) === 't' ? ' pixel' : ''), alt: s.name, draggable: 'false' }), t('Waiting for the game…'));
    var dev = s.view ? pane('dev', h('img', { class: 'pic', alt: '', draggable: 'false' }), t('Waiting for the game…')) : null;
    if (dev) dev.aspect = s.view.w / s.view.h;
    var stage = h('div', { class: 'stage' }, disp.el, dev ? dev.el : null);
    screenEl.textContent = '';
    screenEl.appendChild(h('header', { class: 'bar' },
      h('button', { type: 'button', onclick: closeScreen }, '‹ ' + t('Back')),
      h('b', null, s.name),
      dev ? h('small', { class: 'hint' }, t('Tap the keys in the picture as in the cab.')) : null));
    screenEl.appendChild(stage);
    screenEl.hidden = false;
    shown = { id: id, s: s, stage: stage, disp: disp, dev: dev };
    touchDisplay(disp.img, s);
    if (dev) touchDevice(dev, s);
    layoutScreen();
    pictures(shown, disp, 'api/frame');
    if (dev) pictures(shown, dev, 'api/view');
  }

  /* A device, full screen: its picture and a back button, nothing else. */
  function openPanel(s) {
    var img = h('img', { class: 'pic', alt: s.name, draggable: 'false' });
    var waiting = h('p', { class: 'waiting' }, t('Waiting for the game…'));
    var dev = { el: h('figure', { class: 'pane full' }, img, waiting), img: img, waiting: waiting, url: null, aspect: s.view.w / s.view.h };
    var stage = h('div', { class: 'stage' }, dev.el);
    screenEl.textContent = '';
    screenEl.classList.add('full');
    screenEl.appendChild(stage);
    screenEl.appendChild(h('button', { type: 'button', class: 'back', 'aria-label': t('Back'), title: t('Back'), onclick: closeScreen }, '‹'));
    screenEl.hidden = false;
    shown = { id: s.id, s: s, stage: stage, disp: null, dev: dev, full: true };
    touchDevice(dev, s);
    layoutScreen();
    pictures(shown, dev, 'api/view');
    // (the browser's own bars out of the way too, where it lets a page do that)
    try {
      var go = screenEl.requestFullscreen || screenEl.webkitRequestFullscreen;
      var p = go && go.call(screenEl, { navigationUI: 'hide' });
      if (p && p.catch) p.catch(function () { /* not allowed: the page is full enough */ });
    } catch (x) { /* an old browser */ }
  }

  function closeScreen() {
    if (shown) {
      [shown.disp, shown.dev].forEach(function (p) { if (p && p.url) URL.revokeObjectURL(p.url); });
    }
    if (shown && shown.full) {
      try {
        var fs = document.fullscreenElement || document.webkitFullscreenElement;
        var leave = document.exitFullscreen || document.webkitExitFullscreen;
        if (fs && leave) {
          var p = leave.call(document);
          if (p && p.catch) p.catch(function () { /* already left */ });
        }
      } catch (x) { /* an old browser */ }
    }
    shown = null;
    ui.screen = null;
    screenEl.hidden = true;
    screenEl.classList.remove('full');
    screenEl.textContent = '';
  }

  /* The display and the device as large as the stage allows: side by side at the same height,
     or one above the other at the same width - whichever leaves them bigger. */
  function layoutScreen() {
    if (!shown) return;
    var area = shown.stage.getBoundingClientRect();
    var gap = 12;
    var W = area.width - 2 * gap, H = area.height - 2 * gap;
    if (!(W > 0 && H > 0)) return;
    var s = shown.s;
    // (the display as it is in the bus: its texture may be stretched onto it)
    var ad = s.size[0] > 0 && s.size[1] > 0 ? Math.min(8, Math.max(0.2, s.size[0] / s.size[1])) : (shown.disp && shown.disp.aspect) || 2;
    var place = function (p, x, y, w, hh) {
      p.el.style.left = Math.round(x) + 'px';
      p.el.style.top = Math.round(y) + 'px';
      p.el.style.width = Math.round(w) + 'px';
      p.el.style.height = Math.round(hh) + 'px';
      // (a small display enlarged a lot: its pixels square and sharp, as an LCD's)
      if (p.img.naturalWidth) p.img.classList.toggle('pixel', p.img.classList.contains('pixel') || p.img.naturalWidth * 2.5 < w);
    };
    var fit = function (a, w, hh) { return w / hh > a ? [hh * a, hh] : [w, w / a]; };
    if (shown.full) {
      // the whole screen: as large as the picture fits, in the middle - turned a quarter when
      // that makes it clearly larger (it then lies across a phone held upright)
      var d = shown.dev;
      if (!d.aspect) return;
      var all = fit(d.aspect, area.width, area.height);
      var turned = fit(d.aspect, area.height, area.width);
      d.rot = turned[0] * turned[1] > all[0] * all[1] * 1.02;
      var size = d.rot ? turned : all;
      place(d, (area.width - size[0]) / 2, (area.height - size[1]) / 2, size[0], size[1]);
      d.el.classList.toggle('turned', d.rot);
      if (shown.drawn) shown.drawn.redraw();
      return;
    }
    if (!shown.dev) {
      var one = fit(ad, W, H);
      place(shown.disp, gap + (W - one[0]) / 2, gap + (H - one[1]) / 2, one[0], one[1]);
      return;
    }
    var av = shown.dev.aspect;
    // side by side: one height for both, as much as the width allows
    var hs = Math.min(H, (W - gap) / (ad + av));
    // one above the other: one width for both (the display at most a third of the height)
    var wsV = Math.min(W, (H - gap) / (1 / ad + 1 / av));
    var side = hs * hs * (ad + av);
    var stack = wsV * wsV * (1 / ad + 1 / av);
    if (side >= stack) {
      var x = gap + (W - gap - hs * (ad + av)) / 2;
      var y = gap + (H - hs) / 2;
      place(shown.disp, x, y, hs * ad, hs);
      place(shown.dev, x + hs * ad + gap, y, hs * av, hs);
    } else {
      var top = gap + (H - gap - wsV / ad - wsV / av) / 2;
      var left = gap + (W - wsV) / 2;
      place(shown.disp, left, top, wsV, wsV / ad);
      place(shown.dev, left, top + wsV / ad + gap, wsV, wsV / av);
    }
  }

  // (a device of pages on the whole screen: laid out again once the browser has made it so)
  ['fullscreenchange', 'webkitfullscreenchange'].forEach(function (n) {
    document.addEventListener(n, function () { setTimeout(layoutScreen, 50); });
  });

  var wasWide = null;
  window.addEventListener('resize', function () {
    layoutScreen();
    var w = wide();
    if (w !== wasWide) {
      wasWide = w;
      render(true);
      startNav();
    }
  });

  /* Where a pointer is on a picture, 0..1 across it. */
  function where(img, e) {
    var r = img.getBoundingClientRect();
    return { x: Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)), y: Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)) };
  }

  /* The same on a pane that may be turned a quarter (clockwise: the picture's top on the
     screen's right). */
  function whereOn(p, e) {
    if (!p.rot) return where(p.img, e);
    var r = p.el.getBoundingClientRect();
    var dx = e.clientX - (r.left + r.width / 2), dy = e.clientY - (r.top + r.height / 2);
    var clamp = function (x) { return Math.min(1, Math.max(0, x)); };
    // (turned, the picture's width runs down the screen and its height right to left)
    return { x: clamp(0.5 + dy / r.height), y: clamp(0.5 - dx / r.width) };
  }

  /* Taps on the display: where on it, as 0..1 across the part shown. */
  function touchDisplay(img, s) {
    var down = false;
    var lastMove = 0;
    img.addEventListener('pointerdown', function (e) {
      e.preventDefault();
      try { img.setPointerCapture(e.pointerId); } catch (x) { /* an old browser */ }
      down = true;
      var p = where(img, e);
      send({ do: 'pointer', screen: s.id, kind: 'down', u: p.x, v: p.y });
    });
    img.addEventListener('pointermove', function (e) {
      /* only a page follows the finger; a switch is pressed and let go */
      if (!down || !s.html || e.timeStamp - lastMove < 50) return;
      lastMove = e.timeStamp;
      var p = where(img, e);
      send({ do: 'pointer', screen: s.id, kind: 'move', u: p.x, v: p.y });
    });
    var up = function (e) {
      if (!down) return;
      down = false;
      var p = where(img, e);
      send({ do: 'pointer', screen: s.id, kind: 'up', u: p.x, v: p.y });
    };
    img.addEventListener('pointerup', up);
    img.addEventListener('pointercancel', up);
  }

  /* Taps on the device's picture: clicked into the cab where the picture shows them. The
     finger leaves a mark, and the key under it lights up, at once - the picture follows a
     moment later. */
  function touchDevice(p, s) {
    var img = p.img;
    var down = false;
    var page = false;
    var lastMove = 0;
    var mark = function (at) {
      var dot = h('i', { class: 'touchdot', style: { left: at.x * 100 + '%', top: at.y * 100 + '%' } });
      p.el.appendChild(dot);
      setTimeout(function () { dot.remove(); }, 450);
      var spot = (s.view.keys || []).filter(function (r) { return at.x >= r[0] && at.x <= r[2] && at.y >= r[1] && at.y <= r[3]; })[0];
      if (spot) {
        var lit = h('i', { class: 'spot', style: { left: spot[0] * 100 + '%', top: spot[1] * 100 + '%', width: (spot[2] - spot[0]) * 100 + '%', height: (spot[3] - spot[1]) * 100 + '%' } });
        p.el.appendChild(lit);
        setTimeout(function () { lit.remove(); }, 400);
      }
    };
    img.addEventListener('pointerdown', function (e) {
      e.preventDefault();
      try { img.setPointerCapture(e.pointerId); } catch (x) { /* an old browser */ }
      down = true;
      page = false;
      var at = whereOn(p, e);
      mark(at);
      send({ do: 'tap', screen: s.id, kind: 'down', x: at.x, y: at.y }).then(function (j) { page = !!(j && j.page); });
    });
    img.addEventListener('pointermove', function (e) {
      /* only a page under the finger follows it */
      if (!down || !page || e.timeStamp - lastMove < 50) return;
      lastMove = e.timeStamp;
      var at = whereOn(p, e);
      send({ do: 'tap', screen: s.id, kind: 'move', x: at.x, y: at.y });
    });
    var up = function (e) {
      if (!down) return;
      down = false;
      var at = whereOn(p, e);
      send({ do: 'tap', screen: s.id, kind: 'up', x: at.x, y: at.y });
    };
    img.addEventListener('pointerup', up);
    img.addEventListener('pointercancel', up);
  }

  // ---- a device drawn by the page (its form, `companion::form` in the game)

  /*
   * The device is not a picture of the cab but drawn here, flat and straight, from its form:
   * the triangles of its face with their textures, the way the game draws them (a depth
   * buffer, the opaque parts first and the blended ones from the back), as sharp as this
   * screen is. What changes comes as data (`api/live`): which parts show (an ALMEX's menu
   * pages), where the moving ones are (a key pressed in), the strings of its text textures -
   * written here in the bus's own `.oft` font with the game's rules (`omsi_content::font`).
   * Only what a script paints pixel by pixel (a script texture, an `[htmltexture]` page)
   * comes as a picture. A tap goes to the switch under the finger as a ray straight at the
   * face finds it in the cab; the key under it darkens while it is held.
   */

  /* Rust's `f32::round`: halves away from nought. */
  function roundAway(x) {
    return x < 0 ? -Math.round(-x) : Math.round(x);
  }

  /* A font as the page writes with it: its glyphs, its stand-ins, its bitmap's pixels. */
  function makeFont(j, px) {
    var first = {};
    j.g.forEach(function (g, k) { if (first[g[0]] === undefined) first[g[0]] = k; });
    return { h: Math.max(1, j.h), gap: j.gap, space: j.space, iw: j.iw, ih: j.ih, g: j.g, first: first, alias: j.alias || {}, px: px };
  }

  function isSpace(ch) {
    return /\s/.test(ch);
  }

  /* The glyph `Font::glyph` draws for a letter (-1: none, only the gap moves on). */
  function glyphOf(f, ch) {
    var k = f.first[ch];
    if (k !== undefined) return k;
    k = f.alias[ch];
    if (k !== undefined) return k;
    if (ch >= 'a' && ch <= 'z') {
      k = f.first[ch.toUpperCase()];
      if (k !== undefined) return k;
    }
    return -1;
  }

  /* `FontAtlas::text_width`: every letter's width and the gap after it. */
  function textWidth(f, chars) {
    var w = 0;
    chars.forEach(function (ch) {
      if (isSpace(ch)) w += f.space;
      else {
        var k = glyphOf(f, ch);
        if (k >= 0) w += Math.max(0, f.g[k][2] - f.g[k][1]);
      }
      w += f.gap;
    });
    return w;
  }

  /* `TextAlign::offset`. */
  function alignOffset(o, grid, width, advance, gap) {
    var visible = Math.max(0, advance - Math.max(0, gap));
    var x;
    if (o === 1) x = 0;
    else if (o === 2) x = width - visible;
    else if (o === 3) x = roundAway((width - visible) / 2);
    else if (o === 5) x = Math.ceil((width - visible) / 2);
    else x = Math.floor((width - visible) / 2);
    var g = Math.max(1, grid);
    if (o === 2 || o === 5) x = Math.ceil(x / g) * g;
    else if (o === 3) x = roundAway(x / g) * g;
    else x = Math.floor(x / g) * g;
    return Math.max(0, Math.trunc(x));
  }

  /* One line into `out` (w x h RGBA) at row `top`: `FontAtlas::render_unscaled`. */
  function writeLine(f, t, chars, out, w, h, top, rows) {
    var gh = f.h;
    var y0 = Math.trunc((rows - gh) / 2);
    var x = alignOffset(t.o, t.g, w, textWidth(f, chars), f.gap);
    var px = f.px;
    chars.forEach(function (ch) {
      if (isSpace(ch)) {
        x += f.space + f.gap;
        return;
      }
      var k = glyphOf(f, ch);
      if (k < 0) {
        x += f.gap;
        return;
      }
      var g = f.g[k];
      var gw = Math.max(0, g[2] - g[1]);
      for (var gy = 0; gy < gh; gy++) {
        var sy = g[3] + gy, dy = y0 + gy;
        if (sy < 0 || sy >= f.ih || dy < 0 || dy >= rows || top + dy >= h) continue;
        for (var gx = 0; gx < gw; gx++) {
          var sx = g[1] + gx, dx = x + gx;
          if (sx < 0 || sx >= f.iw || dx < 0 || dx >= w) continue;
          var si = (sy * f.iw + sx) * 4;
          var a = px[si + 3];
          if (a === 0) continue;
          var di = ((top + dy) * w + dx) * 4;
          var af = a / 255, inv = 1 - af;
          var r = t.full ? px[si] : t.rgb[0], gg = t.full ? px[si + 1] : t.rgb[1], b = t.full ? px[si + 2] : t.rgb[2];
          out[di] = Math.floor(r * af + out[di] * inv);
          out[di + 1] = Math.floor(gg * af + out[di + 1] * inv);
          out[di + 2] = Math.floor(b * af + out[di + 2] * inv);
          out[di + 3] = Math.max(out[di + 3], a);
        }
      }
      x += gw + f.gap;
    });
  }

  /* `fit_scale`: a smaller font magnified by whole times (a pixel font stays crisp), a taller
   * one shrunk to the line. */
  function fitScale(lineH, fontH) {
    if (!(lineH > 0 && fontH > 0)) return 1;
    var s = lineH / fontH;
    return s >= 1 ? Math.floor(s + 1e-4) : s;
  }

  /* `resample_into`: `src` (sw x sh) scaled to dw x dh over `out` (w x h) at (x0, y0), every
   * pixel the average of the part of `src` under it. */
  function resampleInto(src, sw, sh, out, w, h, x0, y0, dw, dh) {
    if (sw <= 0 || sh <= 0 || dw <= 0 || dh <= 0) return;
    var fx = sw / dw, fy = sh / dh;
    var span = function (a, b, n) {
      var v = [];
      for (var i = Math.max(0, Math.floor(a)); i < b && i < n; i++) {
        var lo = Math.max(a, i), hi = Math.min(b, i + 1);
        if (hi > lo) v.push([i, hi - lo]);
      }
      return v;
    };
    var cols = [];
    for (var x = 0; x < dw; x++) cols.push(span(x * fx, (x + 1) * fx, sw));
    for (var y = 0; y < dh; y++) {
      var oy = y0 + y;
      if (oy < 0 || oy >= h) continue;
      var rows = span(y * fy, (y + 1) * fy, sh);
      for (var cx = 0; cx < dw; cx++) {
        var ox = x0 + cx;
        if (ox < 0 || ox >= w) continue;
        var a = 0, r = 0, g = 0, b = 0, total = 0;
        for (var ri = 0; ri < rows.length; ri++) {
          for (var ci = 0; ci < cols[cx].length; ci++) {
            var k = rows[ri][1] * cols[cx][ci][1];
            var si = (rows[ri][0] * sw + cols[cx][ci][0]) * 4;
            a += src[si + 3] * k;
            r += src[si] * k;
            g += src[si + 1] * k;
            b += src[si + 2] * k;
            total += k;
          }
        }
        if (a <= 0 || total <= 0) continue;
        var cover = Math.min(255, Math.max(0, Math.round(a / total)));
        if (cover === 0) continue;
        var di = (oy * w + ox) * 4, inv = 1 - cover / 255;
        out[di] = Math.min(255, Math.round(r / total + out[di] * inv));
        out[di + 1] = Math.min(255, Math.round(g / total + out[di + 1] * inv));
        out[di + 2] = Math.min(255, Math.round(b / total + out[di + 2] * inv));
        out[di + 3] = Math.max(out[di + 3], cover);
      }
    }
  }

  /* `FontAtlas::render_fitted`: a display font the player chose for the bus, every line in the
   * place a line of the bus's own font has (`t.lh` rows), its letters scaled to fill it; a line
   * too wide at that size drawn smaller. */
  function writeFitted(f, t, text) {
    var w = t.w, h = t.h, lh = Math.max(1, t.lh);
    var out = new Uint8Array(w * h * 4);
    var lines = text.split('@');
    var block = lh * lines.length;
    var top = lines.length === 1 ? Math.trunc((h - block) / 2) : Math.floor(Math.max(0, h - block) / 2);
    var scale = fitScale(lh, f.h);
    for (var i = 0; i < lines.length; i++) {
      var slot = top + i * lh;
      if (slot >= h) break;
      var chars = Array.from(lines[i]);
      var adv = textWidth(f, chars);
      var visible = Math.max(0, adv - Math.max(0, f.gap));
      if (visible <= 0) continue;
      var s = scale;
      if (visible * s > w) s = Math.min(s, fitScale(w, visible));
      var tw = Math.max(1, adv), th = f.h;
      var src = new Uint8Array(tw * th * 4);
      writeLine(f, { o: 1, g: 1, full: t.full, rgb: t.rgb }, chars, src, tw, th, 0, th);
      var dw = Math.max(1, roundAway(tw * s)), dh = Math.max(1, roundAway(th * s));
      var x0 = alignOffset(t.o, t.g, w, dw, roundAway(f.gap * s));
      resampleInto(src, tw, th, out, w, h, x0, slot + Math.trunc((lh - dh) / 2), dw, dh);
    }
    return out;
  }

  /* A text texture's picture of `text`: `FontAtlas::render_aligned` ('@' breaks lines). */
  function writeText(f, t, text) {
    var w = t.w, h = t.h;
    var out = new Uint8Array(w * h * 4);
    if (!f) return out;
    if (t.lh > 0) return writeFitted(f, t, text);
    if (text.indexOf('@') >= 0) {
      var lines = text.split('@');
      var lh = f.h;
      var top = Math.floor(Math.max(0, h - lh * lines.length) / 2);
      for (var i = 0; i < lines.length; i++) {
        var y0 = top + i * lh;
        if (y0 >= h) break;
        writeLine(f, t, Array.from(lines[i]), out, w, h, y0, lh);
      }
      return out;
    }
    writeLine(f, t, Array.from(text), out, w, h, 0, h);
    return out;
  }

  var VS = 'attribute vec3 aPos; attribute vec2 aUv; uniform mat4 uMove; uniform vec4 uSheet; varying vec2 vUv;' +
    'void main() { vec4 p = uMove * vec4(aPos, 1.0); vUv = aUv;' +
    ' gl_Position = vec4(2.0 * p.x / uSheet.x - 1.0, 1.0 - 2.0 * p.y / uSheet.y, 1.0 - 2.0 * (p.z + uSheet.w) / (uSheet.z + uSheet.w), 1.0); }';
  var FS = 'precision highp float; uniform sampler2D uTex; uniform int uHasTex; uniform vec4 uColour; uniform int uAlpha; uniform int uWrap;' +
    ' uniform vec4 uCrop; uniform float uFade; uniform float uDim; varying vec2 vUv;' +
    'void main() { vec4 c = uColour;' +
    ' if (uHasTex == 1) { vec2 uv = uWrap == 1 ? fract(vUv) : clamp(vUv, 0.0, 1.0); uv = (uv - uCrop.xy) / (uCrop.zw - uCrop.xy);' +
    '  vec4 t = texture2D(uTex, uv); c = vec4(c.rgb * t.rgb, t.a); }' +
    ' if (uAlpha == 0) c.a = 1.0; else if (uAlpha == 1) { if (c.a < 0.5) discard; c.a = 1.0; }' +
    ' gl_FragColor = vec4(c.rgb * uDim, c.a * uFade); }';

  function shader(gl, type, src) {
    var s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) || 'shader');
    return s;
  }

  /* The GL side of a form: its program, a buffer per part, its textures. */
  function makeDrawer(canvas, form) {
    var gl = canvas.getContext('webgl2', { alpha: false, antialias: true, premultipliedAlpha: false, preserveDrawingBuffer: false }) ||
      canvas.getContext('webgl', { alpha: false, antialias: true, premultipliedAlpha: false, preserveDrawingBuffer: false });
    if (!gl) return null;
    var gl2 = typeof WebGL2RenderingContext !== 'undefined' && gl instanceof WebGL2RenderingContext;
    var big = gl2 || gl.getExtension('OES_element_index_uint');
    var prog = gl.createProgram();
    gl.attachShader(prog, shader(gl, gl.VERTEX_SHADER, VS));
    gl.attachShader(prog, shader(gl, gl.FRAGMENT_SHADER, FS));
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog) || 'program');
    var loc = {};
    ['aPos', 'aUv'].forEach(function (n) { loc[n] = gl.getAttribLocation(prog, n); });
    ['uMove', 'uSheet', 'uTex', 'uHasTex', 'uColour', 'uAlpha', 'uWrap', 'uCrop', 'uFade', 'uDim'].forEach(function (n) { loc[n] = gl.getUniformLocation(prog, n); });
    var q = form.q;
    var parts = form.parts.map(function (p) {
      var n = p.v.length / 5;
      var pos = new Float32Array(n * 3), uv = new Float32Array(n * 2);
      for (var i = 0; i < n; i++) {
        pos[i * 3] = p.v[i * 5] / q;
        pos[i * 3 + 1] = p.v[i * 5 + 1] / q;
        pos[i * 3 + 2] = p.v[i * 5 + 2] / q;
        uv[i * 2] = p.v[i * 5 + 3] / q;
        uv[i * 2 + 1] = p.v[i * 5 + 4] / q;
      }
      var wide = n > 65535;
      if (wide && !big) return null;
      var bp = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, bp);
      gl.bufferData(gl.ARRAY_BUFFER, pos, gl.STATIC_DRAW);
      var bu = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, bu);
      gl.bufferData(gl.ARRAY_BUFFER, uv, gl.STATIC_DRAW);
      var bi = gl.createBuffer();
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, bi);
      gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, wide ? new Uint32Array(p.i) : new Uint16Array(p.i), gl.STATIC_DRAW);
      return { p: p, pos: bp, uv: bu, idx: bi, count: p.i.length, type: wide ? gl.UNSIGNED_INT : gl.UNSIGNED_SHORT };
    });
    var textures = {};
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
    gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    /* A texture from a picture or from pixels; sharp pixels for a display's. */
    function put(key, src, w, h, sharp, crop) {
      var t = textures[key];
      if (!t) {
        t = textures[key] = { tex: gl.createTexture(), crop: crop || [0, 0, 1, 1] };
      }
      if (crop) t.crop = crop;
      gl.bindTexture(gl.TEXTURE_2D, t.tex);
      if (src instanceof Uint8Array) gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, src);
      else gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, src);
      var mips = !sharp && gl2;
      if (mips) gl.generateMipmap(gl.TEXTURE_2D);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, mips ? gl.LINEAR_MIPMAP_LINEAR : (sharp ? gl.NEAREST : gl.LINEAR));
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, sharp ? gl.NEAREST : gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    }
    var ident = new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
    /* Everything shown, as `live` has it; `dim` the meshes held down by a finger. */
    function draw(live, dim) {
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clearColor(0, 0, 0, 1);
      gl.clearDepth(1);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      gl.enable(gl.DEPTH_TEST);
      gl.depthFunc(gl.LEQUAL);
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      gl.disable(gl.CULL_FACE);
      gl.useProgram(prog);
      gl.uniform4f(loc.uSheet, form.w / q, form.h / q, form.front / q, form.behind / q);
      gl.uniform1i(loc.uTex, 0);
      gl.activeTexture(gl.TEXTURE0);
      parts.forEach(function (d, k) {
        if (!d) return;
        var p = d.p;
        if (live.shown && live.shown.charAt(p.m) === '0') return;
        var t = null;
        if (p.tex) {
          t = p.tex.f !== undefined ? textures['f' + p.tex.f] : p.tex.t !== undefined ? textures['t' + p.tex.t] : textures['s' + p.tex.s];
          /* (a picture not here yet: the part waits for it rather than showing blank) */
          if (!t) return;
        }
        var m = live.moved[p.m];
        gl.uniformMatrix4fv(loc.uMove, false, m || ident);
        var c = p.c;
        gl.uniform4f(loc.uColour, c[0], c[1], c[2], c[3]);
        gl.uniform1i(loc.uAlpha, p.a);
        gl.uniform1i(loc.uWrap, p.wrap ? 1 : 0);
        gl.uniform1f(loc.uFade, live.fade[k] === undefined ? 1 : live.fade[k]);
        gl.uniform1f(loc.uDim, dim && dim[p.m] ? 0.72 : 1);
        gl.uniform1i(loc.uHasTex, t ? 1 : 0);
        if (t) {
          gl.bindTexture(gl.TEXTURE_2D, t.tex);
          gl.uniform4f(loc.uCrop, t.crop[0], t.crop[1], t.crop[2], t.crop[3]);
        }
        gl.depthMask(!!p.zw);
        gl.bindBuffer(gl.ARRAY_BUFFER, d.pos);
        gl.enableVertexAttribArray(loc.aPos);
        gl.vertexAttribPointer(loc.aPos, 3, gl.FLOAT, false, 0, 0);
        gl.bindBuffer(gl.ARRAY_BUFFER, d.uv);
        gl.enableVertexAttribArray(loc.aUv);
        gl.vertexAttribPointer(loc.aUv, 2, gl.FLOAT, false, 0, 0);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, d.idx);
        gl.drawElements(gl.TRIANGLES, d.count, d.type, 0);
      });
      gl.depthMask(true);
    }
    return { gl: gl, put: put, draw: draw };
  }

  /* A picture fetched with the device's key. */
  function fetchPicture(path) {
    return call(path).then(function (r) {
      if (r.status !== 200) throw new Error(String(r.status));
      var uv = r.headers.get('X-Uv');
      return r.blob().then(function (b) {
        return new Promise(function (done, fail) {
          var img = new Image();
          var url = URL.createObjectURL(b);
          img.onload = function () { URL.revokeObjectURL(url); done({ img: img, uv: uv ? uv.split(',').map(Number) : null }); };
          img.onerror = function () { URL.revokeObjectURL(url); fail(new Error('picture')); };
          img.src = url;
        });
      });
    });
  }

  /* A picture's pixels (a font's bitmap). */
  function pixelsOf(img) {
    var c = document.createElement('canvas');
    c.width = img.naturalWidth;
    c.height = img.naturalHeight;
    var g = c.getContext('2d');
    g.drawImage(img, 0, 0);
    return g.getImageData(0, 0, c.width, c.height).data;
  }

  /* A device drawn on the whole screen, and a back button. */
  function openForm(s) {
    var canvas = h('canvas', { class: 'pic formgl', 'aria-label': s.name });
    var fx = h('canvas', { class: 'formfx', 'aria-hidden': 'true' });
    var waiting = h('p', { class: 'waiting' }, t('Waiting for the game…'));
    var pane = { el: h('figure', { class: 'pane full form' }, canvas, fx, waiting), img: canvas, fx: fx, waiting: waiting, aspect: 0 };
    var stage = h('div', { class: 'stage' }, pane.el);
    screenEl.textContent = '';
    screenEl.classList.add('full');
    screenEl.appendChild(stage);
    screenEl.appendChild(h('button', { type: 'button', class: 'back', 'aria-label': t('Back'), title: t('Back'), onclick: closeScreen }, '‹'));
    screenEl.hidden = false;
    var view = { id: s.id, s: s, stage: stage, disp: null, dev: pane, full: true, form: s.form, drawn: null };
    shown = view;
    try {
      var go = screenEl.requestFullscreen || screenEl.webkitRequestFullscreen;
      var fp = go && go.call(screenEl, { navigationUI: 'hide' });
      if (fp && fp.catch) fp.catch(function () { /* not allowed: the page is full enough */ });
    } catch (x) { /* an old browser */ }
    call('api/form?screen=' + encodeURIComponent(s.id))
      .then(function (r) {
        if (r.status !== 200) throw new Error(String(r.status));
        return r.json();
      })
      .then(function (form) {
        if (shown !== view) return;
        startForm(view, form);
      })
      .catch(function () {
        /* no form after all: the device's picture, as before */
        if (shown !== view) return;
        if (s.view) openPanel(s);
      });
  }

  function startForm(view, form) {
    var pane = view.dev;
    var q = form.q;
    pane.aspect = form.w / form.h;
    var drawer;
    try {
      drawer = makeDrawer(pane.img, form);
    } catch (x) {
      drawer = null;
    }
    if (!drawer) {
      if (view.s.view) openPanel(view.s);
      return;
    }
    var st = {
      form: form,
      drawer: drawer,
      live: null,
      texts: {},
      fonts: {},
      strings: {},
      held: null,
      dirty: true,
      frame: 0
    };
    view.drawn = st;
    (form.texts || []).forEach(function (tt) { st.texts[tt.n] = tt; });
    var redraw = function () {
      if (st.frame || shown !== view || !st.live) return;
      st.frame = requestAnimationFrame(function () {
        st.frame = 0;
        if (shown !== view) return;
        fitCanvas(pane);
        drawer.draw(st.live, st.held ? st.held.dim : null);
        pane.img.classList.add('on');
        pane.waiting.hidden = true;
      });
    };
    st.redraw = redraw;
    /* a text texture written again with its string and font */
    var writeTex = function (n) {
      var tt = st.texts[n];
      if (!tt) return;
      var f = tt.font >= 0 ? st.fonts[tt.font] : null;
      if (tt.font >= 0 && !f) return;
      var s = st.strings[n] === undefined ? '' : st.strings[n];
      drawer.put('t' + n, writeText(f, tt, s), tt.w, tt.h, true);
    };
    // the textures, the fonts, the script textures' pictures
    var files = {};
    form.parts.forEach(function (p) { if (p.tex && p.tex.f !== undefined) files[p.tex.f] = true; });
    Object.keys(files).forEach(function (f) {
      fetchPicture('api/tex?n=' + f).then(function (r) {
        if (shown !== view) return;
        drawer.put('f' + f, r.img, 0, 0, false, r.uv || [0, 0, 1, 1]);
        redraw();
      }).catch(function () { /* drawn without it */ });
    });
    var fontIds = {};
    (form.texts || []).forEach(function (tt) { if (tt.font >= 0) fontIds[tt.font] = true; });
    Object.keys(fontIds).forEach(function (k) {
      Promise.all([call('api/font?n=' + k).then(function (r) { return r.json(); }), fetchPicture('api/fontimg?n=' + k)])
        .then(function (res) {
          if (shown !== view) return;
          st.fonts[k] = makeFont(res[0], pixelsOf(res[1].img));
          (form.texts || []).forEach(function (tt) { if (String(tt.font) === String(k)) writeTex(tt.n); });
          redraw();
        })
        .catch(function () { /* its texts stay empty */ });
    });
    (form.texts || []).forEach(function (tt) { if (tt.font < 0) writeTex(tt.n); });
    (form.scripts || []).forEach(function (n) { scriptFeed(view, st, n); });
    touchForm(view, st);
    layoutScreen();
    redraw();
    liveFeed(view, st, writeTex);
  }

  /* The backing store of the device's canvas: its size on the screen in device pixels. */
  function fitCanvas(pane) {
    var r = pane.el.getBoundingClientRect();
    var turned = !!pane.rot;
    var cw = turned ? r.height : r.width, ch = turned ? r.width : r.height;
    var dpr = Math.min(window.devicePixelRatio || 1, 3);
    var w = Math.max(1, Math.min(4096, Math.round(cw * dpr))), hh = Math.max(1, Math.min(4096, Math.round(ch * dpr)));
    [pane.img, pane.fx].forEach(function (c) {
      if (c.width !== w || c.height !== hh) {
        c.width = w;
        c.height = hh;
      }
    });
  }

  /* What lives on the device, as it changes. */
  function liveFeed(view, st, writeTex) {
    var v = 0;
    var q = st.form.q;
    var next = function () {
      if (shown !== view) return;
      call('api/live?screen=' + encodeURIComponent(view.id) + '&after=' + v)
        .then(function (r) {
          if (shown !== view) return null;
          if (r.status === 200) {
            return r.json().then(function (j) {
              if (shown !== view) return;
              v = j.v;
              var l = j.live || {};
              var moved = {};
              (l.m || []).forEach(function (m) {
                var a = m[1];
                // (rows of a 3x4: as a column-major 4x4 for GL)
                moved[m[0]] = new Float32Array([a[0], a[4], a[8], 0, a[1], a[5], a[9], 0, a[2], a[6], a[10], 0, a[3], a[7], a[11], 1]);
              });
              var fade = {};
              (l.a || []).forEach(function (a) { fade[a[0]] = a[1]; });
              st.live = { shown: l.s || '', moved: moved, fade: fade, rows: l.m || [] };
              Object.keys(l.t || {}).forEach(function (n) {
                if (st.strings[n] !== l.t[n]) {
                  st.strings[n] = l.t[n];
                  writeTex(n);
                }
              });
              st.redraw();
            });
          }
          if (r.status === 204) return null;
          return sleep(1000);
        })
        .catch(function () { return sleep(1000); })
        .then(next);
    };
    next();
  }

  /* A script texture's picture, as it changes. */
  function scriptFeed(view, st, n) {
    var seq = 0;
    var next = function () {
      if (shown !== view) return;
      call('api/frame?screen=x' + n + '&after=' + seq)
        .then(function (r) {
          if (shown !== view) return null;
          if (r.status === 200) {
            seq = Number(r.headers.get('X-Seq')) || seq + 1;
            return r.blob().then(function (b) {
              return new Promise(function (done) {
                var img = new Image();
                var url = URL.createObjectURL(b);
                img.onload = function () {
                  URL.revokeObjectURL(url);
                  if (shown === view) {
                    st.drawer.put('s' + n, img, 0, 0, true);
                    st.redraw();
                  }
                  done();
                };
                img.onerror = function () { URL.revokeObjectURL(url); done(); };
                img.src = url;
              });
            });
          }
          if (r.status === 204) return null;
          return sleep(1000);
        })
        .catch(function () { return sleep(1000); })
        .then(next);
    };
    next();
  }

  /* Where a finger is on the device (metres across and down its face). */
  function onFace(view, st, e) {
    var at = whereOn(view.dev, e);
    return { x: at.x * st.form.w / st.form.q, y: at.y * st.form.h / st.form.q };
  }

  /* A point moved as mesh `m` moved. */
  function movedPoint(st, m, x, y, z) {
    var a = null;
    ((st.live && st.live.rows) || []).forEach(function (r) { if (r[0] === m) a = r[1]; });
    if (!a) return [x, y, z];
    return [a[0] * x + a[1] * y + a[2] * z + a[3], a[4] * x + a[5] * y + a[6] * z + a[7], a[8] * x + a[9] * y + a[10] * z + a[11]];
  }

  /* How far before the face a flat triangle is at (x, y), when (x, y) lies on it. */
  function depthOn(t, x, y) {
    var det = (t[3] - t[0]) * (t[7] - t[1]) - (t[4] - t[1]) * (t[6] - t[0]);
    if (Math.abs(det) < 1e-12) return null;
    var w1 = ((x - t[0]) * (t[7] - t[1]) - (y - t[1]) * (t[6] - t[0])) / det;
    var w2 = ((t[3] - t[0]) * (y - t[1]) - (t[4] - t[1]) * (x - t[0])) / det;
    var w0 = 1 - w1 - w2;
    if (w0 < -1e-5 || w1 < -1e-5 || w2 < -1e-5) return null;
    return { z: t[2] * w0 + t[5] * w1 + t[8] * w2, w: [w0, w1, w2] };
  }

  /* What a finger at (x, y) presses, as a ray straight at the face finds it in the cab: a
     page (an `[htmltexture]`, where on it) before a switch, the nearest switch. */
  function pressedAt(st, x, y) {
    var f = st.form, q = f.q;
    var shownMesh = function (m) { return !st.live || st.live.shown.charAt(m) !== '0'; };
    var best = null;
    (f.parts || []).forEach(function (p) {
      if (!p.tex || p.tex.s === undefined || (f.pages || []).indexOf(p.tex.s) < 0 || !shownMesh(p.m)) return;
      for (var i = 0; i < p.i.length; i += 3) {
        var tri = [], uv = [];
        for (var k = 0; k < 3; k++) {
          var j = p.i[i + k] * 5;
          tri = tri.concat(movedPoint(st, p.m, p.v[j] / q, p.v[j + 1] / q, p.v[j + 2] / q));
          uv.push([p.v[j + 3] / q, p.v[j + 4] / q]);
        }
        var d = depthOn(tri, x, y);
        if (d && (!best || d.z > best.z)) {
          var u = uv[0][0] * d.w[0] + uv[1][0] * d.w[1] + uv[2][0] * d.w[2];
          var v = uv[0][1] * d.w[0] + uv[1][1] * d.w[1] + uv[2][1] * d.w[2];
          best = { z: d.z, page: p.tex.s, u: u - Math.floor(u), v: v - Math.floor(v) };
        }
      }
    });
    if (best) return best;
    (f.touch || []).forEach(function (t, k) {
      if (!shownMesh(t.m)) return;
      for (var i = 0; i < t.t.length; i += 9) {
        var tri = [];
        for (var c = 0; c < 3; c++) tri = tri.concat(movedPoint(st, t.m, t.t[i + c * 3] / q, t.t[i + c * 3 + 1] / q, t.t[i + c * 3 + 2] / q));
        var d = depthOn(tri, x, y);
        if (d && (!best || d.z > best.z)) best = { z: d.z, touch: k, mesh: t.m };
      }
    });
    return best;
  }

  /* The same, forgiving a finger a little off its switch (as the cab's rings round the
     mouse's ray). */
  function pressedNear(view, st, at) {
    var hit = pressedAt(st, at.x, at.y);
    if (hit) return hit;
    var r = view.dev.el.getBoundingClientRect();
    var perMetre = Math.max(r.width, r.height) / Math.max(st.form.w, st.form.h) * st.form.q;
    for (var ring = 1; ring <= 2; ring++) {
      var rad = 6 * ring / perMetre;
      for (var k = 0; k < 8 * ring; k++) {
        var a = k / (8 * ring) * Math.PI * 2;
        hit = pressedAt(st, at.x + Math.cos(a) * rad, at.y + Math.sin(a) * rad);
        if (hit) return hit;
      }
    }
    return null;
  }

  /* A switch held: the key itself darkens (its own parts); a switch with nothing drawn of
     its own (a touch field over a display) is shaded where it is, faintly. */
  function holdMark(view, st, hit) {
    var dim = {};
    dim[hit.mesh] = true;
    var drawnOwn = st.form.parts.some(function (p) { return p.m === hit.mesh && (!st.live || st.live.shown.charAt(p.m) !== '0'); });
    st.held = { dim: drawnOwn ? dim : null, touch: hit.touch, since: Date.now() };
    var fx = view.dev.fx;
    var g = fx.getContext('2d');
    g.clearRect(0, 0, fx.width, fx.height);
    if (!drawnOwn) {
      var t = st.form.touch[hit.touch], q = st.form.q;
      var sx = fx.width / (st.form.w / q), sy = fx.height / (st.form.h / q);
      g.fillStyle = 'rgba(0, 0, 0, 0.22)';
      for (var i = 0; i < t.t.length; i += 9) {
        g.beginPath();
        for (var c = 0; c < 3; c++) {
          var pt = movedPoint(st, t.m, t.t[i + c * 3] / q, t.t[i + c * 3 + 1] / q, t.t[i + c * 3 + 2] / q);
          if (c === 0) g.moveTo(pt[0] * sx, pt[1] * sy);
          else g.lineTo(pt[0] * sx, pt[1] * sy);
        }
        g.closePath();
        g.fill();
      }
    }
    st.redraw();
  }

  function letGo(view, st) {
    if (!st.held) return;
    var wait = Math.max(0, 140 - (Date.now() - st.held.since));
    var held = st.held;
    setTimeout(function () {
      if (st.held !== held) return;
      st.held = null;
      var fx = view.dev.fx;
      fx.getContext('2d').clearRect(0, 0, fx.width, fx.height);
      st.redraw();
    }, wait);
  }

  /* Taps on the device: its switches pressed and let go, a page under the finger followed. */
  function touchForm(view, st) {
    var el = view.dev.img;
    var down = null;
    var lastMove = 0;
    el.addEventListener('pointerdown', function (e) {
      e.preventDefault();
      try { el.setPointerCapture(e.pointerId); } catch (x) { /* an old browser */ }
      var at = onFace(view, st, e);
      var hit = pressedNear(view, st, at);
      down = hit;
      if (!hit) return;
      if (hit.page !== undefined) {
        send({ do: 'page', screen: view.id, page: hit.page, kind: 'down', u: hit.u, v: hit.v });
        return;
      }
      holdMark(view, st, hit);
      send({ do: 'touch', screen: view.id, touch: hit.touch, down: true });
    });
    el.addEventListener('pointermove', function (e) {
      if (!down || down.page === undefined || e.timeStamp - lastMove < 50) return;
      lastMove = e.timeStamp;
      var at = onFace(view, st, e);
      var hit = pressedAt(st, at.x, at.y);
      if (hit && hit.page === down.page) {
        down.u = hit.u;
        down.v = hit.v;
      }
      send({ do: 'page', screen: view.id, page: down.page, kind: 'move', u: down.u, v: down.v });
    });
    var up = function () {
      if (!down) return;
      var was = down;
      down = null;
      if (was.page !== undefined) {
        send({ do: 'page', screen: view.id, page: was.page, kind: 'up', u: was.u, v: was.v });
        return;
      }
      letGo(view, st);
      send({ do: 'touch', screen: view.id, touch: was.touch, down: false });
    };
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
  }

  /* The pictures of a pane: each request waits for one newer than the last, so they come at
     the pace this device takes them in, and stop when the screen is closed. */
  function pictures(view, p, path) {
    var seq = 0;
    var next = function () {
      if (shown !== view) return;
      call(path + '?screen=' + encodeURIComponent(view.id) + '&after=' + seq)
        .then(function (r) {
          if (shown !== view) return null;
          if (r.status === 200) {
            seq = Number(r.headers.get('X-Seq')) || seq + 1;
            return r.blob().then(function (b) {
              if (shown !== view) return;
              var url = URL.createObjectURL(b);
              var old = p.url;
              p.url = url;
              p.img.onload = function () {
                if (old) URL.revokeObjectURL(old);
                p.waiting.hidden = true;
                p.img.classList.add('on');
                if (!p.aspect && p.img.naturalHeight) {
                  p.aspect = p.img.naturalWidth / p.img.naturalHeight;
                }
                layoutScreen();
              };
              p.img.src = url;
            });
          }
          if (r.status === 204) return null;
          return sleep(1000);
        })
        .catch(function () { return sleep(1000); })
        .then(next);
    };
    next();
  }

  // ------------------------------------------------------------------ start

  applySize();
  wasWide = wide();
  render(true);
  if (pairParam && !key) {
    var code = pairParam;
    pairParam = null;
    pair(code);
  }
  loadTexts().then(poll);
})();
