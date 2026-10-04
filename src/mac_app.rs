//! The macOS app: config file, dialogs, the Terminal commands, and launching.
//! The Objective-C and LaunchServices calls themselves are in `mac.rs`.
//!
//! Links arrive through AppKit (see `mac::run_app`). The command line is only
//! used when the binary is run from Terminal, for `--test` and friends.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::mac::{self, AlertStyle};
use crate::rules::{self, Config, Plan, Resolved};

const CONFIG_NAME: &str = "urlrouter.txt";
/// Used when no rule matches. Safari ships with macOS.
const LAST_RESORT: &str = "safari";

/// Whether feedback goes to dialogs (opened by macOS) or the terminal.
static GUI: AtomicBool = AtomicBool::new(false);

pub fn run() -> i32 {
    let args: Vec<String> = env::args().skip(1).collect();
    let cli = |r: Result<(), String>| match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("urlrouter: {e}");
            1
        }
    };
    // LaunchServices passes nothing useful on the command line (very old
    // macOS adds a -psn_ argument), so only arguments we recognise mean
    // we were started from Terminal.
    match args.first().map(String::as_str) {
        Some("--test") => match args.get(1) {
            Some(url) => cli(test(url)),
            None => cli(Err(
                "--test needs a URL, e.g.  urlrouter --test https://youtube.com/".into(),
            )),
        },
        Some("--register") => cli(register()),
        Some("--edit") => cli(edit_config()),
        Some("--launch-default") => cli(launch(None)),
        Some("--help" | "-h") => {
            println!("{}", help_text());
            0
        }
        Some(u) if u.contains("://") => cli(args.iter().try_for_each(|u| launch(Some(u)))),
        _ => {
            GUI.store(true, Ordering::SeqCst);
            mac::run_app(on_url, launched_directly)
        }
    }
}

fn on_url(url: &str) {
    if let Err(e) = launch(Some(url)) {
        mac::alert(
            "URL Router couldn't open a link",
            &e,
            &["OK"],
            AlertStyle::Critical,
        );
    }
}

/// Shown when the app is opened without a link (double-clicked).
fn launched_directly() {
    let path = match config_path() {
        Ok(p) => p,
        Err(e) => {
            mac::alert("URL Router", &e, &["OK"], AlertStyle::Critical);
            return;
        }
    };
    let choice = mac::alert(
        "URL Router",
        &format!(
            "Opens each link in the browser chosen by your rules, and Slack \
             links in the Slack app.\n\nRules file:\n{}",
            path.display()
        ),
        &["Make Default Browser", "Edit Rules", "Quit"],
        AlertStyle::Informational,
    );
    let result = match choice {
        0 => register(),
        1 => edit_config(),
        _ => Ok(()),
    };
    if let Err(e) = result {
        mac::alert("URL Router", &e, &["OK"], AlertStyle::Critical);
    }
}

fn notify(title: &str, text: &str) {
    if GUI.load(Ordering::SeqCst) {
        mac::alert(title, text, &["OK"], AlertStyle::Informational);
    } else {
        println!("{text}");
    }
}

fn warn(text: &str) {
    if GUI.load(Ordering::SeqCst) {
        mac::alert("URL Router", text, &["OK"], AlertStyle::Warning);
    } else {
        eprintln!("urlrouter: warning: {text}");
    }
}

/// ~/Library/Application Support/URLRouter/urlrouter.txt. It can't live
/// next to the binary as on Windows: that's inside the signed .app bundle,
/// and editing anything in there breaks the signature.
fn config_path() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or("HOME is not set")?;
    let dir = PathBuf::from(home).join("Library/Application Support/URLRouter");
    let path = dir.join(CONFIG_NAME);
    if !path.is_file() {
        fs::create_dir_all(&dir)
            .and_then(|()| fs::write(&path, crate::DEFAULT_CONFIG_MAC))
            .map_err(|e| format!("Could not create {}: {e}", path.display()))?;
    }
    Ok(path)
}

