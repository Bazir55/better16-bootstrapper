#![no_std]
#![no_main]

// Better16StudioLauncher.exe. The same entry point as main.rs (see there),
// with Studio picked before anything else runs.
#[unsafe(no_mangle)]
pub extern "C" fn start() -> ! {
    better16_bootstrapper::exit(better16_bootstrapper::run_studio())
}
