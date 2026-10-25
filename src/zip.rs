//! Just enough of the zip format to unpack the client's packages: stored
//! and deflated entries, no zip64, no encryption. The packages are built by
//! our own deploy script, so anything outside that is an error rather than
//! something to support.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::error::{Error, Result, bail};

pub trait Source {
    fn len(&self) -> Result<u64>;
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()>;
}

impl Source for crate::fs::File {
    fn len(&self) -> Result<u64> {
        crate::fs::File::len(self)
    }
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        crate::fs::File::read_exact_at(self, offset, buf)
    }
}

impl Source for [u8] {
    fn len(&self) -> Result<u64> {
        Ok(<[u8]>::len(self) as u64)
    }
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let start = offset as usize;
        match self.get(start..start + buf.len()) {
            Some(src) => {
                buf.copy_from_slice(src);
                Ok(())
            }
            None => bail!("read past the end of the archive"),
        }
    }
}

#[derive(Debug)]
pub struct Entry {
    /// Path inside the archive, with backslashes.
    pub name: String,
    pub is_dir: bool,
    method: u16,
    crc: u32,
    compressed_size: u32,
    size: u32,
    header_offset: u32,
}

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn entries<S: Source + ?Sized>(src: &S) -> Result<Vec<Entry>> {
    let len = src.len()?;
    if len < 22 {
        bail!("not a zip file (too short)");
    }

    // The end record is the last 22 bytes plus a comment of up to 64 KB,
    // so search backwards through that much for its signature.
    let tail_len = len.min(22 + 0xFFFF) as usize;
    let mut tail = vec![0u8; tail_len];
    src.read_exact_at(len - tail_len as u64, &mut tail)?;

    let eocd = (0..=tail_len - 22)
        .rev()
        .find(|&i| u32_at(&tail, i) == EOCD_SIG)
        .ok_or_else(|| Error::new("not a zip file (no end of central directory)"))?;

    let count = u16_at(&tail, eocd + 10) as usize;
    let cd_size = u32_at(&tail, eocd + 12) as usize;
    let cd_offset = u32_at(&tail, eocd + 16) as u64;
    if count == 0xFFFF || cd_offset == 0xFFFF_FFFF {
        bail!("zip64 archives are not supported");
    }

    let mut cd = vec![0u8; cd_size];
    src.read_exact_at(cd_offset, &mut cd)?;

    let mut out = Vec::with_capacity(count);
    let mut p = 0;
    for _ in 0..count {
        if p + 46 > cd.len() || u32_at(&cd, p) != CENTRAL_SIG {
            bail!("corrupt zip central directory");
        }
        let flags = u16_at(&cd, p + 8);
        let name_len = u16_at(&cd, p + 28) as usize;
        let extra_len = u16_at(&cd, p + 30) as usize;
        let comment_len = u16_at(&cd, p + 32) as usize;

        let raw_name = cd
            .get(p + 46..p + 46 + name_len)
            .ok_or_else(|| Error::new("corrupt zip central directory"))?;
        let name = String::from_utf8_lossy(raw_name).replace('/', "\\");

        if flags & 1 != 0 {
            bail!("{name} is encrypted");
        }

        out.push(Entry {
            is_dir: name.ends_with('\\'),
            name,
            method: u16_at(&cd, p + 10),
            crc: u32_at(&cd, p + 16),
            compressed_size: u32_at(&cd, p + 20),
            size: u32_at(&cd, p + 24),
            header_offset: u32_at(&cd, p + 42),
        });

        p += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

/// Unpacks one entry into memory and checks its CRC.
pub fn read<S: Source + ?Sized>(src: &S, e: &Entry) -> Result<Vec<u8>> {
    let mut local = [0u8; 30];
    src.read_exact_at(e.header_offset as u64, &mut local)?;
    if u32_at(&local, 0) != LOCAL_SIG {
        bail!("corrupt zip entry {}", e.name);
    }
    // The local header can carry a different extra field than the central
    // one, so its own lengths decide where the data starts.
    let skip = 30 + u16_at(&local, 26) as u64 + u16_at(&local, 28) as u64;

    let mut packed = vec![0u8; e.compressed_size as usize];
    src.read_exact_at(e.header_offset as u64 + skip, &mut packed)?;

    let data = match e.method {
        0 => packed,
        8 => miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, e.size as usize)
            .map_err(|err| Error::new(format!("{} is damaged ({:?})", e.name, err.status)))?,
        m => bail!("{} uses compression method {m}, which isn't supported", e.name),
    };

    if data.len() != e.size as usize || crc32(&data) != e.crc {
        bail!("{} failed its CRC check", e.name);
    }
    Ok(data)
}

/// Whether an entry name is safe to join onto the install folder: relative,
/// no drive letter, no "..".
pub fn is_safe_path(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('\\')
        && !name.contains(':')
        && name.split('\\').all(|part| part != "..")
}

const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;

    // A two-file archive made with Python's zipfile:
    //   a.txt  "hello hello hello hello" (deflated)
    //   d/b.txt "hi" (stored)
    const SAMPLE: &[u8] = &[
        0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x21, 0x00, 0xe3, 0x51,
        0x3d, 0x8d, 0x0a, 0x00, 0x00, 0x00, 0x17, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x61, 0x2e,
        0x74, 0x78, 0x74, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01, 0x50, 0x4b, 0x03,
        0x04, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0xac, 0x2a, 0x93, 0xd8, 0x02,
        0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x64, 0x2f, 0x62, 0x2e, 0x74,
        0x78, 0x74, 0x68, 0x69, 0x50, 0x4b, 0x01, 0x02, 0x14, 0x00, 0x14, 0x00, 0x00, 0x00, 0x08, 0x00,
        0x00, 0x00, 0x21, 0x00, 0xe3, 0x51, 0x3d, 0x8d, 0x0a, 0x00, 0x00, 0x00, 0x17, 0x00, 0x00, 0x00,
        0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa4, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x61, 0x2e, 0x74, 0x78, 0x74, 0x50, 0x4b, 0x01, 0x02, 0x14, 0x00, 0x14, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0xac, 0x2a, 0x93, 0xd8, 0x02, 0x00, 0x00, 0x00, 0x02,
        0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa4,
        0x01, 0x2d, 0x00, 0x00, 0x00, 0x64, 0x2f, 0x62, 0x2e, 0x74, 0x78, 0x74, 0x50, 0x4b, 0x05, 0x06,
        0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00, 0x68, 0x00, 0x00, 0x00, 0x54, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];

    #[test]
    fn crc_of_check_string() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn reads_sample_archive() {
        let list = entries(SAMPLE).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "a.txt");
        assert_eq!(list[1].name, "d\\b.txt");
        assert_eq!(read(SAMPLE, &list[0]).unwrap(), b"hello hello hello hello");
        assert_eq!(read(SAMPLE, &list[1]).unwrap(), b"hi");
    }

    #[test]
    fn rejects_escaping_paths() {
        assert!(is_safe_path("content\\fonts\\a.ttf"));
        assert!(!is_safe_path("..\\evil.exe"));
        assert!(!is_safe_path("a\\..\\..\\evil.exe"));
        assert!(!is_safe_path("C:\\Windows\\evil.exe"));
        assert!(!is_safe_path("\\evil.exe"));
    }
}
