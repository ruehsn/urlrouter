//! The Objective-C runtime, AppKit and LaunchServices calls the Mac app needs,
//! declared by hand so the crate stays dependency-free. Every framework
//! linked here ships with macOS.
//!
//! macOS hands links to the default browser as Apple Events delivered through
//! AppKit, never as command-line arguments, so the app runs a real
//! NSApplication with a delegate class built at runtime.

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

type Id = *mut c_void;
type Sel = *const c_void;
type Imp = *const c_void;

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra_bytes: usize) -> Id;
    fn objc_registerClassPair(class: Id);
    fn class_addMethod(class: Id, name: Sel, imp: Imp, types: *const c_char) -> u8;
    fn objc_msgSend();
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[link(name = "Foundation", kind = "framework")]
extern "C" {}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    // CFStringRef parameters; NSString is toll-free bridged, so we pass those.
    fn LSSetDefaultHandlerForURLScheme(scheme: Id, bundle_id: Id) -> i32;
    fn LSSetDefaultRoleHandlerForContentType(content_type: Id, role: u32, bundle_id: Id) -> i32;
}

/// Sends an Objective-C message. objc_msgSend has to be called through a
/// pointer of the exact function type (it isn't variadic on Apple silicon),
/// so every call states its argument and return types: `arg; Type`.
macro_rules! msg {
    ($ret:ty, $recv:expr, $sel:literal $(, $arg:expr; $t:ty)*) => {{
        #[allow(clippy::missing_transmute_annotations)]
        let f: unsafe extern "C" fn(Id, Sel $(, $t)*) -> $ret =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f($recv, sel(concat!($sel, "\0")) $(, $arg)*)
    }};
}

/// `name` must be nul-terminated.
fn sel(name: &str) -> Sel {
    unsafe { sel_registerName(name.as_ptr().cast()) }
}

/// `name` must be nul-terminated.
fn class(name: &str) -> Id {
    unsafe { objc_getClass(name.as_ptr().cast()) }
}

fn nsstring(s: &str) -> Id {
    let c = CString::new(s.replace('\0', " ")).expect("NULs were replaced");
    unsafe { msg!(Id, class("NSString\0"), "stringWithUTF8String:", c.as_ptr(); *const c_char) }
}

