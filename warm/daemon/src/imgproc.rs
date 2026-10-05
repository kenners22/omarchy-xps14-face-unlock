//! The OpenCV operations Howdy's scan uses, in Rust: CLAHE, the darkness
//! histogram, INTER_AREA downscaling and 90-degree rotation. Each follows
//! OpenCV's own code (imgproc/src/clahe.cpp, histogram.cpp, resize.cpp) step
//! for step, float rounding included, so the results are byte-for-byte what
//! cv2 gives (checked against OpenCV 5.0 on real IR frames).

/// An 8-bit grey image, rows packed with no padding.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

impl Image {
    pub fn new(data: Vec<u8>, width: usize, height: usize) -> Image {
        assert_eq!(data.len(), width * height);
        Image { data, width, height }
    }
}

/// cv::saturate_cast<uchar>(float): round half to even, then clamp.
fn saturate_u8(v: f32) -> u8 {
    v.round_ties_even().clamp(0.0, 255.0) as u8
}

/// BORDER_REFLECT_101 for an index at most one length past the end.
fn reflect101(i: usize, n: usize) -> usize {
    if i < n {
        i
    } else {
        (2 * n).saturating_sub(2 + i)
    }
}

/// cv::createCLAHE(clip_limit, (tiles_x, tiles_y)).apply(src)
pub fn clahe(src: &Image, clip_limit: f64, tiles_x: usize, tiles_y: usize) -> Image {
    const HIST: usize = 256;
    let (w, h) = (src.width, src.height);
    // Sizes that don't divide into tiles get the LUTs from a reflected border
    let ext_w = if w % tiles_x == 0 { w } else { w + tiles_x - w % tiles_x };
    let ext_h = if h % tiles_y == 0 { h } else { h + tiles_y - h % tiles_y };
    let (tile_w, tile_h) = (ext_w / tiles_x, ext_h / tiles_y);
    let tile_total = (tile_w * tile_h) as i32;
    let lut_scale = (HIST - 1) as f32 / tile_total as f32;
    let clip = if clip_limit > 0.0 { ((clip_limit * tile_total as f64 / HIST as f64) as i32).max(1) } else { 0 };

    // One LUT per tile
    let mut lut = vec![0u8; tiles_x * tiles_y * HIST];
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let mut hist = [0i32; HIST];
            for y in ty * tile_h..(ty + 1) * tile_h {
                let row = &src.data[reflect101(y, h) * w..][..w];
                let (x0, x1) = (tx * tile_w, (tx + 1) * tile_w);
                if x1 <= w {
                    for &p in &row[x0..x1] {
                        hist[p as usize] += 1;
                    }
                } else {
                    for x in x0..x1 {
                        hist[row[reflect101(x, w)] as usize] += 1;
                    }
                }
            }
            if clip > 0 {
                let mut clipped = 0;
                for v in hist.iter_mut() {
                    if *v > clip {
                        clipped += *v - clip;
                        *v = clip;
                    }
                }
                let batch = clipped / HIST as i32;
                let mut residual = clipped - batch * HIST as i32;
                for v in hist.iter_mut() {
                    *v += batch;
                }
                if residual != 0 {
                    let step = (HIST as i32 / residual).max(1) as usize;
                    let mut i = 0;
                    while i < HIST && residual > 0 {
                        hist[i] += 1;
                        i += step;
                        residual -= 1;
                    }
                }
            }
            let tile_lut = &mut lut[(ty * tiles_x + tx) * HIST..][..HIST];
            let mut sum = 0i32;
            for (out, &v) in tile_lut.iter_mut().zip(&hist) {
                sum += v;
                *out = saturate_u8(sum as f32 * lut_scale);
            }
        }
    }

    // Bilinear blend of the four nearest tiles' LUTs
    let inv_tw = 1.0f32 / tile_w as f32;
    let inv_th = 1.0f32 / tile_h as f32;
    let cols: Vec<(usize, usize, f32, f32)> = (0..w)
        .map(|x| {
            let txf = x as f32 * inv_tw - 0.5;
            let tx1 = txf.floor() as i32;
            let xa = txf - tx1 as f32;
            let tx2 = (tx1 + 1).min(tiles_x as i32 - 1) as usize;
            (tx1.max(0) as usize * HIST, tx2 * HIST, xa, 1.0 - xa)
        })
        .collect();
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let tyf = y as f32 * inv_th - 0.5;
        let ty1 = tyf.floor() as i32;
        let ya = tyf - ty1 as f32;
        let ya1 = 1.0 - ya;
        let ty2 = (ty1 + 1).min(tiles_y as i32 - 1) as usize;
        let plane1 = &lut[ty1.max(0) as usize * tiles_x * HIST..][..tiles_x * HIST];
        let plane2 = &lut[ty2 * tiles_x * HIST..][..tiles_x * HIST];
        let src_row = &src.data[y * w..][..w];
        let out_row = &mut out[y * w..][..w];
        for ((o, &v), &(i1, i2, xa, xa1)) in out_row.iter_mut().zip(src_row).zip(&cols) {
            let (a, b) = (i1 + v as usize, i2 + v as usize);
            let res = (plane1[a] as f32 * xa1 + plane1[b] as f32 * xa) * ya1
                + (plane2[a] as f32 * xa1 + plane2[b] as f32 * xa) * ya;
            *o = saturate_u8(res);
        }
    }
    Image::new(out, w, h)
}

