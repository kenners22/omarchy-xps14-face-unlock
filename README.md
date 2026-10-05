# IR face unlock on the Dell XPS 14 (2026) with Omarchy

Windows Hello-style face unlock that works in the dark on the **Dell XPS 14
DA14260** (Intel Panther Lake, IPU7) running **Omarchy 4**. It uses the laptop's
Himax HM1092 IR camera, which has no mainline Linux driver yet, and it leaves
Omarchy's own camera stack (`intel-ipu7-camera`: `intel-cvs` 1.0.6,
`ipu7-drivers`) untouched, so the normal webcam keeps working.

Unlocks sudo, polkit (system prompts, 1Password) and the lock screen in about
**0.7 s** after you look at it.

> Community work, not an Omarchy feature. The IR driver is reverse-engineered,
> and the `ipu-bridge` rebuild is pinned to one kernel. Read the whole page
> before running anything, and keep your password: it stays the fallback
> everywhere.

## Status

| | |
|---|---|
| Tested on | XPS 14 DA14260, Omarchy 4.0.4, kernel `7.2.5-3-omarchy` |
| IR stream | 648x368 @ ~29 fps on IPU7 CSI2 port 2; flood LED switched by the driver |
| Webcam | unaffected (Omarchy's relay at "Hardware ISP Camera") |
| Not yet tested | IR after suspend/resume; XPS 16 (same chips, should be close) |

### Why this works without replacing Omarchy's drivers

On the XPS 14 the IR sensor is wired straight to the IPU, not through the Intel
CVS chip. The ACPI tables show it: `LNK0` (HIMX1092) depends only on its PMIC
and the USB bridge, while `LNK1` (the OV08X40 webcam) also depends on `CVSS`.
So the sensor needs only two things: a driver, and an `ipu-bridge` entry that
gives it a place in the camera graph. The full
[svp7500-camera-fix-pack](https://github.com/jibsta210/svp7500-camera-fix-pack)
also replaces `intel_cvs` and `ipu-bridge` with older copies, which breaks
Omarchy's webcam path and flips the image. **Don't install the full pack on Omarchy.**

## Pieces

| Folder | What it does |
|---|---|
| `setup/setup-face-unlock.sh` | Installs Howdy (`howdy-git`, AUR), enrolls you, adds `pam_howdy` to sudo, polkit and the lock screen, after checking recognition works. `--undo`. |
| `ir/install-ir.sh` | Takes a bootable snapper snapshot, then installs two DKMS modules. **`hm1092`** is the IR sensor driver from [HritwikSinghal/svp7500-camera-fix-pack](https://github.com/HritwikSinghal/svp7500-camera-fix-pack) `v1.1`, unchanged. **`ipu-bridge-himx`** is Omarchy's exact `ipu-bridge.c` for `7.2.5-3-omarchy` (kernel.org v7.2.5 + linux-omarchy patches `0540`/`0541`; the rebuild matches the shipped module's srcversion) plus the `HIMX1092` entry from mainline `4fdb0342f05e` (in Linux 7.3). `--undo`. |
| `ir/enable-ir-howdy.sh` | Switches Howdy to the IR camera. It downloads the fix pack's raw IR reader for Howdy (pinned commit, sha256-checked) and applies `ir_reader-fixes.patch`: a frame timeout so a stalled stream can't hang sudo, absolute `media-ctl`, and ignoring size changes. It adds a udev rule giving the logged-in user the IR capture node, which Omarchy's `71-ipu7-hide-isys.rules` makes root-only, then re-enrolls. `--undo` goes back to the webcam. |
| `warm/` | Makes it fast. `howdy-warmd` keeps Howdy's OpenCV and dlib models loaded as a root service and runs Howdy's own scan loop with Howdy's config and face models. `howdy-warm-auth` (Rust, std only) is what PAM runs via `pam_exec` instead of `pam_howdy`. If the service is down the client runs stock Howdy, so face unlock still works, just slower. `install-warm.sh` builds it, tests it on you, then switches PAM; `--undo`. |
| `lock/` | A patch for a **clone** of Omarchy's lock screen (`omarchy plugin clone omarchy.lock`) that adds a face-scan path, plus `enable-lock-autoscan.sh` for its PAM stack. When the lock comes up after 20 s without input (idle lock) or after resume, it scans at once. When you lock it yourself (Super+Ctrl+L), it waits for a key or trackpad press, so locking at the desk doesn't unlock straight away. The patched lock screen also looks like Omarchy's boot unlock screen instead of the blurred wallpaper, and follows the style you pick under Style > Unlock in Omarchy's menu (same logo, padlock and colours), with a face icon in the password box. Based on [pb3975/omarchy-face-auth](https://github.com/pb3975/omarchy-face-auth). |

## Order

```bash
bash setup/setup-face-unlock.sh           # Howdy on the webcam first (works in light)
bash ir/install-ir.sh && reboot           # IR driver + ipu-bridge entry
bash ir/enable-ir-howdy.sh                # Howdy on the IR camera, re-enroll
bash warm/install-warm.sh                 # fast path (needs Rust: mise use -g rust@stable)
# lock screen: clone + patch (see lock/face-lock.patch header), then
bash lock/enable-lock-autoscan.sh
```

### Speed (XPS 14, looking at the camera)

| | sudo by face |
|---|---|
| Stock Howdy | ~1.4 s (~0.9 s of it is starting Python and loading OpenCV/dlib for every scan) |
| With `warm/` | ~0.7 s (IR sensor wake ~0.25 s + one frame matched) |

Setting Howdy's `max_height = 240` (the IR frame is 368 high) saves another
~0.3 s with no loss of accuracy here.

## Things that will need care

- **Kernel updates.** `ipu-bridge-himx` is built for `7.2.5-3-omarchy` only
  (`AUTOINSTALL=no`), on purpose. A new kernel falls back to its stock
  `ipu-bridge`: the webcam is fine, but IR is off and face unlock falls back to
  your password until the source is refreshed for that kernel. From Linux 7.3
  the `HIMX1092` entry is upstream, so only `hm1092` is needed.
- **Howdy updates** (`howdy-git` rebuilds) overwrite the recorder hook; re-run
  `ir/enable-ir-howdy.sh`.
- **Security.** The warm service runs as root. A non-root caller can only ask
  about its own user (SO_PEERCRED). SSH sessions and a closed lid are refused,
  as with `pam_howdy`. The client ignores environment overrides, so a user
  can't point sudo at a fake service.
- **Face unlock is weaker than a password.** The IR camera rejects photos far
  better than a webcam does, but treat it as a convenience.

## Credits and licences

See [NOTICE.md](NOTICE.md). Original code here is MIT ([LICENSE](LICENSE)).
