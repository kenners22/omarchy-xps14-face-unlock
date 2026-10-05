//! `face-unlock warm`: fast face unlock. Howdy's face recognition kept loaded
//! in a background service (howdy-warmd: Rust, with dlib reached through C++)
//! and a small client (howdy-warm-auth) that sudo, polkit and the lock screen
//! run through pam_exec instead of pam_howdy. Cuts ~0.5 s off every scan.
//! Was warm/install-warm.sh.
//!
//! If the service is down, the client hands over to stock Howdy (compare.py),
//! so face unlock keeps working; the password is the fallback everywhere.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::bail;
use crate::pam::{HOWDY_REQUIRED, HOWDY_SUFFICIENT, WARM_CLIENT, WARM_REQUIRED, WARM_SUFFICIENT, WARM_STACKS};
use crate::sys::{self, cmd, green, output, run, stamp, sudo, user, Result};

const LIB: &str = "/usr/local/lib/howdy-warm";
const UNIT: &str = "/etc/systemd/system/howdy-warm.service";
const SOCKET: &str = "/run/howdy-warm/socket";

/// Swap pam_howdy <-> the warm client in every stack that has it.
fn switch_pam(to_warm: bool) -> Result {
    let pairs = [(HOWDY_SUFFICIENT, WARM_SUFFICIENT), (HOWDY_REQUIRED, WARM_REQUIRED)];
    for f in WARM_STACKS {
        if !sys::exists(f) {
            continue;
        }
        let changed = sys::edit(f, |t| {
            pairs.iter().fold(t.to_string(), |t, (howdy, warm)| {
                if to_warm {
                    sys::swap_line(&t, howdy, warm)
                } else {
                    sys::swap_line(&t, warm, howdy)
                }
            })
        })?;
        if changed {
            println!("  {f}");
        }
    }
    Ok(())
}

pub fn undo() -> Result {
    sys::sudo_v()?;
    green("Putting pam_howdy back...");
    switch_pam(false)?;
    sys::ok(&mut sudo(&["systemctl", "disable", "--now", "howdy-warm.service"]));
    run(&mut sudo(&["rm", "-f", UNIT]))?;
    run(&mut sudo(&["systemctl", "daemon-reload"]))?;
    run(&mut sudo(&["rm", "-rf", LIB]))?;
    println!("Done: stock Howdy again. (dlib and openblas stay installed: sudo pacman -R dlib openblas)");
    Ok(())
}

fn cargo() -> String {
    for c in ["cargo".to_string(), format!("{}/.cargo/bin/cargo", std::env::var("HOME").unwrap_or_default())] {
        if sys::ok(&mut cmd(&c, &["--version"])) {
            return c;
        }
    }
    output(&mut cmd("mise", &["which", "cargo"])).unwrap_or_else(|_| "cargo".into())
}

pub fn install() -> Result {
    sys::sudo_v()?;
    let repo = sys::repo();

    // 1. dlib's C++ library (the Python one Howdy uses is built into python-dlib),
    // and OpenBLAS for its maths: a separate library next to the system BLAS,
    // which stays as it is
    if !sys::ok(&mut cmd("pacman", &["-Q", "dlib", "openblas"])) {
        green("Installing dlib and OpenBLAS...");
        run(&mut sudo(&["pacman", "-S", "--needed", "dlib", "openblas"]))?;
    }

    // 2. Build the service and the client (as you, not root)
    green("Building howdy-warmd and howdy-warm-auth...");
    run(cmd(&cargo(), &["build", "--release", "--quiet", "-p", "howdy-warmd", "-p", "howdy-warm-auth"]).current_dir(&repo))?;

    // 3. Install service + client, start it
    green("Installing and starting howdy-warm...");
    let bin = repo.join("target/release");
    let src = |name: &str| bin.join(name).to_string_lossy().into_owned();
    run(&mut sudo(&["install", "-D", "-m", "0755", &src("howdy-warm-auth"), WARM_CLIENT]))?;
    run(&mut sudo(&["install", "-D", "-m", "0755", &src("howdy-warmd"), &format!("{LIB}/howdy-warmd")]))?;
    let unit = repo.join("warm/howdy-warm.service");
    run(&mut sudo(&["install", "-m", "0644", &unit.to_string_lossy(), UNIT]))?;
    run(&mut sudo(&["systemctl", "daemon-reload"]))?;
    run(&mut sudo(&["systemctl", "enable", "--now", "howdy-warm.service"]))?;
    run(&mut sudo(&["systemctl", "restart", "howdy-warm.service"]))?;
    let start = Instant::now();
    while !Path::new(SOCKET).exists() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(250));
    }
    if !Path::new(SOCKET).exists() {
        bail!("Service didn't start: journalctl -u howdy-warm");
    }

    // 4. Prove it works before touching PAM
    green("Testing (look at the camera)...");
    if sys::code(&mut cmd(WARM_CLIENT, &[&user()])) != 0 {
        bail!("Not recognised. PAM left unchanged.");
    }

    // 5. Switch PAM
    green("Switching PAM to the warm client:");
    switch_pam(true)?;

    println!();
    green("Done. Try: sudo -k; sudo true");
    println!("Backups: /etc/pam.d/*.bak.{}   Undo: face-unlock warm --undo", stamp());
    Ok(())
}