/// Howdy's darkness: % of pixels in the lowest of 8 bins of
/// cv2.calcHist([img], [0], None, [8], [0, 256]), computed in float32 as numpy
/// does. 100 for an empty image.
pub fn darkness(img: &Image) -> f32 {
    let total = img.data.len() as f32;
    if total == 0.0 {
        return 100.0;
    }
    let dark = img.data.iter().filter(|&&p| p < 32).count() as f32;
    dark / total * 100.0
}

/// One weight of OpenCV's area-resize table: source index `si` adds
/// `alpha` x its value to destination index `di`.
struct Decimate {
    di: u32,
    si: u32,
    alpha: f32,
}

/// Is a resize by `f` one that `resize_area` does (a non-integer shrink)?
pub fn area_supported(f: f64) -> bool {
    let scale = 1.0 / f;
    scale >= 1.0 && (scale - scale.round_ties_even()).abs() >= f64::EPSILON
}

/// computeResizeAreaTab
fn area_tab(ssize: usize, dsize: usize, scale: f64) -> Vec<Decimate> {
    let mut tab = Vec::with_capacity(ssize * 2);
    for dx in 0..dsize {
        let fsx1 = dx as f64 * scale;
        let fsx2 = fsx1 + scale;
        let cell = scale.min(ssize as f64 - fsx1);
        let mut sx1 = fsx1.ceil() as i64;
        let sx2 = (fsx2.floor() as i64).min(ssize as i64 - 1);
        sx1 = sx1.min(sx2);
        if sx1 as f64 - fsx1 > 1e-3 {
            tab.push(Decimate { di: dx as u32, si: (sx1 - 1) as u32, alpha: ((sx1 as f64 - fsx1) / cell) as f32 });
        }
        for sx in sx1..sx2 {
            tab.push(Decimate { di: dx as u32, si: sx as u32, alpha: (1.0 / cell) as f32 });
        }
        if fsx2 - sx2 as f64 > 1e-3 {
            tab.push(Decimate { di: dx as u32, si: sx2 as u32, alpha: ((fsx2 - sx2 as f64).min(1.0).min(cell) / cell) as f32 });
        }
    }
    tab
}

