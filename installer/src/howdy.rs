//! /etc/howdy/config.ini edits and the checks the scripts shared.

use crate::sys::{self, Result};

pub const CFG: &str = "/etc/howdy/config.ini";
pub const COMPARE: &str = "/usr/lib/howdy/compare.py";

/// Is `line` the setting `key` (`^key *=`)?
fn is_key(line: &str, key: &str) -> bool {
    line.strip_prefix(key).is_some_and(|r| r.trim_start_matches(' ').starts_with('='))
}

/// The value of `key`, from the first line that sets it.
pub fn get(key: &str) -> Option<String> {
    sys::read(CFG)
        .lines()
        .find(|l| is_key(l, key))
        .and_then(|l| l.split_once('='))
        .map(|(_, v)| v.trim().to_string())
}

/// `sed -i "s|^key *=.*|key = value|"`, then show the line, as the scripts did.
pub fn set(key: &str, value: &str) -> Result {
    set_all(&[(key, value)])
}

/// Several settings in one write, so the config is never left half-switched.
pub fn set_all(pairs: &[(&str, &str)]) -> Result {
    let mut text = sys::read_root(CFG)?;
    for (key, value) in pairs {
        text = text
            .lines()
            .map(|l| if is_key(l, key) { format!("{key} = {value}\n") } else { format!("{l}\n") })
            .collect();
    }
    sys::sudo_write(CFG, &text)?;
    for (key, _) in pairs {
        for l in text.lines().filter(|l| l.starts_with(&format!("{key} "))) {
            println!("{l}");
        }
    }
    Ok(())
}

/// Howdy set up for the XPS 14 IR camera (by `face-unlock ir-howdy`)?
pub fn uses_ir() -> bool {
    get("recording_plugin").is_some_and(|v| v.starts_with("ir"))
}

/// The IPU7 RGB camera's relay node ("Hardware ISP Camera"), if present.
pub fn rgb_camera() -> Option<String> {
    let dir = std::fs::read_dir("/sys/class/video4linux").ok()?;
    let mut nodes: Vec<_> = dir.flatten().map(|e| e.path()).collect();
    nodes.sort();
    nodes.into_iter().find_map(|d| {
        let name = std::fs::read_to_string(d.join("name")).ok()?;
        let node = d.file_name()?.to_string_lossy().into_owned();
        (name.trim() == "Hardware ISP Camera").then(|| format!("/dev/{node}"))
    })
}

/// Start the IPU7 RGB relay if it isn't running.
pub fn start_rgb_relay() -> Result {
    if !sys::ok(&mut sys::cmd("systemctl", &["is-active", "-q", "v4l2-relayd@ipu7"])) {
        sys::run(&mut sys::sudo(&["systemctl", "start", "v4l2-relayd@ipu7"]))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn matches_keys_like_the_sed_pattern() {
        assert!(super::is_key("timeout = 4", "timeout"));
        assert!(super::is_key("timeout=4", "timeout"));
        assert!(!super::is_key("timeout_notice = true", "timeout"));
        assert!(!super::is_key("# timeout = 4", "timeout"));
    }
}
