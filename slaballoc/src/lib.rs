#![cfg_attr(not(test), no_std)]

mod allocator;
mod util;

pub use allocator::SlabAllocator;

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::hint::spin_loop as cpu_relax;
use core::sync::atomic::{AtomicBool, Ordering};

struct SpinLock<T> {
    locked: AtomicBool,
    inner: UnsafeCell<T>,
}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self { locked: AtomicBool::new(false), inner: UnsafeCell::new(value) }
    }

    fn lock(&self) -> SpinLockGuard<'_, T> {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            cpu_relax();
        }
        SpinLockGuard { lock: self }
    }
}

unsafe impl<T: Send> Sync for SpinLock<T> {}
unsafe impl<T: Send> Send for SpinLock<T> {}

struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<'a, T> core::ops::Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target { unsafe { &*self.lock.inner.get() } }
}

impl<'a, T> core::ops::DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target { unsafe { &mut *self.lock.inner.get() } }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) { self.lock.locked.store(false, Ordering::Release); }
}

pub struct LockedAlloc(SpinLock<SlabAllocator>);

impl LockedAlloc {
    pub const fn new() -> Self { Self(SpinLock::new(SlabAllocator::new())) }

    /// # Safety
    /// `heap_start..heap_start+heap_size` doit désigner une région valide, unique,
    /// accessible en lecture/écriture, initialisée avant toute allocation.
    pub unsafe fn init(&self, heap_start: usize, heap_size: usize) {
        let mut g = self.0.lock();
        unsafe { g.init(heap_start, heap_size) };
    }
}

impl Default for LockedAlloc {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl GlobalAlloc for LockedAlloc {
    /// # Safety
    /// Respecte le contrat de `GlobalAlloc`: `layout` doit être valide.
    /// La région fournie à `init` doit rester exclusive au runtime.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut g = self.0.lock();
        g.alloc(layout)
    }

    /// # Safety
    /// Le pointeur `ptr` doit provenir d’un précédent `alloc` avec le même `layout`.
    /// Pas de double free ni de mutation concurrente au même emplacement.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut g = self.0.lock();
        g.dealloc(ptr, layout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;

    #[test]
    fn small_alloc_dealloc() {
        let mut buf = vec![0u8; 8192];
        let alloc = LockedAlloc::new();
        unsafe { alloc.init(buf.as_mut_ptr() as usize, buf.len()) }

        let l = Layout::from_size_align(32, 8).unwrap();
        let p = unsafe { alloc.alloc(l) };
        assert!(!p.is_null());
        unsafe { alloc.dealloc(p, l) };
        let q = unsafe { alloc.alloc(l) };
        assert!(!q.is_null());
    }

    #[test]
    fn refill_many_objects() {
        let mut buf = vec![0u8; 64 * 1024];
        let alloc = LockedAlloc::new();
        unsafe { alloc.init(buf.as_mut_ptr() as usize, buf.len()) }

        let l = Layout::from_size_align(16, 8).unwrap();
        let mut ptrs = [core::ptr::null_mut(); 200];
        for i in 0..ptrs.len() {
            ptrs[i] = unsafe { alloc.alloc(l) };
            assert!(!ptrs[i].is_null());
        }
        for &p in &ptrs {
            unsafe { alloc.dealloc(p, l) };
        }
        for i in 0..ptrs.len() {
            ptrs[i] = unsafe { alloc.alloc(l) };
            assert!(!ptrs[i].is_null());
        }
    }

    #[test]
    fn alignment_constraints_respected() {
        let mut buf = vec![0u8; 32 * 1024];
        let alloc = LockedAlloc::new();
        unsafe { alloc.init(buf.as_mut_ptr() as usize, buf.len()) }

        let l = Layout::from_size_align(24, 32).unwrap();
        let p = unsafe { alloc.alloc(l) } as usize;
        assert_ne!(p, 0);
        assert_eq!(p % 32, 0);
    }

    #[test]
    fn boundary_classes_and_large() {
        let mut buf = vec![0u8; 128 * 1024];
        let alloc = LockedAlloc::new();
        unsafe { alloc.init(buf.as_mut_ptr() as usize, buf.len()) }

        let l_exact = Layout::from_size_align(4096, 64).unwrap();
        let p1 = unsafe { alloc.alloc(l_exact) } as usize;
        assert_ne!(p1, 0);

        let l_over = Layout::from_size_align(4097, 8).unwrap();
        let q1 = unsafe { alloc.alloc(l_over) } as usize;
        let q2 = unsafe { alloc.alloc(l_over) } as usize;
        assert_ne!(q1, 0);
        assert_ne!(q2, 0);
        assert_ne!(q1, q2);
    }

    #[test]
    fn freelist_reuse_lifo() {
        let mut buf = vec![0u8; 32 * 1024];
        let alloc = LockedAlloc::new();
        unsafe { alloc.init(buf.as_mut_ptr() as usize, buf.len()) }

        let l = Layout::from_size_align(64, 8).unwrap();
        let a = unsafe { alloc.alloc(l) } as usize;
        let b = unsafe { alloc.alloc(l) } as usize;
        unsafe { alloc.dealloc(a as *mut u8, l) };
        unsafe { alloc.dealloc(b as *mut u8, l) };
        let c = unsafe { alloc.alloc(l) } as usize;
        assert_eq!(c, b);
    }
}
