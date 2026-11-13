//! Median-cut colour quantization (port of `quantize.c`).
//!
//! The algorithm is a safe-Rust translation; linked lists use indices.
//! Output is identical to upstream, including its integer rounding.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use crate::alloc::{slice, slice_mut};
use crate::types::*;
use core::ffi::{c_int, c_uint};

const COLOR_ARRAY_SIZE: usize = 32768;
const BITS_PER_PRIM_COLOR: u32 = 5;
const MAX_PRIM_COLOR: usize = 0x1f;
const NONE: usize = usize::MAX;

#[derive(Clone, Copy)]
struct QuantizedColor {
    rgb: [u8; 3],
    new_index: u8,
    count: i64,
    next: usize,
}

#[derive(Clone, Copy)]
struct Subdivision {
    rgb_min: [u8; 3],
    rgb_width: [u8; 3],
    num_entries: u32,
    count: u64,
    head: usize,
}

fn index_of(r: u8, g: u8, b: u8) -> usize {
    let s = 8 - BITS_PER_PRIM_COLOR;
    (((r >> s) as usize) << (2 * BITS_PER_PRIM_COLOR)) + (((g >> s) as usize) << BITS_PER_PRIM_COLOR) + (b >> s) as usize
}

/// Quantize `red/green/blue` (one byte per pixel each) to at most
/// `map_size` colours. Returns the number of colours used.
pub fn quantize(
    red: &[u8],
    green: &[u8],
    blue: &[u8],
    map_size: usize,
    output: &mut [u8],
    color_map: &mut [GifColorType],
) -> Option<usize> {
    let mut colors: Vec<QuantizedColor> = (0..COLOR_ARRAY_SIZE)
        .map(|i| QuantizedColor {
            rgb: [(i >> 10) as u8, ((i >> 5) & MAX_PRIM_COLOR) as u8, (i & MAX_PRIM_COLOR) as u8],
            new_index: 0,
            count: 0,
            next: NONE,
        })
        .collect();
    for k in 0..output.len() {
        colors[index_of(red[k], green[k], blue[k])].count += 1;
    }

    let mut subdiv = [Subdivision { rgb_min: [0; 3], rgb_width: [255; 3], num_entries: 0, count: 0, head: NONE }; 256];

    // Chain the colours that actually occur.
    let first = colors.iter().position(|c| c.count > 0)?; // empty image: upstream reads out of bounds
    subdiv[0].head = first;
    let mut tail = first;
    let mut entries = 1;
    for i in first + 1..COLOR_ARRAY_SIZE {
        if colors[i].count > 0 {
            colors[tail].next = i;
            tail = i;
            entries += 1;
        }
    }
    colors[tail].next = NONE;
    subdiv[0].num_entries = entries;
    subdiv[0].count = output.len() as u64;

    let new_size = subdivide(&mut colors, &mut subdiv, map_size);

    for c in color_map.iter_mut().take(map_size).skip(new_size) {
        *c = GifColorType::default();
    }
    // Each output colour is the average of its member colours.
    for (i, sub) in subdiv.iter().enumerate().take(new_size) {
        let n = sub.num_entries as i64;
        if n == 0 {
            continue;
        }
        let (mut r, mut g, mut b) = (0i64, 0i64, 0i64);
        let mut q = sub.head;
        while q != NONE {
            colors[q].new_index = i as u8;
            r += colors[q].rgb[0] as i64;
            g += colors[q].rgb[1] as i64;
            b += colors[q].rgb[2] as i64;
            q = colors[q].next;
        }
        let s = 8 - BITS_PER_PRIM_COLOR;
        color_map[i] = GifColorType { Red: ((r << s) / n) as u8, Green: ((g << s) / n) as u8, Blue: ((b << s) / n) as u8 };
    }
    for k in 0..output.len() {
        output[k] = colors[index_of(red[k], green[k], blue[k])].new_index;
    }
    Some(new_size)
}

