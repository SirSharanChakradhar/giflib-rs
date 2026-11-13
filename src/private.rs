//! The hidden per-file state behind `GifFileType->Private`, and its I/O.

use crate::lzw::{ByteSink, ByteSource, Lzw};
use crate::types::*;
use core::ffi::c_int;
use std::fs::File;
use std::io::{BufReader, BufWriter, ErrorKind, Read, Write};

pub const FILE_STATE_WRITE: i32 = 0x01;
pub const FILE_STATE_SCREEN: i32 = 0x02;
pub const FILE_STATE_IMAGE: i32 = 0x04;
pub const FILE_STATE_READ: i32 = 0x08;

pub enum Io {
    None,
    ReadFile(BufReader<File>),
    WriteFile(BufWriter<File>),
    ReadFunc(InputFunc),
    WriteFunc(OutputFunc),
}

pub struct Private {
    pub file_state: i32,
    pub io: Io,
    pub gif89: bool,
    pub lzw: Lzw,
}

impl Private {
    pub fn new(io: Io, file_state: i32) -> Box<Private> {
        Box::new(Private { file_state, io, gif89: false, lzw: Lzw::new() })
    }
    pub fn readable(&self) -> bool {
        self.file_state & FILE_STATE_READ != 0
    }
    pub fn writeable(&self) -> bool {
        self.file_state & FILE_STATE_WRITE != 0
    }
}

/// Borrow the private state of a handle.
///
/// # Safety
/// `gif` must be a live handle created by this library.
pub unsafe fn private<'a>(gif: *mut GifFileType) -> &'a mut Private {
    unsafe { &mut *((*gif).Private as *mut Private) }
}

/// A byte stream bound to one handle. User callbacks receive the public
/// `GifFileType*`, exactly as with upstream.
pub struct Stream<'a> {
    pub gif: *mut GifFileType,
    pub io: &'a mut Io,
}

impl Stream<'_> {
    /// Read up to `buf.len()` bytes; returns how many were read
    /// (callbacks may return anything, as in C).
    pub fn read(&mut self, buf: &mut [u8]) -> i64 {
        match self.io {
            Io::ReadFile(r) => {
                let mut n = 0;
                while n < buf.len() {
                    match r.read(&mut buf[n..]) {
                        Ok(0) => break,
                        Ok(k) => n += k,
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                n as i64
            }
            Io::ReadFunc(Some(f)) => unsafe { f(self.gif, buf.as_mut_ptr(), buf.len() as c_int) as i64 },
            _ => 0,
        }
    }

    /// Write `buf`; returns the byte count reported (0 on failure).
    pub fn write(&mut self, buf: &[u8]) -> i64 {
        match self.io {
            Io::WriteFile(w) => {
                if w.write_all(buf).is_ok() {
                    buf.len() as i64
                } else {
                    0
                }
            }
            Io::WriteFunc(Some(f)) => unsafe { f(self.gif, buf.as_ptr(), buf.len() as c_int) as i64 },
            _ => 0,
        }
    }
}

impl ByteSource for Stream<'_> {
    fn read_exact(&mut self, buf: &mut [u8]) -> bool {
        self.read(buf) == buf.len() as i64
    }
}

impl ByteSink for Stream<'_> {
    fn write_all(&mut self, buf: &[u8]) -> bool {
        self.write(buf) == buf.len() as i64
    }
}

/// Split a handle into its stream and LZW state.
///
/// # Safety
/// `gif` must be a live handle created by this library.
pub unsafe fn parts<'a>(gif: *mut GifFileType) -> (Stream<'a>, &'a mut Lzw) {
    let p = unsafe { private(gif) };
    (Stream { gif, io: &mut p.io }, &mut p.lzw)
}
