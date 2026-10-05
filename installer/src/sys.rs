//! Running commands, sudo and file edits: what the bash scripts did with
//! `set -e`, `sudo`, `sed -i` and `grep`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T = ()> = std::result::Result<T, String>;

/// Print a message and stop, like a bash script's `echo ...; exit 1`.
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => { return Err(format!($($arg)*)) };
}

pub fn green(msg: &str) {
    println!("\x1b[32m{msg}\x1b[0m");
}

pub fn yellow(msg: &str) {
    println!("\x1b[33m{msg}\x1b[0m");
}

/// One timestamp per run, for `.bak.<stamp>` backups.
pub fn stamp() -> u64 {
    static STAMP: OnceLock<u64> = OnceLock::new();
    *STAMP.get_or_init(|| SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}

pub fn user() -> String {
    std::env::var("USER").unwrap_or_default()
}

/// The repository this binary was built from (target/release/face-unlock -> repo).
pub fn repo() -> PathBuf {
    let from_exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.ancestors().nth(3).map(Path::to_path_buf))
        .filter(|r| r.join("warm/daemon/Cargo.toml").exists());
    from_exe.unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
}

fn describe(cmd: &Command) -> String {
    let mut s = cmd.get_program().to_string_lossy().into_owned();
    for a in cmd.get_args() {
        s.push(' ');
        s.push_str(&a.to_string_lossy());
    }
    s
}

/// Run with the terminal attached; a non-zero exit is an error.
pub fn run(cmd: &mut Command) -> Result {
    let status = cmd.status().map_err(|e| format!("{}: {e}", describe(cmd)))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("failed ({status}): {}", describe(cmd)))
    }
}

/// Run quietly; true if it exited 0.
pub fn ok(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Exit code of a command run with the terminal attached (-1 if it couldn't run).
pub fn code(cmd: &mut Command) -> i32 {
    cmd.status().ok().and_then(|s| s.code()).unwrap_or(-1)
}

/// Stdout of a command (stderr shown); error on a non-zero exit.
pub fn output(cmd: &mut Command) -> Result<String> {
    let out = cmd.stderr(Stdio::inherit()).output().map_err(|e| format!("{}: {e}", describe(cmd)))?;
    if !out.status.success() {
        bail!("failed ({}): {}", out.status, describe(cmd));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn cmd(program: &str, args: &[&str]) -> Command {
    let mut c = Command::new(program);
    c.args(args);
    c
}

pub fn sudo(args: &[&str]) -> Command {
    cmd("sudo", args)
}

/// Ask for the sudo password up front.
pub fn sudo_v() -> Result {
    run(&mut sudo(&["-v"]))
}

/// Write a root-owned file (keeps its owner and mode if it exists).
pub fn sudo_write(path: &str, content: &str) -> Result {
    let mut child = sudo(&["tee", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| format!("sudo tee {path}: {e}"))?;
    child.stdin.take().unwrap().write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        bail!("couldn't write {path}");
    }
    Ok(())
}

/// `sudo cp -a FILE FILE.bak.<stamp>`, if FILE exists.
pub fn backup(path: &str) -> Result {
    if Path::new(path).exists() {
        run(&mut sudo(&["cp", "-a", path, &format!("{path}.bak.{}", stamp())]))?;
    }
    Ok(())
}

/// Contents of a file, or "" if it's missing or unreadable. Only for checks:
/// anything that will be written back goes through `read_root`.
pub fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Contents of a root-owned file that is about to be rewritten. Never "" by
/// accident: unreadable as us means `sudo cat`, and a missing file is an error,
/// so an edit can't replace a file it couldn't see.
pub fn read_root(path: &str) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let out = sudo(&["cat", path]).stderr(Stdio::inherit()).output().map_err(|e| e.to_string())?;
            if !out.status.success() {
                bail!("couldn't read {path}");
            }
            String::from_utf8(out.stdout).map_err(|_| format!("{path} isn't text"))
        }
        Err(e) => bail!("{path}: {e}"),
    }
}

pub fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// Lines of `text` with `new` added after the first line that `after` matches
/// (`sed "/pattern/a new"` adds after every match; these files have one).
pub fn insert_after(text: &str, after: impl Fn(&str) -> bool, new: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(line);
        out.push('\n');
        if after(line) {
            out.push_str(new);
            out.push('\n');
        }
    }
    out
}

/// Lines of `text` without the ones containing `needle` (`sed "/needle/d"`).
pub fn delete_lines(text: &str, needle: &str) -> String {
    text.lines().filter(|l| !l.contains(needle)).map(|l| format!("{l}\n")).collect()
}

/// Replace whole lines equal to `from` with `to`.
pub fn swap_line(text: &str, from: &str, to: &str) -> String {
    text.lines().map(|l| if l == from { format!("{to}\n") } else { format!("{l}\n") }).collect()
}

/// Edit a root-owned file: back it up and write the result, if it changed.
pub fn edit(path: &str, f: impl Fn(&str) -> String) -> Result<bool> {
    let old = read_root(path)?;
    let new = f(&old);
    if new == old {
        return Ok(false);
    }
    backup(path)?;
    sudo_write(path, &new)?;
    Ok(true)
}

// --- Ctrl-C during an enrol: note it, let the child die, then put things back

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_interrupt(_: i32) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

pub fn catch_interrupts() {
    for sig in [libc::SIGINT, libc::SIGTERM] {
        unsafe { libc::signal(sig, on_interrupt as extern "C" fn(i32) as libc::sighandler_t) };
    }
}

pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_edits() {
        let pam = "#%PAM-1.0\nauth include system-auth\n";
        assert_eq!(insert_after(pam, |l| l.starts_with("#%PAM"), "X"), "#%PAM-1.0\nX\nauth include system-auth\n");
        assert_eq!(delete_lines("a\npam_howdy.so b\nc\n", "pam_howdy.so"), "a\nc\n");
        assert_eq!(swap_line("a\nb\n", "b", "B"), "a\nB\n");
    }
}
