//! Files and directories. Paths are plain `String`s with backslashes;
//! they only become UTF-16 at the API boundary.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::win::{self, Owned, wide};

pub struct File(Owned);

impl File {
    pub fn open(path: &str) -> Result<File> {
        let h = unsafe {
            win::CreateFileW(
                wide(path).as_ptr(),
                win::GENERIC_READ,
                win::FILE_SHARE_READ | win::FILE_SHARE_DELETE,
                core::ptr::null_mut(),
                win::OPEN_EXISTING,
                win::FILE_ATTRIBUTE_NORMAL,
                core::ptr::null_mut(),
            )
        };
        if h == win::INVALID_HANDLE_VALUE {
            return Err(Error::last_os(&format!("Could not open {path}")));
        }
        Ok(File(Owned(h)))
    }

    pub fn create(path: &str) -> Result<File> {
        // A read-only file left behind by something else would make
        // CREATE_ALWAYS fail with "access denied".
        clear_readonly(path);
        let h = unsafe {
            win::CreateFileW(
                wide(path).as_ptr(),
                win::GENERIC_WRITE,
                0,
                core::ptr::null_mut(),
                win::CREATE_ALWAYS,
                win::FILE_ATTRIBUTE_NORMAL,
                core::ptr::null_mut(),
            )
        };
        if h == win::INVALID_HANDLE_VALUE {
            return Err(Error::last_os(&format!("Could not create {path}")));
        }
        Ok(File(Owned(h)))
    }

    pub fn len(&self) -> Result<u64> {
        let mut size = 0i64;
        if unsafe { win::GetFileSizeEx(self.0.0, &mut size) } == 0 {
            return Err(Error::last_os("GetFileSizeEx"));
        }
        Ok(size as u64)
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut n = 0;
        let ok = unsafe { win::ReadFile(self.0.0, buf.as_mut_ptr(), buf.len() as u32, &mut n, core::ptr::null_mut()) };
        if ok == 0 {
            return Err(Error::last_os("ReadFile"));
        }
        Ok(n as usize)
    }

    pub fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        if unsafe { win::SetFilePointerEx(self.0.0, offset as i64, core::ptr::null_mut(), win::FILE_BEGIN) } == 0 {
            return Err(Error::last_os("SetFilePointerEx"));
        }
        let mut done = 0;
        while done < buf.len() {
            let n = self.read(&mut buf[done..])?;
            if n == 0 {
                return Err(Error::new("unexpected end of file"));
            }
            done += n;
        }
        Ok(())
    }

    pub fn write_all(&self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            // Big writes go in pieces; XP can fail a single huge WriteFile
            // on network drives with ERROR_NO_SYSTEM_RESOURCES.
            let chunk = buf.len().min(1 << 20);
            let mut n = 0;
            let ok = unsafe { win::WriteFile(self.0.0, buf.as_ptr(), chunk as u32, &mut n, core::ptr::null_mut()) };
            if ok == 0 {
                return Err(Error::last_os("WriteFile"));
            }
            buf = &buf[n as usize..];
        }
        Ok(())
    }
}

/// %TEMP%\Better16\, with the trailing backslash. Created if missing.
pub fn temp_dir() -> String {
    let dir = temp_dir_path();
    let _ = create_dir_all(&dir);
    dir
}

fn temp_dir_path() -> String {
    let mut buf = [0u16; win::MAX_PATH + 1];
    let n = unsafe { win::GetTempPathW(buf.len() as u32, buf.as_mut_ptr()) };
    let mut dir = win::from_wide(&buf[..n as usize]);
    if !dir.ends_with('\\') {
        dir.push('\\');
    }
    dir.push_str("Better16\\");
    dir
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') {
        format!("{dir}{name}")
    } else {
        format!("{dir}\\{name}")
    }
}

pub fn parent(path: &str) -> Option<&str> {
    let path = path.trim_end_matches('\\');
    path.rfind('\\').map(|i| &path[..i])
}

fn attributes(path: &str) -> u32 {
    unsafe { win::GetFileAttributesW(wide(path).as_ptr()) }
}

pub fn exists(path: &str) -> bool {
    attributes(path) != win::INVALID_FILE_ATTRIBUTES
}

pub fn is_dir(path: &str) -> bool {
    let a = attributes(path);
    a != win::INVALID_FILE_ATTRIBUTES && a & win::FILE_ATTRIBUTE_DIRECTORY != 0
}

pub fn size(path: &str) -> Option<u64> {
    File::open(path).ok()?.len().ok()
}

