//! The HM1092 IR camera, read the way Howdy's ir_reader.py reads it: set up the
//! IPU7 media graph with media-ctl, stream SGRBG10 frames over V4L2 mmap, and
//! treat each 16-bit sample as 10-bit grey (the sensor is mono; the Bayer tag
//! is only what the firmware declares). Switches the IR flood LED's sysfs node
//! on while streaming, as ir_reader does.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::process::Command;
use std::time::Duration;

const MEDIA_CTL: &str = "/usr/bin/media-ctl";
const SENSOR_ENTITY: &str = "hm1092";
const FMT: &str = "SGRBG10_1X10/648x368";
pub const IR_LED: &str = "/sys/class/leds/HIMX1092_00::ir_flood_led/brightness";
const PIX_FMT_SGRBG10: u32 = 0x3031_4142; // 'BA10'
const NBUF: u32 = 4;
/// Longest wait for one frame before giving up (the first takes ~0.3 s).
const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

// <linux/videodev2.h>, x86_64 (sizes checked against the kernel headers)
const VIDIOC_S_FMT: u64 = 0xc0d0_5605;
const VIDIOC_REQBUFS: u64 = 0xc014_5608;
const VIDIOC_QUERYBUF: u64 = 0xc058_5609;
const VIDIOC_QBUF: u64 = 0xc058_560f;
const VIDIOC_DQBUF: u64 = 0xc058_5611;
const VIDIOC_STREAMON: u64 = 0x4004_5612;
const VIDIOC_STREAMOFF: u64 = 0x4004_5613;
const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const FIELD_NONE: u32 = 1;

#[repr(C)]
#[derive(Default)]
struct PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    enc: u32,
    quantization: u32,
    xfer_func: u32,
}

#[repr(C)]
struct Format {
    typ: u32,
    _pad: u32,
    pix: PixFormat,
    _rest: [u8; 200 - 48],
}

#[repr(C)]
#[derive(Default)]
struct RequestBuffers {
    count: u32,
    typ: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
}

#[repr(C)]
#[derive(Default)]
struct Buffer {
    index: u32,
    typ: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    _pad: u32,
    timestamp: [i64; 2],
    timecode: [u32; 4],
    sequence: u32,
    memory: u32,
    offset: u64, // union m: mmap offset in the low 32 bits
    length: u32,
    reserved2: u32,
    request_fd: u32,
    _pad2: u32,
}

const _: () = assert!(std::mem::size_of::<Format>() == 208);
const _: () = assert!(std::mem::size_of::<RequestBuffers>() == 20);
const _: () = assert!(std::mem::size_of::<Buffer>() == 88);

