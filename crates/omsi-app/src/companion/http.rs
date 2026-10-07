//! The little HTTP the companion's server speaks: one request per connection (`Connection:
//! close`, as the WebSocket gateway's status page does), a request head of 8 KiB at most and
//! a body of 4 KiB at most (what a phone sends fits in a few hundred bytes, and an unbounded
//! stream is a way to fill the game's memory). Parsing, routing and the commands a device may
//! send are here, apart from the sockets, so that they can be tested as they are.

use serde_json::Value;

/// Longest request head (request line and headers).
pub(crate) const MAX_HEAD: usize = 8 * 1024;
/// Longest request body.
pub(crate) const MAX_BODY: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Request {
    pub method: String,
    /// The path, percent-decoded, without the query.
    pub path: String,
    pub query: Vec<(String, String)>,
    /// Header names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub(crate) fn query(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpError {
    /// The head is not complete yet: read more.
    Incomplete,
    TooLarge,
    Bad,
}

/// Parse a request head from the bytes read so far: the request and where its body begins.
pub(crate) fn parse_head(bytes: &[u8]) -> Result<(Request, usize), HttpError> {
    let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Err(if bytes.len() > MAX_HEAD { HttpError::TooLarge } else { HttpError::Incomplete });
    };
    if end > MAX_HEAD {
        return Err(HttpError::TooLarge);
    }
    let head = std::str::from_utf8(&bytes[..end]).map_err(|_| HttpError::Bad)?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().ok_or(HttpError::Bad)?.split(' ');
    let method = first.next().filter(|m| !m.is_empty() && m.bytes().all(|b| b.is_ascii_uppercase())).ok_or(HttpError::Bad)?.to_string();
    let target = first.next().filter(|t| t.starts_with('/')).ok_or(HttpError::Bad)?;
    if !first.next().is_some_and(|v| v.starts_with("HTTP/1.")) {
        return Err(HttpError::Bad);
    }
    let (raw_path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    let path = percent_decode(raw_path).ok_or(HttpError::Bad)?;
    if path.contains("..") || path.contains('\\') || path.contains('\0') {
        return Err(HttpError::Bad);
    }
    let query = raw_query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            Some((percent_decode(&k.replace('+', " "))?, percent_decode(&v.replace('+', " "))?))
        })
        .collect();
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    Ok((Request { method, path, query, headers, body: Vec::new() }, end + 4))
}

