//! /etc/howdy/config.ini, read the way Python's configparser reads it: keys
//! are case-insensitive, `#` and `;` start a comment only at the start of a
//! line, and booleans are 1/yes/true/on or 0/no/false/off.

use std::collections::HashMap;
use std::fs;

pub const PATH: &str = "/etc/howdy/config.ini";

#[derive(Default)]
pub struct Config {
    values: HashMap<(String, String), String>,
}

impl Config {
    pub fn load() -> Config {
        fs::read_to_string(PATH).map(|s| Config::parse(&s)).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Config {
        let mut values = HashMap::new();
        let mut section = String::new();
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') || t.starts_with(';') {
                continue;
            }
            if let Some(name) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = name.trim().to_string();
            } else if let Some((k, v)) = t.split_once(['=', ':']) {
                values.insert((section.clone(), k.trim().to_lowercase()), v.trim().to_string());
            }
        }
        Config { values }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.values.get(&(section.to_string(), key.to_string())).map(String::as_str)
    }

    pub fn bool(&self, section: &str, key: &str, default: bool) -> bool {
        match self.get(section, key).map(str::to_lowercase).as_deref() {
            Some("1" | "yes" | "true" | "on") => true,
            Some("0" | "no" | "false" | "off") => false,
            _ => default,
        }
    }

    pub fn int(&self, section: &str, key: &str, default: i64) -> i64 {
        self.get(section, key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    pub fn float(&self, section: &str, key: &str, default: f64) -> f64 {
        self.get(section, key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_howdy_style_ini() {
        let c = Config::parse(
            "[core]\n# comment\nabort_if_ssh = true\nuse_cnn = False\n\n[video]\nCertainty = 3.5\ntimeout = 6\n; x\ndevice_path = /dev/video16\n",
        );
        assert!(c.bool("core", "abort_if_ssh", false));
        assert!(!c.bool("core", "use_cnn", true));
        assert_eq!(c.float("video", "certainty", 0.0), 3.5);
        assert_eq!(c.int("video", "timeout", 4), 6);
        assert_eq!(c.int("video", "missing", 4), 4);
        assert_eq!(c.get("video", "device_path"), Some("/dev/video16"));
    }
}
