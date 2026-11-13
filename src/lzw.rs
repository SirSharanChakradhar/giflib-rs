//! The LZW engine: decoding and encoding of GIF raster data.
//!
//! This module is entirely safe Rust. It is the part of giflib that parses
//! untrusted bytes, and it mirrors the upstream algorithm step for step so
//! that output is bit-identical, but every table access is bounds-checked.

use crate::types::*;
use alloc::boxed::Box;
use alloc::vec;
use core::ffi::c_int;

pub const LZ_MAX_CODE: usize = 4095;
pub const LZ_BITS: i32 = 12;
pub const FLUSH_OUTPUT: i32 = 4096;
pub const FIRST_CODE: i32 = 4097;
pub const NO_SUCH_CODE: i32 = 4098;

/// "Return GIF_ERROR but leave `GifFile->Error` untouched" (upstream does this in a few places).
pub const KEEP_ERROR: c_int = -1;

pub type LzwResult<T> = Result<T, c_int>;

/// Somewhere to read compressed bytes from.
pub trait ByteSource {
    /// Fill `buf` completely; `false` on short read or error.
    fn read_exact(&mut self, buf: &mut [u8]) -> bool;
}

/// Somewhere to write compressed bytes to.
pub trait ByteSink {
    /// Write all of `buf`; `false` on error.
    fn write_all(&mut self, buf: &[u8]) -> bool;
}

const HT_SIZE: usize = 8192;
const HT_KEY_MASK: u32 = 0x1FFF;
const HT_EMPTY: u32 = 0xFFFFF;

/// The encoder's open-addressing hash table (same layout as `gif_hash.c`).
pub struct HashTable {
    pub table: Box<[u32]>,
}

impl HashTable {
    pub fn new() -> Self {
        HashTable { table: vec![u32::MAX; HT_SIZE].into_boxed_slice() }
    }
    pub fn clear(&mut self) {
        self.table.fill(u32::MAX);
    }
    fn key_item(item: u32) -> usize {
        (((item >> 12) ^ item) & HT_KEY_MASK) as usize
    }
    pub fn insert(&mut self, key: u32, code: i32) {
        Self::insert_in(&mut self.table, key, code)
    }
    pub fn exists(&self, key: u32) -> i32 {
        Self::exists_in(&self.table, key)
    }
    /// Insert into a raw 8192-entry table (also used by the exported C helpers).
    pub fn insert_in(table: &mut [u32], key: u32, code: i32) {
        let mut h = Self::key_item(key);
        while table[h] >> 12 != HT_EMPTY {
            h = (h + 1) & HT_KEY_MASK as usize;
        }
        table[h] = (key << 12) | (code as u32 & 0x0FFF);
    }
    /// Look up a key in a raw table; -1 if absent.
    pub fn exists_in(table: &[u32], key: u32) -> i32 {
        let mut h = Self::key_item(key);
        loop {
            let k = table[h] >> 12;
            if k == HT_EMPTY {
                return -1;
            }
            if k == key {
                return (table[h] & 0x0FFF) as i32;
            }
            h = (h + 1) & HT_KEY_MASK as usize;
        }
    }
}

impl Default for HashTable {
    fn default() -> Self {
        Self::new()
    }
}

/// LZW state shared by the reader and the writer (`GifFilePrivateType` minus I/O).
pub struct Lzw {
    /// Pascal-style block buffer: `buf[0]` is a length/count.
    pub buf: [u8; 256],
    pub bits_per_pixel: i32,
    pub clear_code: i32,
    pub eof_code: i32,
    pub running_code: i32,
    pub running_bits: i32,
    pub max_code1: i32,
    pub last_code: i32,
    pub crnt_code: i32,
    pub stack_ptr: i32,
    pub shift_state: i32,
    pub shift_dword: u64,
    pub pixel_count: u64,
    stack: Box<[u8]>,  // LZ_MAX_CODE
    suffix: Box<[u8]>, // LZ_MAX_CODE + 1
    prefix: Box<[u32]>, // LZ_MAX_CODE + 1
    pub hash: Option<HashTable>,
}

impl Default for Lzw {
    fn default() -> Self {
        Self::new()
    }
}

const CODE_MASKS: [u64; 13] = [
    0x0000, 0x0001, 0x0003, 0x0007, 0x000f, 0x001f, 0x003f, 0x007f, 0x00ff, 0x01ff, 0x03ff,
    0x07ff, 0x0fff,
];

