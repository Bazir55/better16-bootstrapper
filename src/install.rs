//! Getting the current build onto disk and registered.
//!
//! Layout, per user, no admin rights needed:
//!
//!     %LOCALAPPDATA%\Better16\
//!         Better16Launcher.exe        the copy the player's URL scheme points at
//!         Better16StudioLauncher.exe  and Studio's
//!         Versions\version-xxxxxxxx\  one folder per build, player or Studio
//!
//! A build is identified by the string the setup server returns from
//! /version (the player) or /versionQTStudio (Studio). Its files are
//! /<version>-<package>.zip, plus /<version>-rbxManifest.txt listing every
//! file with its MD5.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::config::{self, SETUP_URL, product};
use crate::error::{Error, Result, bail};
use crate::fs;
use crate::http::Session;
use crate::log::log;
use crate::md5::{self, Md5};
use crate::reg::{self, Key};
use crate::shell::{self, Folder};
use crate::ui;
use crate::win;
use crate::zip;

const PLACE_EXTENSION: &str = "Software\\Classes\\.rbxl";
const PLACE_CLASS: &str = "Software\\Classes\\Roblox.Place";

pub fn root() -> Result<String> {
    Ok(fs::join(&shell::folder(Folder::LocalAppData)?, config::PRODUCT))
}

pub fn launcher_path(root: &str) -> String {
    fs::join(root, product().launcher_exe)
}

fn versions_dir(root: &str) -> String {
    fs::join(root, "Versions")
}

pub fn version_dir(root: &str, version: &str) -> String {
    fs::join(&versions_dir(root), version)
}

fn downloads_dir() -> String {
    if config::is_studio() {
        format!("{}studio-packages", fs::temp_dir())
    } else {
        format!("{}packages", fs::temp_dir())
    }
}

/// The version the registry says is installed, if its files are there.
pub fn installed_version(root: &str) -> Option<String> {
    let version = reg::get_str(product().reg_key, "version")?;
    let exe = fs::join(&version_dir(root, &version), product().app_exe);
    fs::exists(&exe).then_some(version)
}

pub fn latest_version(http: &Session) -> Result<String> {
    let version = http.get_string(&format!("{SETUP_URL}/{}", product().version_file))?;
    let version = version.trim();
    // It becomes a folder name and part of every URL, so be picky.
    let valid = version.starts_with("version-")
        && version.len() <= 64
        && version.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !valid {
        bail!("The setup server sent an odd version string: {version:?}");
    }
    Ok(String::from(version))
}

/// Downloads a newer launcher if the server has one. Returns the path of the
/// downloaded exe; the caller runs it and quits.
///
/// The server opts in by publishing <version>-Better16LauncherVersion.txt
/// (Better16StudioLauncherVersion.txt for Studio). Without it nothing
/// happens, which is also what keeps this launcher from "updating" itself to
/// the old C++ one that the same server hands out.
pub fn find_update(http: &Session, version: &str) -> Result<Option<String>> {
    let name = product().update_name;
    let remote = match http.get_string(&format!("{SETUP_URL}/{version}-{name}Version.txt")) {
        Ok(v) => String::from(v.trim()),
        Err(Error::NotFound(_)) => return Ok(None),
        Err(e) => {
            log!("could not check for a launcher update: {e}");
            return Ok(None);
        }
    };
    if remote == config::VERSION || remote.is_empty() || !remote.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Ok(None);
    }

    ui::status(&format!("Getting the latest {}...", product().name));
    log!("launcher {} -> {remote}", config::VERSION);
    let path = format!("{}{name}-{remote}.exe", fs::temp_dir());
    http.download(&format!("{SETUP_URL}/{version}-{name}.exe"), &path, &mut |_, _| ui::check_cancel())?;

    let mut head = [0u8; 2];
    fs::File::open(&path)?.read_exact_at(0, &mut head)?;
    if &head != b"MZ" {
        bail!("The downloaded launcher update is not a program");
    }
    Ok(Some(path))
}

