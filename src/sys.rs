//! Minimal OS layer: the same libc calls upstream giflib uses (open, read,
//! write, close, malloc), so the library does not need Rust's `std`.

use alloc::boxed::Box;
use alloc::vec;
use core::ffi::{c_char, c_int, c_void};

unsafe extern "C" {
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn close(fd: c_int) -> c_int;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[link_name = "__errno_location"]
    fn errno_location() -> *mut c_int;
    #[cfg(any(target_vendor = "apple", target_os = "freebsd"))]
    #[link_name = "__error"]
    fn errno_location() -> *mut c_int;
    #[cfg(any(target_os = "netbsd", target_os = "openbsd"))]
    #[link_name = "__errno"]
    fn errno_location() -> *mut c_int;
}

const EINTR: c_int = 4;

#[cfg(any(target_os = "linux", target_os = "android"))]
mod flags {
    pub const O_RDONLY: i32 = 0;
    pub const O_WRONLY: i32 = 0o1;
    pub const O_CREAT: i32 = 0o100;
    pub const O_EXCL: i32 = 0o200;
    pub const O_TRUNC: i32 = 0o1000;
    pub const O_CLOEXEC: i32 = 0o2000000;
}
#[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"))]
mod flags {
    pub const O_RDONLY: i32 = 0;
    pub const O_WRONLY: i32 = 0x1;
    pub const O_CREAT: i32 = 0x200;
    pub const O_TRUNC: i32 = 0x400;
    pub const O_EXCL: i32 = 0x800;
    pub const O_CLOEXEC: i32 = 0;
}
use flags::*;

fn interrupted() -> bool {
    unsafe { *errno_location() == EINTR }
}

/// `open(path, O_RDONLY)`; -1 on failure.
pub unsafe fn open_read(path: *const c_char) -> c_int {
    unsafe { open(path, O_RDONLY | O_CLOEXEC) }
}

/// Open for writing with mode 0600, failing if it exists when `exclusive`.
pub unsafe fn open_write(path: *const c_char, exclusive: bool) -> c_int {
    let extra = if exclusive { O_EXCL } else { O_TRUNC };
    unsafe { open(path, O_WRONLY | O_CREAT | O_CLOEXEC | extra, 0o600 as c_int) }
}

const BUF_SIZE: usize = 8192;

/// A buffered reader over a file descriptor it owns (like a `FILE*` from `fdopen`).
pub struct FdReader {
    fd: c_int,
    buf: Box<[u8]>,
    pos: usize,
    len: usize,
}

impl FdReader {
    pub fn new(fd: c_int) -> Self {
        FdReader { fd, buf: vec![0; BUF_SIZE].into_boxed_slice(), pos: 0, len: 0 }
    }

    /// Fill `out` as far as possible; returns the number of bytes read.
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        let mut n = 0;
        while n < out.len() {
            if self.pos == self.len {
                let got = loop {
                    let r = unsafe { read(self.fd, self.buf.as_mut_ptr().cast(), self.buf.len()) };
                    if r < 0 && interrupted() {
                        continue;
                    }
                    break r;
                };
                if got <= 0 {
                    break;
                }
                self.pos = 0;
                self.len = got as usize;
            }
            let k = (self.len - self.pos).min(out.len() - n);
            out[n..n + k].copy_from_slice(&self.buf[self.pos..self.pos + k]);
            self.pos += k;
            n += k;
        }
        n
    }
}

impl Drop for FdReader {
    fn drop(&mut self) {
        unsafe { close(self.fd) };
    }
}

/// A buffered writer over a file descriptor it owns.
pub struct FdWriter {
    fd: c_int,
    buf: Box<[u8]>,
    len: usize,
    failed: bool,
    closed: bool,
}

impl FdWriter {
    pub fn new(fd: c_int) -> Self {
        FdWriter { fd, buf: vec![0; BUF_SIZE].into_boxed_slice(), len: 0, failed: false, closed: false }
    }

