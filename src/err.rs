//! Error strings (port of `gif_err.c`).

#![allow(non_snake_case)]

use crate::types::*;
use core::ffi::{c_char, c_int, CStr};
use core::ptr::null;

fn message(code: c_int) -> Option<&'static CStr> {
    Some(match code {
        E_GIF_ERR_OPEN_FAILED | D_GIF_ERR_OPEN_FAILED => c"Failed to open given file",
        E_GIF_ERR_WRITE_FAILED => c"Failed to write to given file",
        E_GIF_ERR_HAS_SCRN_DSCR => c"Screen descriptor has already been set",
        E_GIF_ERR_HAS_IMAG_DSCR => c"Image descriptor is still active",
        E_GIF_ERR_NO_COLOR_MAP | D_GIF_ERR_NO_COLOR_MAP => c"Neither global nor local color map",
        E_GIF_ERR_DATA_TOO_BIG | D_GIF_ERR_DATA_TOO_BIG => c"Number of pixels bigger than width * height",
        E_GIF_ERR_NOT_ENOUGH_MEM | D_GIF_ERR_NOT_ENOUGH_MEM => c"Failed to allocate required memory",
        E_GIF_ERR_DISK_IS_FULL => c"Write failed (disk full?)",
        E_GIF_ERR_CLOSE_FAILED | D_GIF_ERR_CLOSE_FAILED => c"Failed to close given file",
        E_GIF_ERR_NOT_WRITEABLE => c"Given file was not opened for write",
        D_GIF_ERR_READ_FAILED => c"Failed to read from given file",
        D_GIF_ERR_NOT_GIF_FILE => c"Data is not in GIF format",
        D_GIF_ERR_NO_SCRN_DSCR => c"No screen descriptor detected",
        D_GIF_ERR_NO_IMAG_DSCR => c"No Image Descriptor detected",
        D_GIF_ERR_WRONG_RECORD => c"Wrong record type detected",
        D_GIF_ERR_NOT_READABLE => c"Given file was not opened for read",
        D_GIF_ERR_IMAGE_DEFECT => c"Image is defective, decoding aborted",
        D_GIF_ERR_EOF_TOO_SOON => c"Image EOF detected before image complete",
        _ => return None,
    })
}

/// Human-readable description of an error code, or NULL if unknown.
#[unsafe(no_mangle)]
pub extern "C" fn GifErrorString(ErrorCode: c_int) -> *const c_char {
    message(ErrorCode).map_or(null(), CStr::as_ptr)
}
