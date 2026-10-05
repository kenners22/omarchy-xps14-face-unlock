//! `face-unlock ir`: the XPS 14 IR camera (Himax HM1092). Adds two kernel
//! modules next to Omarchy's camera stack; intel-cvs / ipu7-drivers stay as
//! they are. Was ir/install-ir.sh.
//!
//!   hm1092           IR sensor driver (community, reverse-engineered)
//!   ipu-bridge-himx  Omarchy's ipu-bridge + the HIMX1092 entry from kernel 7.3,
//!                    so the sensor gets a place in the camera graph
//!
//! `face-unlock ir-howdy`: switch Howdy from the RGB camera to the IR camera
//! (works in the dark). Was ir/enable-ir-howdy.sh.

use crate::bail;
use crate::howdy::{self, CFG};
use crate::sys::{self, cmd, green, output, run, stamp, sudo, user, Result};

const KERNEL: &str = "7.2.5-3-omarchy";
const MODULES: [(&str, &str, &str); 2] = [
    // (dkms name, version, folder under ir/)
    ("hm1092", "1.1", "hm1092"),
    ("ipu-bridge-himx", "7.2.5.3", "ipu-bridge"),
];

const REC: &str = "/usr/lib/howdy/recorders";
const RULE: &str = "/etc/udev/rules.d/72-hm1092-ir-capture.rules";
const IR_NODE_NAME: &str = "name=Intel IPU7 ISYS Capture 16";
// Howdy's IR reader and its hook come from the fix pack (no licence, so they are
// fetched rather than copied here), pinned and checksum-checked, then patched with
// ir/howdy/ir_reader-fixes.patch (frame timeout, absolute media-ctl, ignore size changes).
const FIXPACK: &str =
    "https://raw.githubusercontent.com/HritwikSinghal/svp7500-camera-fix-pack/25cc4c9e282912a404cfb188283964ed983e87ed/howdy";
const READER_SHA: &str = "78b7fd214e12c50769116295b2e8d10c09dbab0c94ac6499228ee3d4b64ab0f0";
const HOOK_SHA: &str = "b226d56805b545c2b83992cc19281679b4b317d5b22824049a83dae13ffcc335";

fn uname() -> String {
    output(&mut cmd("uname", &["-r"])).unwrap_or_default()
}

fn module_file(module: &str) -> String {
    output(&mut cmd("modinfo", &["-F", "filename", module])).unwrap_or_default()
}

// --- kernel modules ---------------------------------------------------------

pub fn install_modules() -> Result {
    sys::sudo_v()?;
    let running = uname();
    if running != KERNEL {
        bail!("This ipu-bridge source is exact for {KERNEL}, but you're on {running}. Stopping.");
    }

    // 1. Bootable snapshot first, so a bad result is one boot-menu pick away
    green("Taking a snapshot...");
    let snap = output(&mut sudo(&[
        "snapper", "-c", "root", "create", "-t", "single", "-c", "number", "-u", "important=yes", "-p", "-d",
        "Before XPS 14 IR camera modules",
    ]))?;
    // Make it a Limine boot entry; without that the snapshot is no quick way back
    if sys::ok(&mut cmd("sh", &["-c", "command -v limine-snapper-sync"])) && run(&mut sudo(&["limine-snapper-sync"])).is_err()
    {
        bail!("limine-snapper-sync failed, so snapshot #{snap} isn't in the boot menu. Stopping.");
    }
    println!("Snapshot #{snap} (boot it from the Limine menu if the camera breaks)");

    // 2. Build and install both modules with DKMS
    let ir = sys::repo().join("ir");
    for (name, ver, dir) in MODULES {
        green(&format!("Building {name} {ver}..."));
        let dest = format!("/usr/src/{name}-{ver}");
        let m = format!("{name}/{ver}");
        run(&mut sudo(&["rm", "-rf", &dest]))?;
        run(&mut sudo(&["cp", "-r", &ir.join(dir).to_string_lossy(), &dest]))?;
        sys::ok(&mut sudo(&["dkms", "remove", &m, "--all"]));
        run(&mut sudo(&["dkms", "add", &m]))?;
        run(&mut sudo(&["dkms", "install", &m, "-k", KERNEL]))?;
    }
    run(&mut sudo(&["depmod", "-a"]))?;

    println!();
    println!("{}", module_file("ipu_bridge"));
    println!("{}", module_file("hm1092"));
    green("Done. Reboot, then: face-unlock ir-howdy");
    println!("Undo: face-unlock ir --undo   (or boot snapshot #{snap})");
    Ok(())
}

