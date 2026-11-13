//! GIF encoding: the C API of `egif_lib.c`.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use crate::alloc::*;
use crate::private::*;
use crate::types::*;
use core::ffi::{c_char, c_int, c_void, CStr};
use core::mem::size_of;
use core::ptr::null_mut;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::BufWriter;
use std::os::fd::FromRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;

const CODE_MASK: [u8; 9] = [0x00, 0x01, 0x03, 0x07, 0x0f, 0x1f, 0x3f, 0x7f, 0xff];
const INTERLACED_OFFSET: [usize; 4] = [0, 4, 2, 1];
const INTERLACED_JUMPS: [usize; 4] = [8, 8, 4, 2];

unsafe fn set_error(out: *mut c_int, code: c_int) {
    if !out.is_null() {
        unsafe { *out = code };
    }
}

unsafe fn fail(gif: *mut GifFileType, code: c_int) -> c_int {
    unsafe { (*gif).Error = code };
    GIF_ERROR
}

/// Write raw bytes; returns the count the stream reported.
unsafe fn write(gif: *mut GifFileType, buf: &[u8]) -> i64 {
    let (mut s, _) = unsafe { parts(gif) };
    s.write(buf)
}

unsafe fn put_word(gif: *mut GifFileType, word: c_int) {
    unsafe { write(gif, &(word as u16).to_le_bytes()) };
}

unsafe fn writeable(gif: *mut GifFileType) -> bool {
    unsafe { private(gif).writeable() }
}

