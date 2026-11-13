//! Text and box drawing onto saved images (port of `gif_font.c`).

#![allow(non_snake_case, clippy::missing_safety_doc)]

use crate::gifalloc::slice_mut;
use crate::font_table::GifAsciiTable8x8;
use crate::types::*;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, CStr};

const FONT_W: i64 = 8;
const FONT_H: i64 = 8;

/// A clipped view of an image's raster.
struct Canvas<'a> {
    px: &'a mut [u8],
    w: usize,
    h: usize,
}

impl Canvas<'_> {
    unsafe fn of<'a>(image: *mut SavedImage) -> Option<Canvas<'a>> {
        unsafe {
            if image.is_null() || (*image).RasterBits.is_null() {
                return None;
            }
            let d = &(*image).ImageDesc;
            if d.Width <= 0 || d.Height <= 0 {
                return None;
            }
            let (w, h) = (d.Width as usize, d.Height as usize);
            let n = w.checked_mul(h)?;
            Some(Canvas { px: slice_mut((*image).RasterBits, n), w, h })
        }
    }

    fn put(&mut self, x: i64, y: i64, color: u8) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = color;
        }
    }

    fn text(&mut self, x: i64, y: i64, text: &[u8], color: u8) {
        for row in 0..FONT_H {
            for (i, &ch) in text.iter().enumerate() {
                if ch >= 128 {
                    continue;
                }
                let glyph = GifAsciiTable8x8[ch as usize][row as usize];
                let gx = x + i as i64 * FONT_W;
                for j in 0..FONT_W {
                    if glyph & (1 << (FONT_W - 1 - j)) != 0 {
                        self.put(gx + j, y + row, color);
                    }
                }
            }
        }
    }

    fn outline(&mut self, x: i64, y: i64, w: i64, d: i64, color: u8) {
        if w < 0 || d < 0 {
            return;
        }
        for j in 0..=w {
            self.put(x + j, y, color);
            self.put(x + j, y + d, color);
        }
        for j in 0..=d {
            self.put(x, y + j, color);
            self.put(x + w, y + j, color);
        }
    }

    fn fill(&mut self, x: i64, y: i64, w: i64, d: i64, color: u8) {
        if w <= 0 || d <= 0 {
            return;
        }
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.w as i64);
        let y1 = (y + d).min(self.h as i64);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for row in y0..y1 {
            let start = row as usize * self.w;
            self.px[start + x0 as usize..start + x1 as usize].fill(color);
        }
    }
}

unsafe fn c_bytes<'a>(s: *const c_char) -> Option<&'a [u8]> {
    if s.is_null() { None } else { Some(unsafe { CStr::from_ptr(s) }.to_bytes()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifDrawText8x8(Image: *mut SavedImage, x: c_int, y: c_int, legend: *const c_char, color: c_int) {
    unsafe {
        if let (Some(mut c), Some(text)) = (Canvas::of(Image), c_bytes(legend)) {
            c.text(x as i64, y as i64, text, color as u8);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifDrawBox(Image: *mut SavedImage, x: c_int, y: c_int, w: c_int, d: c_int, color: c_int) {
    unsafe {
        if let Some(mut c) = Canvas::of(Image) {
            c.outline(x as i64, y as i64, w as i64, d as i64, color as u8);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifDrawRectangle(Image: *mut SavedImage, x: c_int, y: c_int, w: c_int, d: c_int, color: c_int) {
    unsafe {
        if let Some(mut c) = Canvas::of(Image) {
            c.fill(x as i64, y as i64, w as i64, d as i64, color as u8);
        }
    }
}

/// Draw multi-line text in a filled, outlined box. A line starting with a
/// tab is centred.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifDrawBoxedText8x8(
    Image: *mut SavedImage,
    x: c_int,
    y: c_int,
    legend: *const c_char,
    border: c_int,
    bg: c_int,
    fg: c_int,
) {
    let Some(text) = (unsafe { c_bytes(legend) }) else { return };
    if border < 0 {
        return;
    }
    let lines = split_lines(text);
    // Box size, measured as upstream does: every '\r' and '\n' counts as a
    // line break here, even though "\r\n" is drawn as a single break.
    let is_eol = |c: &u8| *c == b'\r' || *c == b'\n';
    let text_width =
        text.split(is_eol).map(|l| l.iter().filter(|&&c| c != b'\t').count()).max().unwrap_or(0) as i64;
    let line_count = text.iter().filter(|c| is_eol(c)).count() as i64 + 1;
    let border = border as i64;
    let (x, y) = (x as i64, y as i64);
    let box_w = (2 * border + text_width * FONT_W).min(c_int::MAX as i64);
    let box_h = (2 * border + line_count * FONT_H).min(c_int::MAX as i64);

    let Some(mut c) = (unsafe { Canvas::of(Image) }) else { return };
    c.fill(x + 1, y + 1, box_w - 1, box_h - 1, bg as u8);
    for (i, line) in lines.iter().enumerate() {
        let (line, lead) = match line.split_first() {
            Some((b'\t', rest)) => (rest, (text_width - rest.len() as i64).max(0) / 2),
            _ => (*line, 0),
        };
        c.text(x + border + lead * FONT_W, y + border + FONT_H * i as i64, line, fg as u8);
    }
    c.outline(x, y, box_w, box_h, fg as u8);
}

/// Split on `\r`, `\n`, `\r\n` or `\n\r`, as upstream does.
fn split_lines(text: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut rest = text;
    loop {
        match rest.iter().position(|&c| c == b'\r' || c == b'\n') {
            Some(p) => {
                lines.push(&rest[..p]);
                let mut next = p + 1;
                if let Some(&n) = rest.get(next) {
                    if (rest[p] == b'\r' && n == b'\n') || (rest[p] == b'\n' && n == b'\r') {
                        next += 1;
                    }
                }
                rest = &rest[next..];
            }
            None => {
                lines.push(rest);
                return lines;
            }
        }
    }
}
