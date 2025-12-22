#![allow(dead_code)]

pub const fn align_up(x: usize, align: usize) -> usize {
    (x + align - 1) & !(align - 1)
}

pub const fn is_power_of_two(x: usize) -> bool {
    x != 0 && (x & (x - 1)) == 0
}
