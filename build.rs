// Writes the launcher's .res file (manifest, version info, bitmaps, icon) and passes
// the linker flags a CRT-less XP executable needs.
//
// The .res is assembled by hand so the build doesn't depend on rc.exe or
// windres: link.exe and lld-link both take a .res file as a plain input.

use std::env;
use std::fs;
use std::path::PathBuf;

const RT_BITMAP: u16 = 2;
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const RT_MANIFEST: u16 = 24;
const LANG_EN_US: u16 = 0x0409;

// The window's pictures, by resource id. src/ui.rs uses the same numbers.
const BITMAPS: &[(u16, &str)] = &[
    (101, "logo.bmp"),
    (102, "logo@2x.bmp"),
    (103, "cancel.bmp"),
    (104, "cancel@2x.bmp"),
    (105, "cancel-hover.bmp"),
    (106, "cancel-hover@2x.bmp"),
    (107, "ok.bmp"),
    (108, "ok@2x.bmp"),
    (109, "ok-hover.bmp"),
    (110, "ok-hover@2x.bmp"),
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-env-changed=B16_SETUP_URL");
    println!("cargo:rerun-if-env-changed=B16_BASE_URL");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        panic!("the launcher only builds for Windows targets");
    }

    // The runtime shims in src/rt.rs are 32-bit x86, and so is XP.
    if env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86") {
        panic!("build for i686-pc-windows-msvc (the default in .cargo/config.toml)");
    }

    for arg in [
        "/ENTRY:start",
        // 5.01 in the PE header, or XP's loader refuses the file.
        "/SUBSYSTEM:WINDOWS,5.01",
        "/NODEFAULTLIB",
        "/SAFESEH:NO",
        "/MANIFEST:NO",
    ] {
        println!("cargo:rustc-link-arg-bins={arg}");
    }

    // rustc always asks for a PDB on MSVC. The shipped exe doesn't need one,
    // nor a pointer to it in its headers.
    if env::var("PROFILE").as_deref() == Ok("release") {
        println!("cargo:rustc-link-arg-bins=/DEBUG:NONE");
    }

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("launcher.res");
    fs::write(&out, resources()).unwrap();
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

fn resources() -> Vec<u8> {
    let mut res = Vec::new();

    // Every .res file starts with an empty entry; cvtres and lld check for it.
    entry(&mut res, 0, 0, &[], 0);

    entry(&mut res, RT_MANIFEST, 1, MANIFEST.as_bytes(), LANG_EN_US);
    entry(&mut res, RT_VERSION, 1, &version_info(), LANG_EN_US);

    for &(id, file) in BITMAPS {
        let bmp = fs::read(format!("assets/{file}")).unwrap_or_else(|e| panic!("assets/{file}: {e}"));
        assert!(bmp.starts_with(b"BM"), "assets/{file} is not a .bmp file");
        // A bitmap resource is the file without its 14-byte file header.
        entry(&mut res, RT_BITMAP, id, &bmp[14..], LANG_EN_US);
    }

    let icon = PathBuf::from("assets/better16.ico");
    if let Ok(ico) = fs::read(&icon) {
        add_icon(&mut res, &ico);
    }

    res
}

fn entry(res: &mut Vec<u8>, kind: u16, name: u16, data: &[u8], lang: u16) {
    let mut header = Vec::new();
    header.extend((data.len() as u32).to_le_bytes());
    header.extend(32u32.to_le_bytes()); // header size, fixed for numeric type and name
    header.extend([0xFF, 0xFF]);
    header.extend(kind.to_le_bytes());
    header.extend([0xFF, 0xFF]);
    header.extend(name.to_le_bytes());
    header.extend(0u32.to_le_bytes()); // data version
    let flags: u16 = if kind == 0 { 0 } else { 0x1030 }; // moveable | pure | discardable
    header.extend(flags.to_le_bytes());
    header.extend(lang.to_le_bytes());
    header.extend(0u32.to_le_bytes()); // version
    header.extend(0u32.to_le_bytes()); // characteristics
    assert_eq!(header.len(), 32);

    res.extend(header);
    res.extend(data);
    align4(res);
}

