#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

#[cfg(not(test))]
extern crate alloc;

mod vga;

#[cfg(not(test))]
use bootloader::BootInfo;
#[cfg(not(test))]
use bootloader::entry_point;
#[cfg(not(test))]
use fat32_parser::Fat32;
#[cfg(not(test))]
use core::panic::PanicInfo;
#[cfg(not(test))]
use slaballoc::LockedAlloc;

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOC: LockedAlloc = LockedAlloc::new();

#[cfg(not(test))]
const HEAP_SIZE: usize = 512 * 1024;

#[cfg(not(test))]
#[repr(align(64))]
struct AlignedHeap<const N: usize> { buf: [u8; N] }

#[cfg(not(test))]
static mut HEAP: AlignedHeap<HEAP_SIZE> = AlignedHeap { buf: [0; HEAP_SIZE] };

#[cfg(not(test))]
#[inline(always)]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

#[cfg(not(test))]
#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let mut v: u8;
    core::arch::asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags));
    v
}

#[cfg(not(test))]
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

#[cfg(not(test))]
fn serial_write_byte(b: u8) {
    unsafe {
        while (inb(0x3F8 + 5) & 0x20) == 0 {}
        outb(0x3F8, b);
    }
}

#[cfg(not(test))]
fn serial_print(s: &str) {
    for &b in s.as_bytes() {
        serial_write_byte(b);
    }
    serial_write_byte(b'\n');
}

#[cfg(not(test))]
fn init_heap() {
    unsafe {
        let start = core::ptr::addr_of_mut!(HEAP.buf[0]) as usize;
        GLOBAL_ALLOC.init(start, HEAP_SIZE);
    }
}

#[cfg(not(test))]
entry_point!(kernel_main);

#[cfg(not(test))]
fn kernel_main(_boot_info: &'static BootInfo) -> ! {
    init_heap();
    serial_init();

    crate::vga::print_at_row("Hello VGA from The Heap", 0x0F, 0);
    serial_print("Hello serial from The Heap");

    let img = build_demo_fat32_image();
    if let Ok(fs) = Fat32::new(&img) {
        if let Ok(entries) = fs.list_root() {
            let mut s = alloc::string::String::from("ROOT: ");
            for (i, e) in entries.iter().enumerate() {
                if i > 0 { s.push(' '); }
                s.push_str(&e.name);
            }
            crate::vga::print_at_row(&s, 0x0F, 1);
            serial_print(&s);
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

#[cfg(not(test))]
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

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! { loop { core::hint::spin_loop(); } }

#[cfg(test)]
fn main() {}
