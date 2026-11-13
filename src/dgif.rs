//! GIF decoding: the C API of `dgif_lib.c`.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use crate::gifalloc::*;
use crate::lzw::{ByteSource, KEEP_ERROR, LzwResult};
use crate::private::*;
use crate::types::*;
use alloc::boxed::Box;
use core::ffi::{c_char, c_int, c_void};
use core::mem::size_of;
use core::ptr::{self, null_mut};

const INTERLACED_OFFSET: [usize; 4] = [0, 4, 2, 1];
const INTERLACED_JUMPS: [usize; 4] = [8, 8, 4, 2];

unsafe fn set_error(out: *mut c_int, code: c_int) {
    if !out.is_null() {
        unsafe { *out = code };
    }
}

/// Store an LZW error on the handle and return `GIF_ERROR`.
unsafe fn fail(gif: *mut GifFileType, code: c_int) -> c_int {
    if code != KEEP_ERROR {
        unsafe { (*gif).Error = code };
    }
    GIF_ERROR
}

unsafe fn check(gif: *mut GifFileType, r: LzwResult<()>) -> c_int {
    match r {
        Ok(()) => GIF_OK,
        Err(code) => unsafe { fail(gif, code) },
    }
}

/// Free a half-built handle (closes its file, if any).
unsafe fn destroy(gif: *mut GifFileType) {
    unsafe {
        if !(*gif).Private.is_null() {
            drop(Box::from_raw((*gif).Private as *mut Private));
        }
        GifFreeMapObject((*gif).SColorMap);
        free(gif.cast());
    }
}

unsafe fn read_word(gif: *mut GifFileType) -> Option<GifWord> {
    let (mut s, _) = unsafe { parts(gif) };
    let mut c = [0u8; 2];
    if s.read_exact(&mut c) {
        Some(u16::from_le_bytes(c) as GifWord)
    } else {
        unsafe { (*gif).Error = D_GIF_ERR_READ_FAILED };
        None
    }
}

unsafe fn read_bytes(gif: *mut GifFileType, buf: &mut [u8]) -> bool {
    let (mut s, _) = unsafe { parts(gif) };
    s.read_exact(buf)
}