impl Lzw {
    pub fn new() -> Self {
        Lzw {
            buf: [0; 256],
            bits_per_pixel: 0,
            clear_code: 0,
            eof_code: 0,
            running_code: 0,
            running_bits: 0,
            max_code1: 0,
            last_code: 0,
            crnt_code: 0,
            stack_ptr: 0,
            shift_state: 0,
            shift_dword: 0,
            pixel_count: 0,
            stack: vec![0; LZ_MAX_CODE].into_boxed_slice(),
            suffix: vec![0; LZ_MAX_CODE + 1].into_boxed_slice(),
            prefix: vec![0; LZ_MAX_CODE + 1].into_boxed_slice(),
            hash: None,
        }
    }

    fn reset_codes(&mut self) {
        self.running_code = self.eof_code + 1;
        self.running_bits = self.bits_per_pixel + 1;
        self.max_code1 = 1 << self.running_bits;
    }

    // ----------------------------------------------------------------- decoding

    /// Read the LZW minimum code size and reset the decoder (`DGifSetupDecompress`).
    pub fn setup_decompress(&mut self, src: &mut impl ByteSource) -> LzwResult<()> {
        let mut code_size = [0u8; 1];
        if !src.read_exact(&mut code_size) {
            return Err(D_GIF_ERR_READ_FAILED);
        }
        let bits = code_size[0] as i32;
        if !(2..=8).contains(&bits) {
            // Only happens on a severely malformed GIF (upstream's error code).
            return Err(D_GIF_ERR_READ_FAILED);
        }
        self.buf[0] = 0;
        self.bits_per_pixel = bits;
        self.clear_code = 1 << bits;
        self.eof_code = self.clear_code + 1;
        self.reset_codes();
        self.stack_ptr = 0;
        self.last_code = NO_SUCH_CODE;
        self.shift_state = 0;
        self.shift_dword = 0;
        self.prefix.fill(NO_SUCH_CODE as u32);
        Ok(())
    }

    /// Next byte of compressed data, reading a new sub-block when needed.
    fn buffered_input(&mut self, src: &mut impl ByteSource) -> LzwResult<u8> {
        if self.buf[0] == 0 {
            if !src.read_exact(&mut self.buf[..1]) {
                return Err(D_GIF_ERR_READ_FAILED);
            }
            // An empty block here means the stream ended without an EOF code.
            if self.buf[0] == 0 {
                return Err(D_GIF_ERR_IMAGE_DEFECT);
            }
            let len = self.buf[0] as usize;
            if !src.read_exact(&mut self.buf[1..=len]) {
                return Err(D_GIF_ERR_READ_FAILED);
            }
            let next = self.buf[1];
            self.buf[1] = 2; // buf[1] now indexes the next unread byte
            self.buf[0] -= 1;
            Ok(next)
        } else {
            let idx = self.buf[1] as usize;
            let next = self.buf[idx];
            self.buf[1] = self.buf[1].wrapping_add(1);
            self.buf[0] -= 1;
            Ok(next)
        }
    }

    /// Read one variable-width code (`DGifDecompressInput`).
    fn decompress_input(&mut self, src: &mut impl ByteSource) -> LzwResult<i32> {
        if self.running_bits > LZ_BITS {
            return Err(D_GIF_ERR_IMAGE_DEFECT);
        }
        while self.shift_state < self.running_bits {
            let byte = self.buffered_input(src)?;
            self.shift_dword |= (byte as u64) << self.shift_state;
            self.shift_state += 8;
        }
        let code = (self.shift_dword & CODE_MASKS[self.running_bits as usize]) as i32;
        self.shift_dword >>= self.running_bits;
        self.shift_state -= self.running_bits;

        if self.running_code < LZ_MAX_CODE as i32 + 2 {
            self.running_code += 1;
            if self.running_code > self.max_code1 && self.running_bits < LZ_BITS {
                self.max_code1 <<= 1;
                self.running_bits += 1;
            }
        }
        Ok(code)
    }

    /// Follow the prefix chain down to a literal pixel (`DGifGetPrefixChar`).
    fn prefix_char(&self, mut code: i32) -> i32 {
        let mut i = 0;
        while code > self.clear_code && i <= LZ_MAX_CODE {
            i += 1;
            if code > LZ_MAX_CODE as i32 {
                return NO_SUCH_CODE;
            }
            code = self.prefix[code as usize] as i32;
        }
        code
    }

