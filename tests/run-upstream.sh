#!/bin/sh
# Build upstream giflib's utilities against *this* library (instead of the C
# libgif) and run upstream's own regression suite with them.
#
#   GIFLIB_SRC=/path/to/giflib tests/run-upstream.sh
#
# Without GIFLIB_SRC the upstream sources are cloned into target/.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SRC=${GIFLIB_SRC:-$ROOT/target/giflib-src}
WORK=$ROOT/target/upstream-tests
LIB=$ROOT/target/release

if [ ! -d "$SRC" ]; then
    git clone --depth 1 https://git.code.sf.net/p/giflib/code "$SRC"
fi

cargo build --release --manifest-path "$ROOT/Cargo.toml"
ln -sf libgif.so "$LIB/libgif.so.7"

rm -rf "$WORK"
mkdir -p "$WORK"
cp -r "$SRC"/. "$WORK"/
cd "$WORK"
# Make sure nothing can pick up the C library by accident.
rm -f ./*.o libgif.* libutil.*

UTILS="gif2rgb gifbuild gifclrmp gifecho giffilter giffix gifinto gifsponge \
       giftext giftool gifwedge gifbg gifcolor gifhisto"
for u in $UTILS; do
    ${CC:-gcc} -std=gnu99 -O2 -w -I"$ROOT/include" -o "$u" "$u.c" getarg.c qprintf.c \
        -L"$LIB" -lgif -Wl,-rpath,"$LIB" -lm
done

# Prove the utilities really use the Rust library.
ldd ./gif2rgb | grep -q "$LIB/libgif.so.7" || { echo "gif2rgb is not linked against $LIB" >&2; exit 1; }
echo "utilities linked against $LIB/libgif.so.7"

make --no-print-directory -C tests tap