/// How long the body is, by `Content-Length` (none: 0). Too long a body is an error.
pub(crate) fn body_length(r: &Request) -> Result<usize, HttpError> {
    match r.header("content-length") {
        None => Ok(0),
        Some(v) => match v.parse::<usize>() {
            Ok(n) if n <= MAX_BODY => Ok(n),
            Ok(_) => Err(HttpError::TooLarge),
            Err(_) => Err(HttpError::Bad),
        },
    }
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The files of the page (`assets/companion`), built into the game: nothing is read from the
/// disk, so no request can make the server read another file.
pub(crate) const ASSETS: [(&str, &str, &str); 5] = [
    ("/", "text/html; charset=utf-8", include_str!("../../../../assets/companion/index.html")),
    ("/app.js", "text/javascript; charset=utf-8", include_str!("../../../../assets/companion/app.js")),
    ("/style.css", "text/css; charset=utf-8", include_str!("../../../../assets/companion/style.css")),
    ("/icon.svg", "image/svg+xml", include_str!("../../../../assets/companion/icon.svg")),
    // (the navigator's map: drawn on the page from what `/api/nav`, `/api/trip` and
    // `/api/roads` send)
    ("/map.js", "text/javascript; charset=utf-8", include_str!("../../../../assets/companion/map.js")),
];

/// Where a request goes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Route {
    /// One of [`ASSETS`] (its index).
    Asset(usize),
    /// The page's manifest ("Add to home screen").
    Manifest,
    /// `POST /api/pair`: a pairing code for a device key. No key needed.
    Pair,
    /// `GET /api/texts?lang=`: the page's texts in a language. No key needed (the pairing
    /// screen is translated too).
    Texts(String),
    /// `GET /api/state?after=<version>`: the state, once it is newer than `after` (or after a
    /// while without news).
    State(u64),
    /// `GET /api/frame?screen=<id>&after=<seq>`: a screen's picture newer than `after`.
    Frame(String, u64),
    /// `GET /api/view?screen=<id>&after=<seq>`: the live picture of the screen's device (the
    /// screen and its keys as the cab shows them) newer than `after`.
    View(String, u64),
    /// `GET /api/form?screen=<id>`: the device as the page draws it (`companion::form`).
    Form(String),
    /// `GET /api/live?screen=<id>&after=<version>`: what changed on it since `after`; asking
    /// for it is what keeps it made.
    Live(String, u64),
    /// `GET /api/tex?n=<id>`: a texture file a form shows, as PNG (the part of it the form
    /// uses).
    Tex(usize),
    /// `GET /api/font?n=<id>` and `/api/fontimg?n=<id>`: a font the text textures of a form
    /// are written in, and its bitmap.
    Font(usize),
    FontImg(usize),
    /// `POST /api/do`: a [`Command`] as JSON.
    Do,
    /// `GET /api/nav?after=<version>`: the navigator's live picture once it is newer than
    /// `after`; asking for it is what keeps it made (see `companion::nav`).
    Nav(u64),
    /// `GET /api/trip`: the route as a line and the trip's stops.
    Trip,
    /// `GET /api/roads?x=&y=&r=&tol=`: the map's roads within `r` metres of a place,
    /// thinned to `tol` metres.
    Roads { x: f64, y: f64, r: f64, tol: f64 },
    /// `GET /api/qr`: the QR code another device scans to open the page and pair at once (a
    /// paired device may show it: the code is no secret to it).
    PairQr,
    /// `GET /api/company`: the bus company the launcher has open (`companion::company`), and
    /// `POST /api/company`: an order for it, as strict JSON.
    Company,
    CompanyOrder,
    NotFound,
    /// A known path with the wrong method.
    Method,
}

impl Route {
    /// Whether the route wants a paired device's key.
    pub(crate) fn needs_key(&self) -> bool {
        matches!(self, Route::State(_) | Route::Frame(..) | Route::View(..) | Route::Form(_) | Route::Live(..) | Route::Tex(_) | Route::Font(_) | Route::FontImg(_) | Route::Do | Route::Nav(_) | Route::Trip | Route::Roads { .. } | Route::PairQr | Route::Company | Route::CompanyOrder)
    }
}

pub(crate) fn route(r: &Request) -> Route {
    let get = r.method == "GET" || r.method == "HEAD";
    let post = r.method == "POST";
    let wants = |ok: bool, to: Route| if ok { to } else { Route::Method };
    if let Some(i) = ASSETS.iter().position(|a| a.0 == r.path || (a.0 == "/" && r.path == "/index.html")) {
        return wants(get, Route::Asset(i));
    }
    let number = |name: &str| r.query(name).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    match r.path.as_str() {
        "/manifest.webmanifest" => wants(get, Route::Manifest),
        "/api/pair" => wants(post, Route::Pair),
        "/api/texts" => {
            let lang: String = r.query("lang").unwrap_or("").chars().filter(|c| c.is_ascii_alphabetic() || *c == '-').take(8).collect();
            wants(get, Route::Texts(lang.to_ascii_lowercase()))
        }
        "/api/state" => wants(get, Route::State(number("after"))),
        "/api/frame" => {
            let screen = r.query("screen").unwrap_or("");
            if !screen_id_ok(screen) {
                return Route::NotFound;
            }
            wants(get, Route::Frame(screen.to_string(), number("after")))
        }
        "/api/view" => {
            let screen = r.query("screen").unwrap_or("");
            if !screen_id_ok(screen) {
                return Route::NotFound;
            }
            wants(get, Route::View(screen.to_string(), number("after")))
        }
        "/api/form" | "/api/live" => {
            let screen = r.query("screen").unwrap_or("");
            if !screen_id_ok(screen) {
                return Route::NotFound;
            }
            wants(get, if r.path == "/api/form" { Route::Form(screen.to_string()) } else { Route::Live(screen.to_string(), number("after")) })
        }
        "/api/tex" | "/api/font" | "/api/fontimg" => {
            let Some(n) = r.query("n").and_then(|v| v.parse::<usize>().ok()).filter(|n| *n < 100_000) else { return Route::NotFound };
            wants(
                get,
                match r.path.as_str() {
                    "/api/tex" => Route::Tex(n),
                    "/api/font" => Route::Font(n),
                    _ => Route::FontImg(n),
                },
            )
        }
        "/api/do" => wants(post, Route::Do),
        "/api/nav" => wants(get, Route::Nav(number("after"))),
        "/api/trip" => wants(get, Route::Trip),
        "/api/roads" => {
            let f = |name: &str| r.query(name).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
            // (a place on a map: within a few hundred kilometres of its origin)
            let (Some(x), Some(y)) = (f("x").filter(|v| v.abs() < 1e7), f("y").filter(|v| v.abs() < 1e7)) else { return Route::NotFound };
            let radius = f("r").unwrap_or(1600.0).clamp(200.0, 6000.0);
            let tol = f("tol").unwrap_or(0.0).clamp(0.0, 25.0);
            wants(get, Route::Roads { x, y, r: radius, tol })
        }
        "/api/qr" => wants(get, Route::PairQr),
        "/api/company" => {
            if post {
                Route::CompanyOrder
            } else {
                wants(get, Route::Company)
            }
        }
        _ => Route::NotFound,
    }
}

