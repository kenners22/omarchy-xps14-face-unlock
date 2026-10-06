//! howdy-warmd: Howdy's face recognition, kept loaded.
//!
//! Stock Howdy starts a fresh Python process for every sudo / polkit / lock
//! screen scan and spends ~0.9 s loading OpenCV and dlib's models before the
//! camera turns on. This service loads the models once and answers requests
//! on a local socket; howdy-warm-auth (run by pam_exec) asks it.
//!
//! The scan is Howdy's compare.py loop, with the same config
//! (/etc/howdy/config.ini, re-read every request), face models
//! (/etc/howdy/models/<user>.dat), dlib models, certainty threshold,
//! dark-frame handling and timeout. Howdy's OpenCV steps are re-implemented
//! to give the same bytes (imgproc.rs), dlib is called from C++ (face.cpp),
//! and the camera is read like Howdy's IR reader (camera.rs).
//!
//! Protocol: the client sends "AUTH <user>\n" and gets one line back:
//!   OK | NO timeout | NO dark | NO nomodel | NO disabled | NO ssh | NO lid
//!   | NO policy | NO error | FALLBACK
//! FALLBACK means the config asks for something this service doesn't do (a
//! camera other than the IR reader); the client then runs stock Howdy.
//!
//! Policy: a non-root caller may only ask about its own user (SO_PEERCRED), so
//! a local process can't use this to check someone else's face. Callers inside
//! an SSH session are refused, and nothing scans with the lid shut -- the same
//! abort_if_ssh / abort_if_lid_closed rules pam_howdy applies. This goes
//! further than pam_howdy: while logind has any remote session open for the
//! user (or root), face unlock is off for that user everywhere, at the desk
//! too, and the password is asked instead. Like pam_howdy, it can't tell a
//! user service planted over an earlier SSH login from a local one.

mod camera;
mod config;
mod face;
mod imgproc;

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{env, fs, process, thread};

use camera::Camera;
use config::Config;
use face::{Models, DIM};
use imgproc::{Image, Rotate};

const DLIB_DATA: &str = "/usr/share/dlib-data";
const MODELS_DIR: &str = "/etc/howdy/models";

macro_rules! log {
    ($($arg:tt)*) => {{
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, $($arg)*);
        let _ = out.flush();
    }};
}

fn dlib_model(name: &str) -> String {
    format!("{DLIB_DATA}/{name}")
}

fn load_models(use_cnn: bool) -> Result<Models, String> {
    let cnn = dlib_model("mmod_human_face_detector.dat");
    Models::load(
        &dlib_model("shape_predictor_5_face_landmarks.dat"),
        &dlib_model("dlib_face_recognition_resnet_model_v1.dat"),
        use_cnn.then_some(cnn.as_str()),
    )
}

/// The user's enrolled encodings: every "data" list from /etc/howdy/models/<user>.dat.
fn user_encodings(user: &str) -> Vec<[f64; DIM]> {
    let Ok(text) = fs::read_to_string(format!("{MODELS_DIR}/{user}.dat")) else { return vec![] };
    let Ok(serde_json::Value::Array(models)) = serde_json::from_str(&text) else { return vec![] };
    let mut out = Vec::new();
    for model in &models {
        for enc in model["data"].as_array().into_iter().flatten() {
            let v: Vec<f64> = enc.as_array().into_iter().flatten().filter_map(|x| x.as_f64()).collect();
            if let Ok(a) = <[f64; DIM]>::try_from(v) {
                out.push(a);
            }
        }
    }
    out
}

/// Distance from a face to the nearest enrolled encoding (Howdy's np.linalg.norm + argmin).
fn nearest(encodings: &[[f64; DIM]], desc: &[f32; DIM]) -> f64 {
    encodings
        .iter()
        .map(|e| e.iter().zip(desc).map(|(a, &b)| (a - b as f64).powi(2)).sum::<f64>().sqrt())
        .fold(f64::INFINITY, f64::min)
}

// --- Checks pam_howdy makes before scanning ----------------------------------

fn lid_closed() -> bool {
    let Ok(dir) = fs::read_dir("/proc/acpi/button/lid") else { return false };
    dir.flatten()
        .any(|e| fs::read_to_string(e.path().join("state")).is_ok_and(|s| s.contains("closed")))
}

