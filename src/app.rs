//! What to do with the command line we were given.
//!
//!     Better16Launcher.exe                    install / update / repair, then open the site
//!     Better16Launcher.exe better16://...     update if needed, then join the game
//!     Better16Launcher.exe -uninstall [-quiet]
//!     Better16Launcher.exe --version

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::config::{self, product};
use crate::error::{Error, Result};
use crate::http::Session;
use crate::install;
use crate::link;
use crate::log::log;
use crate::shell;
use crate::ui;
use crate::win::{self, Owned, wide};

const MB_ICONINFORMATION: u32 = 0x40;
const MB_ICONERROR: u32 = 0x10;
const MB_YESNO: u32 = 0x04;
const IDYES: i32 = 6;

#[derive(Default)]
struct Options {
    link: Option<String>,
    uninstall: bool,
    quiet: bool,
    no_self_update: bool,
    version: bool,
    ide: bool,
    file: Option<String>,
}

fn options(args: &[String]) -> Options {
    let scheme = config::scheme();
    let mut o = Options::default();
    for arg in args {
        let lower = arg.to_ascii_lowercase();
        if lower.starts_with(&format!("{scheme}:")) {
            o.link = Some(arg.clone());
            continue;
        }
        if config::is_studio() && !arg.starts_with('-') && crate::fs::exists(arg) {
            o.file = Some(arg.clone());
            continue;
        }
        match lower.trim_start_matches('-') {
            "uninstall" => o.uninstall = true,
            "quiet" | "q" => o.quiet = true,
            "no-self-update" => o.no_self_update = true,
            "version" => o.version = true,
            "ide" if config::is_studio() => o.ide = true,
            // Old shortcuts pass things like -browser; nothing to do.
            _ => log!("ignoring argument {arg}"),
        }
    }
    o
}

/// The link with its ticket blanked out, for the log.
fn redact(link: &str) -> String {
    let mut out = String::new();
    for (i, part) in link.split('&').enumerate() {
        if i > 0 {
            out.push('&');
        }
        let lower = part.to_ascii_lowercase();
        if lower.starts_with("key=") || lower.contains("gameinfo:") || lower.contains("gameinfo=") {
            out.push_str(&part[..part.find(['=', ':']).map_or(0, |p| p + 1)]);
            out.push_str("***");
        } else {
            out.push_str(part);
        }
    }
    out
}

pub fn run() -> u32 {
    crate::log::open();
    let args = win::args();
    let o = options(&args);
    let name = product().name;
    match &o.link {
        Some(l) => log!("{name} Launcher {} started from a link: {}", config::VERSION, redact(l)),
        None => log!("{name} Launcher {} started without a link (install / repair)", config::VERSION),
    }

    install::remove_stale_launcher();

    if o.version {
        win::message_box(
            core::ptr::null_mut(),
            &format!("{name} Launcher {}", config::VERSION),
            name,
            MB_ICONINFORMATION,
        );
        return 0;
    }

    if o.uninstall {
        let ok = o.quiet
            || win::message_box(
                core::ptr::null_mut(),
                &format!("Remove {name} and everything it downloaded?"),
                name,
                MB_YESNO,
            ) == IDYES;
        if !ok {
            return ui::EXIT_CANCELLED;
        }
        // The installed launcher can't delete itself, so hand the job to a
        // copy in %TEMP%, the way most uninstallers do it.
        let me = win::current_exe();
        if install::is_installed_launcher(&me) {
            let copy = format!("{}{}Uninstall.exe", crate::fs::temp_dir(), product().update_name);
            let cmd = format!("{} -uninstall -quiet", quote(&copy));
            let handed_off = crate::fs::copy(&me, &copy).and_then(|()| spawn(&copy, &cmd, None));
            match handed_off {
                Ok(_) => return 0,
                Err(e) => log!("could not run the uninstaller from TEMP: {e}"),
            }
        }
        return ui::run(install::uninstall);
    }

    // Check the link before showing anything, so a broken one gets a clear
    // message instead of a download.
    if let Some(l) = &o.link {
        if let Err(msg) = parse_link(l) {
            log!("bad link: {msg}");
            let what = if config::is_studio() { "link" } else { "game link" };
            win::message_box(
                core::ptr::null_mut(),
                &format!("This {what} can't be opened: {msg}."),
                name,
                MB_ICONERROR,
            );
            return ui::EXIT_FAILED;
        }
    }

    ui::run(work)
}

fn parse_link(uri: &str) -> core::result::Result<String, &'static str> {
    let params = link::parse(uri, &config::scheme()).ok_or("it isn't a Better16 link")?;
    if config::is_studio() {
        link::studio_args_from(&params)
    } else {
        link::launch_from(&params).map(|l| l.client_args())
    }
}

fn start_args(o: &Options) -> Option<String> {
    if let Some(args) = o.link.as_deref().and_then(|l| parse_link(l).ok()) {
        return Some(args);
    }
    if !config::is_studio() {
        return None;
    }
    let mut args = String::from("-ide");
    if let Some(file) = &o.file {
        args.push_str(" -fileLocation ");
        args.push_str(&quote(file));
    }
    Some(args)
}