pub fn remove_modules() -> Result {
    sys::sudo_v()?;
    if howdy::uses_ir() {
        bail!("Howdy still uses the IR camera. First: face-unlock ir-howdy --undo");
    }
    for (name, ver, _) in MODULES {
        let m = format!("{name}/{ver}");
        let status = output(&mut cmd("dkms", &["status", &m])).unwrap_or_default();
        if !status.is_empty() {
            run(&mut sudo(&["dkms", "remove", &m, "--all"]))?;
        }
    }
    run(&mut sudo(&["depmod", "-a"]))?;
    // Only drop the sources once the override modules are really gone
    for module in ["ipu_bridge", "hm1092"] {
        let file = module_file(module);
        if file.contains("updates/dkms") {
            bail!("{module} is still installed from DKMS ({file}). Sources kept; check: dkms status");
        }
    }
    for (name, ver, _) in MODULES {
        run(&mut sudo(&["rm", "-rf", &format!("/usr/src/{name}-{ver}")]))?;
    }
    println!("Removed. Reboot to go back to the stock ipu-bridge (hm1092 can't be unloaded live).");
    Ok(())
}

// --- Howdy on the IR camera -------------------------------------------------

fn reload_ir_node_rules() -> Result {
    run(&mut sudo(&["udevadm", "control", "--reload"]))?;
    run(&mut sudo(&["udevadm", "trigger", "--subsystem-match=video4linux", &format!("--attr-match={IR_NODE_NAME}")]))
}

/// Download the fix pack's IR reader and hook into `dir`, check them, patch the reader.
fn fetch_howdy_files(dir: &str) -> Result {
    for (file, sha) in [("ir_reader.py", READER_SHA), ("ir-recorder-video_capture.patch", HOOK_SHA)] {
        let path = format!("{dir}/{file}");
        run(&mut cmd("curl", &["-fsSL", &format!("{FIXPACK}/{file}"), "-o", &path]))?;
        let sum = output(&mut cmd("sha256sum", &[&path]))?;
        if sum.split_whitespace().next() != Some(sha) {
            bail!("{file} doesn't match its pinned checksum. Nothing switched.");
        }
    }
    let patch = sys::repo().join("ir/howdy/ir_reader-fixes.patch");
    let patch = std::fs::File::open(&patch).map_err(|e| format!("{}: {e}", patch.display()))?;
    run(cmd("patch", &["-p1", "-s"]).current_dir(dir).stdin(patch))
}

/// Swap face models only after the new one is recognised; on any failure put
/// the old model and config back, so a failed enrol never leaves face unlock empty.
fn enroll() -> Result {
    let me = user();
    let models = format!("/etc/howdy/models/{me}.dat");
    let models_bak = format!("{models}.bak.{}", stamp());
    let cfg_bak = format!("{CFG}.bak.{}", stamp());
    sys::ok(&mut sudo(&["cp", "-a", &models, &models_bak]));
    let restore = |why: String| -> Result {
        println!("Restoring the previous face model and config.");
        if sys::exists(&models_bak) {
            let _ = run(&mut sudo(&["cp", "-a", &models_bak, &models]));
        }
        if sys::exists(&cfg_bak) {
            let _ = run(&mut sudo(&["cp", "-a", &cfg_bak, CFG]));
        }
        Err(why)
    };

    sys::catch_interrupts();
    sys::ok(&mut sudo(&["howdy", "-U", &me, "-y", "clear"]));
    green("Look at the camera to enroll your face.");
    if run(&mut sudo(&["howdy", "-U", &me, "add"])).is_err() || sys::interrupted() {
        return restore("Enrolling didn't finish.".into());
    }
    // Test as the user, not root: that's how the lock screen runs it
    green(&format!("Testing recognition as {me} (look at the camera)..."));
    let rc = sys::code(&mut cmd("python3", &[crate::howdy::COMPARE, &me]));
    if rc != 0 || sys::interrupted() {
        return restore(format!("Not recognised (exit {rc})."));
    }
    println!("Recognised.");
    Ok(())
}

