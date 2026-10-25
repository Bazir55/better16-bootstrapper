//! The slice of the Win32 API the launcher uses, declared by hand.
//!
//! Every function here exists on Windows XP SP2. That's the whole reason
//! this file is hand-written instead of pulled from a bindings crate: it's
//! the list to check when adding a call. Things that look harmless but are
//! Vista-only include RegDeleteTreeW, GetTickCount64, SHGetKnownFolderPath,
//! InitializeSRWLock and anything named *Ex2.
//!
//! Imports are raw-dylib, so no import libraries from the Windows SDK are
//! needed to link.

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;

pub type Handle = *mut c_void;
pub type HWND = *mut c_void;
pub type HKEY = *mut c_void;
pub type BOOL = i32;

pub const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
pub const MAX_PATH: usize = 260;

macro_rules! import {
    ($dll:literal { $($item:tt)* }) => {
        // On 32-bit x86, stdcall names come decorated (_Foo@8). The DLLs
        // export them plain, hence import_name_type.
        #[cfg_attr(target_arch = "x86", link(name = $dll, kind = "raw-dylib", import_name_type = "undecorated"))]
        #[cfg_attr(not(target_arch = "x86"), link(name = $dll, kind = "raw-dylib"))]
        unsafe extern "system" { $($item)* }
    };
}
pub(crate) use import;

#[repr(C)]
pub struct SecurityAttributes {
    pub length: u32,
    pub descriptor: *mut c_void,
    pub inherit: BOOL,
}

#[repr(C)]
pub struct StartupInfo {
    pub cb: u32,
    pub reserved: *mut u16,
    pub desktop: *mut u16,
    pub title: *mut u16,
    pub x: u32,
    pub y: u32,
    pub x_size: u32,
    pub y_size: u32,
    pub x_chars: u32,
    pub y_chars: u32,
    pub fill_attribute: u32,
    pub flags: u32,
    pub show_window: u16,
    pub reserved2_len: u16,
    pub reserved2: *mut u8,
    pub std_input: Handle,
    pub std_output: Handle,
    pub std_error: Handle,
}

#[repr(C)]
pub struct ProcessInformation {
    pub process: Handle,
    pub thread: Handle,
    pub process_id: u32,
    pub thread_id: u32,
}

#[repr(C)]
pub struct SystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

#[repr(C)]
pub struct FindData {
    pub attributes: u32,
    // FILETIMEs. Not u64: that would be 8-byte aligned on x86 and shift
    // every field after it.
    pub creation_time: [u32; 2],
    pub last_access_time: [u32; 2],
    pub last_write_time: [u32; 2],
    pub size_high: u32,
    pub size_low: u32,
    pub reserved0: u32,
    pub reserved1: u32,
    pub file_name: [u16; MAX_PATH],
    pub alternate_name: [u16; 14],
}

pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const FILE_SHARE_READ: u32 = 1;
pub const FILE_SHARE_WRITE: u32 = 2;
pub const FILE_SHARE_DELETE: u32 = 4;
pub const CREATE_ALWAYS: u32 = 2;
pub const OPEN_EXISTING: u32 = 3;
pub const OPEN_ALWAYS: u32 = 4;
pub const FILE_APPEND_DATA: u32 = 4;
pub const FILE_ATTRIBUTE_READONLY: u32 = 0x01;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
pub const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
pub const FILE_BEGIN: u32 = 0;
pub const MOVEFILE_REPLACE_EXISTING: u32 = 1;

pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_ALREADY_EXISTS: u32 = 183;

pub const WAIT_OBJECT_0: u32 = 0;
pub const WAIT_ABANDONED: u32 = 0x80;
pub const WAIT_TIMEOUT: u32 = 258;


pub const FORMAT_MESSAGE_IGNORE_INSERTS: u32 = 0x200;
pub const FORMAT_MESSAGE_FROM_HMODULE: u32 = 0x800;
pub const FORMAT_MESSAGE_FROM_SYSTEM: u32 = 0x1000;

