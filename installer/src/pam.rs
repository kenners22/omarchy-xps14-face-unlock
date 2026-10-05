//! The PAM lines face unlock adds, and the stacks it adds them to.

pub const SUDO: &str = "/etc/pam.d/sudo";
pub const POLKIT: &str = "/etc/pam.d/polkit-1";
pub const LOCK_PASSWORD: &str = "/etc/pam.d/omarchy-lock-password";
pub const LOCK_FACE: &str = "/etc/pam.d/omarchy-lock-face";

pub const WARM_CLIENT: &str = "/usr/local/lib/howdy-warm/howdy-warm-auth";
pub const HOWDY_SUFFICIENT: &str = "auth       sufficient                  pam_howdy.so";
pub const HOWDY_REQUIRED: &str = "auth       required                    pam_howdy.so";
// stdout shows sudo's "Identified face as ..."
pub const WARM_SUFFICIENT: &str =
    "auth       sufficient                  pam_exec.so quiet stdout /usr/local/lib/howdy-warm/howdy-warm-auth";
pub const WARM_REQUIRED: &str = "auth       required                    pam_exec.so quiet /usr/local/lib/howdy-warm/howdy-warm-auth";

/// The stacks the warm client replaces pam_howdy in.
pub const WARM_STACKS: [&str; 3] = [SUDO, POLKIT, LOCK_FACE];

/// Does any stack go through the warm client?
pub fn warm_in_use() -> bool {
    WARM_STACKS.iter().any(|f| crate::sys::read(f).contains(WARM_CLIENT))
}
