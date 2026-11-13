//! Colour-map, extension-block and saved-image helpers (port of `gifalloc.c`).
//!
//! Everything a C caller can see or free is allocated with the C allocator
//! (`malloc`/`calloc`/`realloc`/`free`), exactly like upstream, because C
//! programs are allowed to free these fields themselves.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use crate::types::*;
use core::ffi::{c_int, c_uint, c_void};
use core::mem::size_of;
use core::ptr::{self, null_mut};

unsafe extern "C" {
    pub fn malloc(size: usize) -> *mut c_void;
    pub fn calloc(nmemb: usize, size: usize) -> *mut c_void;
    pub fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    pub fn free(ptr: *mut c_void);
}

/// Slice from a C pointer; tolerates `NULL` when `len == 0`.
pub unsafe fn slice<'a, T>(p: *const T, len: usize) -> &'a [T] {
    if len == 0 || p.is_null() {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(p, len) }
    }
}

/// Mutable slice from a C pointer; tolerates `NULL` when `len == 0`.
pub unsafe fn slice_mut<'a, T>(p: *mut T, len: usize) -> &'a mut [T] {
    if len == 0 || p.is_null() {
        &mut []
    } else {
        unsafe { core::slice::from_raw_parts_mut(p, len) }
    }
}

/// Overflow-checked `realloc`, like OpenBSD's `reallocarray(3)`.
pub unsafe fn reallocarray<T>(p: *mut T, nmemb: usize) -> *mut T {
    match nmemb.checked_mul(size_of::<T>()) {
        Some(bytes) => unsafe { realloc(p.cast(), bytes).cast() },
        None => null_mut(),
    }
}

/// Exported for ABI compatibility with upstream's private helper.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openbsd_reallocarray(
    optr: *mut c_void,
    nmemb: usize,
    size: usize,
) -> *mut c_void {
    match nmemb.checked_mul(size) {
        Some(bytes) => unsafe { realloc(optr, bytes) },
        None => null_mut(),
    }
}

/// Smallest bit-field size that `n` fits in (1..=8, or 9 if it does not fit).
#[unsafe(no_mangle)]
pub extern "C" fn GifBitSize(n: c_int) -> c_int {
    let mut i = 1;
    while i <= 8 {
        if (1 << i) >= n {
            break;
        }
        i += 1;
    }
    i
}

