#[cfg(not(test))]
pub fn print_at_row(s: &str, color: u8, row: usize) {
    let buffer = 0xb8000 as *mut u8;
    let base = row * 80 * 2;
    for (col, &b) in s.as_bytes().iter().enumerate().take(80) {
        unsafe {
            core::ptr::write_volatile(buffer.add(base + col * 2), b);
            core::ptr::write_volatile(buffer.add(base + col * 2 + 1), color);
        }
    }
}
