//! HTTP over WinINet. It ships with every Windows since 95, follows the
//! user's IE proxy settings, and handles redirects, which covers everything
//! a launcher needs.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;

use crate::error::{Error, Result};
use crate::log::log;
use crate::win::{self, BOOL, Handle, wide};
use crate::{config, fs};

const INTERNET_OPEN_TYPE_PRECONFIG: u32 = 0;
const INTERNET_FLAG_RELOAD: u32 = 0x8000_0000;
const INTERNET_FLAG_NO_CACHE_WRITE: u32 = 0x0400_0000;
const INTERNET_FLAG_NO_COOKIES: u32 = 0x0008_0000;
const INTERNET_FLAG_NO_UI: u32 = 0x0000_0200;
const INTERNET_FLAG_PRAGMA_NOCACHE: u32 = 0x0000_0100;
const INTERNET_OPTION_CONNECT_TIMEOUT: u32 = 2;
const INTERNET_OPTION_RECEIVE_TIMEOUT: u32 = 6;
const HTTP_QUERY_CONTENT_LENGTH: u32 = 5;
const HTTP_QUERY_STATUS_CODE: u32 = 19;
const HTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

win::import!("wininet" {
    fn InternetOpenW(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> Handle;
    fn InternetOpenUrlW(session: Handle, url: *const u16, headers: *const u16, headers_len: u32, flags: u32, context: usize) -> Handle;
    fn InternetReadFile(h: Handle, buf: *mut u8, len: u32, read: *mut u32) -> BOOL;
    fn InternetCloseHandle(h: Handle) -> BOOL;
    fn InternetSetOptionW(h: Handle, option: u32, buf: *const c_void, len: u32) -> BOOL;
    fn HttpQueryInfoW(h: Handle, level: u32, buf: *mut c_void, len: *mut u32, index: *mut u32) -> BOOL;
});

struct Internet(Handle);

impl Drop for Internet {
    fn drop(&mut self) {
        unsafe { InternetCloseHandle(self.0) };
    }
}

pub struct Session(Internet);

pub struct Response {
    handle: Internet,
    pub length: Option<u64>,
}

impl Session {
    pub fn new() -> Result<Session> {
        let agent = wide(&format!("Better16Launcher/{}", config::VERSION));
        let h = unsafe {
            InternetOpenW(agent.as_ptr(), INTERNET_OPEN_TYPE_PRECONFIG, core::ptr::null(), core::ptr::null(), 0)
        };
        if h.is_null() {
            return Err(Error::last_os("Could not start WinINet"));
        }
        let timeout: u32 = 30_000;
        for option in [INTERNET_OPTION_CONNECT_TIMEOUT, INTERNET_OPTION_RECEIVE_TIMEOUT] {
            unsafe { InternetSetOptionW(h, option, (&raw const timeout).cast(), 4) };
        }
        Ok(Session(Internet(h)))
    }

    pub fn get(&self, url: &str) -> Result<Response> {
        // Always go to the server. A stale cached `version` would keep
        // people on an old client.
        let flags = INTERNET_FLAG_RELOAD
            | INTERNET_FLAG_NO_CACHE_WRITE
            | INTERNET_FLAG_PRAGMA_NOCACHE
            | INTERNET_FLAG_NO_COOKIES
            | INTERNET_FLAG_NO_UI;
        let h = unsafe { InternetOpenUrlW(self.0.0, wide(url).as_ptr(), core::ptr::null(), 0, flags, 0) };
        if h.is_null() {
            return Err(Error::last_os(&format!("Could not connect to {}", config::host_of(url))));
        }
        let handle = Internet(h);

        let status = query_number(h, HTTP_QUERY_STATUS_CODE).unwrap_or(0);
        match status {
            200 => {}
            404 => return Err(Error::NotFound(String::from(url))),
            _ => return Err(Error::new(format!("{url} answered HTTP {status}"))),
        }

        let length = query_number(h, HTTP_QUERY_CONTENT_LENGTH).map(u64::from);
        Ok(Response { handle, length })
    }

    pub fn get_string(&self, url: &str) -> Result<String> {
        let mut resp = self.get(url)?;
        let mut body = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = resp.read(&mut buf)?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&buf[..n]);
            if body.len() > 1 << 20 {
                return Err(Error::new(format!("{url} sent far more than expected")));
            }
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }

    /// Downloads `url` into `path`, retrying a couple of times on network
    /// errors. `progress` gets (bytes so far, total if known) and can stop
    /// the download by returning an error.
    pub fn download(
        &self,
        url: &str,
        path: &str,
        progress: &mut dyn FnMut(u64, Option<u64>) -> Result<()>,
    ) -> Result<()> {
        let mut attempt = 1;
        loop {
            match self.download_once(url, path, progress) {
                Ok(()) => return Ok(()),
                Err(e @ (Error::Cancelled | Error::NotFound(_))) => return Err(e),
                Err(e) if attempt < 3 => {
                    log!("download of {url} failed (attempt {attempt}): {e}");
                    win::sleep(2000 * attempt);
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn download_once(
        &self,
        url: &str,
        path: &str,
        progress: &mut dyn FnMut(u64, Option<u64>) -> Result<()>,
    ) -> Result<()> {
        let mut resp = self.get(url)?;
        let file = fs::File::create(path)?;
        let mut buf = vec![0u8; 64 * 1024];
        let mut done = 0u64;
        progress(0, resp.length)?;
        loop {
            let n = resp.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            done += n as u64;
            progress(done, resp.length)?;
        }
        if let Some(len) = resp.length {
            if done != len {
                return Err(Error::new(format!("{url} was cut off at {done} of {len} bytes")));
            }
        }
        Ok(())
    }
}

impl Response {
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut n = 0;
        if unsafe { InternetReadFile(self.handle.0, buf.as_mut_ptr(), buf.len() as u32, &mut n) } == 0 {
            return Err(Error::last_os("Download interrupted"));
        }
        Ok(n as usize)
    }
}

fn query_number(h: Handle, what: u32) -> Option<u32> {
    let mut value = 0u32;
    let mut len = 4u32;
    let ok = unsafe {
        HttpQueryInfoW(h, what | HTTP_QUERY_FLAG_NUMBER, (&raw mut value).cast(), &mut len, core::ptr::null_mut())
    };
    (ok != 0).then_some(value)
}
