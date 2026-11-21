//! macOS's malloc keeps every freed page charged to the process until memory pressure, so a
//! burst of large transfers stays as its high-water. Blocks from `THRESHOLD` up map straight
//! from the VM and unmap on free, which glibc does for itself once its threshold is pinned.

use std::alloc::{GlobalAlloc, Layout, System};

const THRESHOLD: usize = 128 * 1024;

pub struct LargeBlocks;

fn mapped(layout: Layout) -> bool {
    // mmap aligns to a page, at least 4 KiB everywhere.
    layout.size() >= THRESHOLD && layout.align() <= 4096
}

// SAFETY: a block goes back to whichever of `System` and the VM it came from, decided by the
// same `Layout` on both sides.
unsafe impl GlobalAlloc for LargeBlocks {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !mapped(layout) {
            return System.alloc(layout);
        }
        let block = libc::mmap(
            std::ptr::null_mut(),
            layout.size(),
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        );
        if block == libc::MAP_FAILED {
            std::ptr::null_mut()
        } else {
            block.cast()
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if mapped(layout) {
            // Fresh anonymous pages are zero.
            self.alloc(layout)
        } else {
            System.alloc_zeroed(layout)
        }
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        if mapped(layout) {
            libc::munmap(block.cast(), layout.size());
        } else {
            System.dealloc(block, layout);
        }
    }

    unsafe fn realloc(&self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
        if !mapped(layout) && !mapped(new_layout) {
            return System.realloc(block, layout, new_size);
        }
        let moved = self.alloc(new_layout);
        if !moved.is_null() {
            std::ptr::copy_nonoverlapping(block, moved, layout.size().min(new_size));
            self.dealloc(block, layout);
        }
        moved
    }
}
