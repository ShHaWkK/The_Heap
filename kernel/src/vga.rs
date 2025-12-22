pub fn print_at_top(s: &str, color: u8) {
    let buffer = 0xb8000 as *mut u8;
    for (col, &b) in s.as_bytes().iter().enumerate() {
        unsafe {
            core::ptr::write_volatile(buffer.add(col * 2), b);
            core::ptr::write_volatile(buffer.add(col * 2 + 1), color);
        }
    }
}
