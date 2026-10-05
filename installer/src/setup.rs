//! `face-unlock setup`: Howdy for sudo, polkit (incl. 1Password) and the
//! Omarchy lock screen. Was setup/setup-face-unlock.sh.
//!
//! Camera per machine:
//!   FACE_UNLOCK_CAMERA=/dev/...  a camera of your own, e.g. a USB IR camera
//!           (greyscale, works in the dark): use a stable /dev/v4l/by-path/ name
//!   XPS 14  before `face-unlock ir`: the RGB camera through the Intel IPU7 relay
//!           (v4l2loopback, "Hardware ISP Camera"). Needs light on your face, and
//!           a good photo could fool it. After `face-unlock ir-howdy`: the IR camera.

use crate::howdy;
use crate::pam::{self, HOWDY_SUFFICIENT};
use crate::sys::{self, cmd, green, run, stamp, sudo, user, yellow, Result};
use crate::bail;

pub fn undo() -> Result {
    if pam::warm_in_use() {
        bail!("Face unlock goes through the warm service. First: face-unlock warm --undo");
    }
    for f in [pam::SUDO, pam::POLKIT, pam::LOCK_PASSWORD] {
        if sys::edit(f, |t| sys::delete_lines(t, "pam_howdy.so"))? {
            println!("Removed from {f}");
        }
    }
    // Lock-screen auto-scan stack (`face-unlock lock`)
    if sys::exists(pam::LOCK_FACE) {
        run(&mut sudo(&["rm", pam::LOCK_FACE]))?;
        println!("Removed {}", pam::LOCK_FACE);
    }
    let left: Vec<String> = std::fs::read_dir("/etc/pam.d")
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path().to_string_lossy().into_owned())
        .filter(|p| !p.contains(".bak.") && sys::read(p).contains("pam_howdy.so"))
        .collect();
    if !left.is_empty() {
        println!("{}", left.join("\n"));
        bail!("pam_howdy is still in the files above. Remove it by hand.");
    }
    println!("Face unlock disabled (howdy-git is still installed; remove with: omarchy pkg remove howdy-git)");
    Ok(())
}

pub fn install() -> Result {
    sys::sudo_v()?;
    let me = user();

    // 1. Install (python-dlib compiles from source: this can take 10-20 minutes)
    if !sys::ok(&mut cmd("pacman", &["-Q", "howdy-git"])) {
        green("Installing Howdy...");
        run(&mut cmd("omarchy", &["pkg", "aur", "add", "howdy-git"]))?;
    }

    // 2. Point Howdy at this machine's camera
    green("Configuring camera...");
    if howdy::uses_ir() {
        // XPS 14 IR camera, set up by `face-unlock ir-howdy`: leave it alone
        for key in ["recording_plugin", "device_path"] {
            println!("{key} = {}", howdy::get(key).unwrap_or_default());
        }
    } else if let Some(camera) = std::env::var("FACE_UNLOCK_CAMERA").ok().filter(|c| !c.is_empty()) {
        if !sys::exists(&camera) {
            bail!("FACE_UNLOCK_CAMERA={camera} doesn't exist. Stopping before touching PAM.");
        }
        howdy::set("device_path", &camera)?;
    } else if let Some(camera) = howdy::rgb_camera() {
        // The IPU7 relay sends black frames for the first ~1.5 s after opening;
        // Howdy skips them as too dark, so give it longer before giving up.
        howdy::start_rgb_relay()?;
        howdy::set("device_path", &camera)?;
        howdy::set("timeout", "6")?;
        yellow("RGB camera only (no IR driver on this laptop): needs light on your face.");
    } else {
        bail!("No supported camera found. Stopping before touching PAM.");
    }

    // 3. Enroll your face
    if !sys::ok(&mut sudo(&["howdy", "-U", &me, "list"])) {
        green("Look at the camera to enroll your face.");
        run(&mut sudo(&["howdy", "-U", &me, "add"]))?;
    }

    // 4. Verify before touching PAM: stop if it can't recognise you.
    // (Not `howdy test`: that opens a preview window, which root can't on Wayland.)
    // compare.py is what pam_howdy runs; exit 0 = recognised.
    green("Testing recognition (look at the camera)...");
    match sys::code(&mut sudo(&["python3", howdy::COMPARE, &me])) {
        0 => println!("Recognised."),
        rc => {
            let why = match rc {
                10 => "no face model enrolled".to_string(),
                11 => "timed out without recognising you".to_string(),
                13 => "too dark".to_string(),
                _ => format!("exit code {rc}"),
            };
            bail!("Not recognised ({why}). Stopping before enabling PAM. Try more light, or: sudo howdy add");
        }
    }

    // 5. Enable in PAM (the password stays the fallback everywhere)
    green("Enabling face unlock...");
    let after_first_line = |t: &str| {
        if t.contains("pam_howdy.so") || pam::warm_in_use() {
            return t.to_string();
        }
        let mut lines: Vec<&str> = t.lines().collect();
        lines.insert(1.min(lines.len()), HOWDY_SUFFICIENT);
        lines.iter().map(|l| format!("{l}\n")).collect()
    };

    // sudo
    sys::edit(pam::SUDO, after_first_line)?;

    // polkit (system dialogs + 1Password "unlock using system authentication")
    if !sys::exists(pam::POLKIT) {
        run(&mut sudo(&["cp", "/usr/lib/pam.d/polkit-1", pam::POLKIT]))?;
    }
    sys::edit(pam::POLKIT, after_first_line)?;

    // Lock screen, after the faillock preauth line. Skipped when auto-scan is on:
    // then omarchy-lock-face does the scanning and Enter must stay password-only,
    // or both stacks fight over the camera.
    if !sys::exists(pam::LOCK_FACE) {
        sys::edit(pam::LOCK_PASSWORD, |t| {
            if t.contains("pam_howdy.so") {
                t.to_string()
            } else {
                sys::insert_after(t, |l| l.contains("pam_faillock.so preauth"), HOWDY_SUFFICIENT)
            }
        })?;
    }

    println!();
    green("Done. Try it: open a new terminal and run  sudo -k; sudo true");
    println!("Backups: /etc/pam.d/*.bak.{}   Undo: face-unlock setup --undo", stamp());
    Ok(())
}
