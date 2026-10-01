// openOMSI website: a small hash router that shows the overview, the download page and the
// Markdown files of docs/ (copied next to this page by .github/workflows/pages.yml).
const REPO = "openOMSI-Project/openOMSI";
const DOCS = [
  { file: "USER_GUIDE", title: "User guide", icon: "sports_esports" },
  { file: "ANDROID", title: "Android & mobile", icon: "smartphone" },
  { file: "MODDING", title: "Modding beyond OMSI 2", icon: "handyman" },
  { file: "PBR", title: "PBR materials", icon: "texture" },
  { file: "BUILDING", title: "Building", icon: "build" },
  { file: "FORMATS", title: "Content formats", icon: "description" },
  { file: "ARCHITECTURE", title: "Architecture", icon: "account_tree" },
  { file: "ROUTES", title: "Routes", icon: "alt_route" },
  { file: "PLUGINS", title: "Plugins", icon: "extension" },
  { file: "SERVER", title: "Dedicated server", icon: "dns" },
  { file: "VERSIONING", title: "Versioning & releases", icon: "new_releases" },
];
const PLATFORMS = [
  { key: "windows-x64", name: "Windows", icon: "desktop_windows", note: "64-bit (x64), Windows 10 or newer", os: "windows" },
  { key: "windows-arm64", name: "Windows on ARM", icon: "desktop_windows", note: "ARM64 (Snapdragon laptops), Windows 11", os: "windows-arm" },
  { key: "macos-arm64", name: "macOS (Apple silicon)", icon: "laptop_mac", note: "M1 or newer, macOS 11 or newer", os: "mac" },
  { key: "macos-x64", name: "macOS (Intel)", icon: "laptop_mac", note: "Intel Macs, macOS 11 or newer", os: "mac-intel" },
  { key: "linux-x64", name: "Linux", icon: "computer", note: "x86-64, Vulkan drivers", os: "linux" },
  { key: "linux-arm64", name: "Linux on ARM", icon: "computer", note: "ARM64, Vulkan drivers", os: "linux-arm" },
  { key: "android-arm64", ext: "apk", name: "Android", icon: "smartphone", note: "arm64 phones and tablets, Android 8.0+", os: "android" },
];
// The dedicated server: no window, for hosting a session (see the Dedicated server page).
const SERVERS = [
  { key: "server-windows-x64", name: "Server for Windows", icon: "dns", note: "x64, no window" },
  { key: "server-windows-arm64", name: "Server for Windows on ARM", icon: "dns", note: "ARM64, no window" },
  { key: "server-linux-x64", name: "Server for Linux", icon: "dns", note: "x86-64, no window" },
  { key: "server-linux-arm64", name: "Server for Linux on ARM", icon: "dns", note: "ARM64 (Raspberry Pi 4/5, ARM VPS), no window" },
];

// Which build fits the visitor, roughly: the one to put first and mark.
function visitorOs() {
  const ua = navigator.userAgent || "";
  if (/Android/i.test(ua)) return "android";
  if (/Windows/i.test(ua)) return /ARM|aarch64/i.test(ua) ? "windows-arm" : "windows";
  if (/Mac OS X|Macintosh/i.test(ua)) return "mac";
  if (/Linux/i.test(ua)) return /aarch64|arm64/i.test(ua) ? "linux-arm" : "linux";
  return "";
}

const view = document.getElementById("view");
const drawer = document.getElementById("drawer");
const scrim = document.getElementById("scrim");

document.getElementById("doc-links").innerHTML = DOCS.map(d =>
  `<a href="#/docs/${d.file}" data-route="/docs/${d.file}"><span class="material-icons">${d.icon}</span>${d.title}</a>`).join("");

function toggleDrawer(open) {
  drawer.classList.toggle("open", open);
  scrim.classList.toggle("open", open);
}
document.getElementById("menu-btn").onclick = () => toggleDrawer(!drawer.classList.contains("open"));
scrim.onclick = () => toggleDrawer(false);