/// A screen's id: `s` (script texture), `t` (text texture) or `p` (a device of pages) and its
/// number; `x` and a number is the whole picture of a script texture a form shows.
pub(crate) fn screen_id_ok(id: &str) -> bool {
    id.len() >= 2 && id.len() <= 6 && matches!(id.as_bytes()[0], b's' | b't' | b'p' | b'x') && id[1..].bytes().all(|b| b.is_ascii_digit())
}

/// What the pointer did on a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pointer {
    Down,
    Move,
    Up,
}

/// What a device may ask the game to do. A fixed list, and nothing else gets past the
/// parser: no file, no profile, no setting is reachable from the network.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Command {
    /// The keypad: the number alone (the first step), then with the code.
    SignOn { number: String, code: Option<String> },
    SignOff,
    /// Sign the duty order.
    Accept,
    /// Drive without a duty.
    Free,
    /// Start (true) or end a break.
    Break(bool),
    /// The duty menu: the lines.
    Lines,
    /// The tours of line number `line` of the last list of lines.
    Tours { line: usize },
    /// Take on tour `tour` of the last list of tours of line `line`.
    Pick { line: usize, tour: usize },
    /// A press, move or release on a screen, at `u`, `v` (0..1 across the part shown).
    Pointer { screen: String, kind: Pointer, u: f32, v: f32 },
    /// A key beside a screen pressed (`down`) or let go.
    Key { screen: String, key: usize, down: bool },
    /// A press, move or release on the live picture of a screen's device, at `x`, `y` (0..1
    /// across the picture): clicked into the cab along the camera's ray through that point.
    Tap { screen: String, kind: Pointer, x: f32, y: f32 },
    /// Touch area `touch` of a screen's form pressed (`down`) or let go: its switch clicked as
    /// the mouse clicks it.
    Touch { screen: String, touch: usize, down: bool },
    /// A press, move or release on the `[htmltexture]` page in script texture `page` that a
    /// screen's form shows, at `u`, `v` of the texture.
    Page { screen: String, page: usize, kind: Pointer, u: f32, v: f32 },
}

