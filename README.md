# The_Heap

Kernel Rust `no_std` (Phil-OS style) + slab allocator (global allocator) + intégration FAT32.

## Build / Run
- Installer `bootimage`:
  - `cargo install bootimage`
- Lancer le kernel en QEMU:
  - `cargo run -p kernel`

## Tests
- `cargo test -p slaballoc`
- `cargo test -p fat32`