let release = null;
let releaseAt = 0;
async function latestRelease() {
  // (a release comes with every push: what was fetched ten minutes ago may be old)
  if (release && Date.now() - releaseAt < 120000) return release;
  releaseAt = Date.now();
  try {
    const r = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`);
    release = r.ok ? await r.json() : { none: true };
  } catch { release = { none: true }; }
  return release;
}

function slug(text) {
  return text.toLowerCase().replace(/<[^>]+>/g, "").replace(/[^\w\- ]+/g, "").trim().replace(/\s+/g, "-");
}

function showTemplate(id) {
  view.innerHTML = "";
  view.appendChild(document.getElementById(id).content.cloneNode(true));
}

async function home() {
  showTemplate("home");
  const rel = await latestRelease();
  const chip = document.querySelector("#version-chip span:last-child");
  if (chip) chip.textContent = rel.tag_name ? `Latest: ${rel.tag_name.replace(/^v/, "")}` : "No release yet";
}

async function download() {
  showTemplate("download");
  const rel = await latestRelease();
  const grid = document.getElementById("dl-grid");
  const ver = document.getElementById("dl-version");
  if (!rel.tag_name) {
    ver.textContent = "No release has been published yet.";
    return;
  }
  const v = rel.tag_name.replace(/^v/, "");
  ver.innerHTML = `Latest version: <b>${v}</b> · ${new Date(rel.published_at).toLocaleDateString()}`;
  // Each file by its exact name: matched by its ending alone, "-windows-x64.zip" was the
  // dedicated server's zip as well, and the Windows button downloaded the server.
  const card = p => {
    const file = `openOMSI-${v}-${p.key}.${p.ext || "zip"}`;
    const a = (rel.assets || []).find(a => a.name === file);
    const size = a ? ` · ${(a.size / 1048576).toFixed(0)} MB` : "";
    const mine = p.os && p.os === visitorOs();
    return `<div class="card elevation-${mine ? 3 : 1} dl-card${mine ? " dl-mine" : ""}"><div class="dl-head"><span class="material-icons card-icon">${p.icon}</span>${mine ? `<span class="dl-badge">Your system</span>` : ""}</div>
      <h3>${p.name}</h3><p>${p.note}${size}</p>
      ${a ? `<a class="btn btn-contained" href="${a.browser_download_url}" download><span class="material-icons">download</span>Download</a>`
          : `<p>Not in this release.</p>`}</div>`;
  };
  const games = [...PLATFORMS].sort((x, y) => (y.os === visitorOs()) - (x.os === visitorOs()));
  grid.innerHTML = games.map(card).join("")
    + `<h2 class="section-title" style="grid-column:1/-1">Dedicated server</h2>`
    + SERVERS.map(card).join("");
}

async function doc(name, anchor) {
  const meta = DOCS.find(d => d.file === name);
  view.innerHTML = `<div class="content"><div class="doc"><div class="loading">Loading…</div></div></div>`;
  let md;
  try {
    const r = await fetch(`docs/${name}.md`);
    if (!r.ok) throw new Error(r.status);
    md = await r.text();
  } catch {
    view.querySelector(".doc").innerHTML = `<h1>Not found</h1><p>There is no document called <code>${name}</code>.</p>`;
    return;
  }
  const box = view.querySelector(".doc");
  box.innerHTML = `<div class="doc-toolbar"><a href="https://github.com/${REPO}/blob/main/docs/${name}.md">
    <span class="material-icons" style="font-size:18px">edit</span> Edit on GitHub</a></div>` + marked.parse(md);
  document.title = `${meta ? meta.title : name} · openOMSI`;
  // heading anchors
  box.querySelectorAll("h1, h2, h3, h4").forEach(h => {
    h.id = slug(h.textContent);
    if (h.tagName !== "H1") h.insertAdjacentHTML("beforeend", `<a class="anchor" href="#/docs/${name}#${h.id}">#</a>`);
  });
  // links between documents stay inside the site; other repository files go to GitHub
  box.querySelectorAll("a[href]").forEach(a => {
    const href = a.getAttribute("href");
    if (/^(https?:|mailto:|#)/.test(href)) return;
    const m = href.match(/^(?:\.\/)?([A-Z_]+)\.md(?:#(.*))?$/);
    if (m && DOCS.some(d => d.file === m[1])) {
      a.setAttribute("href", `#/docs/${m[1]}${m[2] ? "#" + m[2] : ""}`);
    } else {
      a.setAttribute("href", `https://github.com/${REPO}/blob/main/docs/${href}`);
    }
  });
  if (anchor) document.getElementById(anchor)?.scrollIntoView();
  else window.scrollTo(0, 0);
}


// --- Releases and issues, read from the GitHub API (60 requests an hour per visitor
// without a token: every list is fetched once per visit and kept)
const cache = {};
async function gh(path) {
  if (cache[path]) return cache[path];
  const r = await fetch(`https://api.github.com/repos/${REPO}/${path}`, { headers: { Accept: "application/vnd.github+json" } });
  if (!r.ok) throw new Error(r.status === 403 ? "GitHub's hourly request limit is reached - try again later." : `GitHub answered ${r.status}.`);
  return (cache[path] = await r.json());
}
function esc(t) {
  return String(t ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}
function md(text) {
  // the author's markdown, with anything active taken out of it (scripts, frames, event
  // handlers, javascript: links)
  const t = document.createElement("template");
  t.innerHTML = marked.parse(text || "");
  t.content.querySelectorAll("script, style, iframe, object, embed, form, link, meta").forEach(e => e.remove());
  t.content.querySelectorAll("*").forEach(e => [...e.attributes].forEach(a => {
    if (/^on/i.test(a.name) || (/^(href|src)$/i.test(a.name) && /^\s*(javascript|data):/i.test(a.value) && !/^data:image\//i.test(a.value))) e.removeAttribute(a.name);
  }));
  return t.innerHTML;
}
function ago(date) {
  const s = (Date.now() - new Date(date)) / 1000;
  for (const [n, u] of [[31536000, "year"], [2592000, "month"], [86400, "day"], [3600, "hour"], [60, "minute"]]) {
    if (s >= n) { const k = Math.floor(s / n); return `${k} ${u}${k > 1 ? "s" : ""} ago`; }
  }
  return "just now";
}
function labelChips(labels) {
  return (labels || []).map(l => `<span class="label" style="--lc:#${esc(l.color)}">${esc(l.name)}</span>`).join("");
}

async function releases() {
  view.innerHTML = `<div class="content"><h1>Releases</h1>
    <p class="lead">Every version, newest first. The newest one is also on the <a href="#/download">Download</a> page.</p>
    <div id="rel-list"><div class="loading">Loading…</div></div></div>`;
  document.title = "Releases · openOMSI";
  const box = document.getElementById("rel-list");
  let list;
  try { list = await gh("releases?per_page=30"); } catch (e) { box.innerHTML = `<p>${esc(e.message)} <a href="https://github.com/${REPO}/releases">Releases on GitHub</a></p>`; return; }
  if (!list.length) { box.innerHTML = "<p>No release has been published yet.</p>"; return; }
  box.innerHTML = list.map((r, i) => {
    const assets = (r.assets || []).map(a => `<a class="asset" href="${esc(a.browser_download_url)}"><span class="material-icons">download</span>${esc(a.name)} <small>${(a.size / 1048576).toFixed(0)} MB</small></a>`).join("");
    return `<details class="card elevation-1 release"${i === 0 ? " open" : ""}>
      <summary><span class="rel-tag">${esc(r.tag_name.replace(/^v/, ""))}</span>
        ${i === 0 ? '<span class="label" style="--lc:#2da44e">Latest</span>' : ""}${r.prerelease ? '<span class="label" style="--lc:#bf8700">Pre-release</span>' : ""}
        <span class="rel-name">${esc(r.name && r.name !== r.tag_name ? r.name : "")}</span>
        <span class="spacer"></span><span class="muted">${new Date(r.published_at).toLocaleDateString()}</span></summary>
      <div class="doc rel-body">${md(r.body) || "<p class='muted'>No notes.</p>"}</div>
      ${assets ? `<div class="assets">${assets}</div>` : ""}
    </details>`;
  }).join("");
}

let issueState = "open", issueQuery = "";
async function issues(number) {
  if (number) return issue(number);
  view.innerHTML = `<div class="content"><h1>Issues</h1>
    <p class="lead">Bugs and wishes. To report one, you need a GitHub account:
      <a class="btn btn-contained" href="https://github.com/${REPO}/issues/new"><span class="material-icons">add</span>New issue</a></p>
    <div class="issue-bar">
      <div class="tabs"><button data-s="open">Open</button><button data-s="closed">Closed</button><button data-s="all">All</button></div>
      <input id="iq" type="search" placeholder="Filter by title or label" value="${esc(issueQuery)}">
    </div>
    <div id="issue-list"><div class="loading">Loading…</div></div></div>`;
  document.title = "Issues · openOMSI";
  view.querySelectorAll(".tabs button").forEach(b => {
    b.classList.toggle("on", b.dataset.s === issueState);
    b.onclick = () => { issueState = b.dataset.s; issues(); };
  });
  const box = document.getElementById("issue-list");
  let list;
  try { list = await gh(`issues?state=${issueState}&per_page=100&sort=updated`); } catch (e) { box.innerHTML = `<p>${esc(e.message)} <a href="https://github.com/${REPO}/issues">Issues on GitHub</a></p>`; return; }
  // (the issues list also carries pull requests)
  list = list.filter(i => !i.pull_request);
  const draw = () => {
    const q = issueQuery.trim().toLowerCase();
    const shown = list.filter(i => !q || i.title.toLowerCase().includes(q) || (i.labels || []).some(l => l.name.toLowerCase().includes(q)) || String(i.number) === q.replace("#", ""));
    box.innerHTML = shown.length ? `<div class="card elevation-1 issue-list">${shown.map(i => `
      <a class="issue-row" href="#/issues/${i.number}">
        <span class="material-icons ${i.state === "open" ? "st-open" : "st-closed"}">${i.state === "open" ? "radio_button_unchecked" : "check_circle"}</span>
        <span class="issue-main"><span class="issue-title">${esc(i.title)}</span> ${labelChips(i.labels)}
          <span class="muted">#${i.number} · ${esc(i.user?.login)} · updated ${ago(i.updated_at)}</span></span>
        ${i.comments ? `<span class="muted cm"><span class="material-icons">chat_bubble_outline</span>${i.comments}</span>` : ""}
      </a>`).join("")}</div>` : `<p class="muted">Nothing here.</p>`;
  };
  document.getElementById("iq").oninput = e => { issueQuery = e.target.value; draw(); };
  draw();
}

async function issue(n) {
  view.innerHTML = `<div class="content"><p><a href="#/issues"><span class="material-icons" style="font-size:18px;vertical-align:-3px">arrow_back</span> All issues</a></p><div id="issue-box"><div class="loading">Loading…</div></div></div>`;
  const box = document.getElementById("issue-box");
  let i, comments;
  try { [i, comments] = await Promise.all([gh(`issues/${n}`), gh(`issues/${n}/comments?per_page=100`)]); }
  catch (e) { box.innerHTML = `<p>${esc(e.message)}</p>`; return; }
  document.title = `#${i.number} ${i.title} · openOMSI`;
  const post = (who, when, body) => `<div class="card elevation-1 comment"><div class="comment-head"><img src="${esc(who?.avatar_url)}&s=48" alt=""><b>${esc(who?.login)}</b><span class="muted">${ago(when)}</span></div><div class="doc">${md(body) || "<p class='muted'>No description.</p>"}</div></div>`;
  box.innerHTML = `<h1>${esc(i.title)} <span class="muted">#${i.number}</span></h1>
    <p><span class="label" style="--lc:${i.state === "open" ? "#2da44e" : "#8250df"}">${i.state === "open" ? "Open" : "Closed"}</span> ${labelChips(i.labels)}</p>
    ${post(i.user, i.created_at, i.body)}${comments.map(c => post(c.user, c.created_at, c.body)).join("")}
    <p><a class="btn btn-outlined" href="${esc(i.html_url)}"><span class="material-icons">reply</span>Comment on GitHub</a></p>`;
  window.scrollTo(0, 0);
}

function route() {
  const hash = location.hash.replace(/^#/, "") || "/";
  const [path, anchor] = hash.split("#");
  document.querySelectorAll(".drawer a[data-route]").forEach(a => a.classList.toggle("active", a.dataset.route === path || (a.dataset.route === "/issues" && path.startsWith("/issues/"))));
  toggleDrawer(false);
  document.title = "openOMSI";
  if (path.startsWith("/docs/")) return doc(path.slice(6), anchor);
  if (path === "/download") return download();
  if (path === "/releases") return releases();
  if (path.startsWith("/issues")) return issues(path.split("/")[2]);
  window.scrollTo(0, 0);
  return home();
}
window.addEventListener("hashchange", route);
route();

// Donate: the button opens its menu upwards; a click elsewhere, Escape or a choice closes it
(() => {
  const button = document.getElementById("donate-button");
  const menu = document.getElementById("donate-menu");
  if (!button || !menu) return;
  const open = on => {
    menu.hidden = !on;
    button.setAttribute("aria-expanded", on ? "true" : "false");
    if (on) menu.querySelector("a").focus();
  };
  button.addEventListener("click", e => { e.stopPropagation(); open(menu.hidden); });
  menu.addEventListener("click", e => { if (e.target.closest("a")) open(false); });
  document.addEventListener("click", e => { if (!menu.hidden && !e.target.closest("#donate")) open(false); });
  document.addEventListener("keydown", e => {
    if (e.key === "Escape" && !menu.hidden) { open(false); button.focus(); }
  });
})();
