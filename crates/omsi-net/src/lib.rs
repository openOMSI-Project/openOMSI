//! LAN play: several players on one map see and hear each other's buses and talk.
//!
//! One machine hosts (`LanSession::host`), the others join it (`LanSession::join`, or
//! `LanSession::discover` to find a host on the local network by broadcast). Everybody
//! sends the state of their own vehicle up to twenty times a second over UDP (five times a
//! second while nothing changes; a player without a vehicle sends an empty state once a
//! second, so that the others know it is still there); the host relays every state to
//! every other player (a star), so a client only ever talks to the host - and listens to
//! nobody else. The host also owns the world: a joining player takes the host's date, time
//! of day, weather and season (`Welcome::world`), and the host's clock keeps everybody's in
//! step (`CLOCK`).
//!
//! A host is found by its **session code** (`SessionCode`): the host's LAN address, its
//! port and a random session id with a checksum, written in a base-32 alphabet without
//! look-alike characters (`OMSI-7Q4K-2M9X-HD3P-R8TZ-KC5W-NB6E`). A joining player may give
//! the code, an `ip`, `ip:port`, a bare port (a host on this machine) or nothing (search
//! the network): `parse_join`.
//!
//! The vehicle state is a bit-packed binary datagram (`wire`: position, pitch and bank,
//! speed, steering, lights, indicators, engine speed and pedals, doors, wheel travel, the
//! rear sections of an articulated bus, and the vehicle's own lamp, switch and sound
//! variables). Everything else is plain text, one message per datagram, with `|` between
//! the fields. Every message that opens a conversation carries the protocol version
//! (`PROTOCOL`), and a host turns away a client of another version (with a message saying
//! so) or with the code of another session:
//!
//! ```text
//! HELLO|<proto>|<session or ->|<name>|<bus>|<map>|<date>|<time>|<weather>|<season>|<nonce>
//!                                                client → host (every second until answered)
//! WELCOME|<proto>|<id>|<session>|<host name>|<map>|<date>|<time>|<weather>|<season>|<players>
//!                                                host → client
//! REJECT|<proto>|<reason>                        host → client
//! DISCOVER|<proto>                               broadcast, client → any host
//! HERE|<proto>|<host name>|<session>|<map>|<players>
//!                                                host → client
//! INFO|<id>|<name>|<bus>|<paint>|<line>|<destination>|length|width|box offset|<table>|<tour>|<display texts, hex, comma separated>|<figure .hum>
//!                                                every two seconds and on a change; relayed
//! PLACE|<id>|x|y|z|heading|length|width          client → host, once its bus stands
//! NEAR|<id>|<footprints>                         host → client
//! CLOCK|<map>|<date>|<time>|<weather>|<season>   host → clients, every five seconds
//! CHAT|<id>|<text>                               client → host
//! SAY|<id>|<name>|<text>                         host → clients (a chat line; to one client
//!                                                 alone: an admin's private word)
//! NOTE|<text>                                    host → clients (joined, left, who is here)
//! BYE|<id>
//! WORLD (binary, see `world`)                    host → client: the traffic, people and
//!                                                light programs around the client; client →
//!                                                host: its own people on foot
//! DESC|c|<id>|<file>|<scheme>|<line>|<dest>      host → client: what a car is
//! DESC|p|<id>|<file>                             either way: what a person is
//! WANT|<id>|c<id>,p<id>,…                        client → host: descriptions lost on the way
//! CLAIM|<id>|<person>,…                          client → host: waiting people its bus takes
//! GRANT|<person>,… / DENY|<person>,…             host → client: handed over, or not
//! ```
//!
//! `<footprints>` are the vehicles standing near the place a client's bus was put at
//! (`x,y,z,heading,length,width` separated by `;`), so that the client can move it in
//! front of or behind them instead of into them. `<table>` is the hash of the vehicle's
//! sync table (which lamp, switch and sound variables the state lists, in which order):
//! two games with different versions of a vehicle leave those lists alone.
//!
//! The host checks everything it takes in: a message must come from the address the
//! player joined from and carry that player's id, text is cut to size and cleaned of
//! separators and control characters, numbers must be finite and in range, vehicle paths
//! must stay inside a content folder, and every player may send only so much per second.
//!
//! Nothing here knows about rendering: the game turns the remote states into vehicles it
//! draws and hears like AI traffic, and draws the host's world (`omsi-app::lan_world`).
//! The host simulates the traffic, the timetable buses, the people and the traffic lights
//! for everybody; a client simulates its own bus and the passengers the host hands over to
//! it. Money and the timetable of the player's own duty stay local to each player.
//!
//! A host is joined by its code, or by any of its addresses (`addrs`): the code carries up
//! to three of them (a VPN's first - Hamachi, Radmin VPN, ZeroTier, Tailscale - then the
//! LAN's), and a joining game says hello to all of them at once, takes the first that
//! answers, and gives up with a message saying why that may be after `JOIN_TIMEOUT`.

pub mod addrs;
pub mod bridge;
pub mod wire;
pub mod world;
pub mod vars;
pub mod ws;
pub mod tunnel;
pub mod official;
pub mod lobby;

use std::cell::Cell;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

pub use wire::{
    FLAG_BRAKE, FLAG_ELECTRICS, FLAG_ENGINE, FLAG_FOG, FLAG_HORN, FLAG_KNEELING, FLAG_REVERSE,
    FLAG_STOP_BRAKE, FLAG_VEHICLE, FLAG_WIPERS,
};

/// Version of the message format. Raised whenever a message changes shape.
/// 3: binary vehicle states, INFO, the host's world and clock, chat.
/// 4: the host's traffic, people and light programs (`world`), a nonce in the hello (a
/// client may try several addresses of the host at once), codes with several addresses.
/// 5: the tour a player drives in `INFO` (the host's timetable leaves it out), riders of
/// the players' buses in the world frames (`world::PLAYER_BUS`), and the players' people
/// passed on to the other players.
/// 6: up to 63 sound and moving-part values in a state (a 6-bit count: the AA-FR Agora's
/// sound variables alone filled the 31 there was room for).
pub const PROTOCOL: u32 = 6;
/// (omsi-plugin's `MULTIPLAYER_PORTS` keeps `omsi.send` off this one and the `PORT_RANGE` after it.)
pub const DEFAULT_PORT: u16 = 27015;
/// Ports a host tries after the default one when that is taken (a second session on the
/// same machine).
pub const PORT_RANGE: u16 = 10;
/// States per second each player sends while its vehicle moves or changes.
pub const RATE_HZ: f32 = 20.0;
/// ... and while nothing has changed for a second.
pub const IDLE_RATE_HZ: f32 = 5.0;
/// A player who has not been heard from for this long is gone.
pub const TIMEOUT: Duration = Duration::from_secs(15);
/// A joining game that has no answer from any of the host's addresses within this long
/// gives up with a message saying why that may be (it used to say hello for ever).
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
/// A client that lost its host tries to reach it again for this long, then plays on alone.
pub const RECONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// A player who joined but has sent no state yet is loading the map: it may stay silent
/// this long.
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// A player without a vehicle sends its empty state this often (s), to stay in the session.
pub const HEARTBEAT: f32 = 1.0;
/// Seconds between two INFO messages of a player whose info has not changed.
pub const INFO_EVERY: f32 = 2.0;
/// The shortest time (s) between two `INFO`s: well inside what a host takes from a player
/// (`MESSAGE_RATE`), with room for its other messages.
pub const INFO_MIN_GAP: f32 = 0.25;
/// Seconds between two CLOCK messages of the host.
pub const CLOCK_EVERY: f32 = 5.0;
/// At most this many other players: a host turns away the next one, a client ignores more.
pub const MAX_PEERS: usize = 32;
/// At most this many players may be loading at the same time (joined, no state yet).
pub const MAX_JOINING: usize = 8;
/// How far around the requested spawn a host lists the vehicles standing there (m).
pub const FOOTPRINT_RADIUS: f64 = 250.0;
/// At most this many footprints go into a NEAR (it has to fit into one datagram).
pub const MAX_FOOTPRINTS: usize = 28;
/// Longest datagram taken in; anything longer is not ours.
pub const MAX_DATAGRAM: usize = 1400;
/// Longest player name, chat line, text field (characters).
pub const MAX_NAME: usize = 32;
pub const MAX_CHAT: usize = 160;
const MAX_FIELD: usize = 64;
/// A host takes at most this many states and this many other messages a second from one
/// player (with bursts of as many); chat lines one a second (bursts of three).
const STATE_RATE: (f32, f32) = (40.0, 40.0);
const MESSAGE_RATE: (f32, f32) = (10.0, 20.0);
const CHAT_RATE: (f32, f32) = (1.0, 3.0);
// ---------------------------------------------------------------------------------------
// session codes

/// The code alphabet: 24 letters without I and O, and the digits 2-9 - nothing that reads
/// like something else (0/O, 1/I/L).
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Characters of a code without the `OMSI-` prefix and the dashes.
pub const CODE_CHARS: usize = 24;

/// What a session code carries: where the host is and which session it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCode {
    /// Protocol version of the host that made the code.
    pub protocol: u8,
    /// The host's addresses, best first (a VPN's, then the LAN's; see `addrs`): the joining
    /// game tries them all at once and takes the one that answers. At least one, at most
    /// `MAX_CODE_ADDRS`.
    pub ips: Vec<Ipv4Addr>,
    pub port: u16,
    /// Random 48-bit session id.
    pub session: u64,
}

/// At most this many addresses go into a code (each makes it 6-7 characters longer).
pub const MAX_CODE_ADDRS: usize = 3;

/// CRC-16/CCITT-FALSE.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Marks the checksum of a code in the second layout (the session id first, one address
/// hidden under it); a code of the first layout has the plain checksum.
const CODE_LAYOUT_MARK: u16 = 0x4F43;
/// ... and of the third layout: the session id, then protocol, port and several addresses
/// hidden under the mask (as many as the length says).
const CODE_MULTI_MARK: u16 = 0x4D41;

/// The bytes that hide protocol, address and port: drawn from the session id, so two
/// sessions of one computer do not share the start of their codes.
fn code_mask(session: &[u8], len: usize) -> Vec<u8> {
    let mut z = session.iter().fold(0u64, |a, b| (a << 8) | *b as u64) ^ 0x5DEE_CE66_D1CE_4E5B;
    let mut out = vec![0u8; len];
    for (i, o) in out.iter_mut().enumerate() {
        // splitmix64
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut x = z;
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        *o = ((x ^ (x >> 31)) >> ((i % 8) * 3)) as u8;
    }
    out
}

const MASK48: u64 = 0xFFFF_FFFF_FFFF;
const MIX48: [u64; 2] = [0x9E37_79B9_7F4B, 0xC2B2_AE3D_27D5];

/// The inverse of an odd number modulo 2^48 (Newton's iteration).
fn inverse48(c: u64) -> u64 {
    let mut inv = c;
    for _ in 0..5 {
        inv = inv.wrapping_mul(2u64.wrapping_sub(c.wrapping_mul(inv)));
    }
    inv & MASK48
}

/// A reversible scramble of a 48-bit session id: ids that differ in one bit are written
/// differently all along.
fn mix48(mut x: u64) -> u64 {
    x &= MASK48;
    for c in MIX48 {
        x ^= x >> 24;
        x = x.wrapping_mul(c) & MASK48;
    }
    x ^ (x >> 24)
}

fn unmix48(mut x: u64) -> u64 {
    x &= MASK48;
    for c in MIX48.iter().rev() {
        x ^= x >> 24;
        x = x.wrapping_mul(inverse48(*c)) & MASK48;
    }
    x ^ (x >> 24)
}

impl SessionCode {
    /// A code for one address.
    pub fn single(protocol: u8, ip: Ipv4Addr, port: u16, session: u64) -> SessionCode {
        SessionCode {
            protocol,
            ips: vec![ip],
            port,
            session,
        }
    }

    /// The first (best) address.
    pub fn ip(&self) -> Ipv4Addr {
        self.ips.first().copied().unwrap_or(Ipv4Addr::LOCALHOST)
    }

    /// `OMSI-XXXX-XXXX-…`: the session id first, then protocol, port and address(es) hidden
    /// under a mask drawn from the id, then the checksum, in a base-32 alphabet in groups of
    /// four. One address makes 15 bytes = 24 characters (the second layout: the protocol,
    /// the address and the port); more addresses make the third layout, 11 + 4 per address
    /// bytes (31 characters for two, 37 for three). With the address first, every code of
    /// one computer began with the same 14 characters and two sessions were easy to confuse.
    pub fn encode(&self) -> String {
        let ips: Vec<Ipv4Addr> = if self.ips.is_empty() {
            vec![Ipv4Addr::LOCALHOST]
        } else {
            self.ips.iter().copied().take(MAX_CODE_ADDRS).collect()
        };
        let mut bytes = Vec::with_capacity(11 + 4 * ips.len());
        let sid = mix48(self.session).to_be_bytes();
        bytes.extend_from_slice(&sid[2..8]);
        let mut plain = Vec::with_capacity(5 + 4 * ips.len());
        plain.push(self.protocol);
        let mark = if ips.len() == 1 {
            plain.extend_from_slice(&ips[0].octets());
            plain.extend_from_slice(&self.port.to_be_bytes());
            CODE_LAYOUT_MARK
        } else {
            plain.extend_from_slice(&self.port.to_be_bytes());
            for ip in &ips {
                plain.extend_from_slice(&ip.octets());
            }
            CODE_MULTI_MARK
        };
        let mask = code_mask(&sid[2..8], plain.len());
        bytes.extend(plain.iter().zip(mask).map(|(p, m)| p ^ m));
        let crc = crc16(&bytes) ^ mark;
        bytes.extend_from_slice(&crc.to_be_bytes());
        Self::to_text(&bytes)
    }