unsafe fn open_writer(io: Io, user: *mut c_void, error: *mut c_int) -> *mut GifFileType {
    unsafe {
        let gif = calloc(1, size_of::<GifFileType>()) as *mut GifFileType;
        if gif.is_null() {
            set_error(error, E_GIF_ERR_NOT_ENOUGH_MEM);
            return null_mut();
        }
        let mut p = Private::new(io, FILE_STATE_WRITE);
        p.lzw.hash = Some(crate::lzw::HashTable::new());
        (*gif).Private = Box::into_raw(p).cast();
        (*gif).UserData = user;
        (*gif).Error = 0;
        gif
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifOpenFileName(
    FileName: *const c_char,
    TestExistence: bool,
    Error: *mut c_int,
) -> *mut GifFileType {
    unsafe {
        if FileName.is_null() {
            set_error(Error, E_GIF_ERR_OPEN_FAILED);
            return null_mut();
        }
        let path = OsStr::from_bytes(CStr::from_ptr(FileName).to_bytes());
        let mut opts = OpenOptions::new();
        opts.write(true).mode(0o600);
        if TestExistence {
            opts.create_new(true);
        } else {
            opts.create(true).truncate(true);
        }
        match opts.open(path) {
            Ok(f) => open_writer(Io::WriteFile(BufWriter::new(f)), null_mut(), Error),
            Err(_) => {
                set_error(Error, E_GIF_ERR_OPEN_FAILED);
                null_mut()
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifOpenFileHandle(FileHandle: c_int, Error: *mut c_int) -> *mut GifFileType {
    unsafe {
        let io = if FileHandle < 0 {
            Io::None
        } else {
            Io::WriteFile(BufWriter::new(File::from_raw_fd(FileHandle)))
        };
        open_writer(io, null_mut(), Error)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifOpen(userData: *mut c_void, writeFunc: OutputFunc, Error: *mut c_int) -> *mut GifFileType {
    unsafe { open_writer(Io::WriteFunc(writeFunc), userData, Error) }
}

fn is_gif89_function(f: c_int) -> bool {
    matches!(
        f,
        COMMENT_EXT_FUNC_CODE | GRAPHICS_EXT_FUNC_CODE | PLAINTEXT_EXT_FUNC_CODE | APPLICATION_EXT_FUNC_CODE
    )
}

/// The version that will be written: GIF89a if any GIF89 extension is present.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifGetGifVersion(GifFile: *mut GifFileType) -> *const c_char {
    unsafe {
        let gif = &*GifFile;
        let mut gif89 = false;
        for sp in slice(gif.SavedImages, gif.ImageCount.max(0) as usize) {
            for ep in slice(sp.ExtensionBlocks, sp.ExtensionBlockCount.max(0) as usize) {
                gif89 |= is_gif89_function(ep.Function);
            }
        }
        for ep in slice(gif.ExtensionBlocks, gif.ExtensionBlockCount.max(0) as usize) {
            gif89 |= is_gif89_function(ep.Function);
        }
        let p = private(GifFile);
        p.gif89 |= gif89;
        if p.gif89 { GIF89_STAMP.as_ptr() } else { GIF87_STAMP.as_ptr() }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifSetGifVersion(GifFile: *mut GifFileType, gif89: bool) {
    unsafe { private(GifFile).gif89 = gif89 };
}

unsafe fn write_color_map(gif: *mut GifFileType, map: *const ColorMapObject) -> bool {
    unsafe {
        for c in slice((*map).Colors, (*map).ColorCount.max(0) as usize) {
            if write(gif, &[c.Red, c.Green, c.Blue]) != 3 {
                return false;
            }
        }
        true
    }
}

/// Replace `*slot` with a copy of `map` (or NULL). `false` on allocation failure.
unsafe fn replace_map(slot: *mut *mut ColorMapObject, map: *const ColorMapObject) -> bool {
    unsafe {
        if map.is_null() {
            GifFreeMapObject(*slot);
            *slot = null_mut();
        } else if map != *slot as *const _ {
            GifFreeMapObject(*slot);
            *slot = GifMakeMapObject((*map).ColorCount, (*map).Colors);
            if (*slot).is_null() {
                return false;
            }
        }
        true
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutScreenDesc(
    GifFile: *mut GifFileType,
    Width: c_int,
    Height: c_int,
    ColorRes: c_int,
    BackGround: c_int,
    ColorMap: *const ColorMapObject,
) -> c_int {
    unsafe {
        let gif = GifFile;
        if private(gif).file_state & FILE_STATE_SCREEN != 0 {
            return fail(gif, E_GIF_ERR_HAS_SCRN_DSCR);
        }
        if !writeable(gif) {
            return fail(gif, E_GIF_ERR_NOT_WRITEABLE);
        }
        let version = CStr::from_ptr(EGifGetGifVersion(gif)).to_bytes();
        if write(gif, version) != version.len() as i64 {
            return fail(gif, E_GIF_ERR_WRITE_FAILED);
        }
        (*gif).SWidth = Width;
        (*gif).SHeight = Height;
        (*gif).SColorResolution = ColorRes;
        (*gif).SBackGroundColor = BackGround;
        if !replace_map(&mut (*gif).SColorMap, ColorMap) {
            return fail(gif, E_GIF_ERR_NOT_ENOUGH_MEM);
        }
        put_word(gif, Width);
        put_word(gif, Height);
        let mut flags = if ColorMap.is_null() {
            ((ColorRes - 1) << 4) | 0x07
        } else {
            0x80 | ((ColorRes - 1) << 4) | ((*ColorMap).BitsPerPixel - 1)
        } as u8;
        if !ColorMap.is_null() && (*ColorMap).SortFlag {
            flags |= 0x08;
        }
        write(gif, &[flags, BackGround as u8, (*gif).AspectByte]);
        if !ColorMap.is_null() && !write_color_map(gif, ColorMap) {
            return fail(gif, E_GIF_ERR_WRITE_FAILED);
        }
        private(gif).file_state |= FILE_STATE_SCREEN;
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutImageDesc(
    GifFile: *mut GifFileType,
    Left: c_int,
    Top: c_int,
    Width: c_int,
    Height: c_int,
    Interlace: bool,
    ColorMap: *const ColorMapObject,
) -> c_int {
    unsafe {
        let gif = GifFile;
        let p = private(gif);
        if p.file_state & FILE_STATE_IMAGE != 0 && p.lzw.pixel_count > 0xffff0000 {
            return fail(gif, E_GIF_ERR_HAS_IMAG_DSCR);
        }
        if !writeable(gif) {
            return fail(gif, E_GIF_ERR_NOT_WRITEABLE);
        }
        let img = &mut (*gif).Image;
        img.Left = Left;
        img.Top = Top;
        img.Width = Width;
        img.Height = Height;
        img.Interlace = Interlace;
        if !replace_map(&mut img.ColorMap, ColorMap) {
            return fail(gif, E_GIF_ERR_NOT_ENOUGH_MEM);
        }
        write(gif, &[0x2c]);
        put_word(gif, Left);
        put_word(gif, Top);
        put_word(gif, Width);
        put_word(gif, Height);
        let flags = if ColorMap.is_null() {
            if Interlace { 0x40 } else { 0 }
        } else {
            0x80 | if Interlace { 0x40 } else { 0 } | ((*ColorMap).BitsPerPixel - 1)
        } as u8;
        write(gif, &[flags]);
        if !ColorMap.is_null() && !write_color_map(gif, ColorMap) {
            return fail(gif, E_GIF_ERR_WRITE_FAILED);
        }
        if (*gif).SColorMap.is_null() && (*gif).Image.ColorMap.is_null() {
            return fail(gif, E_GIF_ERR_NO_COLOR_MAP);
        }
        let bits = if !(*gif).Image.ColorMap.is_null() {
            (*(*gif).Image.ColorMap).BitsPerPixel
        } else {
            (*(*gif).SColorMap).BitsPerPixel
        };
        private(gif).file_state |= FILE_STATE_IMAGE;
        let (mut s, lzw) = parts(gif);
        lzw.pixel_count = Width as u32 as u64 * Height as u32 as u64;
        // Upstream ignores a failure here but records it on the handle.
        if let Err(code) = lzw.setup_compress(&mut s, bits) {
            (*gif).Error = code;
        }
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutLine(GifFile: *mut GifFileType, Line: *mut GifPixelType, LineLen: c_int) -> c_int {
    unsafe {
        let gif = GifFile;
        if !writeable(gif) {
            return fail(gif, E_GIF_ERR_NOT_WRITEABLE);
        }
        let len = if LineLen == 0 { (*gif).Image.Width } else { LineLen };
        let (mut s, lzw) = parts(gif);
        if len < 0 || lzw.pixel_count < len as u64 || (len > 0 && Line.is_null()) {
            return fail(gif, E_GIF_ERR_DATA_TOO_BIG);
        }
        lzw.pixel_count -= len as u64;
        let mask = *CODE_MASK.get(lzw.bits_per_pixel as usize).unwrap_or(&0xff);
        let line = slice_mut(Line, len as usize);
        // Upstream masks the caller's buffer in place.
        for px in line.iter_mut() {
            *px &= mask;
        }
        match lzw.compress_line(&mut s, line) {
            Ok(()) => GIF_OK,
            Err(code) => fail(gif, code),
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutPixel(GifFile: *mut GifFileType, Pixel: GifPixelType) -> c_int {
    unsafe {
        let gif = GifFile;
        if !writeable(gif) {
            return fail(gif, E_GIF_ERR_NOT_WRITEABLE);
        }
        let (mut s, lzw) = parts(gif);
        if lzw.pixel_count == 0 {
            return fail(gif, E_GIF_ERR_DATA_TOO_BIG);
        }
        lzw.pixel_count -= 1;
        let px = Pixel & *CODE_MASK.get(lzw.bits_per_pixel as usize).unwrap_or(&0xff);
        match lzw.compress_line(&mut s, &[px]) {
            Ok(()) => GIF_OK,
            Err(code) => fail(gif, code),
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutComment(GifFile: *mut GifFileType, Comment: *const c_char) -> c_int {
    unsafe {
        let text = CStr::from_ptr(Comment).to_bytes();
        if text.len() <= 255 {
            return EGifPutExtension(GifFile, COMMENT_EXT_FUNC_CODE, text.len() as c_int, text.as_ptr().cast());
        }
        if EGifPutExtensionLeader(GifFile, COMMENT_EXT_FUNC_CODE) == GIF_ERROR {
            return GIF_ERROR;
        }
        for chunk in text.chunks(255) {
            if EGifPutExtensionBlock(GifFile, chunk.len() as c_int, chunk.as_ptr().cast()) == GIF_ERROR {
                return GIF_ERROR;
            }
        }
        EGifPutExtensionTrailer(GifFile)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutExtensionLeader(GifFile: *mut GifFileType, ExtCode: c_int) -> c_int {
    unsafe {
        if !writeable(GifFile) {
            return fail(GifFile, E_GIF_ERR_NOT_WRITEABLE);
        }
        write(GifFile, &[0x21, ExtCode as u8]);
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutExtensionBlock(
    GifFile: *mut GifFileType,
    ExtLen: c_int,
    Extension: *const c_void,
) -> c_int {
    unsafe {
        if !writeable(GifFile) {
            return fail(GifFile, E_GIF_ERR_NOT_WRITEABLE);
        }
        write(GifFile, &[ExtLen as u8]);
        write(GifFile, slice(Extension as *const u8, ExtLen.max(0) as usize));
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutExtensionTrailer(GifFile: *mut GifFileType) -> c_int {
    unsafe {
        if !writeable(GifFile) {
            return fail(GifFile, E_GIF_ERR_NOT_WRITEABLE);
        }
        write(GifFile, &[0]);
        GIF_OK
    }
}

/// Write a single-sub-block extension.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutExtension(
    GifFile: *mut GifFileType,
    ExtCode: c_int,
    ExtLen: c_int,
    Extension: *const c_void,
) -> c_int {
    unsafe {
        if !writeable(GifFile) {
            return fail(GifFile, E_GIF_ERR_NOT_WRITEABLE);
        }
        if ExtCode == 0 {
            write(GifFile, &[ExtLen as u8]);
        } else {
            write(GifFile, &[0x21, ExtCode as u8, ExtLen as u8]);
        }
        write(GifFile, slice(Extension as *const u8, ExtLen.max(0) as usize));
        write(GifFile, &[0]);
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifGCBToExtension(GCB: *const GraphicsControlBlock, GifExtension: *mut GifByteType) -> usize {
    unsafe {
        let g = &*GCB;
        let mut flags = 0u8;
        if g.TransparentColor != NO_TRANSPARENT_COLOR {
            flags |= 0x01;
        }
        if g.UserInputFlag {
            flags |= 0x02;
        }
        flags |= ((g.DisposalMode & 0x07) << 2) as u8;
        let delay = (g.DelayTime as u16).to_le_bytes();
        let out = slice_mut(GifExtension, 4);
        out.copy_from_slice(&[flags, delay[0], delay[1], g.TransparentColor as u8]);
        4
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifGCBToSavedExtension(
    GCB: *const GraphicsControlBlock,
    GifFile: *mut GifFileType,
    ImageIndex: c_int,
) -> c_int {
    unsafe {
        if ImageIndex < 0 || ImageIndex > (*GifFile).ImageCount - 1 {
            return GIF_ERROR;
        }
        let sp = &mut *(*GifFile).SavedImages.add(ImageIndex as usize);
        for ep in slice(sp.ExtensionBlocks, sp.ExtensionBlockCount.max(0) as usize) {
            if ep.Function == GRAPHICS_EXT_FUNC_CODE {
                if ep.ByteCount < 4 {
                    return GIF_ERROR;
                }
                EGifGCBToExtension(GCB, ep.Bytes);
                return GIF_OK;
            }
        }
        let mut buf = [0u8; 4];
        let len = EGifGCBToExtension(GCB, buf.as_mut_ptr());
        GifAddExtensionBlock(
            &mut sp.ExtensionBlockCount,
            &mut sp.ExtensionBlocks,
            GRAPHICS_EXT_FUNC_CODE,
            len as u32,
            buf.as_mut_ptr(),
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutCode(GifFile: *mut GifFileType, _CodeSize: c_int, CodeBlock: *const GifByteType) -> c_int {
    unsafe {
        if !writeable(GifFile) {
            return fail(GifFile, E_GIF_ERR_NOT_WRITEABLE);
        }
        EGifPutCodeNext(GifFile, CodeBlock)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutCodeNext(GifFile: *mut GifFileType, CodeBlock: *const GifByteType) -> c_int {
    unsafe {
        if !CodeBlock.is_null() {
            let block = slice(CodeBlock, *CodeBlock as usize + 1);
            if write(GifFile, block) != block.len() as i64 {
                return fail(GifFile, E_GIF_ERR_WRITE_FAILED);
            }
        } else {
            if write(GifFile, &[0]) != 1 {
                return fail(GifFile, E_GIF_ERR_WRITE_FAILED);
            }
            private(GifFile).lzw.pixel_count = 0;
        }
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifCloseFile(GifFile: *mut GifFileType, ErrorCode: *mut c_int) -> c_int {
    unsafe {
        if GifFile.is_null() || (*GifFile).Private.is_null() {
            return GIF_ERROR;
        }
        if !writeable(GifFile) {
            set_error(ErrorCode, E_GIF_ERR_NOT_WRITEABLE);
            drop(Box::from_raw((*GifFile).Private as *mut Private));
            free(GifFile.cast());
            return GIF_ERROR;
        }
        write(GifFile, &[0x3b]);
        GifFreeMapObject((*GifFile).Image.ColorMap);
        (*GifFile).Image.ColorMap = null_mut();
        GifFreeMapObject((*GifFile).SColorMap);
        (*GifFile).SColorMap = null_mut();

        let p = Box::from_raw((*GifFile).Private as *mut Private);
        let closed_ok = match p.io {
            // Flushing the buffer is where a full disk shows up (C: fclose).
            Io::WriteFile(w) => w.into_inner().is_ok(),
            _ => true,
        };
        free(GifFile.cast());
        if !closed_ok {
            set_error(ErrorCode, E_GIF_ERR_CLOSE_FAILED);
            return GIF_ERROR;
        }
        set_error(ErrorCode, E_GIF_SUCCEEDED);
        GIF_OK
    }
}

unsafe fn write_extensions(gif: *mut GifFileType, blocks: *mut ExtensionBlock, count: c_int) -> c_int {
    unsafe {
        let blocks = slice(blocks, count.max(0) as usize);
        for (j, ep) in blocks.iter().enumerate() {
            if ep.Function != CONTINUE_EXT_FUNC_CODE && EGifPutExtensionLeader(gif, ep.Function) == GIF_ERROR {
                return GIF_ERROR;
            }
            if EGifPutExtensionBlock(gif, ep.ByteCount, ep.Bytes.cast()) == GIF_ERROR {
                return GIF_ERROR;
            }
            let last_in_run = blocks.get(j + 1).is_none_or(|next| next.Function != CONTINUE_EXT_FUNC_CODE);
            if last_in_run && EGifPutExtensionTrailer(gif) == GIF_ERROR {
                return GIF_ERROR;
            }
        }
        GIF_OK
    }
}

unsafe fn spew_body(gif: *mut GifFileType) -> c_int {
    unsafe {
        let g = &mut *gif;
        if EGifPutScreenDesc(gif, g.SWidth, g.SHeight, g.SColorResolution, g.SBackGroundColor, g.SColorMap)
            == GIF_ERROR
        {
            return GIF_ERROR;
        }
        for i in 0..g.ImageCount.max(0) as usize {
            let sp = *g.SavedImages.add(i);
            // Images can be deleted by nulling their raster.
            if sp.RasterBits.is_null() {
                continue;
            }
            if write_extensions(gif, sp.ExtensionBlocks, sp.ExtensionBlockCount) == GIF_ERROR {
                return GIF_ERROR;
            }
            let d = sp.ImageDesc;
            if EGifPutImageDesc(gif, d.Left, d.Top, d.Width, d.Height, d.Interlace, d.ColorMap) == GIF_ERROR {
                return GIF_ERROR;
            }
            let (w, h) = (d.Width.max(0) as usize, d.Height.max(0) as usize);
            let put_row = |row: usize| EGifPutLine(gif, sp.RasterBits.add(row * w), d.Width);
            if d.Interlace {
                for pass in 0..4 {
                    let mut row = INTERLACED_OFFSET[pass];
                    while row < h {
                        if put_row(row) == GIF_ERROR {
                            return GIF_ERROR;
                        }
                        row += INTERLACED_JUMPS[pass];
                    }
                }
            } else {
                for row in 0..h {
                    if put_row(row) == GIF_ERROR {
                        return GIF_ERROR;
                    }
                }
            }
        }
        write_extensions(gif, g.ExtensionBlocks, g.ExtensionBlockCount)
    }
}

/// Write an in-memory GIF (as built by `DGifSlurp`) and close the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifSpew(GifFileOut: *mut GifFileType, ErrorCode: *mut c_int) -> c_int {
    unsafe {
        let mut status = spew_body(GifFileOut);
        let mut err = if status == GIF_OK { E_GIF_SUCCEEDED } else { (*GifFileOut).Error };
        GifFreeSavedImages(GifFileOut);
        GifFreeExtensions(&mut (*GifFileOut).ExtensionBlockCount, &mut (*GifFileOut).ExtensionBlocks);
        let mut close_error = E_GIF_SUCCEEDED;
        if EGifCloseFile(GifFileOut, &mut close_error) == GIF_ERROR {
            status = GIF_ERROR;
            if err == E_GIF_SUCCEEDED {
                err = close_error;
            }
        }
        set_error(ErrorCode, if status == GIF_OK { E_GIF_SUCCEEDED } else { err });
        status
    }
}
