#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

//! Noyau Rust Phil‑Opp :
//! - VGA texte (écriture directe dans `0xb8000`)
//! - Port série COM1 pour logs (redirigé par QEMU `-serial stdio`)
//! - Allocateur global (`#[global_allocator]`) avec démonstration `Vec`/`String`
//! - Intégration FAT32 en RAM : listage de `/` et lecture de `HELLO.TXT`

extern crate alloc;

mod vga;

use bootloader::BootInfo;
use bootloader::entry_point;
use fat32_parser::Fat32;
use core::panic::PanicInfo;
use slaballoc::LockedAlloc;

#[global_allocator]
static GLOBAL_ALLOC: LockedAlloc = LockedAlloc::new();

const HEAP_SIZE: usize = 512 * 1024;

#[repr(align(64))]
struct AlignedHeap<const N: usize> { buf: [u8; N] }

static mut HEAP: AlignedHeap<HEAP_SIZE> = AlignedHeap { buf: [0; HEAP_SIZE] };

#[inline(always)]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let mut v: u8;
    core::arch::asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags));
    v
}

#[cfg(test)]
#[repr(u8)]
enum QemuExitCode { Success = 0x10, Failed = 0x11 }

#[cfg(test)]
#[inline(always)]
fn exit_qemu(code: QemuExitCode) {
    unsafe { outb(0xF4, code as u8) }
}

/// Initialise le port série COM1 (38400 8N1) pour l’affichage terminal.
fn serial_init() {
    unsafe {
        outb(0x3F8 + 1, 0x00);
        outb(0x3F8 + 3, 0x80);
        outb(0x3F8 + 0, 0x01);
        outb(0x3F8 + 1, 0x00);
        outb(0x3F8 + 3, 0x03);
        outb(0x3F8 + 2, 0xC7);
        outb(0x3F8 + 4, 0x0B);
    }
}

/// Écrit un octet sur COM1.
fn serial_write_byte(b: u8) {
    unsafe {
        while (inb(0x3F8 + 5) & 0x20) == 0 {}
        outb(0x3F8, b);
    }
}

/// Écrit une chaîne UTF‑8 sur COM1 et ajoute un saut de ligne.
fn serial_print(s: &str) {
    for &b in s.as_bytes() {
        serial_write_byte(b);
    }
    serial_write_byte(b'\n');
}

/// Écrit des `format_args!` sur COM1 sans allocation puis ajoute un saut de ligne
fn serial_println_args(args: core::fmt::Arguments) {
    use core::fmt::Write;
    struct SW;
    impl Write for SW {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for &b in s.as_bytes() {
                serial_write_byte(b);
            }
            Ok(())
        }
    }
    let mut w = SW;
    let _ = w.write_fmt(args);
    serial_write_byte(b'\n');
}

#[macro_export]
macro_rules! serial_println {
    () => { $crate::serial_print("") };
    ($fmt:expr) => { $crate::serial_println_args(core::format_args!($fmt)) };
    ($fmt:expr, $($arg:tt)*) => { $crate::serial_println_args(core::format_args!($fmt, $($arg)*)) };
}

/// Initialise l’allocateur global sur une zone statique alignée.
fn init_heap() {
    unsafe {
        let start = core::ptr::addr_of_mut!(HEAP.buf[0]) as usize;
        GLOBAL_ALLOC.init(start, HEAP_SIZE);
    }
}

entry_point!(kernel_main);

/// Point d’entrée du noyau : init heap/série, Hello VGA/série,
/// intégration FAT32 (listage racine et lecture `HELLO.TXT`),
/// puis boucle infinie.
fn kernel_main(_boot_info: &'static BootInfo) -> ! {
    #[cfg(test)]
    {
        init_heap();
        serial_init();
        test_main();
        exit_qemu(QemuExitCode::Success);
        loop { core::hint::spin_loop(); }
    }

    #[cfg(not(test))]
    {
        init_heap();
        serial_init();

        crate::vga::vga_set_colors(crate::vga::Color::White, crate::vga::Color::Black);
        crate::vga::vga_clear();
        vga_println!("==============================");
        vga_println!(" The Heap – kernel");
        vga_println!("==============================");
        vga_println!();
        serial_println!("The Heap – kernel");

        let img = build_demo_fat32_image();
        if let Ok(fs) = Fat32::new(&img) {
            if let Ok(entries) = fs.list_root() {
                let mut s = alloc::string::String::from("ROOT: ");
                for (i, e) in entries.iter().enumerate() {
                    if i > 0 { s.push(' '); }
                    s.push_str(&e.name);
                }
                vga_println!("{}", s);
                serial_print(&s);
            }
            if let Ok(content) = fs.read_file_by_path("/HELLO.TXT") {
                if let Some(bytes) = content {
                    if let Ok(text) = core::str::from_utf8(&bytes) {
                        vga_println!("{}", text);
                        serial_print(text);
                    }
                }
            }
        }

        use alloc::string::String;
        use alloc::vec::Vec;

        let mut v: Vec<u32> = Vec::with_capacity(64);
        for i in 0..32 {
            v.push(i);
        }
        let s = String::from("The Heap: allocator OK");
        serial_print(&s);
        let _ = (v, s);

        loop { core::hint::spin_loop(); }
    }
}

