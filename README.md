# giflib-rs

A memory-safe, drop-in replacement for [giflib](https://giflib.sourceforge.net/) written in Rust.

giflib is the C library that decodes and encodes GIF images inside OpenJDK, Emacs, GDAL, Leptonica/Tesseract, Krita, OpenImageIO, imlib2, the WebP tools, GNOME's file indexer and many other programs. It parses untrusted files and has a long history of memory-safety bugs. giflib-rs builds a `libgif.so.7` with **the same C API and ABI as giflib 6.1.3**, so existing programs can use it without being changed or recompiled.

## Status

| Check | Result |
|---|---|
| Upstream giflib regression suite (utilities linked against giflib-rs) | **51 / 51 pass** |
| Exported symbols vs. upstream `libgif.so` | **identical (62 symbols)** |
| Soname | `libgif.so.7` |
| Differential testing vs. C giflib: 2,000 corrupted GIFs × `giffilter`, `giftext` | **4,000 / 4,000 byte-identical output, 0 crashes** |
| Real program: `gif2webp` (Ubuntu's binary, unmodified) with Ubuntu's giflib vs. giflib-rs via `LD_LIBRARY_PATH` | **2,008 / 2,008 identical** (8 test images + 2,000 corrupted) |
| Coverage-guided fuzzing (`cargo fuzz`, decode and decode→encode→decode) | **671,282 runs, 0 crashes** (see [Fuzzing](#fuzzing)) |
| Size of `libgif.so` (stripped) | 79 KB (C giflib: 43 KB) |
| Struct layouts (`GifFileType`, `SavedImage`, …) | checked at compile time |

## Design

- **The LZW codec (`src/lzw.rs`) is 100% safe Rust.** This is the code that reads untrusted bytes. It follows upstream's algorithm step by step, so output is bit-identical, but every table access is bounds-checked. Malformed input produces an error code, never memory corruption.
- `unsafe` is confined to the thin layer that speaks the C ABI (raw pointers in and out).
- Memory a C caller can see or free (`SavedImages`, `RasterBits`, colour maps, extension blocks) is allocated with `malloc`, exactly like upstream.
- Built with `panic = "abort"`: nothing ever unwinds across the C boundary.
- No Rust `std` and no dependencies: the library uses the same libc calls as upstream (`open`, `read`, `write`, `malloc`). A linker version script exports exactly upstream's symbols.

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

## Fuzzing

```sh
cd fuzz
ASAN_OPTIONS=max_allocation_size_mb=1024:allocator_may_return_null=1 \
  cargo +nightly fuzz run --fuzz-dir . -O decode corpus/decode seeds
cargo +nightly fuzz run --fuzz-dir . -O roundtrip corpus/roundtrip seeds regressions
```

- `decode` feeds arbitrary bytes to `DGifOpen`/`DGifSlurp` and reads every pixel.
- `roundtrip` decodes, re-encodes with `EGifSpew`, decodes again, and checks every pixel survived.
- `max_allocation_size_mb` makes huge allocations (GIF headers can claim 65535×65535 images) fail, which exercises the out-of-memory paths.

Results so far (30 minutes each, AddressSanitizer, seeded with giflib's test images):

| Target | Runs | Crashes / failures |
|---|---|---|
| `decode` | 268,975 | 0 |
| `roundtrip` (after the fix below) | 402,307 | 0 |

The roundtrip fuzzer found a data-loss bug in upstream giflib 6.1.3 (see below); its input is kept in `fuzz/regressions/`.

## Try it with an existing program

```sh
cargo build --release
ln -sf libgif.so target/release/libgif.so.7
LD_LIBRARY_PATH=$PWD/target/release some-program-that-uses-giflib
```

## Differences from upstream

Behaviour is intentionally identical, except where upstream has undefined behaviour or loses data:

- **`GifMakeSavedImage` keeps extension blocks.** Upstream 6.1.3 allocates and copies them but leaves `ExtensionBlockCount` at 0, so copies (e.g. `gifsponge`) lose every frame delay, transparency setting and loop count. Copying `pic/fire.gif` loses 36 of 36 blocks upstream and 0 here.
- **`EGifSpew` always writes an introducer for an image's first extension block**, even if its label is `0x00` (which upstream confuses with "continuation" and writes a corrupt file).
- `DGifOpenFileHandle` reports the real error when the screen descriptor is unreadable (upstream leaves `*Error` unset).
- Invalid file descriptors and `NULL` buffers return an error instead of crashing.
- `GifQuantizeBuffer` returns `GIF_ERROR` for an empty image (upstream reads out of bounds).

## Roadmap

- [x] Coverage-guided fuzzing (`cargo fuzz`) of decoder and encoder
- [x] Reduce shared-library size (488 KB → 91 KB; no `std`)
- [x] First real consumer: `gif2webp`, byte-identical
- [ ] More real consumers via `LD_LIBRARY_PATH`: Emacs, feh/imlib2, GDAL, OpenJDK
- [ ] OSS-Fuzz integration and CI
- [ ] Debian/Ubuntu packaging as an alternative `libgif7`

## License

MIT, same as giflib. `include/gif_lib.h` and the images in `fuzz/seeds/` are from upstream giflib; see `LICENSE-giflib`.
