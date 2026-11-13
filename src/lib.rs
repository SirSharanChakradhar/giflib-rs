//! giflib-rs: a memory-safe, drop-in replacement for giflib written in Rust.
//!
//! Builds `libgif.so.7` / `libgif.a` exporting the same C API and ABI as
//! giflib 6.1.3, so existing programs work unchanged. The LZW codec, which
//! parses untrusted input, is entirely safe Rust (`lzw` module); `unsafe`
//! is confined to the thin C-facing layer.

pub mod alloc;
pub mod dgif;
pub mod egif;
pub mod err;
pub mod font;
mod font_table;
pub mod hash;
pub mod lzw;
pub mod private;
pub mod quantize;
pub mod types;
