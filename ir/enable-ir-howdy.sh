#!/bin/bash
# Switch Howdy from the RGB camera to the IR camera (works in the dark).
# Needs the modules from install-ir.sh loaded (hm1092 bound to HIMX1092).
#   bash enable-ir-howdy.sh          # IR reader + config + re-enroll
#   bash enable-ir-howdy.sh --undo   # back to the RGB camera (re-enrolls)

set -e

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REC=/usr/lib/howdy/recorders
CFG=/etc/howdy/config.ini
RULE=/etc/udev/rules.d/72-hm1092-ir-capture.rules
STAMP=$(date +%s)
# Howdy's IR reader and its hook come from the fix pack (no licence, so they are
# fetched rather than copied here), pinned and checksum-checked, then patched with
# ir_reader-fixes.patch (frame timeout, absolute media-ctl, ignore size changes).
FIXPACK=https://raw.githubusercontent.com/HritwikSinghal/svp7500-camera-fix-pack/25cc4c9e282912a404cfb188283964ed983e87ed/howdy
READER_SHA=78b7fd214e12c50769116295b2e8d10c09dbab0c94ac6499228ee3d4b64ab0f0
HOOK_SHA=b226d56805b545c2b83992cc19281679b4b317d5b22824049a83dae13ffcc335

fetch_howdy_files() { # into $1
  curl -fsSL "$FIXPACK/ir_reader.py" -o "$1/ir_reader.py"
  curl -fsSL "$FIXPACK/ir-recorder-video_capture.patch" -o "$1/ir-recorder-video_capture.patch"
  echo "$READER_SHA  $1/ir_reader.py" | sha256sum -c --quiet
  echo "$HOOK_SHA  $1/ir-recorder-video_capture.patch" | sha256sum -c --quiet
  (cd "$1" && patch -p1 -s < "$HERE/howdy/ir_reader-fixes.patch")
}

set_cfg() { sudo sed -i "s|^$1 *=.*|$1 = $2|" "$CFG"; grep "^$1 " "$CFG"; }

enroll() {
  # Swap models only after the new one is recognised; on any failure put the old
  # model and config back, so a failed enroll never leaves face unlock empty.
  local models=/etc/howdy/models/$USER.dat
  sudo cp -a "$models" "$models.bak.$STAMP" 2>/dev/null || true
  restore() {
    echo "Restoring the previous face model and config."
    [[ -f $models.bak.$STAMP ]] && sudo cp -a "$models.bak.$STAMP" "$models"
    [[ -f $CFG.bak.$STAMP ]] && sudo cp -a "$CFG.bak.$STAMP" "$CFG"
    exit 1
  }
  trap restore INT TERM
  sudo howdy -U "$USER" -y clear >/dev/null 2>&1 || true
  echo -e "\e[32mLook at the camera to enroll your face.\e[0m"
  sudo howdy -U "$USER" add || restore
  # Test as the user, not root: that's how the lock screen runs it
  echo -e "\e[32mTesting recognition as $USER (look at the camera)...\e[0m"
  rc=0; python3 /usr/lib/howdy/compare.py "$USER" || rc=$?
  (( rc == 0 )) || { echo "Not recognised (exit $rc)."; restore; }
  trap - INT TERM
  echo "Recognised."
}

rgb_camera() {
  local d
  for d in /sys/class/video4linux/video*; do
    [[ $(cat "$d/name" 2>/dev/null) == "Hardware ISP Camera" ]] && echo "/dev/${d##*/}" && return
  done
}

sudo -v

if [[ $1 == --undo ]]; then
  rgb=$(rgb_camera)
  [[ -n $rgb ]] || { echo "RGB camera (Hardware ISP Camera) not found. Stopping."; exit 1; }
  systemctl is-active -q v4l2-relayd@ipu7 || sudo systemctl start v4l2-relayd@ipu7
  sudo cp -a "$CFG" "$CFG.bak.$STAMP"
  set_cfg recording_plugin opencv
  set_cfg device_path "$rgb"
  set_cfg dark_threshold 60
  sudo rm -f "$RULE" && sudo udevadm control --reload
  sudo udevadm trigger --subsystem-match=video4linux --attr-match=name="Intel IPU7 ISYS Capture 16"
  enroll
  echo "Back on the RGB camera. (The ir_reader patch to Howdy is left in place; it's inert.)"
  exit 0
fi

if [[ ! -e /sys/bus/i2c/devices/i2c-HIMX1092:00/driver ]]; then
  echo "IR sensor has no driver loaded. Run install-ir.sh and reboot first."; exit 1
fi

# 1. Let the logged-in user open the IR capture node (Omarchy hides raw IPU7 nodes)
sudo cp "$HERE/72-hm1092-ir-capture.rules" "$RULE"
sudo udevadm control --reload
sudo udevadm trigger --subsystem-match=video4linux --attr-match=name="Intel IPU7 ISYS Capture 16"
sudo udevadm settle

# 2. Howdy IR recorder (raw 10-bit grey reader; the sensor is mono but tagged Bayer)
SRC=$(mktemp -d); trap 'rm -rf "$SRC"' EXIT
fetch_howdy_files "$SRC"
sudo install -m 0644 "$SRC/ir_reader.py" "$REC/ir_reader.py"
if ! grep -q ir_reader "$REC/video_capture.py"; then
  sudo cp -a "$REC/video_capture.py" "$REC/video_capture.py.bak.$STAMP"
  sudo patch -p1 --batch --forward --no-backup-if-mismatch "$REC/video_capture.py" < "$SRC/ir-recorder-video_capture.patch" || {
    sudo cp -a "$REC/video_capture.py.bak.$STAMP" "$REC/video_capture.py"
    echo "Howdy's video_capture.py changed upstream and the IR patch no longer applies. Nothing switched."; exit 1; }
fi

# 3. Config: IR frames are mostly dark background, so raise dark_threshold
sudo cp -a "$CFG" "$CFG.bak.$STAMP"
set_cfg recording_plugin ir
set_cfg device_path /dev/video16
set_cfg dark_threshold 90
# Search a 240-high image (the IR frame is 368): ~0.3 s faster per scan, still a
# clear match (certainty 2.0-3.1 against the 3.5 limit) -- measured 2026-10-02
set_cfg max_height 240

# 4. RGB face models don't match IR images: enroll again
enroll

echo -e "\e[32mDone. Face unlock now uses the IR camera.\e[0m"
echo "Undo: bash $0 --undo"