    /// The first layout (protocol, IPv4, port, session id in the clear): still read.
    #[cfg(test)]
    fn encode_first_layout(&self) -> String {
        let mut bytes = Vec::with_capacity(15);
        bytes.push(self.protocol);
        bytes.extend_from_slice(&self.ip().octets());
        bytes.extend_from_slice(&self.port.to_be_bytes());
        bytes.extend_from_slice(&self.session.to_be_bytes()[2..8]);
        let crc = crc16(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        Self::to_text(&bytes)
    }

    /// Characters a code of `bytes` bytes has.
    fn chars_for(bytes: usize) -> usize {
        (bytes * 8).div_ceil(5)
    }

    fn to_text(bytes: &[u8]) -> String {
        // the bytes as a bit string, 5 bits a character (the last one padded with zeros)
        let n = Self::chars_for(bytes.len());
        let mut chars = Vec::with_capacity(n);
        for i in 0..n {
            let mut v = 0usize;
            for k in 0..5 {
                let bit = i * 5 + k;
                let b = bytes.get(bit / 8).map(|x| (x >> (7 - bit % 8)) & 1).unwrap_or(0);
                v = (v << 1) | b as usize;
            }
            chars.push(ALPHABET[v] as char);
        }
        // (the last group filled up to four with the zero character: a code of two
        // addresses ended in a group of three, and players took it for cut short, #152)
        while chars.len() % 4 != 0 {
            chars.push(ALPHABET[0] as char);
        }
        let groups: Vec<String> = chars.chunks(4).map(|c| c.iter().collect()).collect();
        format!("OMSI-{}", groups.join("-"))
    }

    /// A code's characters (after `OMSI-`) without the filling of its last group, or None
    /// when it has no length a code has.
    fn unpadded(s: &str) -> Option<&str> {
        let lengths = Self::valid_lengths();
        if lengths.contains(&s.len()) {
            return Some(s);
        }
        lengths
            .into_iter()
            .find(|&l| l < s.len() && l.div_ceil(4) * 4 == s.len() && s.as_bytes()[l..].iter().all(|c| *c == ALPHABET[0]))
            .map(|l| &s[..l])
    }

    /// The lengths a code is written with (its last group filled to four).
    fn written_lengths() -> Vec<usize> {
        Self::valid_lengths().into_iter().map(|l| l.div_ceil(4) * 4).collect()
    }

    /// The code lengths (characters after `OMSI-`) that exist: one address, or two to
    /// `MAX_CODE_ADDRS`.
    pub fn valid_lengths() -> Vec<usize> {
        std::iter::once(CODE_CHARS)
            .chain((2..=MAX_CODE_ADDRS).map(|n| Self::chars_for(11 + 4 * n)))
            .collect()
    }

    /// Read a code as a person may type it: any case, with or without the `OMSI` prefix,
    /// dashes and spaces anywhere.
    pub fn decode(text: &str) -> Result<SessionCode, String> {
        let mut s: String = text
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
            .collect::<String>()
            .to_ascii_uppercase();
        let lengths = Self::written_lengths();
        // (O and I are not in the alphabet: a code itself never starts with OMSI)
        if s.starts_with("OMSI") {
            s = s[4..].to_string();
        }
        if let Some(u) = Self::unpadded(&s) {
            s = u.to_string();
        } else {
            let n = s.len();
            return Err(format!(
                "a session code has {} characters after OMSI- (this one has {n}) - copy the whole code",
                lengths
                    .iter()
                    .map(|l| l.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
                    .replacen(", ", " or ", usize::from(lengths.len() == 2))
            ));
        }
        let mut bits: Vec<u8> = Vec::with_capacity(s.len() * 5);
        for c in s.bytes() {
            let v = match ALPHABET.iter().position(|a| *a == c) {
                Some(v) => v,
                None if matches!(c, b'0' | b'O' | b'1' | b'I') => {
                    return Err(format!(
                    "'{}' never appears in a session code - check the character that looks like it",
                    c as char
                ))
                }
                None => return Err(format!("'{}' is not part of a session code", c as char)),
            };
            for k in (0..5).rev() {
                bits.push(((v >> k) & 1) as u8);
            }
        }
        let n_bytes = bits.len() / 8;
        let bytes: Vec<u8> = (0..n_bytes)
            .map(|i| bits[i * 8..i * 8 + 8].iter().fold(0u8, |a, b| (a << 1) | b))
            .collect();
        let crc = u16::from_be_bytes([bytes[n_bytes - 2], bytes[n_bytes - 1]]);
        let body = &bytes[..n_bytes - 2];
        let sum = crc16(body);
        let mut sid = [0u8; 8];
        sid[2..8].copy_from_slice(&body[0..6]);
        let unmask = |from: usize| -> Vec<u8> {
            let mask = code_mask(&body[0..6], body.len() - from);
            body[from..].iter().zip(mask).map(|(b, m)| b ^ m).collect()
        };
        let (protocol, ips, port, session) = if n_bytes == 15 && sum ^ CODE_LAYOUT_MARK == crc {
            let p = unmask(6);
            (
                p[0],
                vec![Ipv4Addr::new(p[1], p[2], p[3], p[4])],
                u16::from_be_bytes([p[5], p[6]]),
                unmix48(u64::from_be_bytes(sid)),
            )
        } else if n_bytes == 15 && sum == crc {
            let mut sid = [0u8; 8];
            sid[2..8].copy_from_slice(&body[7..13]);
            (
                body[0],
                vec![Ipv4Addr::new(body[1], body[2], body[3], body[4])],
                u16::from_be_bytes([body[5], body[6]]),
                u64::from_be_bytes(sid),
            )
        } else if n_bytes > 15 && sum ^ CODE_MULTI_MARK == crc {
            let p = unmask(6);
            let ips = p[3..]
                .chunks_exact(4)
                .map(|o| Ipv4Addr::new(o[0], o[1], o[2], o[3]))
                .collect();
            (
                p[0],
                ips,
                u16::from_be_bytes([p[1], p[2]]),
                unmix48(u64::from_be_bytes(sid)),
            )
        } else {
            return Err("the session code has a typo (its checksum does not match)".into());
        };
        Ok(SessionCode {
            protocol,
            ips,
            port,
            session,
        })
    }

    /// Where the host may be reached, best first.
    pub fn addrs(&self) -> Vec<SocketAddr> {
        self.ips
            .iter()
            .map(|ip| SocketAddr::from((*ip, self.port)))
            .collect()
    }
}

/// Does `text` look like a session code rather than an address?
pub fn looks_like_code(text: &str) -> bool {
    let t = text.trim().to_ascii_uppercase();
    if t.contains(['.', ':']) {
        return false;
    }
    let s: String = t
        .chars()
        .filter(|c| *c != '-' && *c != '_' && !c.is_whitespace())
        .collect();
    if !s.bytes().all(|c| c.is_ascii_alphanumeric()) || !s.bytes().any(|c| c.is_ascii_alphabetic())
    {
        return false;
    }
    // the full code, the code without its prefix, or something that was meant to be one
    // (the prefix and a few groups: a code cut short while copying)
    let ok = |s: &str| SessionCode::unpadded(s).is_some();
    ok(&s) || (s.starts_with("OMSI") && (ok(&s[4..]) || (t.starts_with("OMSI-") && s.len() >= 8)))
}

/// A session id as it is written in messages (12 hex digits).
pub fn session_hex(id: u64) -> String {
    format!("{:012X}", id & 0xFFFF_FFFF_FFFF)
}

fn parse_session_hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim(), 16).ok()
}

/// A fresh random 48-bit session id (the hasher's per-process random keys, the clock and
/// the process id - no dependency needed for this).
pub fn random_session_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.write_u32(std::process::id());
    let v = h.finish() & 0xFFFF_FFFF_FFFF;
    if v == 0 {
        1
    } else {
        v
    }
}

/// The addresses a session code carries: those of this machine another player can reach
/// (`addrs::joinable_addresses`: a gaming VPN's first, then the LAN's), at most
/// `MAX_CODE_ADDRS`; the loopback when there is none (a computer without a network).
///
/// This used to be the first private address `ifconfig` listed - on a Mac with Hamachi and
/// Tailscale that was the home LAN's 192.168.1.x, which a friend on Hamachi cannot reach,
/// and a 169.254 address of a dead adapter counted as private as well.
pub fn code_ipv4s() -> Vec<Ipv4Addr> {
    let mut ips: Vec<Ipv4Addr> = addrs::joinable_addresses().iter().map(|a| a.ip).collect();
    ips.truncate(MAX_CODE_ADDRS);
    if ips.is_empty() {
        ips.push(Ipv4Addr::LOCALHOST);
    }
    ips
}

/// This machine's best address for another player (the first of `code_ipv4s`).
pub fn lan_ipv4() -> Ipv4Addr {
    code_ipv4s()[0]
}

/// Where a joining player wants to go.
#[derive(Debug, Clone, PartialEq)]
pub enum JoinTarget {
    /// Look for a host on the local network.
    Discover,
    /// These addresses (tried all at once, the first that answers is taken); `session` is
    /// set when the player gave a session code.
    Direct {
        addrs: Vec<SocketAddr>,
        session: Option<u64>,
        protocol: Option<u8>,
    },
}

/// What the text of a join field means, without looking any names up (the launcher checks
/// the field as it is typed): a description, or what is wrong with it.
pub fn describe_join(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty()
        || t.eq_ignore_ascii_case("auto")
        || t.eq_ignore_ascii_case("discover")
        || t.eq_ignore_ascii_case("search")
    {
        return Ok("search the local network for a hosted session".into());
    }
    if let Some(url) = ws::ws_url(t) {
        return Ok(format!("a server on the internet, reached over a WebSocket ({url})"));
    }
    if looks_like_code(t) {
        let c = SessionCode::decode(t)?;
        if c.protocol as u32 != PROTOCOL {
            return Err(format!("this code was made by a game with LAN protocol {}, this one speaks {PROTOCOL} - both players need the same version", c.protocol));
        }
        let at: Vec<String> = c
            .ips
            .iter()
            .map(|ip| format!("{ip}:{} ({})", c.port, addrs::classify(*ip, "").label()))
            .collect();
        return Ok(format!(
            "session {} hosted at {}",
            session_hex(c.session),
            at.join(" or ")
        ));
    }
    if t.bytes().all(|b| b.is_ascii_digit()) {
        return match t.parse::<u32>() {
            Ok(p) if (1..=65535).contains(&p) => {
                Ok(format!("a session hosted on this computer, port {p}"))
            }
            _ => Err(format!("{t} is not a port (1 to 65535)")),
        };
    }
    if let Ok(a) = t.parse::<SocketAddr>() {
        return Ok(format!("the host at {a}"));
    }
    if let Ok(ip) = t.parse::<std::net::IpAddr>() {
        return Ok(format!("the host at {ip}, port {DEFAULT_PORT}"));
    }
    let (host, port) = match t.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(p) if p > 0 => (h, p),
            _ => {
                return Err(format!(
                    "the part after ':' must be a port (1 to 65535), not '{p}'"
                ))
            }
        },
        None => (t, DEFAULT_PORT),
    };
    let name_ok = !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && host.bytes().any(|b| b.is_ascii_alphabetic());
    if name_ok
        && (host.contains('.')
            || host.eq_ignore_ascii_case("localhost")
            || !host
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()))
    {
        return Ok(format!("the computer named {host}, port {port}"));
    }
    Err(format!("'{t}' is neither a session code (OMSI-XXXX-…), an address (192.168.1.20 or 192.168.1.20:27015), a port, nor empty (search)"))
}

/// Read what a player typed into the join field: a session code, `ip`, `ip:port`,
/// `host:port`, a bare port (a host on this machine), or nothing / `auto` (search).
pub fn parse_join(text: &str) -> Result<JoinTarget, String> {
    let t = text.trim();
    if t.is_empty()
        || t.eq_ignore_ascii_case("auto")
        || t.eq_ignore_ascii_case("discover")
        || t.eq_ignore_ascii_case("search")
    {
        return Ok(JoinTarget::Discover);
    }
    if looks_like_code(t) {
        let c = SessionCode::decode(t)?;
        return Ok(JoinTarget::Direct {
            addrs: c.addrs(),
            session: Some(c.session),
            protocol: Some(c.protocol),
        });
    }
    if t.bytes().all(|b| b.is_ascii_digit()) {
        return match t.parse::<u32>() {
            Ok(p) if (1..=65535).contains(&p) => Ok(JoinTarget::Direct {
                addrs: vec![SocketAddr::from((Ipv4Addr::LOCALHOST, p as u16))],
                session: None,
                protocol: None,
            }),
            _ => Err(format!("{t} is not a port (1 to 65535)")),
        };
    }
    if let Ok(a) = t.parse::<SocketAddr>() {
        return Ok(JoinTarget::Direct {
            addrs: vec![a],
            session: None,
            protocol: None,
        });
    }
    if let Ok(ip) = t.parse::<std::net::IpAddr>() {
        return Ok(JoinTarget::Direct {
            addrs: vec![SocketAddr::new(ip, DEFAULT_PORT)],
            session: None,
            protocol: None,
        });
    }
    // a host name, with or without a port
    let (host, port) = match t.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(p) if p > 0 => (h, p),
            _ => {
                return Err(format!(
                    "'{t}': the part after ':' must be a port (1 to 65535)"
                ))
            }
        },
        None => (t, DEFAULT_PORT),
    };
    // (a name of digits only would be read as a numeric address by the resolver: "27015:27015")
    if host.is_empty()
        || host.contains(char::is_whitespace)
        || host.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return Err(format!("'{t}' is neither a session code (OMSI-…), an address (192.168.1.20 or 192.168.1.20:27015) nor a port"));
    }
    match (host, port).to_socket_addrs() {
        Ok(mut it) => match it.find(|a| a.is_ipv4()) {
            Some(a) => Ok(JoinTarget::Direct { addrs: vec![a], session: None, protocol: None }),
            None => Err(format!("{host} has no IPv4 address")),
        },
        Err(e) => Err(format!("'{t}' is neither a session code (OMSI-…), an address nor a port, and no computer of that name was found ({e})")),
    }
}

// ---------------------------------------------------------------------------------------
// messages

/// What the world of a player looks like: a joining client takes the host's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldInfo {
    /// Map file relative to the game root (`maps/Grundorf/global.cfg`).
    pub map: String,
    /// YYYY-MM-DD.
    pub date: String,
    /// Seconds since midnight.
    pub time: f64,
    /// Weather file, empty for the map's default.
    pub weather: String,
    /// The season the player chose (`spring`, `summer`, `autumn`, `winter`), with its
    /// phase when not the middle (`autumn-late`, `spring-early`: an older version reads
    /// no season in that and goes by the date, which is the host's phase date), empty
    /// when the date decides as in OMSI.
    pub season: String,
}

impl WorldInfo {
    fn from_fields(parts: &[&str], first: usize) -> WorldInfo {
        let time = field(parts, first + 2)
            .parse::<f64>()
            .ok()
            .filter(|t| t.is_finite())
            .map(|t| t.rem_euclid(86400.0))
            .unwrap_or(0.0);
        WorldInfo {
            map: clean_text(field(parts, first), 260),
            date: clean_date(field(parts, first + 1)),
            time,
            weather: clean_text(field(parts, first + 3), 260),
            season: clean_text(field(parts, first + 4), 16),
        }
    }

