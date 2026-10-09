//! Block flow and wrapped inline text: turns the styled page into boxes, and finds the
//! box under a point.

use super::*;

pub(crate) struct LItem {
    pub(crate) text: String,
    pub(crate) px: f32,
    pub(crate) bold: bool,
    pub(crate) color: [u8; 4],
    pub(crate) dx: f32,
    pub(crate) w: f32,
    pub(crate) node: usize,
}

pub(crate) struct LLine {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) h: f32,
    pub(crate) items: Vec<LItem>,
}

pub(crate) enum Item {
    Block(LBox),
    Line(LLine),
}

pub(crate) struct LBox {
    pub(crate) rect: [f32; 4],
    pub(crate) mb: f32,
    pub(crate) st: Style,
    pub(crate) items: Vec<Item>,
    pub(crate) node: usize,
}

/// Move a laid-out box (and everything in it).
pub(crate) fn shift(b: &mut LBox, dx: f32, dy: f32) {
    b.rect[0] += dx;
    b.rect[1] += dy;
    for it in &mut b.items {
        match it {
            Item::Block(c) => shift(c, dx, dy),
            Item::Line(l) => {
                l.x += dx;
                l.y += dy;
            }
        }
    }
}

/// How wide the content of a box wants to be, counted from `cx` (its content's left edge).
pub(crate) fn natural_width(b: &LBox, cx: f32) -> f32 {
    let mut w = 0.0f32;
    for it in &b.items {
        match it {
            Item::Line(l) => {
                if let Some(li) = l.items.last() {
                    w = w.max(li.dx + li.w);
                }
            }
            Item::Block(c) => w = w.max(c.rect[0] - cx + c.rect[2] + c.st.margin[1]),
        }
    }
    w
}

/// The element under a point: the text run or box on top, `None` outside every box.
pub(crate) fn hit(b: &LBox, x: f32, y: f32) -> Option<usize> {
    if b.st.hidden {
        return None;
    }
    for it in b.items.iter().rev() {
        match it {
            Item::Block(c) => {
                if let Some(n) = hit(c, x, y) {
                    return Some(n);
                }
            }
            Item::Line(l) => {
                if y >= l.y && y < l.y + l.h {
                    for li in &l.items {
                        let x0 = l.x + li.dx;
                        if x >= x0 && x < x0 + li.w {
                            return Some(li.node);
                        }
                    }
                }
            }
        }
    }
    let [rx, ry, rw, rh] = b.rect;
    (x >= rx && x < rx + rw && y >= ry && y < ry + rh).then_some(b.node)
}

pub(crate) struct Layouter<'a> {
    pub(crate) dom: &'a Dom,
    pub(crate) reg: &'a FontRef<'static>,
    pub(crate) bold: &'a FontRef<'static>,
    pub(crate) vw: f32,
    pub(crate) vh: f32,
    pub(crate) imgs: &'a ImageStore,
}

impl<'a> Layouter<'a> {
    pub(crate) fn tw(&self, s: &str, px: f32, bold: bool) -> f32 {
        let font = if bold { self.bold } else { self.reg };
        let sc = PxScale::from(px);
        let mut w = 0.0;
        let mut prev = None;
        for c in s.chars() {
            face_for(font, bold, c, |face, id, fi| {
                let sf = face.as_scaled(sc);
                if let Some((p, pf)) = prev {
                    if pf == fi {
                        w += sf.kern(p, id);
                    }
                }
                w += sf.h_advance(id);
                prev = Some((id, fi));
            });
        }
        w
    }

