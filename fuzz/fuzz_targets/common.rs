//! Shared helpers: in-memory read/write callbacks for the C API.

use core::ffi::{c_int, c_void};
use gif::dgif::*;
use gif::types::*;

pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

pub unsafe extern "C" fn read_cb(g: *mut GifFileType, buf: *mut u8, len: c_int) -> c_int {
    unsafe {
        let r = &mut *((*g).UserData as *mut Reader);
        let n = (len.max(0) as usize).min(r.data.len() - r.pos);
        core::ptr::copy_nonoverlapping(r.data.as_ptr().add(r.pos), buf, n);
        r.pos += n;
        n as c_int
    }
}

#[allow(dead_code)] // used by the roundtrip target only
pub unsafe extern "C" fn write_cb(g: *mut GifFileType, buf: *const u8, len: c_int) -> c_int {
    unsafe {
        let out = &mut *((*g).UserData as *mut Vec<u8>);
        out.extend_from_slice(core::slice::from_raw_parts(buf, len.max(0) as usize));
        len
    }
}

/// Open `data` for reading and slurp it. Returns the handle (or null) and the slurp status.
pub unsafe fn slurp(reader: &mut Reader) -> (*mut GifFileType, c_int) {
    unsafe {
        let mut err = 0;
        let g = DGifOpen(reader as *mut Reader as *mut c_void, Some(read_cb), &mut err);
        if g.is_null() {
            return (g, GIF_ERROR);
        }
        let status = DGifSlurp(g);
        (g, status)
    }
}

pub unsafe fn raster<'a>(sp: &SavedImage) -> &'a [u8] {
    let n = sp.ImageDesc.Width as usize * sp.ImageDesc.Height as usize;
    unsafe { core::slice::from_raw_parts(sp.RasterBits, n) }
}
