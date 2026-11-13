//! giflib-rs: a memory-safe, drop-in replacement for giflib written in Rust.
//!
//! Builds `libgif.so.7` / `libgif.a` exporting the same C API and ABI as
//! giflib 6.1.3, so existing programs work unchanged. The LZW codec, which
//! parses untrusted input, is entirely safe Rust (`lzw` module); `unsafe`
//! is confined to the thin C-facing layer.
//!
//! The library does not use Rust's `std`: it calls the same libc functions
//! as upstream (open/read/write/malloc). The `std` feature only exists so
//! Rust test harnesses such as cargo-fuzz can link it.

// Unwinding builds (e.g. `cargo test`) need std; the real library is panic = abort.
#![cfg_attr(all(panic = "abort", not(test), not(feature = "std")), no_std)]

extern crate alloc;

pub mod dgif;
pub mod egif;
pub mod err;
pub mod font;
mod font_table;
pub mod gifalloc;
pub mod hash;
pub mod lzw;
pub mod private;
pub mod quantize;
mod sys;
pub mod types;
