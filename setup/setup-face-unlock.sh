#!/bin/bash
# Face unlock (Howdy) for sudo, polkit (incl. 1Password) and the Omarchy lock screen.
#   bash setup-face-unlock.sh          # install + enroll + enable
#   bash setup-face-unlock.sh --undo   # remove pam_howdy from all three stacks

set -e

# Camera per machine:
#   FACE_UNLOCK_CAMERA=/dev/v4l/by-path/...  a camera of your own, e.g. a USB IR
#               camera (greyscale, works in the dark)
#   XPS 14      no Linux driver for its IR sensor (Himax HM1092) yet, so use the
#               RGB camera through the Intel IPU7 relay (v4l2loopback, "Hardware
#               ISP Camera"). Needs light on your face, and a good photo could fool it.
USB_IR_CAMERA="${FACE_UNLOCK_CAMERA:-}"
HOWDY_LINE="auth       sufficient                  pam_howdy.so"
STAMP=$(date +%s)

backup() { [[ -f $1 ]] && sudo cp -a "$1" "$1.bak.$STAMP"; }
set_cfg() { sudo sed -i "s|^$1 *=.*|$1 = $2|" /etc/howdy/config.ini; grep "^$1 " /etc/howdy/config.ini; }

ipu7_camera() {
  local d
  for d in /sys/class/video4linux/video*; do
    [[ $(cat "$d/name" 2>/dev/null) == "Hardware ISP Camera" ]] && echo "/dev/${d##*/}" && return
  done
}

if [[ $1 == --undo ]]; then
  for f in /etc/pam.d/sudo /etc/pam.d/polkit-1 /etc/pam.d/omarchy-lock-password; do
    [[ -f $f ]] && grep -q pam_howdy.so "$f" && backup "$f" && sudo sed -i '/pam_howdy\.so/d' "$f" && echo "Removed from $f"
  done
  # Lock-screen auto-scan stack (enable-lock-autoscan.sh)
  [[ -f /etc/pam.d/omarchy-lock-face ]] && sudo rm /etc/pam.d/omarchy-lock-face && echo "Removed /etc/pam.d/omarchy-lock-face"
  if grep -l pam_howdy.so /etc/pam.d/* 2>/dev/null | grep -v '\.bak\.'; then
    echo "pam_howdy is still in the files above. Remove it by hand."; exit 1
  fi
  echo "Face unlock disabled (howdy-git is still installed; remove with: omarchy pkg remove howdy-git)"
  exit 0
fi

sudo -v

# 1. Install (python-dlib compiles from source — this can take 10-20 minutes)
if ! pacman -Q howdy-git &>/dev/null; then
  echo -e "\e[32mInstalling Howdy...\e[0m"
  omarchy pkg aur add howdy-git
fi

# 2. Point Howdy at this machine's camera
echo -e "\e[32mConfiguring camera...\e[0m"
if grep -q '^recording_plugin *= *ir' /etc/howdy/config.ini; then
  # XPS 14 IR camera, set up by ir/enable-ir-howdy.sh: leave it alone
  grep -E '^(recording_plugin|device_path) ' /etc/howdy/config.ini
elif [[ -n $USB_IR_CAMERA && -e $USB_IR_CAMERA ]]; then
  set_cfg device_path "$USB_IR_CAMERA"
elif camera=$(ipu7_camera) && [[ -n $camera ]]; then
  # The IPU7 relay sends black frames for the first ~1.5 s after opening;
  # Howdy skips them as too dark, so give it longer before giving up.
  systemctl is-active -q v4l2-relayd@ipu7 || sudo systemctl start v4l2-relayd@ipu7
  set_cfg device_path "$camera"
  set_cfg timeout 6
  echo -e "\e[33mRGB camera only (no IR driver on this laptop): needs light on your face.\e[0m"
else
  echo "No supported camera found. Stopping before touching PAM."; exit 1
fi

# 3. Enroll your face
if ! sudo howdy -U "$USER" list &>/dev/null; then
  echo -e "\e[32mLook at the camera to enroll your face.\e[0m"
  sudo howdy -U "$USER" add
fi

# 4. Verify before touching PAM — abort if it can't recognise you
# (Not `howdy test`: that opens a preview window, which root can't on Wayland.)
# compare.py is what pam_howdy runs; exit 0 = recognised.
echo -e "\e[32mTesting recognition (look at the camera)...\e[0m"
rc=0; sudo python3 /usr/lib/howdy/compare.py "$USER" || rc=$?
if (( rc == 0 )); then
  echo "Recognised."
else
  case $rc in
    10) why="no face model enrolled" ;;
    11) why="timed out without recognising you" ;;
    13) why="too dark" ;;
    *)  why="exit code $rc" ;;
  esac
  echo "Not recognised ($why). Stopping before enabling PAM. Try more light, or: sudo howdy add"
  exit 1
fi

# 5. Enable in PAM (password stays as the fallback everywhere)
echo -e "\e[32mEnabling face unlock...\e[0m"

# sudo
if ! grep -q pam_howdy.so /etc/pam.d/sudo; then
  backup /etc/pam.d/sudo
  sudo sed -i "1a $HOWDY_LINE" /etc/pam.d/sudo
fi

# polkit (system dialogs + 1Password "unlock using system authentication")
if [[ ! -f /etc/pam.d/polkit-1 ]]; then
  sudo cp /usr/lib/pam.d/polkit-1 /etc/pam.d/polkit-1
fi
if ! grep -q pam_howdy.so /etc/pam.d/polkit-1; then
  backup /etc/pam.d/polkit-1
  sudo sed -i "1a $HOWDY_LINE" /etc/pam.d/polkit-1
fi

# lock screen — after the faillock preauth line. Skipped when auto-scan is on:
# then omarchy-lock-face does the scanning and Enter must stay password-only,
# or both stacks fight over the camera.
if [[ ! -f /etc/pam.d/omarchy-lock-face ]] && ! grep -q pam_howdy.so /etc/pam.d/omarchy-lock-password; then
  backup /etc/pam.d/omarchy-lock-password
  sudo sed -i "/pam_faillock.so preauth/a $HOWDY_LINE" /etc/pam.d/omarchy-lock-password
fi

echo
echo -e "\e[32mDone. Try it: open a new terminal and run  sudo -k; sudo true\e[0m"
echo "Backups: /etc/pam.d/*.bak.$STAMP   Undo: bash $0 --undo"
