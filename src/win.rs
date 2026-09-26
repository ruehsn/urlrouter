//! The handful of Win32 calls the router needs, declared by hand so the crate
//! has no dependencies. Every DLL referenced here (advapi32, user32, shell32)
//! is part of every Windows install.

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use crate::rules::{self, Builtin};

type Hkey = isize;

// Predefined keys are sign-extended 32-bit values, i.e. (HKEY)(LONG)0x80000001.
const HKEY_CURRENT_USER: Hkey = 0x8000_0001u32 as i32 as isize;
const HKEY_LOCAL_MACHINE: Hkey = 0x8000_0002u32 as i32 as isize;

const KEY_WRITE: u32 = 0x0002_0006;
const REG_SZ: u32 = 1;
const RRF_RT_REG_SZ: u32 = 0x0000_0002;
const RRF_SUBKEY_WOW6464KEY: u32 = 0x0001_0000;
const RRF_SUBKEY_WOW6432KEY: u32 = 0x0002_0000;
const ERROR_SUCCESS: i32 = 0;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_MORE_DATA: i32 = 234;

pub const MB_OK: u32 = 0x0;
pub const MB_YESNOCANCEL: u32 = 0x3;
pub const MB_ICONERROR: u32 = 0x10;
pub const MB_ICONQUESTION: u32 = 0x20;
pub const MB_ICONWARNING: u32 = 0x30;
pub const MB_ICONINFORMATION: u32 = 0x40;
const MB_SETFOREGROUND: u32 = 0x0001_0000;
pub const IDYES: i32 = 6;
pub const IDNO: i32 = 7;

const SW_SHOWNORMAL: i32 = 1;
const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
const SHCNF_IDLIST: u32 = 0;
const ASFW_ANY: u32 = u32::MAX;

#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(
        hkey: Hkey,
        subkey: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        sam: u32,
        security: *const c_void,
        result: *mut Hkey,
        disposition: *mut u32,
    ) -> i32;
    fn RegSetValueExW(
        hkey: Hkey,
        name: *const u16,
        reserved: u32,
        kind: u32,
        data: *const u8,
        len: u32,
    ) -> i32;
    fn RegCloseKey(hkey: Hkey) -> i32;
    fn RegDeleteTreeW(hkey: Hkey, subkey: *const u16) -> i32;
    fn RegDeleteKeyValueW(hkey: Hkey, subkey: *const u16, name: *const u16) -> i32;
    fn RegGetValueW(
        hkey: Hkey,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        len: *mut u32,
    ) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    fn AllowSetForegroundWindow(pid: u32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: isize,
        op: *const u16,
        file: *const u16,
        params: *const u16,
        dir: *const u16,
        show: i32,
    ) -> isize;
    fn SHChangeNotify(event: i32, flags: u32, item1: *const c_void, item2: *const c_void);
}

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// Shows a message box and returns the button pressed. MB_SETFOREGROUND
/// matters because we are usually launched in the background by whatever app
/// the link was clicked in, and the box would otherwise hide behind it.
pub fn message_box(text: &str, flags: u32) -> i32 {
    let text = wide(text);
    let caption = wide("URL Router");
    unsafe { MessageBoxW(0, text.as_ptr(), caption.as_ptr(), flags | MB_SETFOREGROUND) }
}

/// Opens a file or URI with its default handler (e.g. the .txt editor, or
/// `ms-settings:` pages).
pub fn shell_open(target: &str) -> Result<(), String> {
    let op = wide("open");
    let file = wide(target);
    let rc = unsafe { ShellExecuteW(0, op.as_ptr(), file.as_ptr(), null(), null(), SW_SHOWNORMAL) };
    // ShellExecute reports success as any value greater than 32.
    if rc > 32 {
        Ok(())
    } else {
        Err(format!("Windows could not open {target} (error {rc})."))
    }
}

/// We received foreground rights from the shell when the link was clicked;
/// passing them on lets the browser window come to the front instead of just
/// flashing in the taskbar.
pub fn allow_child_foreground() {
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
}