/// Shared body of `DGifOpenFileHandle` and `DGifOpen`.
unsafe fn open_reader(io: Io, user: *mut c_void, error: *mut c_int, from_callback: bool) -> *mut GifFileType {
    unsafe {
        let gif = calloc(1, size_of::<GifFileType>()) as *mut GifFileType;
        if gif.is_null() {
            set_error(error, D_GIF_ERR_NOT_ENOUGH_MEM);
            return null_mut();
        }
        (*gif).Private = Box::into_raw(Private::new(io, FILE_STATE_READ)).cast();
        (*gif).UserData = user;

        let mut stamp = [0u8; 6];
        if !read_bytes(gif, &mut stamp) {
            set_error(error, D_GIF_ERR_READ_FAILED);
            destroy(gif);
            return null_mut();
        }
        if &stamp[..3] != b"GIF" {
            set_error(error, D_GIF_ERR_NOT_GIF_FILE);
            destroy(gif);
            return null_mut();
        }
        if DGifGetScreenDesc(gif) == GIF_ERROR {
            // Upstream leaves *Error unset here for file handles; report the real cause instead.
            set_error(error, if from_callback { D_GIF_ERR_NO_SCRN_DSCR } else { (*gif).Error });
            destroy(gif);
            return null_mut();
        }
        (*gif).Error = 0;
        private(gif).gif89 = stamp[4] == b'9';
        gif
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifOpenFileName(FileName: *const c_char, Error: *mut c_int) -> *mut GifFileType {
    unsafe {
        if FileName.is_null() {
            set_error(Error, D_GIF_ERR_OPEN_FAILED);
            return null_mut();
        }
        let fd = crate::sys::open_read(FileName);
        if fd == -1 {
            set_error(Error, D_GIF_ERR_OPEN_FAILED);
            return null_mut();
        }
        DGifOpenFileHandle(fd, Error)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifOpenFileHandle(FileHandle: c_int, Error: *mut c_int) -> *mut GifFileType {
    unsafe {
        if FileHandle < 0 {
            set_error(Error, D_GIF_ERR_READ_FAILED);
            return null_mut();
        }
        // We now own the descriptor; it is closed by DGifCloseFile or on failure.
        open_reader(Io::ReadFile(crate::sys::FdReader::new(FileHandle)), null_mut(), Error, false)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifOpen(userPtr: *mut c_void, readFunc: InputFunc, Error: *mut c_int) -> *mut GifFileType {
    unsafe { open_reader(Io::ReadFunc(readFunc), userPtr, Error, true) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetScreenDesc(GifFile: *mut GifFileType) -> c_int {
    unsafe {
        let gif = GifFile;
        if !private(gif).readable() {
            return fail(gif, D_GIF_ERR_NOT_READABLE);
        }
        let Some(w) = read_word(gif) else { return GIF_ERROR };
        (*gif).SWidth = w;
        let Some(h) = read_word(gif) else { return GIF_ERROR };
        (*gif).SHeight = h;

        let mut buf = [0u8; 3];
        if !read_bytes(gif, &mut buf) {
            GifFreeMapObject((*gif).SColorMap);
            (*gif).SColorMap = null_mut();
            return fail(gif, D_GIF_ERR_READ_FAILED);
        }
        (*gif).SColorResolution = ((((buf[0] & 0x70) as c_int) + 1) >> 4) + 1;
        let sort_flag = buf[0] & 0x08 != 0;
        let bits = (buf[0] & 0x07) as c_int + 1;
        (*gif).SBackGroundColor = buf[1] as c_int;
        (*gif).AspectByte = buf[2];

        if buf[0] & 0x80 != 0 {
            let map = GifMakeMapObject(1 << bits, ptr::null());
            (*gif).SColorMap = map;
            if map.is_null() {
                return fail(gif, D_GIF_ERR_NOT_ENOUGH_MEM);
            }
            (*map).SortFlag = sort_flag;
            if !read_color_map(gif, map) {
                GifFreeMapObject(map);
                (*gif).SColorMap = null_mut();
                return fail(gif, D_GIF_ERR_READ_FAILED);
            }
        } else {
            (*gif).SColorMap = null_mut();
        }
        GIF_OK
    }
}

unsafe fn read_color_map(gif: *mut GifFileType, map: *mut ColorMapObject) -> bool {
    unsafe {
        let colors = slice_mut((*map).Colors, (*map).ColorCount as usize);
        for c in colors {
            let mut rgb = [0u8; 3];
            if !read_bytes(gif, &mut rgb) {
                return false;
            }
            *c = GifColorType { Red: rgb[0], Green: rgb[1], Blue: rgb[2] };
        }
        true
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetGifVersion(GifFile: *mut GifFileType) -> *const c_char {
    unsafe { if private(GifFile).gif89 { GIF89_STAMP.as_ptr() } else { GIF87_STAMP.as_ptr() } }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetRecordType(GifFile: *mut GifFileType, Type: *mut GifRecordType) -> c_int {
    unsafe {
        if !private(GifFile).readable() {
            return fail(GifFile, D_GIF_ERR_NOT_READABLE);
        }
        let mut b = [0u8; 1];
        if !read_bytes(GifFile, &mut b) {
            return fail(GifFile, D_GIF_ERR_READ_FAILED);
        }
        *Type = match b[0] {
            0x2c => IMAGE_DESC_RECORD_TYPE,
            0x21 => EXTENSION_RECORD_TYPE,
            0x3b => TERMINATE_RECORD_TYPE,
            _ => {
                *Type = UNDEFINED_RECORD_TYPE;
                return fail(GifFile, D_GIF_ERR_WRONG_RECORD);
            }
        };
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetImageHeader(GifFile: *mut GifFileType) -> c_int {
    unsafe {
        let gif = GifFile;
        if !private(gif).readable() {
            return fail(gif, D_GIF_ERR_NOT_READABLE);
        }
        for field in 0..4 {
            let Some(v) = read_word(gif) else { return GIF_ERROR };
            let img = &mut (*gif).Image;
            match field {
                0 => img.Left = v,
                1 => img.Top = v,
                2 => img.Width = v,
                _ => img.Height = v,
            }
        }
        let mut b = [0u8; 1];
        if !read_bytes(gif, &mut b) {
            GifFreeMapObject((*gif).Image.ColorMap);
            (*gif).Image.ColorMap = null_mut();
            return fail(gif, D_GIF_ERR_READ_FAILED);
        }
        let bits = (b[0] & 0x07) as c_int + 1;
        (*gif).Image.Interlace = b[0] & 0x40 != 0;

        if !(*gif).Image.ColorMap.is_null() {
            GifFreeMapObject((*gif).Image.ColorMap);
            (*gif).Image.ColorMap = null_mut();
        }
        if b[0] & 0x80 != 0 {
            let map = GifMakeMapObject(1 << bits, ptr::null());
            (*gif).Image.ColorMap = map;
            if map.is_null() {
                return fail(gif, D_GIF_ERR_NOT_ENOUGH_MEM);
            }
            if !read_color_map(gif, map) {
                GifFreeMapObject(map);
                (*gif).Image.ColorMap = null_mut();
                return fail(gif, D_GIF_ERR_READ_FAILED);
            }
        }
        let (mut s, lzw) = parts(gif);
        lzw.pixel_count = (*gif).Image.Width as u32 as u64 * (*gif).Image.Height as u32 as u64;
        let r = lzw.setup_decompress(&mut s);
        check(gif, r)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetImageDesc(GifFile: *mut GifFileType) -> c_int {
    unsafe {
        let gif = GifFile;
        if !private(gif).readable() {
            return fail(gif, D_GIF_ERR_NOT_READABLE);
        }
        if DGifGetImageHeader(gif) == GIF_ERROR {
            return GIF_ERROR;
        }
        let images = if (*gif).SavedImages.is_null() {
            malloc(size_of::<SavedImage>()) as *mut SavedImage
        } else {
            reallocarray((*gif).SavedImages, (*gif).ImageCount as usize + 1)
        };
        if images.is_null() {
            return fail(gif, D_GIF_ERR_NOT_ENOUGH_MEM);
        }
        (*gif).SavedImages = images;
        let sp = images.add((*gif).ImageCount as usize);
        let mut desc = (*gif).Image;
        if !desc.ColorMap.is_null() {
            desc.ColorMap = GifMakeMapObject((*desc.ColorMap).ColorCount, (*desc.ColorMap).Colors);
            if desc.ColorMap.is_null() {
                return fail(gif, D_GIF_ERR_NOT_ENOUGH_MEM);
            }
        }
        sp.write(SavedImage {
            ImageDesc: desc,
            RasterBits: null_mut(),
            ExtensionBlockCount: 0,
            ExtensionBlocks: null_mut(),
        });
        (*gif).ImageCount += 1;
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetLine(GifFile: *mut GifFileType, Line: *mut GifPixelType, LineLen: c_int) -> c_int {
    unsafe {
        let gif = GifFile;
        if !private(gif).readable() {
            return fail(gif, D_GIF_ERR_NOT_READABLE);
        }
        let len = if LineLen == 0 { (*gif).Image.Width } else { LineLen };
        if len < 0 || (len > 0 && Line.is_null()) {
            return fail(gif, D_GIF_ERR_DATA_TOO_BIG);
        }
        let line = slice_mut(Line, len as usize);
        let (mut s, lzw) = parts(gif);
        let r = lzw.get_line(&mut s, line);
        check(gif, r)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetPixel(GifFile: *mut GifFileType, _Pixel: GifPixelType) -> c_int {
    unsafe {
        if !private(GifFile).readable() {
            return fail(GifFile, D_GIF_ERR_NOT_READABLE);
        }
        let (mut s, lzw) = parts(GifFile);
        let r = lzw.get_pixel(&mut s);
        check(GifFile, r)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetExtension(
    GifFile: *mut GifFileType,
    ExtCode: *mut c_int,
    Extension: *mut *mut GifByteType,
) -> c_int {
    unsafe {
        if !private(GifFile).readable() {
            return fail(GifFile, D_GIF_ERR_NOT_READABLE);
        }
        let mut b = [0u8; 1];
        if !read_bytes(GifFile, &mut b) {
            return fail(GifFile, D_GIF_ERR_READ_FAILED);
        }
        *ExtCode = b[0] as c_int;
        DGifGetExtensionNext(GifFile, Extension)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetExtensionNext(GifFile: *mut GifFileType, Extension: *mut *mut GifByteType) -> c_int {
    unsafe {
        let (mut s, lzw) = parts(GifFile);
        match lzw.get_extension_next(&mut s) {
            Ok(true) => *Extension = lzw.buf.as_mut_ptr(),
            Ok(false) => *Extension = null_mut(),
            Err(code) => return fail(GifFile, code),
        }
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifExtensionToGCB(
    GifExtensionLength: usize,
    GifExtension: *const GifByteType,
    GCB: *mut GraphicsControlBlock,
) -> c_int {
    if GifExtensionLength != 4 {
        return GIF_ERROR;
    }
    unsafe {
        let e = slice(GifExtension, 4);
        let g = &mut *GCB;
        g.DisposalMode = ((e[0] >> 2) & 0x07) as c_int;
        g.UserInputFlag = e[0] & 0x02 != 0;
        g.DelayTime = u16::from_le_bytes([e[1], e[2]]) as c_int;
        g.TransparentColor = if e[0] & 0x01 != 0 { e[3] as c_int } else { NO_TRANSPARENT_COLOR };
    }
    GIF_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifSavedExtensionToGCB(
    GifFile: *mut GifFileType,
    ImageIndex: c_int,
    GCB: *mut GraphicsControlBlock,
) -> c_int {
    unsafe {
        if ImageIndex < 0 || ImageIndex > (*GifFile).ImageCount - 1 {
            return GIF_ERROR;
        }
        *GCB = GraphicsControlBlock {
            DisposalMode: DISPOSAL_UNSPECIFIED,
            UserInputFlag: false,
            DelayTime: 0,
            TransparentColor: NO_TRANSPARENT_COLOR,
        };
        let sp = &*(*GifFile).SavedImages.add(ImageIndex as usize);
        for ep in slice(sp.ExtensionBlocks, sp.ExtensionBlockCount.max(0) as usize) {
            if ep.Function == GRAPHICS_EXT_FUNC_CODE {
                return DGifExtensionToGCB(ep.ByteCount as usize, ep.Bytes, GCB);
            }
        }
        GIF_ERROR
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifCloseFile(GifFile: *mut GifFileType, ErrorCode: *mut c_int) -> c_int {
    unsafe {
        if GifFile.is_null() || (*GifFile).Private.is_null() {
            return GIF_ERROR;
        }
        let gif = &mut *GifFile;
        GifFreeMapObject(gif.Image.ColorMap);
        gif.Image.ColorMap = null_mut();
        GifFreeMapObject(gif.SColorMap);
        gif.SColorMap = null_mut();
        GifFreeSavedImages(GifFile);
        GifFreeExtensions(&mut gif.ExtensionBlockCount, &mut gif.ExtensionBlocks);

        let p = Box::from_raw(gif.Private as *mut Private);
        let readable = p.readable();
        drop(p); // closes the file
        free(GifFile.cast());
        if !readable {
            set_error(ErrorCode, D_GIF_ERR_NOT_READABLE);
            return GIF_ERROR;
        }
        set_error(ErrorCode, D_GIF_SUCCEEDED);
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetCode(
    GifFile: *mut GifFileType,
    CodeSize: *mut c_int,
    CodeBlock: *mut *mut GifByteType,
) -> c_int {
    unsafe {
        if !private(GifFile).readable() {
            return fail(GifFile, D_GIF_ERR_NOT_READABLE);
        }
        *CodeSize = private(GifFile).lzw.bits_per_pixel;
        DGifGetCodeNext(GifFile, CodeBlock)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetCodeNext(GifFile: *mut GifFileType, CodeBlock: *mut *mut GifByteType) -> c_int {
    unsafe {
        let (mut s, lzw) = parts(GifFile);
        match lzw.get_code_next(&mut s) {
            Ok(true) => *CodeBlock = lzw.buf.as_mut_ptr(),
            Ok(false) => *CodeBlock = null_mut(),
            Err(code) => return fail(GifFile, code),
        }
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetLZCodes(GifFile: *mut GifFileType, Code: *mut c_int) -> c_int {
    unsafe {
        if !private(GifFile).readable() {
            return fail(GifFile, D_GIF_ERR_NOT_READABLE);
        }
        let (mut s, lzw) = parts(GifFile);
        match lzw.get_lz_code(&mut s) {
            Ok(c) => {
                *Code = c;
                GIF_OK
            }
            Err(code) => fail(GifFile, code),
        }
    }
}

/// Drop the last (partially read) image after an error in `DGifSlurp`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifDecreaseImageCounter(GifFile: *mut GifFileType) {
    unsafe {
        let gif = &mut *GifFile;
        gif.ImageCount -= 1;
        let sp = &mut *gif.SavedImages.add(gif.ImageCount as usize);
        if !sp.RasterBits.is_null() {
            free(sp.RasterBits.cast());
        }
        if !sp.ImageDesc.ColorMap.is_null() {
            GifFreeMapObject(sp.ImageDesc.ColorMap);
        }
        if gif.ImageCount <= 0 {
            free(gif.SavedImages.cast());
            gif.SavedImages = null_mut();
            gif.ImageCount = 0;
            return;
        }
        let shrunk = reallocarray(gif.SavedImages, gif.ImageCount as usize);
        if !shrunk.is_null() {
            gif.SavedImages = shrunk;
        }
    }
}

/// Read a whole GIF into memory (`GifFile->SavedImages`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifSlurp(GifFile: *mut GifFileType) -> c_int {
    unsafe {
        let gif = GifFile;
        (*gif).ExtensionBlocks = null_mut();
        (*gif).ExtensionBlockCount = 0;
        loop {
            let mut record: GifRecordType = UNDEFINED_RECORD_TYPE;
            if DGifGetRecordType(gif, &mut record) == GIF_ERROR {
                return GIF_ERROR;
            }
            match record {
                IMAGE_DESC_RECORD_TYPE => {
                    if DGifGetImageDesc(gif) == GIF_ERROR {
                        return GIF_ERROR;
                    }
                    if slurp_raster(gif) == GIF_ERROR {
                        DGifDecreaseImageCounter(gif);
                        return GIF_ERROR;
                    }
                    let sp = &mut *(*gif).SavedImages.add((*gif).ImageCount as usize - 1);
                    if !(*gif).ExtensionBlocks.is_null() {
                        sp.ExtensionBlocks = (*gif).ExtensionBlocks;
                        sp.ExtensionBlockCount = (*gif).ExtensionBlockCount;
                        (*gif).ExtensionBlocks = null_mut();
                        (*gif).ExtensionBlockCount = 0;
                    }
                }
                EXTENSION_RECORD_TYPE => {
                    let mut function: c_int = 0;
                    let mut data: *mut GifByteType = null_mut();
                    if DGifGetExtension(gif, &mut function, &mut data) == GIF_ERROR {
                        return GIF_ERROR;
                    }
                    if !data.is_null()
                        && GifAddExtensionBlock(
                            &mut (*gif).ExtensionBlockCount,
                            &mut (*gif).ExtensionBlocks,
                            function,
                            *data as u32,
                            data.add(1),
                        ) == GIF_ERROR
                    {
                        return GIF_ERROR;
                    }
                    loop {
                        if DGifGetExtensionNext(gif, &mut data) == GIF_ERROR {
                            return GIF_ERROR;
                        }
                        if data.is_null() {
                            break;
                        }
                        if GifAddExtensionBlock(
                            &mut (*gif).ExtensionBlockCount,
                            &mut (*gif).ExtensionBlocks,
                            CONTINUE_EXT_FUNC_CODE,
                            *data as u32,
                            data.add(1),
                        ) == GIF_ERROR
                        {
                            return GIF_ERROR;
                        }
                    }
                }
                _ => {}
            }
            if record == TERMINATE_RECORD_TYPE {
                break;
            }
        }
        if (*gif).ImageCount == 0 {
            return fail(gif, D_GIF_ERR_NO_IMAG_DSCR);
        }
        GIF_OK
    }
}

/// Allocate and decode the raster of the image just described.
unsafe fn slurp_raster(gif: *mut GifFileType) -> c_int {
    unsafe {
        let sp = &mut *(*gif).SavedImages.add((*gif).ImageCount as usize - 1);
        let w = sp.ImageDesc.Width;
        let h = sp.ImageDesc.Height;
        if w <= 0 || h <= 0 || w > c_int::MAX / h {
            return GIF_ERROR;
        }
        let (w, h) = (w as usize, h as usize);
        let size = w * h;
        sp.RasterBits = reallocarray(null_mut::<u8>(), size);
        if sp.RasterBits.is_null() {
            return GIF_ERROR;
        }
        let raster = slice_mut(sp.RasterBits, size);
        let (mut s, lzw) = parts(gif);
        let result = if sp.ImageDesc.Interlace {
            let mut r = Ok(());
            'passes: for pass in 0..4 {
                let mut row = INTERLACED_OFFSET[pass];
                while row < h {
                    r = lzw.get_line(&mut s, &mut raster[row * w..(row + 1) * w]);
                    if r.is_err() {
                        break 'passes;
                    }
                    row += INTERLACED_JUMPS[pass];
                }
            }
            r
        } else {
            lzw.get_line(&mut s, raster)
        };
        check(gif, result)
    }
}