/// Walk the caller's ancestors looking for an SSH session's environment.
/// Fails closed: if the walk can't finish (the caller already exited, or an
/// ancestor vanished mid-walk), it counts as SSH.
fn in_ssh_session(mut pid: i32) -> bool {
    for _ in 0..64 {
        // pid 0: the caller is in another pid namespace (a container), so its
        // ancestry can't be checked. 1: the walk reached init.
        if pid <= 0 {
            return true;
        }
        if pid == 1 {
            return false;
        }
        let Ok(environ) = fs::read(format!("/proc/{pid}/environ")) else { return true };
        if environ.split(|&b| b == 0).any(|e| {
            e.starts_with(b"SSH_CONNECTION=") || e.starts_with(b"SSH_CLIENT=") || e.starts_with(b"SSH_TTY=")
        }) {
            return true;
        }
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else { return true };
        match stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(1)?.parse().ok()) {
            Some(ppid) => pid = ppid,
            None => return true,
        }
    }
    true
}

/// Does logind have a remote (SSH) session open for `uid`? This holds however
/// the request was started -- `systemd-run --user`, `at`, a D-Bus service --
/// where the environment walk above can't see an SSH ancestor.
fn remote_session(uid: u32) -> bool {
    let Ok(dir) = fs::read_dir("/run/systemd/sessions") else { return false };
    dir.flatten().filter(|e| !e.file_name().to_string_lossy().contains('.')).any(|e| {
        let s = fs::read_to_string(e.path()).unwrap_or_default();
        let has = |line: &str| s.lines().any(|l| l == line);
        has(&format!("UID={uid}")) && has("REMOTE=1") && !has("STATE=closing")
    })
}

// --- The scan: compare.py's main loop ----------------------------------------

enum Prepared {
    /// Empty histogram or entirely dark: Howdy skips it without counting it.
    Blank,
    /// Darker than dark_threshold.
    Dark,
    /// gs: CLAHE'd, for the detector. frame: plain, for the descriptor.
    Ready { gs: Image, frame: Image },
}

/// Howdy's per-frame preparation: CLAHE, the darkness check, then the same
/// resize and rotation of both the CLAHE'd and the plain frame.
fn prepare(raw: Image, scale: f64, rot: Rotate, dark_threshold: f32) -> Prepared {
    let gs = imgproc::clahe(&raw, 2.0, 8, 8);
    let darkness = imgproc::darkness(&gs);
    if darkness == 100.0 {
        return Prepared::Blank;
    }
    if darkness > dark_threshold {
        return Prepared::Dark;
    }
    let (mut gs, mut frame) = (gs, raw);
    if scale != 1.0 {
        // scan() has checked resize_area handles this factor
        frame = imgproc::resize_area(&frame, scale).unwrap_or(frame);
        gs = imgproc::resize_area(&gs, scale).unwrap_or(gs);
    }
    if rot != Rotate::None {
        frame = imgproc::rotate(&frame, rot);
        gs = imgproc::rotate(&gs, rot);
    }
    Prepared::Ready { gs, frame }
}

struct Warm {
    models: Models,
    use_cnn: bool,
}

