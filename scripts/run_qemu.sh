#!/usr/bin/env bash
set -euo pipefail

cargo bootimage -p kernel --target x86_64-the_heap.json -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem

qemu-system-x86_64 \
  -drive format=raw,file=target/x86_64-the_heap/debug/bootimage-kernel.bin \
  -display gtk \
  -serial stdio
