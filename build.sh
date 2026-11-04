#!/bin/sh
# Build `examples/hello.rs` into a static RV64 ELF and print the exact command
# the kernel can use.
#
# No network access is needed: the pinned nightly and its precompiled
# `riscv64gc-unknown-none-elf` core are already installed by rustup.
set -eu
cd "$(dirname "$0")"

TOOLCHAIN="${TOOLCHAIN:-nightly-2026-07-12}"
TARGET=riscv64gc-unknown-none-elf
EXAMPLE=hello
ELF="target/$TARGET/release/examples/$EXAMPLE"

echo "== build command =="
echo "rustup run $TOOLCHAIN cargo build --release --example $EXAMPLE --target $TARGET"
echo

rustup run "$TOOLCHAIN" cargo build --release --example "$EXAMPLE" --target "$TARGET"

echo
echo "== ELF =="
ls -l "$ELF"
file "$ELF" 2>/dev/null || true
echo
echo "== run in pagh-riscv =="
echo "run /path/in/guest/$EXAMPLE"
echo
echo "(the target triple and linker arguments live in .cargo/config.toml;"
echo " the layout is pagh-user.ld, base 0x10000, _start in .text.entry)"