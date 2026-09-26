# URL Router

A tiny Windows program you set as your **default browser**. It doesn't browse
anything itself: every link you click is handed to a real browser chosen by
rules in a plain text file. For example, YouTube goes to Firefox and
everything else goes to Chrome.

- One ~400 KB `urlrouter.exe`. No installer, no .NET, no Visual C++ runtime,
  no DLLs to carry around. It uses only system DLLs that ship with Windows
  10 and 11.
- It registers for the current user only, so it doesn't need admin rights.
- It's portable: keep `urlrouter.exe` and `urlrouter.txt` together in any
  folder, e.g. `C:\Tools\URLRouter\` or on a USB stick.
- Rule changes apply on the next click, with nothing to restart.

## Setup

1. Put `urlrouter.exe` in the folder where it will stay.
2. Double-click it and choose **Yes** (Register). This writes the browser
   registration under `HKEY_CURRENT_USER` and opens *Settings → Default apps*.
3. Select **URL Router** as the web browser. On Windows 11, open its page and
   press **Set default** at the top.
4. Double-click it again and choose **No** (Edit rules) to change `urlrouter.txt`.

Windows 10 and 11 don't let a program make itself the default browser. The
user has to pick it in Settings, so step 3 is always manual.

If you move the .exe, run it again and choose Register so the registry
points at its new location.

## Rules: `urlrouter.txt`

The first run creates this file next to the .exe, with comments that explain
the format. If that folder is read-only (e.g. Program Files), it goes in
`%APPDATA%\URLRouter\` instead.

```text
# <pattern>                  <browser> [extra arguments]
youtube.com                  firefox
youtu.be                     firefox
docs.google.com/spreadsheets edge
https://github.com/my-org    work
example.com                  chrome --app=%URL%

browser work "C:\Program Files\Mozilla Firefox\firefox.exe" -P Work

*                            chrome
```

Rules are checked from top to bottom and **the first match wins**, so keep
`*` last.

| Pattern | Matches |
|---|---|
| `youtube.com` | That host **and all its subdomains** (`www.`, `m.`, `music.`) on any scheme. It does not match `notyoutube.com`. |
| `docs.google.com/document` | That host, limited to paths that start with `/document`. |
| `https://github.com/my-org` | Anything containing `://` is a literal, case-insensitive prefix of the full URL. |
| `*` (or a `default chrome` line) | Everything. |

**Browsers** can be given as:

- **A built-in name:** `chrome`, `firefox`, `edge`, `brave`, `vivaldi`,
  `opera` or `librewolf`. It is found automatically through the registry's
  *App Paths* entry or the usual install folders, whether the browser is
  installed per-user or per-machine.
- **A full path:** `"C:\Path\To\browser.exe"`. Use quotes when the path has
  spaces. `%ENV%` variables such as `%LOCALAPPDATA%` are expanded.
- **A name defined with `browser <name> <command>`:** useful for profiles
  such as `firefox -P Work` or `chrome --profile-directory="Profile 2"`.

Any text after the browser is passed as extra arguments. The URL is added as
the last argument, unless you put `%URL%` where it should go.

A line with a mistake is skipped, and a warning names the line number. Links
still open. If no rule matches (because there's no `*`), the link opens in Edge.

Lines that start with `#` or `;` are comments. Comments can't go at the end
of a rule line.

## Command line

```text
urlrouter.exe <url>              open the URL using the rules (what Windows runs)
urlrouter.exe --test <url>       show which rule matches and the exact command, without opening it
urlrouter.exe --register         register as a browser for the current user
urlrouter.exe --unregister       remove the registration
urlrouter.exe --edit             open urlrouter.txt
urlrouter.exe --launch-default   open the * browser with no URL (the Start-menu "browser" action)
urlrouter.exe                    menu: register / edit rules
```

## What `--register` writes (all under HKCU)

- `Software\Classes\URLRouterURL`: the handler for `http` and `https`.
  Its command is `"…\urlrouter.exe" "%1"`.
- `Software\Classes\URLRouterHTML`: the handler for `.htm`, `.html`,
  `.shtml`, `.xht` and `.xhtml` files.
- `Software\Clients\StartMenuInternet\URLRouter` with a `Capabilities`
  subkey: this is what makes Windows list it as a browser.
- `Software\RegisteredApplications\URLRouter`

`--unregister` deletes exactly these keys.

## Building

The program is Rust with **no crate dependencies**. It calls the few Win32
APIs it needs through hand-written declarations in `src/win.rs`. The C runtime
is linked statically (`.cargo/config.toml`), so the output runs on a clean
Windows install.

**On Windows:** install [Rust](https://rustup.rs) with the default MSVC
toolchain, then:

```powershell
cargo build --release
# -> target\release\urlrouter.exe
```

**From Linux, macOS or WSL:** you need `rustup` and the MinGW-w64 linker
(`apt install gcc-mingw-w64-x86-64`). Then:

```sh
./build.sh
# -> dist/urlrouter.exe
```

The rule-matching logic is in `src/rules.rs`. It is platform-independent and
covered by unit tests that run anywhere:

```sh
cargo test
```