/// cv2.resize(src, None, fx=f, fy=f, interpolation=cv2.INTER_AREA) for a
/// shrink by a non-integer factor (OpenCV's general area path). Returns None
/// for anything OpenCV handles another way (enlarging, which INTER_AREA turns
/// into INTER_LINEAR, or a whole-number shrink, which has its own fast path).
pub fn resize_area(src: &Image, f: f64) -> Option<Image> {
    if !area_supported(f) {
        return None;
    }
    let scale = 1.0 / f;
    // dsize = (saturate_cast<int>(w * f), saturate_cast<int>(h * f))
    let dw = (src.width as f64 * f).round_ties_even() as usize;
    let dh = (src.height as f64 * f).round_ties_even() as usize;
    let xtab = area_tab(src.width, dw, scale);
    let ytab = area_tab(src.height, dh, scale);

    let mut out = vec![0u8; dw * dh];
    let mut buf = vec![0f32; dw];
    let mut sum = vec![0f32; dw];
    let mut prev_dy = ytab.first().map_or(0, |t| t.di as usize);
    let mut buf_row = u32::MAX;
    for t in &ytab {
        let (beta, dy) = (t.alpha, t.di as usize);
        // A source row that straddles two output rows comes up twice in a row:
        // its horizontal sums are the same both times, so keep them.
        if t.si != buf_row {
            let row = &src.data[t.si as usize * src.width..][..src.width];
            buf.iter_mut().for_each(|b| *b = 0.0);
            for x in &xtab {
                buf[x.di as usize] += row[x.si as usize] as f32 * x.alpha;
            }
            buf_row = t.si;
        }
        if dy != prev_dy {
            let d = &mut out[prev_dy * dw..][..dw];
            for dx in 0..dw {
                d[dx] = saturate_u8(sum[dx]);
                sum[dx] = beta * buf[dx];
            }
            prev_dy = dy;
        } else {
            for dx in 0..dw {
                sum[dx] += beta * buf[dx];
            }
        }
    }
    let d = &mut out[prev_dy * dw..][..dw];
    for dx in 0..dw {
        d[dx] = saturate_u8(sum[dx]);
    }
    Some(Image::new(out, dw, dh))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rotate {
    None,
    Clockwise,
    CounterClockwise,
}

/// cv2.rotate(img, ROTATE_90_CLOCKWISE / ROTATE_90_COUNTERCLOCKWISE)
pub fn rotate(src: &Image, r: Rotate) -> Image {
    let (w, h) = (src.width, src.height);
    let pixel = |row: usize, col: usize| match r {
        Rotate::None => src.data[row * w + col],
        Rotate::Clockwise => src.data[(h - 1 - col) * w + row],
        Rotate::CounterClockwise => src.data[col * w + (w - 1 - row)],
    };
    let (ow, oh) = if r == Rotate::None { (w, h) } else { (h, w) };
    let data = (0..oh).flat_map(|row| (0..ow).map(move |col| (row, col))).map(|(row, col)| pixel(row, col)).collect();
    Image::new(data, ow, oh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize) -> Image {
        Image::new((0..w * h).map(|i| (i * 37 % 251) as u8).collect(), w, h)
    }

    #[test]
    fn rotations_undo_each_other() {
        let a = img(5, 3);
        let cw = rotate(&a, Rotate::Clockwise);
        assert_eq!((cw.width, cw.height), (3, 5));
        assert_eq!(rotate(&cw, Rotate::CounterClockwise), a);
        // top-left of a clockwise turn is the original bottom-left
        assert_eq!(cw.data[0], a.data[2 * 5]);
    }

    #[test]
    fn area_resize_keeps_flat_images_flat() {
        let flat = Image::new(vec![77; 648 * 368], 648, 368);
        let r = resize_area(&flat, 240.0 / 368.0).unwrap();
        assert_eq!((r.width, r.height), (423, 240));
        assert!(r.data.iter().all(|&p| p == 77));
    }

    #[test]
    fn area_resize_leaves_other_cases_to_opencv() {
        assert!(resize_area(&img(10, 10), 0.5).is_none());
        assert!(resize_area(&img(10, 10), 1.25).is_none());
    }

    #[test]
    fn clahe_keeps_size_and_handles_uneven_tiles() {
        let a = img(83, 47);
        let c = clahe(&a, 2.0, 8, 8);
        assert_eq!((c.width, c.height), (83, 47));
    }

    #[test]
    fn darkness_counts_the_lowest_bin() {
        let a = Image::new(vec![0, 31, 32, 255], 2, 2);
        assert_eq!(darkness(&a), 50.0);
    }
}
