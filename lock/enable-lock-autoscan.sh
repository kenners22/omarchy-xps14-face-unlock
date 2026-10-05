#!/bin/bash
# Lock screen scans your face automatically (no key press needed).
#   bash enable-lock-autoscan.sh          # enable
#   bash enable-lock-autoscan.sh --undo   # back to "any key + Enter"
#
# Works with the patched lock screen (face-lock.patch on a clone of omarchy.lock), which starts
# a face scan as soon as /etc/pam.d/omarchy-lock-face exists.

set -e

FACE_PAM=/etc/pam.d/omarchy-lock-face
PASSWORD_PAM=/etc/pam.d/omarchy-lock-password
HOWDY_LINE="auth       sufficient                  pam_howdy.so"
STAMP=$(date +%s)

if [[ $1 == --undo ]]; then
  sudo rm -f "$FACE_PAM"
  if ! grep -q pam_howdy.so "$PASSWORD_PAM"; then
    sudo cp -a "$PASSWORD_PAM" "$PASSWORD_PAM.bak.$STAMP"
    sudo sed -i "/pam_faillock.so preauth/a $HOWDY_LINE" "$PASSWORD_PAM"
  fi
  echo "Auto-scan off. Lock screen is back to: type any key + Enter, then look at the camera."
  exit 0
fi

# Face-only stack for the lock screen. faillock preauth first, so a password
# lockout (10 wrong attempts) blocks face unlock too.
sudo tee "$FACE_PAM" >/dev/null <<'EOF'
#%PAM-1.0
# Face unlock for the Omarchy lock screen (Howdy). See github.com/kenners22/omarchy-xps14-face-unlock
auth       required                    pam_faillock.so preauth silent deny=10 unlock_time=120
auth       required                    pam_howdy.so
account    include                     system-local-login
EOF

# The Enter key now checks the password only, so it never fights the
# automatic scan for the camera.
if grep -q pam_howdy.so "$PASSWORD_PAM"; then
  sudo cp -a "$PASSWORD_PAM" "$PASSWORD_PAM.bak.$STAMP"
  sudo sed -i '/pam_howdy\.so/d' "$PASSWORD_PAM"
fi

echo -e "\e[32mDone. Lock the screen (Super+Ctrl+L) and just look at the camera.\e[0m"
echo "Undo: bash $0 --undo"