/// Makes sure `version` is installed and registered. With `verify`, every
/// file is also checked against the manifest and the whole version is
/// reinstalled if anything is off. Returns the version folder.
pub fn ensure(http: &Session, version: &str, verify: bool) -> Result<String> {
    let root = root()?;
    let dir = version_dir(&root, version);
    let previous = installed_version(&root);

    let mut needs_install = previous.as_deref() != Some(version);
    if !needs_install && verify && !files_match_manifest(http, &dir, version)? {
        log!("installed files don't match the manifest, reinstalling");
        needs_install = true;
    }

    if needs_install {
        install_packages(http, &dir, version, previous.is_some())?;
    }

    ui::disable_cancel();
    ui::busy();
    ui::status(&format!("Configuring {}...", product().name));
    if config::is_studio() {
        write_studio_settings(&dir)?;
    }
    install_launcher(&root)?;
    register(&root, &dir, version, previous.is_none())?;

    if needs_install {
        remove_old_versions(&root, version);
    }
    Ok(dir)
}

fn install_packages(http: &Session, dir: &str, version: &str, updating: bool) -> Result<()> {
    let p = product();
    fs::create_dir_all(dir)?;
    if let Some(free) = fs::free_space(dir) {
        if free < p.space_needed {
            bail!(
                "Not enough disk space. {} needs about {} MB free, and there are {} MB.",
                p.name,
                p.space_needed >> 20,
                free >> 20
            );
        }
    }

    // Zips wait in %TEMP% between download and unpack. The install folder
    // already has a Downloads folder belonging to the old launcher.
    let downloads = downloads_dir();
    fs::create_dir_all(&downloads)?;

    // One line for the whole thing, the way the launcher always put it; the
    // bar shows how far along it is.
    ui::status(&format!("{} {} ...", if updating { "Upgrading" } else { "Installing" }, p.name));

    let count = p.packages.len() as u64;
    for (i, &(package, subdir)) in p.packages.iter().enumerate() {
        let i = i as u64;
        let zip_path = fs::join(&downloads, package);

        log!("downloading {package}");
        ui::progress(i * 100, count * 100);
        let url = format!("{SETUP_URL}/{version}-{package}");
        http.download(&url, &zip_path, &mut |done, total| {
            if let Some(total) = total.filter(|&t| t > 0) {
                // Downloading is the first 80% of each package's share.
                ui::progress(i * 100 + done * 80 / total, count * 100);
            }
            ui::check_cancel()
        })?;

        log!("unpacking {package}");
        let target = if subdir.is_empty() { String::from(dir) } else { fs::join(dir, subdir) };
        unpack(&zip_path, &target, &mut |done, total| {
            ui::progress(i * 100 + 80 + done * 20 / total.max(1), count * 100);
            ui::check_cancel()
        })?;
        let _ = fs::remove_file(&zip_path);
    }
    ui::progress(1, 1);

    // The client won't start without AppSettings.xml. RobloxApp.zip brings
    // its own, which is the one the manifest expects, so only fill the gap.
    let settings = fs::join(dir, "AppSettings.xml");
    if !config::is_studio() && !fs::exists(&settings) {
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<Settings>\r\n\t<ContentFolder>content</ContentFolder>\r\n\t<BaseUrl>{}/</BaseUrl>\r\n</Settings>\r\n",
            config::BASE_URL
        );
        fs::write(&settings, xml.as_bytes())?;
    }

    let _ = fs::remove_dir_all(&downloads);
    Ok(())
}

/// The package's AppSettings.xml points at the build tree, so it is always written over.
fn write_studio_settings(dir: &str) -> Result<()> {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<Settings>\r\n\t<ContentFolder>content</ContentFolder>\r\n\t<BaseUrl>http://{}</BaseUrl>\r\n</Settings>\r\n",
        config::host_of(config::BASE_URL)
    );
    fs::write(&fs::join(dir, "AppSettings.xml"), xml.as_bytes())
}

