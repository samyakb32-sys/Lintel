/* METEOR V1 design-language preview. A classic script: no build step, runs from file://.
   The look comes from ../src/ui/tokens.css and components.css; this file fills in the token
   read-outs and sample content, drives the island mock and draws the mascot candidates. */
(function () {
  'use strict';

  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
  const css = getComputedStyle(document.documentElement);
  const tok = name => css.getPropertyValue(name).trim();
  const nums = name => tok(name).match(/-?\d*\.?\d+/g).map(Number);
  const REDUCED = matchMedia('(prefers-reduced-motion: reduce)');
  const TAU = Math.PI * 2;
  const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
  const lerp = (a, b, t) => a + (b - a) * t;
  const cap = s => s[0].toUpperCase() + s.slice(1);

  /* ── Icons: 24×24, one path, stroke 1.75, round caps, currentColor (§7) ── */

  const ICON = {
    home: 'M4 11l8-7 8 7v8a1 1 0 0 1-1 1h-4v-6h-6v6H5a1 1 0 0 1-1-1z',
    chat: 'M5 5h14a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-7l-4.5 3.5V16H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z',
    shelf: 'M4 13l2.5-7h11L20 13v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1zM4 13h4.5a3.5 3.5 0 0 0 7 0H20',
    music: 'M9 18V6l10-2v12M9 18a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0zM19 16a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0z',
    sliders: 'M4 7h9M17 7h3M4 12h3M11 12h9M4 17h11M19 17h1M15 5v4M9 10v4M17 15v4',
    grid: 'M5 5h5v5H5zM14 5h5v5h-5zM5 14h5v5H5zM14 14h5v5h-5z',
    gear: 'M12 3l7.8 4.5v9L12 21l-7.8-4.5v-9zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
    bell: 'M6 16v-5a6 6 0 0 1 12 0v5l1.5 2h-15zM10 20.5a2.2 2.2 0 0 0 4 0',
    close: 'M6 6l12 12M18 6L6 18',
    checkc: 'M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17zM8.2 12.4l2.6 2.6 5-5.4',
    question: 'M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17zM9.6 9.6a2.5 2.5 0 1 1 3.6 2.2c-.8.5-1.2 1-1.2 1.9M12 16.6h.01',
    alert: 'M12 4l9 15.5H3zM12 10v4.5M12 17h.01',
    play: 'M8 5.5v13l11-6.5z',
    pause: 'M8.5 5.5v13M15.5 5.5v13',
    next: 'M6 5.5v13l10-6.5zM18.5 5.5v13',
    prev: 'M18 5.5v13L8 12zM5.5 5.5v13',
    shuffle: 'M4 7h3c4 0 6 10 10 10h3M4 17h3c1.4 0 2.5-1.2 3.5-2.8M13.5 9.8C14.5 8.2 15.6 7 17 7h3M17.5 4.5L20 7l-2.5 2.5M17.5 14.5L20 17l-2.5 2.5',
    repeat: 'M17 3.5l3 3-3 3M4 11.5v-2a3 3 0 0 1 3-3h13M7 20.5l-3-3 3-3M20 12.5v2a3 3 0 0 1-3 3H4',
    volume: 'M4 9.5h3.5L12 6v12l-4.5-3.5H4zM15.5 9a4.2 4.2 0 0 1 0 6M18 6.5a8 8 0 0 1 0 11',
    mute: 'M4 9.5h3.5L12 6v12l-4.5-3.5H4zM16 9.5l5 5M21 9.5l-5 5',
    download: 'M12 4v11M7.5 10.5L12 15l4.5-4.5M5 19.5h14',
    timer: 'M12 7a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM12 10.5V14l2.2 1.6M9.5 3.5h5',
    clipboard: 'M9 4.5h6v3H9zM9 6H6.5a1 1 0 0 0-1 1v12.5a1 1 0 0 0 1 1h11a1 1 0 0 0 1-1V7a1 1 0 0 0-1-1H15',
    file: 'M7 3.5h7l4.5 4.5v11.5a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1zM14 3.5V8h4.5',
    image: 'M5 5h14a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zM4 16l4.5-4.5 4 4 2.5-2.5L20 17M15.5 9.5h.01',
    link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1.2 1.2M14 10a4 4 0 0 0-5.7 0l-3 3A4 4 0 0 0 11 18.7l1.2-1.2',
    code: 'M8.5 7.5L4 12l4.5 4.5M15.5 7.5L20 12l-4.5 4.5M13.5 5l-3 14',
    text: 'M5 6.5h14M5 11h14M5 15.5h9',
    search: 'M11 4.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM16 16l4 4',
    wifi: 'M3.5 9.5a12.5 12.5 0 0 1 17 0M6.5 13a8 8 0 0 1 11 0M9.5 16.3a3.8 3.8 0 0 1 5 0M12 19.5h.01',
    bluetooth: 'M7 8l10 8-5 4V4l5 4-10 8',
    moon: 'M19.5 14A8 8 0 0 1 10 4.5a8 8 0 1 0 9.5 9.5z',
    sun: 'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6L7 7M17 17l1.4 1.4M5.6 18.4L7 17M17 7l1.4-1.4',
    coffee: 'M5 9h11v5a5 5 0 0 1-5 5h-1a5 5 0 0 1-5-5zM16 10h1.5a2.5 2.5 0 0 1 0 5H16M8 3.5V6M12 3.5V6',
    record: 'M12 5a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM12 9.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z',
    shot: 'M4 8V5.5A1.5 1.5 0 0 1 5.5 4H8M16 4h2.5A1.5 1.5 0 0 1 20 5.5V8M20 16v2.5a1.5 1.5 0 0 1-1.5 1.5H16M8 20H5.5A1.5 1.5 0 0 1 4 18.5V16M9 12a3 3 0 1 0 6 0 3 3 0 0 0-6 0z',
    drop: 'M12 3.5c3.5 4 6 6.8 6 10a6 6 0 0 1-12 0c0-3.2 2.5-6 6-10z',
    mic: 'M12 4a3 3 0 0 0-3 3v4a3 3 0 0 0 6 0V7a3 3 0 0 0-3-3zM6 11a6 6 0 0 0 12 0M12 17v3.5',
    lock: 'M7 11V8a5 5 0 0 1 10 0v3M6 11h12a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1z',
    monitor: 'M4 5h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zM9 20.5h6M12 17v3.5',
    plane: 'M12 3.5c1 0 1.5 1 1.5 2.5V10l7 4v2l-7-2v4l2 1.5V21L12 20l-3.5 1v-1.5l2-1.5v-4l-7 2v-2l7-4V6c0-1.5.5-2.5 1.5-2.5z',
    send: 'M12 19V5M6 11l6-6 6 6',
    up: 'M12 19V5M6 11l6-6 6 6',
    share: 'M12 15V4M8 7.5L12 4l4 3.5M6 11.5v7a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1v-7',
    zip: 'M4 5h16v4H4zM5.5 9v9.5a1 1 0 0 0 1 1h11a1 1 0 0 0 1-1V9M10 13h4',
    trash: 'M5 7h14M9.5 7V4.5h5V7M7 7l.8 12.5a1 1 0 0 0 1 1h6.4a1 1 0 0 0 1-1L17 7',
    pin: 'M9 4h6l-1 6 3 3H7l3-3zM12 13v7',
    plus: 'M12 5v14M5 12h14',
    chevd: 'M6.5 9.5L12 15l5.5-5.5',
    terminal: 'M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zM7 10l3 2.5L7 15M12.5 15H17',
    sparkle: 'M12 4l1.8 5.2L19 11l-5.2 1.8L12 18l-1.8-5.2L5 11l5.2-1.8z',
    keyboard: 'M4 7h16a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1zM7 11h1M11.5 11h1M16 11h1M8 14h8',
    shield: 'M12 3.5l7 2.5v5.5c0 4.5-3 7.5-7 9-4-1.5-7-4.5-7-9V6z',
    info: 'M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17zM12 11v5.5M12 7.8h.01',
    island: 'M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zM8.5 5v1.5a2 2 0 0 0 2 2h3a2 2 0 0 0 2-2V5',
    replay: 'M5 12a7 7 0 1 0 2.2-5.1L5 8.5M5 4.5v4h4',
    copy: 'M9 9h10a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1zM5 15V5a1 1 0 0 1 1-1h9',
  };
  const icon = (name, size = 14, cls = '') =>
    `<svg${cls ? ` class="${cls}"` : ''} width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${ICON[name]}"/></svg>`;
  $$('i[data-i]').forEach(el => { el.outerHTML = icon(el.dataset.i, +el.dataset.s || 14, el.className); });

  /* ── Colour maths ─────────────────────────────────────────────────────── */

  const hex = h => {
    h = h.replace('#', '');
    if (h.length === 3) h = h.replace(/./g, '$&$&');
    const n = parseInt(h, 16);
    return [n >> 16, (n >> 8) & 255, n & 255];
  };
  const mix = (a, b, t) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
  const rgba = (c, a = 1) => `rgba(${c[0] | 0},${c[1] | 0},${c[2] | 0},${a})`;
  const lum = c => {
    const [r, g, b] = c.map(v => ((v /= 255) <= .03928 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4));
    return .2126 * r + .7152 * g + .0722 * b;
  };
  const ratio = (a, b) => { const x = lum(a) + .05, y = lum(b) + .05; return x > y ? x / y : y / x; };

  /* ── Motion maths: the tokens' curves and the growth spring ───────────── */

  const bez = (x1, y1, x2, y2) => {
    const f = (a, b, t) => 3 * (1 - t) * (1 - t) * t * a + 3 * (1 - t) * t * t * b + t * t * t;
    return x => {
      let lo = 0, hi = 1, t = x;
      for (let i = 0; i < 14; i++) { if (f(x1, x2, t) < x) lo = t; else hi = t; t = (lo + hi) / 2; }
      return f(y1, y2, t);
    };
  };
  const EASE = { out: bez(...nums('--ease-out')), inOut: bez(...nums('--ease-in-out')) };
  const D = { base: nums('--d-base')[0], slow: nums('--d-slow')[0], exit: nums('--d-exit')[0] };
  const SPRING = { response: .5, damping: .72 };

  /* A value that grows on the spring and shrinks on --d-slow --ease-in-out (§7).
     With reduced motion both become --d-base ease-out. */
  class Tracked {
    constructor(v) { this.v = this.to = v; this.vel = 0; this.mode = 0; }
    go(to) {
      if (to === this.to) return;
      this.to = to;
      if (REDUCED.matches || to < this.v) { this.mode = 2; this.from = this.v; this.t0 = performance.now(); this.vel = 0; }
      else this.mode = 1;
    }
    step(dt, now) {
      if (this.mode === 1) {
        const om = TAU / SPRING.response, n = Math.max(1, Math.ceil(dt * 240)), h = dt / n;
        for (let i = 0; i < n; i++) {
          this.vel += (om * om * (this.to - this.v) - 2 * SPRING.damping * om * this.vel) * h;
          this.v += this.vel * h;
        }
        if (Math.abs(this.to - this.v) < .05 && Math.abs(this.vel) < .5) { this.v = this.to; this.vel = 0; this.mode = 0; }
      } else if (this.mode === 2) {
        const red = REDUCED.matches, p = clamp((now - this.t0) / (red ? D.base : D.slow), 0, 1);
        this.v = lerp(this.from, this.to, (red ? EASE.out : EASE.inOut)(p));
        if (p >= 1) this.mode = 0;
      }
      return this.mode !== 0;
    }
  }

  const loops = new Set();           // per-frame callbacks; return false to stop
  /* Animates an element's width and height: the island shell, the one exception to
     "transform and opacity only". */
  function shell(el, w, h, done) {
    const W = new Tracked(w), H = new Tracked(h);
    const step = (dt, now) => {
      const a = W.step(dt, now), b = H.step(dt, now);
      el.style.width = W.v + 'px';
      el.style.height = Math.max(0, H.v) + 'px';
      if (!a && !b) { if (done) done(); return false; }
    };
    return (nw, nh) => { W.go(nw); H.go(nh); loops.add(step); };
  }

  /* ── 01 Foundations ───────────────────────────────────────────────────── */

  const COLOURS = [
    ['Surfaces', [['--bg', 'Island body'], ['--panel', 'Windows, settings'], ['--card', 'Cards, fields, tiles'], ['--card-2', 'Hover, tab on']]],
    ['Ink', [['--ink', 'Primary text'], ['--ink-2', 'Secondary text'], ['--dim', 'Hints, labels'], ['--dim-2', 'Placeholders, metadata']]],
    ['Accent, lines', [['--accent', 'Primary action, focus, selection'], ['--accent-ink', 'Text on accent'], ['--hairline', 'Borders, separators'], ['--outline', 'Optional island outline']]],
    ['Status', [['--ok', 'On, finished'], ['--warn', 'Approval, active timer'], ['--bad', 'Error, recording, destructive'], ['--info', 'Question']]],
  ];
  $('#swatches').innerHTML = COLOURS.map(([group, list]) =>
    `<div class="dl-swgroup"><span class="dl-cap">${group}</span>${list.map(([n, use]) =>
      `<div class="dl-sw"><i style="background:var(${n})"></i><code>${n}</code><span>${tok(n)}</span><em>${use}</em></div>`).join('')}</div>`).join('');

  const SURFACES = ['--bg', '--panel', '--card', '--card-2'];
  const INKS = ['--ink', '--ink-2', '--dim', '--dim-2', '--accent', '--ok', '--warn', '--bad', '--info'];
  $('#contrast').innerHTML = '<code></code>' + INKS.map(n => `<code>${n.slice(2)}</code>`).join('') +
    SURFACES.map(s => `<code>${s.slice(2)}</code>` + INKS.map(n => {
      const r = ratio(hex(tok(s)), hex(tok(n)));
      return `<span class="${r < 4.5 ? 'low' : ''}" style="background:var(${s});color:var(${n})" title="${n} on ${s}">Aa<small>${r.toFixed(1)}</small></span>`;
    }).join('')).join('');

  // A sample of the pill catalogue (src/core/pills.ts); the mascot takes these colours.
  const NEUTRAL = '#2a2d35';
  const PILLS = [['Neutral', ''], ['Claude Code', '#E07950'], ['Codex', '#2DD4BF'], ['Gemini CLI', '#8AB4F8'], ['OpenCode', '#4ADE80'], ['Copilot CLI', '#818CF8'], ['Amp', '#F59E0B'], ['VS Code', '#F5F6F8']];
  $('#pills').innerHTML = PILLS.slice(1).map(([n, c]) => `<span><i style="--c:${c}"></i>${n}<code>${c.toLowerCase()}</code></span>`).join('');

  const scale = n => tok(n).replace(/px/g, '').replace('/', ' / ');
  const TYPES = [
    ['--t-xl', 600, 'Hidden until needed', 'Page titles, big readouts'],
    ['--t-lg', 600, 'Codex wants to run a command', 'Card and view titles'],
    ['--t-md', 400, 'The island slides out of the top edge when something needs you.', 'Default: body and controls'],
    ['--t-sm', 400, 'Rest the pointer on the strip for 250 ms to open it.', 'Strip text, hints, small buttons'],
    ['--t-xs', 500, 'Keep awake · 2 min ago · 61.4 MB', 'Tile labels, metadata'],
  ];
  $('#types').innerHTML = TYPES.map(([n, wt, text, use]) =>
    `<div class="dl-type"><code>${n}</code><small>${scale(n)}</small><p style="font:${wt} var(${n}) var(--font)">${text}</p><em>${use}</em></div>`).join('') +
    `<div class="dl-type"><code>caps</code><small>${scale('--t-xs')}</small><p class="dl-caps" style="font:600 var(--t-xs) var(--font);color:var(--dim)">Behaviour · Pinned · Now playing</p><em>All-caps labels only: ${tok('--caps')}</em></div>` +
    `<div class="dl-type"><code>weights</code><small>${scale('--t-md')}</small><p>Body 400 · <span style="font-weight:500">Label 500</span> · <span style="font-weight:600">Title 600</span></p><em>Letter-spacing 0</em></div>` +
    `<div class="dl-type"><code>--mono</code><small>${scale('--t-sm')}</small><p style="font:400 var(--t-sm) var(--mono)">cargo test -p svc-audio -- --nocapture</p><em>Commands, paths, token names</em></div>` +
    `<div class="dl-type"><code>tnum</code><small>${scale('--t-md')}</small><p class="dl-tnum">18:42 · 1:47 / 4:12 · 62 % · 61.4 MB</p><em>Readouts use tabular numerals</em></div>`;

  $('#spacing').innerHTML = ['--s-1', '--s-2', '--s-3', '--s-4', '--s-5'].map(n =>
    `<span><i style="width:var(${n})"></i><code>${n}</code><small>${parseFloat(tok(n))}</small></span>`).join('');
  $('#radii').innerHTML = [['--r-xs', 'kbd, meter'], ['--r-sm', 'buttons, fields, tiles'], ['--r-win', 'floating windows'], ['--r-md', 'cards, peek strip'], ['--r-lg', 'open island'], ['--r-pill', 'capsules, switches']].map(([n, use]) =>
    `<span title="${use}"><i style="border-radius:var(${n})"></i><code>${n.slice(2)}</code><small>${parseFloat(tok(n))}</small></span>`).join('');

  /* Motion: every curve at every duration, plotted from the token values. */
  function plot(fn) {
    const S = 96, P = 12, L = S - 2 * P, over = 1.14, pts = [];
    for (let i = 0; i <= 48; i++) pts.push((P + i / 48 * L).toFixed(1) + ',' + (S - P - fn(i / 48) * L / over).toFixed(1));
    const top = (S - P - L / over).toFixed(1);
    return `<svg width="${S}" height="${S}" viewBox="0 0 ${S} ${S}" aria-hidden="true"><path class="grid" fill="none" d="M${P} ${top}H${S - P}M${P} ${S - P}H${S - P}"/><polyline class="line" points="${pts.join(' ')}"/></svg>`;
  }
  const USE = {
    '--ease-out--d-fast': 'Hover, notice in', '--ease-out--d-base': 'Content cross-fade',
    '--ease-in-out--d-exit': 'Notice out', '--ease-in-out--d-slow': 'Island shrink, window close',
  };
  $('#motion').innerHTML = [['--ease-out', EASE.out], ['--ease-in-out', EASE.inOut]].map(([ease, fn]) => `
    <article class="dl-spec">
      <div class="dl-spec-h"><h4>${ease}</h4><code>${tok(ease)}</code></div>
      <div class="dl-ease"><div class="dl-curve">${plot(fn)}</div><div class="dl-runs">${['--d-fast', '--d-base', '--d-exit', '--d-slow'].map(dur => `
        <div class="dl-run-row"><span class="dl-run-name"><code>${dur}</code>${parseFloat(tok(dur))} ms${USE[ease + dur] ? `<small>${USE[ease + dur]}</small>` : ''}</span>
          <div class="dl-track" style="--dur:var(${dur});--ease:var(${ease})"><span><i></i></span></div>
          <button class="btn sm ghost icon" title="Replay" data-run>${icon('replay')}</button></div>`).join('')}
      </div></div>
    </article>`).join('');

  const om = TAU / SPRING.response, z = SPRING.damping, od = om * Math.sqrt(1 - z * z);
  $('#spring-plot').innerHTML = plot(x => 1 - Math.exp(-z * om * x) * (Math.cos(od * x) + z * om / od * Math.sin(od * x)));

  const springDot = new Tracked(0), springRun = $('#spring-track > span');
  const springStep = (dt, now) => { const on = springDot.step(dt, now); springRun.style.transform = `translateX(${springDot.v * 100}%)`; return on; };
  const springPlay = () => { springDot.go(springDot.to ? 0 : 1); loops.add(springStep); };
  $('#spring-replay').addEventListener('click', springPlay);

  const growTo = shell($('#grow-shell'), 288, 32);
  let grown = false;
  function growPlay() {
    grown = !grown;
    growTo(grown ? 640 : 288, grown ? 104 : 32);
    $('#grow-btn').textContent = grown ? 'Shrink' : 'Grow';
    $('#grow-note').textContent = grown ? '640 × 104, grown on the spring' : '288 × 32, shrunk on --d-slow --ease-in-out';
  }
  $('#grow-btn').addEventListener('click', growPlay);
  $('#replay-all').addEventListener('click', () => { $$('.dl-track[style]').forEach(t => t.classList.toggle('go')); springPlay(); growPlay(); });

  /* ── Mascot candidates (Canvas 2D) ────────────────────────────────────── */
  /* Mirrors the §9 engine surface: setState, triggerEmote, bodyColor, lookX/lookY,
     update(dt), draw(ctx, x, y, d), busy. Three original silhouettes, none of them the
     reserved squircle: A an orb with a visor, B an open ring with a core, C a flame. */

  const STATES = ['idle', 'working', 'thinking', 'approval', 'question', 'error', 'finished', 'sleeping', 'listening', 'talking'];   // the last two follow a sound level (§7.6): here a made-up one
  const EMOTES = { love: 1.8, surprised: 1.2, proud: 1.9, wink: 1.2, happy: 1.4 };   // seconds
  const KEYS = STATES.concat(Object.keys(EMOTES));
  const WHITE = [255, 255, 255], BLACK = hex(tok('--bg')), GLASS = mix(BLACK, hex(tok('--panel')), .6);
  const TONE = { approval: hex(tok('--warn')), question: hex(tok('--info')), error: hex(tok('--bad')), finished: hex(tok('--ok')) };
  TONE.love = mix(TONE.error, WHITE, .4);
  TONE.listening = hex(tok('--accent'));
  // B's ring opening per state, in radians (the round caps close about 0.37 of it)
  const GAP = { idle: 1.25, working: 1.9, thinking: 1.25, approval: 0, question: 1.7, error: 0, finished: 0, sleeping: 1.25, love: 0, surprised: .8, proud: 0, wink: 1.25, happy: 0 };

  function heart(ctx, x, y, s) {
    ctx.beginPath();
    ctx.moveTo(x, y + s * .9);
    ctx.bezierCurveTo(x - s * 1.5, y - s * .1, x - s * .75, y - s * 1.2, x, y - s * .4);
    ctx.bezierCurveTo(x + s * .75, y - s * 1.2, x + s * 1.5, y - s * .1, x, y + s * .9);
    ctx.fill();
  }
  function spark(ctx, x, y, s) {
    ctx.beginPath();
    ctx.moveTo(x, y - s);
    ctx.quadraticCurveTo(x, y, x + s, y); ctx.quadraticCurveTo(x, y, x, y + s);
    ctx.quadraticCurveTo(x, y, x - s, y); ctx.quadraticCurveTo(x, y, x, y - s);
    ctx.fill();
  }
  /* A flame: a round base at (cx, cy) radius R, drawn up to a tip at (tx, ty); k hooks the tip. */
  function flame(ctx, cx, cy, R, tx, ty, k) {
    const H = cy - ty, s = Math.min(R * 1.3, H * .72);
    ctx.beginPath();
    ctx.moveTo(tx, ty);
    ctx.bezierCurveTo(tx + R * (.16 - k * .8), ty + H * .44, cx + R, cy - s, cx + R, cy);
    ctx.arc(cx, cy, R, 0, Math.PI);
    ctx.bezierCurveTo(cx - R, cy - s, tx - R * (.16 + k), ty + H * .38, tx, ty);
    ctx.closePath();
  }
  const shade = (ctx, body, y0, y1) => {
    const g = ctx.createLinearGradient(0, y0, 0, y1);
    g.addColorStop(0, rgba(mix(body, WHITE, .3)));
    g.addColorStop(.55, rgba(body));
    g.addColorStop(1, rgba(mix(body, BLACK, .32)));
    return g;
  };
  const glow = (ctx, tint, d) => {
    ctx.fillStyle = ctx.strokeStyle = rgba(tint);
    ctx.shadowColor = rgba(tint, .75);
    ctx.shadowBlur = Math.max(2, d * .12);
  };

  const DRAW = {
    /* A · Visor: an orb, one dark band, one bar of light. */
    A(ctx, m, d, body, tint) {
      const w = m.w, t = m.t, r = d * .46, th = Math.max(1.5, d * .074);
      ctx.fillStyle = shade(ctx, body, -r, r);
      ctx.beginPath(); ctx.arc(0, 0, r, 0, TAU); ctx.fill();
      if (d > 24) {           // a thin rim of light, top left
        ctx.strokeStyle = rgba(WHITE, .28); ctx.lineWidth = Math.max(1, d * .022);
        ctx.beginPath(); ctx.arc(0, 0, r - d * .03, -2.75, -1.2); ctx.stroke();
      }
      const vw = d * .7, vh = d * .28, vy = -d * .1;
      ctx.fillStyle = rgba(GLASS, .94);
      ctx.beginPath(); ctx.roundRect(-vw / 2, vy - vh / 2, vw, vh, vh / 2); ctx.fill();

      ctx.translate(m.lx * d * .07 * (1 - w.sleeping), vy + m.ly * d * .025);
      glow(ctx, tint, d);
      ctx.lineWidth = th;
      const bar = (x, y, wd, ht) => { ht = Math.max(ht, .7); ctx.beginPath(); ctx.roundRect(x - wd / 2, y - ht / 2, wd, ht, ht / 2); ctx.fill(); };
      const pulse = .5 + .5 * Math.sin(t * TAU / 1.2);
      let a = w.idle + w.sleeping + w.surprised + w.wink;
      if (a > .02) {          // the steady bar: blinks, widens when surprised, thins asleep, half-closes to wink
        const wd = d * (.3 * (a - w.surprised) + .5 * w.surprised) / a;
        const wk = w.wink * Math.sin(clamp(m.emoteT / .8, 0, 1) * Math.PI);
        ctx.globalAlpha = a * (1 - .5 * w.sleeping);
        bar(-wd * .28 * wk, 0, wd * (1 - .56 * wk), th * lerp(m.open, .32, w.sleeping) * (1 + .3 * w.surprised));
      }
      if (w.working > .02) for (let i = 3; i >= 0; i--) {      // scanning, with a trail
        ctx.globalAlpha = w.working * [1, .4, .18, .08][i];
        bar(Math.sin((t - i * .05) * TAU / 1.1) * d * .2, 0, d * .14, th);
      }
      if (w.thinking > .02) for (let i = 0; i < 3; i++) {      // three dots in turn
        const p = .5 + .5 * Math.sin(t * TAU / 1.2 - i * 1.1);
        ctx.globalAlpha = w.thinking * (.3 + .7 * p);
        ctx.beginPath(); ctx.arc((i - 1) * d * .14, -p * d * .015, th * .62, 0, TAU); ctx.fill();
      }
      if (w.approval > .02) { ctx.globalAlpha = w.approval * (.5 + .5 * pulse); bar(0, 0, d * .46, th * 1.3); }
      if (w.question > .02) {                                  // a short bar and a raised dot
        ctx.globalAlpha = w.question;
        bar(-d * .07, d * .012, d * .2, th);
        ctx.beginPath(); ctx.arc(d * .13, -d * .03 + Math.sin(t * 3) * d * .012, th * .62, 0, TAU); ctx.fill();
      }
      if (w.error > .02) {                                     // the bar snapped and crossed, flickering
        ctx.globalAlpha = w.error * (Math.sin(t * 31) > .8 ? .35 : 1);
        ctx.beginPath();
        ctx.moveTo(-d * .1, -d * .065); ctx.lineTo(d * .1, d * .065);
        ctx.moveTo(d * .1, -d * .065); ctx.lineTo(-d * .1, d * .065);
        ctx.stroke();
      }
      a = w.finished + w.happy + w.proud;
      if (a > .02) {                                           // an arc
        ctx.globalAlpha = a;
        ctx.beginPath(); ctx.moveTo(-d * .13, d * .04); ctx.quadraticCurveTo(0, -d * .12, d * .13, d * .04); ctx.stroke();
      }
      if (w.love > .02) { ctx.globalAlpha = w.love; heart(ctx, 0, 0, d * .085 * (1 + .12 * Math.sin(t * TAU / .6))); }
      const small = d < 30, snap = small ? Math.round : v => v, lv = REDUCED.matches ? .35 : m.lv;
      if (w.listening > .02) {   // five segments (three under 30 px) rising with the microphone, shimmering; silent: a breathing dashed bar
        const n = small ? 3 : 5, sw = snap(d * (small ? .1 : .06)), gap = snap(d * .03), x0 = snap(-(n * sw + (n - 1) * gap) / 2);
        const breath = REDUCED.matches ? 1 : 1 - .15 * (.5 + .5 * Math.sin(t * TAU / 1.2)) * clamp(1 - lv * 4, 0, 1);
        ctx.globalAlpha = w.listening;
        for (let i = 0; i < n; i++) {
          const sh = REDUCED.matches ? 1 : 1 + .15 * Math.sin(t * TAU * 9 + i * 2.1), ht = snap(Math.max(1, th * (.6 + 2.2 * lv * [.5, .8, 1, .8, .5][small ? i + 1 : i] * sh) * breath));
          ctx.beginPath(); ctx.roundRect(x0 + i * (sw + gap), snap(-ht / 2), sw, ht, sw / 2); ctx.fill();
        }
      }
      if (w.talking > .02) {     // one white bar, taller and narrower with the output, bouncing while it plays
        const wd = snap(d * .34 * (1 - .12 * lv)), ht = snap(Math.max(1, th * (.7 + 1.6 * lv)));
        const bounce = REDUCED.matches ? 0 : -d * .018 * Math.abs(Math.sin(t * Math.PI * 3.5)) * clamp((lv - .05) / .05, 0, 1);
        ctx.globalAlpha = w.talking;
        ctx.beginPath(); ctx.roundRect(snap(-wd / 2), snap(-ht / 2 + bounce), wd, ht, ht / 2); ctx.fill();
      }
    },

    /* B · Halo: an open ring that turns, closes and breaks; a diamond core that becomes the sign. */
    B(ctx, m, d, body, tint) {
      const w = m.w, t = m.t, e = m.emoteT, th = Math.max(1.5, d * .07);
      const R = d * .355 * (1 + .08 * w.surprised * Math.sin(clamp(e / .5, 0, 1) * Math.PI));
      const pulse = .5 + .5 * Math.sin(t * TAU / 1.2);
      ctx.strokeStyle = shade(ctx, body, -R, R);
      ctx.lineWidth = d * .13;
      const arcs = (n, gap, alpha, push) => {
        if (alpha < .02) return;
        ctx.globalAlpha = alpha * (1 - .45 * w.sleeping);
        for (let i = 0; i < n; i++) {
          const a0 = m.ang + i * TAU / n + gap / 2, a1 = m.ang + (i + 1) * TAU / n - gap / 2, mid = (a0 + a1) / 2;
          ctx.beginPath(); ctx.arc(Math.cos(mid) * push, Math.sin(mid) * push, R, a0, a1); ctx.stroke();
        }
      };
      let gap = 0;
      for (const k in GAP) gap += w[k] * GAP[k];
      arcs(1, gap, clamp(1 - w.approval - w.error, 0, 1), 0);
      arcs(2, 1.2, w.approval, d * .02 * pulse);                              // brackets around the core
      arcs(3, .8, w.error, d * (.03 + .015 * Math.sin(t * 23)));              // broken, drifting apart

      const wk = w.wink * Math.sin(clamp(e / .8, 0, 1) * Math.PI);
      const s = d * .125 * (1 + .1 * w.working * Math.sin(t * TAU / .55) + .16 * w.approval * pulse + .3 * w.surprised);
      ctx.translate(
        m.lx * d * .085 + w.thinking * Math.cos(t * TAU / 2.2) * d * .075,
        m.ly * d * .07 + w.thinking * Math.sin(t * TAU / 2.2) * d * .075 - w.question * d * (.3 + .02 * Math.sin(t * 3)) + w.sleeping * d * .11);
      glow(ctx, tint, d);
      if (w.approval > .02) {                                  // a ripple from the core
        const p = (t / 1.2) % 1;
        ctx.globalAlpha = w.approval * (1 - p) * .7; ctx.lineWidth = Math.max(1, d * .025);
        ctx.beginPath(); ctx.arc(0, 0, lerp(s, d * .25, p), 0, TAU); ctx.stroke();
      }
      const a = clamp(1 - w.error - w.finished - w.love, 0, 1) * (1 - .4 * w.sleeping);
      if (a > .02) {                                           // the diamond: blinks flat, winks, sinks asleep
        const sy = s * m.open * (1 - .5 * w.sleeping) * (1 - .88 * wk);
        ctx.globalAlpha = a; ctx.lineWidth = s * .5;
        ctx.beginPath(); ctx.moveTo(0, -sy); ctx.lineTo(s, 0); ctx.lineTo(0, sy); ctx.lineTo(-s, 0); ctx.closePath();
        ctx.fill(); ctx.stroke();
      }
      ctx.lineWidth = th;
      if (w.error > .02) {
        const q = d * .078;
        ctx.globalAlpha = w.error;
        ctx.beginPath(); ctx.moveTo(-q, -q); ctx.lineTo(q, q); ctx.moveTo(q, -q); ctx.lineTo(-q, q); ctx.stroke();
      }
      if (w.finished > .02) {
        ctx.globalAlpha = w.finished;
        ctx.beginPath(); ctx.moveTo(-d * .1, d * .005); ctx.lineTo(-d * .03, d * .07); ctx.lineTo(d * .1, -d * .065); ctx.stroke();
      }
      if (w.love > .02) { ctx.globalAlpha = w.love; heart(ctx, 0, 0, d * .1 * (1 + .12 * Math.sin(t * TAU / .6))); }
    },

    /* C · Ember: a flame with a dark heart and a spark in it; all body language. */
    C(ctx, m, d, body, tint) {
      const w = m.w, t = m.t, e = m.emoteT;
      const n1 = Math.sin(t * 9.1) * .5 + Math.sin(t * 14.7 + 1) * .3 + Math.sin(t * 23.3 + 2) * .2;
      const n2 = Math.sin(t * 7.3 + 4) * .6 + Math.sin(t * 17.9) * .4;
      const emo = w.love + w.surprised + w.proud + w.wink + w.happy;
      const flick = .3 * w.idle + w.working + .2 * w.thinking + .05 * w.approval + .3 * w.question + 1.4 * w.error + .15 * w.finished + .04 * w.sleeping + .5 * emo;
      const tall = w.idle + 1.04 * w.working + w.thinking + 1.06 * w.approval + w.question + .78 * w.error + w.finished + .64 * w.sleeping
        + w.love + 1.18 * w.surprised + 1.08 * w.proud + w.wink + 1.04 * w.happy;
      const wk = w.wink * Math.sin(clamp(e / .6, 0, 1) * Math.PI);
      const sway = Math.sin(t * TAU / 2.6);
      const lean = m.lx * .11 * (1 - w.sleeping) + w.thinking * sway * .1 + n1 * .035 * flick + w.question * .1 + wk * .2;
      const curl = w.question * .9 + w.thinking * .25 * sway + wk * .5;
      const foot = d * .47, rise = d * .96 * (tall + n2 * .04 * flick);
      const R = Math.min(d * .34, rise * .37), by = foot - R, tx = lean * d, ty = foot - rise;
      ctx.fillStyle = shade(ctx, body, ty, foot);
      flame(ctx, 0, by, R, tx, ty, curl); ctx.fill();

      const hr = R * .6, hy = by + R * .16;
      ctx.fillStyle = rgba(GLASS, .94);
      flame(ctx, 0, hy, hr, tx * .5, lerp(hy, ty, .54), curl * .6); ctx.fill();

      const pulse = .5 + .5 * Math.sin(t * TAU / 1.2);
      const er = hr * (.46 + .12 * w.approval * pulse + .14 * w.surprised - .1 * w.sleeping);
      const ex = m.lx * hr * .28, ey = hy + hr * .24 + m.ly * hr * .2;
      glow(ctx, tint, d);
      const a = (1 - w.love) * (1 - .5 * w.sleeping) * (w.error > .5 && Math.sin(t * 29) > .7 ? .4 : 1);
      if (a > .02) {                                           // the spark: blinks flat, swells while it waits
        ctx.globalAlpha = a;
        flame(ctx, ex, ey, er, ex + tx * .22, ey - er * (1 + 1.5 * m.open * (1 - .6 * w.sleeping)), curl * .4); ctx.fill();
      }
      if (w.love > .02) { ctx.globalAlpha = w.love; heart(ctx, 0, hy, hr * .56 * (1 + .12 * Math.sin(t * TAU / .6))); }
      if (w.finished > .02) {                                  // a calm ring breathing out of the spark
        const p = (t / 2.2) % 1;
        ctx.globalAlpha = w.finished * (1 - p) * .6; ctx.lineWidth = Math.max(1, d * .022);
        ctx.beginPath(); ctx.arc(ex, ey, lerp(er, hr, p), 0, TAU); ctx.stroke();
      }
      ctx.shadowBlur = 0;
      if (w.working > .02) for (let i = 0; i < 3; i++) {      // sparks rising off the tip
        const p = (t * .85 + i / 3) % 1;
        ctx.globalAlpha = w.working * (1 - p);
        ctx.fillStyle = rgba(mix(body, WHITE, .45));
        ctx.beginPath(); ctx.arc(tx + Math.sin(i * 2.4 + p * 5) * d * .13, ty + d * .14 - p * d * .3, Math.max(.8, d * .028 * (1 - p * .5)), 0, TAU); ctx.fill();
      }
    },

    /* Mascot hidden (§9): a 10 px dot in the pill colour with a 2 px state ring. */
    dot(ctx, m, d, body, tint) {
      const w = m.w, t = m.t, ring = w.approval + w.question + w.error + w.finished, busy = w.working + w.thinking;
      ctx.globalAlpha = 1 - .5 * w.sleeping;
      ctx.fillStyle = rgba(body);
      ctx.beginPath(); ctx.arc(0, 0, 5, 0, TAU); ctx.fill();
      ctx.lineWidth = 2;
      const lis = w.listening, tal = w.talking;
      if (lis > .02) {         // --accent, swelling with the level (whole pixels), an arc turning
        ctx.globalAlpha = lis * (.45 + .55 * m.lv);
        ctx.strokeStyle = rgba(TONE.listening);
        ctx.beginPath(); ctx.arc(0, 0, Math.floor(8 + 3 * m.lv), 0, TAU); ctx.stroke();
        const a0 = REDUCED.matches ? 0 : t * TAU / 1.4;
        ctx.globalAlpha = lis;
        ctx.beginPath(); ctx.arc(0, 0, 8, a0, a0 + TAU * .25); ctx.stroke();
      }
      if (tal > .02) {         // --ink, flickering with what is played
        ctx.globalAlpha = tal * (.35 + .65 * m.lv);
        ctx.strokeStyle = rgba(WHITE);
        ctx.beginPath(); ctx.arc(0, 0, Math.floor(8 + 3 * m.lv), 0, TAU); ctx.stroke();
      }
      if (ring > .02) {
        ctx.globalAlpha = ring * (1 - .4 * w.approval * (.5 + .5 * Math.sin(t * TAU / 1.2)));
        ctx.strokeStyle = rgba(tint);
        ctx.beginPath(); ctx.arc(0, 0, 8, 0, TAU); ctx.stroke();
      }
      if (busy > .02) {       // not in §9: a turning arc in the pill colour while working or thinking
        const a0 = t * TAU / lerp(1.1, 2.6, w.thinking);
        ctx.globalAlpha = busy * .8;
        ctx.strokeStyle = rgba(body);
        ctx.beginPath(); ctx.arc(0, 0, 8, a0, a0 + TAU * lerp(.7, .3, w.thinking)); ctx.stroke();
      }
    },
  };

  class Mascot {
    constructor(kind, state, color) {
      this.kind = kind; this.state = state || 'idle'; this.bodyColor = color || null;
      this.emote = null; this.emoteT = 0;
      this.lookX = this.lookY = this.lx = this.ly = 0;
      this.t = Math.random() * 20; this.blinkIn = 1 + Math.random() * 4; this.blinkT = 0; this.open = 1;
      this.ang = -Math.PI / 4; this.shakeT = 9;
      this.w = {}; this.lv = 0;
      for (const k of KEYS) this.w[k] = k === this.state ? 1 : 0;
    }
    get busy() { return !!this.emote; }
    setState(s) { if (s === this.state) return; this.state = s; if (s === 'error') this.shakeT = 0; }
    triggerEmote(e) { this.emote = e; this.emoteT = 0; }
    update(dt) {
      const w = this.w, k = 1 - Math.exp(-dt * 11), still = REDUCED.matches, now = this.emote || this.state;
      if (!still) this.t += dt;
      if (this.emote && (this.emoteT += dt) > EMOTES[this.emote]) this.emote = null;
      for (const n of KEYS) w[n] += ((n === now ? 1 : 0) - w[n]) * k;     // faces cross-fade
      // a made-up sound level: syllables of speech with pauses, so the faces have something to follow
      const want = this.state === 'listening' || this.state === 'talking' ? clamp(Math.abs(Math.sin(this.t * 7.3)) * (.5 + .5 * Math.sin(this.t * 1.9)) * 1.3 - .1, 0, 1) : 0;
      this.lv += (want - this.lv) * (1 - Math.exp(-dt / (want > this.lv ? .06 : .18)));
      this.lx += (this.lookX - this.lx) * k;
      this.ly += (this.lookY - this.ly) * k;
      this.shakeT += dt;
      if (!still && (this.blinkIn -= dt) < 0) { this.blinkIn = 3 + Math.random() * 3; this.blinkT = .16; }
      this.blinkT -= dt;
      this.open += ((this.blinkT > 0 ? .1 : 1) - this.open) * (1 - Math.exp(-dt * 50));
      // B's ring: spins while working, drifts while thinking, otherwise settles on a rest angle
      const rest = now === 'question' || now === 'approval' ? -Math.PI / 2 : now === 'sleeping' ? -Math.PI * .75 : -Math.PI / 4;
      if (!still) this.ang += (w.working * TAU / 1.1 - w.thinking * TAU / 7) * dt;
      const turn = ((rest - this.ang + Math.PI) % TAU + TAU) % TAU - Math.PI;
      this.ang += turn * clamp(1 - w.working - w.thinking, 0, 1) * (1 - Math.exp(-dt * 5));
    }
    draw(ctx, x, y, d) {
      const w = this.w, t = this.t, u = d / 58, e = this.emoteT, M = REDUCED.matches ? 0 : 1;
      const body = hex(this.bodyColor || NEUTRAL);
      let tint = WHITE;
      for (const n in TONE) tint = mix(tint, TONE[n], w[n]);
      const dot = this.kind === 'dot';
      const hop = dot ? 0 : M * u * (w.surprised * 6 * Math.sin(clamp(e / .42, 0, 1) * Math.PI) + w.happy * 4 * Math.abs(Math.sin(clamp(e / .84, 0, 1) * TAU)));
      const shake = M * (this.shakeT < .4 ? Math.sin(this.shakeT * TAU * 7.5) * 2 * u * (1 - this.shakeT / .4) : 0);
      const bob = dot ? 0 : M * Math.sin(t * TAU / 2.8) * 1.5 * u * (1 - w.sleeping);
      const breathe = 1 + .03 * w.sleeping * (.5 - .5 * Math.cos(t * TAU / 3));
      ctx.save();
      ctx.translate(x + shake, y + bob - hop);
      if (!dot) ctx.rotate((this.lx * 8 * (1 - w.sleeping) - w.proud * 6 - w.question * 6) * Math.PI / 180);
      ctx.scale(breathe, breathe);
      ctx.lineCap = ctx.lineJoin = 'round';
      DRAW[this.kind](ctx, this, d, body, tint);
      ctx.restore();
      if (dot) return;
      if (w.love > .02) for (let i = 0; i < 2; i++) {          // two hearts rising 24 px
        const p = clamp((e - i * .3) / 1.3, 0, 1);
        ctx.globalAlpha = w.love * Math.sin(p * Math.PI);
        ctx.fillStyle = rgba(TONE.love);
        heart(ctx, x + (i ? .26 : -.22) * d, y - .36 * d - p * 24 * u, Math.max(2, 3.5 * u));
      }
      if (w.proud > .02) {                                     // a four-point sparkle, top right
        ctx.globalAlpha = w.proud;
        ctx.fillStyle = rgba(WHITE);
        spark(ctx, x + .42 * d, y - .4 * d, Math.max(2.5, 7 * u) * (.75 + .25 * Math.sin(t * 6)));
      }
      ctx.globalAlpha = 1;
    }
  }

  /* Canvases are sized by devicePixelRatio and drawn only while on screen. Each is twice the
     mascot's box so hops, hearts and the sparkle have room. */
  const views = [];
  let islandKind = 'A', ptr = null, ptrMoved = false;
  const onScreen = new IntersectionObserver(es => es.forEach(en => { en.target._v.vis = en.isIntersecting; }));
  function mount(el) {
    if (el._v) return;
    const d = +el.dataset.d, c = document.createElement('canvas');
    el.style.width = el.style.height = d + 'px';
    c.style.cssText = `position:absolute;left:${-d / 2}px;top:${-d / 2}px;width:${d * 2}px;height:${d * 2}px;pointer-events:none`;
    el.append(c);
    const m = new Mascot(el.dataset.kind || islandKind, el.dataset.state, el.dataset.color);
    el._v = { el, c, ctx: c.getContext('2d'), m, d, vis: false };
    views.push(el._v);
    onScreen.observe(el);
    let love;                                                  // hover → love after 1.9 s (§9)
    el.addEventListener('pointerenter', () => { love = setTimeout(() => m.triggerEmote('love'), 1900); });
    el.addEventListener('pointerleave', () => clearTimeout(love));
  }
  const mountAll = (root = document) => $$('.mascot', root).forEach(mount);
  function drawMascots(dt) {
    const dpr = window.devicePixelRatio || 1;
    for (let i = views.length - 1; i >= 0; i--) {
      const v = views[i], size = v.d * 2, px = Math.round(size * dpr);
      if (!v.el.isConnected) { onScreen.unobserve(v.el); views.splice(i, 1); continue; }
      if (!v.vis) continue;
      if (v.c.width !== px) v.c.width = v.c.height = px;
      if (ptr && ptrMoved) {
        const r = v.el.getBoundingClientRect();
        v.m.lookX = clamp((ptr.x - r.left - v.d / 2) / 200, -1, 1);
        v.m.lookY = clamp((ptr.y - r.top - v.d / 2) / 200, -1, 1);
      }
      v.m.update(dt);
      v.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      v.ctx.clearRect(0, 0, size, size);
      v.m.draw(v.ctx, size / 2, size / 2, v.d);
    }
    ptrMoved = false;
  }
  addEventListener('pointermove', e => { ptr = { x: e.clientX, y: e.clientY }; ptrMoved = true; }, { passive: true });
  addEventListener('scroll', () => { ptrMoved = true; }, { passive: true });

  const CANDS = [
    ['A', 'Visor', 'An orb with one dark visor band. A single bar of light inside it is the whole face: it scans, blinks, crosses and bends.'],
    ['B', 'Halo', 'An open ring around a floating diamond. The ring turns, closes and breaks; the diamond becomes the sign.'],
    ['C', 'Ember', 'A flame with a dark heart and a spark in it. No face at all: it leans, flares, hooks and sinks.'],
    ['dot', 'Mascot hidden', 'Settings › Mascot off: every mascot becomes a 10 px dot in the pill colour with a 2 px state ring.'],
  ];
  let pillColor = PILLS[1][1];
  $('#m-states').innerHTML = STATES.map((s, i) => `<button class="tab${i ? '' : ' on'}" data-ms="${s}">${cap(s)}</button>`).join('');
  $('#m-emotes').innerHTML = Object.keys(EMOTES).map(e => `<button class="btn sm secondary" data-me="${e}">${cap(e)}</button>`).join('');
  $('#m-colors').innerHTML = PILLS.map(([n, c]) => `<button class="dl-dot${c === pillColor ? ' on' : ''}" title="${n}" aria-label="${n}" data-mc="${c}" style="--c:${c || NEUTRAL}"></button>`).join('');
  $('#cands').innerHTML = CANDS.map(([k, name, line]) => `
    <article class="dl-cand">
      <div class="dl-cand-id"><h4><span>${k === 'dot' ? '10 px' : k}</span>${name}</h4><p>${line}</p>
        <button class="btn sm secondary" data-use="${k}">Show in the island</button></div>
      <div class="dl-trio">${[[58, 'open island'], [28, 'peek strip'], [22, 'mini pill']].map(([d, where]) =>
        `<span><span class="mascot" data-live data-kind="${k}" data-d="${d}" data-color="${pillColor}"></span><span>${d} px<br>${where}</span></span>`).join('')}</div>
      <div class="dl-sheet">${STATES.map(s =>
        `<span><span class="mascot" data-kind="${k}" data-d="28" data-state="${s}" data-color="${pillColor}"></span>${s}</span>`).join('')}</div>
    </article>`).join('');

  const pickOne = (list, on) => list.forEach(x => x.classList.toggle('on', x === on));
  $('#m-ctl').addEventListener('click', e => {
    const b = e.target.closest('button');
    if (!b) return;
    const cand = views.filter(v => v.el.dataset.kind), live = cand.filter(v => 'live' in v.el.dataset);
    if (b.dataset.ms) { pickOne($$('[data-ms]'), b); live.forEach(v => v.m.setState(b.dataset.ms)); }
    if (b.dataset.me) live.forEach(v => v.m.triggerEmote(b.dataset.me));
    if ('mc' in b.dataset) { pickOne($$('[data-mc]'), b); cand.forEach(v => { v.m.bodyColor = b.dataset.mc; }); }
  });
  function setKind(k) {
    islandKind = k;
    views.forEach(v => { if (!v.el.dataset.kind) v.m.kind = k; });
    pickOne($$('[data-mk]'), $(`[data-mk="${k}"]`));
  }

  /* ── 03 The island ────────────────────────────────────────────────────── */

  const SECTIONS = { overview: 188, approval: 204, chat: 296, player: 180, shelf: 192, controls: 212 };   // declared heights
  const STRIPS = {
    peek: { w: 288, name: 'peek · pills', open: 'overview', hold: 2000 },
    music: { w: 340, name: 'peek · live activity', open: 'player', hold: 3000 },
    timer: { w: 340, name: 'peek · live activity', open: 'controls', hold: 3000 },
    volume: { w: 380, name: 'notice · volume', open: 'controls', hold: 1600, notice: true },
    track: { w: 380, name: 'notice · track', open: 'player', hold: 3000, notice: true },
    download: { w: 380, name: 'notice · download', open: 'shelf', hold: 6000, notice: true },
  };
  let vol = .62;
  const bars = '<span class="bars"><i></i><i></i><i></i><i></i></span>';
  const STRIP = {
    peek: () => `<span class="mascot" data-d="28" data-state="working" data-color="#E07950"></span><b>Claude Code</b><span class="d">Editing arbiter.ts</span><span class="isl-gap"></span><span class="isl-grid2" title="4 sessions"><i style="--c:#E07950"></i><i style="--c:#2DD4BF"></i><i style="--c:#8AB4F8"></i><i style="--c:#4ADE80"></i></span>`,
    music: () => `<span class="dl-art a1"></span><b>Low Tide</b><span class="d">Halden Fjord</span><span class="isl-gap"></span>${bars}`,
    timer: () => `<span class="isl-ico warn">${icon('timer')}</span><b>Focus</b><span class="d">until 14:30</span><span class="isl-gap"></span><span class="isl-num" data-timer></span>`,
    volume: () => `${icon(vol ? 'volume' : 'mute')}<b>Speakers</b><span class="isl-gap"></span><span class="meter" style="--v:${vol}"></span><span class="isl-num">${Math.round(vol * 100)}</span>`,
    track: () => `<span class="dl-art a2"></span><b>Paper Lanterns</b><span class="d">Mira Solace</span><span class="isl-gap"></span>${bars}`,
    download: () => `<span class="isl-ico ok">${icon('checkc')}</span><b>Download finished</b><span class="d">meteor-0.4.2-setup.exe</span><span class="isl-gap"></span><button class="btn sm ghost">Show</button>`,
  };
  /* Puts strip content in a box: the old content fades out (or just goes, when the box was
     not on screen), the new one drops or rises in. */
  function strip(box, key, enter, unseen) {
    const old = box.lastElementChild;
    if (unseen) box.replaceChildren();
    else if (old) { old.classList.add('fx-out'); setTimeout(() => old.remove(), D.exit); }
    if (!key) return;
    const el = document.createElement('div');
    el.className = 'strip-in ' + enter;
    el.style.setProperty('--w', (box === stripEl ? STRIPS[key].w : 380) + 'px');
    el.innerHTML = STRIP[key]();
    box.append(el);
    mountAll(el); paint(); syncBars();
  }

  const isl = $('#isl'), wake = $('#wake'), stripEl = $('#isl-strip'), readout = $('#readout');
  const sizeTo = shell(isl, 240, 0, () => hover());
  const isOpen = st => st.startsWith('open:');
  let base = 'hidden', cur = 'hidden', stripKey = '', auto = false, over = false, timers = {};
  $$('.isl-sec').forEach(s => s.style.setProperty('--h', SECTIONS[s.dataset.sec] + 'px'));

  function show(st) {
    const was = isl.dataset.mode, open = isOpen(st), s = STRIPS[st];
    const w = st === 'hidden' ? 240 : open ? 640 : s.w, h = st === 'hidden' ? 0 : open ? SECTIONS[st.slice(5)] : 32;
    cur = st;
    sizeTo(w, h);
    isl.dataset.mode = st === 'hidden' ? 'hidden' : open ? 'open' : 'strip';
    if (s && (st !== stripKey || was !== 'strip')) { stripKey = st; strip(stripEl, st, s.notice ? 'fx-drop' : 'fx-in', was !== 'strip'); }
    if (open) {
      const name = st.slice(5);
      $$('.isl-sec').forEach(x => x.classList.toggle('on', x.dataset.sec === name));
      $$('.isl-head [data-sec]').forEach(b => b.classList.toggle('on', b.dataset.sec === (name === 'approval' ? 'overview' : name)));
    }
    readout.textContent = st === 'hidden' ? 'hidden · wake strip 240 × 6'
      : open ? `open · ${st.slice(5)} · 640 × ${h} · content 608 × ${h - 52}`
        : `${s.name} · ${w} × 32 · shown ${s.hold / 1000} s`;
  }
  const clearTimers = () => { for (const k in timers) clearTimeout(timers[k]); timers = {}; };
  /* The buttons hold a state; with "product timings" on it expires as the product would. */
  function setBase(st) {
    clearTimers();
    if (st === 'volume' && cur === 'volume') {       // same kind: refit only, no re-animation (§5.1)
      vol = vol >= .99 ? 0 : Math.min(1, +(vol + .12).toFixed(2));
      const el = stripEl.lastElementChild;
      $('.meter', el).style.setProperty('--v', vol);
      $('.isl-num', el).textContent = Math.round(vol * 100);
      $('svg', el).outerHTML = icon(vol ? 'volume' : 'mute');
    } else { base = st; show(st); }
    pickOne($$('[data-st]'), $(`[data-st="${st}"]`));
    if (auto && STRIPS[st]) timers.auto = setTimeout(expire, STRIPS[st].hold);
  }
  function expire() { if (over) timers.auto = setTimeout(expire, 600); else setBase('hidden'); }

  /* Pointer behaviour (§4): the wake strip peeks a hidden island at once; resting 250 ms
     on the strip opens it; leaving folds a hover-opened island after 0.6 s and hides a
     peeked one after 2 s. */
  function hover() {
    const now = isl.matches(':hover') || wake.matches(':hover');
    if (now === over) return;
    over = now;
    clearTimeout(timers.rest); clearTimeout(timers.leave); clearTimeout(timers.hide);
    if (over) {
      if (cur === 'hidden') show('peek');
      if (STRIPS[cur]) timers.rest = setTimeout(() => show('open:' + STRIPS[cur].open), 250);
    } else if (cur !== base) {
      const hide = () => { timers.hide = setTimeout(() => show('hidden'), 2000); };
      if (isOpen(cur)) timers.leave = setTimeout(() => { show(STRIPS[base] ? base : 'peek'); if (base === 'hidden') hide(); }, 600);
      else hide();
    }
  }
  for (const el of [isl, wake]) {
    el.addEventListener('pointerenter', hover);
    el.addEventListener('pointerleave', () => setTimeout(hover));
    el.addEventListener('click', () => { if (STRIPS[cur]) setBase('open:' + STRIPS[cur].open); });   // a click opens the related section
  }
  $('#isl-ctl').addEventListener('click', e => { const b = e.target.closest('[data-st]'); if (b) setBase(b.dataset.st); });
  $$('.isl-head [data-sec]').forEach(b => b.addEventListener('click', () => {
    const st = 'open:' + b.dataset.sec;
    if (isOpen(base)) setBase(st); else show(st);
  }));
  $('#isl-kind').addEventListener('click', e => { const b = e.target.closest('[data-mk]'); if (b) setKind(b.dataset.mk); });

  /* Live sample data: a running timer and a playing track. */
  const TRACK = 252, clock = s => Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0');
  let left = 18 * 60 + 42, pos = 107, playing = true;
  function paint() {
    $$('[data-timer]').forEach(el => { el.textContent = clock(left); });
    $$('[data-pos]').forEach(el => { el.textContent = clock(pos); });
    const m = $('[data-seek] .meter');
    if (!m.classList.contains('dragging')) m.style.setProperty('--v', pos / TRACK);
  }
  const syncBars = () => $$('.isl .bars').forEach(b => b.classList.toggle('paused', !playing));
  setInterval(() => { left = left ? left - 1 : 25 * 60; if (playing) pos = (pos + 1) % TRACK; paint(); }, 1000);
  $('#play').addEventListener('click', e => {
    playing = !playing;
    e.currentTarget.innerHTML = icon(playing ? 'pause' : 'play');
    e.currentTarget.title = playing ? 'Pause' : 'Play';
    syncBars();
  });

  /* A meter as a slider. The 6 px bar is the visual; its wrapper is the hit area. */
  $$('[data-slide], [data-seek]').forEach(wrap => {
    const m = $('.meter', wrap), out = $('output', wrap);
    const set = v => {
      v = clamp(v, 0, 1);
      m.style.setProperty('--v', v);
      if (out) out.textContent = Math.round(v * 100);
      if ('seek' in wrap.dataset) { pos = Math.round(v * (TRACK - 1)); $$('[data-pos]').forEach(el => { el.textContent = clock(pos); }); }
    };
    const at = e => { const r = m.getBoundingClientRect(); set((e.clientX - r.left) / r.width); };
    wrap.addEventListener('pointerdown', e => { wrap.setPointerCapture(e.pointerId); m.classList.add('dragging'); at(e); });
    wrap.addEventListener('pointermove', e => { if (m.classList.contains('dragging')) at(e); });
    for (const type of ['pointerup', 'pointercancel']) wrap.addEventListener(type, () => m.classList.remove('dragging'));
    m.addEventListener('keydown', e => {
      const step = e.key === 'ArrowRight' || e.key === 'ArrowUp' ? .05 : e.key === 'ArrowLeft' || e.key === 'ArrowDown' ? -.05 : 0;
      if (step) { e.preventDefault(); set(+m.style.getPropertyValue('--v') + step); }
    });
  });

  $('#isl-compose').addEventListener('submit', e => {
    e.preventDefault();
    const input = $('input', e.target), text = input.value.trim(), msgs = $('#isl-msgs'), waiting = $('.thinking', msgs);
    if (!text) return;
    if (waiting) waiting.remove();
    const p = document.createElement('p');
    p.className = 'me fx-in';
    p.textContent = text;
    msgs.append(p);
    msgs.insertAdjacentHTML('beforeend', '<div class="bot thinking fx-in"><span class="mascot" data-d="22" data-state="thinking" data-color="#E07950"></span><p>Thinking</p></div>');
    mountAll(msgs);
    input.value = '';
    msgs.scrollTop = msgs.scrollHeight;
  });

  /* ── Motion demos that reuse the strip ────────────────────────────────── */

  const fxA = $('#fx-a'), fxB = $('#fx-b'), NOTICES = ['volume', 'track', 'download'];
  let fxFlip = false, fxN = 0;

  /* ── Small interactions shared by the specimens ───────────────────────── */

  document.addEventListener('click', e => {
    const t = e.target;
    const sw = t.closest('.switch');
    if (sw) {
      const on = !sw.classList.contains('on');
      sw.classList.toggle('on', on);
      sw.setAttribute('aria-checked', on);
      if (sw.id === 'isl-outline') isl.classList.toggle('outlined', on);
      if (sw.id === 'isl-auto') { auto = on; setBase(base); }
    }
    const pick = t.closest('[data-pick]'), item = t.closest('.tab, .list-item, .isl-pill');
    if (pick && item && pick.contains(item)) {
      pickOne($$('.tab, .list-item, .isl-pill', pick), item);
      if (item.dataset.name) {                       // an agent pill: it becomes the focused agent
        const f = $('#isl-focus'), m = $('.mascot', f)._v.m;
        m.setState(item.dataset.state); m.bodyColor = item.dataset.color;
        $('b', f).textContent = item.dataset.name;
        $('span:not(.mascot)', f).textContent = item.dataset.task;
        $('small', f).textContent = item.dataset.meta;
      }
    }
    const tile = t.closest('.isl-controls .tile[data-tone]');
    if (tile) tile.classList.toggle(tile.dataset.tone);
    const file = t.closest('.isl-file');
    if (file) file.classList.toggle('on');
    const emote = t.closest('[data-emote]');
    if (emote) $('#approval-mascot')._v.m.triggerEmote(emote.dataset.emote);
    const run = t.closest('[data-run]');
    if (run) run.previousElementSibling.classList.toggle('go');
    const fx = t.closest('[data-fx]');
    if (fx && fx.dataset.fx === 'a') strip(fxA, (fxFlip = !fxFlip) ? 'timer' : 'music', 'fx-in');
    if (fx && fx.dataset.fx === 'b') { strip(fxB); setTimeout(() => strip(fxB, NOTICES[++fxN % 3], 'fx-drop'), D.exit + 200); }
    const use = t.closest('[data-use]');
    if (use) { setKind(use.dataset.use); setBase('open:overview'); $('#island').scrollIntoView(); }
  });

  $('#btn-matrix').innerHTML = '<span></span>' + ['Default', 'Hover', 'Active', 'Focus', 'Disabled'].map(s => `<span class="dl-cap">${s}</span>`).join('') +
    [['primary', 'Allow'], ['secondary', 'Always'], ['ghost', 'Show diff'], ['danger', 'Deny']].map(([kind, label]) =>
      `<code>.${kind}</code>` + ['', 'is-hover', 'is-hover is-active', 'is-focus', ''].map((st, i) =>
        `<button class="btn ${kind} ${st}"${i === 4 ? ' disabled' : ''}>${label}</button>`).join('')).join('');

  /* ── Contents: mark the section in view ───────────────────────────────── */

  const links = $$('#toc a');
  const spy = new IntersectionObserver(es => es.forEach(en => {
    if (en.isIntersecting) pickOne(links, links.find(a => a.getAttribute('href') === '#' + en.target.id));
  }), { rootMargin: '-15% 0px -75% 0px' });
  $$('.dl-sec').forEach(s => spy.observe(s));

  /* ── Start ────────────────────────────────────────────────────────────── */

  mountAll();
  strip(fxA, 'music', '');
  strip(fxB, 'volume', '');
  setBase('hidden');
  // The first time the island scrolls into view it introduces itself: hidden → peek → open.
  const intro = new IntersectionObserver(es => {
    if (!es[0].isIntersecting) return;
    intro.disconnect();
    if (base !== 'hidden') return;
    timers.intro = setTimeout(() => { setBase('peek'); timers.intro = setTimeout(() => setBase('open:overview'), 900); }, 500);
  }, { threshold: .6 });
  intro.observe($('.dl-screen'));

  let last = performance.now();
  requestAnimationFrame(function frame(now) {
    const dt = Math.min(.05, (now - last) / 1000);
    last = now;
    for (const fn of loops) if (fn(dt, now) === false) loops.delete(fn);
    drawMascots(dt);
    requestAnimationFrame(frame);
  });
})();
