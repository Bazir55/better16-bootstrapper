# better16-bootstrapper

The launcher for [Better16](https://www.better16.wtf). It downloads the player,
keeps it up to date, registers the `better16://` links the site's Play button
uses, and starts the game.

One small exe (most of its 600-odd KB is the window's pictures), no installer,
no admin rights. Runs on Windows XP SP2 through
Windows 11.

## How it works

- `Better16Launcher.exe` with no arguments installs or repairs the player and
  opens the site.
- `Better16Launcher.exe better16://placeId=...&key=...` updates if needed and
  joins the game.
- `Better16Launcher.exe -uninstall` removes it again.

Everything goes into `%LOCALAPPDATA%\Better16`. Packages come from the setup
server as `/<version>-<package>.zip`; the current version is whatever
`/version` returns. A plain start also checks every file against
`/<version>-rbxManifest.txt`. A log is written to `%TEMP%\Better16\launcher.log`.

## Why it looks the way it does

Rust's standard library needs Windows 10 now, and the C runtimes that ship with
current Visual Studio call Vista APIs. So the launcher uses neither: it's
`no_std`, links no CRT, and declares every Win32 function it calls by hand in
[`src/win.rs`](src/win.rs). All of them exist on XP. The handful of things a C
runtime would normally supply (allocator, `memcpy`, stack probes, 64-bit
division on x86) are in [`src/rt.rs`](src/rt.rs). Zip inflate is
[miniz_oxide](https://crates.io/crates/miniz_oxide), the only dependency.

HTTP goes through WinINet, so IE proxy settings are respected. The setup server
is reached over plain http by default, since XP can't do TLS 1.2; packages are
CRC-checked on unpack.

## Building

Needs Rust 1.88 or newer and the MSVC linker (Visual Studio Build Tools).

```
rustup target add i686-pc-windows-msvc
cargo build --release
```

The exe ends up in `target/i686-pc-windows-msvc/release/`. To point a build at
another deployment:

```
set B16_SETUP_URL=http://setup.example.org
set B16_BASE_URL=https://www.example.org
cargo build --release
```

The URL scheme and the other names are derived from those two.

The window's pictures (logo and buttons, each with an `@2x` copy for
high-DPI screens) are the `.bmp` files in `assets/` and get embedded at build
time. If `assets/better16.ico` exists it becomes the program icon.

Tests: `cargo test --lib`. Add `-- --include-ignored` to also run the one that
downloads real packages from the setup server.

## License

MIT, see [LICENSE](LICENSE).
