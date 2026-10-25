//! A plain text log in %TEMP%\Better16\launcher.log.
//!
//! When something goes wrong on someone else's machine this file is usually
//! all there is to go on, so every step that touches the network, the disk
//! or the registry writes a line.

use alloc::format;
use alloc::string::String;
use core::fmt;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::{fs, win};

static FILE: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(core::ptr::null_mut());

const MAX_SIZE: u64 = 512 * 1024;

pub fn path() -> String {
    format!("{}launcher.log", fs::temp_dir())
}

pub fn open() {
    let path = path();

    // Start over once it grows past a size anyone would want to read.
    let disposition = match fs::size(&path) {
        Some(n) if n > MAX_SIZE => win::CREATE_ALWAYS,
        _ => win::OPEN_ALWAYS,
    };

    let h = unsafe {
        win::CreateFileW(
            win::wide(&path).as_ptr(),
            win::FILE_APPEND_DATA,
            win::FILE_SHARE_READ | win::FILE_SHARE_WRITE,
            core::ptr::null_mut(),
            disposition,
            win::FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        )
    };
    if h != win::INVALID_HANDLE_VALUE {
        FILE.store(h, Ordering::Release);
    }
}

pub fn write(args: fmt::Arguments) {
    let h = FILE.load(Ordering::Acquire);
    if h.is_null() {
        return;
    }

    let mut t: win::SystemTime = unsafe { core::mem::zeroed() };
    unsafe { win::GetLocalTime(&mut t) };

    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}  {}\r\n",
        t.year, t.month, t.day, t.hour, t.minute, t.second, t.milliseconds, args
    );

    // One WriteFile per line; with FILE_APPEND_DATA that keeps lines from
    // two threads from interleaving.
    let mut written = 0;
    unsafe {
        win::WriteFile(h, line.as_ptr(), line.len() as u32, &mut written, core::ptr::null_mut());
    }
}

macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::write(format_args!($($arg)*))
    };
}
pub(crate) use log;
