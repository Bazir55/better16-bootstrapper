//! The DACL the player is created with.
//!
//! The client closes its own DACL first thing in WinMain, but VMProtect spends
//! ~1.7 s unpacking before WinMain runs, and a handle opened in that window
//! keeps its rights after the DACL changes. Handing CreateProcess the DACL up
//! front leaves no window. Our own handle from CreateProcess still gets full
//! access, which is all `play` needs.

use alloc::format;
use alloc::string::String;
use core::ffi::c_void;
use core::ptr::null_mut;

use crate::log::log;
use crate::win::{self, Owned, wide};

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER: u32 = 1;
const SDDL_REVISION_1: u32 = 1;

// The same set the client denies itself: VM_WRITE | VM_OPERATION |
// CREATE_THREAD | SUSPEND_RESUME | DUP_HANDLE | WRITE_DAC | WRITE_OWNER.
const DENIED: u32 = 0x000C_086A;

win::import!("kernel32" {
    fn GetCurrentProcess() -> win::Handle;
});

win::import!("advapi32" {
    fn OpenProcessToken(process: win::Handle, access: u32, token: *mut win::Handle) -> win::BOOL;
    fn GetTokenInformation(token: win::Handle, class: u32, info: *mut c_void, len: u32, returned: *mut u32) -> win::BOOL;
    fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> win::BOOL;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl: *const u16, revision: u32, sd: *mut *mut c_void, len: *mut u32) -> win::BOOL;
});

/// A LocalAlloc'd security descriptor.
pub struct Descriptor(*mut c_void);

impl Descriptor {
    pub fn as_ptr(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe { win::LocalFree(self.0) };
    }
}

/// None on XP, whose SDDL has no OW, or if the token can't be read. The client
/// then closes itself as it always did.
pub fn client_descriptor() -> Option<Descriptor> {
    let sid = user_sid()?;
    // OW is OWNER RIGHTS. Without it the owner keeps an implicit WRITE_DAC and
    // can put the old DACL back.
    let sddl = format!("D:(D;;0x{DENIED:x};;;OW)(D;;0x{DENIED:x};;;{sid})(A;;GA;;;{sid})(A;;GA;;;SY)");
    let mut sd = null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(wide(&sddl).as_ptr(), SDDL_REVISION_1, &mut sd, null_mut())
    };
    if ok == 0 {
        log!("can't build the client's DACL ({}), starting it without", win::last_error());
        return None;
    }
    Some(Descriptor(sd))
}

fn user_sid() -> Option<String> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let token = Owned(token);

    // TOKEN_USER and the SID it points into. u32s to keep the pointer aligned.
    let mut buf = [0u32; 32];
    let mut len = 0;
    let ok = unsafe {
        GetTokenInformation(token.0, TOKEN_USER, buf.as_mut_ptr().cast(), core::mem::size_of_val(&buf) as u32, &mut len)
    };
    if ok == 0 {
        return None;
    }
    // SID_AND_ATTRIBUTES { Sid, Attributes }: the SID pointer comes first.
    let sid = unsafe { buf.as_ptr().cast::<*mut c_void>().read() };

    let mut text = null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return None;
    }
    let out = unsafe { win::from_wide_ptr(text) };
    unsafe { win::LocalFree(text.cast()) };
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROCESS_VM_WRITE: u32 = 0x0020;
    const PROCESS_CREATE_THREAD: u32 = 0x0002;
    const PROCESS_TERMINATE: u32 = 0x0001;
    const WRITE_DAC: u32 = 0x0004_0000;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const ERROR_ACCESS_DENIED: u32 = 5;

    win::import!("kernel32" {
        fn OpenProcess(access: u32, inherit: win::BOOL, pid: u32) -> win::Handle;
        fn TerminateProcess(process: win::Handle, code: u32) -> win::BOOL;
    });

    fn open(pid: u32, access: u32) -> core::result::Result<Owned, u32> {
        let h = Owned(unsafe { OpenProcess(access, 0, pid) });
        if h.is_valid() { Ok(h) } else { Err(win::last_error()) }
    }

    #[test]
    #[ignore = "starts a process"]
    fn a_locked_down_child_refuses_injection_rights_but_not_its_parent() {
        let dacl = client_descriptor().expect("descriptor");
        let mut sa = win::SecurityAttributes {
            length: core::mem::size_of::<win::SecurityAttributes>() as u32,
            descriptor: dacl.as_ptr(),
            inherit: 0,
        };
        let mut si: win::StartupInfo = unsafe { core::mem::zeroed() };
        si.cb = core::mem::size_of::<win::StartupInfo>() as u32;
        let mut pi: win::ProcessInformation = unsafe { core::mem::zeroed() };
        let mut cmd = wide("cmd.exe /c ping -n 5 127.0.0.1");
        let ok = unsafe {
            win::CreateProcessW(core::ptr::null(), cmd.as_mut_ptr(), &mut sa, null_mut(), 0, 0x0800_0000, null_mut(), core::ptr::null(), &mut si, &mut pi)
        };
        assert_ne!(ok, 0, "CreateProcess: {}", win::last_error());
        let process = Owned(pi.process);
        drop(Owned(pi.thread));

        assert_eq!(open(pi.process_id, PROCESS_VM_WRITE).err(), Some(ERROR_ACCESS_DENIED));
        assert_eq!(open(pi.process_id, PROCESS_CREATE_THREAD).err(), Some(ERROR_ACCESS_DENIED));
        assert_eq!(open(pi.process_id, WRITE_DAC).err(), Some(ERROR_ACCESS_DENIED));
        assert!(open(pi.process_id, SYNCHRONIZE | PROCESS_TERMINATE).is_ok());

        // The handle CreateProcess gave us is not subject to the DACL.
        assert_ne!(unsafe { TerminateProcess(process.0, 0) }, 0);
    }
}
