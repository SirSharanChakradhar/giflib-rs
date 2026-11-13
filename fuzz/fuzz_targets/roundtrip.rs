//! Decode -> encode -> decode, and check that every pixel survives.
#![no_main]

mod common;

use common::*;
use core::ffi::c_void;
use gif::gifalloc::*;
use gif::dgif::*;
use gif::egif::*;
use gif::types::*;
use libfuzzer_sys::fuzz_target;

/// The encoder keeps only the bits a pixel can hold with the image's colour map.
unsafe fn pixel_mask(g: *mut GifFileType, sp: &SavedImage) -> u8 {
    unsafe {
        let map = if !sp.ImageDesc.ColorMap.is_null() { sp.ImageDesc.ColorMap } else { (*g).SColorMap };
        let bits = (*map).BitsPerPixel.max(2);
        if bits >= 8 { 0xff } else { (1u8 << bits) - 1 }
    }
}

fuzz_target!(|data: &[u8]| unsafe {
    let mut reader = Reader { data, pos: 0 };
    let (src, status) = slurp(&mut reader);
    if src.is_null() {
        return;
    }
    let mut err = 0;
    if status != GIF_OK {
        DGifCloseFile(src, &mut err);
        return;
    }

    // Copy the decoded GIF into a writer and spew it into memory.
    let mut encoded: Vec<u8> = Vec::new();
    let out = EGifOpen(&mut encoded as *mut Vec<u8> as *mut c_void, Some(write_cb), &mut err);
    assert!(!out.is_null());
    (*out).SWidth = (*src).SWidth;
    (*out).SHeight = (*src).SHeight;
    (*out).SColorResolution = (*src).SColorResolution;
    (*out).SBackGroundColor = (*src).SBackGroundColor;
    if !(*src).SColorMap.is_null() {
        (*out).SColorMap = GifMakeMapObject((*(*src).SColorMap).ColorCount, (*(*src).SColorMap).Colors);
    }
    let mut encodable = true;
    for i in 0..(*src).ImageCount as usize {
        let sp = &*(*src).SavedImages.add(i);
        encodable &= !(sp.ImageDesc.ColorMap.is_null() && (*src).SColorMap.is_null());
        assert!(!GifMakeSavedImage(out, sp).is_null());
    }
    let spewed = EGifSpew(out, &mut err) == GIF_OK;
    if !encodable {
        DGifCloseFile(src, &mut err);
        return;
    }
    assert!(spewed, "encoding a decodable GIF failed: error {err}");

    // Decode our own output and compare.
    let mut reader2 = Reader { data: &encoded, pos: 0 };
    let (back, status2) = slurp(&mut reader2);
    assert!(!back.is_null() && status2 == GIF_OK, "could not decode our own output");
    assert_eq!((*back).ImageCount, (*src).ImageCount);
    for i in 0..(*src).ImageCount as usize {
        let a = &*(*src).SavedImages.add(i);
        let b = &*(*back).SavedImages.add(i);
        assert_eq!(a.ImageDesc.Width, b.ImageDesc.Width);
        assert_eq!(a.ImageDesc.Height, b.ImageDesc.Height);
        let mask = pixel_mask(src, a);
        let ok = raster(a).iter().zip(raster(b)).all(|(&x, &y)| x & mask == y);
        assert!(ok, "pixels changed in image {i}");
    }
    DGifCloseFile(back, &mut err);
    DGifCloseFile(src, &mut err);
});