    /// Decode exactly `line.len()` pixels (`DGifDecompressLine`).
    pub fn decompress_line(&mut self, src: &mut impl ByteSource, line: &mut [u8]) -> LzwResult<()> {
        let len = line.len();
        let mut i = 0usize;
        let mut stack_ptr = self.stack_ptr as usize;
        let clear = self.clear_code;
        let eof = self.eof_code;
        let mut last_code = self.last_code;

        if stack_ptr > LZ_MAX_CODE {
            return Err(KEEP_ERROR);
        }
        // Drain pixels left over from the previous call.
        while stack_ptr != 0 && i < len {
            stack_ptr -= 1;
            line[i] = self.stack[stack_ptr];
            i += 1;
        }

        while i < len {
            let crnt = self.decompress_input(src)?;
            if crnt == eof {
                return Err(D_GIF_ERR_EOF_TOO_SOON);
            } else if crnt == clear {
                self.prefix.fill(NO_SUCH_CODE as u32);
                self.reset_codes();
                last_code = NO_SUCH_CODE;
                self.last_code = NO_SUCH_CODE;
            } else {
                if crnt < clear {
                    // A literal pixel.
                    line[i] = crnt as u8;
                    i += 1;
                } else {
                    // A table code: walk the chain, pushing suffixes, then pop.
                    let mut crnt_prefix;
                    if self.prefix[crnt as usize] == NO_SUCH_CODE as u32 {
                        crnt_prefix = last_code;
                        // Only valid when crnt is exactly the next code (the KwKwK case).
                        let c = if crnt == self.running_code - 2 {
                            self.prefix_char(last_code)
                        } else {
                            self.prefix_char(crnt)
                        } as u8;
                        self.suffix[(self.running_code - 2) as usize] = c;
                        self.stack[stack_ptr] = c;
                        stack_ptr += 1;
                    } else {
                        crnt_prefix = crnt;
                    }
                    // Bounded walk: a defective image cannot loop forever or overflow.
                    while stack_ptr < LZ_MAX_CODE
                        && crnt_prefix > clear
                        && crnt_prefix <= LZ_MAX_CODE as i32
                    {
                        self.stack[stack_ptr] = self.suffix[crnt_prefix as usize];
                        stack_ptr += 1;
                        crnt_prefix = self.prefix[crnt_prefix as usize] as i32;
                    }
                    if stack_ptr >= LZ_MAX_CODE || crnt_prefix > LZ_MAX_CODE as i32 {
                        return Err(D_GIF_ERR_IMAGE_DEFECT);
                    }
                    self.stack[stack_ptr] = crnt_prefix as u8;
                    stack_ptr += 1;
                    while stack_ptr != 0 && i < len {
                        stack_ptr -= 1;
                        line[i] = self.stack[stack_ptr];
                        i += 1;
                    }
                }
                let next = self.running_code - 2;
                if last_code != NO_SUCH_CODE
                    && next < LZ_MAX_CODE as i32 + 1
                    && self.prefix[next as usize] == NO_SUCH_CODE as u32
                {
                    self.prefix[next as usize] = last_code as u32;
                    let c = if crnt == next {
                        self.prefix_char(last_code)
                    } else {
                        self.prefix_char(crnt)
                    };
                    self.suffix[next as usize] = c as u8;
                }
                last_code = crnt;
            }
        }
        self.last_code = last_code;
        self.stack_ptr = stack_ptr as i32;
        Ok(())
    }

    /// Read the next raw sub-block into `buf` (`DGifGetCodeNext`).
    /// Returns `true` if a block was read, `false` on the terminating empty block.
    pub fn get_code_next(&mut self, src: &mut impl ByteSource) -> LzwResult<bool> {
        let mut len = [0u8; 1];
        if !src.read_exact(&mut len) {
            return Err(D_GIF_ERR_READ_FAILED);
        }
        if len[0] > 0 {
            self.buf[0] = len[0];
            if !src.read_exact(&mut self.buf[1..=len[0] as usize]) {
                return Err(D_GIF_ERR_READ_FAILED);
            }
            Ok(true)
        } else {
            self.buf[0] = 0;
            self.pixel_count = 0;
            Ok(false)
        }
    }

