#!/bin/bash
# Fast face unlock: Howdy kept loaded in a background service (howdy-warmd) and a
# small Rust client (howdy-warm-auth) that sudo, polkit and the lock screen run
# through pam_exec instead of pam_howdy. Cuts ~0.5 s off every scan.
#   bash install-warm.sh          # build, install, start, switch PAM
#   bash install-warm.sh --undo   # PAM back to pam_howdy, service removed
#
# If the service is down, the client hands over to stock Howdy (compare.py), so
# face unlock keeps working; the password is the fallback everywhere.

set -e

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
LIB=/usr/local/lib/howdy-warm
CLIENT=$LIB/howdy-warm-auth
UNIT=/etc/systemd/system/howdy-warm.service
STAMP=$(date +%s)
# pam_howdy line -> pam_exec line, per stack (stdout shows sudo's "Identified face as ...")
SUFFICIENT_OLD="auth       sufficient                  pam_howdy.so"
SUFFICIENT_NEW="auth       sufficient                  pam_exec.so quiet stdout $CLIENT"
REQUIRED_OLD="auth       required                    pam_howdy.so"
REQUIRED_NEW="auth       required                    pam_exec.so quiet $CLIENT"
STACKS=(/etc/pam.d/sudo /etc/pam.d/polkit-1 /etc/pam.d/omarchy-lock-face)

swap() { # file from to
  grep -qF "$2" "$1" || return 0
  sudo cp -a "$1" "$1.bak.$STAMP"
  sudo sed -i "s|^$(printf '%s' "$2" | sed 's/[.[\*^$|]/\\&/g')\$|$3|" "$1"
  grep -qF "$3" "$1" && echo "  $1"
}

sudo -v

if [[ $1 == --undo ]]; then
  echo -e "\e[32mPutting pam_howdy back...\e[0m"
  for f in "${STACKS[@]}"; do
    [[ -f $f ]] || continue
    swap "$f" "$SUFFICIENT_NEW" "$SUFFICIENT_OLD"
    swap "$f" "$REQUIRED_NEW" "$REQUIRED_OLD"
  done
  sudo systemctl disable --now howdy-warm.service 2>/dev/null || true
  sudo rm -f "$UNIT"; sudo systemctl daemon-reload
  sudo rm -rf "$LIB"
  echo "Done: stock Howdy again."
  exit 0
fi

# 1. Build the client (as you, not root)
echo -e "\e[32mBuilding the Rust client...\e[0m"
CARGO=$(command -v cargo || mise which cargo 2>/dev/null || echo ~/.cargo/bin/cargo)
(cd "$HERE/client" && "$CARGO" build --release --quiet)

# 2. Install service + client, start it
echo -e "\e[32mInstalling and starting howdy-warm...\e[0m"
sudo install -D -m 0755 "$HERE/client/target/release/howdy-warm-auth" "$CLIENT"
sudo install -D -m 0755 "$HERE/howdy-warmd" "$LIB/howdy-warmd"
sudo install -m 0644 "$HERE/howdy-warm.service" "$UNIT"
sudo systemctl daemon-reload
sudo systemctl enable --now howdy-warm.service
sudo systemctl restart howdy-warm.service
for _ in $(seq 1 40); do [[ -S /run/howdy-warm/socket ]] && break; sleep 0.25; done
[[ -S /run/howdy-warm/socket ]] || { echo "Service didn't start: journalctl -u howdy-warm"; exit 1; }

# 3. Prove it works before touching PAM
echo -e "\e[32mTesting (look at the camera)...\e[0m"
"$CLIENT" "$USER" || { echo "Not recognised. PAM left unchanged."; exit 1; }

# 4. Switch PAM
echo -e "\e[32mSwitching PAM to the warm client:\e[0m"
for f in "${STACKS[@]}"; do
  [[ -f $f ]] || continue
  swap "$f" "$SUFFICIENT_OLD" "$SUFFICIENT_NEW"
  swap "$f" "$REQUIRED_OLD" "$REQUIRED_NEW"
done

echo
echo -e "\e[32mDone. Try: sudo -k; sudo true\e[0m"
echo "Backups: /etc/pam.d/*.bak.$STAMP   Undo: bash $0 --undo"