    fn fields(&self) -> String {
        format!(
            "{}|{}|{:.2}|{}|{}",
            clean_text(&self.map, 260),
            clean_date(&self.date),
            self.time,
            clean_text(&self.weather, 260),
            clean_text(&self.season, 16)
        )
    }
}

/// `YYYY-MM-DD`, or empty for anything else.
fn clean_date(s: &str) -> String {
    let v: Vec<&str> = s.trim().split('-').collect();
    let ok = v.len() == 3
        && v[0].len() == 4
        && v.iter()
            .all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit()));
    let month_day = ok
        && matches!(v[1].parse::<u32>(), Ok(1..=12))
        && matches!(v[2].parse::<u32>(), Ok(1..=31));
    if month_day {
        s.trim().to_string()
    } else {
        String::new()
    }
}

/// Where a vehicle stands and how big it is (for spawning next to it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Footprint {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Degrees, clockwise from north.
    pub heading: f32,
    pub length: f32,
    pub width: f32,
}

impl Footprint {
    fn encode(&self) -> String {
        format!(
            "{:.2},{:.2},{:.2},{:.1},{:.1},{:.1}",
            self.x, self.y, self.z, self.heading, self.length, self.width
        )
    }

    fn decode(s: &str) -> Option<Footprint> {
        let v: Vec<f64> = s
            .split(',')
            .map(|x| x.trim().parse::<f64>())
            .collect::<Result<_, _>>()
            .ok()?;
        Self::from_numbers(&v)
    }

    /// From x, y, z, heading, length, width - finite, and of a size a vehicle can have.
    fn from_numbers(v: &[f64]) -> Option<Footprint> {
        if v.len() != 6
            || v.iter().any(|x| !x.is_finite())
            || v[0].abs() > 1.0e8
            || v[1].abs() > 1.0e8
            || v[2].abs() > 1.0e5
        {
            return None;
        }
        Some(Footprint {
            x: v[0],
            y: v[1],
            z: v[2],
            heading: v[3].rem_euclid(360.0) as f32,
            length: v[4].clamp(0.0, 60.0) as f32,
            width: v[5].clamp(0.0, 8.0) as f32,
        })
    }

    fn distance2(&self, x: f64, y: f64) -> f64 {
        (self.x - x).powi(2) + (self.y - y).powi(2)
    }
}

/// Pose of a coupled section (the rear of an articulated bus).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartPose {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub heading: f32,
}

/// What a player says about its vehicle: who and what (`INFO`, every two seconds), and
/// where it is and what it does (`STATE`, up to twenty times a second).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pose {
    pub id: u32,
    pub name: String,
    /// Vehicle file relative to its content root (`Vehicles/MAN_NL_NG/MAN_EN92_main.bus`),
    /// empty for a player without a vehicle.
    pub bus: String,
    /// Paint scheme name (empty for the default).
    pub paint: String,
    /// Line and destination as the bus displays them.
    pub line: String,
    pub destination: String,
    /// The timetable tour the player drives, `<line>/<tour>` (empty for none).
    pub tour: String,
    /// What the vehicle's `[texttexture]` displays show (their string variables, in the
    /// model's order), so another player's bus shows the same destination and line signs
    /// rather than what its depot file makes of the line and terminus names.
    pub texts: Vec<String>,
    /// What the vehicle's `[matl_freetex]` string variables hold (in the order of their
    /// names, see the game's `lan.rs`): the picture a roller blind or a sign shows, which the
    /// others' copy of the bus cannot work out, its scripts not running there.
    pub freetex: Vec<String>,
    /// The player's own figure (`.hum` relative to its content root), for the driver at the
    /// wheel and the walker the others draw (empty: they pick one of the map's drivers).
    pub figure: String,
    /// Length and width (m) of the box around the whole vehicle (rear sections included),
    /// and how far its centre lies ahead of the vehicle's origin (negative: behind).
    pub length: f32,
    pub width: f32,
    pub box_offset: f32,
    /// Hash of the vehicle's sync table (see `lamps`, `switches`, `values`), 0 for none.
    pub table: u32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub heading: f32,
    pub pitch: f32,
    pub bank: f32,
    pub speed_kmh: f32,
    /// Front wheel angle (degrees, positive to the right).
    pub steer_deg: f32,
    /// `FLAG_*` bits.
    pub flags: u32,
    /// Head lights: 0 off, 1 parking lights, 2 dipped, 3 main beam.
    pub head: u8,
    /// Interior lights 0 (off) … 3 (full).
    pub interior: u8,
    /// Indicators: 0 off, 1 left, 2 right, 3 hazard.
    pub blinker: u8,
    /// Engine speed (rpm), throttle and brake pedal (0..1).
    pub rpm: f32,
    pub throttle: f32,
    pub brake: f32,
    /// Passengers aboard.
    pub passengers: u32,
    /// Door openings 0..1, `door_0` first.
    pub doors: Vec<f32>,
    /// Suspension travel of each wheel (m), axle by axle, left then right.
    pub suspension: Vec<f32>,
    /// Coupled sections, front to back.
    pub rear: Vec<PartPose>,
    /// The vehicle's lamp variables (0..1), its `[visible]` switches (small integers) and
    /// the variables its outside sounds and moving parts follow, in sync table order.
    pub lamps: Vec<f32>,
    pub switches: Vec<f32>,
    pub values: Vec<f32>,
    /// The player out of the seat, walking about (their name goes with them).
    pub walker: Option<Walker>,
    /// The sender's clock when the state left (ms, wrapping), for drawing the others'
    /// buses between two states as they were sent rather than as they arrived; 0 unknown.
    pub sent_ms: u32,
    /// The driver is keying the map-wide bus radio (GreenTeaSpeak voice); see `voice`.
    pub radio_keyed: bool,
}

/// A player on foot: where, facing where, how fast, and whether sitting in some bus.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Walker {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Where the body faces (degrees).
    pub heading: f32,
    pub speed: f32,
    /// Which way the walker goes (degrees): not where the body faces when stepping
    /// sideways or backwards (NaN from an older game: along `heading`).
    pub course: f32,
    pub seated: bool,
    /// Aboard a player's bus: that player's id, the place in its cabin (bus frame, m) and
    /// the seat taken (the others draw the walker in that bus, not at a world point that
    /// lags behind it).
    pub aboard: Option<Aboard>,
}

/// Where a walker is in a player's bus.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Aboard {
    /// The bus's player (the host is 1).
    pub owner: u32,
    pub local: [f32; 3],
    pub seat: Option<u16>,
}

impl Pose {
    /// Does this pose carry a vehicle (rather than being a player's heartbeat)?
    pub fn has_vehicle(&self) -> bool {
        self.flags & FLAG_VEHICLE != 0 && !self.bus.is_empty()
    }

    fn encode_info(&self) -> String {
        let head = format!(
            "INFO|{}|{}|{}|{}|{}|{}|{:.2}|{:.2}|{:.2}|{:08X}|{}|",
            self.id,
            clean_text(&self.name, MAX_NAME),
            vehicle_path(&self.bus).unwrap_or_default(),
            clean_text(&self.paint, MAX_FIELD),
            clean_text(&self.line, 16),
            clean_text(&self.destination, MAX_FIELD),
            finite_or(self.length, 0.0).clamp(0.0, 60.0),
            finite_or(self.width, 0.0).clamp(0.0, 8.0),
            finite_or(self.box_offset, 0.0).clamp(-40.0, 40.0),
            self.table,
            clean_text(&self.tour, MAX_FIELD),
        );
        let figure = human_path(&self.figure).unwrap_or_default();
        // the display texts get what room is left in one datagram (long vehicle and figure
        // paths and a destination in another alphabet made an INFO too long to be taken in:
        // the others never learnt which bus the player drove)
        let room = MAX_DATAGRAM.saturating_sub(head.len() + figure.len() + 1);
        let info = format!("{head}{}|{figure}", encode_texts(&self.texts, MAX_TEXTS, MAX_TEXT_LEN, room));
        // the `[matl_freetex]` pictures last, in what room is left (an older game reads the
        // fields it knows and passes this one by)
        let room = MAX_DATAGRAM.saturating_sub(info.len() + 1);
        format!("{info}|{}", encode_texts(&self.freetex, MAX_FREETEX, MAX_FREETEX_LEN, room))
    }

    /// The info fields of an `INFO` message (checked and cleaned), or None.
    fn decode_info(parts: &[&str]) -> Option<Pose> {
        if parts.len() < 11 || parts[0] != "INFO" {
            return None;
        }
        let num = |i: usize, lo: f32, hi: f32| {
            parts[i]
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite() && *v >= lo && *v <= hi)
        };
        Some(Pose {
            id: parts[1].trim().parse().ok()?,
            name: clean_text(parts[2], MAX_NAME),
            bus: vehicle_path(parts[3]).unwrap_or_default(),
            paint: clean_text(parts[4], MAX_FIELD),
            line: clean_text(parts[5], 16),
            destination: clean_text(parts[6], MAX_FIELD),
            length: num(7, 0.0, 60.0)?,
            width: num(8, 0.0, 8.0)?,
            box_offset: num(9, -40.0, 40.0)?,
            table: u32::from_str_radix(parts[10].trim(), 16).ok()?,
            tour: parts.get(11).map(|t| clean_text(t, MAX_FIELD)).unwrap_or_default(),
            texts: parts.get(12).map(|t| decode_texts(t, MAX_TEXTS, MAX_TEXT_LEN)).unwrap_or_default(),
            figure: parts.get(13).and_then(|f| human_path(f)).unwrap_or_default(),
            freetex: parts.get(14).map(|t| decode_texts(t, MAX_FREETEX, MAX_FREETEX_LEN)).unwrap_or_default(),
            ..Default::default()
        })
    }

    /// Take the who-and-what of `info`, keeping the state.
    fn set_info(&mut self, info: &Pose) {
        self.name = info.name.clone();
        self.bus = info.bus.clone();
        self.paint = info.paint.clone();
        self.line = info.line.clone();
        self.destination = info.destination.clone();
        self.tour = info.tour.clone();
        self.texts = info.texts.clone();
        self.freetex = info.freetex.clone();
        self.figure = info.figure.clone();
        self.length = info.length;
        self.width = info.width;
        self.box_offset = info.box_offset;
        self.table = info.table;
    }

    /// Take the state of `s`, keeping the who-and-what.
    fn set_state(&mut self, s: Pose) {
        let keep = std::mem::take(self);
        *self = Pose {
            id: keep.id,
            name: keep.name,
            bus: keep.bus,
            paint: keep.paint,
            line: keep.line,
            destination: keep.destination,
            tour: keep.tour,
            texts: keep.texts,
            freetex: keep.freetex,
            figure: keep.figure,
            length: keep.length,
            width: keep.width,
            box_offset: keep.box_offset,
            table: keep.table,
            ..s
        };
    }

    pub fn footprint(&self) -> Footprint {
        let h = (self.heading as f64).to_radians();
        let d = self.box_offset as f64;
        Footprint {
            x: self.x + h.sin() * d,
            y: self.y + h.cos() * d,
            z: self.z,
            heading: self.heading,
            length: self.length.max(1.0),
            width: self.width.max(1.0),
        }
    }

    /// A pose that only says who somebody is (before their first state).
    fn placeholder(id: u32, name: &str, bus: &str) -> Pose {
        Pose {
            id,
            name: name.to_string(),
            bus: bus.to_string(),
            ..Default::default()
        }
    }
}

fn finite_or(v: f32, or: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        or
    }
}

/// Text from the network or for it: no field separators or control characters, no
/// surrounding blanks, at most `max` characters.
/// Display texts at most (and characters each) an `INFO` carries.
pub const MAX_TEXTS: usize = 12;
const MAX_TEXT_LEN: usize = 32;
/// `[matl_freetex]` strings at most (and characters each): paths to a picture, longer than
/// a display's text (`..\..\Anzeigen\Rollband_FC\<depot>\17.tga`).
pub const MAX_FREETEX: usize = 8;
const MAX_FREETEX_LEN: usize = 128;

/// Display texts as one `INFO` field: each as hex of its UTF-8, comma separated (a text may
/// hold anything, the field no `|`).
fn encode_texts(texts: &[String], max: usize, max_len: usize, room: usize) -> String {
    texts
        .iter()
        .take(max)
        .map(|t| {
            let t: String = t.chars().filter(|c| !c.is_control()).take(max_len).collect();
            t.bytes().map(|b| format!("{b:02x}")).collect::<String>()
        })
        .scan(0usize, |used, h| {
            // (the whole INFO stays well inside a datagram)
            *used += h.len() + 1;
            (*used <= room.min(720)).then_some(h)
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn decode_texts(field: &str, max: usize, max_len: usize) -> Vec<String> {
    if field.trim().is_empty() {
        return Vec::new();
    }
    field
        .split(',')
        .take(max)
        .map(|h| {
            let bytes: Vec<u8> = (0..h.len() / 2).filter_map(|i| u8::from_str_radix(h.get(2 * i..2 * i + 2)?, 16).ok()).collect();
            String::from_utf8_lossy(&bytes).chars().filter(|c| !c.is_control()).take(max_len).collect()
        })
        .collect()
}

pub fn clean_text(s: &str, max: usize) -> String {
    let t: String = s
        .chars()
        .map(|c| if c == '|' || c.is_control() { ' ' } else { c })
        .take(max * 4)
        .collect();
    t.trim()
        .chars()
        .take(max)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The longest vehicle path taken from the network.
const MAX_VEHICLE_PATH: usize = 260;

/// `Pose::bus` as a path relative to a content root that may be looked up on this machine,
/// or None. A pose comes from another machine, so the path is not trusted: it must be
/// relative (no leading `/`, no drive, no `:`), without empty, `.` or `..` parts (or parts
/// of dots only, which Windows reads as those), and name a `.bus` or `.ovh` file.
pub fn vehicle_path(bus: &str) -> Option<String> {
    relative_path(bus, &["bus", "ovh"])
}

/// A person's figure file (`.hum`) as another game names it: the same rules as
/// [`vehicle_path`] (relative, no `..`, no drive letters), for the driver and walker figure.
pub fn human_path(hum: &str) -> Option<String> {
    relative_path(hum, &["hum"])
}

fn relative_path(path: &str, exts: &[&str]) -> Option<String> {
    let p = path.trim().replace('\\', "/");
    if p.is_empty()
        || p.len() > MAX_VEHICLE_PATH
        || p.starts_with('/')
        || p.chars().any(|c| c == ':' || c == '|' || c.is_control())
    {
        return None;
    }
    if p.split('/')
        .any(|part| part.trim_matches(|c| c == '.' || c == ' ').is_empty())
    {
        return None;
    }
    let file = p.rsplit('/').next()?;
    let ext = file.rsplit_once('.')?.1.to_ascii_lowercase();
    exts.contains(&ext.as_str()).then_some(p)
}

fn field<'a>(parts: &[&'a str], i: usize) -> &'a str {
    parts.get(i).copied().unwrap_or("")
}

/// A host's answer as the client keeps it.
#[derive(Debug, Clone)]
pub struct Welcome {
    pub host_name: String,
    pub session: u64,
    /// The host's world when it answered.
    pub world: WorldInfo,
    /// When the welcome arrived (the host's clock has moved on since).
    pub at: Instant,
    /// Players in the session, the host included, when we joined.
    pub players: usize,
}

/// The host's clock as it last told us.
#[derive(Debug, Clone)]
pub struct HostClock {
    pub world: WorldInfo,
    pub at: Instant,
    /// How fast the host's clock runs (1 real time; the host's "time speed").
    pub speed: f64,
}

impl HostClock {
    /// The host's time of day now (seconds since midnight; past midnight it runs on over
    /// 86400, so that the date can be moved on with it).
    pub fn time_now(&self) -> f64 {
        self.world.time + self.at.elapsed().as_secs_f64() * self.speed
    }
}

/// A client whose bus stands somewhere and who wants to know what stands there (host side).
#[derive(Debug, Clone)]
pub struct JoinRequest {
    pub id: u32,
    pub addr: SocketAddr,
    pub name: String,
    pub bus: String,
    /// Where the client's bus is about to stand.
    pub spawn: Footprint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Host,
    Client,
}

/// Something the game shows in its message line.
#[derive(Debug, Clone, PartialEq)]
pub enum LanEvent {
    /// A chat line: who wrote it and what (`mine` for our own).
    Chat {
        id: u32,
        name: String,
        text: String,
        mine: bool,
    },
    /// Somebody joined or left, the connection came or went.
    Notice(String),
}

/// A token bucket: `rate` a second, at most `burst` at once.
#[derive(Debug, Clone)]
struct Bucket {
    tokens: f32,
    last: Instant,
}

impl Bucket {
    fn new(burst: f32) -> Bucket {
        Bucket {
            tokens: burst,
            last: Instant::now(),
        }
    }

    fn take(&mut self, (rate, burst): (f32, f32)) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.last).as_secs_f32() * rate).min(burst);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// A remote player as this machine knows them.