fn load_config() -> Result<(Config, PathBuf), String> {
    let path = config_path()?;
    let bytes = fs::read(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let cfg = Config::parse(&rules::decode_text(&bytes));
    if !cfg.errors.is_empty() {
        warn(&format!(
            "Some lines in {} were ignored:\n\n{}",
            path.display(),
            cfg.errors.join("\n")
        ));
    }
    Ok((cfg, path))
}

fn locate_builtin(b: &rules::Builtin) -> Option<String> {
    if b.mac.is_empty() {
        None
    } else {
        mac::app_path_for_bundle_id(b.mac)
    }
}

fn env_var(name: &str) -> Option<String> {
    env::var(name).ok()
}

/// Applies the rules, refusing any that point back at URL Router.
fn resolve(cfg: &Config, url: Option<&str>) -> Result<Resolved, String> {
    let r = rules::resolve(cfg, url, LAST_RESORT, &locate_builtin, &env_var)?;
    if let Plan::Run { program, .. } = &r.plan {
        refuse_self(program)?;
    }
    Ok(r)
}

/// Pointing a rule at URL Router itself would relaunch it forever.
fn refuse_self(program: &str) -> Result<(), String> {
    let by_name = Path::new(program.trim_end_matches('/'))
        .file_stem()
        .is_some_and(|n| n.eq_ignore_ascii_case("URL Router"));
    let by_path = mac::own_bundle().is_some_and(|(_, own)| {
        match (fs::canonicalize(program), fs::canonicalize(own)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    });
    if by_name || by_path {
        return Err(format!(
            "A rule points at URL Router itself ({program}); that would loop forever."
        ));
    }
    Ok(())
}

fn launch(url: Option<&str>) -> Result<(), String> {
    if let Some(u) = url {
        // `open` would read a leading '-' as one of its own options. Links
        // always start with a scheme, so refuse rather than guess.
        if u.starts_with('-') {
            return Err(format!(
                "Refusing to open something that looks like a command-line option:\n{u}"
            ));
        }
    }
    let (cfg, path) = load_config()?;
    let resolved = resolve(&cfg, url)?;
    let out = Command::new("/usr/bin/open")
        .args(rules::mac_open_args(&resolved.plan, url))
        .output()
        .map_err(|e| format!("Could not run /usr/bin/open: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let hint = match resolved.plan {
        Plan::OpenSlack(_) => "\n\nIs the Slack app installed?",
        Plan::Run { .. } => "",
    };
    Err(format!(
        "{detail}{hint}\n\nURL: {}\nRule used: {}\nConfig: {}",
        url.unwrap_or("(none)"),
        resolved.why,
        path.display()
    ))
}

fn test(url: &str) -> Result<(), String> {
    let (cfg, path) = load_config()?;
    let resolved = resolve(&cfg, Some(url))?;
    let action = match &resolved.plan {
        Plan::OpenSlack(link) => format!("open in the Slack app: {link}"),
        Plan::Run { .. } => {
            let mut cmd = String::from("open");
            for a in rules::mac_open_args(&resolved.plan, Some(url)) {
                cmd.push(' ');
                cmd.push_str(&quote(&a));
            }
            format!("run: {cmd}")
        }
    };
    println!(
        "URL:     {url}\nRule:    {}\nAction:  {action}\nConfig:  {}",
        resolved.why,
        path.display()
    );
    Ok(())
}

/// Display-only shell quoting for --test; the real launch passes arguments
/// directly, without a shell.
fn quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=?&%+,@".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

fn register() -> Result<(), String> {
    let (id, _) = mac::own_bundle().ok_or(
        "Run this from inside \"URL Router.app\": macOS needs the app bundle, not the bare binary.",
    )?;
    mac::set_default_browser(&id)?;
    notify(
        "URL Router",
        "macOS will ask you to confirm: choose \"Use URL Router\".\n\n\
         You can also change it in System Settings → Desktop & Dock → \
         Default web browser.",
    );
    Ok(())
}

fn edit_config() -> Result<(), String> {
    let path = config_path()?;
    // -t: the default text editor, whatever .txt files normally open in.
    let status = Command::new("/usr/bin/open")
        .arg("-t")
        .arg(&path)
        .status()
        .map_err(|e| format!("Could not run /usr/bin/open: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Could not open {}", path.display()))
    }
}

fn help_text() -> &'static str {
    "urlrouter <url>              open the URL using the rules\n\
     urlrouter --test <url>       show which rule matches and what would run, without opening it\n\
     urlrouter --register         ask macOS to make URL Router the default browser\n\
     urlrouter --edit             open urlrouter.txt in your text editor\n\
     urlrouter --launch-default   open the * browser with no URL\n\
     \n\
     Opened by macOS (or double-clicked), URL Router routes the links it is\n\
     given, or shows a menu if there are none."
}