import!("kernel32" {
    pub fn GetProcessHeap() -> Handle;
    pub fn HeapAlloc(heap: Handle, flags: u32, bytes: usize) -> *mut c_void;
    pub fn HeapReAlloc(heap: Handle, flags: u32, mem: *mut c_void, bytes: usize) -> *mut c_void;
    pub fn HeapFree(heap: Handle, flags: u32, mem: *mut c_void) -> BOOL;
    pub fn ExitProcess(code: u32) -> !;
    pub fn GetLastError() -> u32;
    pub fn CloseHandle(h: Handle) -> BOOL;
    pub fn Sleep(ms: u32);
    pub fn GetLocalTime(t: *mut SystemTime);
    pub fn GetModuleHandleW(name: *const u16) -> Handle;
    pub fn GetModuleFileNameW(module: Handle, buf: *mut u16, size: u32) -> u32;
    pub fn GetCommandLineW() -> *const u16;
    pub fn GetTempPathW(size: u32, buf: *mut u16) -> u32;
    pub fn LocalFree(mem: *mut c_void) -> *mut c_void;
    pub fn FormatMessageW(flags: u32, source: *const c_void, id: u32, lang: u32, buf: *mut u16, size: u32, args: *mut c_void) -> u32;

    pub fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut SecurityAttributes, disposition: u32, flags: u32, template: Handle) -> Handle;
    pub fn ReadFile(h: Handle, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut c_void) -> BOOL;
    pub fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut c_void) -> BOOL;
    pub fn SetFilePointerEx(h: Handle, distance: i64, new_pos: *mut i64, method: u32) -> BOOL;
    pub fn GetFileSizeEx(h: Handle, size: *mut i64) -> BOOL;
    pub fn DeleteFileW(name: *const u16) -> BOOL;
    pub fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> BOOL;
    pub fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: BOOL) -> BOOL;
    pub fn CreateDirectoryW(name: *const u16, sa: *mut SecurityAttributes) -> BOOL;
    pub fn RemoveDirectoryW(name: *const u16) -> BOOL;
    pub fn GetFileAttributesW(name: *const u16) -> u32;
    pub fn SetFileAttributesW(name: *const u16, attrs: u32) -> BOOL;
    pub fn FindFirstFileW(pattern: *const u16, data: *mut FindData) -> Handle;
    pub fn FindNextFileW(h: Handle, data: *mut FindData) -> BOOL;
    pub fn FindClose(h: Handle) -> BOOL;
    pub fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> BOOL;

    pub fn CreateThread(sa: *mut SecurityAttributes, stack: usize, start: unsafe extern "system" fn(*mut c_void) -> u32, param: *mut c_void, flags: u32, id: *mut u32) -> Handle;
    pub fn CreateMutexW(sa: *mut SecurityAttributes, owned: BOOL, name: *const u16) -> Handle;
    pub fn ReleaseMutex(h: Handle) -> BOOL;
    pub fn CreateEventW(sa: *mut SecurityAttributes, manual_reset: BOOL, initial: BOOL, name: *const u16) -> Handle;
    pub fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    pub fn WaitForMultipleObjects(count: u32, handles: *const Handle, all: BOOL, ms: u32) -> u32;
    pub fn CreateProcessW(app: *const u16, cmd: *mut u16, psa: *mut SecurityAttributes, tsa: *mut SecurityAttributes, inherit: BOOL, flags: u32, env: *mut c_void, dir: *const u16, si: *mut StartupInfo, pi: *mut ProcessInformation) -> BOOL;
});

import!("user32" {
    pub fn MessageBoxW(owner: HWND, text: *const u16, caption: *const u16, flags: u32) -> i32;
    pub fn AllowSetForegroundWindow(pid: u32) -> BOOL;
});

import!("shell32" {
    pub fn CommandLineToArgvW(cmd: *const u16, argc: *mut i32) -> *mut *mut u16;
    pub fn SHGetFolderPathW(owner: HWND, csidl: i32, token: Handle, flags: u32, path: *mut u16) -> i32;
    pub fn ShellExecuteW(owner: HWND, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> Handle;
    pub fn SHChangeNotify(event: i32, flags: u32, item1: *const c_void, item2: *const c_void);
});

pub struct Owned(pub Handle);

impl Owned {
    pub fn is_valid(&self) -> bool {
        !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if self.is_valid() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// NUL-terminated UTF-16 copy of `s`, for passing to a W function.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Reads UTF-16 up to the first NUL (or the end of the slice).
pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    char::decode_utf16(buf[..len].iter().copied())
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Reads a NUL-terminated UTF-16 string the OS handed us.
///
/// # Safety
/// `p` must point at a NUL-terminated string.
pub unsafe fn from_wide_ptr(p: *const u16) -> String {
    // Volatile, or LLVM recognises the loop and emits a call to wcslen,
    // which isn't there to call.
    let mut len = 0;
    while unsafe { p.add(len).read_volatile() } != 0 {
        len += 1;
    }
    from_wide(unsafe { core::slice::from_raw_parts(p, len) })
}

pub fn last_error() -> u32 {
    unsafe { GetLastError() }
}

/// The system's text for an error code, e.g. "Access is denied." Codes in
/// the 12000 range belong to WinINet and are looked up there.
pub fn error_text(code: u32) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe {
        if (12000..13000).contains(&code) {
            let module = GetModuleHandleW(wide("wininet.dll").as_ptr());
            FormatMessageW(
                FORMAT_MESSAGE_FROM_HMODULE | FORMAT_MESSAGE_IGNORE_INSERTS,
                module,
                code,
                0,
                buf.as_mut_ptr(),
                buf.len() as u32,
                core::ptr::null_mut(),
            )
        } else {
            FormatMessageW(
                FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
                core::ptr::null(),
                code,
                0,
                buf.as_mut_ptr(),
                buf.len() as u32,
                core::ptr::null_mut(),
            )
        }
    };
    let text = from_wide(&buf[..n as usize]);
    String::from(text.trim_end())
}

pub fn message_box(owner: HWND, text: &str, caption: &str, flags: u32) -> i32 {
    unsafe { MessageBoxW(owner, wide(text).as_ptr(), wide(caption).as_ptr(), flags) }
}

pub fn sleep(ms: u32) {
    unsafe { Sleep(ms) }
}

pub fn current_exe() -> String {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    from_wide(&buf[..n as usize])
}

/// Command line arguments, without the program name.
pub fn args() -> Vec<String> {
    let mut argc = 0;
    let mut out = Vec::new();
    unsafe {
        let argv = CommandLineToArgvW(GetCommandLineW(), &mut argc);
        if argv.is_null() {
            return out;
        }
        for i in 1..argc as usize {
            out.push(from_wide_ptr(*argv.add(i)));
        }
        LocalFree(argv.cast());
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn struct_sizes_match_the_sdk() {
        assert_eq!(core::mem::size_of::<super::FindData>(), 592);
        assert_eq!(core::mem::size_of::<super::StartupInfo>(), 68);
        assert_eq!(core::mem::size_of::<super::SystemTime>(), 16);
    }
}
