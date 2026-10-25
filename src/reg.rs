//! Registry access, HKEY_CURRENT_USER only. The launcher installs per user
//! and never needs admin rights.

use alloc::format;
use alloc::string::String;
use alloc::vec;

use crate::error::{Error, Result};
use crate::win::{self, HKEY, wide};

// Predefined keys are sign-extended 32-bit values.
const HKEY_CURRENT_USER: HKEY = 0x8000_0001u32 as i32 as isize as HKEY;
const KEY_READ: u32 = 0x20019;
const KEY_WRITE: u32 = 0x20006;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const REG_DWORD: u32 = 4;

win::import!("advapi32" {
    fn RegCreateKeyExW(key: HKEY, sub: *const u16, reserved: u32, class: *const u16, options: u32, sam: u32, sa: *mut win::SecurityAttributes, result: *mut HKEY, disposition: *mut u32) -> i32;
    fn RegOpenKeyExW(key: HKEY, sub: *const u16, options: u32, sam: u32, result: *mut HKEY) -> i32;
    fn RegSetValueExW(key: HKEY, name: *const u16, reserved: u32, kind: u32, data: *const u8, len: u32) -> i32;
    fn RegQueryValueExW(key: HKEY, name: *const u16, reserved: *mut u32, kind: *mut u32, data: *mut u8, len: *mut u32) -> i32;
    fn RegCloseKey(key: HKEY) -> i32;
});

// RegDeleteTreeW would do, but it's Vista and later.
win::import!("shlwapi" {
    fn SHDeleteKeyW(key: HKEY, sub: *const u16) -> i32;
});

pub struct Key(HKEY);

impl Key {
    pub fn create(path: &str) -> Result<Key> {
        let mut key = core::ptr::null_mut();
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(path).as_ptr(),
                0,
                core::ptr::null(),
                0,
                KEY_READ | KEY_WRITE,
                core::ptr::null_mut(),
                &mut key,
                core::ptr::null_mut(),
            )
        };
        if rc != 0 {
            return Err(Error::os(&format!("Could not write HKCU\\{path}"), rc as u32));
        }
        Ok(Key(key))
    }

    pub fn open(path: &str) -> Option<Key> {
        let mut key = core::ptr::null_mut();
        let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(path).as_ptr(), 0, KEY_READ, &mut key) };
        (rc == 0).then(|| Key(key))
    }

    pub fn create_with_default(path: &str, value: &str) -> Result<Key> {
        let key = Key::create(path)?;
        key.set_str("", value)?;
        Ok(key)
    }

    pub fn set_str(&self, name: &str, value: &str) -> Result<()> {
        self.set_string(name, value, REG_SZ)
    }

    pub fn set_expand_str(&self, name: &str, value: &str) -> Result<()> {
        self.set_string(name, value, REG_EXPAND_SZ)
    }

    fn set_string(&self, name: &str, value: &str, kind: u32) -> Result<()> {
        let data = wide(value);
        let rc = unsafe {
            RegSetValueExW(self.0, wide(name).as_ptr(), 0, kind, data.as_ptr().cast(), (data.len() * 2) as u32)
        };
        if rc != 0 {
            return Err(Error::os(&format!("Could not set registry value {name}"), rc as u32));
        }
        Ok(())
    }

    pub fn set_dword(&self, name: &str, value: u32) -> Result<()> {
        let rc = unsafe { RegSetValueExW(self.0, wide(name).as_ptr(), 0, REG_DWORD, (&raw const value).cast(), 4) };
        if rc != 0 {
            return Err(Error::os(&format!("Could not set registry value {name}"), rc as u32));
        }
        Ok(())
    }

    pub fn get_str(&self, name: &str) -> Option<String> {
        let name = wide(name);
        let mut kind = 0;
        let mut len = 0u32;
        unsafe {
            if RegQueryValueExW(self.0, name.as_ptr(), core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), &mut len) != 0 {
                return None;
            }
            if kind != REG_SZ && kind != REG_EXPAND_SZ {
                return None;
            }
            // Room for a terminator the stored value might be missing.
            let mut buf = vec![0u16; len as usize / 2 + 1];
            if RegQueryValueExW(self.0, name.as_ptr(), core::ptr::null_mut(), &mut kind, buf.as_mut_ptr().cast(), &mut len) != 0 {
                return None;
            }
            Some(win::from_wide(&buf))
        }
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

/// Deletes a key with everything under it. Missing keys are fine.
pub fn delete_tree(path: &str) {
    unsafe { SHDeleteKeyW(HKEY_CURRENT_USER, wide(path).as_ptr()) };
}

pub fn get_str(path: &str, name: &str) -> Option<String> {
    Key::open(path)?.get_str(name)
}