pub fn enable_howdy() -> Result {
    sys::sudo_v()?;
    if !sys::exists("/sys/bus/i2c/devices/i2c-HIMX1092:00/driver") {
        bail!("IR sensor has no driver loaded. Run `face-unlock ir` and reboot first.");
    }

    // 1. Let the logged-in user open the IR capture node (Omarchy hides raw IPU7 nodes)
    let rule = sys::repo().join("ir/72-hm1092-ir-capture.rules");
    run(&mut sudo(&["cp", &rule.to_string_lossy(), RULE]))?;
    reload_ir_node_rules()?;
    run(&mut sudo(&["udevadm", "settle"]))?;

    // 2. Howdy IR recorder (raw 10-bit grey reader; the sensor is mono but tagged Bayer)
    let tmp = output(&mut cmd("mktemp", &["-d"]))?;
    let res = install_reader(&tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    res?;

    // 3. Config: IR frames are mostly dark background, so raise dark_threshold
    run(&mut sudo(&["cp", "-a", CFG, &format!("{CFG}.bak.{}", stamp())]))?;
    howdy::set("recording_plugin", "ir")?;
    howdy::set("device_path", "/dev/video16")?;
    howdy::set("dark_threshold", "90")?;
    // Search a 240-high image (the IR frame is 368): ~0.3 s faster per scan, still a
    // clear match (certainty 2.0-3.1 against the 3.5 limit) -- measured 2026-10-02
    howdy::set("max_height", "240")?;

    // 4. RGB face models don't match IR images: enrol again
    enroll()?;

    green("Done. Face unlock now uses the IR camera.");
    println!("Undo: face-unlock ir-howdy --undo");
    Ok(())
}

fn install_reader(tmp: &str) -> Result {
    fetch_howdy_files(tmp)?;
    run(&mut sudo(&["install", "-m", "0644", &format!("{tmp}/ir_reader.py"), &format!("{REC}/ir_reader.py")]))?;
    let video_capture = format!("{REC}/video_capture.py");
    if !sys::read(&video_capture).contains("ir_reader") {
        let bak = format!("{video_capture}.bak.{}", stamp());
        run(&mut sudo(&["cp", "-a", &video_capture, &bak]))?;
        let hook = std::fs::File::open(format!("{tmp}/ir-recorder-video_capture.patch")).map_err(|e| e.to_string())?;
        let patched = run(sudo(&["patch", "-p1", "--batch", "--forward", "--no-backup-if-mismatch", &video_capture]).stdin(hook));
        if patched.is_err() {
            run(&mut sudo(&["cp", "-a", &bak, &video_capture]))?;
            bail!("Howdy's video_capture.py changed upstream and the IR patch no longer applies. Nothing switched.");
        }
    }
    Ok(())
}

pub fn disable_howdy() -> Result {
    sys::sudo_v()?;
    let Some(rgb) = howdy::rgb_camera() else {
        bail!("RGB camera (Hardware ISP Camera) not found. Stopping.");
    };
    howdy::start_rgb_relay()?;
    run(&mut sudo(&["cp", "-a", CFG, &format!("{CFG}.bak.{}", stamp())]))?;
    howdy::set("recording_plugin", "opencv")?;
    howdy::set("device_path", &rgb)?;
    howdy::set("dark_threshold", "60")?;
    run(&mut sudo(&["rm", "-f", RULE]))?;
    reload_ir_node_rules()?;
    enroll()?;
    println!("Back on the RGB camera. (The ir_reader patch to Howdy is left in place; it's inert.)");
    Ok(())
}