impl Command {
    pub(crate) fn from_json(v: &Value) -> Option<Command> {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str());
        let n = |k: &str| v.get(k).and_then(|x| x.as_u64()).filter(|x| *x < 10_000).map(|x| x as usize);
        let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0) as f32);
        let digits = |x: &str| (1..=12).contains(&x.len()) && x.bytes().all(|b| b.is_ascii_digit());
        let screen = || s("screen").filter(|x| screen_id_ok(x)).map(str::to_string);
        Some(match s("do")? {
            "sign_on" => {
                let number = s("number").filter(|x| digits(x))?.to_string();
                let code = match v.get("code") {
                    None | Some(Value::Null) => None,
                    Some(c) => Some(c.as_str().filter(|x| digits(x))?.to_string()),
                };
                Command::SignOn { number, code }
            }
            "sign_off" => Command::SignOff,
            "accept" => Command::Accept,
            "free" => Command::Free,
            "break" => Command::Break(v.get("on").and_then(|x| x.as_bool())?),
            "lines" => Command::Lines,
            "tours" => Command::Tours { line: n("line")? },
            "pick" => Command::Pick { line: n("line")?, tour: n("tour")? },
            what @ ("pointer" | "tap" | "page") => {
                let kind = match s("kind")? {
                    "down" => Pointer::Down,
                    "move" => Pointer::Move,
                    "up" => Pointer::Up,
                    _ => return None,
                };
                match what {
                    "tap" => Command::Tap { screen: screen()?, kind, x: f("x")?, y: f("y")? },
                    "page" => Command::Page { screen: screen()?, page: n("page")?, kind, u: f("u")?, v: f("v")? },
                    _ => Command::Pointer { screen: screen()?, kind, u: f("u")?, v: f("v")? },
                }
            }
            "touch" => Command::Touch { screen: screen()?, touch: n("touch")?, down: v.get("down").and_then(|x| x.as_bool())? },
            "key" => Command::Key { screen: screen()?, key: n("key")?, down: v.get("down").and_then(|x| x.as_bool())? },
            _ => return None,
        })
    }
}

/// An answer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Response {
    pub status: u16,
    pub ctype: &'static str,
    pub body: Vec<u8>,
    pub headers: Vec<(&'static str, String)>,
}

impl Response {
    pub(crate) fn new(status: u16, ctype: &'static str, body: impl Into<Vec<u8>>) -> Response {
        Response { status, ctype, body: body.into(), headers: Vec::new() }
    }

    pub(crate) fn json(v: &Value) -> Response {
        Response::new(200, "application/json; charset=utf-8", v.to_string())
    }

    pub(crate) fn status(status: u16) -> Response {
        Response::new(status, "text/plain; charset=utf-8", reason(status))
    }

    pub(crate) fn with(mut self, name: &'static str, value: impl Into<String>) -> Response {
        self.headers.push((name, value.into()));
        self
    }

    /// The whole answer as it goes on the wire; `head_only` for HEAD.
    pub(crate) fn to_bytes(&self, head_only: bool) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n",
            self.status,
            reason(self.status),
            self.ctype,
            self.body.len()
        );
        for (k, v) in &self.headers {
            // (no line breaks into the head from a value)
            out.push_str(&format!("{k}: {}\r\n", v.replace(['\r', '\n'], " ")));
        }
        out.push_str("\r\n");
        let mut bytes = out.into_bytes();
        if !head_only {
            bytes.extend_from_slice(&self.body);
        }
        bytes
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// The page's content security policy: its own files and nothing else; pictures also as
/// `blob:` (the screens are fetched with the device key and shown from a blob).
pub(crate) const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob: data:; connect-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

#[cfg(test)]
mod tests {
    use super::*;

    fn req(text: &str) -> Request {
        parse_head(text.as_bytes()).unwrap().0
    }

    #[test]
    fn a_request_head_is_read_with_its_query_and_headers() {
        let r = req("GET /api/frame?screen=s3&after=12 HTTP/1.1\r\nHost: 192.168.1.20\r\nX-Companion-Key: abc\r\n\r\n");
        assert_eq!((r.method.as_str(), r.path.as_str()), ("GET", "/api/frame"));
        assert_eq!(r.query("screen"), Some("s3"));
        assert_eq!(r.header("x-companion-key"), Some("abc"));
        assert_eq!(route(&r), Route::Frame("s3".into(), 12));
        assert_eq!(parse_head(b"GET / HTTP/1.1\r\nHost: x\r\n"), Err(HttpError::Incomplete));
        assert_eq!(parse_head(&vec![b'a'; MAX_HEAD + 1]), Err(HttpError::TooLarge));
        assert_eq!(parse_head(b"get / HTTP/1.1\r\n\r\n"), Err(HttpError::Bad));
        assert_eq!(parse_head(b"GET /../x HTTP/1.1\r\n\r\n"), Err(HttpError::Bad));
        assert_eq!(parse_head(b"GET /%2e%2e/x HTTP/1.1\r\n\r\n"), Err(HttpError::Bad));
    }

