#![no_std]
#![no_main]

// The linker is told to start here (/ENTRY:start in build.rs). There's no
// CRT startup code in front of it, so this really is the first instruction
// of the process.
#[unsafe(no_mangle)]
pub extern "C" fn start() -> ! {
    better16_bootstrapper::exit(better16_bootstrapper::run())
}