/// Allocate a colour map of `ColorCount` entries (must be a power of two),
/// optionally initialised from `ColorMap`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifMakeMapObject(
    ColorCount: c_int,
    ColorMap: *const GifColorType,
) -> *mut ColorMapObject {
    if ColorCount != (1 << GifBitSize(ColorCount)) {
        return null_mut();
    }
    unsafe {
        let object = malloc(size_of::<ColorMapObject>()) as *mut ColorMapObject;
        if object.is_null() {
            return null_mut();
        }
        let colors =
            calloc(ColorCount as usize, size_of::<GifColorType>()) as *mut GifColorType;
        if colors.is_null() {
            free(object.cast());
            return null_mut();
        }
        if !ColorMap.is_null() {
            ptr::copy_nonoverlapping(ColorMap, colors, ColorCount as usize);
        }
        object.write(ColorMapObject {
            ColorCount,
            BitsPerPixel: GifBitSize(ColorCount),
            SortFlag: false,
            Colors: colors,
        });
        object
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifFreeMapObject(Object: *mut ColorMapObject) {
    if !Object.is_null() {
        unsafe {
            free((*Object).Colors.cast());
            free(Object.cast());
        }
    }
}

/// Union of two colour maps. Colours of `ColorIn2` that are new get fresh
/// slots; `ColorTransIn2` receives the old-index -> union-index translation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifUnionColorMap(
    ColorIn1: *const ColorMapObject,
    ColorIn2: *const ColorMapObject,
    ColorTransIn2: *mut GifPixelType,
) -> *mut ColorMapObject {
    if ColorIn1.is_null() || ColorIn2.is_null() {
        return null_mut();
    }
    unsafe {
        let c1 = &*ColorIn1;
        let c2 = &*ColorIn2;
        let union = GifMakeMapObject(c1.ColorCount.max(c2.ColorCount) * 2, ptr::null());
        if union.is_null() {
            return null_mut();
        }
        let n1 = c1.ColorCount.max(0) as usize;
        let n2 = c2.ColorCount.max(0) as usize;
        let cap = (*union).ColorCount as usize;
        let in1 = slice(c1.Colors, n1);
        let in2 = slice(c2.Colors, n2);
        let out = slice_mut((*union).Colors, cap);
        let trans = slice_mut(ColorTransIn2, n2);

        out[..n1].copy_from_slice(in1);
        let mut crnt_slot = n1;
        // Back down past trailing black slots of map 1 (upstream behaviour).
        while crnt_slot > 0 && in1[crnt_slot - 1] == GifColorType::default() {
            crnt_slot -= 1;
        }
        let mut i = 0;
        while i < n2 && crnt_slot <= 256 {
            match in1.iter().position(|c| *c == in2[i]) {
                Some(j) => trans[i] = j as GifPixelType,
                None => {
                    out[crnt_slot] = in2[i];
                    trans[i] = crnt_slot as GifPixelType;
                    crnt_slot += 1;
                }
            }
            i += 1;
        }
        if crnt_slot > 256 {
            GifFreeMapObject(union);
            return null_mut();
        }
        let new_bits = GifBitSize(crnt_slot as c_int);
        let round_up_to = 1usize << new_bits;
        if round_up_to != cap {
            for c in &mut out[crnt_slot..round_up_to] {
                *c = GifColorType::default();
            }
            if round_up_to < cap {
                let new_map = reallocarray((*union).Colors, round_up_to);
                if new_map.is_null() {
                    GifFreeMapObject(union);
                    return null_mut();
                }
                (*union).Colors = new_map;
            }
        }
        (*union).ColorCount = round_up_to as c_int;
        (*union).BitsPerPixel = new_bits;
        union
    }
}

/// Apply a colour translation table to every pixel of an image.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifApplyTranslation(
    Image: *mut SavedImage,
    Translation: *const GifPixelType,
) {
    unsafe {
        let desc = &(*Image).ImageDesc;
        let size = (desc.Height as usize).wrapping_mul(desc.Width as usize);
        if size == 0 || (*Image).RasterBits.is_null() {
            return;
        }
        let raster = slice_mut((*Image).RasterBits, size);
        // The table is indexed by pixel value; it need not have 256 entries.
        for px in raster {
            *px = *Translation.add(*px as usize);
        }
    }
}

