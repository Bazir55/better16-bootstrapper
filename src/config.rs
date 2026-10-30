//! Where things live. The two URLs can be overridden at build time:
//!
//!     set B16_SETUP_URL=http://setup.example.org
//!     set B16_BASE_URL=https://www.example.org
//!     cargo build --release
//!
//! Everything else that names the site (the URL schemes, the events the
//! clients signal) is derived from them, so a fork only has to set these.

use alloc::format;
use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

/// Serves `version`, the packages and the manifest. Plain http by default:
/// XP can't negotiate TLS 1.2, and every package is CRC-checked on unpack.
pub const SETUP_URL: &str = match option_env!("B16_SETUP_URL") {
    Some(url) => url,
    None => "http://setup.better16.wtf",
};

/// The website. The client talks to it directly, over https.
pub const BASE_URL: &str = match option_env!("B16_BASE_URL") {
    Some(url) => url,
    None => "https://www.better16.wtf",
};

pub const PRODUCT: &str = "Better16";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Product {
    pub name: &'static str,
    pub launcher_exe: &'static str,
    pub app_exe: &'static str,
    pub version_file: &'static str,
    pub update_name: &'static str,
    /// The client reads this key to find its launcher and install host, so
    /// the name is fixed by the client, not by us.
    pub reg_key: &'static str,
    pub uninstall_key: &'static str,
    pub legacy_uninstall_key: &'static str,
    pub shortcut_name: &'static str,
    pub shortcut_args: &'static str,
    pub shortcut_tip: &'static str,
    pub scheme_suffix: &'static str,
    pub started_event: &'static str,
    pub packages: &'static [(&'static str, &'static str)],
    pub space_needed: u64,
}

pub static PLAYER: Product = Product {
    name: "Better16",
    launcher_exe: "Better16Launcher.exe",
    app_exe: "RobloxPlayerBeta.exe",
    version_file: "version",
    update_name: "Better16Launcher",
    reg_key: "Software\\RobloxReg",
    uninstall_key: "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Better16Player",
    legacy_uninstall_key:
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{373B1718-8CC5-4567-8EE2-9033AD08A680}",
    shortcut_name: "Better16 Player",
    shortcut_args: "",
    shortcut_tip: "Play Better16",
    scheme_suffix: "",
    started_event: "robloxStartedEvent",
    packages: &[
        ("RobloxApp.zip", ""),
        ("redist.zip", ""),
        ("Libraries.zip", ""),
        ("content-fonts.zip", "content\\fonts"),
        ("content-music.zip", "content\\music"),
        ("content-particles.zip", "content\\particles"),
        ("content-sky.zip", "content\\sky"),
        ("content-sounds.zip", "content\\sounds"),
        ("content-textures.zip", "content\\textures"),
        ("content-textures2.zip", "content\\textures"),
        ("content-textures3.zip", "PlatformContent\\pc\\textures"),
        ("content-terrain.zip", "PlatformContent\\pc\\terrain"),
        ("shaders.zip", "shaders"),
    ],
    space_needed: 700 * 1024 * 1024,
};

pub static STUDIO: Product = Product {
    name: "Better16 Studio",
    launcher_exe: "Better16StudioLauncher.exe",
    app_exe: "RobloxStudioBeta.exe",
    version_file: "versionQTStudio",
    update_name: "Better16StudioLauncher",
    reg_key: "Software\\StudioQTRobloxReg",
    uninstall_key: "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Better16Studio",
    legacy_uninstall_key:
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{2922D6F1-2865-4EFA-97A9-94EEAB3AFA14}",
    shortcut_name: "Better16 Studio",
    shortcut_args: "-ide",
    shortcut_tip: "Build with Better16 Studio",
    scheme_suffix: "-studio",
    started_event: "robloxQTStudioStartedEvent",
    packages: &[
        ("redist.zip", ""),
        ("RobloxStudio.zip", ""),
        ("Libraries.zip", ""),
        ("content-fonts.zip", "content\\fonts"),
        ("content-music.zip", "content\\music"),
        ("content-particles.zip", "content\\particles"),
        ("content-sky.zip", "content\\sky"),
        ("content-sounds.zip", "content\\sounds"),
        ("content-textures.zip", "content\\textures"),
        ("content-textures2.zip", "content\\textures"),
        ("content-textures3.zip", "PlatformContent\\pc\\textures"),
        ("content-terrain.zip", "PlatformContent\\pc\\terrain"),
        ("content-scripts.zip", "content\\scripts"),
        ("shaders.zip", "shaders"),
        ("BuiltInPlugins.zip", "BuiltInPlugins"),
        ("imageformats.zip", "imageformats"),
    ],
    space_needed: 800 * 1024 * 1024,
};

static IS_STUDIO: AtomicBool = AtomicBool::new(false);

pub fn select_studio() {
    IS_STUDIO.store(true, Ordering::Relaxed);
}

pub fn is_studio() -> bool {
    IS_STUDIO.load(Ordering::Relaxed)
}

pub fn product() -> &'static Product {
    if is_studio() { &STUDIO } else { &PLAYER }
}

/// "https://www.better16.wtf/x" -> "www.better16.wtf"
pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', ':']).next().unwrap_or(rest)
}

/// The site's short name: the second label of a three-part host, or the
/// first label otherwise. "www.better16.wtf" -> "better16". The site builds
/// its Play links with the same rule, so neither side spells it out.
pub fn site_name(host: &str) -> &str {
    let mut labels = host.split('.');
    let first = labels.next().unwrap_or(host);
    match (labels.next(), labels.next()) {
        (Some(second), Some(_)) => second,
        _ => first,
    }
}

pub fn scheme() -> String {
    format!("{}{}", site_name(host_of(BASE_URL)), product().scheme_suffix)
}

pub fn started_event_name() -> String {
    format!("{}/{}", host_of(BASE_URL), product().started_event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_names() {
        assert_eq!(host_of("https://www.better16.wtf"), "www.better16.wtf");
        assert_eq!(host_of("http://setup.better16.wtf/version"), "setup.better16.wtf");
        assert_eq!(host_of("http://localhost:8080/"), "localhost");
        assert_eq!(site_name("www.better16.wtf"), "better16");
        assert_eq!(site_name("better16.wtf"), "better16");
        assert_eq!(site_name("localhost"), "localhost");
    }

    #[test]
    fn studio_differs_where_it_must() {
        assert_ne!(PLAYER.launcher_exe, STUDIO.launcher_exe);
        assert_ne!(PLAYER.reg_key, STUDIO.reg_key);
        assert_ne!(PLAYER.update_name, STUDIO.update_name);
        assert!(STUDIO.packages.iter().any(|(p, _)| *p == "RobloxStudio.zip"));
        assert!(!STUDIO.packages.iter().any(|(p, _)| *p == "RobloxApp.zip"));
    }
}