    #[test]
    fn routes_answer_only_their_own_method_and_unknown_paths_are_not_found() {
        assert_eq!(route(&req("GET / HTTP/1.1\r\n\r\n")), Route::Asset(0));
        assert_eq!(route(&req("GET /app.js HTTP/1.1\r\n\r\n")), Route::Asset(1));
        assert_eq!(route(&req("POST / HTTP/1.1\r\n\r\n")), Route::Method);
        assert_eq!(route(&req("GET /api/pair HTTP/1.1\r\n\r\n")), Route::Method);
        assert_eq!(route(&req("GET /settings.cfg HTTP/1.1\r\n\r\n")), Route::NotFound);
        assert_eq!(route(&req("GET /api/frame?screen=../x HTTP/1.1\r\n\r\n")), Route::NotFound);
        assert_eq!(route(&req("GET /api/texts?lang=nl%22 HTTP/1.1\r\n\r\n")), Route::Texts("nl".into()));
        assert!(route(&req("POST /api/do HTTP/1.1\r\n\r\n")).needs_key());
        assert!(!route(&req("POST /api/pair HTTP/1.1\r\n\r\n")).needs_key());
    }

    #[test]
    fn the_navigators_routes_want_a_key_and_a_sensible_place() {
        assert_eq!(route(&req("GET /api/nav?after=7 HTTP/1.1\r\n\r\n")), Route::Nav(7));
        assert_eq!(route(&req("GET /api/trip HTTP/1.1\r\n\r\n")), Route::Trip);
        assert_eq!(route(&req("GET /api/qr HTTP/1.1\r\n\r\n")), Route::PairQr);
        assert_eq!(route(&req("GET /api/roads?x=-120.5&y=3000 HTTP/1.1\r\n\r\n")), Route::Roads { x: -120.5, y: 3000.0, r: 1600.0, tol: 0.0 });
        assert_eq!(route(&req("GET /api/roads?x=1&y=2&r=99999&tol=-3 HTTP/1.1\r\n\r\n")), Route::Roads { x: 1.0, y: 2.0, r: 6000.0, tol: 0.0 });
        assert_eq!(route(&req("GET /api/roads?x=1 HTTP/1.1\r\n\r\n")), Route::NotFound);
        assert_eq!(route(&req("GET /api/roads?x=NaN&y=1 HTTP/1.1\r\n\r\n")), Route::NotFound);
        assert_eq!(route(&req("GET /api/roads?x=1e9&y=1 HTTP/1.1\r\n\r\n")), Route::NotFound);
        assert_eq!(route(&req("POST /api/trip HTTP/1.1\r\n\r\n")), Route::Method);
        assert_eq!(route(&req("GET /api/view?screen=s4&after=3 HTTP/1.1\r\n\r\n")), Route::View("s4".into(), 3));
        assert_eq!(route(&req("GET /api/view?screen=../4 HTTP/1.1\r\n\r\n")), Route::NotFound);
        assert_eq!(route(&req("GET /api/form?screen=p40 HTTP/1.1\r\n\r\n")), Route::Form("p40".into()));
        assert_eq!(route(&req("GET /api/live?screen=t3&after=9 HTTP/1.1\r\n\r\n")), Route::Live("t3".into(), 9));
        assert_eq!(route(&req("GET /api/live?screen=../3 HTTP/1.1\r\n\r\n")), Route::NotFound);
        assert_eq!(route(&req("GET /api/tex?n=12 HTTP/1.1\r\n\r\n")), Route::Tex(12));
        assert_eq!(route(&req("GET /api/font?n=1 HTTP/1.1\r\n\r\n")), Route::Font(1));
        assert_eq!(route(&req("GET /api/fontimg?n=1 HTTP/1.1\r\n\r\n")), Route::FontImg(1));
        assert_eq!(route(&req("GET /api/tex?n=x HTTP/1.1\r\n\r\n")), Route::NotFound);
        assert_eq!(route(&req("GET /api/frame?screen=x3 HTTP/1.1\r\n\r\n")), Route::Frame("x3".into(), 0));
        for r in ["/api/nav", "/api/trip", "/api/roads?x=0&y=0", "/api/qr", "/api/view?screen=s1", "/api/form?screen=s1", "/api/live?screen=s1", "/api/tex?n=0", "/api/font?n=0", "/api/fontimg?n=0"] {
            assert!(route(&req(&format!("GET {r} HTTP/1.1\r\n\r\n"))).needs_key(), "{r}");
        }
    }

