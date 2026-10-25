//! Shell bits: special folders, shortcuts, opening a URL.

use alloc::format;
use alloc::string::String;
use core::ffi::c_void;

use crate::error::{Error, Result};
use crate::win::{self, BOOL, wide};

const CSIDL_PROGRAMS: i32 = 0x02;
const CSIDL_DESKTOPDIRECTORY: i32 = 0x10;
const CSIDL_LOCAL_APPDATA: i32 = 0x1c;
const CSIDL_FLAG_CREATE: i32 = 0x8000;

const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
const SW_SHOWNORMAL: i32 = 1;

pub enum Folder {
    LocalAppData,
    Programs,
    Desktop,
}

/// SHGetFolderPath rather than the Known Folder API, which XP doesn't have.
/// On XP LocalAppData is "Local Settings\Application Data" under the
/// profile; on anything newer it's AppData\Local.
pub fn folder(which: Folder) -> Result<String> {
    let csidl = match which {
        Folder::LocalAppData => CSIDL_LOCAL_APPDATA,
        Folder::Programs => CSIDL_PROGRAMS,
        Folder::Desktop => CSIDL_DESKTOPDIRECTORY,
    };
    let mut buf = [0u16; win::MAX_PATH];
    let hr = unsafe {
        win::SHGetFolderPathW(core::ptr::null_mut(), csidl | CSIDL_FLAG_CREATE, core::ptr::null_mut(), 0, buf.as_mut_ptr())
    };
    if hr < 0 {
        return Err(Error::os("Could not find a user folder", hr as u32));
    }
    Ok(win::from_wide(&buf))
}

pub fn open_url(url: &str) {
    unsafe {
        win::ShellExecuteW(
            core::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(url).as_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Tells Explorer and browsers that a URL scheme was (un)registered.
pub fn notify_associations_changed() {
    unsafe { win::SHChangeNotify(SHCNE_ASSOCCHANGED, 0, core::ptr::null(), core::ptr::null()) };
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const CLSID_SHELL_LINK: Guid = Guid(0x00021401, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_ISHELL_LINK_W: Guid = Guid(0x000214F9, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IPERSIST_FILE: Guid = Guid(0x0000010B, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

const CLSCTX_INPROC_SERVER: u32 = 1;
const COINIT_APARTMENTTHREADED: u32 = 2;

win::import!("ole32" {
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(clsid: *const Guid, outer: *mut c_void, ctx: u32, iid: *const Guid, out: *mut *mut c_void) -> i32;
});

/// COM for the current thread, for as long as this lives.
pub struct Com(bool);

impl Com {
    pub fn init() -> Com {
        Com(unsafe { CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED) } >= 0)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// A COM interface pointer. Methods are called by vtable slot, which is all
/// that's needed for the two interfaces used here.
struct Interface(*mut c_void);

type QueryInterfaceFn = unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32;
type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;
type SetStringFn = unsafe extern "system" fn(*mut c_void, *const u16) -> i32;
type SetIconFn = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;
type SaveFn = unsafe extern "system" fn(*mut c_void, *const u16, BOOL) -> i32;

impl Interface {
    unsafe fn slot<F: Copy>(&self, index: usize) -> F {
        unsafe {
            let vtable = *(self.0 as *const *const *const c_void);
            core::mem::transmute_copy(&*vtable.add(index))
        }
    }

    fn query(&self, iid: &Guid) -> Result<Interface> {
        let mut out = core::ptr::null_mut();
        let hr = unsafe { self.slot::<QueryInterfaceFn>(0)(self.0, iid, &mut out) };
        check(hr, "QueryInterface")?;
        Ok(Interface(out))
    }

    fn call_str(&self, index: usize, value: &str) -> Result<()> {
        let hr = unsafe { self.slot::<SetStringFn>(index)(self.0, wide(value).as_ptr()) };
        check(hr, "IShellLink")
    }
}

impl Drop for Interface {
    fn drop(&mut self) {
        unsafe { self.slot::<ReleaseFn>(2)(self.0) };
    }
}

fn check(hr: i32, what: &str) -> Result<()> {
    if hr < 0 {
        return Err(Error::os(what, hr as u32));
    }
    Ok(())
}

// IShellLinkW slots, counting IUnknown's three.
const SET_DESCRIPTION: usize = 7;
const SET_WORKING_DIRECTORY: usize = 9;
const SET_ARGUMENTS: usize = 11;
const SET_ICON_LOCATION: usize = 17;
const SET_PATH: usize = 20;
// IPersistFile::Save, after IUnknown, GetClassID, IsDirty and Load.
const SAVE: usize = 6;

/// Writes a .lnk at `lnk` pointing at `target`. Needs COM on this thread.
pub fn create_shortcut(lnk: &str, target: &str, args: &str, description: &str) -> Result<()> {
    let mut obj = core::ptr::null_mut();
    let hr = unsafe {
        CoCreateInstance(&CLSID_SHELL_LINK, core::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELL_LINK_W, &mut obj)
    };
    check(hr, "Could not create a shortcut")?;
    let link = Interface(obj);

    link.call_str(SET_PATH, target)?;
    link.call_str(SET_ARGUMENTS, args)?;
    link.call_str(SET_DESCRIPTION, description)?;
    if let Some(dir) = crate::fs::parent(target) {
        link.call_str(SET_WORKING_DIRECTORY, dir)?;
    }
    let hr = unsafe { link.slot::<SetIconFn>(SET_ICON_LOCATION)(link.0, wide(target).as_ptr(), 0) };
    check(hr, "IShellLink")?;

    let file = link.query(&IID_IPERSIST_FILE)?;
    let hr = unsafe { file.slot::<SaveFn>(SAVE)(file.0, wide(lnk).as_ptr(), 1) };
    check(hr, &format!("Could not save {lnk}"))
}
