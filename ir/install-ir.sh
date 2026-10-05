#!/bin/bash
# XPS 14 IR camera (Himax HM1092) for face unlock in the dark. Adds two kernel
# modules next to Omarchy's camera stack; intel-cvs / ipu7-drivers stay as they are.
#   bash install-ir.sh          # snapshot, then build + install both modules
#   bash install-ir.sh --undo   # remove both modules (reboot afterwards)
#
#   hm1092           IR sensor driver (community, reverse-engineered)
#   ipu-bridge-himx  Omarchy's ipu-bridge + the HIMX1092 entry from kernel 7.3,
#                    so the sensor gets a place in the camera graph

set -e

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
KERNEL=7.2.5-3-omarchy
MODULES=(hm1092/1.1 ipu-bridge-himx/7.2.5.3)

sudo -v

if [[ $1 == --undo ]]; then
  if grep -q '^recording_plugin *= *ir' /etc/howdy/config.ini 2>/dev/null; then
    echo "Howdy still uses the IR camera. First: bash $HERE/enable-ir-howdy.sh --undo"
    exit 1
  fi
  for m in "${MODULES[@]}"; do
    dkms status "$m" 2>/dev/null | grep -q . && sudo dkms remove "$m" --all
  done
  sudo depmod -a
  # Only drop the sources once the override modules are really gone
  for mod in ipu_bridge hm1092; do
    if modinfo -F filename "$mod" 2>/dev/null | grep -q updates/dkms; then
      echo "$mod is still installed from DKMS ($(modinfo -F filename $mod)). Sources kept; check: dkms status"; exit 1
    fi
  done
  for m in "${MODULES[@]}"; do sudo rm -rf "/usr/src/${m%/*}-${m#*/}"; done
  echo "Removed. Reboot to go back to the stock ipu-bridge (hm1092 can't be unloaded live)."
  exit 0
fi

if [[ $(uname -r) != "$KERNEL" ]]; then
  echo "This ipu-bridge source is exact for $KERNEL, but you're on $(uname -r). Stopping."
  exit 1
fi

# 1. Bootable snapshot first, so a bad result is one boot-menu pick away
echo -e "\e[32mTaking a snapshot...\e[0m"
snap=$(sudo snapper -c root create -t single -c number -u important=yes -p -d "Before XPS 14 IR camera modules")
# Make it a Limine boot entry; without that the snapshot is no quick way back
if command -v limine-snapper-sync >/dev/null; then
  sudo limine-snapper-sync || { echo "limine-snapper-sync failed, so snapshot #$snap isn't in the boot menu. Stopping."; exit 1; }
fi
echo "Snapshot #$snap (boot it from the Limine menu if the camera breaks)"

# 2. Build and install both modules with DKMS
for m in "${MODULES[@]}"; do
  name=${m%/*} ver=${m#*/} src=$HERE/${name%-himx}
  echo -e "\e[32mBuilding $name $ver...\e[0m"
  sudo rm -rf "/usr/src/$name-$ver"
  sudo cp -r "$src" "/usr/src/$name-$ver"
  sudo dkms remove "$m" --all &>/dev/null || true
  sudo dkms add "$m"
  sudo dkms install "$m" -k "$KERNEL"
done
sudo depmod -a

echo
modinfo -F filename ipu_bridge
modinfo -F filename hm1092
echo -e "\e[32mDone. Reboot, then: bash $HERE/enable-ir-howdy.sh\e[0m"
echo "Undo: bash $0 --undo   (or boot snapshot #$snap)"
