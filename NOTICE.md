# Third-party code

The MIT licence in `LICENSE` covers the original scripts, `warm/howdy-warmd`,
`warm/client/`, `ir/howdy/ir_reader-fixes.patch` and the udev rule. Everything
else keeps its own licence:

| File | Source | Licence |
|---|---|---|
| `ir/hm1092/hm1092.c` | [HritwikSinghal/svp7500-camera-fix-pack](https://github.com/HritwikSinghal/svp7500-camera-fix-pack) `v1.1` (`25cc4c9e`), `dkms/hm1092-1.0/hm1092.c`, unchanged | GPL-2.0-only (SPDX header in the file) |
| `ir/ipu-bridge/ipu-bridge.c` | Linux v7.2.5 `drivers/media/pci/intel/ipu-bridge.c` + linux-omarchy patches `0540`/`0541` + mainline `4fdb0342f05e` | GPL-2.0 |
| `ir/ipu-bridge/0001-...patch` | mainline commit `4fdb0342f05e` (Jake Steinman, Sakari Ailus) | GPL-2.0 |
| Howdy IR reader + hook | **Not in this repo.** `ir/enable-ir-howdy.sh` downloads them from the fix pack at a pinned commit and checks their sha256: that repository has no licence, so they aren't redistributed here | — |
| `warm/howdy-warmd` scan loop | follows Howdy's `compare.py` ([boltgolt/howdy](https://github.com/boltgolt/howdy)) | MIT |
| `lock/face-lock.patch` | changes to Omarchy's lock plugin ([omacom/omarchy](https://github.com/omacom/omarchy), MIT), based on [pb3975/omarchy-face-auth](https://github.com/pb3975/omarchy-face-auth) | MIT, see `lock/LICENSE.face-auth` |

Thanks to Jake Steinman (the LINK_FREQ fix that made the HM1092 stream),
Hritwik Singhal (the maintained fix pack) and pkolbas (Dell Pro 14 report in
[omarchy-pkgs#366](https://github.com/omacom/omarchy-pkgs/issues/366)).
