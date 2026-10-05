# Third-party code

The MIT licence in `LICENSE` covers `installer/`, `warm/daemon/`,
`warm/client/`, `ir/howdy/ir_reader-fixes.patch` and the udev rule. Everything
else keeps its own licence:

| File | Source | Licence |
|---|---|---|
| `ir/hm1092/hm1092.c` | [HritwikSinghal/svp7500-camera-fix-pack](https://github.com/HritwikSinghal/svp7500-camera-fix-pack) `v1.1` (`25cc4c9e`), `dkms/hm1092-1.0/hm1092.c`, unchanged | GPL-2.0-only (SPDX header in the file) |
| `ir/ipu-bridge/ipu-bridge.c` | Linux v7.2.5 `drivers/media/pci/intel/ipu-bridge.c` + linux-omarchy patches `0540`/`0541` + mainline `4fdb0342f05e` | GPL-2.0 |
| `ir/ipu-bridge/0001-...patch` | mainline commit `4fdb0342f05e` (Jake Steinman, Sakari Ailus) | GPL-2.0 |
| Howdy IR reader + hook | **Not in this repo.** `face-unlock ir-howdy` downloads them from the fix pack at a pinned commit and checks their sha256: that repository has no licence, so they aren't redistributed here | — |
| `warm/daemon/src/main.rs` scan loop | follows Howdy's `compare.py` ([boltgolt/howdy](https://github.com/boltgolt/howdy)) | MIT |
| `warm/daemon/src/face.cpp` network definitions | dlib's Python bindings, `tools/python/src/face_recognition.cpp` and `cnn_face_detector.cpp` ([davisking/dlib](https://github.com/davisking/dlib)) | Boost Software License 1.0 |
| `warm/daemon/src/imgproc.rs` | follows OpenCV's `imgproc/src/clahe.cpp` and `resize.cpp` (computeResizeAreaTab, resizeArea_) step for step, to give the same results ([opencv/opencv](https://github.com/opencv/opencv)) | Apache-2.0 |
| `warm/daemon/src/camera.rs` | written here; it takes the same steps as the fix pack's `ir_reader.py` (media-ctl set-up, the 10-bit frame layout, the LED node) | MIT |
| `lock/face-lock.patch` | changes to Omarchy's lock plugin ([omacom/omarchy](https://github.com/omacom/omarchy), MIT), based on [pb3975/omarchy-face-auth](https://github.com/pb3975/omarchy-face-auth) | MIT, see `lock/LICENSE.face-auth` |

Thanks to Jake Steinman (the LINK_FREQ fix that made the HM1092 stream),
Hritwik Singhal (the maintained fix pack) and pkolbas (Dell Pro 14 report in
[omarchy-pkgs#366](https://github.com/omacom/omarchy-pkgs/issues/366)).
