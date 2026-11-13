//! Decode arbitrary bytes and touch every decoded pixel.
#![no_main]

mod common;

use common::*;
use gif::dgif::*;
use gif::types::*;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| unsafe {
    let mut reader = Reader { data, pos: 0 };
    let (g, status) = slurp(&mut reader);
    if g.is_null() {
        return;
    }
    if status == GIF_OK {
        for i in 0..(*g).ImageCount {
            let sp = &*(*g).SavedImages.add(i as usize);
            let sum = raster(sp).iter().fold(0u8, |a, &b| a ^ b);
            core::hint::black_box(sum);
            let mut gcb = GraphicsControlBlock::default();
            DGifSavedExtensionToGCB(g, i, &mut gcb);
        }
    }
    let mut err = 0;
    DGifCloseFile(g, &mut err);
});
