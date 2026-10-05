//! howdy-warm-auth: run by pam_exec in place of pam_howdy.
//!
//! Asks howdy-warmd (Howdy with its models kept loaded) to look for PAM_USER's
//! face and exits 0 only on a match. If the service isn't running, it hands
//! over to stock Howdy's compare.py, so face unlock keeps working (just slower).
//! Anything else -- timeout, refusal, a garbled reply -- is a failure, and PAM
//! moves on to the password.

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{exit, Command};
use std::time::Duration;

const SOCKET: &str = "/run/howdy-warm/socket";
const COMPARE: &str = "/usr/lib/howdy/compare.py";
// Howdy's own timeout is a few seconds; this only guards against a hung service.
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

fn valid_user(u: &str) -> bool {
    !u.is_empty()
        && u.len() <= 32
        && u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn main() {
    // pam_exec exports PAM_USER; an argument is accepted for testing by hand.
    let user = env::var("PAM_USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| env::args().nth(1))
        .unwrap_or_default();
    if !valid_user(&user) {
        exit(2);
    }

    let mut stream = match UnixStream::connect(SOCKET) {
        Ok(s) => s,
        Err(_) => {
            // Service down: same result as before it existed. exec only returns on error.
            let _ = Command::new("/usr/bin/python3").arg(COMPARE).arg(&user).exec();
            exit(1);
        }
    };
    let _ = stream.set_read_timeout(Some(REPLY_TIMEOUT));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    if writeln!(stream, "AUTH {user}").is_err() {
        exit(1);
    }
    let mut reply = String::new();
    if BufReader::new(&stream).read_line(&mut reply).is_err() {
        exit(1);
    }

    // Shown by sudo when pam_exec runs with `stdout`; ignored elsewhere.
    match reply.trim() {
        "OK" => {
            println!("Identified face as {user}");
            exit(0);
        }
        r => {
            if let Some(why) = r.strip_prefix("NO ") {
                if why == "timeout" {
                    println!("Failure, timeout reached");
                }
            }
            exit(1);
        }
    }
}
