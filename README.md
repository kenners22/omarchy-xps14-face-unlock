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

Everything is driven by one Rust tool, `face-unlock`, with a step per job.
Each step uses `sudo` where it needs root, so run it as yourself, and each has
`--undo`.

| Step | What it does |
|---|---|
| `face-unlock setup` | Installs Howdy (`howdy-git`, AUR), enrolls you, adds `pam_howdy` to sudo, polkit and the lock screen, after checking recognition works. Uses the XPS 14's camera, or your own with `FACE_UNLOCK_CAMERA=/dev/v4l/by-path/...`. |
| `face-unlock ir` | Takes a bootable snapper snapshot, then installs two DKMS modules from `ir/`. **`hm1092`** is the IR sensor driver from [HritwikSinghal/svp7500-camera-fix-pack](https://github.com/HritwikSinghal/svp7500-camera-fix-pack) `v1.1`, unchanged. **`ipu-bridge-himx`** is Omarchy's exact `ipu-bridge.c` for `7.2.5-3-omarchy` (kernel.org v7.2.5 + linux-omarchy patches `0540`/`0541`; the rebuild matches the shipped module's srcversion) plus the `HIMX1092` entry from mainline `4fdb0342f05e` (in Linux 7.3). |
| `face-unlock ir-howdy` | Switches Howdy to the IR camera. It downloads the fix pack's raw IR reader for Howdy (pinned commit, sha256-checked) and applies `ir/howdy/ir_reader-fixes.patch`: a frame timeout so a stalled stream can't hang sudo, absolute `media-ctl`, and ignoring size changes. It adds a udev rule giving the logged-in user the IR capture node, which Omarchy's `71-ipu7-hide-isys.rules` makes root-only, then re-enrolls. `--undo` goes back to the webcam. |
| `face-unlock warm` | Makes it fast (`warm/`). `howdy-warmd` is a root service that keeps the face models loaded and runs Howdy's scan loop with Howdy's config and face models. It is Rust, with Howdy's OpenCV steps (CLAHE, the darkness check, the resize) re-implemented to give OpenCV's results byte for byte, and dlib's C++ library (Arch's `dlib` package, which this step installs) reached through one small C++ file in place of its Python bindings. It reads the IR camera itself, the same way Howdy's IR reader does. `howdy-warm-auth` (Rust, std only) is what PAM runs via `pam_exec` instead of `pam_howdy`. If the service is down, or Howdy is set to a camera other than the IR reader, the client runs stock Howdy, so face unlock still works, just slower. The step builds both, tests them on you, then switches PAM. |
| `face-unlock lock` | The PAM stack for the patched lock screen in `lock/`: a patch for a **clone** of Omarchy's lock screen (`omarchy plugin clone omarchy.lock`) that adds a face-scan path. When the lock comes up after 20 s without input (idle lock) or after resume, it scans at once. When you lock it yourself (Super+Ctrl+L), it waits for a key or trackpad press, so locking at the desk doesn't unlock straight away. The patched lock screen also looks like Omarchy's boot unlock screen instead of the blurred wallpaper, and follows the style you pick under Style > Unlock in Omarchy's menu (same logo, padlock and colours), with a face icon in the password box. Based on [pb3975/omarchy-face-auth](https://github.com/pb3975/omarchy-face-auth). With the warm service installed, the lock screen goes through it too. |

## Order

Needs Rust (`mise use -g rust@stable`).

```bash
cargo build --release                     # builds face-unlock (the daemon is built by the warm step)
alias face-unlock=$PWD/target/release/face-unlock
face-unlock setup                         # Howdy on the webcam first (works in light)
face-unlock ir && reboot                  # IR driver + ipu-bridge entry
face-unlock ir-howdy                      # Howdy on the IR camera, re-enroll
face-unlock warm                          # fast path (installs dlib and openblas)
# lock screen: clone + patch (see lock/face-lock.patch header), then
face-unlock lock
```

### Speed (XPS 14, looking at the camera)

| | sudo by face |
|---|---|
| Stock Howdy | ~1.4 s (~0.9 s of it is starting Python and loading OpenCV/dlib for every scan) |
| With `warm/` | ~0.27 s: the sum of the steps below, IR camera start ~0.22 s + one frame matched |

Where a warm unlock's time goes (measured 5 October 2026):

| Step | Time |
|---|---|
| media-ctl set-up, opening the node | 15 ms |
| STREAMON: waking the IPU7 from runtime suspend | ~107 ms (only if it has been idle for over ~2 s) |
| STREAMON: IPU7 stream start | ~15 ms |
| STREAMON: the sensor's 238 register writes, one USB round trip each (its I2C bus is the `usbio` USB bridge) | ~80 ms |
| First frame | ~20 ms |
| Matching that frame | ~27 ms |
| STREAMOFF (~0.2 s) | after the answer, so not on the path |

The service answers before it closes the camera. The two biggest remaining
costs are outside this repo's user-space code: the IPU7 waking up (kept awake
with `power/control=on` on PCI device `0000:00:05.0`, at some battery cost) and
the register writes (fewer round trips if `hm1092` sent consecutive registers
as bursts: 238 writes are 102 runs).

Per frame, the Rust service gives the same results as Howdy's Python code on
the same frames: the image steps are byte-for-byte OpenCV's (CLAHE, darkness,
resize, rotation, on real IR frames and test patterns), and face descriptors
are within 1e-6. A frame without a face takes 16 ms (dlib's face detector). A
frame with one takes 27 ms, down from 124 ms, because dlib's face network runs
on OpenBLAS instead of the reference BLAS that Arch's dlib (and Howdy's
python-dlib) use. OpenBLAS is linked into the service only; the system BLAS
stays as it is. The service holds about 56 MB resident against 177 MB for the
Python one, and links no OpenCV.

Setting Howdy's `max_height = 240` (the IR frame is 368 high) saves another
~0.3 s with no loss of accuracy here.

## Things that will need care

- **Kernel updates.** `ipu-bridge-himx` is built for `7.2.5-3-omarchy` only
  (`AUTOINSTALL=no`), on purpose. A new kernel falls back to its stock
  `ipu-bridge`: the webcam is fine, but IR is off and face unlock falls back to
  your password until the source is refreshed for that kernel. From Linux 7.3
  the `HIMX1092` entry is upstream, so only `hm1092` is needed.
- **Howdy updates** (`howdy-git` rebuilds) overwrite the recorder hook; re-run
  `face-unlock ir-howdy`. (The warm service has its own camera reader, so it
  keeps working in the meantime.)
- **dlib updates.** `howdy-warmd` links Arch's `libdlib.so`. If a `dlib`
  upgrade changes its soname, rebuild: `face-unlock warm`. (OpenCV upgrades
  don't affect it: it doesn't link OpenCV.)
- **Security.** The warm service runs as root. A non-root caller can only ask
  about its own user (SO_PEERCRED). SSH sessions and a closed lid are refused,
  as with `pam_howdy`. The client ignores environment overrides, so a user
  can't point sudo at a fake service.
- **Face unlock is weaker than a password.** The IR camera rejects photos far
  better than a webcam does, but treat it as a convenience.

## Credits and licences

See [NOTICE.md](NOTICE.md). Original code here is MIT ([LICENSE](LICENSE)).