/// Runs one scan. The camera is left in `camera` rather than closed: stopping
/// the IPU7 stream takes ~0.2 s, and the caller answers before paying it.
fn scan(warm: &mut Warm, camera: &mut Option<Camera>, user: &str, config: &Config) -> &'static str {
    let encodings = user_encodings(user);
    if encodings.is_empty() {
        return "NO nomodel";
    }

    let timeout = config.int("video", "timeout", 4) as f64;
    let dark_threshold = config.float("video", "dark_threshold", 60.0);
    let video_certainty = config.float("video", "certainty", 3.5) / 10.0;
    let max_height = config.float("video", "max_height", 320.0);
    let rotate = config.int("video", "rotate", 0);
    let device = config.get("video", "device_path").unwrap_or("").to_string();

    // use_cnn is read once at start-up in Howdy too; follow a change without a restart
    let use_cnn = config.bool("core", "use_cnn", false);
    if use_cnn != warm.use_cnn {
        match load_models(use_cnn) {
            Ok(m) => *warm = Warm { models: m, use_cnn },
            Err(e) => {
                log!("{user}: reloading models (cnn={use_cnn}) failed: {e}");
                return "NO error";
            }
        }
    }

    let cam = match Camera::open(&device) {
        Ok(c) => camera.insert(c),
        Err(e) => {
            log!("{user}: camera {device}: {e}");
            return "NO error";
        }
    };
    // Howdy's VideoCapture throws away one frame on open "to wake the camera".
    // Streaming starts on the first read anyway, so that frame is used here:
    // one frame period (~33 ms) sooner when it already shows the face.
    let mut gray = Vec::new();

    let height = if rotate == 2 { cam.width } else { cam.height } as f64;
    // Howdy: (max_height / height) or 1
    let scaling_factor = if max_height / height == 0.0 { 1.0 } else { max_height / height };
    if scaling_factor != 1.0 && !imgproc::area_supported(scaling_factor) {
        // Enlarging or a whole-number shrink: OpenCV does those another way
        log!("{user}: max_height {max_height} needs a resize this service doesn't do; handing over to Howdy");
        return "FALLBACK";
    }

    let (mut frames, mut valid_frames, mut dark_tries) = (0u32, 0u32, 0u32);
    let mut lowest = 10.0f64;
    let (mut level_sum, mut read_frames) = (0u64, 0u64);
    let start = Instant::now();
    loop {
        frames += 1;
        if start.elapsed().as_secs_f64() > timeout {
            // mean grey level and the LED's state: to tell an unlit scene from a covered camera
            let mean = level_sum as f64 / read_frames.max(1) as f64 / (cam.width * cam.height) as f64;
            log!(
                "{user}: no match in {timeout}s (frames {frames}, dark {dark_tries}, best {:.2}, mean level {mean:.1}, led {})",
                lowest * 10.0,
                camera::ir_led_state()
            );
            return if dark_tries == valid_frames { "NO dark" } else { "NO timeout" };
        }

        match cam.read(&mut gray) {
            Some(sum) => (level_sum, read_frames) = (level_sum + sum, read_frames + 1),
            None => {
                log!("{user}: camera read failed");
                return "NO error";
            }
        }

        let rot = match (rotate, frames % 3, frames % 2) {
            (1, 1, _) => Rotate::CounterClockwise,
            (1, 2, _) => Rotate::Clockwise,
            (2, _, 0) => Rotate::CounterClockwise,
            (2, _, _) => Rotate::Clockwise,
            _ => Rotate::None,
        };
        let raw = Image::new(std::mem::take(&mut gray), cam.width, cam.height);
        let (gs, frame) = match prepare(raw, scaling_factor, rot, dark_threshold as f32) {
            Prepared::Blank => continue,
            Prepared::Dark => {
                valid_frames += 1;
                dark_tries += 1;
                continue;
            }
            Prepared::Ready { gs, frame } => {
                valid_frames += 1;
                (gs, frame)
            }
        };
        let faces = match warm.models.faces(&gs, &frame) {
            Ok(f) => f,
            Err(e) => {
                log!("{user}: scan error: {e}");
                return "NO error";
            }
        };
        for desc in &faces {
            let m = nearest(&encodings, desc);
            lowest = lowest.min(m);
            if 0.0 < m && m < video_certainty {
                log!(
                    "{user}: Login approved (certainty {:.2}, {frames} frames, {:.2}s)",
                    m * 10.0,
                    start.elapsed().as_secs_f64()
                );
                return "OK";
            }
        }
    }
}

// --- The socket ---------------------------------------------------------------

fn peer(conn: &UnixStream) -> Option<(i32, u32)> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            conn.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    (rc == 0).then_some((cred.pid, cred.uid))
}

fn user_uid(name: &str) -> Option<u32> {
    let cname = std::ffi::CString::new(name).ok()?;
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut res = std::ptr::null_mut();
    let rc = unsafe { libc::getpwnam_r(cname.as_ptr(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut res) };
    (rc == 0 && !res.is_null()).then_some(pw.pw_uid)
}

fn user_name(uid: u32) -> Option<String> {
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut res = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut res) };
    if rc != 0 || res.is_null() {
        return None;
    }
    Some(unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned())
}

/// A user name as PAM and the client accept it; it becomes part of a path.
fn valid_user(u: &str) -> bool {
    !u.is_empty() && u.len() <= 32 && u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The "scanning" file the bar widget watches, there for as long as this lives.
struct ScanningFlag<'a>(&'a Path);

impl<'a> ScanningFlag<'a> {
    fn raise(path: &'a Path) -> Self {
        let _ = fs::File::create(path);
        ScanningFlag(path)
    }
}

