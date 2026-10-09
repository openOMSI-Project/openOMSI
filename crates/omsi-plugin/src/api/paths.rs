//! The paths a plugin names: always relative to one of its own folders, never out of it.

use std::path::{Component, Path, PathBuf};

/// Longest path a plugin may give.
const MAX_PATH: usize = 260;

/// `rel` inside `base`: `/` or `\` separated, no `..`, no root, no drive, no device names, no
/// empty or dotted-only parts. The error says what is wrong.
pub fn inside(base: &Path, rel: &str) -> Result<PathBuf, String> {
    let clean = clean(rel)?;
    Ok(if clean.is_empty() { base.to_path_buf() } else { base.join(clean) })
}

/// `rel` checked and written with `/` (empty: the folder itself).
pub fn clean(rel: &str) -> Result<String, String> {
    if rel.len() > MAX_PATH {
        return Err(format!("a path is at most {MAX_PATH} characters"));
    }
    if rel.contains('\0') {
        return Err("the path holds a zero character".into());
    }
    let mut parts = Vec::new();
    for part in rel.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return Err(format!("\"{rel}\": a path cannot go up (..)")),
            _ => {}
        }
        if part.contains(':') {
            return Err(format!("\"{rel}\": no drive or stream names (:)"));
        }
        if part.trim_end_matches(['.', ' ']).is_empty() || part.ends_with(['.', ' ']) {
            return Err(format!("\"{rel}\": a name cannot end in a dot or a space"));
        }
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4" | "LPT1" | "LPT2" | "LPT3") {
            return Err(format!("\"{rel}\": {stem} is a device name"));
        }
        // (a component the system reads as a root - only reachable through the checks above
        // on odd systems, but checked as the last word)
        if Path::new(part).components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(format!("\"{rel}\": not a plain name"));
        }
        parts.push(part);
    }
    if rel.starts_with(['/', '\\']) {
        return Err(format!("\"{rel}\": the path is relative to the plugin's folder"));
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside() {
        let base = Path::new("/data/p");
        assert_eq!(inside(base, "a/b.txt").unwrap(), base.join("a/b.txt"));
        assert_eq!(inside(base, "a\\b.txt").unwrap(), base.join("a/b.txt"));
        assert_eq!(inside(base, "./x").unwrap(), base.join("x"));
        assert_eq!(inside(base, "").unwrap(), base);
        for bad in ["../x", "a/../../x", "/etc/passwd", "\\x", "C:/x", "c:x", "a/NUL", "con.txt", "x.", "x ", "...", "a\0b"] {
            assert!(inside(base, bad).is_err(), "{bad}");
        }
    }
}