fn ioctl<T>(fd: i32, req: u64, arg: &mut T) -> std::io::Result<()> {
    if unsafe { libc::ioctl(fd, req as _, arg as *mut T) } < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Write the IR flood LED. Returns whether the write worked.
pub fn set_ir_led(on: bool) -> bool {
    OpenOptions::new()
        .write(true)
        .open(IR_LED)
        .and_then(|mut f| f.write_all(if on { b"1" } else { b"0" }))
        .is_ok()
}

pub fn ir_led_state() -> String {
    std::fs::read_to_string(IR_LED).map(|s| s.trim().to_string()).unwrap_or_else(|_| "?".into())
}

// --- media graph (ir_reader.configure_pipeline) -----------------------------

fn topology(dev: &str) -> String {
    Command::new(MEDIA_CTL)
        .args(["-d", dev, "-p"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// "- entity 12: name (1 pad, ...)" -> "name"
fn entity_name(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix("- entity ")?;
    let (_, name) = rest.split_once(": ")?;
    Some(name.split(" (").next().unwrap_or(name).trim())
}

/// The first `-> "target"` link from `entity` whose target contains `want`.
fn link_from<'a>(topo: &'a str, entity: &str, want: &str) -> Option<&'a str> {
    let mut inside = false;
    for line in topo.lines() {
        if let Some(name) = entity_name(line) {
            if inside {
                return None;
            }
            inside = name == entity;
            continue;
        }
        if inside {
            if let Some(i) = line.find("-> \"") {
                let target = &line[i + 4..];
                let target = &target[..target.find('"')?];
                if target.contains(want) {
                    return Some(target);
                }
            }
        }
    }
    None
}

/// Set the pad formats and enable the CSI2 -> capture link, then return the
/// capture node. Everything is found by entity name: the /dev numbers move
/// between boots.
pub fn configure_pipeline() -> Option<String> {
    let mut devs: Vec<String> = std::fs::read_dir("/dev")
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("media"))
        .map(|n| format!("/dev/{n}"))
        .collect();
    devs.sort();
    for dev in devs {
        let topo = topology(&dev);
        if !topo.contains(SENSOR_ENTITY) {
            continue;
        }
        let Some(sensor) = topo.lines().filter_map(entity_name).find(|n| n.starts_with(SENSOR_ENTITY)) else {
            continue;
        };
        let Some(csi2) = link_from(&topo, sensor, "CSI2") else { continue };
        let Some(cap) = link_from(&topo, csi2, "Capture") else { continue };
        for args in [
            ["--set-v4l2".to_string(), format!("\"{sensor}\":0 [fmt:{FMT}]")],
            ["--set-v4l2".to_string(), format!("\"{csi2}\":0 [fmt:{FMT}]")],
            ["--set-v4l2".to_string(), format!("\"{csi2}\":1 [fmt:{FMT}]")],
            ["-l".to_string(), format!("\"{csi2}\":1 -> \"{cap}\":0 [1]")],
        ] {
            let _ = Command::new(MEDIA_CTL).args(["-d", &dev]).args(&args).output();
        }
        let mut ent = None;
        for line in topology(&dev).lines() {
            if let Some(name) = entity_name(line) {
                ent = Some(name.to_string());
            }
            if line.contains("device node name /dev/video") && ent.as_deref() == Some(cap) {
                return line.split_whitespace().last().map(String::from);
            }
        }
        return None;
    }
    None
}

// --- capture ----------------------------------------------------------------

pub struct Camera {
    file: File,
    buffers: Vec<(*mut libc::c_void, usize)>,
    streaming: bool,
    led_on: bool,
    pub width: usize,
    pub height: usize,
    bytesperline: usize,
}

impl Camera {
    /// `device` is config.ini's device_path; the media graph wins if it disagrees.
    pub fn open(device: &str) -> std::io::Result<Camera> {
        let device = configure_pipeline().unwrap_or_else(|| device.to_string());
        let file = OpenOptions::new().read(true).write(true).open(&device)?;
        let fd = file.as_raw_fd();

        let mut fmt = Format {
            typ: BUF_TYPE_VIDEO_CAPTURE,
            _pad: 0,
            pix: PixFormat {
                width: 648,
                height: 368,
                pixelformat: PIX_FMT_SGRBG10,
                field: FIELD_NONE,
                ..Default::default()
            },
            _rest: [0; 152],
        };
        ioctl(fd, VIDIOC_S_FMT, &mut fmt)?;
        let width = fmt.pix.width as usize;
        let height = fmt.pix.height as usize;
        let bytesperline = if fmt.pix.bytesperline > 0 { fmt.pix.bytesperline as usize } else { width * 2 };
        if bytesperline < width * 2 {
            return Err(std::io::Error::other(format!("stride {bytesperline} too small for {width} pixels")));
        }

        let mut req = RequestBuffers { count: NBUF, typ: BUF_TYPE_VIDEO_CAPTURE, memory: MEMORY_MMAP, ..Default::default() };
        ioctl(fd, VIDIOC_REQBUFS, &mut req)?;

        let mut cam = Camera { file, buffers: Vec::new(), streaming: false, led_on: false, width, height, bytesperline };
        for i in 0..req.count {
            let mut buf = Buffer { index: i, typ: BUF_TYPE_VIDEO_CAPTURE, memory: MEMORY_MMAP, ..Default::default() };
            ioctl(fd, VIDIOC_QUERYBUF, &mut buf)?;
            let len = buf.length as usize;
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    (buf.offset & 0xffff_ffff) as libc::off_t,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err(std::io::Error::last_os_error());
            }
            cam.buffers.push((ptr, len));
            ioctl(fd, VIDIOC_QBUF, &mut buf)?;
        }
        Ok(cam)
    }

    fn start(&mut self) -> std::io::Result<()> {
        self.led_on = set_ir_led(true);
        let mut typ = BUF_TYPE_VIDEO_CAPTURE as i32;
        ioctl(self.file.as_raw_fd(), VIDIOC_STREAMON, &mut typ)?;
        self.streaming = true;
        Ok(())
    }

    /// The next frame as 8-bit grey (width x height, no padding) into `out`;
    /// returns the sum of its pixels. None on a stalled stream or a failed dequeue.
    pub fn read(&mut self, out: &mut Vec<u8>) -> Option<u64> {
        if !self.streaming && self.start().is_err() {
            return None;
        }
        let fd = self.file.as_raw_fd();
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut pfd, 1, FRAME_TIMEOUT.as_millis() as i32) } <= 0 {
            return None;
        }
        let mut buf = Buffer { typ: BUF_TYPE_VIDEO_CAPTURE, memory: MEMORY_MMAP, ..Default::default() };
        if ioctl(fd, VIDIOC_DQBUF, &mut buf).is_err() {
            return None;
        }
        let (ptr, len) = *self.buffers.get(buf.index as usize)?;
        let need = self.bytesperline * self.height;
        let mut sum = 0u64;
        if len >= need {
            let raw = unsafe { std::slice::from_raw_parts(ptr as *const u8, need) };
            out.clear();
            out.reserve(self.width * self.height);
            for row in raw.chunks_exact(self.bytesperline) {
                // 16-bit little-endian containers holding 10-bit values: keep the top 8 bits
                out.extend(row[..self.width * 2].as_chunks::<2>().0.iter().map(|&p| {
                    let v = (u16::from_le_bytes(p) >> 2) as u8;
                    sum += v as u64;
                    v
                }));
            }
        }
        let requeued = ioctl(fd, VIDIOC_QBUF, &mut buf).is_ok();
        (len >= need && requeued).then_some(sum)
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        if self.streaming {
            let mut typ = BUF_TYPE_VIDEO_CAPTURE as i32;
            let _ = ioctl(self.file.as_raw_fd(), VIDIOC_STREAMOFF, &mut typ);
        }
        for &(ptr, len) in &self.buffers {
            unsafe { libc::munmap(ptr, len) };
        }
        if self.led_on {
            set_ir_led(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOPO: &str = r#"
- entity 1: Intel IPU7 CSI2 2 (2 pads, 2 links, 0 routes)
            type V4L2 subdev subtype Unknown flags 0
	pad0: Sink
		<- "hm1092 2-0024":0 [ENABLED,IMMUTABLE]
	pad1: Source
		-> "Intel IPU7 ISYS Capture 16":0 []

- entity 40: Intel IPU7 ISYS Capture 16 (1 pad, 1 link)
             type Node subtype V4L flags 0
             device node name /dev/video16
	pad0: Sink
		<- "Intel IPU7 CSI2 2":1 []

- entity 90: hm1092 2-0024 (1 pad, 1 link, 0 routes)
             type V4L2 subdev subtype Sensor flags 0
	pad0: Source
		-> "Intel IPU7 CSI2 2":0 [ENABLED,IMMUTABLE]
"#;

    #[test]
    fn follows_the_media_graph() {
        let sensor = TOPO.lines().filter_map(entity_name).find(|n| n.starts_with("hm1092")).unwrap();
        assert_eq!(sensor, "hm1092 2-0024");
        let csi2 = link_from(TOPO, sensor, "CSI2").unwrap();
        assert_eq!(csi2, "Intel IPU7 CSI2 2");
        assert_eq!(link_from(TOPO, csi2, "Capture"), Some("Intel IPU7 ISYS Capture 16"));
    }
}
