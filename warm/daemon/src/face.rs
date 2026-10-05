//! Safe wrapper round face.cpp (dlib).

use std::ffi::{c_char, c_int, CStr, CString};

use crate::imgproc::Image;

#[repr(C)]
struct FwModels {
    _private: [u8; 0],
}

extern "C" {
    fn fw_load(sp: *const c_char, rec: *const c_char, cnn: *const c_char, err: *mut c_char, errlen: usize) -> *mut FwModels;
    fn fw_free(m: *mut FwModels);
    #[allow(clippy::too_many_arguments)]
    fn fw_faces(
        m: *mut FwModels,
        gs: *const u8,
        frame: *const u8,
        w: c_int,
        h: c_int,
        desc: *mut f32,
        max_faces: c_int,
        err: *mut c_char,
        errlen: usize,
    ) -> c_int;
}

const MAX_FACES: usize = 8;
pub const DIM: usize = 128;

pub struct Models {
    ptr: *mut FwModels,
}

// The models are only ever used under the camera lock, one scan at a time.
unsafe impl Send for Models {}

fn err_string(buf: &[c_char]) -> String {
    unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
}

impl Models {
    pub fn load(shape_predictor: &str, recognition: &str, cnn_detector: Option<&str>) -> Result<Models, String> {
        let sp = CString::new(shape_predictor).map_err(|e| e.to_string())?;
        let rec = CString::new(recognition).map_err(|e| e.to_string())?;
        let cnn = cnn_detector.map(CString::new).transpose().map_err(|e| e.to_string())?;
        let mut err = [0 as c_char; 512];
        let ptr = unsafe {
            fw_load(
                sp.as_ptr(),
                rec.as_ptr(),
                cnn.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                err.as_mut_ptr(),
                err.len(),
            )
        };
        if ptr.is_null() {
            return Err(err_string(&err));
        }
        Ok(Models { ptr })
    }

    /// Faces found in `gs` (the CLAHE'd frame), described from `frame` (the
    /// plain one): one 128-d descriptor each.
    pub fn faces(&mut self, gs: &Image, frame: &Image) -> Result<Vec<[f32; DIM]>, String> {
        assert!(gs.width == frame.width && gs.height == frame.height);
        let mut desc = vec![0f32; DIM * MAX_FACES];
        let mut err = [0 as c_char; 512];
        let n = unsafe {
            fw_faces(
                self.ptr,
                gs.data.as_ptr(),
                frame.data.as_ptr(),
                gs.width as c_int,
                gs.height as c_int,
                desc.as_mut_ptr(),
                MAX_FACES as c_int,
                err.as_mut_ptr(),
                err.len(),
            )
        };
        if n < 0 {
            return Err(err_string(&err));
        }
        Ok(desc.as_chunks::<DIM>().0[..n as usize].to_vec())
    }
}

impl Drop for Models {
    fn drop(&mut self) {
        unsafe { fw_free(self.ptr) }
    }
}
