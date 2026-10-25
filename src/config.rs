//! Where things live. The two URLs can be overridden at build time:
//!
//!     set B16_SETUP_URL=http://setup.example.org
//!     set B16_BASE_URL=https://www.example.org
//!     cargo build --release
//!
//! Everything else that names the site (the URL scheme, the event the
//! client signals) is derived from them, so a fork only has to set these.

use alloc::format;
use alloc::string::String;

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
pub const LAUNCHER_EXE: &str = "Better16Launcher.exe";
pub const CLIENT_EXE: &str = "RobloxPlayerBeta.exe";
pub const SHORTCUT_NAME: &str = "Better16 Player";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The client reads this key to find its launcher and install host, so the
/// name is fixed by the client, not by us.
pub const CLIENT_KEY: &str = "Software\\RobloxReg";
pub const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Better16Player";
/// What the old C++ launcher registered itself under. Removed on install so
/// Add/Remove Programs doesn't list the player twice.
pub const LEGACY_UNINSTALL_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{373B1718-8CC5-4567-8EE2-9033AD08A680}";

/// Client packages and the folder each one unpacks into, relative to the
/// version folder.
pub const PACKAGES: &[(&str, &str)] = &[
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
];

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

/// The URL scheme the site's Play button uses, e.g. `better16`.
pub fn scheme() -> String {
    String::from(site_name(host_of(BASE_URL)))
}

/// The named event the client sets once its window is up. Its name is
/// compiled into the client as "<base host>/robloxStartedEvent".
pub fn started_event_name() -> String {
    format!("{}/robloxStartedEvent", host_of(BASE_URL))
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
}