fn unpack(zip_path: &str, target: &str, progress: &mut dyn FnMut(u64, u64) -> Result<()>) -> Result<()> {
    let file = fs::File::open(zip_path)?;
    let entries = zip::entries(&file).map_err(|e| Error::new(format!("{zip_path}: {e}")))?;
    fs::create_dir_all(target)?;

    let total = entries.len() as u64;
    for (n, e) in entries.iter().enumerate() {
        if !zip::is_safe_path(&e.name) {
            bail!("{zip_path} contains a file outside its folder: {}", e.name);
        }
        let path = fs::join(target, e.name.trim_end_matches('\\'));
        if e.is_dir {
            fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = fs::parent(&path) {
            fs::create_dir_all(parent)?;
        }
        let data = zip::read(&file, e)?;
        fs::write(&path, &data)?;
        progress(n as u64 + 1, total)?;
    }
    Ok(())
}

fn files_match_manifest(http: &Session, dir: &str, version: &str) -> Result<bool> {
    ui::status("Performing file check...");
    let manifest = match http.get_string(&format!("{SETUP_URL}/{version}-rbxManifest.txt")) {
        Ok(m) => m,
        Err(e @ Error::Cancelled) => return Err(e),
        Err(e) => {
            // Can't check, so don't punish anyone for it.
            log!("no manifest, skipping the file check: {e}");
            return Ok(true);
        }
    };

    match first_mismatch(dir, &manifest, "")? {
        Some(name) => {
            log!("missing or changed: {name}");
            Ok(false)
        }
        None => Ok(true),
    }
}

/// Goes through the manifest (pairs of lines: path, then its MD5) and
/// returns the first file under `prefix` that is missing or different.
fn first_mismatch(dir: &str, manifest: &str, prefix: &str) -> Result<Option<String>> {
    let lines: Vec<&str> = manifest.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let total = (lines.len() / 2) as u64;
    let mut buf = alloc::vec![0u8; 256 * 1024];

    for (n, pair) in lines.chunks_exact(2).enumerate() {
        ui::check_cancel()?;
        ui::progress(n as u64, total);
        let (name, expected) = (pair[0], pair[1]);
        if !zip::is_safe_path(name) || !name.starts_with(prefix) {
            continue;
        }
        if config::is_studio() && name.eq_ignore_ascii_case("AppSettings.xml") {
            continue;
        }
        let Ok(file) = fs::File::open(&fs::join(dir, name)) else {
            return Ok(Some(String::from(name)));
        };
        let mut h = Md5::new();
        loop {
            let read = file.read(&mut buf)?;
            if read == 0 {
                break;
            }
            h.update(&buf[..read]);
        }
        if !md5::hex(&h.finish()).eq_ignore_ascii_case(expected) {
            return Ok(Some(String::from(name)));
        }
    }
    ui::progress(1, 1);
    Ok(None)
}

/// Copies the running launcher to its fixed place, unless it's already the
/// one running from there.
fn install_launcher(root: &str) -> Result<()> {
    let me = win::current_exe();
    let target = launcher_path(root);
    if me.eq_ignore_ascii_case(&target) {
        return Ok(());
    }
    fs::create_dir_all(root)?;
    // The old copy may be running (another window waiting on the lock, for
    // one). A running exe can't be overwritten but it can be renamed.
    let old = format!("{target}.old");
    if fs::exists(&target) {
        let _ = fs::remove_file(&old);
        fs::rename(&target, &old)?;
    }
    fs::copy(&me, &target)
}

/// Leftover from install_launcher; gone once nothing runs it any more.
pub fn remove_stale_launcher() {
    if let Ok(root) = root() {
        let _ = fs::remove_file(&format!("{}.old", launcher_path(&root)));
    }
}

fn register(root: &str, dir: &str, version: &str, first_install: bool) -> Result<()> {
    let p = product();
    let launcher = launcher_path(root);
    let scheme = config::scheme();

    let client = Key::create(p.reg_key)?;

    // A scheme some earlier install registered under another name.
    if let Some(old) = client.get_str("protocol handler scheme") {
        if !old.is_empty() && !old.eq_ignore_ascii_case(&scheme) {
            log!("removing old URL scheme {old}");
            reg::delete_tree(&format!("Software\\Classes\\{old}"));
        }
    }

    let class = Key::create_with_default(&format!("Software\\Classes\\{scheme}"), &format!("URL: {} Protocol", p.name))?;
    class.set_str("URL Protocol", "")?;
    Key::create_with_default(&format!("Software\\Classes\\{scheme}\\DefaultIcon"), &format!("\"{launcher}\",0"))?;
    Key::create_with_default(
        &format!("Software\\Classes\\{scheme}\\shell\\open\\command"),
        &format!("\"{launcher}\" \"%1\""),
    )?;
    if config::is_studio() {
        register_place_files(&launcher, dir)?;
    }
    shell::notify_associations_changed();

    client.set_str("", &launcher)?;
    client.set_str("install host", config::host_of(SETUP_URL))?;
    client.set_str("protocol handler scheme", &scheme)?;

    reg::delete_tree(p.legacy_uninstall_key);
    let uninstall = Key::create(p.uninstall_key)?;
    uninstall.set_str("DisplayName", p.shortcut_name)?;
    uninstall.set_str("DisplayIcon", &format!("{launcher},0"))?;
    uninstall.set_str("DisplayVersion", config::VERSION)?;
    uninstall.set_str("Comments", version)?;
    uninstall.set_str("Publisher", config::PRODUCT)?;
    uninstall.set_str("URLInfoAbout", config::BASE_URL)?;
    uninstall.set_str("InstallLocation", root)?;
    uninstall.set_expand_str("UninstallString", &format!("\"{launcher}\" -uninstall"))?;
    uninstall.set_dword("NoModify", 1)?;
    uninstall.set_dword("NoRepair", 1)?;
    if first_install {
        let mut t: win::SystemTime = unsafe { core::mem::zeroed() };
        unsafe { win::GetLocalTime(&mut t) };
        uninstall.set_str("InstallDate", &format!("{:04}{:02}{:02}", t.year, t.month, t.day))?;
    }

    // Shortcuts are nice to have; a locked-down Start menu shouldn't fail
    // the install.
    if let Err(e) = create_shortcuts(&launcher, first_install) {
        log!("shortcuts: {e}");
    }

    // Last, so an install that died halfway is redone next time.
    client.set_str("version", version)?;
    log!("registered {version}, launcher at {launcher}");
    Ok(())
}

fn register_place_files(launcher: &str, dir: &str) -> Result<()> {
    let ext = Key::create_with_default(PLACE_EXTENSION, "Roblox.Place")?;
    drop(ext);
    Key::create(&format!("{PLACE_EXTENSION}\\Roblox.Place\\ShellNew"))?;
    Key::create_with_default(PLACE_CLASS, "Roblox Place")?;
    Key::create_with_default(
        &format!("{PLACE_CLASS}\\DefaultIcon"),
        &format!("{},0", fs::join(dir, product().app_exe)),
    )?;
    Key::create_with_default(&format!("{PLACE_CLASS}\\shell\\Open"), "Open")?;
    Key::create_with_default(
        &format!("{PLACE_CLASS}\\shell\\Open\\command"),
        &format!("\"{launcher}\" -ide \"%1\""),
    )?;
    Ok(())
}

fn unregister_place_files(launcher: &str) {
    let ours = reg::get_str(&format!("{PLACE_CLASS}\\shell\\Open\\command"), "")
        .is_some_and(|cmd| cmd.to_ascii_lowercase().contains(&launcher.to_ascii_lowercase()));
    if ours {
        reg::delete_tree(PLACE_EXTENSION);
        reg::delete_tree(PLACE_CLASS);
    }
}

fn create_shortcuts(launcher: &str, desktop: bool) -> Result<()> {
    let p = product();
    let lnk = format!("{}.lnk", p.shortcut_name);
    let programs = fs::join(&shell::folder(Folder::Programs)?, config::PRODUCT);
    fs::create_dir_all(&programs)?;
    shell::create_shortcut(&fs::join(&programs, &lnk), launcher, p.shortcut_args, p.shortcut_tip)?;

    let on_desktop = fs::join(&shell::folder(Folder::Desktop)?, &lnk);
    if desktop || fs::exists(&on_desktop) {
        shell::create_shortcut(&on_desktop, launcher, p.shortcut_args, p.shortcut_tip)?;
    }
    Ok(())
}

fn is_own_build(dir: &str) -> bool {
    let studio = fs::exists(&fs::join(dir, config::STUDIO.app_exe));
    if config::is_studio() {
        studio
    } else {
        fs::exists(&fs::join(dir, config::PLAYER.app_exe)) && !studio
    }
}

fn remove_old_versions(root: &str, keep: &str) {
    let versions = versions_dir(root);
    for e in fs::read_dir(&versions) {
        let path = fs::join(&versions, &e.name);
        if !e.is_dir || e.is_link || e.name.eq_ignore_ascii_case(keep) || !is_own_build(&path) {
            continue;
        }
        // A client that's still running keeps its folder locked; it'll go
        // on a later update.
        match fs::remove_dir_all(&path) {
            Ok(()) => log!("removed {path}"),
            Err(err) => log!("could not remove {path}: {err}"),
        }
    }
}

pub fn uninstall() -> Result<()> {
    let p = product();
    let root = root()?;
    ui::disable_cancel();
    ui::busy();
    ui::status(&format!("Uninstalling {}", p.name));

    if let Some(scheme) = reg::get_str(p.reg_key, "protocol handler scheme") {
        if !scheme.is_empty() {
            reg::delete_tree(&format!("Software\\Classes\\{scheme}"));
        }
    }
    reg::delete_tree(&format!("Software\\Classes\\{}", config::scheme()));
    if config::is_studio() {
        unregister_place_files(&launcher_path(&root));
    }
    reg::delete_tree(p.reg_key);
    reg::delete_tree(p.uninstall_key);
    shell::notify_associations_changed();

    let lnk = format!("{}.lnk", p.shortcut_name);
    if let Ok(desktop) = shell::folder(Folder::Desktop) {
        let _ = fs::remove_file(&fs::join(&desktop, &lnk));
    }
    if let Ok(programs) = shell::folder(Folder::Programs) {
        let folder = fs::join(&programs, config::PRODUCT);
        let _ = fs::remove_file(&fs::join(&folder, &lnk));
        fs::remove_dir_if_empty(&folder);
    }

    let versions = versions_dir(&root);
    let mut locked = false;
    for e in fs::read_dir(&versions) {
        let path = fs::join(&versions, &e.name);
        if e.is_dir && !e.is_link && is_own_build(&path) {
            if let Err(err) = fs::remove_dir_all(&path) {
                log!("{err}");
                locked = true;
            }
        }
    }
    let _ = fs::remove_dir_all(&downloads_dir());
    if locked {
        bail!("Some files could not be removed. Close {} if it's running and try again.", p.name);
    }

    // This runs from a copy in %TEMP% (see app.rs), so the installed
    // launcher can go too, once the copy that started us has exited.
    let launcher = launcher_path(&root);
    for _ in 0..20 {
        if fs::remove_file(&launcher).is_ok() {
            break;
        }
        win::sleep(250);
    }
    let _ = fs::remove_file(&format!("{launcher}.old"));
    fs::remove_dir_if_empty(&versions);
    fs::remove_dir_if_empty(&root);
    log!("uninstalled");
    Ok(())
}

/// Whether `exe` is the installed launcher, which can't delete itself.
pub fn is_installed_launcher(exe: &str) -> bool {
    root().is_ok_and(|root| exe.eq_ignore_ascii_case(&launcher_path(&root)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // cargo test --lib -- --ignored
    #[test]
    #[ignore = "downloads from the live setup server"]
    fn live_package_unpacks_and_matches_manifest() {
        let http = Session::new().unwrap();
        let version = latest_version(&http).unwrap();
        let dir = format!("{}live-test", fs::temp_dir());
        let _ = fs::remove_dir_all(&dir);

        for (package, subdir) in [("content-sky.zip", "content\\sky"), ("shaders.zip", "shaders")] {
            let zip_path = fs::join(&fs::temp_dir(), package);
            http.download(&format!("{SETUP_URL}/{version}-{package}"), &zip_path, &mut |_, _| Ok(()))
                .unwrap();
            unpack(&zip_path, &fs::join(&dir, subdir), &mut |_, _| Ok(())).unwrap();
            fs::remove_file(&zip_path).unwrap();
        }

        let manifest = http.get_string(&format!("{SETUP_URL}/{version}-rbxManifest.txt")).unwrap();
        for prefix in ["content\\sky\\", "shaders\\"] {
            assert!(manifest.contains(prefix), "manifest lists nothing under {prefix}");
            assert_eq!(first_mismatch(&dir, &manifest, prefix).unwrap(), None);
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