#[cfg(not(test))]
/// Construit une image FAT32 miniature en mémoire pour la démo.
fn build_demo_fat32_image() -> alloc::vec::Vec<u8> {
    use alloc::vec;
    const SECTOR_SIZE: usize = 512;
    const NUM_SECTORS: usize = 10;
    let mut disk = vec![0u8; SECTOR_SIZE * NUM_SECTORS];
    {
        let b = &mut disk[0..SECTOR_SIZE];
        b[11] = 0x00;
        b[12] = 0x02;
        b[13] = 0x01;
        b[14] = 0x01;
        b[15] = 0x00;
        b[16] = 0x01;
        b[36] = 0x01;
        b[37] = 0x00;
        b[38] = 0x00;
        b[39] = 0x00;
        b[44] = 0x02;
        b[45] = 0x00;
        b[46] = 0x00;
        b[47] = 0x00;
    }
    {
        let fat_start = SECTOR_SIZE;
        let fat = &mut disk[fat_start..fat_start + SECTOR_SIZE];
        let eoc = 0x0FFF_FFFFu32.to_le_bytes();
        fat[2 * 4..2 * 4 + 4].copy_from_slice(&eoc);
        fat[3 * 4..3 * 4 + 4].copy_from_slice(&eoc);
        fat[4 * 4..4 * 4 + 4].copy_from_slice(&eoc);
    }
    {
        let root_off = 2 * SECTOR_SIZE;
        let dir = &mut disk[root_off..root_off + SECTOR_SIZE];
        let mut hello = [0u8; 32];
        hello[0..8].copy_from_slice(b"HELLO   ");
        hello[8..11].copy_from_slice(b"TXT");
        hello[11] = 0x20;
        hello[26] = 0x03;
        hello[27] = 0x00;
        hello[28] = 5;
        dir[0..32].copy_from_slice(&hello);
        let mut subdir = [0u8; 32];
        subdir[0..8].copy_from_slice(b"DIR     ");
        subdir[8..11].copy_from_slice(b"   ");
        subdir[11] = 0x10;
        subdir[26] = 0x04;
        subdir[27] = 0x00;
        dir[32..64].copy_from_slice(&subdir);
        dir[64] = 0x00;
    }
    {
        let off = 3 * SECTOR_SIZE;
        disk[off..off + 5].copy_from_slice(b"HELLO");
    }
    {
        let off = 4 * SECTOR_SIZE;
        disk[off] = 0x00;
    }
    disk
}

#[panic_handler]
/// Panic handler : en mode test, signale l’échec à QEMU (port 0xF4).
fn panic(info: &PanicInfo) -> ! {
    #[cfg(test)]
    {
        exit_qemu(QemuExitCode::Failed);
    }
    #[cfg(not(test))]
    {
        // Assure que COM1 est configuré
        serial_init();
        // Met la couleur VGA sur rouge vif
        crate::vga::vga_set_colors(crate::vga::Color::LightRed, crate::vga::Color::Black);
        vga_println!();
        vga_println!("=== PANIC ===");
        serial_println_args(core::format_args!("=== PANIC ==="));

        {
            let msg = info.message();
            vga_println!("{}", msg);
            serial_println_args(core::format_args!("{}", msg));
        }

        if let Some(loc) = info.location() {
            vga_println!("at {}:{}:{}", loc.file(), loc.line(), loc.column());
            serial_println_args(core::format_args!("at {}:{}:{}", loc.file(), loc.line(), loc.column()));
        }
    }
    loop { core::hint::spin_loop(); }
}

#[alloc_error_handler]
/// Gestionnaire d’échec d’allocation : en mode test, signale l’échec à QEMU.
fn alloc_error(_layout: core::alloc::Layout) -> ! {
    #[cfg(test)]
    {
        exit_qemu(QemuExitCode::Failed);
    }
    loop { core::hint::spin_loop(); }
}

#[cfg(test)]
/// Test runner minimal pour le framework de test custom.
fn test_runner(tests: &[&dyn Fn()]) {
    for t in tests {
        t();
    }
}

#[cfg(test)]
/// Test de base : allocation sur le heap global.
#[test_case]
fn heap_alloc_works() {
    let mut v = alloc::vec::Vec::new();
    v.push(1);
    assert_eq!(v.len(), 1);
}

#[cfg(test)]
fn build_test_fat32_image() -> alloc::vec::Vec<u8> {
    use alloc::vec;
    const S: usize = 512;
    let mut disk = vec![0u8; S * 10];
    {
        let b = &mut disk[0..S];
        b[11] = 0x00;
        b[12] = 0x02;
        b[13] = 0x01;
        b[14] = 0x01;
        b[15] = 0x00;
        b[16] = 0x01;
        b[36] = 0x01;
        b[37] = 0x00;
        b[38] = 0x00;
        b[39] = 0x00;
        b[44] = 0x02;
        b[45] = 0x00;
        b[46] = 0x00;
        b[47] = 0x00;
    }
    {
        let fat = &mut disk[S..S * 2];
        let eoc = 0x0FFF_FFFFu32.to_le_bytes();
        fat[2 * 4..2 * 4 + 4].copy_from_slice(&eoc);
        fat[3 * 4..3 * 4 + 4].copy_from_slice(&eoc);
        fat[4 * 4..4 * 4 + 4].copy_from_slice(&eoc);
    }
    {
        let dir = &mut disk[2 * S..3 * S];
        let mut hello = [0u8; 32];
        hello[0..8].copy_from_slice(b"HELLO   ");
        hello[8..11].copy_from_slice(b"TXT");
        hello[11] = 0x20;
        hello[26] = 0x03;
        hello[27] = 0x00;
        hello[28] = 5;
        dir[0..32].copy_from_slice(&hello);
        dir[64] = 0x00;
    }
    {
        let off = 3 * S;
        disk[off..off + 5].copy_from_slice(b"HELLO");
    }
    disk
}

#[cfg(test)]
#[test_case]
fn kernel_fat32_alloc_integration() {
    let img = build_test_fat32_image();
    let fs = fat32_parser::Fat32::new(&img).unwrap();
    let content = fs.read_file_by_path("/HELLO.TXT").unwrap().unwrap();
    assert_eq!(content, b"HELLO");
}
