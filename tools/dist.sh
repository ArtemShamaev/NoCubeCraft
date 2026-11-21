#!/bin/sh
# Builds the single-file game for Linux and Windows into dist/.
# Windows needs: rustup target add x86_64-pc-windows-gnu, and mingw-w64-gcc.
# Builds run at low priority, so the machine stays usable.
set -e
cd "$(dirname "$0")/.."
mkdir -p dist

nice -n 19 cargo build --release
cp target/release/nocubecraft dist/NoCubeCraft.x86_64

if [ "$1" != "--linux-only" ]; then
    nice -n 19 cargo build --release --target x86_64-pc-windows-gnu
    cp target/x86_64-pc-windows-gnu/release/nocubecraft.exe dist/NoCubeCraft.exe
fi
ls -lh dist
