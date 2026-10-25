//! Better16 Launcher: installs, updates and starts the Better16 player.
//!
//! No std, no CRT, every import picked by hand, so the same exe runs on
//! Windows XP SP2 and on Windows 11. See `win.rs` for the import list and
//! `rt.rs` for what stands in for the C runtime.
//!
//! Unit tests build against std on the host as usual: `cargo test --lib`.

#![cfg_attr(not(test), no_std)]

#[cfg(not(target_arch = "x86"))]
compile_error!("build for i686-pc-windows-msvc; the runtime shims in rt.rs are 32-bit x86 only");

extern crate alloc;

mod app;
mod config;
mod error;
mod fs;
mod http;
mod install;
mod link;
mod log;
mod md5;
mod reg;
#[cfg(not(test))]
mod rt;
mod shell;
mod ui;
#[cfg_attr(test, allow(dead_code))]
mod win;
mod zip;

pub use app::run;

/// Ends the process. For main.rs, which has no other way in.
pub fn exit(code: u32) -> ! {
    unsafe { win::ExitProcess(code) }
}