    pub(crate) fn style_of(&self, idx: usize, ps: &Style) -> Style {
        let n = &self.dom.nodes[idx];
        let mut st = ps.inherit();
        let pf = ps.font_px;
        match n.tag.as_str() {
            "b" | "strong" => {
                st.inline = true;
                st.bold = true;
            }
            "span" | "i" | "em" | "a" | "small" | "u" | "label" | "code" => st.inline = true,
            "h1" => {
                st.font_px = pf * 2.0;
                st.bold = true;
                st.margin[0] = st.font_px * 0.67;
                st.margin[2] = st.font_px * 0.67;
            }
            "h2" => {
                st.font_px = pf * 1.5;
                st.bold = true;
                st.margin[0] = st.font_px * 0.83;
                st.margin[2] = st.font_px * 0.83;
            }
            "h3" => {
                st.font_px = pf * 1.17;
                st.bold = true;
                st.margin[0] = st.font_px;
                st.margin[2] = st.font_px;
            }
            "p" => {
                st.margin[0] = pf;
                st.margin[2] = pf;
            }
            "body" => st.margin = [8.0; 4],
            "button" => {
                st.inline_block = true;
                st.bg = [0xe0, 0xe0, 0xe0, 255];
                st.color = [0, 0, 0, 255];
                st.padding = [6.0, 12.0, 6.0, 12.0];
                st.align = 1;
                st.radius = 4.0;
            }
            "img" => st.inline_block = true,
            "head" | "style" | "script" | "title" | "meta" | "link" => st.none = true,
            _ => {}
        }
        if n.tag == "small" {
            st.font_px = pf * 0.83;
        }
        let mut matched: Vec<(u32, usize, &Rule)> = self
            .dom
            .rules
            .iter()
            .filter_map(|r| {
                r.sels.iter().filter(|c| self.dom.matches(idx, c)).map(specificity).max().map(|s| (s, r.order, r))
            })
            .collect();
        matched.sort_by_key(|(s, o, _)| (*s, *o));
        for (_, _, r) in matched {
            for (k, v) in &r.decls {
                st.apply(k, v, pf, self.vw, self.vh);
            }
        }
        for (k, v) in &n.inline {
            st.apply(k, v, pf, self.vw, self.vh);
        }
        if n.tag == "img" {
            self.size_img(idx, &mut st);
        }
        st.node = idx;
        st
    }

    /// The size of an `<img>`: CSS first, then its `width`/`height` attributes, then the
    /// picture's own size; a missing side keeps the picture's proportions.
    fn size_img(&self, idx: usize, st: &mut Style) {
        let n = &self.dom.nodes[idx];
        let u = Units { font: st.font_px, vw: self.vw, vh: self.vh };
        if st.width.is_none() && !n.attr_w.is_empty() {
            st.width = parse_len(&n.attr_w, &u);
        }
        if st.height.is_none() && !n.attr_h.is_empty() {
            st.height = parse_len(&n.attr_h, &u);
        }
        let (nw, nh) = match self.imgs.dims(&n.src) {
            Some((w, h)) if !n.src.is_empty() => (w as f32, h as f32),
            _ => (0.0, 0.0),
        };
        match (st.width, st.height) {
            (None, None) => {
                st.width = Some(Len::Px(nw));
                st.height = Some(Len::Px(nh));
            }
            (Some(Len::Px(w)), None) => st.height = Some(Len::Px(if nw > 0.0 { w * nh / nw } else { 0.0 })),
            (None, Some(Len::Px(h))) => st.width = Some(Len::Px(if nh > 0.0 { h * nw / nh } else { 0.0 })),
            (Some(Len::Pct(_)), None) if nw > 0.0 => st.aspect = nh / nw,
            _ => {}
        }
    }

    pub(crate) fn inline_runs(&self, idx: usize, st: &Style, out: &mut Vec<(String, Style)>) {
        for &k in &self.dom.nodes[idx].kids {
            let kn = &self.dom.nodes[k];
            if let Some(t) = &kn.text {
                out.push((t.clone(), st.clone()));
                continue;
            }
            let cs = self.style_of(k, st);
            if cs.none {
                continue;
            }
            if kn.tag == "br" {
                out.push(("\n".into(), cs));
            } else if cs.inline {
                self.inline_runs(k, &cs, out);
            }
        }
    }

