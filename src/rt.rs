//! The few things a C runtime would normally provide.
//!
//! The launcher links no CRT at all. The msvcrt.dll that ships with XP is
//! too old for modern compilers, the static CRTs from current Visual Studio
//! call Vista APIs, and Rust's std wants Windows 10. What's left to fill in
//! is small: an allocator, a panic handler, the mem* functions, and the
//! helpers LLVM calls on 32-bit x86 for stack probes and 64-bit division.

use core::alloc::{GlobalAlloc, Layout};
use core::arch::naked_asm;
use core::ffi::c_void;

use crate::win;

struct ProcessHeap;

// HeapAlloc hands out 8-byte aligned blocks on 32-bit Windows. Anything
// stricter gets an oversized block, with the real pointer stashed in the
// word just below the aligned one.
const HEAP_ALIGN: usize = 8;

unsafe impl GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            if layout.align() <= HEAP_ALIGN {
                return win::HeapAlloc(win::GetProcessHeap(), 0, layout.size()).cast();
            }
            let raw = win::HeapAlloc(win::GetProcessHeap(), 0, layout.size() + layout.align()) as *mut u8;
            if raw.is_null() {
                return raw;
            }
            let aligned = raw.add(layout.align() - (raw as usize & (layout.align() - 1)));
            *(aligned as *mut *mut u8).sub(1) = raw;
            aligned
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            let raw = if layout.align() <= HEAP_ALIGN { ptr } else { *(ptr as *mut *mut u8).sub(1) };
            win::HeapFree(win::GetProcessHeap(), 0, raw.cast());
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe {
            if layout.align() <= HEAP_ALIGN {
                return win::HeapReAlloc(win::GetProcessHeap(), 0, ptr.cast(), new_size).cast();
            }
            let new = self.alloc(Layout::from_size_align_unchecked(new_size, layout.align()));
            if !new.is_null() {
                core::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
            new
        }
    }
}

#[global_allocator]
static HEAP: ProcessHeap = ProcessHeap;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let text = alloc::format!("{info}");
    crate::log::write(format_args!("panic: {text}"));
    win::message_box(
        core::ptr::null_mut(),
        &alloc::format!("The launcher hit a bug and has to close.\n\n{text}"),
        "Better16",
        0x10,
    );
    unsafe { win::ExitProcess(3) }
}