    fn write_raw(&mut self, mut data: &[u8]) -> bool {
        while !data.is_empty() {
            let r = unsafe { write(self.fd, data.as_ptr().cast(), data.len()) };
            if r < 0 && interrupted() {
                continue;
            }
            if r <= 0 {
                return false;
            }
            data = &data[r as usize..];
        }
        true
    }

    fn flush(&mut self) -> bool {
        let pending = self.len;
        self.len = 0;
        let buf = core::mem::take(&mut self.buf);
        let ok = self.write_raw(&buf[..pending]);
        self.buf = buf;
        ok
    }

    /// Write all of `data`; `false` if a write error occurred (now or earlier).
    pub fn write(&mut self, data: &[u8]) -> bool {
        if self.failed {
            return false;
        }
        if self.len + data.len() > self.buf.len() && !self.flush() {
            self.failed = true;
            return false;
        }
        if data.len() >= self.buf.len() {
            self.failed = !self.write_raw(data);
        } else {
            self.buf[self.len..self.len + data.len()].copy_from_slice(data);
            self.len += data.len();
        }
        !self.failed
    }

    /// Flush and close; `false` on any error (C: `fclose` failing).
    pub fn close(mut self) -> bool {
        let flushed = !self.failed && self.flush();
        self.closed = true;
        let closed = unsafe { close(self.fd) } == 0;
        flushed && closed
    }
}

impl Drop for FdWriter {
    fn drop(&mut self) {
        if !self.closed {
            if !self.failed {
                self.flush();
            }
            unsafe { close(self.fd) };
        }
    }
}

/// Rust heap allocations go to the C heap, like everything else in giflib.
#[cfg(all(panic = "abort", not(test), not(feature = "std")))]
mod runtime {
    use crate::gifalloc::{free, malloc, realloc};
    use core::alloc::{GlobalAlloc, Layout};
    use core::ffi::{c_int, c_void};

    unsafe extern "C" {
        fn posix_memalign(memptr: *mut *mut c_void, alignment: usize, size: usize) -> c_int;
        fn abort() -> !;
    }

    const MALLOC_ALIGN: usize = 16;

    struct CAllocator;

    unsafe impl GlobalAlloc for CAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe {
                if layout.align() <= MALLOC_ALIGN {
                    malloc(layout.size()).cast()
                } else {
                    let mut p = core::ptr::null_mut();
                    if posix_memalign(&mut p, layout.align(), layout.size()) == 0 { p.cast() } else { core::ptr::null_mut() }
                }
            }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
            unsafe { free(ptr.cast()) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            unsafe {
                if layout.align() <= MALLOC_ALIGN {
                    realloc(ptr.cast(), new_size).cast()
                } else {
                    let new = self.alloc(Layout::from_size_align_unchecked(new_size, layout.align()));
                    if !new.is_null() {
                        core::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                        free(ptr.cast());
                    }
                    new
                }
            }
        }
    }

    #[global_allocator]
    static ALLOCATOR: CAllocator = CAllocator;

    /// A bug in this library stops the process instead of corrupting memory.
    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        unsafe { abort() }
    }

    // Precompiled `core` refers to `rust_eh_personality` from its unwind
    // tables. We never unwind (panic = abort), so it is never called, but the
    // reference must resolve or the shared library fails to load with
    // RTLD_NOW. Define it as a *hidden* symbol so it is not exported.
    #[cfg(any(target_arch = "x86_64", target_arch = "x86", target_arch = "aarch64", target_arch = "riscv64"))]
    core::arch::global_asm!(
        ".pushsection .text.rust_eh_personality,\"ax\",@progbits",
        ".globl rust_eh_personality",
        ".hidden rust_eh_personality",
        ".type rust_eh_personality,@function",
        "rust_eh_personality:",
        "ret",
        ".popsection",
    );}
