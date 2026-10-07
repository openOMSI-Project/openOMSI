/*
 * The navigator's map on a phone or tablet: the game's navigator (crates/omsi-app/src/
 * navigator.rs) drawn again in the browser from what the game sends (companion/nav.rs).
 *
 * Two ways of looking, as in the game:
 *  - following the bus (the small navigator): tilted 3D behind and above the bus, turning
 *    with it and zooming out with speed - the same camera as the game's (52 degrees down,
 *    40 degrees of view, the bus a little below the middle); or flat, still turning with it;
 *  - looking round (the city map): flat, north up, dragged and pinched (or the wheel).
 *    "Centre" goes back to following.
 *
 * What is drawn: the roads (casing, surface, the main roads brighter), the route ahead in
 * blue with arrows on it and the part driven in grey, the trip's stops as the stop signs the
 * settings choose (German H, British roundel, French arret) with their names and times, the
 * other traffic (public transport with its line), and the bus as a white arrow.
 *
 * The page gives it what it has (`setTrip`, `setNav`, `setRoads`, `setStyle`) and draws the
 * texts itself; the map asks for roads when it looks somewhere it has none (`wantRoads`).
 * It draws only while something moves: a phone's battery matters in a bus.
 */
'use strict';

(function () {
  var PITCH = 52 * Math.PI / 180;
  var FOV = 40 * Math.PI / 180;
  var FAR = 4500;

  var C = {
    ground: '#0b111b',
    casing: 'rgba(8, 10, 14, 0.95)',
    road: '#404754',
    main: '#545c6b',
    /* (the accent's, set from the game's state: `setAccent`) */
    route: '#f58620',
    routeCasing: '#6c3b0e',
    driven: '#3e4656',
    arrow: 'rgba(255, 255, 255, 0.9)',
    bus: '#ffffff',
    busEdge: 'rgba(8, 10, 14, 0.85)',
    label: 'rgba(10, 12, 18, 0.86)',
    labelInk: '#e8ebf2',
    labelSoft: '#959db0',
    next: '#f58620',
    nextGlow: 'rgba(245, 134, 32, 0.28)',
    now: '#f0b429',
    ai: ['#468cff', '#2eb85c', '#e23a34', '#f0be1e']
  };

  function clamp(x, a, b) { return x < a ? a : x > b ? b : x; }

  /* The way from angle a to b (degrees), -180..180. */
  function turn(a, b) {
    var d = (b - a) % 360;
    if (d > 180) d -= 360;
    if (d < -180) d += 360;
    return d;
  }

  function hhmm(sec) {
    if (sec === null || sec === undefined || !isFinite(sec)) return '';
    var m = Math.floor(sec / 60);
    var pad = function (n) { return (n < 10 ? '0' : '') + n; };
    return pad(Math.floor(m / 60) % 24) + ':' + pad(((m % 60) + 60) % 60);
  }

  // ------------------------------------------------------------------ the stop signs

  /*
   * A stop sign of `style` centred at (x, y), `d` pixels across. Drawn from paths, so that
   * they are sharp at any size: the German "H" (green on a yellow disc), the British
   * roundel (a red ring with its bar) and the French arret (a white bus on a blue square).
   */
  function stopSign(ctx, style, x, y, d) {
    var r = d / 2;
    ctx.save();
    ctx.translate(x, y);
    if (style === 'uk') {
      ctx.fillStyle = '#ffffff';
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
      ctx.strokeStyle = '#da291c';
      ctx.lineWidth = r * 0.36;
      ctx.beginPath(); ctx.arc(0, 0, r * 0.66, 0, Math.PI * 2); ctx.stroke();
      ctx.fillStyle = '#da291c';
      ctx.fillRect(-r * 0.98, -r * 0.2, r * 1.96, r * 0.4);
      ctx.fillStyle = '#ffffff';
      ctx.fillRect(-r * 0.62, -r * 0.07, r * 1.24, r * 0.14);
      ctx.lineWidth = Math.max(1, d * 0.06);
      ctx.strokeStyle = 'rgba(8, 10, 14, 0.9)';
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.stroke();
    } else if (style === 'fr') {
      var s = r * 0.92;
      roundRect(ctx, -s, -s, s * 2, s * 2, s * 0.32);
      ctx.fillStyle = '#1f5fbf'; ctx.fill();
      ctx.lineWidth = Math.max(1, d * 0.06);
      ctx.strokeStyle = 'rgba(8, 10, 14, 0.9)'; ctx.stroke();
      // a bus seen from the side: body, windows, wheels
      ctx.fillStyle = '#ffffff';
      roundRect(ctx, -s * 0.66, -s * 0.46, s * 1.32, s * 0.78, s * 0.14); ctx.fill();
      ctx.fillStyle = '#1f5fbf';
      ctx.fillRect(-s * 0.54, -s * 0.34, s * 0.3, s * 0.26);
      ctx.fillRect(-s * 0.16, -s * 0.34, s * 0.3, s * 0.26);
      ctx.fillRect(s * 0.22, -s * 0.34, s * 0.32, s * 0.26);
      ctx.fillStyle = '#ffffff';
      ctx.beginPath(); ctx.arc(-s * 0.36, s * 0.38, s * 0.13, 0, Math.PI * 2); ctx.fill();
      ctx.beginPath(); ctx.arc(s * 0.36, s * 0.38, s * 0.13, 0, Math.PI * 2); ctx.fill();
    } else {
      ctx.fillStyle = '#f7d117';
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
      ctx.lineWidth = r * 0.16;
      ctx.strokeStyle = '#0f7a3b';
      ctx.beginPath(); ctx.arc(0, 0, r * 0.9, 0, Math.PI * 2); ctx.stroke();
      // the H, drawn: two posts and the bar
      ctx.fillStyle = '#0f7a3b';
      var w = r * 0.2, h = r * 1.0;
      ctx.fillRect(-r * 0.42, -h / 2, w, h);
      ctx.fillRect(r * 0.42 - w, -h / 2, w, h);
      ctx.fillRect(-r * 0.42, -w / 2, r * 0.84, w);
      ctx.lineWidth = Math.max(1, d * 0.06);
      ctx.strokeStyle = 'rgba(8, 10, 14, 0.9)';
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.stroke();
    }
    ctx.restore();
  }

  function roundRect(ctx, x, y, w, h, r) {
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.arcTo(x + w, y, x + w, y + h, r);
    ctx.arcTo(x + w, y + h, x, y + h, r);
    ctx.arcTo(x, y + h, x, y, r);
    ctx.arcTo(x, y, x + w, y, r);
    ctx.closePath();
  }

  /* The chequered flag over the trip's last stop. */
  function flag(ctx, x, y, s) {
    ctx.save();
    ctx.translate(x, y);
    ctx.fillStyle = 'rgba(8, 10, 14, 0.9)';
    ctx.fillRect(-s * 0.5 - 1, -s * 0.5 - 1, s + 2, s * 0.75 + 2);
    var n = 4, c = s / n;
    for (var i = 0; i < n; i++) {
      for (var j = 0; j < 3; j++) {
        ctx.fillStyle = (i + j) % 2 ? '#111' : '#fff';
        ctx.fillRect(-s * 0.5 + i * c, -s * 0.5 + j * c, c, c);
      }
    }
    ctx.restore();
  }

  /*
   * The player's own pins, set on the game's city map (crates/omsi-app/src/nav_pins.rs), in
   * the same violet as there: the destination a pin standing on its place with a chequered
   * flag in its head (green with a tick once there), a via a disc with its number.
   */
  var PIN = '#9b5cf6', PIN_EDGE = 'rgba(22, 10, 44, 0.92)', PIN_DONE = '#37d67a';

  function pinFlag(ctx, x, y, r, arrived) {
    var h = r * 2.1, g = Math.acos(1 / 2.1);
    var drop = function (rr, lift) {
      var cy = y - lift - rr * 2.1;
      ctx.beginPath();
      ctx.moveTo(x, y - lift);
      ctx.arc(x, cy, rr, Math.PI / 2 + g, Math.PI / 2 - g + Math.PI * 2);
      ctx.closePath();
    };
    var e = Math.max(1, r * 0.14);
    ctx.save();
    ctx.fillStyle = 'rgba(0, 0, 0, 0.45)';
    ctx.beginPath(); ctx.arc(x, y, r * 0.28, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = PIN_EDGE; drop(r + e, 0); ctx.fill();
    ctx.fillStyle = arrived ? PIN_DONE : PIN; drop(r, e * 1.4); ctx.fill();
    var hy = y - e * 1.4 - h;
    ctx.fillStyle = '#ffffff';
    ctx.beginPath(); ctx.arc(x, hy, r * 0.64, 0, Math.PI * 2); ctx.fill();
    if (arrived) {
      ctx.strokeStyle = '#1d7a44';
      ctx.lineWidth = r * 0.22;
      ctx.lineCap = 'round';
      ctx.beginPath(); ctx.moveTo(x - r * 0.3, hy); ctx.lineTo(x - r * 0.05, hy + r * 0.25); ctx.lineTo(x + r * 0.32, hy - r * 0.22); ctx.stroke();
    } else {
      // (a small chequered flag on its pole)
      var s = r * 0.62, c = s / 3;
      ctx.fillStyle = '#4c2a8a';
      ctx.fillRect(x - s * 0.5, hy - s * 0.45, r * 0.09, s);
      for (var i = 0; i < 3; i++) {
        for (var j = 0; j < 2; j++) {
          ctx.fillStyle = (i + j) % 2 ? '#ffffff' : '#4c2a8a';
          ctx.fillRect(x - s * 0.5 + r * 0.09 + i * c * 0.9, hy - s * 0.45 + j * c, c * 0.9, c);
        }
      }
    }
    ctx.restore();
  }

  function pinVia(ctx, x, y, r, n, font) {
    ctx.save();
    ctx.fillStyle = PIN_EDGE;
    ctx.beginPath(); ctx.arc(x, y, r + Math.max(1, r * 0.16), 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = '#ffffff';
    ctx.beginPath(); ctx.arc(x, y, r, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = PIN;
    ctx.beginPath(); ctx.arc(x, y, r * 0.8, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = '#ffffff';
    ctx.font = '900 ' + Math.round(r * (n > 9 ? 0.95 : 1.15)) + 'px ' + font;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(String(n), x, y + r * 0.06);
    ctx.restore();
  }

  // ------------------------------------------------------------------ the map

  function OmsiMap(hooks) {
    var el = document.createElement('div');
    el.className = 'map-canvas';
    var canvas = document.createElement('canvas');
    canvas.setAttribute('aria-hidden', 'true');
    el.appendChild(canvas);
    var ctx = canvas.getContext('2d');

    var dpr = 1;
    var W = 0, H = 0;
    var inset = { top: 0, bottom: 0, left: 0, right: 0 };
    /* What else lies over the map (the turn, the buttons): no label goes under it. */
    var avoid = [];
    var k = 1;
    var style = 'de';
    var trip = null;
    var nav = null;
    var roads = null;
    var roadsAsked = null;

    /* How the map looks: following (3D or flat) or looking round (flat, north up). */
    var view = { mode: 'follow', flat: false, zoom: 1, cx: 0, cy: 0, mpp: 2, rot: 0 };
    /* What is shown, eased towards what the game said last. */
    var shown = null;
    var cam = { heading: 0, z: 140, rot: 0 };
    var raf = 0;
    var last = 0;
    var idle = 0;
    var widths = {};

    function px(n) { return n * k * dpr; }

    // ---- sizes

    function resize() {
      var r = el.getBoundingClientRect();
      dpr = Math.min(window.devicePixelRatio || 1, 3);
      var w = Math.max(1, Math.round(r.width * dpr));
      var h = Math.max(1, Math.round(r.height * dpr));
      if (w !== canvas.width || h !== canvas.height) {
        canvas.width = w;
        canvas.height = h;
      }
      W = w;
      H = h;
      wake();
    }

    if (window.ResizeObserver) new ResizeObserver(resize).observe(el);
    window.addEventListener('resize', resize);

    // ---- the cameras

    /* The tilted camera behind the bus at (bx, by), facing `heading`, `z` metres away. */
    function camera3d(bx, by, heading, z) {
      var h = heading * Math.PI / 180;
      var fx = Math.sin(h), fy = Math.cos(h);
      var lx = bx + fx * z * 0.28, ly = by + fy * z * 0.28;
      var ex = lx - fx * z * Math.cos(PITCH), ey = ly - fy * z * Math.cos(PITCH), ez = z * Math.sin(PITCH);
      var dx = lx - ex, dy = ly - ey, dz = -ez;
      var dl = Math.sqrt(dx * dx + dy * dy + dz * dz);
      var f = [dx / dl, dy / dl, dz / dl];
      var rl = Math.sqrt(f[1] * f[1] + f[0] * f[0]);
      var r = [f[1] / rl, -f[0] / rl, 0];
      var u = [r[1] * f[2] - r[2] * f[1], r[2] * f[0] - r[0] * f[2], r[0] * f[1] - r[1] * f[0]];
      var vh = Math.max(1, H - inset.top - inset.bottom);
      var F = (vh / 2) / Math.tan(FOV / 2);
      var sx0 = inset.left + (W - inset.left - inset.right) / 2, sy0 = inset.top + vh / 2;
      // (nothing on the ground nearer than about four fifths of the distance shows: a road
      // cut much nearer runs off the bottom many times as wide as the screen)
      var near = z * 0.4;
      return {
        flat: false,
        near: near,
        rot: heading,
        /* Into the camera's space: across, up, depth. */
        view: function (x, y) {
          var vx = x - ex, vy = y - ey, vz = -ez;
          return [vx * r[0] + vy * r[1], vx * u[0] + vy * u[1] + vz * u[2], vx * f[0] + vy * f[1] + vz * f[2]];
        },
        screen: function (v) {
          var s = F / v[2];
          return [sx0 + v[0] * s, sy0 - v[1] * s, s];
        },
        proj: function (x, y) {
          var v = this.view(x, y);
          return v[2] < near || v[2] > FAR ? null : this.screen(v);
        },
        centre: [lx, ly],
        reach: z * 4.5
      };
    }

    /* A flat camera over (cx, cy), `mpp` metres a pixel, `rot` degrees the way up. */
    function camera2d(cx, cy, mpp, rot) {
      var a = rot * Math.PI / 180;
      var rx = Math.cos(a), ry = -Math.sin(a), fx = Math.sin(a), fy = Math.cos(a);
      var vh = Math.max(1, H - inset.top - inset.bottom);
      var sx0 = inset.left + (W - inset.left - inset.right) / 2, sy0 = inset.top + vh / 2;
      return {
        flat: true,
        rot: rot,
        mpp: mpp,
        proj: function (x, y) {
          var dx = x - cx, dy = y - cy;
          return [sx0 + (dx * rx + dy * ry) / mpp, sy0 - (dx * fx + dy * fy) / mpp, 1 / mpp];
        },
        unproj: function (sx, sy) {
          var xs = (sx - sx0) * mpp, ys = -(sy - sy0) * mpp;
          return [cx + xs * rx + ys * fx, cy + xs * ry + ys * fy];
        },
        centre: [cx, cy],
        reach: Math.hypot(W, H) * mpp * 0.6
      };
    }

    /* The camera for this frame. */
    function camera() {
      var b = shown || { x: 0, y: 0, h: 0 };
      if (view.mode === 'free') return camera2d(view.cx, view.cy, view.mpp, cam.rot);
      if (view.flat) {
        var vh = Math.max(1, H - inset.top - inset.bottom);
        var mpp = cam.z * 2.6 / vh;
        var h = cam.heading * Math.PI / 180;
        var ahead = vh * mpp * 0.2;
        return camera2d(b.x + Math.sin(h) * ahead, b.y + Math.cos(h) * ahead, mpp, cam.heading);
      }
      return camera3d(b.x, b.y, cam.heading, cam.z);
    }

    // ---- drawing

    /*
     * Lines through the camera: each stretch cut at the near plane, projected, and gathered
     * into one path per width (in pixels, to half a pixel), so that a road narrows into the
     * distance without a thousand strokes. `width(metres, scale)` gives a stretch's width.
     */
    function strokeLines(c, lines, widthOf, color, cap) {
      var paths = {};
      var seg = function (a, b, w) {
        var key = Math.round(w * 2) / 2;
        if (key <= 0) return;
        var p = paths[key] || (paths[key] = new Path2D());
        p.moveTo(a[0], a[1]);
        p.lineTo(b[0], b[1]);
      };
      var minX = -W * 0.2, maxX = W * 1.2, minY = -H * 0.2, maxY = H * 1.2;
      var off = function (a, b) {
        return (a[0] < minX && b[0] < minX) || (a[0] > maxX && b[0] > maxX) || (a[1] < minY && b[1] < minY) || (a[1] > maxY && b[1] > maxY);
      };
      for (var i = 0; i < lines.length; i++) {
        var L = lines[i];
        var pts = L.pts;
        if (c.flat) {
          var w = widthOf(L.w, 1 / c.mpp);
          var prev = c.proj(pts[0], pts[1]);
          for (var j = 2; j < pts.length; j += 2) {
            var cur = c.proj(pts[j], pts[j + 1]);
            if (!off(prev, cur)) seg(prev, cur, w);
            prev = cur;
          }
        } else {
          var va = c.view(pts[0], pts[1]);
          for (var q = 2; q < pts.length; q += 2) {
            var vb = c.view(pts[q], pts[q + 1]);
            var a = va, b = vb;
            va = vb;
            if (a[2] < c.near && b[2] < c.near) continue;
            if (a[2] > FAR && b[2] > FAR) continue;
            if (a[2] < c.near || b[2] < c.near) {
              var t = (c.near - a[2]) / (b[2] - a[2]);
              var cut = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, c.near];
              if (a[2] < c.near) a = cut; else b = cut;
            }
            var pa = c.screen(a), pb = c.screen(b);
            if (off(pa, pb)) continue;
            // a long straight stretch (a road thinned to its bends) in pieces, so that it
            // narrows into the distance
            var ratio = Math.max(pa[2], pb[2]) / Math.min(pa[2], pb[2]);
            var n = ratio > 1.12 ? Math.min(24, Math.ceil(Math.log(ratio) / Math.log(1.12))) : 1;
            var p0 = pa;
            for (var m = 1; m <= n; m++) {
              var f = m / n;
              var p1 = m === n ? pb : c.screen([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]);
              if (!off(p0, p1)) seg(p0, p1, widthOf(L.w, (p0[2] + p1[2]) / 2));
              p0 = p1;
            }
          }
        }
      }
      c2.lineCap = cap || 'round';
      c2.lineJoin = 'round';
      c2.strokeStyle = color;
      Object.keys(paths).forEach(function (key) {
        c2.lineWidth = Number(key);
        c2.stroke(paths[key]);
      });
    }
    var c2 = ctx;

    function drawRoads(c) {
      if (!roads) return;
      var reach = c.reach + 400;
      var near = [];
      var centre = c.centre;
      for (var i = 0; i < roads.list.length; i++) {
        var r = roads.list[i];
        if (r.x1 < centre[0] - reach || r.x0 > centre[0] + reach || r.y1 < centre[1] - reach || r.y0 > centre[1] + reach) continue;
        near.push(r);
      }
      var minW = px(1.2);
      strokeLines(c, near, function (w, s) { return Math.max(minW + px(1.4), (w + 1.6) * s); }, C.casing);
      strokeLines(c, near.filter(function (r) { return !r.main; }), function (w, s) { return Math.max(minW, (w + 0.2) * s); }, C.road);
      strokeLines(c, near.filter(function (r) { return r.main; }), function (w, s) { return Math.max(minW * 1.3, (w + 0.2) * s); }, C.main);
    }

    /* The route, cut where the bus is: the part driven grey, the rest blue with arrows. */
    function drawRoute(c) {
      if (!trip || !trip.pts || trip.pts.length < 4) return;
      var pts = trip.pts, along = trip.along;
      var at = shown && shown.along !== null && shown.along !== undefined ? shown.along : -1;
      var done = [], ahead = [];
      var n = along.length;
      var split = -1;
      for (var i = 0; i < n; i++) {
        if (along[i] <= at) split = i;
      }
      if (split >= 0) {
        var d = pts.slice(0, (split + 1) * 2);
        var a = pts.slice(split * 2);
        if (split + 1 < n) {
          // (the point where the bus is, on the stretch it is on)
          var t = (at - along[split]) / Math.max(1e-6, along[split + 1] - along[split]);
          var x = pts[split * 2] + (pts[split * 2 + 2] - pts[split * 2]) * t;
          var y = pts[split * 2 + 1] + (pts[split * 2 + 3] - pts[split * 2 + 1]) * t;
          d.push(x, y);
          a = [x, y].concat(pts.slice((split + 1) * 2));
        }
        done.push({ pts: d, w: 3 });
        ahead.push({ pts: a, w: 3 });
      } else {
        ahead.push({ pts: pts, w: 3 });
      }
      var minW = px(4);
      strokeLines(c, done, function (w, s) { return Math.max(px(2.5), w * 0.7 * s); }, C.driven);
      strokeLines(c, ahead, function (w, s) { return Math.max(minW + px(2), (w + 1.2) * s); }, C.routeCasing);
      strokeLines(c, ahead, function (w, s) { return Math.max(minW, (w - 0.2) * s); }, C.route);
      arrows(c, Math.max(0, at));
    }

    /* Arrows along the route ahead, every 28 m for 900 m (as the game's), further apart
       when they would crowd. */
    function arrows(c, from) {
      var pts = trip.pts, along = trip.along, n = along.length;
      if (n < 2) return;
      var every = 28;
      if (c.flat) every = Math.max(28, c.mpp * px(26));
      var j = 0;
      c2.strokeStyle = C.arrow;
      c2.lineCap = 'round';
      c2.lineJoin = 'round';
      for (var s = from + every * 0.6; s < from + Math.max(900, every * 12) && s < along[n - 1]; s += every) {
        while (j + 1 < n && along[j + 1] < s) j++;
        if (j + 1 >= n) break;
        var t = (s - along[j]) / Math.max(1e-6, along[j + 1] - along[j]);
        var x0 = pts[j * 2], y0 = pts[j * 2 + 1], x1 = pts[j * 2 + 2], y1 = pts[j * 2 + 3];
        var dx = x1 - x0, dy = y1 - y0, dl = Math.hypot(dx, dy) || 1;
        dx /= dl; dy /= dl;
        var x = x0 + (x1 - x0) * t, y = y0 + (y1 - y0) * t;
        var len = 0.9, half = 0.8;
        var tip = c.proj(x + dx * len * 0.5, y + dy * len * 0.5);
        var l = c.proj(x - dx * len * 0.5 - dy * half, y - dy * len * 0.5 + dx * half);
        var r = c.proj(x - dx * len * 0.5 + dy * half, y - dy * len * 0.5 - dx * half);
        if (!tip || !l || !r) continue;
        var w = c.flat ? px(1.6) : Math.max(px(1.2), 0.32 * tip[2]);
        var size = Math.hypot(tip[0] - l[0], tip[1] - l[1]);
        if (size < px(2.5)) {
          // (too small to be an arrow: scaled up round its tip)
          var g = px(4) / Math.max(size, 0.01);
          l = [tip[0] + (l[0] - tip[0]) * g, tip[1] + (l[1] - tip[1]) * g];
          r = [tip[0] + (r[0] - tip[0]) * g, tip[1] + (r[1] - tip[1]) * g];
          w = px(1.4);
        }
        if (tip[0] < -50 || tip[0] > W + 50 || tip[1] < -50 || tip[1] > H + 50) continue;
        c2.lineWidth = w;
        c2.beginPath();
        c2.moveTo(l[0], l[1]);
        c2.lineTo(tip[0], tip[1]);
        c2.lineTo(r[0], r[1]);
        c2.stroke();
      }
    }

    function drawTraffic(c) {
      var ai = nav && nav.ai;
      if (!ai || !ai.length) return;
      var tags = [];
      for (var i = 0; i < ai.length; i++) {
        var v = ai[i];
        var p = c.proj(v[0], v[1]);
        if (!p || p[0] < -20 || p[0] > W + 20 || p[1] < -20 || p[1] > H + 20) continue;
        var big = v[3] !== 0 ? 1.35 : 1;
        var r = Math.max(px(2.6) * big, 1.2 * big * p[2]);
        c2.fillStyle = 'rgba(8, 8, 8, 0.9)';
        c2.beginPath(); c2.arc(p[0], p[1], r + px(1.2), 0, Math.PI * 2); c2.fill();
        c2.fillStyle = C.ai[v[3]] || C.ai[0];
        c2.beginPath(); c2.arc(p[0], p[1], r, 0, Math.PI * 2); c2.fill();
        if (v[4]) tags.push([p, v[3], String(v[4])]);
      }
      // the lines of the public transport over their dots, none on another
      var taken = [];
      c2.font = '800 ' + px(10) + 'px ' + font;
      c2.textAlign = 'center';
      c2.textBaseline = 'middle';
      tags.forEach(function (t) {
        var w = measure(t[2], c2.font) + px(8);
        var h = px(14);
        var box = [t[0][0] - w / 2, t[0][1] - px(20), w, h];
        if (taken.some(function (o) { return overlap(o, box); })) return;
        taken.push(box);
        c2.fillStyle = 'rgba(10, 10, 10, 0.9)';
        roundRect(c2, box[0] - px(1), box[1] - px(1), box[2] + px(2), box[3] + px(2), px(4)); c2.fill();
        c2.fillStyle = C.ai[t[1]];
        roundRect(c2, box[0], box[1], box[2], box[3], px(3.5)); c2.fill();
        c2.fillStyle = '#0f0f0f';
        c2.fillText(t[2], t[0][0], box[1] + h / 2 + px(0.5));
      });
    }

    function overlap(a, b) {
      return a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3];
    }

    var font = 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif';

    /* `text` in `f`, shortened with an ellipsis to `room` pixels. */
    function fit(text, f, room) {
      if (measure(text, f) <= room) return text;
      var lo = 0, hi = text.length;
      while (lo < hi) {
        var mid = Math.ceil((lo + hi) / 2);
        if (measure(text.slice(0, mid) + '…', f) <= room) lo = mid;
        else hi = mid - 1;
      }
      return text.slice(0, lo).trim() + '…';
    }

    function measure(text, f) {
      var key = f + '|' + text;
      if (widths[key] === undefined) {
        c2.font = f;
        widths[key] = c2.measureText(text).width;
      }
      return widths[key];
    }

    /* The trip's stops: the signs, the next one ringed in blue, the last with its flag, and
       their names and times where there is room (the next stop first). */
    function drawStops(c) {
      if (!trip || !trip.stops) return;
      var stops = trip.stops;
      var nextK = nav && nav.next ? nav.next.k : null;
      var placed = [];
      var lastK = stops.length ? stops[stops.length - 1].k : -1;
      for (var i = 0; i < stops.length; i++) {
        var s = stops[i];
        if (!s.at) continue;
        var p = c.proj(s.at[0], s.at[1]);
        if (!p || p[0] < -30 || p[0] > W + 30 || p[1] < -30 || p[1] > H + 30) continue;
        var done = nextK !== null && s.k < nextK;
        placed.push({ s: s, p: p, done: done, next: s.k === nextK, last: s.k === lastK });
      }
      // (the far ones first, so that the near ones lie on top)
      placed.sort(function (a, b) { return a.p[2] - b.p[2]; });
      var crowded = [];
      // (in 3D a sign further off than the bus is smaller, down to two thirds)
      var atBus = shown && !c.flat ? c.proj(shown.x, shown.y) : null;
      placed.forEach(function (m) {
        var d = px(m.next ? 22 : 17);
        if (atBus) d *= clamp(Math.sqrt(m.p[2] / atBus[2]), 0.66, 1.1);
        // (signs on top of each other: the one already there stays)
        if (!m.next && crowded.some(function (q) { return Math.hypot(q[0] - m.p[0], q[1] - m.p[1]) < d * 0.7; })) return;
        crowded.push(m.p);
        c2.globalAlpha = m.done ? 0.45 : 1;
        if (m.next) {
          c2.fillStyle = C.nextGlow;
          c2.beginPath(); c2.arc(m.p[0], m.p[1], d * 0.95, 0, Math.PI * 2); c2.fill();
          c2.strokeStyle = C.next;
          c2.lineWidth = px(2.5);
          c2.beginPath(); c2.arc(m.p[0], m.p[1], d * 0.72, 0, Math.PI * 2); c2.stroke();
        }
        stopSign(c2, style, m.p[0], m.p[1], d);
        if (m.last) flag(c2, m.p[0] + d * 0.55, m.p[1] - d * 0.85, px(11));
        c2.globalAlpha = 1;
        m.d = d;
      });
      // the labels: the next stop, the last, then the rest in order of the trip
      var order = placed.filter(function (m) { return m.d; }).slice().sort(function (a, b) {
        var ra = a.next ? 0 : a.last ? 1 : 2, rb = b.next ? 0 : b.last ? 1 : 2;
        return ra - rb || a.s.k - b.s.k;
      });
      var signs = order.map(function (m) { return [m.p[0] - m.d / 2, m.p[1] - m.d / 2, m.d, m.d]; });
      var taken = avoid.slice();
      var many = c.flat ? c.mpp < 4 : true;
      var count = 0;
      order.forEach(function (m) {
        if (m.done) return;
        if (!m.next && !m.last && (!many || (!c.flat && count >= 4))) return;
        var time = hhmm(m.s.arr);
        var f1 = '700 ' + px(12) + 'px ' + font, f2 = '600 ' + px(11.5) + 'px ' + font;
        // (a long name on a narrow phone: shortened to fit the map's width)
        var room = Math.min(px(260), W - inset.left - inset.right - px(16)) - measure(time, f2) - px(24);
        var name = fit(m.s.name, f1, room);
        var w = measure(name, f1) + px(8) + measure(time, f2) + px(16);
        var h = px(22);
        var g = m.d / 2 + px(6);
        var spots = [[m.p[0] + g, m.p[1] - h / 2], [m.p[0] - g - w, m.p[1] - h / 2], [m.p[0] + g * 0.6, m.p[1] - h - g * 0.6], [m.p[0] + g * 0.6, m.p[1] + g * 0.6], [m.p[0] - w - g * 0.6, m.p[1] - h - g * 0.6], [m.p[0] - w - g * 0.6, m.p[1] + g * 0.6], [m.p[0] - w / 2, m.p[1] - g - h], [m.p[0] - w / 2, m.p[1] + g]];
        var box = null;
        for (var i = 0; i < spots.length; i++) {
          var b = [spots[i][0], spots[i][1], w, h];
          if (b[0] < inset.left + px(6) || b[0] + w > W - inset.right - px(6) || b[1] < inset.top + px(4) || b[1] + h > H - inset.bottom - px(4)) continue;
          if (taken.some(function (o) { return overlap(o, b); })) continue;
          // (over another stop's sign, not over its own)
          if (signs.some(function (o) { return overlap(o, b) && !(o[0] === m.p[0] - m.d / 2 && o[1] === m.p[1] - m.d / 2); })) continue;
          box = b;
          break;
        }
        if (!box) return;
        taken.push(box);
        count++;
        c2.fillStyle = m.next ? 'rgba(18, 52, 112, 0.94)' : C.label;
        roundRect(c2, box[0], box[1], box[2], box[3], px(6)); c2.fill();
        c2.textAlign = 'left';
        c2.textBaseline = 'middle';
        c2.font = f1;
        c2.fillStyle = C.labelInk;
        c2.fillText(name, box[0] + px(8), box[1] + h / 2 + px(0.5));
        c2.font = f2;
        c2.fillStyle = m.next ? '#ffd27a' : C.labelSoft;
        c2.fillText(time, box[0] + px(8) + measure(name, f1) + px(8), box[1] + h / 2 + px(0.5));
      });
    }

    /* The player's own pins (the game's city map sets them): the vias numbered, the
       destination's flag; the first on top. */
    function drawPins(c) {
      var pins = nav && nav.pins;
      if (!pins || !pins.length) return;
      for (var i = pins.length - 1; i >= 0; i--) {
        var p = pins[i];
        var q = c.proj(p.x, p.y);
        if (!q || q[0] < -40 || q[0] > W + 40 || q[1] < -20 || q[1] > H + 60) continue;
        if (p.via === 0) pinFlag(c2, q[0], q[1], px(9), nav.arrived);
        else pinVia(c2, q[0], q[1], px(8.5), p.via, font);
      }
    }

    /* The bus: a plain white arrow, as the game's. */
    function drawBus(c) {
      if (!shown) return;
      var p = c.proj(shown.x, shown.y);
      if (!p) return;
      var a = turn(c.rot, shown.h) * Math.PI / 180;
      var s = px(11);
      var rot = function (x, y) { return [p[0] + (x * Math.cos(a) - y * Math.sin(a)) * s, p[1] + (x * Math.sin(a) + y * Math.cos(a)) * s]; };
      var tip = rot(0, -1), l = rot(-0.72, 0.82), m = rot(0, 0.4), r = rot(0.72, 0.82);
      c2.lineJoin = 'round';
      c2.fillStyle = C.bus;
      c2.strokeStyle = C.busEdge;
      c2.lineWidth = px(3);
      c2.beginPath();
      c2.moveTo(tip[0], tip[1]); c2.lineTo(l[0], l[1]); c2.lineTo(m[0], m[1]); c2.lineTo(r[0], r[1]); c2.closePath();
      c2.stroke();
      c2.fill();
    }

    function draw() {
      c2.setTransform(1, 0, 0, 1, 0, 0);
      c2.fillStyle = C.ground;
      c2.fillRect(0, 0, W, H);
      var c = camera();
      drawRoads(c);
      drawRoute(c);
      drawTraffic(c);
      drawStops(c);
      drawPins(c);
      drawBus(c);
      // the far end fades into the dark, as in the game
      if (!c.flat) {
        var g = c2.createLinearGradient(0, inset.top, 0, inset.top + (H - inset.top) * 0.28);
        g.addColorStop(0, 'rgba(11, 17, 27, 0.9)');
        g.addColorStop(1, 'rgba(11, 17, 27, 0)');
        c2.fillStyle = g;
        c2.fillRect(0, 0, W, inset.top + (H - inset.top) * 0.28);
      }
      askRoads(c);
    }

    /* Roads round where the map looks, when those it has do not reach there (or are too
       coarse for the zoom). */
    function askRoads(c) {
      if (!hooks.wantRoads || !nav) return;
      var reach = c.reach;
      var r = clamp(reach * 1.5, 1200, 6000);
      var tol = c.flat ? clamp(c.mpp * 0.5, 0, 12) : 0;
      var have = roadsAsked || roads;
      if (have && have.v === (nav.roads || 0)) {
        var dx = c.centre[0] - have.cx, dy = c.centre[1] - have.cy;
        // (zoomed far out, the roads within a few kilometres are all there is)
        var inside = Math.hypot(dx, dy) + Math.min(reach, 5000) < have.r * 1.02;
        var fine = (have.tol || 0) <= tol * 2 + 0.5;
        if (inside && fine) return;
      }
      if (roadsAsked) return;
      roadsAsked = { cx: Math.round(c.centre[0]), cy: Math.round(c.centre[1]), r: Math.round(r), tol: Math.round(tol * 10) / 10, v: nav.roads || 0 };
      var asked = roadsAsked;
      hooks.wantRoads(asked.cx, asked.cy, asked.r, asked.tol, function (j) {
        if (roadsAsked !== asked) return;
        roadsAsked = null;
        if (j) setRoads(j, asked.tol);
        else roads = roads || null;
      });
    }

    // ---- moving

    /* Ease what is shown towards what the game said; true while anything still moves. */
    function step(dt) {
      if (!nav || !nav.bus) return false;
      var target = { x: nav.bus[0], y: nav.bus[1], h: nav.h || 0, along: nav.along };
      if (!shown || Math.hypot(target.x - shown.x, target.y - shown.y) > 80) {
        shown = { x: target.x, y: target.y, h: target.h, along: target.along };
        cam.heading = target.h;
        cam.z = followZoom();
        return true;
      }
      var e = 1 - Math.exp(-dt / 0.18);
      var moved = Math.hypot(target.x - shown.x, target.y - shown.y) > 0.02 || Math.abs(turn(shown.h, target.h)) > 0.05;
      shown.x += (target.x - shown.x) * e;
      shown.y += (target.y - shown.y) * e;
      shown.h += turn(shown.h, target.h) * e;
      if (target.along === null || target.along === undefined || shown.along === null || shown.along === undefined || Math.abs(target.along - shown.along) > 200) shown.along = target.along;
      else shown.along += (target.along - shown.along) * e;
      var hc = turn(cam.heading, shown.h);
      cam.heading += hc * (1 - Math.exp(-dt / 0.35));
      var z = followZoom();
      cam.z += (z - cam.z) * (1 - Math.exp(-dt / 1.6));
      var rotGoal = view.mode === 'free' ? 0 : cam.heading;
      var rc = turn(cam.rot, rotGoal);
      cam.rot += rc * (1 - Math.exp(-dt / 0.25));
      return moved || Math.abs(hc) > 0.05 || Math.abs(z - cam.z) > 0.2 || Math.abs(rc) > 0.05;
    }

    /* How far the following camera stands off: further out the faster the bus goes (as
       the game's), further on a tall map (a tablet's shows more than a phone's, not the
       same bigger), times the driver's own zoom. */
    function followZoom() {
      var v = nav ? Math.abs(nav.v || 0) : 0;
      var tall = clamp((H - inset.top - inset.bottom) / dpr / k / 380, 1, 1.8);
      return clamp(110 + v * 2.2, 110, 280) * tall * view.zoom;
    }

    function frame(now) {
      raf = 0;
      var dt = last ? Math.min(0.1, (now - last) / 1000) : 0.016;
      last = now;
      var moving = step(dt);
      draw();
      if (moving || gestures.size) idle = 0;
      else idle++;
      if (idle < 3) raf = requestAnimationFrame(frame);
      else last = 0;
    }

    function wake() {
      idle = 0;
      if (!raf) raf = requestAnimationFrame(frame);
    }

    // ---- touch, mouse and wheel

    var gestures = new Map();
    var pinch = null;
    var tapAt = 0;

    function local(e) {
      var r = canvas.getBoundingClientRect();
      return [(e.clientX - r.left) * dpr, (e.clientY - r.top) * dpr];
    }

    /* Looking round from where the map looks now. */
    function free() {
      if (view.mode === 'free') return;
      var c = camera();
      var vh = Math.max(1, H - inset.top - inset.bottom);
      view.mode = 'free';
      view.cx = c.centre[0];
      view.cy = c.centre[1];
      view.mpp = c.flat ? c.mpp : cam.z * 2.6 / vh;
      cam.rot = c.rot;
      if (hooks.onMode) hooks.onMode(view.mode);
    }

    function zoomAt(f, sx, sy) {
      if (view.mode !== 'free') {
        view.zoom = clamp(view.zoom * f, 0.35, 4);
        wake();
        return;
      }
      var c = camera();
      var before = c.unproj(sx, sy);
      view.mpp = clamp(view.mpp * f, 0.08, 40);
      var after = camera().unproj(sx, sy);
      view.cx += before[0] - after[0];
      view.cy += before[1] - after[1];
      wake();
    }

    canvas.addEventListener('pointerdown', function (e) {
      try {
        canvas.setPointerCapture(e.pointerId);
      } catch (x) {
        /* an old browser */
      }
      gestures.set(e.pointerId, local(e));
      if (gestures.size === 2) {
        var p = Array.from(gestures.values());
        pinch = { d: Math.hypot(p[0][0] - p[1][0], p[0][1] - p[1][1]), m: [(p[0][0] + p[1][0]) / 2, (p[0][1] + p[1][1]) / 2] };
      }
      // a double tap zooms in
      var now = e.timeStamp;
      if (gestures.size === 1 && now - tapAt < 300) {
        var q = local(e);
        zoomAt(0.6, q[0], q[1]);
        tapAt = 0;
      } else {
        tapAt = now;
      }
    });

    canvas.addEventListener('pointermove', function (e) {
      if (!gestures.has(e.pointerId)) return;
      var prev = gestures.get(e.pointerId);
      var cur = local(e);
      gestures.set(e.pointerId, cur);
      if (gestures.size === 1) {
        var dx = cur[0] - prev[0], dy = cur[1] - prev[1];
        if (view.mode !== 'free' && Math.hypot(dx, dy) < 2) return;
        free();
        var c = camera();
        var a = c.unproj(prev[0], prev[1]), b = c.unproj(cur[0], cur[1]);
        view.cx -= b[0] - a[0];
        view.cy -= b[1] - a[1];
        tapAt = 0;
        wake();
      } else if (gestures.size === 2 && pinch) {
        var p = Array.from(gestures.values());
        var d = Math.hypot(p[0][0] - p[1][0], p[0][1] - p[1][1]);
        var m = [(p[0][0] + p[1][0]) / 2, (p[0][1] + p[1][1]) / 2];
        if (d > 10 && pinch.d > 10) zoomAt(pinch.d / d, m[0], m[1]);
        if (view.mode === 'free') {
          var cc = camera();
          var a2 = cc.unproj(pinch.m[0], pinch.m[1]), b2 = cc.unproj(m[0], m[1]);
          view.cx -= b2[0] - a2[0];
          view.cy -= b2[1] - a2[1];
        }
        pinch = { d: d, m: m };
        tapAt = 0;
      }
    });

    var up = function (e) {
      gestures.delete(e.pointerId);
      if (gestures.size < 2) pinch = null;
    };
    canvas.addEventListener('pointerup', up);
    canvas.addEventListener('pointercancel', up);
    canvas.addEventListener('wheel', function (e) {
      e.preventDefault();
      var q = local(e);
      zoomAt(Math.exp(e.deltaY * 0.0015), q[0], q[1]);
    }, { passive: false });

    // ---- what the page gives it

    function setRoads(j, tol) {
      var list = [];
      var cx = j.x || 0, cy = j.y || 0;
      (j.roads || []).forEach(function (a) {
        var pts = new Float64Array(a.length - 2);
        var x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
        for (var i = 2; i + 1 < a.length; i += 2) {
          var x = cx + a[i] / 10, y = cy + a[i + 1] / 10;
          pts[i - 2] = x;
          pts[i - 1] = y;
          if (x < x0) x0 = x; if (x > x1) x1 = x;
          if (y < y0) y0 = y; if (y > y1) y1 = y;
        }
        if (pts.length >= 4) list.push({ pts: pts, w: a[0] / 10, main: a[1] === 1, x0: x0, y0: y0, x1: x1, y1: y1 });
      });
      roads = { v: j.v || 0, cx: cx, cy: cy, r: j.r || 0, tol: tol || 0, list: list };
      wake();
    }

    var api = {
      el: el,
      setTrip: function (t) {
        trip = t && t.pts ? { v: t.v, pts: t.pts, along: t.along || [], stops: t.stops || [] } : t && t.stops ? { v: t.v, pts: [], along: [], stops: t.stops } : null;
        wake();
      },
      setNav: function (n) {
        nav = n;
        wake();
      },
      setRoads: setRoads,
      setStyle: function (s) {
        if (s !== style) {
          style = s;
          wake();
        }
      },
      /* The parts of the map other things lie over (CSS pixels), and the interface size. */
      setInsets: function (top, bottom, left, right, size) {
        var n = { top: top * dpr, bottom: bottom * dpr, left: (left || 0) * dpr, right: (right || 0) * dpr };
        if (n.top !== inset.top || n.bottom !== inset.bottom || n.left !== inset.left || n.right !== inset.right || size !== k) {
          inset = n;
          k = size || 1;
          widths = {};
          wake();
        }
      },
      /* Boxes over the map (CSS pixels: x, y, width, height) no label should go under. */
      setAvoid: function (rects) {
        var n = (rects || []).map(function (r) { return [r[0] * dpr, r[1] * dpr, r[2] * dpr, r[3] * dpr]; });
        if (JSON.stringify(n) !== JSON.stringify(avoid)) {
          avoid = n;
          wake();
        }
      },
      follow: function () {
        view.mode = 'follow';
        if (hooks.onMode) hooks.onMode(view.mode);
        wake();
      },
      setFlat: function (flat) {
        view.flat = !!flat;
        wake();
      },
      zoom: function (f) {
        var vh = H - inset.top - inset.bottom;
        zoomAt(f, inset.left + (W - inset.left - inset.right) / 2, inset.top + vh / 2);
      },
      mode: function () { return view.mode; },
      resize: resize,
      wake: wake
    };
    return api;
  }

  window.OmsiMap = OmsiMap;
  window.OmsiMap.stopSign = stopSign;
  /* The route and the next stop in the interface's accent (`#rrggbb`), its casing darker. */
  window.OmsiMap.setAccent = function (base, casing, rgb) {
    C.route = base;
    C.next = base;
    C.nextGlow = 'rgba(' + rgb + ', 0.28)';
    C.routeCasing = casing;
  };
})();
