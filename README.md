<img src="assets/icon.png" alt="" width="112" align="right">

# URL Router

A tiny program for **Windows and macOS** that you set as your **default
browser**. It doesn't browse anything itself: every link you click is handed
to a real browser chosen by rules in a plain text file. For example, YouTube
goes to Firefox, everything else goes to Chrome, and Slack links open
straight in the **Slack app** instead of a browser tab.

- No installer and no runtime. On Windows it's a single ~400 KB
  `urlrouter.exe` that uses only DLLs that ship with Windows 10 and 11. On
  macOS it's a small native app for Apple silicon and Intel, macOS 11 or later.
- It never needs admin rights.
- Rule changes apply on the next click, with nothing to restart.

## Download

Get the files from the
[latest release](https://github.com/ruehsn/urlrouter/releases/latest).
Neither is signed by a paid developer certificate, so the first launch needs
one extra step:

- **Windows: `urlrouter.exe`.** SmartScreen warns the first time you run
  it. Choose **More info → Run anyway**.
- **Mac: `URLRouter-mac.zip`.** Unzip it and move **URL Router** to
  Applications. The first time you open it, macOS says it can't verify the
  developer. Go to **System Settings → Privacy & Security**, scroll down, and
  click **Open Anyway** next to the message about URL Router. To skip the
  prompt instead, run this in Terminal:
  `xattr -dr com.apple.quarantine "/Applications/URL Router.app"`

## Setup on Windows

1. Put `urlrouter.exe` in the folder where it will stay.
2. Double-click it and choose **Yes** (Register). This writes the browser
   registration under `HKEY_CURRENT_USER` and opens *Settings → Default apps*.
3. Select **URL Router** as the web browser. On Windows 11, open its page and
   press **Set default** at the top.
4. Double-click it again and choose **No** (Edit rules) to change `urlrouter.txt`.

Windows 10 and 11 don't let a program make itself the default browser. The
user has to pick it in Settings, so step 3 is always manual.

It's portable: keep `urlrouter.exe` and `urlrouter.txt` together in any
folder. If you move the .exe, run it again and choose Register so the
registry points at its new location.

## Setup on Mac

1. Move **URL Router** to Applications.
2. Open it and choose **Make Default Browser**. macOS asks you to confirm:
   choose **Use "URL Router"**. You can also set it later in **System
   Settings → Desktop & Dock → Default web browser**.
3. Open it again and choose **Edit Rules** to change `urlrouter.txt`.

URL Router has no Dock icon or window. When you click a link it starts,
hands the link on, and quits within a fraction of a second.

## Rules: `urlrouter.txt`

The first run creates this file with comments that explain the format:

- **Windows:** next to the .exe. If that folder is read-only (e.g. Program
  Files), it goes in `%APPDATA%\URLRouter\` instead.
- **Mac:** `~/Library/Application Support/URLRouter/urlrouter.txt`.

```text
# <pattern>                  <browser> [extra arguments]
slack.com                    slack
youtube.com                  firefox
youtu.be                     firefox
docs.google.com/spreadsheets edge
https://github.com/my-org    work
example.com                  chrome --app=%URL%

browser work chrome --profile-directory="Profile 1"

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

- **A built-in name**, found automatically wherever the browser is installed:
  - `chrome`, `firefox`, `edge`, `brave`, `vivaldi` and `opera` on both
    platforms.
  - `safari` and `arc` on Mac.
  - `librewolf` on Windows.
- **`slack`:** opens the link in the Slack app. See
  [Slack links](#slack-links) below.
- **Any other app:**
  - Windows: a full path such as `"C:\Path\To\browser.exe"`. `%ENV%`
    variables such as `%LOCALAPPDATA%` are expanded.
  - Mac: an app name such as `"Google Chrome Canary"`, or a path such as
    `"/Applications/Some Browser.app"`.

  Use quotes when there are spaces.
- **A name you define with `browser <name> <command>`:** useful for profiles
  such as `chrome --profile-directory="Profile 2"` or `firefox -P Work`.
  Chrome's profile names are folder names; see the **Profile Path** at
  `chrome://version`.

Any text after the browser is passed as extra arguments. The URL is added as
the last argument, unless you put `%URL%` where it should go.

A line with a mistake is skipped, and a warning names the line number. Links
still open. If no rule matches (because there's no `*`), the link opens in
Edge on Windows or Safari on Mac.

Lines that start with `#` or `;` are comments. Comments can't go at the end
of a rule line.

## Slack links

With a `slack` rule, Slack links open directly in the Slack desktop app.
There's no browser tab and no "Open in Slack?" page.

```text
slack-team   acme   T0123ABCD
slack.com    slack
*            chrome
```

These links open in the app:

| Link | Opens |
|---|---|
| `acme.slack.com/archives/C…` | the channel or DM |
| `acme.slack.com/archives/C…/p…` (a message link) | the message in its channel |
| `acme.slack.com/team/U…` | the person's profile |
| `acme.slack.com/files/…/F…` and `files.slack.com/files-pri/T…-F…` | the file |
| `acme.slack.com/` | the workspace |
| `app.slack.com/client/T…/…` | the channel, message, profile, or workspace |

**Workspace links need the team ID.** Slack's app links use the workspace's
team ID, which `acme.slack.com` addresses don't contain. Add a
`slack-team <workspace> <team ID>` line for each workspace you use. To find
the ID, open the workspace in a browser: the address changes to
`app.slack.com/client/T0123ABCD/…`, and the part starting with `T` (or `E`
for Enterprise Grid) is the team ID. For an Enterprise Grid address like
`acme.enterprise.slack.com`, use `slack-team acme.enterprise E0123ABCD`.
`app.slack.com` links already contain the ID, so they work without any
`slack-team` line.

**Some links fall through to the next rule.** A Slack link that can't be
opened in the app moves on to the next matching rule, usually your browser.
That includes:
- sign-in, admin, help and pricing pages, and `api.slack.com`
- workspaces without a `slack-team` line

Message links ask Slack to jump to that message. If Slack doesn't, you still
land in the right channel.

## Command line

**Windows:**

```text
urlrouter.exe <url>              open the URL using the rules (what Windows runs)
urlrouter.exe --test <url>       show which rule matches and the exact command, without opening it
urlrouter.exe --register         register as a browser for the current user
urlrouter.exe --unregister       remove the registration
urlrouter.exe --edit             open urlrouter.txt
urlrouter.exe --launch-default   open the * browser with no URL (the Start-menu "browser" action)
urlrouter.exe                    menu: register / edit rules
```

**Mac** (in Terminal; the program is inside the app):

```text
"/Applications/URL Router.app/Contents/MacOS/urlrouter" --test <url>
```

The Mac program supports the same `--test`, `--register`, `--edit` and
`--launch-default` options, and prints its results in Terminal. To stop
using URL Router on a Mac, pick another browser in System Settings.

## What `--register` writes on Windows (all under HKCU)

- `Software\Classes\URLRouterURL`: the handler for `http` and `https`.
  Its command is `"…\urlrouter.exe" "%1"`.
- `Software\Classes\URLRouterHTML`: the handler for `.htm`, `.html`,
  `.shtml`, `.xht` and `.xhtml` files.
- `Software\Clients\StartMenuInternet\URLRouter` with a `Capabilities`
  subkey: this is what makes Windows list it as a browser.
- `Software\RegisteredApplications\URLRouter`

`--unregister` deletes exactly these keys.

## How it's built

It's written in Rust with **no crate dependencies**:

| File | What it does |
|---|---|
| `src/rules.rs` | the shared rule engine |
| `src/slack.rs` | converts Slack links |
| `src/win.rs` | the Win32 calls (registry, dialogs), declared by hand |
| `src/mac.rs` | the AppKit and LaunchServices calls, declared by hand |
| `src/win_app.rs`, `src/mac_app.rs` | each platform's app |
| `build.rs` | embeds the icon in the Windows exe |
| `assets/make_icons.py` | draws the icon and writes the `.ico`, `.icns` and previews |

On macOS, links reach the default browser as Apple Events rather than
command-line arguments, so the Mac app runs a minimal `NSApplication` to
receive them.

The icon's three-way split uses the colours of Firefox, Edge and Chrome, but
it's an original drawing rather than a mix of their logos, which their owners
don't allow to be altered or combined. To change it, edit
`assets/make_icons.py`, run it (it needs Pillow), and commit the files it
writes.

**Windows, on Windows:** install [Rust](https://rustup.rs) with the default
MSVC toolchain, then run `cargo build --release`. The output is
`target\release\urlrouter.exe`. The C runtime is linked statically
(`.cargo/config.toml`), so it runs on a clean Windows install.

**Windows, from Linux, macOS or WSL:** you need `rustup` and the MinGW-w64
linker (`apt install gcc-mingw-w64-x86-64`). Then run `./build.sh`. The
output is `dist/urlrouter.exe`.

**Mac:** you need Rust and the Xcode command-line tools. Run
`macos/build-app.sh`. It builds a universal binary, assembles and ad-hoc-signs
`dist/URL Router.app`, and zips it. `macos/e2e-test.sh` then opens a link
through the built app the way macOS does, and checks where it ends up. It
backs up your own `urlrouter.txt` and restores it afterwards.

The rule engine and the Slack converter are covered by unit tests that run
on any platform:

```sh
cargo test
```

## Releases

GitHub Actions ([`.github/workflows/build.yml`](.github/workflows/build.yml))
runs on every push and pull request:
- builds and tests the Windows exe on a Windows runner
- builds the Mac app on a macOS runner and runs its end-to-end test

The outputs are attached to each run.

When a push to `main` carries a `version` in `Cargo.toml` that has no release
yet, the workflow publishes release `v<version>` with both downloads
attached. To ship a new version, bump `version` in `Cargo.toml` and push.
