#[cfg(not(test))]
use core::fmt;

#[cfg(not(test))]
struct Writer {
    row: usize,
    col: usize,
    attr: u8,
}

#[cfg(not(test))]
use core::cell::UnsafeCell;
#[cfg(not(test))]
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(test))]
struct SpinLock<T> {
    locked: AtomicBool,
    inner: UnsafeCell<T>,
}

#[cfg(not(test))]
impl<T> SpinLock<T> {
    const fn new(value: T) -> Self {
        Self { locked: AtomicBool::new(false), inner: UnsafeCell::new(value) }
    }
    fn lock(&self) -> SpinLockGuard<'_, T> {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {}
        SpinLockGuard { lock: self }
    }
}

#[cfg(not(test))]
unsafe impl<T: Send> Sync for SpinLock<T> {}
#[cfg(not(test))]
unsafe impl<T: Send> Send for SpinLock<T> {}

#[cfg(not(test))]
struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

#[cfg(not(test))]
impl<'a, T> core::ops::Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target { unsafe { &*self.lock.inner.get() } }
}

#[cfg(not(test))]
impl<'a, T> core::ops::DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target { unsafe { &mut *self.lock.inner.get() } }
}

#[cfg(not(test))]
impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) { self.lock.locked.store(false, Ordering::Release); }
}

#[cfg(not(test))]
static WRITER: SpinLock<Writer> = SpinLock::new(Writer { row: 0, col: 0, attr: 0x0F });

#[cfg(not(test))]
impl Writer {
    fn write_byte(&mut self, b: u8) {
        if b == b'\n' {
            self.newline();
            return;
        }
        let buffer = 0xb8000 as *mut u8;
        let idx = (self.row * 80 + self.col) * 2;
        unsafe {
            core::ptr::write_volatile(buffer.add(idx), b);
            core::ptr::write_volatile(buffer.add(idx + 1), self.attr);
        }
        self.col += 1;
        if self.col >= 80 {
            self.newline();
        }
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row < 24 {
            self.row += 1;
            return;
        }
        let buffer = 0xb8000 as *mut u8;
        for r in 0..24 {
            for c in 0..80 {
                let src = ((r + 1) * 80 + c) * 2;
                let dst = (r * 80 + c) * 2;
                unsafe {
                    let ch = core::ptr::read_volatile(buffer.add(src));
                    let at = core::ptr::read_volatile(buffer.add(src + 1));
                    core::ptr::write_volatile(buffer.add(dst), ch);
                    core::ptr::write_volatile(buffer.add(dst + 1), at);
                }
            }
        }
        for c in 0..80 {
            let idx = (24 * 80 + c) * 2;
            unsafe {
                core::ptr::write_volatile(buffer.add(idx), b' ');
                core::ptr::write_volatile(buffer.add(idx + 1), self.attr);
            }
        }
        self.row = 24;
    }
}

#[cfg(not(test))]
impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            self.write_byte(b);
        }
        Ok(())
    }
}

#[cfg(not(test))]
#[allow(dead_code)]
pub fn vga_set_color(fg: u8, bg: u8) {
    let a = ((bg & 0x0F) << 4) | (fg & 0x0F);
    let mut g = WRITER.lock();
    g.attr = a;
}

#[cfg(not(test))]
#[allow(dead_code)]
#[repr(u8)]
pub enum Color {
    Black = 0x00,
    Blue = 0x01,
    Green = 0x02,
    Cyan = 0x03,
    Red = 0x04,
    Magenta = 0x05,
    Brown = 0x06,
    LightGray = 0x07,
    DarkGray = 0x08,
    LightBlue = 0x09,
    LightGreen = 0x0A,
    LightCyan = 0x0B,
    LightRed = 0x0C,
    Pink = 0x0D,
    Yellow = 0x0E,
    White = 0x0F,
}

#[cfg(not(test))]
pub fn vga_set_colors(foreground: Color, background: Color) {
    let a = ((background as u8 & 0x0F) << 4) | (foreground as u8 & 0x0F);
    let mut g = WRITER.lock();
    g.attr = a;
}

#[cfg(not(test))]
pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    let mut g = WRITER.lock();
    let _ = g.write_fmt(args);
}

#[cfg(not(test))]
pub fn vga_clear() {
    let buffer = 0xb8000 as *mut u8;
    let mut g = WRITER.lock();
    for r in 0..25 {
        for c in 0..80 {
            let idx = (r * 80 + c) * 2;
            unsafe {
                core::ptr::write_volatile(buffer.add(idx), b' ');
                core::ptr::write_volatile(buffer.add(idx + 1), g.attr);
            }
        }
    }
    g.row = 0;
    g.col = 0;
}

#[cfg(not(test))]
#[macro_export]
macro_rules! vga_print {
    ($($arg:tt)*) => {
        $crate::vga::_print(core::format_args!($($arg)*));
    };
}

#[cfg(not(test))]
#[macro_export]
macro_rules! vga_println {
    () => {
        $crate::vga::_print(core::format_args!("\n"))
    };
    ($fmt:expr) => {
        $crate::vga::_print(core::format_args!(core::concat!($fmt, "\n")))
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::vga::_print(core::format_args!(core::concat!($fmt, "\n"), $($arg)*))
    };
}
