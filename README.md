# giflib-rs

A memory-safe, drop-in replacement for [giflib](https://giflib.sourceforge.net/) written in Rust.

giflib is the C library that decodes and encodes GIF images inside ImageMagick, GDAL, Tesseract, Emacs, GNOME's file indexer and many other programs. It parses untrusted files and has a long history of memory-safety bugs. giflib-rs builds a `libgif.so.7` with **the same C API and ABI as giflib 6.1.3**, so existing programs can use it without being changed or recompiled.

## Status

| Check | Result |
|---|---|
| Upstream giflib regression suite (utilities linked against giflib-rs) | **51 / 51 pass** |
| Exported symbols vs. upstream `libgif.so` | **identical (61 symbols)** |
| Soname | `libgif.so.7` |
| Differential fuzzing vs. C giflib: 2,000 corrupted GIFs × `giffilter`, `giftext` | **4,000 / 4,000 byte-identical output, 0 crashes** |
| Struct layouts (`GifFileType`, `SavedImage`, …) | checked at compile time |

## Design

- **The LZW codec (`src/lzw.rs`) is 100% safe Rust.** This is the code that reads untrusted bytes. It follows upstream's algorithm step by step, so output is bit-identical, but every table access is bounds-checked. Malformed input produces an error code, never memory corruption.
- `unsafe` is confined to the thin layer that speaks the C ABI (raw pointers in and out).
- Memory a C caller can see or free (`SavedImages`, `RasterBits`, colour maps, extension blocks) is allocated with `malloc`, exactly like upstream.
- Built with `panic = "abort"`: nothing ever unwinds across the C boundary.
- No dependencies beyond the Rust standard library.

## Build

```sh
cargo build --release
# target/release/libgif.so  (soname libgif.so.7)
# target/release/libgif.a
# header: include/gif_lib.h
```

## Test

```sh
cargo test --release           # unit tests (LZW round-trips, garbage input)
tests/run-upstream.sh          # upstream suite against this library
```

`run-upstream.sh` clones upstream giflib (or uses `GIFLIB_SRC=/path/to/giflib`), builds its utilities against `target/release/libgif.so.7`, checks with `ldd` that they really use it, and runs upstream's `make -C tests tap`.

## Try it with an existing program

```sh
cargo build --release
ln -sf libgif.so target/release/libgif.so.7
LD_LIBRARY_PATH=$PWD/target/release some-program-that-uses-giflib
```

## Differences from upstream

Behaviour is intentionally identical, except where upstream has undefined behaviour:

- `DGifOpenFileHandle` reports the real error when the screen descriptor is unreadable (upstream leaves `*Error` unset).
- Invalid file descriptors and `NULL` buffers return an error instead of crashing.
- `GifQuantizeBuffer` returns `GIF_ERROR` for an empty image (upstream reads out of bounds).

## Roadmap

- [ ] Coverage-guided fuzzing (`cargo fuzz`) of the decoder
- [ ] Run against real consumers (ImageMagick, GDAL, Emacs) via `LD_LIBRARY_PATH`
- [ ] Debian/Ubuntu packaging as an alternative `libgif7`
- [ ] Reduce shared-library size

## License

MIT, same as giflib. `include/gif_lib.h` is from upstream giflib; see `LICENSE-giflib`.
