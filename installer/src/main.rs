//! face-unlock: IR face unlock for the Dell XPS 14 (2026) on Omarchy.
//!
//! Each step is a subcommand, run in this order; `--undo` reverses it. Steps
//! use sudo for what needs root, so run this as yourself.

mod howdy;
mod ir;
mod lock;
mod pam;
mod setup;
mod sys;
mod warm;

use std::process::exit;

const USAGE: &str = "\
usage: face-unlock STEP [--undo]

Steps, in order:
  setup      Howdy on the webcam first (works in light): install, enrol, add to PAM
  ir         IR camera kernel modules (snapshot first); reboot afterwards
  ir-howdy   Howdy on the IR camera, re-enrol
  warm       fast path: the warm service and its PAM client (installs dlib, openblas)
  lock       lock screen scans by itself (after patching the lock screen:
             see lock/face-lock.patch)

--undo reverses a step. For a camera of your own (e.g. a USB IR camera), run
setup as: FACE_UNLOCK_CAMERA=/dev/v4l/by-path/... face-unlock setup";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let undo = args.iter().any(|a| a == "--undo");
    let step = args.iter().find(|a| !a.starts_with("--")).map(String::as_str);
    if args.iter().any(|a| a == "-h" || a == "--help") || step.is_none() {
        println!("{USAGE}");
        exit(if step.is_none() && !args.iter().any(|a| a == "-h" || a == "--help") { 2 } else { 0 });
    }
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("Run this as yourself, not root: it uses sudo where it needs to.");
        exit(1);
    }

    let result = match (step.unwrap(), undo) {
        ("setup", false) => setup::install(),
        ("setup", true) => setup::undo(),
        ("ir", false) => ir::install_modules(),
        ("ir", true) => ir::remove_modules(),
        ("ir-howdy", false) => ir::enable_howdy(),
        ("ir-howdy", true) => ir::disable_howdy(),
        ("warm", false) => warm::install(),
        ("warm", true) => warm::undo(),
        ("lock", false) => lock::install(),
        ("lock", true) => lock::undo(),
        (other, _) => {
            eprintln!("unknown step: {other}\n\n{USAGE}");
            exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("{e}");
        exit(1);
    }
}