fn add_icon(res: &mut Vec<u8>, ico: &[u8]) {
    let u16_at = |i: usize| u16::from_le_bytes([ico[i], ico[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([ico[i], ico[i + 1], ico[i + 2], ico[i + 3]]);

    assert!(ico.len() >= 6 && u16_at(2) == 1, "assets/better16.ico is not an .ico file");
    let count = u16_at(4) as usize;

    let mut group = Vec::new();
    group.extend(0u16.to_le_bytes());
    group.extend(1u16.to_le_bytes());
    group.extend((count as u16).to_le_bytes());

    for i in 0..count {
        let e = 6 + i * 16;
        let size = u32_at(e + 8) as usize;
        let offset = u32_at(e + 12) as usize;
        let id = (i + 1) as u16;

        entry(res, RT_ICON, id, &ico[offset..offset + size], LANG_EN_US);

        // Same 12 leading bytes as the file's directory entry, then the
        // resource id where the file had an offset.
        group.extend(&ico[e..e + 12]);
        group.extend(id.to_le_bytes());
    }

    entry(res, RT_GROUP_ICON, 1, &group, LANG_EN_US);
}

fn version_info() -> Vec<u8> {
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts = version.split('.').map(|p| p.parse::<u16>().unwrap_or(0));
    let (a, b, c) = (
        parts.next().unwrap_or(0) as u32,
        parts.next().unwrap_or(0) as u32,
        parts.next().unwrap_or(0) as u32,
    );

    let mut fixed = Vec::new();
    for v in [
        0xFEEF04BDu32,      // signature
        0x0001_0000,        // struct version
        (a << 16) | b,      // file version
        c << 16,
        (a << 16) | b,      // product version
        c << 16,
        0x3F,               // flags mask
        0,                  // flags
        0x0004_0004,        // VOS_NT_WINDOWS32
        1,                  // VFT_APP
        0,
        0,
        0,
    ] {
        fixed.extend(v.to_le_bytes());
    }

    let strings = [
        ("CompanyName", "Better16"),
        ("FileDescription", "Better16 Launcher"),
        ("FileVersion", version.as_str()),
        ("InternalName", "Better16Launcher"),
        ("OriginalFilename", "Better16Launcher.exe"),
        ("ProductName", "Better16"),
        ("ProductVersion", version.as_str()),
    ];
    let strings: Vec<Vec<u8>> = strings.iter().map(|(k, v)| text_node(k, v)).collect();

    let table = node("040904b0", &[], 0, 1, &strings);
    let string_info = node("StringFileInfo", &[], 0, 1, &[table]);

    let translation = node("Translation", &0x04B0_0409u32.to_le_bytes(), 4, 0, &[]);
    let var_info = node("VarFileInfo", &[], 0, 1, &[translation]);

    node("VS_VERSION_INFO", &fixed, fixed.len() as u16, 0, &[string_info, var_info])
}

fn text_node(key: &str, value: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut words = 0u16;
    for unit in value.encode_utf16().chain([0]) {
        bytes.extend(unit.to_le_bytes());
        words += 1;
    }
    node(key, &bytes, words, 1, &[])
}

// One block of the VS_VERSIONINFO tree: length, value length, type, key,
// value, children, each piece padded to 4 bytes.
fn node(key: &str, value: &[u8], value_len: u16, kind: u16, children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0, 0];
    b.extend(value_len.to_le_bytes());
    b.extend(kind.to_le_bytes());
    for unit in key.encode_utf16().chain([0]) {
        b.extend(unit.to_le_bytes());
    }
    align4(&mut b);
    b.extend(value);
    for child in children {
        align4(&mut b);
        b.extend(child);
    }
    let len = b.len() as u16;
    b[..2].copy_from_slice(&len.to_le_bytes());
    b
}

fn align4(b: &mut Vec<u8>) {
    while b.len() % 4 != 0 {
        b.push(0);
    }
}

// trustInfo sits in the asm.v2 namespace on purpose: XP SP2 trips over the
// asm.v3 spelling, while Vista and later read either one.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Better16.Launcher" version="1.0.0.0" processorArchitecture="*"/>
  <description>Better16 Launcher</description>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v2">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{e2011457-1546-43c5-a5fe-008deee3d3f0}"/>
      <supportedOS Id="{35138b9a-5d96-4fbd-8e2d-a2440225f93a}"/>
      <supportedOS Id="{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true</dpiAware>
    </windowsSettings>
  </application>
</assembly>
"#;