fn clear_readonly(path: &str) {
    let a = attributes(path);
    if a != win::INVALID_FILE_ATTRIBUTES && a & win::FILE_ATTRIBUTE_READONLY != 0 {
        unsafe { win::SetFileAttributesW(wide(path).as_ptr(), a & !win::FILE_ATTRIBUTE_READONLY) };
    }
}

pub fn create_dir_all(path: &str) -> Result<()> {
    let path = path.trim_end_matches('\\');
    if path.is_empty() || path.ends_with(':') || is_dir(path) {
        return Ok(());
    }
    if unsafe { win::CreateDirectoryW(wide(path).as_ptr(), core::ptr::null_mut()) } != 0 {
        return Ok(());
    }
    match win::last_error() {
        win::ERROR_ALREADY_EXISTS => Ok(()),
        win::ERROR_PATH_NOT_FOUND => {
            if let Some(p) = parent(path) {
                create_dir_all(p)?;
            }
            if unsafe { win::CreateDirectoryW(wide(path).as_ptr(), core::ptr::null_mut()) } == 0
                && win::last_error() != win::ERROR_ALREADY_EXISTS
            {
                return Err(Error::last_os(&format!("Could not create folder {path}")));
            }
            Ok(())
        }
        code => Err(Error::os(&format!("Could not create folder {path}"), code)),
    }
}

pub fn write(path: &str, data: &[u8]) -> Result<()> {
    File::create(path)?.write_all(data)
}

pub fn remove_file(path: &str) -> Result<()> {
    clear_readonly(path);
    if unsafe { win::DeleteFileW(wide(path).as_ptr()) } == 0 {
        let code = win::last_error();
        if code != win::ERROR_FILE_NOT_FOUND && code != win::ERROR_PATH_NOT_FOUND {
            return Err(Error::os(&format!("Could not delete {path}"), code));
        }
    }
    Ok(())
}

pub fn copy(from: &str, to: &str) -> Result<()> {
    clear_readonly(to);
    if unsafe { win::CopyFileW(wide(from).as_ptr(), wide(to).as_ptr(), 0) } == 0 {
        return Err(Error::last_os(&format!("Could not copy {from} to {to}")));
    }
    Ok(())
}

pub fn rename(from: &str, to: &str) -> Result<()> {
    let ok = unsafe { win::MoveFileExW(wide(from).as_ptr(), wide(to).as_ptr(), win::MOVEFILE_REPLACE_EXISTING) };
    if ok == 0 {
        return Err(Error::last_os(&format!("Could not move {from} to {to}")));
    }
    Ok(())
}

pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
}

pub fn read_dir(dir: &str) -> Vec<DirEntry> {
    let mut out = Vec::new();
    let mut data: win::FindData = unsafe { core::mem::zeroed() };
    let h = unsafe { win::FindFirstFileW(wide(&join(dir, "*")).as_ptr(), &mut data) };
    if h == win::INVALID_HANDLE_VALUE {
        return out;
    }
    loop {
        let name = win::from_wide(&data.file_name);
        if name != "." && name != ".." {
            out.push(DirEntry {
                name,
                is_dir: data.attributes & win::FILE_ATTRIBUTE_DIRECTORY != 0,
                is_link: data.attributes & win::FILE_ATTRIBUTE_REPARSE_POINT != 0,
            });
        }
        if unsafe { win::FindNextFileW(h, &mut data) } == 0 {
            break;
        }
    }
    unsafe { win::FindClose(h) };
    out
}

/// Deletes a folder and everything in it. Junctions are removed, never
/// followed.
pub fn remove_dir_all(dir: &str) -> Result<()> {
    if !exists(dir) {
        return Ok(());
    }
    for e in read_dir(dir) {
        let path = join(dir, &e.name);
        if e.is_dir && !e.is_link {
            remove_dir_all(&path)?;
        } else if e.is_dir {
            unsafe { win::RemoveDirectoryW(wide(&path).as_ptr()) };
        } else {
            remove_file(&path)?;
        }
    }
    if unsafe { win::RemoveDirectoryW(wide(dir).as_ptr()) } == 0 {
        return Err(Error::last_os(&format!("Could not delete folder {dir}")));
    }
    Ok(())
}

pub fn remove_dir_if_empty(dir: &str) {
    unsafe { win::RemoveDirectoryW(wide(dir).as_ptr()) };
}

pub fn free_space(dir: &str) -> Option<u64> {
    let mut avail = 0u64;
    let ok = unsafe {
        win::GetDiskFreeSpaceExW(wide(dir).as_ptr(), &mut avail, core::ptr::null_mut(), core::ptr::null_mut())
    };
    (ok != 0).then_some(avail)
}