impl Drop for ScanningFlag<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0);
    }
}

fn handle(mut conn: UnixStream, warm: &Mutex<Warm>, scanning_flag: &Path) {
    let Some((pid, uid)) = peer(&conn) else { return };
    let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    if BufReader::new((&conn).take(256)).read_line(&mut line).is_err() {
        return;
    }
    let Some(user) = line.trim().strip_prefix("AUTH ").filter(|u| valid_user(u)).map(str::to_string) else { return };

    let mut reply = |r: &str| {
        let _ = conn.write_all(format!("{r}\n").as_bytes());
    };
    if uid != 0 && user_name(uid).as_deref() != Some(user.as_str()) {
        log!("refused: uid {uid} asked about {user:?}");
        return reply("NO policy");
    }

    let config = Config::load();
    if config.bool("core", "disabled", false) {
        return reply("NO disabled");
    }
    let remote = || [Some(uid), user_uid(&user)].into_iter().flatten().any(remote_session);
    if config.bool("core", "abort_if_ssh", true) && (in_ssh_session(pid) || remote()) {
        log!("{user}: refused, SSH session");
        return reply("NO ssh");
    }
    if config.bool("core", "abort_if_lid_closed", true) && lid_closed() {
        return reply("NO lid");
    }
    if config.get("video", "recording_plugin") != Some("ir") {
        return reply("FALLBACK");
    }

    // One camera: requests take turns
    let mut warm = warm.lock().unwrap_or_else(|e| e.into_inner());
    let _flag = ScanningFlag::raise(scanning_flag);
    let mut camera = None;
    let result = scan(&mut warm, &mut camera, &user, &config);
    if result == "FALLBACK" {
        // stock Howdy is about to open the camera itself
        drop(camera.take());
    }
    reply(result);
    // Now close the camera (~0.2 s), still holding the lock so the next scan waits for it
    drop(camera);
}

// --- Start-up -----------------------------------------------------------------