    pub(crate) fn finish_line(&self, items: &mut Vec<LItem>, lw: &mut f32, lh: &mut f32, out: &mut Vec<LLine>, cy: &mut f32, x: f32, w: f32, align: u8) {
        if let Some(l) = items.last_mut() {
            let t = l.text.trim_end().to_string();
            let nw = self.tw(&t, l.px, l.bold);
            *lw -= l.w - nw;
            l.w = nw;
            l.text = t;
        }
        let off = match align {
            1 => (w - *lw) / 2.0,
            2 => w - *lw,
            _ => 0.0,
        }
            .max(0.0);
        let h = *lh;
        out.push(LLine { x: x + off, y: *cy, h, items: std::mem::take(items) });
        *cy += h;
        *lw = 0.0;
        *lh = 0.0;
    }

    pub(crate) fn lines(&self, runs: &[(String, Style)], x: f32, y: f32, w: f32, align: u8) -> (Vec<LLine>, f32) {
        let mut out = Vec::new();
        let mut cy = y;
        let mut items: Vec<LItem> = Vec::new();
        let (mut lw, mut lh) = (0.0f32, 0.0f32);
        for (text, st) in runs {
            if text == "\n" {
                if lh == 0.0 {
                    lh = st.font_px * st.line_h;
                }
                self.finish_line(&mut items, &mut lw, &mut lh, &mut out, &mut cy, x, w, align);
                continue;
            }
            for tok in text.split_inclusive(' ') {
                let full = self.tw(tok, st.font_px, st.bold);
                let t = tok.trim_end();
                let trimmed = if t.len() == tok.len() { full } else { self.tw(t, st.font_px, st.bold) };
                if !items.is_empty() && lw + trimmed > w + 0.01 {
                    self.finish_line(&mut items, &mut lw, &mut lh, &mut out, &mut cy, x, w, align);
                }
                if items.is_empty() && tok.trim().is_empty() {
                    continue;
                }
                items.push(LItem { text: tok.to_string(), px: st.font_px, bold: st.bold, color: st.color, dx: lw, w: full, node: st.node });
                lw += full;
                lh = lh.max(st.font_px * st.line_h);
            }
        }
        if !items.is_empty() {
            self.finish_line(&mut items, &mut lw, &mut lh, &mut out, &mut cy, x, w, align);
        }
        (out, cy - y)
    }

    pub(crate) fn build(&self, idx: usize, ps: &Style, x0: f32, y0: f32, avail: f32, pct_h: f32) -> LBox {
        self.build_w(idx, ps, x0, y0, avail, pct_h, None)
    }

