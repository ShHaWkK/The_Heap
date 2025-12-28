#!/usr/bin/env bash
set -euo pipefail

cargo bootimage --manifest-path kernel/Cargo.toml --target x86_64-the_heap.json -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem

display_arg="-display gtk"
if [ -z "${DISPLAY:-}" ]; then
  display_arg="-display curses"
fi

qemu-system-x86_64 \
  -drive format=raw,file=target/x86_64-the_heap/debug/bootimage-kernel.bin \
  ${display_arg} \
  -serial stdio
