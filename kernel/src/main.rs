#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader::entry_point;
use core::panic::PanicInfo;
use core::alloc::{Layout, GlobalAlloc};
use alloc::format;
use fat32_parser::{Fat32, Fat32Mut};
use slaballoc::LockedAlloc;

/// Écrit un octet sur un port d’E/S x86.
///
/// Safety
/// - Le port doit être valide pour le contexte matériel courant
/// - L’appel s’effectue en mode noyau sans I/O concurrente non maîtrisée
/// - Ne pas utiliser sur OS hôte: code destiné au kernel booté dans QEMU
#[inline(always)]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

/// Lit un octet depuis un port d’E/S x86.
///
/// Safety
/// - Le port doit être valide et lisible dans le contexte matériel courant
/// - Ne pas utiliser sur OS hôte: code destiné au kernel booté dans QEMU
#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let mut v: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack, preserves_flags));
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

#[global_allocator]
static GLOBAL_ALLOC: LockedAlloc = LockedAlloc::new();
static mut HEAP_SPACE: [u8; 128 * 1024] = [0; 128 * 1024];
const HEAP_SIZE: usize = 128 * 1024;

fn serial_init() {
    unsafe {
        // Safety: accès contrôlés aux registres de COM1 (0x3F8..)
        outb(0x3F8 + 1, 0x00);
        outb(0x3F8 + 3, 0x80);
        outb(0x3F8 + 0, 0x01);
        outb(0x3F8 + 1, 0x00);
        outb(0x3F8 + 3, 0x03);
        outb(0x3F8 + 2, 0xC7);
        outb(0x3F8 + 4, 0x0B);
    }
}

fn serial_write_byte(b: u8) {
    unsafe {
        // Safety: polling du LSR (Line Status Register) sur COM1
        while (inb(0x3F8 + 5) & 0x20) == 0 {}
        outb(0x3F8, b);
    }
}

fn serial_write_str(s: &str) {
    for &b in s.as_bytes() {
        serial_write_byte(b);
    }
    serial_write_byte(b'\n');
}

fn vga_write_line(row: usize, s: &str) {
    let base = 0xb8000 as *mut u8;
    let mut col = 0usize;
    for &b in s.as_bytes() {
        unsafe {
            let off = (row * 80 + col) * 2;
            core::ptr::write_volatile(base.add(off), b);
            core::ptr::write_volatile(base.add(off + 1), 0x07);
        }
        col += 1;
        if col >= 80 { break; }
    }
}

fn build_test_image() -> [u8; 5120] {
    const SECTOR_SIZE: usize = 512;
    const NUM_SECTORS: usize = 10;
    let mut disk = [0u8; SECTOR_SIZE * NUM_SECTORS];
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
        let eoc = 0x0F_FF_FF_F8u32.to_le_bytes();
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

fn demo() {
    unsafe {
        let p = core::ptr::addr_of_mut!(HEAP_SPACE) as *mut u8 as usize;
        GLOBAL_ALLOC.init(p, HEAP_SIZE);
    };
    serial_init();
    serial_write_str("The Heap - kernel");
    vga_write_line(0, "The Heap - kernel");
    let mut disk = build_test_image();
    let ro = Fat32::new(&disk).unwrap();
    let root = ro.list_root().unwrap();
    let mut names = alloc::vec::Vec::new();
    for e in root { names.push(e.name); }
    serial_write_str(&format!("ROOT: {}", names.join(" ")));
    vga_write_line(1, &format!("ROOT: {}", names.join(" ")));
    let hello = ro.read_file_by_path("/HELLO.TXT").unwrap().unwrap();
    let hello_s = core::str::from_utf8(&hello).unwrap();
    serial_write_str(hello_s);
    vga_write_line(2, hello_s);
    {
        let mut rw = Fat32Mut::new(&mut disk).unwrap();
        rw.write_file_by_path("/NEW.TXT", b"NEW!").unwrap();
    }
    let ro2 = Fat32::new(&disk).unwrap();
    let root2 = ro2.list_root().unwrap();
    let mut names2 = alloc::vec::Vec::new();
    for e in root2 { names2.push(e.name); }
    serial_write_str(&format!("ROOT (apres ecriture): {}", names2.join(" ")));
    vga_write_line(3, &format!("ROOT (apres ecriture): {}", names2.join(" ")));
    let newc = ro2.read_file_by_path("/NEW.TXT").unwrap().unwrap();
    let new_s = core::str::from_utf8(&newc).unwrap();
    serial_write_str(new_s);
    vga_write_line(4, new_s);
    let l = Layout::from_size_align(32, 8).unwrap();
    let p = unsafe { GLOBAL_ALLOC.alloc(l) };
    assert!(!p.is_null());
    unsafe { GLOBAL_ALLOC.dealloc(p, l) };
    serial_write_str("The Heap: allocator OK");
    vga_write_line(5, "The Heap: allocator OK");
}

entry_point!(kernel_main);

fn kernel_main(_: &'static bootloader::BootInfo) -> ! {
    #[cfg(test)]
    {
        demo();
        test_main();
        exit_qemu(QemuExitCode::Success);
        loop { core::hint::spin_loop(); }
    }
    #[cfg(not(test))]
    {
        demo();
        loop { core::hint::spin_loop(); }
    }
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    #[cfg(test)]
    {
        exit_qemu(QemuExitCode::Failed);
    }
    loop { core::hint::spin_loop(); }
}

#[cfg(test)]
fn test_runner(tests: &[&dyn Fn()]) {
    for t in tests {
        t();
    }
}

#[cfg(test)]
#[test_case]
fn it_works() {
    assert_eq!(1 + 1, 2);
}