extern "C" fn on_term(sig: libc::c_int) {
    // Async-signal-safe: open/write/close only. A scan killed mid-stream must
    // not leave the IR emitter on.
    unsafe {
        let fd = libc::open(c"/sys/class/leds/HIMX1092_00::ir_flood_led/brightness".as_ptr(), libc::O_WRONLY);
        if fd >= 0 {
            libc::write(fd, b"0".as_ptr() as *const libc::c_void, 1);
            libc::close(fd);
        }
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

/// `howdy-warmd --frames USER W H FILE...`: run the scan's image steps on raw
/// 8-bit grey frames and print what each gives. For checking against Howdy.
fn check_frames(args: &[String]) -> ! {
    let [user, w, h, files @ ..] = args else {
        eprintln!("usage: howdy-warmd --frames USER WIDTH HEIGHT FILE...");
        process::exit(2)
    };
    let (w, h): (usize, usize) = (w.parse().unwrap(), h.parse().unwrap());
    let config = Config::load();
    let mut models = load_models(config.bool("core", "use_cnn", false)).unwrap_or_else(|e| {
        eprintln!("{e}");
        process::exit(1)
    });
    let encodings = user_encodings(user);
    let scale = config.float("video", "max_height", 320.0) / h as f64;
    for f in files {
        let data = fs::read(f).expect("frame");
        let t = Instant::now();
        let raw = Image::new(data, w, h);
        let dark = imgproc::darkness(&imgproc::clahe(&raw, 2.0, 8, 8));
        let res = match prepare(raw, scale, Rotate::None, 100.0) {
            Prepared::Ready { gs, frame } => Some(models.faces(&gs, &frame).unwrap()),
            _ => None,
        };
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        match res {
            Some(faces) => {
                let d: Vec<String> = faces.iter().map(|x| format!("{:.4}", nearest(&encodings, x) * 10.0)).collect();
                println!("{f} dark {dark:.4} faces [{}] {ms:.0}ms", d.join(", "));
            }
            None => println!("{f} dark {dark:.4} (dark/blank) {ms:.0}ms"),
        }
    }
    process::exit(0)
}

/// `howdy-warmd --camera-test`: open the camera the way a scan does, read a
/// few frames and report timings, brightness and the LED. No face matching.
fn camera_test() -> ! {
    let config = Config::load();
    let device = config.get("video", "device_path").unwrap_or("").to_string();
    let t = Instant::now();
    let mut cam = Camera::open(&device).unwrap_or_else(|e| {
        eprintln!("camera {device}: {e}");
        process::exit(1)
    });
    println!("opened {}x{} in {:.0} ms", cam.width, cam.height, t.elapsed().as_secs_f64() * 1000.0);
    let mut gray = Vec::new();
    for i in 0..5 {
        match cam.read(&mut gray) {
            Some(sum) => println!(
                "frame {i}: {:.0} ms, mean level {:.1}, led {}",
                t.elapsed().as_secs_f64() * 1000.0,
                sum as f64 / (cam.width * cam.height) as f64,
                camera::ir_led_state()
            ),
            None => {
                println!("frame {i}: read failed");
                process::exit(1)
            }
        }
    }
    process::exit(0)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--frames") => check_frames(&args[2..]),
        Some("--camera-test") => camera_test(),
        _ => {}
    }

    // The override is for testing the service as a normal user; the client
    // never takes one (a user-chosen socket could answer "OK" for sudo).
    let socket_path = env::var("HOWDY_WARM_SOCKET").unwrap_or_else(|_| "/run/howdy-warm/socket".into());
    // read by the bar widget
    let scanning_flag = Path::new(&socket_path).with_file_name("scanning");

    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        unsafe { libc::signal(sig, on_term as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    }

    // --- Models: loaded once, the whole point of this service
    let start = Instant::now();
    let use_cnn = Config::load().bool("core", "use_cnn", false);
    let mut models = load_models(use_cnn).unwrap_or_else(|e| {
        log!("loading models failed: {e}");
        process::exit(1)
    });
    // Run the detector once so its first real use doesn't pay for warm-up
    let blank = Image::new(vec![128u8; 320 * 240], 320, 240);
    let _ = models.faces(&blank, &blank);
    log!("models loaded in {:.2}s (cnn={})", start.elapsed().as_secs_f64(), if use_cnn { "True" } else { "False" });

    let _ = fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).unwrap_or_else(|e| {
        log!("can't listen on {socket_path}: {e}");
        process::exit(1)
    });
    // Anyone may connect; handle() decides who may ask about whom
    let _ = fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o666));
    log!("listening on {socket_path}");

    let warm = Arc::new(Mutex::new(Warm { models, use_cnn }));
    let scanning_flag = Arc::new(scanning_flag);
    let slots = Arc::new(Slots::default());
    for conn in listener.incoming() {
        let conn = match conn {
            Ok(c) => c,
            // e.g. out of file descriptors: don't spin
            Err(_) => {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        // Any local user can connect, so cap what they can queue: over the cap
        // the connection is just closed, and that request falls to the password.
        let Some((_, uid)) = peer(&conn) else { continue };
        let Some(slot) = Slots::take(&slots, uid) else { continue };
        let (warm, flag) = (warm.clone(), scanning_flag.clone());
        thread::spawn(move || {
            let _slot = slot;
            handle(conn, &warm, &flag)
        });
    }
}

/// Open requests, in all and per caller uid.
#[derive(Default)]
struct Slots {
    per_uid: Mutex<std::collections::HashMap<u32, usize>>,
}

const MAX_REQUESTS: usize = 16;
const MAX_REQUESTS_PER_UID: usize = 2;
/// Root is PAM itself (sudo, polkit), so it may have a few more in flight.
const MAX_REQUESTS_ROOT: usize = 4;

/// One open request; frees its place when dropped.
struct Slot {
    slots: Arc<Slots>,
    uid: u32,
}

impl Slots {
    fn take(slots: &Arc<Slots>, uid: u32) -> Option<Slot> {
        let mut per_uid = slots.per_uid.lock().unwrap_or_else(|e| e.into_inner());
        let total: usize = per_uid.values().sum();
        let limit = if uid == 0 { MAX_REQUESTS_ROOT } else { MAX_REQUESTS_PER_UID };
        if total >= MAX_REQUESTS || per_uid.get(&uid).copied().unwrap_or(0) >= limit {
            return None;
        }
        *per_uid.entry(uid).or_insert(0) += 1;
        Some(Slot { slots: slots.clone(), uid })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut per_uid = self.slots.per_uid.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = per_uid.get_mut(&self.uid) {
            *n -= 1;
            if *n == 0 {
                per_uid.remove(&self.uid);
            }
        }
    }
}