fn work() -> Result<()> {
    let args = win::args();
    let o = options(&args);
    let start = start_args(&o);

    let _lock = single_instance()?;
    ui::busy();
    ui::status(&format!("Connecting to {}...", product().name));
    let http = Session::new()?;

    let version = match install::latest_version(&http) {
        Ok(v) => v,
        Err(e) => {
            let root = install::root()?;
            match (&start, install::installed_version(&root)) {
                (Some(a), Some(v)) => {
                    log!("could not check for updates ({e}), starting {v} as is");
                    return play(&install::version_dir(&root, &v), a);
                }
                _ => return Err(e),
            }
        }
    };
    log!("server version {version}");

    if !o.no_self_update {
        if let Some(exe) = install::find_update(&http, &version)? {
            let mut cmd = quote(&exe);
            for a in &args {
                cmd.push(' ');
                cmd.push_str(&quote(a));
            }
            cmd.push_str(" --no-self-update");
            spawn(&exe, &cmd, None)?;
            return Ok(());
        }
    }

    let first_install = install::installed_version(&install::root()?).is_none();

    // A plain start is the "fix it" button, so that's when files get checked.
    let plain = o.link.is_none() && !o.ide && o.file.is_none();
    let dir = install::ensure(&http, &version, plain)?;

    match &start {
        Some(a) => play(&dir, a),
        // Fresh install from the downloaded exe: say so and point at the
        // site's Play buttons. Otherwise just take them to the site.
        None if first_install => {
            ui::success();
            Ok(())
        }
        None => {
            shell::open_url(config::BASE_URL);
            Ok(())
        }
    }
}

fn play(dir: &str, args: &str) -> Result<()> {
    ui::disable_cancel();
    ui::busy();
    ui::status(&format!("Starting {}...", product().name));

    let event = config::started_event_name();
    let exe = crate::fs::join(dir, product().app_exe);
    let mut cmd = format!("{} {args}", quote(&exe));
    if config::is_studio() {
        cmd.push_str(" -startEvent ");
        cmd.push_str(&event);
    }

    let started = Owned(unsafe { win::CreateEventW(core::ptr::null_mut(), 1, 0, wide(&event).as_ptr()) });

    let process = spawn(&exe, &cmd, Some(dir))?;
    unsafe { win::AllowSetForegroundWindow(u32::MAX) };

    let handles = [started.0, process.0];
    let count = if started.is_valid() { 2 } else { 1 };
    let first = if started.is_valid() { 0 } else { 1 };
    let r = unsafe { win::WaitForMultipleObjects(count, handles[first..].as_ptr(), 0, 20_000) };
    match r {
        win::WAIT_TIMEOUT => log!("the client didn't report in within 20 s"),
        _ if count == 2 && r == win::WAIT_OBJECT_0 + 1 => log!("the client exited during startup"),
        _ => log!("client started"),
    }
    Ok(())
}

/// Holds the launcher-wide lock. Two launchers unpacking into the same
/// folder at once would make a mess, and a double-clicked Play button is
/// all it takes.
struct Lock(Owned);

impl Drop for Lock {
    fn drop(&mut self) {
        unsafe { win::ReleaseMutex(self.0.0) };
    }
}

fn single_instance() -> Result<Lock> {
    let mutex = Owned(unsafe { win::CreateMutexW(core::ptr::null_mut(), 0, wide(product().update_name).as_ptr()) });
    if !mutex.is_valid() {
        return Err(Error::last_os("CreateMutex"));
    }
    let mut told = false;
    loop {
        match unsafe { win::WaitForSingleObject(mutex.0, 250) } {
            win::WAIT_OBJECT_0 | win::WAIT_ABANDONED => return Ok(Lock(mutex)),
            win::WAIT_TIMEOUT => {
                if !told {
                    ui::status("Please wait...");
                    told = true;
                }
                ui::check_cancel()?;
            }
            _ => return Err(Error::last_os("WaitForSingleObject")),
        }
    }
}

/// Quotes one argument the way CommandLineToArgvW reads it back.
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return String::from(arg);
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(core::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
                continue;
            }
            _ => {}
        }
        if c != '\\' {
            out.extend(core::iter::repeat_n('\\', backslashes));
            backslashes = 0;
            out.push(c);
        }
    }
    out.extend(core::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

fn spawn(exe: &str, cmd: &str, dir: Option<&str>) -> Result<Owned> {
    log!("starting {exe}");
    let mut si: win::StartupInfo = unsafe { core::mem::zeroed() };
    si.cb = core::mem::size_of::<win::StartupInfo>() as u32;
    let mut pi: win::ProcessInformation = unsafe { core::mem::zeroed() };
    let mut cmd: Vec<u16> = wide(cmd);
    let dir = dir.map(wide);
    let ok = unsafe {
        win::CreateProcessW(
            wide(exe).as_ptr(),
            cmd.as_mut_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0,
            0,
            core::ptr::null_mut(),
            dir.as_ref().map_or(core::ptr::null(), |d| d.as_ptr()),
            &mut si,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(Error::last_os(&format!("Could not start {exe}")));
    }
    unsafe { win::CloseHandle(pi.thread) };
    Ok(Owned(pi.process))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_round_trips_simple_cases() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote("C:\\Program Files\\x.exe"), "\"C:\\Program Files\\x.exe\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("dir with space\\"), "\"dir with space\\\\\"");
        assert_eq!(quote(""), "\"\"");
    }

    #[test]
    fn tickets_stay_out_of_the_log() {
        assert_eq!(redact("better16://placeId=1&key=SECRET"), "better16://placeId=1&key=***");
        assert_eq!(
            redact("better16-studio://launchmode=build&gameinfo=SECRET&avatar=true"),
            "better16-studio://launchmode=build&gameinfo=***&avatar=true"
        );
    }
}