/// Median-cut until `map_size` boxes exist or no box can be split.
fn subdivide(colors: &mut [QuantizedColor], subdiv: &mut [Subdivision; 256], map_size: usize) -> usize {
    let mut new_size = 1usize;
    while map_size > new_size {
        // The box with the widest side (that has more than one colour).
        let mut max_size = -1i32;
        let mut index = 0;
        let mut axis = 0;
        for (i, sub) in subdiv.iter().enumerate().take(new_size) {
            for j in 0..3 {
                if sub.rgb_width[j] as i32 > max_size && sub.num_entries > 1 {
                    max_size = sub.rgb_width[j] as i32;
                    index = i;
                    axis = j;
                }
            }
        }
        if max_size == -1 {
            return new_size;
        }

        // Sort the box's colours along `axis` (ties broken by the other axes).
        let mut members = Vec::with_capacity(subdiv[index].num_entries as usize);
        let mut q = subdiv[index].head;
        while q != NONE && members.len() < subdiv[index].num_entries as usize {
            members.push(q);
            q = colors[q].next;
        }
        let key = |q: usize| {
            let c = colors[q].rgb;
            (c[axis] as u32) << 16 | (c[(axis + 1) % 3] as u32) << 8 | c[(axis + 2) % 3] as u32
        };
        members.sort_by_key(|&q| key(q));
        for w in members.windows(2) {
            colors[w[0]].next = w[1];
        }
        colors[*members.last().expect("box has > 1 entry")].next = NONE;
        subdiv[index].head = members[0];

        // Walk until half the pixel count is covered.
        let mut q = members[0];
        let mut sum = (subdiv[index].count / 2) as i64 - colors[q].count;
        let mut num_entries = 1u32;
        let mut count = colors[q].count;
        loop {
            let next = colors[q].next;
            if next == NONE {
                break;
            }
            sum -= colors[next].count;
            if sum < 0 || colors[next].next == NONE {
                break;
            }
            q = next;
            num_entries += 1;
            count += colors[q].count;
        }
        let second = colors[q].next;
        let max_color = (colors[q].rgb[axis] as u32) << (8 - BITS_PER_PRIM_COLOR);
        let min_color = (colors[second].rgb[axis] as u32) << (8 - BITS_PER_PRIM_COLOR);

        // Split: the second half becomes a new box.
        colors[q].next = NONE;
        let parent = subdiv[index];
        let mut child = Subdivision {
            head: second,
            count: count as u64,
            num_entries: parent.num_entries - num_entries,
            ..parent
        };
        subdiv[index].count -= count as u64;
        subdiv[index].num_entries = num_entries;
        child.rgb_width[axis] =
            (child.rgb_min[axis] as u32).wrapping_add(child.rgb_width[axis] as u32).wrapping_sub(min_color) as u8;
        child.rgb_min[axis] = min_color as u8;
        subdiv[index].rgb_width[axis] = max_color.wrapping_sub(subdiv[index].rgb_min[axis] as u32) as u8;
        subdiv[new_size] = child;
        new_size += 1;
    }
    new_size
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifQuantizeBuffer(
    Width: c_uint,
    Height: c_uint,
    ColorMapSize: *mut c_int,
    RedInput: *const GifByteType,
    GreenInput: *const GifByteType,
    BlueInput: *const GifByteType,
    OutputBuffer: *mut GifByteType,
    OutputColorMap: *mut GifColorType,
) -> c_int {
    let Some(pixels) = (Width as usize).checked_mul(Height as usize) else {
        return GIF_ERROR;
    };
    unsafe {
        let map_size = (*ColorMapSize).clamp(0, 256) as usize;
        let red = slice(RedInput, pixels);
        let green = slice(GreenInput, pixels);
        let blue = slice(BlueInput, pixels);
        let out = slice_mut(OutputBuffer, pixels);
        let map = slice_mut(OutputColorMap, map_size.max(1));
        match quantize(red, green, blue, map_size, out, map) {
            Some(n) => {
                *ColorMapSize = n as c_int;
                GIF_OK
            }
            None => GIF_ERROR,
        }
    }
}