// LLVM won't turn the loops below back into calls to themselves: it leaves
// functions with these names alone.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let (d, s) = (dst as *mut u8, src as *const u8);
    unsafe {
        let mut i = 0;
        while i + 4 <= n {
            (d.add(i) as *mut u32).write_unaligned((s.add(i) as *const u32).read_unaligned());
            i += 4;
        }
        while i < n {
            *d.add(i) = *s.add(i);
            i += 1;
        }
        dst
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let (d, s) = (dst as *mut u8, src as *const u8);
    unsafe {
        if (dst as usize) <= (src as usize) || (dst as usize) >= (src as usize) + n {
            return memcpy(dst, src, n);
        }
        // Overlapping with the destination above the source: copy from the
        // end. Volatile so this can't be "simplified" into a memmove call.
        let mut i = n;
        while i > 0 {
            i -= 1;
            d.add(i).write_volatile(s.add(i).read_volatile());
        }
        dst
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dst: *mut c_void, c: i32, n: usize) -> *mut c_void {
    let d = dst as *mut u8;
    unsafe {
        let byte = c as u8;
        let word = u32::from_ne_bytes([byte; 4]);
        let mut i = 0;
        while i + 4 <= n {
            (d.add(i) as *mut u32).write_unaligned(word);
            i += 4;
        }
        while i < n {
            *d.add(i) = byte;
            i += 1;
        }
        dst
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    let (a, b) = (a as *const u8, b as *const u8);
    unsafe {
        for i in 0..n {
            let (x, y) = (*a.add(i), *b.add(i));
            if x != y {
                return x as i32 - y as i32;
            }
        }
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    unsafe { memcmp(a, b, n) }
}

/// Floating point "is used" marker the MSVC toolchain expects.
#[unsafe(no_mangle)]
pub static _fltused: i32 = 0;

/// The precompiled core and alloc libraries carry C++-style exception
/// tables that name this handler. With panic=abort nothing ever unwinds
/// through them, so when Windows asks, say "not mine" and let the search
/// continue.
#[unsafe(no_mangle)]
pub extern "C" fn __CxxFrameHandler3(_: *mut c_void, _: *mut c_void, _: *mut c_void, _: *mut c_void) -> u32 {
    1 // ExceptionContinueSearch
}

/// Stack probe for frames bigger than a page. On 32-bit Windows the stack
/// grows one guard page at a time, so a big frame has to touch each page
/// on the way down before using it.
///
/// Called with the frame size in eax. Unlike the x64 version, this one also
/// moves esp down by that much before returning, which is what LLVM
/// expects on x86.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _chkstk() {
    naked_asm!(
        "push ecx",
        "push edx",
        "lea ecx, [esp + 12]",      // esp as the caller had it before the call
        "mov edx, ecx",
        "sub edx, eax",             // where esp has to end up
        "2:",
        "sub ecx, 0x1000",
        "cmp ecx, edx",
        "jbe 3f",
        "test dword ptr [ecx], eax",
        "jmp 2b",
        "3:",
        "test dword ptr [edx], eax",
        "mov eax, edx",
        "pop edx",
        "pop ecx",                  // esp now points at our return address
        "xchg eax, esp",
        "push dword ptr [eax]",
        "ret",
    )
}

// 64-bit division on 32-bit x86. LLVM emits calls to these MSVC helpers,
// which take both operands on the stack, return in edx:eax and pop their
// own arguments. The wrappers re-push the operands for an ordinary cdecl
// function and do the rest in Rust.

macro_rules! divide_helper {
    ($name:ident, $imp:ident) => {
        #[unsafe(naked)]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name() {
            naked_asm!(
                "push dword ptr [esp + 16]",
                "push dword ptr [esp + 16]",
                "push dword ptr [esp + 16]",
                "push dword ptr [esp + 16]",
                "call {imp}",
                "add esp, 16",
                "ret 16",
                imp = sym $imp,
            )
        }
    };
}

divide_helper!(_aulldiv, udiv);
divide_helper!(_aullrem, urem);
divide_helper!(_alldiv, sdiv);
divide_helper!(_allrem, srem);

extern "C" fn udiv(n: u64, d: u64) -> u64 {
    divmod(n, d).0
}

extern "C" fn urem(n: u64, d: u64) -> u64 {
    divmod(n, d).1
}

extern "C" fn sdiv(n: i64, d: i64) -> i64 {
    let q = divmod(n.unsigned_abs(), d.unsigned_abs()).0 as i64;
    if (n < 0) != (d < 0) { q.wrapping_neg() } else { q }
}

extern "C" fn srem(n: i64, d: i64) -> i64 {
    let r = divmod(n.unsigned_abs(), d.unsigned_abs()).1 as i64;
    if n < 0 { r.wrapping_neg() } else { r }
}

/// Shift-and-subtract long division, with a fast path when both halves fit
/// in 32 bits. Written without `/` on u64, for obvious reasons.
fn divmod(n: u64, d: u64) -> (u64, u64) {
    if d == 0 {
        unsafe { win::ExitProcess(4) }
    }
    if n < d {
        return (0, n);
    }
    if n >> 32 == 0 {
        let (n, d) = (n as u32, d as u32);
        return ((n / d) as u64, (n % d) as u64);
    }
    let shift = d.leading_zeros() - n.leading_zeros();
    let mut d = d << shift;
    let mut r = n;
    let mut q = 0u64;
    for _ in 0..=shift {
        q <<= 1;
        if r >= d {
            r -= d;
            q |= 1;
        }
        d >>= 1;
    }
    (q, r)
}
