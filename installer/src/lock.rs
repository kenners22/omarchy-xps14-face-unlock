//! `face-unlock lock`: the lock screen scans your face by itself (no key
//! press needed). Was lock/enable-lock-autoscan.sh.
//!
//! Works with the patched lock screen (lock/face-lock.patch on a clone of
//! omarchy.lock), which starts a face scan as soon as
//! /etc/pam.d/omarchy-lock-face exists.

use crate::pam::{self, HOWDY_REQUIRED, HOWDY_SUFFICIENT, LOCK_FACE, LOCK_PASSWORD, WARM_CLIENT, WARM_REQUIRED, WARM_SUFFICIENT};
use crate::sys::{self, green, run, sudo, Result};

pub fn undo() -> Result {
    let old = sys::read_root(LOCK_PASSWORD)?;
    if !old.lines().any(|l| l.contains("pam_faillock.so preauth")) {
        crate::bail!("{LOCK_PASSWORD} has no pam_faillock preauth line to put face unlock after. Nothing changed.");
    }
    // Back to "any key + Enter, then look at the camera", through the warm
    // client when sudo uses it
    let face = if pam::warm_in_use() { WARM_SUFFICIENT } else { HOWDY_SUFFICIENT };
    sys::edit(LOCK_PASSWORD, |t| {
        if t.contains("pam_howdy.so") || t.contains(WARM_CLIENT) {
            t.to_string()
        } else {
            sys::insert_after(t, |l| l.contains("pam_faillock.so preauth"), face)
        }
    })?;
    run(&mut sudo(&["rm", "-f", LOCK_FACE]))?;
    println!("Auto-scan off. Lock screen is back to: type any key + Enter, then look at the camera.");
    Ok(())
}

pub fn install() -> Result {
    // Face-only stack for the lock screen. faillock preauth first, so a password
    // lockout (10 wrong attempts) blocks face unlock too. With the warm service
    // installed, it goes straight through the warm client.
    let face = if sys::exists(WARM_CLIENT) && pam::warm_in_use() { WARM_REQUIRED } else { HOWDY_REQUIRED };
    sys::sudo_write(
        LOCK_FACE,
        &format!(
            "#%PAM-1.0
# Face unlock for the Omarchy lock screen (Howdy). See github.com/kenners22/omarchy-xps14-face-unlock
auth       required                    pam_faillock.so preauth silent deny=10 unlock_time=120
{face}
account    include                     system-local-login
"
        ),
    )?;

    // The Enter key now checks the password only, so it never fights the
    // automatic scan for the camera.
    sys::edit(LOCK_PASSWORD, |t| sys::delete_lines(t, "pam_howdy.so"))?;

    green("Done. Lock the screen (Super+Ctrl+L) and just look at the camera.");
    println!("Undo: face-unlock lock --undo");
    Ok(())
}