#[derive(Debug, Clone)]
pub struct Peer {
    /// Where the player's datagrams come from (host side).
    pub addr: Option<SocketAddr>,
    pub pose: Pose,
    pub last_seen: Instant,
    /// A state has arrived (not just the hello).
    pub has_pose: bool,
    /// An INFO has arrived: `pose.bus` and the rest are known.
    pub has_info: bool,
    last_seq: u16,
    state_at: Instant,
    states: Bucket,
    messages: Bucket,
    chat: Bucket,
    /// Datagrams of this player the host threw away (too many at once).
    pub dropped: u32,
    /// The nonce of the player's hellos (host side).
    nonce: Option<u64>,
    /// The latest states as they came (arrival, state), oldest first: the game draws the
    /// player's bus between two of them (`Pose::sent_ms`), not only the newest.
    pub history: std::collections::VecDeque<(Instant, Pose)>,
}

impl Peer {
    fn new(addr: Option<SocketAddr>, pose: Pose) -> Peer {
        let now = Instant::now();
        Peer {
            addr,
            pose,
            last_seen: now,
            has_pose: false,
            has_info: false,
            last_seq: 0,
            state_at: now,
            states: Bucket::new(STATE_RATE.1),
            messages: Bucket::new(MESSAGE_RATE.1),
            chat: Bucket::new(CHAT_RATE.1),
            dropped: 0,
            history: std::collections::VecDeque::new(),
            nonce: None,
        }
    }

    /// A player's name as the others read it.
    fn label(&self) -> String {
        if self.pose.name.is_empty() {
            format!("player {}", self.pose.id)
        } else {
            self.pose.name.clone()
        }
    }
}

/// The vehicle a path names, for messages: `MAN_EN92_main`.
fn vehicle_label(bus: &str) -> String {
    let f = bus.rsplit('/').next().unwrap_or("");
    f.rsplit_once('.').map(|(s, _)| s).unwrap_or(f).to_string()
}

pub struct LanSession {
    socket: UdpSocket,
    pub role: Role,
    /// The host's address (clients): the one that answered, or the only one there is to try;
    /// None for the host and while several addresses are being tried.
    pub host: Option<SocketAddr>,
    /// The addresses a client tries at once until one answers (a session code carries the
    /// host's VPN and LAN addresses).
    pub candidates: Vec<SocketAddr>,
    /// Seconds the client has been trying (ticked time: the minute the game spends loading
    /// its map between two ticks does not count), and since when it lost a host it had (see
    /// `JOIN_TIMEOUT`, `RECONNECT_TIMEOUT`).
    trying: f32,
    lost_at: Option<Instant>,
    /// The client gave up (no answer / host lost) but keeps saying hello now and then: a
    /// welcome brings it back without restarting the game (see `reconnect`).
    timed_out: bool,
    join_timeout: Duration,
    /// "Port unreachable" answers to our hellos (a computer that is there, without a session
    /// on that port; Windows and Linux report them).
    refused: u32,
    /// A refusal from one of several addresses tried at once (see `REJECT`).
    other_reject: Option<String>,
    /// Random number of this client, sent with every hello: a client that tries several
    /// addresses of the host at once is one player to it, whichever way its hellos came.
    nonce: u64,
    /// Say hello once more to the address that answered (see `on_welcome`).
    confirm: bool,
    pub my_id: u32,
    pub my_name: String,
    /// The host's session id; a client learns it from the code or the welcome.
    pub session: u64,
    /// Whether the client was given the session id (a code) and insists on it.
    session_required: bool,
    /// Our own world: sent in hellos, welcomes and the host's clock messages (the game
    /// keeps its time up to date with `set_clock`).
    pub world: WorldInfo,
    peers: HashMap<u32, Peer>,
    next_id: u32,
    send_acc: f32,
    hello_acc: f32,
    info_acc: f32,
    clock_acc: f32,
    place_acc: f32,
    pub connected: bool,
    /// The host turned us away (the reason), or the target cannot be reached at all.
    pub rejected: Option<String>,
    /// The host's own word that we are not to play there: sent away (`KICK`, a ban too) or
    /// turned away at the door (`REJECT`), with its message. Unlike a lost connection
    /// (`rejected` alone, the game plays on and may `/reconnect`), the game ends on it.
    pub turned_away: Option<String>,
    /// What the host said when it let us in (the latest welcome after a reconnect).
    pub welcome: Option<Welcome>,
    /// Counts the welcomes after which the connection was (re)made: the game takes the
    /// host's world again when it changes.
    pub welcomes: u32,
    /// Differences between the host's world and ours that remain (another map; a weather
    /// the game could not take over).
    pub warnings: Vec<String>,
    /// The game has placed its vehicle according to the host's list (set by the game).
    pub spawn_settled: bool,
    /// Where our bus stands, for the host's list of what stands there (client).
    place: Option<Footprint>,
    /// The host's list of the vehicles near `place`.
    pub near: Option<Vec<Footprint>>,
    /// Clients asking what stands where they are (host).
    pending: Vec<JoinRequest>,
    /// Vehicles of the host's own world (its bus, AI traffic), for the lists it sends.
    local_footprints: Vec<Footprint>,
    host_seen: Instant,
    host_clock: Option<HostClock>,
    events: Vec<LanEvent>,
    chat: Bucket,
    seq: u16,
    /// The last state sent (without its header), and for how long it has not changed.
    last_state: Vec<u8>,
    unchanged: f32,
    last_info: String,
    /// `TIMEOUT`, `LOAD_TIMEOUT` and `HEARTBEAT` (shorter in the tests).
    timeout: Duration,
    load_timeout: Duration,
    heartbeat: f32,
    buf: Vec<u8>,
    /// Bytes sent and received so far.
    sent: Cell<u64>,
    pub received: u64,
    /// The shared world (see `world`): frames, descriptions and handed-over passengers
    /// the host sent (client), and what the clients asked for (host).
    world_in: Vec<(Instant, world::WorldFrame)>,
    /// Every script variable of our vehicle going out (`vars.rs`), and the others' coming in.
    var_sender: vars::VarSender,
    vars_in: Vec<vars::VarsIn>,
    descs: Vec<world::Desc>,
    wants: Vec<(u32, Vec<world::EntityRef>)>,
    claims: Vec<(u32, Vec<u32>)>,
    grants: Vec<(Vec<u32>, bool)>,
    world_seq: u16,
    /// People on foot of the clients' own (boarding and leaving their buses), as they send
    /// them up (host): (player, when it came, frame); and their descriptions.
    world_up: Vec<(u32, Instant, world::WorldFrame)>,
    descs_up: Vec<(u32, world::Desc)>,
    /// When the session began (the host's world clock counts from it).
    started: Instant,
    /// The clock the states and world frames are stamped with (ms since `started`): on
    /// by each frame's time step and kept near the clock on the wall. Stamped as they left,
    /// a state carried the moment of sending, some milliseconds after the moment of the
    /// frame it shows, more or less as the frame took long or not - the others drew the
    /// bus between states a little too far apart or too close, on and on: the jitter.
    frame_ms: Option<f64>,
    /// Bytes of world datagrams and descriptions sent / received so far.
    world_sent: Cell<u64>,
    pub world_received: u64,
    /// The code as a bridge over the internet (see `bridge`): STUN, UPnP, the rendezvous.
    bridge: Option<bridge::Bridge>,
    /// Players who went lately (host): (nonce, name, id, when). One who comes back soon
    /// keeps their number and is "back", not a newcomer.
    gone_lately: Vec<(Option<u64>, String, u32, Instant)>,
    /// Players the host sent away (host): their nonces, so they are not let in again.
    banned: Vec<(u64, String)>,
    /// Commands for this game (`command`): (from, text).
    commands: Vec<(u32, String)>,
    /// How fast the session's clock runs (the host's time speed; a client: the host's as
    /// its clock messages say).
    pub clock_speed: f64,
}

/// This machine's addresses for another player on its own network, with `port`.
fn local_addrs(port: u16) -> Vec<SocketAddr> {
    code_ipv4s()
        .into_iter()
        .filter(|ip| !ip.is_loopback())
        .map(|ip| SocketAddr::from((ip, port)))
        .collect()
}