fn rust_string(ns: Id) -> Option<String> {
    if ns.is_null() {
        return None;
    }
    let p = unsafe { msg!(*const c_char, ns, "UTF8String") };
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

fn shared_app() -> Id {
    unsafe { msg!(Id, class("NSApplication\0"), "sharedApplication") }
}

/// Path of the installed app with this bundle identifier, wherever
/// LaunchServices knows it to be (/Applications, ~/Applications, ...).
pub fn app_path_for_bundle_id(bundle_id: &str) -> Option<String> {
    unsafe {
        let ws = msg!(Id, class("NSWorkspace\0"), "sharedWorkspace");
        let url = msg!(Id, ws, "URLForApplicationWithBundleIdentifier:", nsstring(bundle_id); Id);
        if url.is_null() {
            return None;
        }
        rust_string(msg!(Id, url, "path"))
    }
}

/// The enclosing .app bundle. None when the binary runs outside a bundle
/// (then mainBundle is just the binary's folder and has no identifier).
pub fn own_bundle() -> Option<(String, String)> {
    unsafe {
        let bundle = msg!(Id, class("NSBundle\0"), "mainBundle");
        let id = rust_string(msg!(Id, bundle, "bundleIdentifier"))?;
        let path = rust_string(msg!(Id, bundle, "bundlePath"))?;
        Some((id, path))
    }
}

/// Asks LaunchServices to make `bundle_id` the default browser. Since macOS
/// 12 the system then shows its own "Use … / Keep …" confirmation; apps
/// cannot change the default browser silently.
pub fn set_default_browser(bundle_id: &str) -> Result<(), String> {
    let id = nsstring(bundle_id);
    for scheme in ["http", "https"] {
        let rc = unsafe { LSSetDefaultHandlerForURLScheme(nsstring(scheme), id) };
        if rc != 0 {
            return Err(format!(
                "macOS refused to make URL Router the {scheme} handler (error {rc})."
            ));
        }
    }
    // Also open local .html files through the rules. Best-effort: browsers
    // work fine without it.
    const VIEWER: u32 = 0x2; // kLSRolesViewer
    unsafe { LSSetDefaultRoleHandlerForContentType(nsstring("public.html"), VIEWER, id) };
    Ok(())
}

#[derive(Clone, Copy)]
pub enum AlertStyle {
    Warning = 0,
    Informational = 1,
    Critical = 2,
}

/// Shows a modal alert in front of other apps and returns the index of the
/// button pressed (0 = first).
pub fn alert(title: &str, text: &str, buttons: &[&str], style: AlertStyle) -> usize {
    unsafe {
        let app = shared_app();
        // We are a background-only app (LSUIElement), so bring ourselves
        // forward or the alert opens behind whatever the user was using.
        if msg!(u8, app, "respondsToSelector:", sel("activate\0"); Sel) != 0 {
            msg!((), app, "activate"); // macOS 14+
        } else {
            msg!((), app, "activateIgnoringOtherApps:", 1; i8);
        }
        let alert = msg!(Id, class("NSAlert\0"), "new");
        msg!((), alert, "setMessageText:", nsstring(title); Id);
        msg!((), alert, "setInformativeText:", nsstring(text); Id);
        msg!((), alert, "setAlertStyle:", style as usize; usize);
        for b in buttons {
            msg!(Id, alert, "addButtonWithTitle:", nsstring(b); Id);
        }
        // NSAlertFirstButtonReturn is 1000, then 1001, ...
        (msg!(isize, alert, "runModal") - 1000).max(0) as usize
    }
}

static ON_URL: OnceLock<fn(&str)> = OnceLock::new();
static ON_LAUNCHED_DIRECTLY: OnceLock<fn()> = OnceLock::new();
static GOT_URLS: AtomicBool = AtomicBool::new(false);
static FINISHED_LAUNCHING: AtomicBool = AtomicBool::new(false);

fn terminate() {
    unsafe { msg!((), shared_app(), "terminate:", null_mut(); Id) };
}

/// -application:openURLs: — links (and .html files, as file:// URLs).
extern "C" fn open_urls(_this: Id, _cmd: Sel, _app: Id, urls: Id) {
    GOT_URLS.store(true, Ordering::SeqCst);
    let on_url = ON_URL.get().expect("set before the app runs");
    unsafe {
        for i in 0..msg!(usize, urls, "count") {
            let url = msg!(Id, urls, "objectAtIndex:", i; usize);
            if let Some(s) = rust_string(msg!(Id, url, "absoluteString")) {
                on_url(&s);
            }
        }
    }
    // On a cold launch this arrives before applicationDidFinishLaunching:,
    // which then quits. If we are already up, this was a later click: quit now.
    if FINISHED_LAUNCHING.load(Ordering::SeqCst) {
        terminate();
    }
}

/// -applicationDidFinishLaunching:
extern "C" fn did_finish_launching(this: Id, _cmd: Sel, _note: Id) {
    FINISHED_LAUNCHING.store(true, Ordering::SeqCst);
    if GOT_URLS.load(Ordering::SeqCst) {
        terminate();
        return;
    }
    // AppKit documents the open event as arriving before this method, but
    // give a late one half a second before concluding the user opened the
    // app directly (e.g. double-clicked it in Finder).
    unsafe {
        msg!((), this, "performSelector:withObject:afterDelay:",
            sel("urlRouterLaunchedDirectly:\0"); Sel, null_mut(); Id, 0.5; f64);
    }
}

extern "C" fn launched_directly(_this: Id, _cmd: Sel, _arg: Id) {
    if !GOT_URLS.load(Ordering::SeqCst) {
        (ON_LAUNCHED_DIRECTLY.get().expect("set before the app runs"))();
    }
    terminate();
}

/// Runs the app until it has handled the links it was opened with (calling
/// `on_url` for each) or, if it was opened without any, until
/// `on_launched_directly` returns. Never returns.
pub fn run_app(on_url: fn(&str), on_launched_directly: fn()) -> ! {
    let _ = ON_URL.set(on_url);
    let _ = ON_LAUNCHED_DIRECTLY.set(on_launched_directly);
    unsafe {
        let app = shared_app();
        // Accessory: no Dock icon or menu bar while we flash by. Info.plist's
        // LSUIElement says the same; this covers running the bare binary.
        msg!((), app, "setActivationPolicy:", 1; isize);

        let cls = objc_allocateClassPair(class("NSObject\0"), c"URLRouterAppDelegate".as_ptr(), 0);
        let methods: [(&str, Imp, &CStr); 3] = [
            (
                "application:openURLs:\0",
                open_urls as extern "C" fn(Id, Sel, Id, Id) as Imp,
                c"v@:@@",
            ),
            (
                "applicationDidFinishLaunching:\0",
                did_finish_launching as extern "C" fn(Id, Sel, Id) as Imp,
                c"v@:@",
            ),
            (
                "urlRouterLaunchedDirectly:\0",
                launched_directly as extern "C" fn(Id, Sel, Id) as Imp,
                c"v@:@",
            ),
        ];
        for (name, imp, types) in methods {
            class_addMethod(cls, sel(name), imp, types.as_ptr());
        }
        objc_registerClassPair(cls);

        // NSApplication holds its delegate weakly; this one is never
        // released, which is what we want for the life of the process.
        let delegate = msg!(Id, cls, "new");
        msg!((), app, "setDelegate:", delegate; Id);
        msg!((), app, "run");
    }
    std::process::exit(0)
}
