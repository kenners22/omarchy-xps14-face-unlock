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

/// Goes right above the face line on sudo and polkit. Their face line comes
/// before `include system-auth`, so on its own a face match would skip
/// system-auth's faillock check: after 10 wrong passwords, sudo would still
/// open by face. This stops it, as the lock screen's stack already does.
pub const FAILLOCK_GUARD: &str = "auth       requisite                   pam_faillock.so preauth silent deny=10 unlock_time=120";

/// Add FAILLOCK_GUARD above every face line that doesn't have it yet.
pub fn guard_faillock(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines() {
        let face = line == HOWDY_SUFFICIENT || line == WARM_SUFFICIENT;
        if face && out.last() != Some(&FAILLOCK_GUARD) {
            out.push(FAILLOCK_GUARD);
        }
        out.push(line);
    }
    out.iter().map(|l| format!("{l}\n")).collect()
}

/// The stacks the warm client replaces pam_howdy in.
pub const WARM_STACKS: [&str; 3] = [SUDO, POLKIT, LOCK_FACE];

/// Does any stack go through the warm client?
pub fn warm_in_use() -> bool {
    WARM_STACKS.iter().any(|f| crate::sys::read(f).contains(WARM_CLIENT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_goes_above_the_face_line_once() {
        let stack = format!("#%PAM-1.0\n{WARM_SUFFICIENT}\nauth include system-auth\n");
        let guarded = guard_faillock(&stack);
        assert_eq!(guarded, format!("#%PAM-1.0\n{FAILLOCK_GUARD}\n{WARM_SUFFICIENT}\nauth include system-auth\n"));
        assert_eq!(guard_faillock(&guarded), guarded);
        assert_eq!(guard_faillock("#%PAM-1.0\nauth include system-auth\n"), "#%PAM-1.0\nauth include system-auth\n");
    }
}