/// Append an extension block, copying `Len` bytes from `ExtData` (if non-null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifAddExtensionBlock(
    ExtensionBlockCount: *mut c_int,
    ExtensionBlocks: *mut *mut ExtensionBlock,
    Function: c_int,
    Len: c_uint,
    ExtData: *mut u8,
) -> c_int {
    unsafe {
        let count = (*ExtensionBlockCount).max(0) as usize;
        let blocks = if (*ExtensionBlocks).is_null() {
            malloc(size_of::<ExtensionBlock>()) as *mut ExtensionBlock
        } else {
            reallocarray(*ExtensionBlocks, count + 1)
        };
        if blocks.is_null() {
            return GIF_ERROR;
        }
        *ExtensionBlocks = blocks;
        let ep = blocks.add(count);
        *ExtensionBlockCount = (count + 1) as c_int;
        let bytes = malloc(Len as usize) as *mut u8;
        ep.write(ExtensionBlock {
            ByteCount: Len as c_int,
            Bytes: bytes,
            Function,
        });
        if bytes.is_null() {
            return GIF_ERROR;
        }
        if !ExtData.is_null() {
            ptr::copy_nonoverlapping(ExtData, bytes, Len as usize);
        }
        GIF_OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifFreeExtensions(
    ExtensionBlockCount: *mut c_int,
    ExtensionBlocks: *mut *mut ExtensionBlock,
) {
    unsafe {
        let blocks = *ExtensionBlocks;
        if blocks.is_null() {
            return;
        }
        for k in 0..(*ExtensionBlockCount).max(0) as usize {
            free((*blocks.add(k)).Bytes.cast());
        }
        free(blocks.cast());
        *ExtensionBlocks = null_mut();
        *ExtensionBlockCount = 0;
    }
}

/// Free the last image in `GifFile->SavedImages` (upstream private helper).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FreeLastSavedImage(GifFile: *mut GifFileType) {
    unsafe {
        if GifFile.is_null() || (*GifFile).SavedImages.is_null() {
            return;
        }
        (*GifFile).ImageCount -= 1;
        let sp = (*GifFile).SavedImages.add((*GifFile).ImageCount as usize);
        free_saved_image_contents(sp);
    }
}

unsafe fn free_saved_image_contents(sp: *mut SavedImage) {
    unsafe {
        if !(*sp).ImageDesc.ColorMap.is_null() {
            GifFreeMapObject((*sp).ImageDesc.ColorMap);
            (*sp).ImageDesc.ColorMap = null_mut();
        }
        if !(*sp).RasterBits.is_null() {
            free((*sp).RasterBits.cast());
        }
        GifFreeExtensions(&mut (*sp).ExtensionBlockCount, &mut (*sp).ExtensionBlocks);
    }
}

/// Append an image to `GifFile->SavedImages`, deep-copying `CopyFrom` if given.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifMakeSavedImage(
    GifFile: *mut GifFileType,
    CopyFrom: *const SavedImage,
) -> *mut SavedImage {
    unsafe {
        let gif = &mut *GifFile;
        let images = if gif.SavedImages.is_null() {
            malloc(size_of::<SavedImage>()) as *mut SavedImage
        } else {
            reallocarray(gif.SavedImages, gif.ImageCount as usize + 1)
        };
        if images.is_null() {
            return null_mut();
        }
        gif.SavedImages = images;
        let sp = images.add(gif.ImageCount as usize);
        gif.ImageCount += 1;

        if CopyFrom.is_null() {
            ptr::write_bytes(sp, 0, 1);
            return sp;
        }
        let src = &*CopyFrom;
        // Start from a copy, with every heap pointer cleared so that a
        // partial failure never frees the caller's data.
        sp.write(SavedImage {
            ImageDesc: GifImageDesc {
                ColorMap: null_mut(),
                ..src.ImageDesc
            },
            RasterBits: null_mut(),
            ExtensionBlockCount: 0,
            ExtensionBlocks: null_mut(),
        });
        if !src.ImageDesc.ColorMap.is_null() {
            let cm = &*src.ImageDesc.ColorMap;
            (*sp).ImageDesc.ColorMap = GifMakeMapObject(cm.ColorCount, cm.Colors);
            if (*sp).ImageDesc.ColorMap.is_null() {
                FreeLastSavedImage(GifFile);
                return null_mut();
            }
        }
        let size = (src.ImageDesc.Height as usize).wrapping_mul(src.ImageDesc.Width as usize);
        (*sp).RasterBits = reallocarray(null_mut::<u8>(), size);
        if (*sp).RasterBits.is_null() {
            FreeLastSavedImage(GifFile);
            return null_mut();
        }
        ptr::copy_nonoverlapping(src.RasterBits, (*sp).RasterBits, size);
        if !src.ExtensionBlocks.is_null() {
            let n = src.ExtensionBlockCount.max(0) as usize;
            let blocks = calloc(n, size_of::<ExtensionBlock>()) as *mut ExtensionBlock;
            if blocks.is_null() {
                FreeLastSavedImage(GifFile);
                return null_mut();
            }
            (*sp).ExtensionBlocks = blocks;
            (*sp).ExtensionBlockCount = n as c_int;
            for k in 0..n {
                let s = &*src.ExtensionBlocks.add(k);
                let d = &mut *blocks.add(k);
                d.Function = s.Function;
                d.ByteCount = s.ByteCount;
                if s.ByteCount > 0 {
                    d.Bytes = malloc(s.ByteCount as usize) as *mut u8;
                    if d.Bytes.is_null() {
                        FreeLastSavedImage(GifFile);
                        return null_mut();
                    }
                    ptr::copy_nonoverlapping(s.Bytes, d.Bytes, s.ByteCount as usize);
                }
            }
        }
        sp
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifFreeSavedImages(GifFile: *mut GifFileType) {
    unsafe {
        if GifFile.is_null() || (*GifFile).SavedImages.is_null() {
            return;
        }
        for k in 0..(*GifFile).ImageCount.max(0) as usize {
            free_saved_image_contents((*GifFile).SavedImages.add(k));
        }
        free((*GifFile).SavedImages.cast());
        (*GifFile).SavedImages = null_mut();
    }
}