    #[test]
    fn the_body_has_a_limit() {
        let r = req("POST /api/do HTTP/1.1\r\nContent-Length: 5000\r\n\r\n");
        assert_eq!(body_length(&r), Err(HttpError::TooLarge));
        let r = req("POST /api/do HTTP/1.1\r\nContent-Length: 12\r\n\r\n");
        assert_eq!(body_length(&r), Ok(12));
    }

    #[test]
    fn only_the_listed_commands_get_through() {
        let c = |t: &str| Command::from_json(&serde_json::from_str(t).unwrap());
        assert_eq!(c(r#"{"do":"sign_on","number":"482913"}"#), Some(Command::SignOn { number: "482913".into(), code: None }));
        assert_eq!(c(r#"{"do":"sign_on","number":"482913","code":"5821"}"#), Some(Command::SignOn { number: "482913".into(), code: Some("5821".into()) }));
        assert_eq!(c(r#"{"do":"sign_on","number":"48a913"}"#), None);
        assert_eq!(c(r#"{"do":"sign_on","number":"482913","code":12}"#), None);
        assert_eq!(c(r#"{"do":"pick","line":2,"tour":0}"#), Some(Command::Pick { line: 2, tour: 0 }));
        assert_eq!(c(r#"{"do":"pointer","screen":"s7","kind":"down","u":1.4,"v":0.25}"#), Some(Command::Pointer { screen: "s7".into(), kind: Pointer::Down, u: 1.0, v: 0.25 }));
        assert_eq!(c(r#"{"do":"pointer","screen":"q7","kind":"down","u":0,"v":0}"#), None);
        assert_eq!(c(r#"{"do":"touch","screen":"p12","touch":4,"down":true}"#), Some(Command::Touch { screen: "p12".into(), touch: 4, down: true }));
        assert_eq!(c(r#"{"do":"touch","screen":"p12","touch":-1,"down":true}"#), None);
        assert_eq!(c(r#"{"do":"page","screen":"s2","page":2,"kind":"move","u":0.5,"v":1.5}"#), Some(Command::Page { screen: "s2".into(), page: 2, kind: Pointer::Move, u: 0.5, v: 1.0 }));
        assert_eq!(c(r#"{"do":"key","screen":"t2","key":3,"down":true}"#), Some(Command::Key { screen: "t2".into(), key: 3, down: true }));
        assert_eq!(c(r#"{"do":"tap","screen":"s3","kind":"up","x":0.25,"y":-2}"#), Some(Command::Tap { screen: "s3".into(), kind: Pointer::Up, x: 0.25, y: 0.0 }));
        assert_eq!(c(r#"{"do":"tap","screen":"s3","kind":"down","u":0.2,"v":0.2}"#), None);
        assert_eq!(c(r#"{"do":"break","on":true}"#), Some(Command::Break(true)));
        assert_eq!(c(r#"{"do":"delete_profile"}"#), None);
        assert_eq!(c(r#"{"number":"1"}"#), None);
    }

    #[test]
    fn an_answer_says_what_it_is_and_keeps_header_values_on_one_line() {
        let r = Response::new(200, "text/plain", "hi").with("X-Seq", "4\r\nEvil: 1").to_bytes(false);
        let text = String::from_utf8(r).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Length: 2\r\n"));
        assert!(text.contains("X-Seq: 4  Evil: 1\r\n"));
        assert!(text.ends_with("\r\n\r\nhi"));
        let head = String::from_utf8(Response::status(404).to_bytes(true)).unwrap();
        assert!(head.ends_with("\r\n\r\n"));
    }
}