    /// Read the next extension sub-block into `buf` (`DGifGetExtensionNext`).
    pub fn get_extension_next(&mut self, src: &mut impl ByteSource) -> LzwResult<bool> {
        let mut len = [0u8; 1];
        if !src.read_exact(&mut len) {
            return Err(D_GIF_ERR_READ_FAILED);
        }
        if len[0] > 0 {
            self.buf[0] = len[0];
            if !src.read_exact(&mut self.buf[1..=len[0] as usize]) {
                return Err(D_GIF_ERR_READ_FAILED);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Skip the rest of the image data up to its terminating empty block.
    fn skip_to_end_of_image(&mut self, src: &mut impl ByteSource) -> LzwResult<()> {
        while self.get_code_next(src)? {}
        Ok(())
    }

    /// `DGifGetLine` after argument validation.
    pub fn get_line(&mut self, src: &mut impl ByteSource, line: &mut [u8]) -> LzwResult<()> {
        if self.pixel_count < line.len() as u64 {
            return Err(D_GIF_ERR_DATA_TOO_BIG);
        }
        self.pixel_count -= line.len() as u64;
        self.decompress_line(src, line)?;
        if self.pixel_count == 0 {
            self.skip_to_end_of_image(src)?;
        }
        Ok(())
    }

    /// `DGifGetPixel` (the decoded pixel is discarded, as upstream does).
    pub fn get_pixel(&mut self, src: &mut impl ByteSource) -> LzwResult<()> {
        if self.pixel_count == 0 {
            return Err(D_GIF_ERR_DATA_TOO_BIG);
        }
        self.pixel_count -= 1;
        let mut px = [0u8; 1];
        self.decompress_line(src, &mut px)?;
        if self.pixel_count == 0 {
            self.skip_to_end_of_image(src)?;
        }
        Ok(())
    }

    /// `DGifGetLZCodes`: next raw code, or -1 at end of image.
    pub fn get_lz_code(&mut self, src: &mut impl ByteSource) -> LzwResult<i32> {
        let code = self.decompress_input(src)?;
        if code == self.eof_code {
            self.skip_to_end_of_image(src)?;
            return Ok(-1);
        } else if code == self.clear_code {
            self.reset_codes();
        }
        Ok(code)
    }

    // ----------------------------------------------------------------- encoding

    /// Write the code size and reset the encoder (`EGifSetupCompress`).
    pub fn setup_compress(&mut self, sink: &mut impl ByteSink, bits_per_pixel: i32) -> LzwResult<()> {
        let bits = bits_per_pixel.max(2);
        let _ = sink.write_all(&[bits as u8]);
        self.buf[0] = 0;
        self.bits_per_pixel = bits;
        self.clear_code = 1 << bits;
        self.eof_code = self.clear_code + 1;
        self.reset_codes();
        self.crnt_code = FIRST_CODE;
        self.shift_state = 0;
        self.shift_dword = 0;
        self.hash.get_or_insert_with(HashTable::new).clear();
        if !self.compress_output(sink, self.clear_code) {
            return Err(E_GIF_ERR_DISK_IS_FULL);
        }
        Ok(())
    }

    /// Compress `line` (`EGifCompressLine`).
    pub fn compress_line(&mut self, sink: &mut impl ByteSink, line: &[u8]) -> LzwResult<()> {
        let mut i = 0;
        let mut crnt = if self.crnt_code == FIRST_CODE {
            match line.first() {
                Some(&p) => {
                    i = 1;
                    p as i32
                }
                None => return Ok(()),
            }
        } else {
            self.crnt_code
        };
        let mut hash = self.hash.take().unwrap_or_default();
        let result = (|| {
            while i < line.len() {
                let pixel = line[i];
                i += 1;
                let key = ((crnt as u32) << 8) + pixel as u32;
                let found = hash.exists(key);
                if found >= 0 {
                    crnt = found;
                } else {
                    if !self.compress_output(sink, crnt) {
                        return Err(E_GIF_ERR_DISK_IS_FULL);
                    }
                    crnt = pixel as i32;
                    if self.running_code >= LZ_MAX_CODE as i32 {
                        // Table full: emit a clear code and start over.
                        if !self.compress_output(sink, self.clear_code) {
                            return Err(E_GIF_ERR_DISK_IS_FULL);
                        }
                        self.reset_codes();
                        hash.clear();
                    } else {
                        hash.insert(key, self.running_code);
                        self.running_code += 1;
                    }
                }
            }
            self.crnt_code = crnt;
            if self.pixel_count == 0 {
                // End of image: last code, EOF code, then flush.
                if !self.compress_output(sink, crnt)
                    || !self.compress_output(sink, self.eof_code)
                    || !self.compress_output(sink, FLUSH_OUTPUT)
                {
                    return Err(E_GIF_ERR_DISK_IS_FULL);
                }
            }
            Ok(())
        })();
        self.hash = Some(hash);
        result
    }

    /// Pack one code into bytes (`EGifCompressOutput`). `false` on write error.
    fn compress_output(&mut self, sink: &mut impl ByteSink, code: i32) -> bool {
        let mut ok = true;
        if code == FLUSH_OUTPUT {
            while self.shift_state > 0 {
                ok &= self.buffered_output(sink, (self.shift_dword & 0xff) as i32);
                self.shift_dword >>= 8;
                self.shift_state -= 8;
            }
            self.shift_state = 0;
            ok &= self.buffered_output(sink, FLUSH_OUTPUT);
        } else {
            self.shift_dword |= (code as u64) << self.shift_state;
            self.shift_state += self.running_bits;
            while self.shift_state >= 8 {
                ok &= self.buffered_output(sink, (self.shift_dword & 0xff) as i32);
                self.shift_dword >>= 8;
                self.shift_state -= 8;
            }
        }
        if self.running_code >= self.max_code1 && code <= 4095 {
            self.running_bits += 1;
            self.max_code1 = 1 << self.running_bits;
        }
        ok
    }

    /// Buffer bytes into 255-byte sub-blocks (`EGifBufferedOutput`).
    fn buffered_output(&mut self, sink: &mut impl ByteSink, c: i32) -> bool {
        if c == FLUSH_OUTPUT {
            let n = self.buf[0] as usize;
            if n != 0 && !sink.write_all(&self.buf[..=n]) {
                return false;
            }
            self.buf[0] = 0;
            sink.write_all(&self.buf[..1])
        } else {
            if self.buf[0] == 255 {
                if !sink.write_all(&self.buf[..]) {
                    return false;
                }
                self.buf[0] = 0;
            }
            self.buf[0] += 1;
            self.buf[self.buf[0] as usize] = c as u8;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VecSink(Vec<u8>);
    impl ByteSink for VecSink {
        fn write_all(&mut self, buf: &[u8]) -> bool {
            self.0.extend_from_slice(buf);
            true
        }
    }
    struct SliceSource<'a>(&'a [u8]);
    impl ByteSource for SliceSource<'_> {
        fn read_exact(&mut self, buf: &mut [u8]) -> bool {
            if self.0.len() < buf.len() {
                return false;
            }
            let (head, tail) = self.0.split_at(buf.len());
            buf.copy_from_slice(head);
            self.0 = tail;
            true
        }
    }

    fn roundtrip(bits: i32, pixels: &[u8]) {
        let mut enc = Lzw::new();
        let mut sink = VecSink(Vec::new());
        enc.pixel_count = pixels.len() as u64;
        enc.setup_compress(&mut sink, bits).unwrap();
        enc.pixel_count = 0; // single call compresses everything and flushes
        enc.compress_line(&mut sink, pixels).unwrap();

        let mut dec = Lzw::new();
        let mut src = SliceSource(&sink.0);
        dec.setup_decompress(&mut src).unwrap();
        dec.pixel_count = pixels.len() as u64;
        let mut out = vec![0u8; pixels.len()];
        dec.get_line(&mut src, &mut out).unwrap();
        assert_eq!(out, pixels);
        assert!(src.0.is_empty(), "decoder must consume the terminator");
    }

    #[test]
    fn roundtrip_small_and_large() {
        roundtrip(2, &[0, 1, 2, 3, 3, 3, 3, 0, 1, 2]);
        let noisy: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        roundtrip(8, &noisy);
        let flat = vec![7u8; 100_000];
        roundtrip(4, &flat);
    }

    #[test]
    fn garbage_never_panics() {
        // Every possible 3-byte stream after a valid code size must fail cleanly or succeed.
        for a in 0..=255u8 {
            for b in [0u8, 1, 2, 255] {
                let data = [8, 2, a, b, 0];
                let mut dec = Lzw::new();
                let mut src = SliceSource(&data);
                dec.setup_decompress(&mut src).unwrap();
                dec.pixel_count = 64;
                let mut out = [0u8; 64];
                let _ = dec.get_line(&mut src, &mut out);
            }
        }
    }
}