impl LanSession {
    fn new(socket: UdpSocket, role: Role, name: &str, world: WorldInfo) -> LanSession {
        let now = Instant::now();
        LanSession {
            socket,
            role,
            host: None,
            candidates: Vec::new(),
            trying: 0.0,
            lost_at: None,
            timed_out: false,
            join_timeout: omsi_cfg::flags::OMSI_LAN_JOIN_TIMEOUT.live_var()
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| *v > 0.0)
                .map(Duration::from_secs_f32)
                .unwrap_or(JOIN_TIMEOUT),
            refused: 0,
            other_reject: None,
            nonce: random_session_id() ^ ((std::process::id() as u64) << 48),
            confirm: false,
            my_id: if role == Role::Host { 1 } else { 0 },
            my_name: clean_text(name, MAX_NAME),
            session: 0,
            session_required: false,
            world,
            peers: HashMap::new(),
            next_id: 2,
            send_acc: 0.0,
            hello_acc: 1.0,
            info_acc: INFO_EVERY,
            clock_acc: 0.0,
            place_acc: 1.0,
            connected: role == Role::Host,
            rejected: None,
            turned_away: None,
            welcome: None,
            welcomes: 0,
            warnings: Vec::new(),
            spawn_settled: role == Role::Host,
            place: None,
            near: None,
            pending: Vec::new(),
            local_footprints: Vec::new(),
            host_seen: now,
            host_clock: None,
            events: Vec::new(),
            chat: Bucket::new(CHAT_RATE.1),
            seq: 0,
            last_state: Vec::new(),
            unchanged: 0.0,
            last_info: String::new(),
            timeout: TIMEOUT,
            load_timeout: LOAD_TIMEOUT,
            heartbeat: HEARTBEAT,
            buf: vec![0; 8192],
            sent: Cell::new(0),
            received: 0,
            world_in: Vec::new(),
            var_sender: vars::VarSender::default(),
            vars_in: Vec::new(),
            descs: Vec::new(),
            wants: Vec::new(),
            claims: Vec::new(),
            grants: Vec::new(),
            world_seq: 0,
            world_up: Vec::new(),
            descs_up: Vec::new(),
            started: now,
            frame_ms: None,
            world_sent: Cell::new(0),
            world_received: 0,
            bridge: None,
            gone_lately: Vec::new(),
            banned: Vec::new(),
            commands: Vec::new(),
            clock_speed: 1.0,
        }
    }

    /// Host a session on `port` (all interfaces). With `try_next` the next few ports are
    /// tried when that one is taken (another session runs on this machine).
    pub fn host(
        port: u16,
        name: &str,
        world: WorldInfo,
        try_next: bool,
    ) -> std::io::Result<LanSession> {
        let mut last_err = None;
        let tries = if try_next { PORT_RANGE } else { 1 };
        for p in port..port.saturating_add(tries) {
            match UdpSocket::bind(("0.0.0.0", p)) {
                Ok(socket) => {
                    socket.set_nonblocking(true)?;
                    socket.set_broadcast(true)?;
                    let mut s = LanSession::new(socket, Role::Host, name, world);
                    s.session = random_session_id();
                    s.bridge = bridge::Bridge::start(true, s.session, local_addrs(p), p);
                    log::info!("LAN: hosting session {} on port {p} as '{name}' (protocol {PROTOCOL}), code {}", session_hex(s.session), s.code().map(|c| c.encode()).unwrap_or_default());
                    if p != port {
                        log::info!("LAN: port {port} is taken (another session on this machine?), using {p}");
                    }
                    return Ok(s);
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| std::io::Error::other("no port")))
    }

    /// Join the host at one of `addrs` (tried all at once); `session` is the id from a
    /// session code (checked by the host).
    pub fn join_addr(
        addrs: Vec<SocketAddr>,
        session: Option<u64>,
        name: &str,
        world: WorldInfo,
    ) -> std::io::Result<LanSession> {
        let socket = UdpSocket::bind(("0.0.0.0", 0))?;
        socket.set_nonblocking(true)?;
        let mut s = LanSession::new(socket, Role::Client, name, world);
        if addrs.len() == 1 {
            s.host = Some(addrs[0]);
        }
        s.candidates = addrs;
        if let Some(id) = session {
            s.session = id;
            s.session_required = true;
            // a code works over the internet too: the rendezvous tells the host's public
            // address and the host opens the way (see `bridge`); that takes a little longer
            let port = s.local_addr().map(|a| a.port()).unwrap_or(0);
            s.bridge = bridge::Bridge::start(false, id, local_addrs(port), port);
            if s.bridge.is_some() {
                s.join_timeout = s.join_timeout.max(Duration::from_secs(25));
                s.host = None;
                // `OMSI_BRIDGE_ONLY`: forget the code's own addresses, so that only what the
                // rendezvous tells is tried (to check that path)
                if omsi_cfg::flags::OMSI_BRIDGE_ONLY.live_os().is_some() {
                    s.candidates.clear();
                }
            }
        }
        log::info!(
            "LAN: joining {} as '{name}'{}",
            s.candidates
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(" or "),
            session
                .map(|id| format!(" (session {})", session_hex(id)))
                .unwrap_or_default()
        );
        Ok(s)
    }

    /// Join by what the player typed (see `parse_join`); discovery waits up to `wait`.
    pub fn join(
        target: &str,
        name: &str,
        world: WorldInfo,
        wait: Duration,
    ) -> Result<LanSession, String> {
        match parse_join(target)? {
            JoinTarget::Discover => match Self::discover(DEFAULT_PORT, name, world, wait) {
                Ok(Some(s)) => Ok(s),
                Ok(None) => Err(format!("no host answered on the local network within {:.0} s (is a session hosted? a firewall may block UDP port {DEFAULT_PORT})", wait.as_secs_f32())),
                Err(e) => Err(format!("searching the network failed: {e}")),
            },
            JoinTarget::Direct { addrs, session, protocol } => {
                if let Some(p) = protocol {
                    if p as u32 != PROTOCOL {
                        return Err(format!("the session code was made by a game with LAN protocol {p}, this game speaks protocol {PROTOCOL} - both players need the same version"));
                    }
                }
                Self::join_addr(addrs, session, name, world).map_err(|e| format!("cannot open a network socket: {e}"))
            }
        }
    }

    /// Look for a host on the local network for up to `wait`; joins the first that answers.
    pub fn discover(
        port: u16,
        name: &str,
        world: WorldInfo,
        wait: Duration,
    ) -> std::io::Result<Option<LanSession>> {
        let socket = UdpSocket::bind(("0.0.0.0", 0))?;
        socket.set_broadcast(true)?;
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        let started = Instant::now();
        let mut buf = [0u8; 1024];
        // the limited broadcast, the loopback (a host on this machine, on any port of its
        // range), and this machine's own subnet broadcast (macOS does not always route
        // 255.255.255.255)
        let mut targets: Vec<SocketAddr> = vec![SocketAddr::from(([255, 255, 255, 255], port))];
        for p in port..port.saturating_add(PORT_RANGE) {
            targets.push(SocketAddr::from(([127, 0, 0, 1], p)));
        }
        // every subnet of this machine (a Hamachi or ZeroTier network carries broadcasts too)
        for a in addrs::joinable_addresses() {
            let b = a.broadcast.unwrap_or_else(|| {
                let o = a.ip.octets();
                Ipv4Addr::new(o[0], o[1], o[2], 255)
            });
            let t = SocketAddr::from((b, port));
            if !targets.contains(&t) {
                targets.push(t);
            }
        }
        let msg = format!("DISCOVER|{PROTOCOL}");
        while started.elapsed() < wait {
            for t in &targets {
                let _ = socket.send_to(msg.as_bytes(), t);
            }
            if let Ok((n, from)) = socket.recv_from(&mut buf) {
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                let parts: Vec<&str> = text.split('|').collect();
                if parts[0] == "HERE" {
                    if field(&parts, 1).parse::<u32>().ok() != Some(PROTOCOL) {
                        log::warn!("LAN: host at {from} speaks protocol {} (we speak {PROTOCOL}); skipping it", field(&parts, 1));
                        continue;
                    }
                    log::info!(
                        "LAN: found host '{}' at {from} (session {}, {} on {})",
                        clean_text(field(&parts, 2), MAX_NAME),
                        field(&parts, 3),
                        field(&parts, 5),
                        clean_text(field(&parts, 4), 260)
                    );
                    let session = parse_session_hex(field(&parts, 3));
                    return Self::join_addr(vec![from], session, name, world).map(Some);
                }
            }
        }
        Ok(None)
    }

    /// The code other players join this session with (hosts only).
    pub fn code(&self) -> Option<SessionCode> {
        if self.role != Role::Host {
            return None;
        }
        let port = self.local_addr()?.port();
        // the address the internet sees first (when the router kept the port: forwarded, or
        // mapped to the same one), then this machine's own
        let mut ips = code_ipv4s();
        if let Some(SocketAddr::V4(p)) = self.bridge.as_ref().and_then(|b| b.public()) {
            if p.port() == port && !ips.contains(p.ip()) {
                ips.insert(0, *p.ip());
                ips.truncate(MAX_CODE_ADDRS);
            }
        }
        Some(SessionCode {
            protocol: PROTOCOL as u8,
            ips,
            port,
            session: self.session,
        })
    }

    /// Everybody else as we know them.
    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.values()
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Bytes sent so far.
    pub fn sent(&self) -> u64 {
        self.sent.get()
    }

    /// The weather the session runs with.
    pub fn weather(&self) -> &str {
        &self.world.weather
    }

    /// The host's weather changed: a host sends it with a clock message at once, not with the
    /// next one of every five seconds.
    pub fn set_weather(&mut self, weather: &str) {
        let weather = clean_text(&weather.trim().replace('\\', "/"), 260);
        if self.world.weather != weather {
            self.world.weather = weather;
            if self.role == Role::Host {
                self.clock_acc = CLOCK_EVERY;
            }
        }
    }

    /// The game's clock: what the host's welcomes and clock messages say.
    pub fn set_clock(&mut self, date: &str, time: f64) {
        if self.world.date != date {
            self.world.date = date.to_string();
        }
        self.world.time = time;
    }

    /// Our world after the game took over the host's (a client): only what still differs
    /// is warned about.
    pub fn set_world(&mut self, world: WorldInfo) {
        self.world = world;
        if let Some(w) = self.welcome.as_ref().map(|w| w.world.clone()) {
            self.compare_worlds(&w);
        }
    }

    /// Chat lines and notices since the last call.
    pub fn take_events(&mut self) -> Vec<LanEvent> {
        std::mem::take(&mut self.events)
    }

    /// The host's clock, when it said something new since the last call (clients).
    pub fn take_host_clock(&mut self) -> Option<HostClock> {
        self.host_clock.take()
    }

    /// The host's own vehicles (its bus, its AI traffic) for the lists it sends.
    pub fn set_local_footprints(&mut self, f: Vec<Footprint>) {
        self.local_footprints = f;
    }

    /// Clients waiting to hear what stands where their bus is: the game may refresh the
    /// footprints and call `answer_joins`; otherwise the next `tick` answers with what it has.
    pub fn pending_joins(&self) -> &[JoinRequest] {
        &self.pending
    }

    /// Ask the host what stands around our bus (clients): sent until it answers (`near`).
    pub fn request_near(&mut self, at: Footprint) {
        self.place = Some(at);
        self.near = None;
        self.place_acc = 1.0;
    }

    /// Answer every waiting client with the vehicles near where its bus stands.
    pub fn answer_joins(&mut self) {
        for req in std::mem::take(&mut self.pending) {
            let mut near: Vec<Footprint> = self.local_footprints.clone();
            for (pid, p) in &self.peers {
                if *pid != req.id && p.has_pose && p.has_info && p.pose.has_vehicle() {
                    near.push(p.pose.footprint());
                }
            }
            near.retain(|f| {
                f.distance2(req.spawn.x, req.spawn.y) < FOOTPRINT_RADIUS * FOOTPRINT_RADIUS
            });
            near.sort_by(|a, b| {
                a.distance2(req.spawn.x, req.spawn.y)
                    .total_cmp(&b.distance2(req.spawn.x, req.spawn.y))
            });
            near.truncate(MAX_FOOTPRINTS);
            let fp = near
                .iter()
                .map(|f| f.encode())
                .collect::<Vec<_>>()
                .join(";");
            self.send(format!("NEAR|{}|{fp}", req.id).as_bytes(), req.addr);
            log::info!(
                "LAN: told player {} '{}' about {} vehicle(s) standing near their bus",
                req.id,
                req.name,
                near.len()
            );
        }
    }

    /// A command for player `to` (1 the host): the host passes it on. Used for switches
    /// worked in another player's bus and for the host's administration.
    pub fn command(&mut self, to: u32, text: &str) {
        let text = clean_text(text, MAX_CHAT);
        if text.is_empty() || to == self.my_id {
            return;
        }
        let msg = format!("CMD|{}|{to}|{text}", self.my_id);
        match self.role {
            Role::Host => {
                if let Some(a) = self.peers.get(&to).and_then(|p| p.addr) {
                    self.send(msg.as_bytes(), a);
                }
            }
            Role::Client => {
                if let Some(h) = self.host {
                    self.send(msg.as_bytes(), h);
                }
            }
        }
    }

    /// The commands that came for this game: (from, text).
    pub fn take_commands(&mut self) -> Vec<(u32, String)> {
        std::mem::take(&mut self.commands)
    }

    fn on_command(&mut self, parts: &[&str], from: SocketAddr) {
        let (Some(sender), Some(to)) = (field(parts, 1).parse::<u32>().ok(), field(parts, 2).parse::<u32>().ok()) else {
            return;
        };
        let text = clean_text(field(parts, 3), MAX_CHAT);
        if text.is_empty() {
            return;
        }
        match self.role {
            Role::Host => {
                // only from the player it says, at their address
                if self.checked_peer(sender, from, MESSAGE_RATE, false).is_none() {
                    return;
                }
                if to == self.my_id {
                    self.commands.push((sender, text));
                } else if let Some(a) = self.peers.get(&to).and_then(|p| p.addr) {
                    self.send(format!("CMD|{sender}|{to}|{text}").as_bytes(), a);
                }
            }
            Role::Client => {
                if Some(from) == self.host && to == self.my_id && self.commands.len() < 64 {
                    self.commands.push((sender, text));
                }
            }
        }
    }

    /// Host: send player `id` away (`ban`: for the rest of the session, the same reason given
    /// at the door when it comes back).
    pub fn kick(&mut self, id: u32, reason: &str, ban: bool) {
        if self.role != Role::Host {
            return;
        }
        let Some(p) = self.peers.remove(&id) else { return };
        let reason = clean_text(reason, 200);
        if let Some(a) = p.addr {
            self.send(format!("KICK|{reason}").as_bytes(), a);
        }
        if ban {
            if let Some(n) = p.nonce {
                self.banned.push((n, reason.clone()));
            }
        }
        self.broadcast(format!("BYE|{id}").as_bytes(), None);
        self.notice(format!("{} was sent away by the host", p.label()), None);
    }

    /// Say something to everybody. Our own line comes back as an event as well.
    /// The host: a chat line for one player only, under `name` - `SAY` to that player's
    /// address alone, so the others never see it, and a game of any version shows it like any
    /// chat line (an admin's private word to a driver).
    pub fn say_to(&mut self, to: u32, name: &str, text: &str) -> Result<(), String> {
        let text = clean_text(text, MAX_CHAT);
        if text.is_empty() {
            return Err("nothing to say".into());
        }
        if !matches!(self.role, Role::Host) {
            return Err("only the host speaks to one player".into());
        }
        let name = clean_text(name, MAX_NAME);
        let addr = self.peers.get(&to).and_then(|p| p.addr).ok_or_else(|| format!("no player {to}"))?;
        self.send(format!("SAY|{}|{name}|{text}", self.my_id).as_bytes(), addr);
        log::info!("LAN chat to {to} <{name}> {text}");
        Ok(())
    }

    pub fn say(&mut self, text: &str) -> Result<(), String> {
        let text = clean_text(text, MAX_CHAT);
        if text.is_empty() {
            return Err("nothing to say".into());
        }
        if !self.connected {
            return Err("not connected to a session".into());
        }
        if !self.chat.take(CHAT_RATE) {
            return Err("not so fast: one line a second".into());
        }
        match self.role {
            Role::Host => self.broadcast(
                format!("SAY|{}|{}|{text}", self.my_id, self.my_name).as_bytes(),
                None,
            ),
            Role::Client => {
                if let Some(h) = self.host {
                    self.send(format!("CHAT|{}|{text}", self.my_id).as_bytes(), h);
                }
            }
        }
        log::info!("LAN chat <{}> {text}", self.my_name);
        self.events.push(LanEvent::Chat {
            id: self.my_id,
            name: self.my_name.clone(),
            text,
            mine: true,
        });
        Ok(())
    }

    /// A host line under a chosen name (admin notices to every player). Not rate-limited.
    pub fn announce(&mut self, name: &str, text: &str) -> Result<(), String> {
        let text = clean_text(text, MAX_CHAT);
        if text.is_empty() {
            return Err("nothing to say".into());
        }
        if !matches!(self.role, Role::Host) {
            return Err("only the host announces".into());
        }
        if !self.connected {
            return Err("not connected to a session".into());
        }
        let name = clean_text(name, MAX_NAME);
        self.broadcast(format!("SAY|{}|{name}|{text}", self.my_id).as_bytes(), None);
        log::info!("LAN announce <{name}> {text}");
        self.events.push(LanEvent::Chat {
            id: self.my_id,
            name,
            text,
            mine: true,
        });
        Ok(())
    }

    fn send(&self, data: &[u8], to: SocketAddr) {
        if self.socket.send_to(data, to).is_ok() {
            self.sent.set(self.sent.get() + data.len() as u64);
        }
    }

    /// To every player (host), but `except`.
    fn broadcast(&self, data: &[u8], except: Option<u32>) {
        for (id, peer) in &self.peers {
            if Some(*id) != except {
                if let Some(a) = peer.addr {
                    self.send(data, a);
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------
    // the shared world (host → clients, see `world`)

    /// The host's world clock: ms since the session began.
    pub fn world_ms(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    /// The moment of this frame on our clock (ms), for stamping what is sent of it.
    pub fn stamp_ms(&self) -> u32 {
        self.frame_ms.map(|m| m.max(0.0) as u32).unwrap_or_else(|| self.world_ms())
    }

    /// The address of player `id` (host).
    fn peer_addr(&self, id: u32) -> Option<SocketAddr> {
        self.peers.get(&id).and_then(|p| p.addr)
    }

    /// Send player `id` the world around it (host). Returns the bytes sent.
    pub fn send_world(&mut self, id: u32, frame: &world::WorldFrame) -> usize {
        let Some(to) = self.peer_addr(id).filter(|_| self.role == Role::Host) else {
            return 0;
        };
        self.world_seq = self.world_seq.wrapping_add(1);
        let mut f = frame.clone();
        f.seq = self.world_seq;
        f.host_ms = self.stamp_ms();
        let mut n = 0;
        for d in world::encode(&f, PROTOCOL as u8) {
            self.send(&d, to);
            n += d.len();
        }
        self.world_sent.set(self.world_sent.get() + n as u64);
        n
    }

    /// Tell player `id` what a car or person of the world is (host).
    pub fn send_desc(&self, id: u32, desc: &world::Desc) {
        if let Some(to) = self.peer_addr(id).filter(|_| self.role == Role::Host) {
            let text = desc.encode();
            self.send(text.as_bytes(), to);
            self.world_sent.set(self.world_sent.get() + text.len() as u64);
        }
    }

    /// Send the host the people of our own who are on foot near our bus (client): those
    /// walking up to it from the stop, and those who got off and walk away.
    pub fn send_world_up(&mut self, frame: &world::WorldFrame) -> usize {
        let (Role::Client, Some(h), true) = (self.role, self.host, self.connected) else {
            return 0;
        };
        self.world_seq = self.world_seq.wrapping_add(1);
        let f = world::WorldFrame {
            seq: self.world_seq,
            host_ms: self.stamp_ms(),
            cars: Vec::new(),
            lights: Vec::new(),
            ..frame.clone()
        };
        let mut n = 0;
        for d in world::encode(&f, PROTOCOL as u8) {
            self.send(&d, h);
            n += d.len();
        }
        self.world_sent.set(self.world_sent.get() + n as u64);
        n
    }

    /// ... and what they are (client).
    pub fn send_desc_up(&self, desc: &world::Desc) {
        if let (Role::Client, Some(h), true, world::Desc::Person { .. }) =
            (self.role, self.host, self.connected, desc)
        {
            let text = desc.encode();
            self.send(text.as_bytes(), h);
            self.world_sent.set(self.world_sent.get() + text.len() as u64);
        }
    }

    /// The clients' people that came since the last call (host).
    pub fn take_world_up(&mut self) -> Vec<(u32, Instant, world::WorldFrame)> {
        std::mem::take(&mut self.world_up)
    }

    pub fn take_descs_up(&mut self) -> Vec<(u32, world::Desc)> {
        std::mem::take(&mut self.descs_up)
    }

    /// Ask the host for the descriptions of these (client).
    pub fn want(&self, what: &[world::EntityRef]) {
        if let (Role::Client, Some(h), true) = (self.role, self.host, self.connected) {
            for chunk in what.chunks(64) {
                let text = format!("WANT|{}|{}", self.my_id, world::EntityRef::list(chunk));
                self.send(text.as_bytes(), h);
            }
        }
    }

    /// Ask the host for the people waiting at a stop who are about to board our bus
    /// (client): it answers with `GRANT` for those it hands over (they leave its world and
    /// become ours) and `DENY` for the others (they boarded another bus meanwhile).
    pub fn claim(&self, people: &[u32]) {
        if let (Role::Client, Some(h), true) = (self.role, self.host, self.connected) {
            for chunk in people.chunks(64) {
                let ids: Vec<String> = chunk.iter().map(|i| i.to_string()).collect();
                self.send(format!("CLAIM|{}|{}", self.my_id, ids.join(",")).as_bytes(), h);
            }
        }
    }

    /// Answer a player's claim (host).
    pub fn answer_claim(&self, id: u32, granted: &[u32], denied: &[u32]) {
        let Some(to) = self.peer_addr(id) else { return };
        for (word, list) in [("GRANT", granted), ("DENY", denied)] {
            for chunk in list.chunks(64).filter(|c| !c.is_empty()) {
                let ids: Vec<String> = chunk.iter().map(|i| i.to_string()).collect();
                self.send(format!("{word}|{}", ids.join(",")).as_bytes(), to);
            }
        }
    }

    /// World frames that came since the last call, with when they came (client).
    pub fn take_world(&mut self) -> Vec<(Instant, world::WorldFrame)> {
        std::mem::take(&mut self.world_in)
    }

    /// Descriptions that came (client).
    pub fn take_descs(&mut self) -> Vec<world::Desc> {
        std::mem::take(&mut self.descs)
    }

    /// Players asking what something is (host).
    pub fn take_wants(&mut self) -> Vec<(u32, Vec<world::EntityRef>)> {
        std::mem::take(&mut self.wants)
    }

    /// Players claiming waiting passengers (host).
    pub fn take_claims(&mut self) -> Vec<(u32, Vec<u32>)> {
        std::mem::take(&mut self.claims)
    }

    /// The host's answers to our claims: (people, granted) (client).
    pub fn take_grants(&mut self) -> Vec<(Vec<u32>, bool)> {
        std::mem::take(&mut self.grants)
    }

    /// Bytes of the shared world sent so far (host).
    pub fn world_sent(&self) -> u64 {
        self.world_sent.get()
    }

    /// Tell every player (host) and ourselves.
    fn notice(&mut self, text: String, except: Option<u32>) {
        log::info!("LAN: {text}");
        self.broadcast(
            format!("NOTE|{}", clean_text(&text, 200)).as_bytes(),
            except,
        );
        self.events.push(LanEvent::Notice(text));
    }

    fn reject(&self, to: SocketAddr, reason: &str) {
        self.send(
            format!("REJECT|{PROTOCOL}|{}", clean_text(reason, 300)).as_bytes(),
            to,
        );
    }

    fn send_hello(&self, mine: &Pose) {
        // (to every address of the host until one of them answers)
        let to: Vec<SocketAddr> = match self.host {
            Some(h) => vec![h],
            None => self.candidates.clone(),
        };
        if to.is_empty() {
            return;
        }
        let session = if self.session_required {
            session_hex(self.session)
        } else {
            "-".to_string()
        };
        let msg = format!(
            "HELLO|{PROTOCOL}|{session}|{}|{}|{}|{:016X}",
            self.my_name,
            vehicle_path(&mine.bus).unwrap_or_default(),
            self.world.fields(),
            self.nonce
        );
        for h in to {
            self.send(msg.as_bytes(), h);
        }
    }

    fn send_welcome(&self, id: u32, to: SocketAddr) {
        let msg = format!(
            "WELCOME|{PROTOCOL}|{id}|{}|{}|{}|{}",
            session_hex(self.session),
            self.my_name,
            self.world.fields(),
            self.peers.len() + 1
        );
        self.send(msg.as_bytes(), to);
    }

    /// What differs between the host's world and ours and stays so: the map (players on
    /// different maps never meet). Date, time, weather and season are the game's to take
    /// over; what it cannot take (a weather that is not installed) it adds to `warnings`.
    fn compare_worlds(&mut self, host: &WorldInfo) {
        let mine = &self.world;
        let mut w = Vec::new();
        let norm = |s: &str| s.trim().replace('\\', "/").to_ascii_lowercase();
        if !host.map.is_empty() && norm(&host.map) != norm(&mine.map) {
            w.push(format!(
                "the host drives on {}, you on {} - you will not meet",
                host.map, mine.map
            ));
        }
        for line in &w {
            if !self.warnings.contains(line) {
                log::warn!("LAN: {line}");
            }
        }
        self.warnings = w;
    }

    /// A joining player that is not in the world yet (fetching the host's mods): the host
    /// is told every 2 s that we are still here, without a state (which would show us), and
    /// what arrives is taken in.
    pub fn keepalive(&mut self, dt: f32, mine: &Pose) {
        if let Some(b) = self.bridge.as_mut() {
            b.tick(dt, &self.socket);
        }
        self.hello_acc += dt;
        if self.role == Role::Client && self.hello_acc >= 2.0 {
            self.hello_acc = 0.0;
            self.send_hello(mine);
        }
        let mut gone = Vec::new();
        self.receive(&mut gone);
        if self.role == Role::Client {
            self.host_seen = Instant::now();
        }
    }

    /// One frame: send our state when due, take in what arrived, forget the silent.
    /// Returns the ids of players that left this frame.
    pub fn tick(&mut self, dt: f32, mine: &Pose) -> Vec<u32> {
        let mut gone = Vec::new();
        let wall = self.started.elapsed().as_secs_f64() * 1000.0;
        self.frame_ms = Some(match self.frame_ms {
            Some(prev) => {
                let c = prev + dt as f64 * 1000.0;
                // (a frame longer than the step the game takes, or a pause: the wall again;
                // ahead of it, it waits - it never goes back)
                if wall - c > 250.0 {
                    wall
                } else if c - wall > 250.0 {
                    prev
                } else {
                    (c + (wall - c) * 0.05).max(prev)
                }
            }
            None => wall,
        });
        if let Some(b) = self.bridge.as_mut() {
            b.tick(dt, &self.socket);
            // the host's addresses the rendezvous told (a client still trying)
            if self.role == Role::Client && !self.connected {
                for a in b.host_addrs() {
                    if !self.candidates.contains(&a) {
                        log::info!("LAN: the relay says the host is also at {a}");
                        self.candidates.push(a);
                    }
                }
            }
        }
        // a host whose game did not answer the requests in time answers them now
        if self.role == Role::Host && !self.pending.is_empty() {
            self.answer_joins();
        }
        self.send_acc += dt;
        self.hello_acc += dt;
        self.info_acc += dt;
        self.clock_acc += dt;
        self.place_acc += dt;
        if self.role == Role::Client && !self.connected && self.rejected.is_none() {
            self.trying += dt.min(0.5);
        }
        if self.role == Role::Client
            && !self.connected
            && (self.rejected.is_none() && self.hello_acc >= 1.0
                || self.timed_out && self.hello_acc >= 5.0)
        {
            self.hello_acc = 0.0;
            self.send_hello(mine);
        }
        if self.connected {
            if std::mem::take(&mut self.confirm) {
                self.send_hello(mine);
            }
            self.send_own(dt, mine);
        }
        self.receive(&mut gone);
        // time out the silent: a player who has sent no state yet is still loading
        let now = Instant::now();
        let silent: Vec<u32> = self
            .peers
            .iter()
            .filter(|(_, p)| {
                now.duration_since(p.last_seen)
                    > if p.has_pose {
                        self.timeout
                    } else {
                        self.load_timeout
                    }
            })
            .map(|(id, _)| *id)
            .collect();
        for id in silent {
            let Some(p) = self.peers.remove(&id) else {
                continue;
            };
            log::info!("LAN: player {id} timed out");
            gone.push(id);
            self.gone_lately.push((p.nonce, p.pose.name.clone(), id, now));
            if self.role == Role::Host {
                self.broadcast(format!("BYE|{id}").as_bytes(), None);
                self.notice(format!("{} lost the connection", p.label()), None);
            }
        }
        if self.role == Role::Client
            && self.connected
            && now.duration_since(self.host_seen) > self.timeout
        {
            log::warn!("LAN: the host has gone quiet; trying to reach it again");
            self.events.push(LanEvent::Notice(
                "the host has gone quiet; trying to reach it again".into(),
            ));
            self.connected = false;
            self.lost_at = Some(now);
        }
        self.check_join_timeout();
        gone
    }

    /// A client that never got an answer gives up after `JOIN_TIMEOUT`, one that lost its
    /// host after `RECONNECT_TIMEOUT` (`rejected` says why; the game plays on alone).
    fn check_join_timeout(&mut self) {
        if self.role != Role::Client || self.connected || self.rejected.is_some() {
            return;
        }
        let why = match (self.welcome.is_some(), self.lost_at) {
            (false, _) if self.trying >= self.join_timeout.as_secs_f32() => {
                self.no_answer(self.join_timeout)
            }
            (true, Some(at)) if at.elapsed() >= RECONNECT_TIMEOUT => format!(
                "the host has not answered for {:.0} s - the session is over (playing on alone; still trying, or type /reconnect)",
                RECONNECT_TIMEOUT.as_secs_f32()
            ),
            _ => return,
        };
        log::warn!("LAN: {why}");
        self.events.push(LanEvent::Notice(why.clone()));
        self.rejected = Some(why);
        self.timed_out = true;
    }

    /// Say hello once more at once (the way to the host was made again: a new connection
    /// is a new address to it, which it only learns from a hello). Clients only.
    pub fn rehello(&mut self) {
        if self.role == Role::Client {
            self.confirm = true;
        }
    }

    /// Try the host again at once (a client that was turned away, timed out or sent away):
    /// the game takes the host's world again when the welcome comes. False for a host.
    pub fn reconnect(&mut self) -> bool {
        if self.role != Role::Client {
            return false;
        }
        self.rejected = None;
        self.turned_away = None;
        self.timed_out = false;
        self.connected = false;
        self.other_reject = None;
        self.refused = 0;
        self.trying = 0.0;
        self.lost_at = Some(Instant::now());
        self.hello_acc = 1.0;
        if self.candidates.len() > 1 {
            self.host = None;
        }
        log::info!("LAN: reconnecting");
        self.events.push(LanEvent::Notice("reconnecting ...".into()));
        true
    }

    /// Why nobody may have answered, for the player.
    fn no_answer(&self, after: Duration) -> String {
        let tried: Vec<String> = self.candidates.iter().map(|a| a.to_string()).collect();
        let port = self.candidates.first().map(|a| a.port()).unwrap_or(DEFAULT_PORT);
        if self.refused >= 2 && self.candidates.len() == 1 {
            return format!(
                "{} answers, but no session runs on port {port} there - check the port, or whether the host has started its game",
                self.candidates[0].ip()
            );
        }
        if let Some(r) = self.other_reject.as_ref() {
            return format!("the host did not answer; {r}");
        }
        format!(
            "no answer from {} within {:.0} s. Is the session still running, and was the whole code copied? The host's firewall must let the game receive UDP on port {port} (on Windows allow it for public networks too). If both of you are behind a mobile or carrier network that changes the port for every connection, the way cannot be opened: the host can forward UDP port {port} on the router, or you both use a VPN",
            tried.join(" or "),
            after.as_secs_f32()
        )
    }

    /// Our state (when due), our info (every two seconds or when it changed), the host's
    /// clock and a client's question about its spawn.
    fn send_own(&mut self, dt: f32, mine: &Pose) {
        let mut p = mine.clone();
        p.id = self.my_id;
        p.name = self.my_name.clone();
        if p.bus.is_empty() {
            p.flags &= !FLAG_VEHICLE;
        }
        // (the clock is left out of the comparison: an unchanged state is still unchanged)
        p.sent_ms = 0;
        let data = wire::encode_state(&p, PROTOCOL as u8, self.seq);
        let body = &data[wire::STATE_HEADER..];
        if body == self.last_state.as_slice() {
            self.unchanged += dt;
        } else {
            self.unchanged = 0.0;
        }
        // a heartbeat without a vehicle, the full rate while anything changes, less when not
        let interval = if p.flags & FLAG_VEHICLE == 0 {
            self.heartbeat
        } else if self.unchanged > 1.0 {
            1.0 / IDLE_RATE_HZ
        } else {
            1.0 / RATE_HZ
        };
        // (the remainder is kept, so the rate holds on average whatever the frame times)
        if self.send_acc + 1.0e-4 >= interval {
            self.send_acc = (self.send_acc - interval).clamp(0.0, interval);
            self.last_state = body.to_vec();
            self.seq = self.seq.wrapping_add(1);
            // stamped with the moment of the frame it shows (see `Pose::sent_ms`)
            p.sent_ms = self.stamp_ms().max(1);
            let data = wire::encode_state(&p, PROTOCOL as u8, self.seq);
            match self.role {
                Role::Host => self.broadcast(&data, None),
                Role::Client => {
                    if let Some(h) = self.host {
                        self.send(&data, h);
                    }
                }
            }
        }
        let info = p.encode_info();
        // Sent when it changes, but no more often than INFO_MIN_GAP: a roller blind turning
        // through its numbers or a pilot screen changes the `[matl_freetex]` pictures many
        // times a second, and the host took ten messages a second and dropped the rest.
        if (info != self.last_info && self.info_acc >= INFO_MIN_GAP) || self.info_acc >= INFO_EVERY {
            self.info_acc = 0.0;
            match self.role {
                Role::Host => self.broadcast(info.as_bytes(), None),
                Role::Client => {
                    if let Some(h) = self.host {
                        self.send(info.as_bytes(), h);
                    }
                }
            }
            self.last_info = info;
        }
        if self.role == Role::Host && self.clock_acc >= CLOCK_EVERY {
            self.clock_acc = 0.0;
            self.broadcast(format!("CLOCK|{}|{}", self.world.fields(), self.clock_speed).as_bytes(), None);
        }
        if let (Role::Client, Some(at), None, Some(h)) =
            (self.role, self.place, self.near.as_ref(), self.host)
        {
            if self.place_acc >= 1.0 {
                self.place_acc = 0.0;
                self.send(
                    format!(
                        "PLACE|{}|{:.2}|{:.2}|{:.2}|{:.1}|{:.1}|{:.1}",
                        self.my_id, at.x, at.y, at.z, at.heading, at.length, at.width
                    )
                    .as_bytes(),
                    h,
                );
            }
        }
    }

    fn receive(&mut self, gone: &mut Vec<u32>) {
        loop {
            let (n, from) = match self.socket.recv_from(&mut self.buf) {
                Ok(x) => x,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                // an ICMP "port unreachable" from a host that is not running shows up here
                Err(e)
                    if e.kind() == std::io::ErrorKind::ConnectionReset
                        || e.kind() == std::io::ErrorKind::ConnectionRefused =>
                {
                    self.refused += 1;
                    continue;
                }
                Err(_) => break,
            };
            self.received += n as u64;
            // the bridge's own datagrams (a STUN answer, a punch): not the game's
            if bridge::is_bridge_packet(&self.buf[..n]) {
                let data = self.buf[..n].to_vec();
                if let Some(b) = self.bridge.as_mut() {
                    b.receive(&data);
                }
                continue;
            }
            // a client listens to its host only: a welcome, a rejection, a goodbye or a state
            // from anybody else is not the session's (discovery has its own socket)
            if self.role == Role::Client {
                // while the host's addresses are being tried, an answer may come from any of
                // them - or from another address of that computer, on the host's port (a
                // machine on several networks may answer from the one its route prefers)
                let trying = !self.connected
                    && (self.candidates.contains(&from)
                        || self.candidates.iter().any(|c| c.port() == from.port()));
                if Some(from) != self.host && !trying {
                    continue;
                }
                if Some(from) == self.host {
                    self.host_seen = Instant::now();
                }
            }
            if n == 0 || n > MAX_DATAGRAM {
                continue;
            }
            let data = self.buf[..n].to_vec();
            if data[0] == world::WORLD_MAGIC {
                // a client takes the world from its host (above); a host takes the people of
                // a client's own from that client (within its rate)
                if self.role == Role::Host {
                    let Some(id) = self
                        .peers
                        .iter()
                        .find(|(_, p)| p.addr == Some(from))
                        .map(|(id, _)| *id)
                    else {
                        continue;
                    };
                    if self.checked_peer(id, from, STATE_RATE, true).is_none() {
                        continue;
                    }
                    if let Some(mut f) = world::decode(&data, PROTOCOL as u8) {
                        self.world_received += n as u64;
                        f.cars.clear();
                        f.lights.clear();
                        if self.world_up.len() < 256 {
                            self.world_up.push((id, Instant::now(), f));
                        }
                    }
                } else {
                    if let Some(f) = world::decode(&data, PROTOCOL as u8) {
                        self.world_received += n as u64;
                        // (a game that stopped ticking does not pile them up without end)
                        if self.world_in.len() < 256 {
                            self.world_in.push((Instant::now(), f));
                        }
                    }
                }
                continue;
            }
            if data[0] == wire::STATE_MAGIC {
                self.on_state(&data, from);
                continue;
            }
            if data[0] == vars::VARS_MAGIC {
                self.on_vars(&data, from);
                continue;
            }
            let Ok(text) = std::str::from_utf8(&data) else {
                continue;
            };
            let parts: Vec<&str> = text.split('|').collect();
            match (parts[0], self.role) {
                ("DISCOVER", Role::Host) => {
                    let msg = format!(
                        "HERE|{PROTOCOL}|{}|{}|{}|{}",
                        self.my_name,
                        session_hex(self.session),
                        clean_text(&self.world.map, 260),
                        self.peers.len() + 1
                    );
                    self.send(msg.as_bytes(), from);
                }
                ("HELLO", Role::Host) => self.on_hello(&parts, from),
                ("WELCOME", Role::Client) => self.on_welcome(&parts, from),
                ("REJECT", Role::Client) if self.host.is_none() || Some(from) == self.host => {
                    let reason = clean_text(field(&parts, 2), 300);
                    // One of several addresses tried at once may belong to another computer
                    // (a LAN address like ours in the friend's house): its "no" is kept for
                    // the message, but the others may still answer.
                    if self.host.is_none() && self.candidates.len() > 1 {
                        if self.other_reject.is_none() {
                            log::info!("LAN: {from} turned us away ({reason}); still trying the host's other addresses");
                        }
                        self.other_reject = Some(format!("{from}: {reason}"));
                        continue;
                    }
                    if self.rejected.is_none() {
                        log::warn!("LAN: the host turned us away: {reason}");
                        self.events.push(LanEvent::Notice(format!(
                            "the host turned us away: {reason}"
                        )));
                    }
                    // (the game ends on a door closed before it ever played there, or on a ban;
                    // a reconnect refused for now - the session full, say - leaves it playing
                    // on to `/reconnect` later)
                    if self.welcomes == 0 || reason.contains("sent you away") {
                        self.turned_away = Some(reason.clone());
                    }
                    self.rejected = Some(reason);
                    self.connected = false;
                }
                ("INFO", _) => self.on_info(&parts, from),
                ("PLACE", Role::Host) => self.on_place(&parts, from),
                ("NEAR", Role::Client) => {
                    if field(&parts, 1).parse::<u32>().ok() == Some(self.my_id)
                        && self.place.is_some()
                    {
                        self.near = Some(
                            field(&parts, 2)
                                .split(';')
                                .filter_map(Footprint::decode)
                                .take(MAX_FOOTPRINTS)
                                .collect(),
                        );
                    }
                }
                ("CLOCK", Role::Client) => {
                    let world = WorldInfo::from_fields(&parts, 1);
                    // (after the world's fields: how fast the host's clock runs)
                    let speed = field(&parts, 6).parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0 && *v <= 1000.0).unwrap_or(1.0);
                    self.clock_speed = speed;
                    if !world.date.is_empty() {
                        self.host_clock = Some(HostClock {
                            world,
                            at: Instant::now(),
                            speed,
                        });
                    }
                }
                ("CHAT", Role::Host) => self.on_chat(&parts, from),
                ("CMD", _) => self.on_command(&parts, from),
                ("KICK", Role::Client) => {
                    let reason = clean_text(field(&parts, 1), 200);
                    log::warn!("LAN: the host sent us away: {reason}");
                    self.events.push(LanEvent::Notice(format!("the host sent you away: {reason}")));
                    self.rejected = Some(format!("the host sent you away: {reason}"));
                    self.turned_away = Some(reason);
                    self.connected = false;
                }
                ("WANT", Role::Host) | ("CLAIM", Role::Host) => {
                    let Some(id) = field(&parts, 1).parse::<u32>().ok() else {
                        continue;
                    };
                    if self.checked_peer(id, from, MESSAGE_RATE, false).is_none() {
                        continue;
                    }
                    if parts[0] == "WANT" {
                        let list = world::EntityRef::parse_list(field(&parts, 2));
                        if !list.is_empty() && self.wants.len() < 64 {
                            self.wants.push((id, list));
                        }
                    } else {
                        let list: Vec<u32> = field(&parts, 2)
                            .split(',')
                            .filter_map(|x| x.trim().parse().ok())
                            .take(64)
                            .collect();
                        if !list.is_empty() && self.claims.len() < 64 {
                            self.claims.push((id, list));
                        }
                    }
                }
                ("DESC", Role::Host) => {
                    let Some(id) = self
                        .peers
                        .iter()
                        .find(|(_, p)| p.addr == Some(from))
                        .map(|(id, _)| *id)
                    else {
                        continue;
                    };
                    if self.checked_peer(id, from, MESSAGE_RATE, false).is_none() {
                        continue;
                    }
                    if let Some(d @ world::Desc::Person { .. }) = world::Desc::decode(&parts) {
                        if self.descs_up.len() < 256 {
                            self.descs_up.push((id, d));
                        }
                    }
                }
                ("DESC", Role::Client) => {
                    if let Some(d) = world::Desc::decode(&parts) {
                        self.world_received += n as u64;
                        if self.descs.len() < 1024 {
                            self.descs.push(d);
                        }
                    }
                }
                ("GRANT", Role::Client) | ("DENY", Role::Client) => {
                    let list: Vec<u32> = field(&parts, 1)
                        .split(',')
                        .filter_map(|x| x.trim().parse().ok())
                        .take(64)
                        .collect();
                    if !list.is_empty() {
                        self.grants.push((list, parts[0] == "GRANT"));
                    }
                }
                ("SAY", Role::Client) => {
                    let (Some(id), name, text) = (
                        field(&parts, 1).parse::<u32>().ok(),
                        clean_text(field(&parts, 2), MAX_NAME),
                        clean_text(field(&parts, 3), MAX_CHAT),
                    ) else {
                        continue;
                    };
                    if !text.is_empty() {
                        log::info!("LAN chat <{name}> {text}");
                        self.events.push(LanEvent::Chat {
                            id,
                            name,
                            text,
                            mine: false,
                        });
                    }
                }
                ("NOTE", Role::Client) => {
                    let text = clean_text(field(&parts, 1), 200);
                    if !text.is_empty() {
                        log::info!("LAN: {text}");
                        self.events.push(LanEvent::Notice(text));
                    }
                }
                ("BYE", _) => self.on_bye(&parts, from, text, gone),
                ("HELLO", Role::Client) | ("DISCOVER", Role::Client) => {}
                // an older game that got past the hello somehow (or sends its poses blindly)
                ("POSE", Role::Host) if !self.peers.values().any(|p| p.addr == Some(from)) => {
                    self.reject(from, &format!("the host runs LAN protocol {PROTOCOL}, your game an older one - both players need the same version of the game"));
                }
                _ => {}
            }
        }
    }

    /// The peer `id` when the datagram really comes from it (host), and it may send more.
    fn checked_peer(
        &mut self,
        id: u32,
        from: SocketAddr,
        rate: (f32, f32),
        state: bool,
    ) -> Option<&mut Peer> {
        let peer = self.peers.get_mut(&id)?;
        if peer.addr != Some(from) {
            return None;
        }
        let ok = if state {
            peer.states.take(rate)
        } else {
            peer.messages.take(rate)
        };
        if !ok {
            peer.dropped += 1;
            if peer.dropped.is_power_of_two() {
                log::warn!(
                    "LAN: player {id} sends too much; {} datagram(s) dropped so far",
                    peer.dropped
                );
            }
            return None;
        }
        peer.last_seen = Instant::now();
        Some(peer)
    }

    /// Every script variable of our vehicle (see `vars.rs`): `float_ids[k]` holds
    /// `floats[k]`, the same for the strings, `table` names them (a hash of their names). Called
    /// every frame; it sends what is due.
    #[allow(clippy::too_many_arguments)]
    pub fn send_vars(&mut self, table: u32, float_ids: &[u16], floats: &[f32], string_ids: &[u16], strings: &[String], dt: f32) {
        let msgs = self.var_sender.tick(PROTOCOL as u8, self.my_id, table, float_ids, floats, string_ids, strings, dt);
        for m in msgs {
            match self.role {
                Role::Host => self.broadcast(&m, None),
                Role::Client => {
                    if let Some(h) = self.host {
                        self.send(&m, h);
                    }
                }
            }
        }
    }

    /// The others' vehicle variables that came since the last call, oldest first.
    pub fn take_vars(&mut self) -> Vec<vars::VarsIn> {
        std::mem::take(&mut self.vars_in)
    }

    fn on_vars(&mut self, data: &[u8], from: SocketAddr) {
        let Some(v) = vars::decode(data, PROTOCOL as u8) else { return };
        if v.id == self.my_id || v.id == 0 {
            return;
        }
        match self.role {
            // (as a state: only a player we welcomed, from its address, within its rate)
            Role::Host => {
                if self.checked_peer(v.id, from, STATE_RATE, true).is_none() {
                    return;
                }
                self.broadcast(data, Some(v.id));
            }
            Role::Client => {
                if !self.peers.contains_key(&v.id) {
                    return;
                }
            }
        }
        // (a game that stopped reading does not pile them up without end)
        if self.vars_in.len() < 1024 {
            self.vars_in.push(v);
        }
    }

    fn on_state(&mut self, data: &[u8], from: SocketAddr) {
        let Some((id, seq, state)) = wire::decode_state(data, PROTOCOL as u8) else {
            return;
        };
        if id == self.my_id || id == 0 {
            return;
        }
        let peer = match self.role {
            // only players we welcomed, from the address we welcomed, are taken and relayed
            Role::Host => match self.checked_peer(id, from, STATE_RATE, true) {
                Some(p) => p,
                None => return,
            },
            Role::Client => {
                if !self.peers.contains_key(&id) && self.peers.len() >= MAX_PEERS {
                    return;
                }
                self.peers
                    .entry(id)
                    .or_insert_with(|| Peer::new(None, Pose::placeholder(id, "", "")))
            }
        };
        // a state older than the last one (datagrams overtake each other) is dropped, unless
        // the sender has started counting again
        let now = Instant::now();
        if peer.has_pose
            && !wire::seq_newer(seq, peer.last_seq)
            && now.duration_since(peer.state_at) < Duration::from_secs(1)
        {
            return;
        }
        peer.last_seq = seq;
        peer.state_at = now;
        peer.last_seen = now;
        // (a heartbeat of a game still loading its world is no state yet: a load that holds
        // its window for longer than the time-out cost the player the session, and they
        // came back as a new one - "joined", "left", "joined")
        peer.has_pose |= state.flags & FLAG_VEHICLE != 0 || state.walker.is_some();
        if peer.history.len() >= 12 {
            peer.history.pop_front();
        }
        peer.history.push_back((now, state.clone()));
        peer.pose.set_state(state);
        if self.role == Role::Host {
            self.broadcast(data, Some(id));
        }
    }

    fn on_info(&mut self, parts: &[&str], from: SocketAddr) {
        let Some(info) = Pose::decode_info(parts) else {
            return;
        };
        let id = info.id;
        if id == self.my_id || id == 0 {
            return;
        }
        let host = self.role == Role::Host;
        let relay;
        {
            let peer = match self.role {
                Role::Host => match self.checked_peer(id, from, MESSAGE_RATE, false) {
                    Some(p) => p,
                    None => return,
                },
                Role::Client => {
                    if !self.peers.contains_key(&id) && self.peers.len() >= MAX_PEERS {
                        return;
                    }
                    self.peers
                        .entry(id)
                        .or_insert_with(|| Peer::new(None, Pose::placeholder(id, "", "")))
                }
            };
            let mut info = info;
            // a player keeps the name it joined with unless it sends one
            if info.name.is_empty() {
                info.name = peer.pose.name.clone();
            }
            let changed =
                !peer.has_info || peer.pose.bus != info.bus || peer.pose.name != info.name;
            if changed {
                log::info!(
                    "LAN: player {id} is '{}' in {}",
                    info.name,
                    if info.bus.is_empty() {
                        "no vehicle".to_string()
                    } else {
                        info.bus.clone()
                    }
                );
            }
            peer.pose.set_info(&info);
            peer.has_info = true;
            peer.last_seen = Instant::now();
            relay = host.then(|| peer.pose.encode_info());
        }
        if let Some(text) = relay {
            // passed on as the host read it, not as it came
            self.broadcast(text.as_bytes(), Some(id));
        }
    }

    fn on_place(&mut self, parts: &[&str], from: SocketAddr) {
        let Some(id) = field(parts, 1).parse::<u32>().ok() else {
            return;
        };
        let v: Vec<f64> = (2..8)
            .filter_map(|i| field(parts, i).parse::<f64>().ok())
            .collect();
        let Some(spawn) = Footprint::from_numbers(&v) else {
            return;
        };
        let Some(peer) = self.checked_peer(id, from, MESSAGE_RATE, false) else {
            return;
        };
        let (name, bus) = (peer.pose.name.clone(), peer.pose.bus.clone());
        if !self.pending.iter().any(|r| r.id == id) {
            self.pending.push(JoinRequest {
                id,
                addr: from,
                name,
                bus,
                spawn,
            });
        }
    }

    fn on_chat(&mut self, parts: &[&str], from: SocketAddr) {
        let Some(id) = field(parts, 1).parse::<u32>().ok() else {
            return;
        };
        let text = clean_text(field(parts, 2), MAX_CHAT);
        let Some(peer) = self.checked_peer(id, from, MESSAGE_RATE, false) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if !peer.chat.take(CHAT_RATE) {
            peer.dropped += 1;
            return;
        }
        let name = peer.label();
        log::info!("LAN chat <{name}> {text}");
        self.broadcast(format!("SAY|{id}|{name}|{text}").as_bytes(), Some(id));
        self.events.push(LanEvent::Chat {
            id,
            name,
            text,
            mine: false,
        });
    }

    fn on_bye(&mut self, parts: &[&str], from: SocketAddr, text: &str, gone: &mut Vec<u32>) {
        let Some(id) = field(parts, 1).parse::<u32>().ok() else {
            return;
        };
        if self.role == Role::Host
            && !self
                .peers
                .get(&id)
                .map(|p| p.addr == Some(from))
                .unwrap_or(false)
        {
            return;
        }
        if let Some(p) = self.peers.remove(&id) {
            log::info!("LAN: player {id} '{}' left", p.pose.name);
            gone.push(id);
            self.gone_lately.push((p.nonce, p.pose.name.clone(), id, Instant::now()));
            if self.role == Role::Host {
                self.broadcast(text.as_bytes(), None);
                self.notice(format!("{} left", p.label()), None);
            }
        }
        if self.role == Role::Client && id == 1 && self.connected {
            // the host ended the session: look for it again (it may restart)
            self.connected = false;
            self.lost_at = Some(Instant::now());
            self.events
                .push(LanEvent::Notice("the host ended the session".into()));
        }
    }

    fn on_hello(&mut self, parts: &[&str], from: SocketAddr) {
        let proto = field(parts, 1).parse::<u32>().unwrap_or(1);
        let name = match clean_text(field(parts, 3), MAX_NAME) {
            n if n.is_empty() => "Driver".to_string(),
            n => n,
        };
        if proto != PROTOCOL {
            log::warn!("LAN: {from} speaks protocol {proto}, we speak {PROTOCOL}; turned away");
            self.reject(from, &format!("the host runs LAN protocol {PROTOCOL}, your game protocol {proto} - both players need the same version of the game"));
            return;
        }
        let asked = field(parts, 2);
        if asked != "-" && !asked.is_empty() && parse_session_hex(asked) != Some(self.session) {
            log::warn!(
                "LAN: '{name}' at {from} asked for session {asked}, this is {}; turned away",
                session_hex(self.session)
            );
            let code = self.code().map(|c| c.encode()).unwrap_or_default();
            self.reject(from, &format!("wrong session code: this host now runs another session ({code}) - ask for the current code"));
            return;
        }
        let bus = vehicle_path(field(parts, 4)).unwrap_or_default();
        let world = WorldInfo::from_fields(parts, 5);
        let nonce = u64::from_str_radix(field(parts, 10), 16).ok();
        // a returning client keeps its id; so does one that tried several of our addresses
        // at once, and the address its latest hello came from is the one it has chosen
        let same = |p: &Peer| p.addr == Some(from) || (nonce.is_some() && p.nonce == nonce);
        let (id, here) = match self.peers.iter().find(|(_, p)| same(p)) {
            Some((id, _)) => {
                let id = *id;
                if let Some(p) = self.peers.get_mut(&id) {
                    if p.addr != Some(from) {
                        log::info!("LAN: player {id} now talks from {from}");
                        p.addr = Some(from);
                    }
                }
                (id, None)
            }
            None if self.peers.len() >= MAX_PEERS => {
                log::warn!("LAN: '{name}' at {from} wants to join, but {MAX_PEERS} players are here already; turned away");
                self.reject(
                    from,
                    &format!("the session is full ({} players)", MAX_PEERS + 1),
                );
                return;
            }
            None if self.peers.values().filter(|p| !p.has_pose).count() >= MAX_JOINING => {
                self.reject(
                    from,
                    "the host is letting other players in - try again in a minute",
                );
                return;
            }
            None if nonce.is_some_and(|n| self.banned.iter().any(|b| b.0 == n)) => {
                let why = self.banned.iter().find(|b| Some(b.0) == nonce).map(|b| b.1.clone()).unwrap_or_default();
                self.reject(from, &if why.is_empty() { "the host has sent you away from this session".to_string() } else { format!("the host has sent you away from this session: {why}") });
                return;
            }
            None => {
                // back after a lost connection (or a restart of the game): the number it had
                self.gone_lately.retain(|g| g.3.elapsed() < Duration::from_secs(300));
                let back = self
                    .gone_lately
                    .iter()
                    // by the nonce only: a name is no proof (anybody may call themselves
                    // so); a game without a nonce (none sends none now) by its name
                    .position(|g| match (nonce, g.0) {
                        (Some(a), Some(b)) => a == b,
                        (None, None) => g.1 == name,
                        _ => false,
                    })
                    .map(|k| self.gone_lately.remove(k));
                let id = match back.as_ref().map(|g| g.2).filter(|i| !self.peers.contains_key(i)) {
                    Some(i) => i,
                    None => self.fresh_id(),
                };
                // who is here already, for the newcomer
                let my_bus = self
                    .last_info
                    .split('|')
                    .nth(3)
                    .map(vehicle_label)
                    .filter(|b| !b.is_empty());
                let mut here: Vec<String> = vec![format!(
                    "{} (host{})",
                    self.my_name,
                    my_bus.map(|b| format!(", {b}")).unwrap_or_default()
                )];
                let mut others: Vec<(u32, String)> = self
                    .peers
                    .iter()
                    .map(|(pid, p)| {
                        (
                            *pid,
                            if p.pose.bus.is_empty() {
                                p.label()
                            } else {
                                format!("{} ({})", p.label(), vehicle_label(&p.pose.bus))
                            },
                        )
                    })
                    .collect();
                others.sort();
                here.extend(others.into_iter().map(|(_, s)| s));
                let mut peer = Peer::new(Some(from), Pose::placeholder(id, &name, &bus));
                peer.nonce = nonce;
                self.peers.insert(id, peer);
                log::info!(
                    "LAN: player {id} '{name}' joined from {from} with {} on {}",
                    if bus.is_empty() { "no vehicle" } else { &bus },
                    world.map
                );
                let what = if bus.is_empty() {
                    String::new()
                } else {
                    format!(" with {}", vehicle_label(&bus))
                };
                if back.is_some() {
                    log::info!("LAN: player {id} '{name}' is back");
                    self.notice(format!("{name} is back"), Some(id));
                } else {
                    self.notice(format!("{name} joined{what}"), Some(id));
                }
                (id, Some(here.join(", ")))
            }
        };
        if let Some(p) = self.peers.get_mut(&id) {
            p.last_seen = Instant::now();
        }
        self.send_welcome(id, from);
        if let Some(here) = here {
            self.send(
                format!("NOTE|In this session: {}", clean_text(&here, 200)).as_bytes(),
                from,
            );
        }
    }

    /// The next player id nobody has (2 … 65535; the host is 1).
    fn fresh_id(&mut self) -> u32 {
        loop {
            let id = self.next_id;
            self.next_id = if self.next_id >= u16::MAX as u32 {
                2
            } else {
                self.next_id + 1
            };
            if !self.peers.contains_key(&id) {
                return id;
            }
        }
    }

    fn on_welcome(&mut self, parts: &[&str], from: SocketAddr) {
        let was_timed_out = self.timed_out;
        let proto = field(parts, 1).parse::<u32>().unwrap_or(1);
        if proto != PROTOCOL {
            let why = format!("the host runs LAN protocol {proto}, this game protocol {PROTOCOL}");
            self.turned_away = Some(why.clone());
            self.rejected = Some(why);
            return;
        }
        let Some(id) = field(parts, 2)
            .parse::<u32>()
            .ok()
            .filter(|id| (2..=u16::MAX as u32).contains(id))
        else {
            return;
        };
        let session = parse_session_hex(field(parts, 3)).unwrap_or(0);
        if self.session_required && session != self.session {
            self.rejected = Some(format!(
                "the host answered with session {} instead of {}",
                session_hex(session),
                session_hex(self.session)
            ));
            return;
        }
        let host_name = clean_text(field(parts, 4), MAX_NAME);
        let world = WorldInfo::from_fields(parts, 5);
        let players = field(parts, 10)
            .parse::<usize>()
            .unwrap_or(1)
            .min(MAX_PEERS + 1);
        let first = !self.connected;
        if self.host != Some(from) {
            self.host = Some(from);
            if self.candidates.len() > 1 {
                // the host heard our hellos on all its addresses: this one, sent last,
                // tells it which of them we talk to from now on
                log::info!("LAN: the host answered at {from}");
                self.confirm = true;
            }
        }
        self.my_id = id;
        self.session = session;
        self.connected = true;
        self.lost_at = None;
        if was_timed_out {
            // the host is back after we had given up
            self.timed_out = false;
            self.rejected = None;
        }
        self.host_seen = Instant::now();
        if first {
            log::info!("LAN: connected to '{host_name}' (session {}), we are player {id}; the host's world: {} {} {:02}:{:02} weather '{}' season '{}'", session_hex(session), world.map, world.date, (world.time / 3600.0) as i32, ((world.time % 3600.0) / 60.0) as i32, world.weather, world.season);
            self.events.push(LanEvent::Notice(format!(
                "connected to {host_name}'s session ({players} player(s))"
            )));
            self.welcomes += 1;
            self.welcome = Some(Welcome {
                host_name,
                session,
                world: world.clone(),
                at: Instant::now(),
                players,
            });
            self.host_clock = Some(HostClock {
                world: world.clone(),
                at: Instant::now(),
                speed: self.clock_speed,
            });
            self.compare_worlds(&world);
            // the host forgot us (it restarted): the spawn question goes again
            if self.place.is_some() && self.near.is_none() {
                self.place_acc = 1.0;
            }
        }
    }

    /// Tell the others we are leaving.
    pub fn leave(&self) {
        let msg = format!("BYE|{}", self.my_id);
        match self.role {
            Role::Host => self.broadcast(msg.as_bytes(), None),
            Role::Client => {
                if let (Some(h), true) = (self.host, self.connected) {
                    self.send(msg.as_bytes(), h);
                }
            }
        }
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.socket.local_addr().ok()
    }
}

impl Drop for LanSession {
    fn drop(&mut self) {
        self.leave();
    }
}

#[cfg(test)]
mod tests;