fn reg_get_string(root: Hkey, subkey: &str, value: Option<&str>, view: u32) -> Option<String> {
    let sub = wide(subkey);
    let val = value.map(wide);
    let val_ptr = val.as_ref().map_or(null(), |v| v.as_ptr());
    // RRF_RT_REG_SZ also accepts REG_EXPAND_SZ and expands it for us.
    let flags = RRF_RT_REG_SZ | view;
    let mut len: u32 = 0;
    let rc = unsafe {
        RegGetValueW(
            root,
            sub.as_ptr(),
            val_ptr,
            flags,
            null_mut(),
            null_mut(),
            &mut len,
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    // The size of an expanded string can change between calls, so retry a
    // couple of times if it grew.
    for _ in 0..3 {
        let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
        let mut cb = (buf.len() * 2) as u32;
        let rc = unsafe {
            RegGetValueW(
                root,
                sub.as_ptr(),
                val_ptr,
                flags,
                null_mut(),
                buf.as_mut_ptr().cast(),
                &mut cb,
            )
        };
        match rc {
            ERROR_SUCCESS => {
                let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                return Some(String::from_utf16_lossy(&buf[..n]));
            }
            ERROR_MORE_DATA => len = cb,
            _ => return None,
        }
    }
    None
}

fn reg_set_string(subkey: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    let sub = wide(subkey);
    let mut key: Hkey = 0;
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            0,
            null(),
            0,
            KEY_WRITE,
            null(),
            &mut key,
            null_mut(),
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(format!(r"could not create HKCU\{subkey} (error {rc})"));
    }
    let name_w = name.map(wide);
    let data = wide(value);
    let rc = unsafe {
        RegSetValueExW(
            key,
            name_w.as_ref().map_or(null(), |n| n.as_ptr()),
            0,
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    unsafe { RegCloseKey(key) };
    if rc != ERROR_SUCCESS {
        return Err(format!(r"could not write HKCU\{subkey} (error {rc})"));
    }
    Ok(())
}

/// Finds an installed browser: the registry `App Paths` entry (which every
/// mainstream browser installer writes, per-user or per-machine, in either
/// registry view), then the usual install folders.
pub fn locate_builtin(b: &Builtin) -> Option<String> {
    let key = format!(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{}",
        b.exe
    );
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [RRF_SUBKEY_WOW6464KEY, RRF_SUBKEY_WOW6432KEY] {
            if let Some(p) = reg_get_string(root, &key, None, view) {
                let p = p.trim().trim_matches('"');
                if !p.is_empty() && Path::new(p).is_file() {
                    return Some(p.to_string());
                }
            }
        }
    }
    b.paths
        .iter()
        .map(|p| rules::expand_env(p, &env_var))
        .find(|p| Path::new(p).is_file())
}

pub fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

// Registry names. APP_ID is what Windows lists under Settings > Default apps.
pub const APP_ID: &str = "URLRouter";
const APP_NAME: &str = "URL Router";
const PROGID_URL: &str = "URLRouterURL";
const PROGID_HTML: &str = "URLRouterHTML";
const CLIENT_KEY: &str = r"Software\Clients\StartMenuInternet\URLRouter";
const REGISTERED_APPS: &str = r"Software\RegisteredApplications";

/// Registers this .exe as a browser for the current user only (HKCU), so no
/// administrator rights are needed. Windows 10/11 deliberately do not let a
/// program make itself the default — the user picks it in Settings — so this
/// only makes it *eligible* and the caller then opens that Settings page.
pub fn register(exe: &Path) -> Result<(), String> {
    let exe = exe.display().to_string();
    let open_url = format!("\"{exe}\" \"%1\"");
    let icon = format!("\"{exe}\",0");

    for (progid, desc) in [
        (PROGID_URL, "URL Router URL"),
        (PROGID_HTML, "URL Router HTML Document"),
    ] {
        let base = format!(r"Software\Classes\{progid}");
        reg_set_string(&base, None, desc)?;
        reg_set_string(&format!(r"{base}\DefaultIcon"), None, &icon)?;
        reg_set_string(&format!(r"{base}\shell\open\command"), None, &open_url)?;
    }
    // Marks the URL ProgID as a protocol handler.
    reg_set_string(
        &format!(r"Software\Classes\{PROGID_URL}"),
        Some("URL Protocol"),
        "",
    )?;

    reg_set_string(CLIENT_KEY, None, APP_NAME)?;
    reg_set_string(&format!(r"{CLIENT_KEY}\DefaultIcon"), None, &icon)?;
    // What Windows runs when it wants "the browser" with no link (e.g. the
    // Start menu entry): open the catch-all browser rather than our dialog.
    reg_set_string(
        &format!(r"{CLIENT_KEY}\shell\open\command"),
        None,
        &format!("\"{exe}\" --launch-default"),
    )?;

    let caps = format!(r"{CLIENT_KEY}\Capabilities");
    reg_set_string(&caps, Some("ApplicationName"), APP_NAME)?;
    reg_set_string(
        &caps,
        Some("ApplicationDescription"),
        "Opens each link in the browser chosen by your urlrouter.txt rules.",
    )?;
    reg_set_string(&caps, Some("ApplicationIcon"), &icon)?;
    reg_set_string(
        &format!(r"{caps}\StartMenu"),
        Some("StartMenuInternet"),
        APP_ID,
    )?;
    for scheme in ["http", "https"] {
        reg_set_string(
            &format!(r"{caps}\URLAssociations"),
            Some(scheme),
            PROGID_URL,
        )?;
    }
    for ext in [".htm", ".html", ".shtml", ".xht", ".xhtml"] {
        reg_set_string(&format!(r"{caps}\FileAssociations"), Some(ext), PROGID_HTML)?;
    }
    reg_set_string(REGISTERED_APPS, Some(APP_ID), &caps)?;

    notify_assoc_changed();
    Ok(())
}

pub fn unregister() -> Result<(), String> {
    let mut errors = Vec::new();
    for key in [
        format!(r"Software\Classes\{PROGID_URL}"),
        format!(r"Software\Classes\{PROGID_HTML}"),
        CLIENT_KEY.to_string(),
    ] {
        let k = wide(&key);
        let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
        if rc != ERROR_SUCCESS && rc != ERROR_FILE_NOT_FOUND {
            errors.push(format!(r"HKCU\{key} (error {rc})"));
        }
    }
    let sub = wide(REGISTERED_APPS);
    let name = wide(APP_ID);
    let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr()) };
    if rc != ERROR_SUCCESS && rc != ERROR_FILE_NOT_FOUND {
        errors.push(format!(r"HKCU\{REGISTERED_APPS}\{APP_ID} (error {rc})"));
    }
    notify_assoc_changed();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("Could not remove: {}", errors.join(", ")))
    }
}

fn notify_assoc_changed() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, null(), null()) };
}
