use core::alloc::Layout;
use core::ptr::NonNull;

use crate::util::{align_up, is_power_of_two};

/// Taille d'une "page" interne(c'est un chunk)
pub const CHUNK_SIZE: usize = 4096;

/// Classes de tailles (slabs)
const SIZE_CLASSES: &[usize] = &[
    8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096,
];

#[repr(C)]
struct FreeNode {
    next: Option<NonNull<FreeNode>>,
}

// Pointeurs vers des blocs bruts sur le heap du kernel.
// Leur transfert entre threads est sûr si l’allocator est protégé par un lock.
unsafe impl Send for FreeNode {}

/// Un cache de freelist par classe de taille.
#[derive(Copy, Clone)]
struct Cache {
    head: Option<NonNull<FreeNode>>,
}

impl Cache {
    const fn new() -> Self {
        Self { head: None }
    }
}

/// Etat interne de l'allocator (protégé par lock côté wrapper)
pub struct SlabAllocator {
    heap_start: usize,
    heap_end: usize,
    bump: usize,
    caches: [Cache; SIZE_CLASSES.len()],
    initialized: bool,
}

unsafe impl Send for SlabAllocator {}

impl SlabAllocator {
    pub const fn new() -> Self {
        Self {
            heap_start: 0,
            heap_end: 0,
            bump: 0,
            caches: [Cache::new(); SIZE_CLASSES.len()],
            initialized: false,
        }
    }

    /// Initialise l'allocator avec une région mémoire `[heap_start, heap_start + heap_size)`.
    ///
    /// # Safety
    /// - `heap_start..heap_start+heap_size` doit être une région valide, accessible en écriture.
    /// - Cette fonction doit être appelée une seule fois (avant toute alloc).
    /// - `heap_start` doit être aligné au moins à 16 (recommandé).
    pub unsafe fn init(&mut self, heap_start: usize, heap_size: usize) {
        self.heap_start = heap_start;
        self.heap_end = heap_start + heap_size;
        self.bump = heap_start;
        self.initialized = true;
    }

    fn class_index_for(layout: Layout) -> Option<usize> {
        let size = layout.size().max(layout.align());
        for (i, &cls) in SIZE_CLASSES.iter().enumerate() {
            if size <= cls {
                return Some(i);
            }
        }
        None
    }

    /// # Safety
    /// Écrit des zéros dans la région retournée et manipule des pointeurs bruts.
    /// Nécessite que `self` ait été initialisé sur une région exclusive et alignée.
    fn alloc_from_bump(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        debug_assert!(self.initialized);
        debug_assert!(is_power_of_two(layout.align()));

        let start = align_up(self.bump, layout.align());
        let end = start.checked_add(layout.size())?;
        if end > self.heap_end {
            return None;
        }
        self.bump = end;

        // Zero-fill (facilite debug/démo)
        unsafe {
            core::ptr::write_bytes(start as *mut u8, 0, layout.size());
            NonNull::new(start as *mut u8)
        }
    }

    /// # Safety
    /// Découpe un chunk en liste libre via écriture de `FreeNode` sur la région brute.
    /// Suppose que le chunk alloué ne chevauche pas d’objets actifs.
    fn refill_cache(&mut self, idx: usize) -> bool {
        let block_size = SIZE_CLASSES[idx];

        // On prend un chunk (4K) depuis bump
        let chunk_layout = Layout::from_size_align(CHUNK_SIZE, CHUNK_SIZE).ok();
        let chunk_layout = match chunk_layout {
            Some(l) => l,
            None => return false,
        };

        let chunk = match self.alloc_from_bump(chunk_layout) {
            Some(p) => p.as_ptr() as usize,
            None => return false,
        };

        // On découpe le chunk en blocs de block_size et on push dans freelist
        let mut off = chunk;
        let chunk_end = chunk + CHUNK_SIZE;

        while off + block_size <= chunk_end {
            unsafe {
                let node = off as *mut FreeNode;
                (*node).next = self.caches[idx].head;
                self.caches[idx].head = NonNull::new(node);
            }
            off += block_size;
        }

        true
    }

    /// # Safety
    /// Retourne un pointeur brut valide ou nul. Les classes petites utilisent la freelist,
    /// les grosses tailles consomment la région bump sans recyclage.
    pub fn alloc(&mut self, layout: Layout) -> *mut u8 {
        if !self.initialized {
            return core::ptr::null_mut();
        }

        if let Some(idx) = Self::class_index_for(layout) {
            // Fast path: pop freelist
            if self.caches[idx].head.is_none() && !self.refill_cache(idx) {
                return core::ptr::null_mut();
            }

            let head = match self.caches[idx].head {
                Some(h) => h,
                None => return core::ptr::null_mut(),
            };
            unsafe {
                self.caches[idx].head = (*head.as_ptr()).next;
                head.as_ptr() as *mut u8
            }
        } else {
            // Gros alloc => bump direct (simple)
            self.alloc_from_bump(layout)
                .map(|p| p.as_ptr())
                .unwrap_or(core::ptr::null_mut())
        }
    }

    /// # Safety
    /// `ptr` doit provenir d’un `alloc` avec ce `layout`. Pas de double free, pas de mélange
    /// de classes de taille. Les grosses tailles ne sont pas recyclées en V1.
    pub fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() || !self.initialized {
            return;
        }

        if let Some(idx) = Self::class_index_for(layout) {
            unsafe {
                let node = ptr as *mut FreeNode;
                (*node).next = self.caches[idx].head;
                self.caches[idx].head = NonNull::new(node);
            }
        } else {
            // bump allocations "gros": on ne récupère pas (simple, acceptable pour V1)
        }
    }
}

impl Default for SlabAllocator {
    fn default() -> Self {
        Self::new()
    }
}