    /// `force` fixes the content width (the row layout of inline blocks uses it).
    pub(crate) fn build_w(&self, idx: usize, ps: &Style, x0: f32, y0: f32, avail: f32, pct_h: f32, force: Option<f32>) -> LBox {
        let st = self.style_of(idx, ps);
        let [mt, mr, mb, ml] = st.margin;
        let [pt, pr, pb, pl] = st.padding;
        if force.is_none() && st.inline_block && st.width.is_none() {
            // shrink to fit: lay it out at full width, then narrow it to what the content used
            let auto = (avail - ml - mr - pl - pr).max(0.0);
            let probe = self.build_w(idx, ps, x0, y0, avail, pct_h, Some(auto));
            let nat = natural_width(&probe, x0 + ml + pl);
            return self.build_w(idx, ps, x0, y0, avail, pct_h, Some(nat.ceil().min(auto)));
        }
        let (outer_w, content_w) = match (force, st.width) {
            (Some(cw), _) => (cw + pl + pr, cw),
            (None, Some(l)) => {
                let cw = l.px(avail).max(0.0);
                (cw + pl + pr, cw)
            }
            (None, None) => {
                let ow = (avail - ml - mr).max(0.0);
                (ow, (ow - pl - pr).max(0.0))
            }
        };
        let bx = if st.margin_auto && st.width.is_some() { x0 + ((avail - outer_w) / 2.0).max(0.0) } else { x0 + ml };
        let by = y0 + mt;
        let cx = bx + pl;
        let mut cy = by + pt;
        let child_pct_h = match st.height {
            Some(l) => l.px(pct_h),
            None => pct_h,
        };
        let mut items = Vec::new();
        let mut runs: Vec<(String, Style)> = Vec::new();
        let flush = |runs: &mut Vec<(String, Style)>, cy: &mut f32, items: &mut Vec<Item>| {
            if runs.iter().any(|(t, _)| !t.trim().is_empty() || t == "\n") {
                let (lines, h) = self.lines(runs, cx, *cy, content_w, st.align);
                *cy += h;
                items.extend(lines.into_iter().map(Item::Line));
            }
            runs.clear();
        };
        // consecutive `inline-block` children are laid out in rows, wrapping at the content width
        let mut group: Vec<usize> = Vec::new();
        let place = |group: &mut Vec<usize>, cy: &mut f32, items: &mut Vec<Item>| {
            if group.is_empty() {
                return;
            }
            let mut boxes: Vec<LBox> =
                group.drain(..).map(|k| self.build_w(k, &st, 0.0, 0.0, content_w, child_pct_h, None)).collect();
            let occupied = |b: &LBox| b.st.margin[3] + b.rect[2] + b.st.margin[1];
            let mut i = 0;
            while i < boxes.len() {
                let (mut j, mut rw) = (i, 0.0f32);
                while j < boxes.len() {
                    let ow = occupied(&boxes[j]);
                    if j > i && rw + ow > content_w + 0.01 {
                        break;
                    }
                    rw += ow;
                    j += 1;
                }
                let off = match st.align {
                    1 => (content_w - rw) / 2.0,
                    2 => content_w - rw,
                    _ => 0.0,
                }
                    .max(0.0);
                let rh = boxes[i..j].iter().map(|b| b.st.margin[0] + b.rect[3] + b.st.margin[2]).fold(0.0, f32::max);
                let mut x = cx + off;
                for b in &mut boxes[i..j] {
                    let ow = occupied(b);
                    shift(b, x, *cy);
                    x += ow;
                }
                *cy += rh;
                i = j;
            }
            items.extend(boxes.into_iter().map(Item::Block));
        };
        for &k in &self.dom.nodes[idx].kids {
            let kn = &self.dom.nodes[k];
            if let Some(t) = &kn.text {
                place(&mut group, &mut cy, &mut items);
                runs.push((t.clone(), st.clone()));
                continue;
            }
            let cs = self.style_of(k, &st);
            if cs.none {
                continue;
            }
            if kn.tag == "br" {
                place(&mut group, &mut cy, &mut items);
                runs.push(("\n".into(), cs));
                continue;
            }
            if cs.inline_block {
                flush(&mut runs, &mut cy, &mut items);
                group.push(k);
                continue;
            }
            if cs.inline {
                place(&mut group, &mut cy, &mut items);
                self.inline_runs(k, &cs, &mut runs);
                continue;
            }
            place(&mut group, &mut cy, &mut items);
            flush(&mut runs, &mut cy, &mut items);
            let child = self.build(k, &st, cx, cy, content_w, child_pct_h);
            cy = child.rect[1] + child.rect[3] + child.mb;
            items.push(Item::Block(child));
        }
        place(&mut group, &mut cy, &mut items);
        flush(&mut runs, &mut cy, &mut items);
        let content_h = match st.height {
            Some(l) => l.px(pct_h),
            None if st.aspect > 0.0 => content_w * st.aspect,
            None => cy - (by + pt),
        };
        // a button with a fixed height keeps its label in the middle
        if self.dom.nodes[idx].tag == "button" && st.height.is_some() {
            let dy = ((content_h - (cy - (by + pt))) / 2.0).max(0.0);
            if dy > 0.0 {
                for it in &mut items {
                    match it {
                        Item::Block(c) => shift(c, 0.0, dy),
                        Item::Line(l) => l.y += dy,
                    }
                }
            }
        }
        let h = content_h + pt + pb;
        LBox { rect: [bx, by, outer_w, h], mb, st, items, node: idx }
    }
}