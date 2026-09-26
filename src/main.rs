//! URL Router: registers as a Windows "browser", and when a link is opened
//! hands it to the real browser picked by the rules in urlrouter.txt.
//!
//! The GUI subsystem means no console window flashes on every click; all
//! feedback goes through message boxes.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg_attr(not(windows), allow(dead_code))]
mod rules;
#[cfg(windows)]
mod win;

pub const DEFAULT_CONFIG: &str = include_str!("default_config.txt");

#[cfg(not(windows))]
fn main() {
    eprintln!("urlrouter only runs on Windows. Build it with --target x86_64-pc-windows-gnu (see README).");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    std::process::exit(app::run());
}

#[cfg(windows)]
mod app {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use crate::rules::{self, Config};
    use crate::win::{self, *};

    const CONFIG_NAME: &str = "urlrouter.txt";
    /// Used when no rule matches. Edge ships with Windows 10/11, so a config
    /// without a `*` line still opens links somewhere instead of dropping them.
    const LAST_RESORT: &str = "edge";

    pub fn run() -> i32 {
        let args: Vec<String> = env::args_os()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let first = args.first().map(|s| s.to_ascii_lowercase());
        let result = match first.as_deref() {
            None => interactive(),
            Some("--register" | "/register") => register(),
            Some("--unregister" | "/unregister") => unregister(),
            Some("--edit" | "/edit") => edit_config(),
            Some("--launch-default") => launch(None),
            Some("--test" | "/test") => match args.get(1) {
                Some(url) => test(url),
                None => Err(
                    "--test needs a URL, e.g.  urlrouter.exe --test https://youtube.com/".into(),
                ),
            },
            Some("--help" | "-h" | "/?" | "/help") => {
                message_box(&help_text(), MB_OK | MB_ICONINFORMATION);
                Ok(())
            }
            Some(_) => args
                .iter()
                .filter(|a| !a.trim().is_empty())
                .try_for_each(|url| launch(Some(url.trim()))),
        };
        match result {
            Ok(()) => 0,
            Err(e) => {
                message_box(
                    &format!("{e}\n\n(Ctrl+C copies this message.)"),
                    MB_OK | MB_ICONERROR,
                );
                1
            }
        }
    }

    fn exe_path() -> Result<PathBuf, String> {
        env::current_exe().map_err(|e| format!("Could not determine where urlrouter.exe is: {e}"))
    }

    /// The config lives next to the .exe so the whole thing stays portable
    /// (copy the folder, done). If the .exe sits somewhere read-only such as
    /// Program Files, %APPDATA%\URLRouter is used instead.
    fn config_path() -> Result<PathBuf, String> {
        let exe = exe_path()?;
        let local = exe.parent().map(|d| d.join(CONFIG_NAME));
        let roaming =
            env::var_os("APPDATA").map(|d| PathBuf::from(d).join("URLRouter").join(CONFIG_NAME));
        for p in local.iter().chain(roaming.iter()) {
            if p.is_file() {
                return Ok(p.clone());
            }
        }
        // First run: write the commented default config.
        let contents = crate::DEFAULT_CONFIG
            .replace("\r\n", "\n")
            .replace('\n', "\r\n");
        let mut last_err = String::from("no candidate location");
        for p in local.iter().chain(roaming.iter()) {
            if let Some(dir) = p.parent() {
                let _ = fs::create_dir_all(dir);
            }
            match fs::write(p, &contents) {
                Ok(()) => return Ok(p.clone()),
                Err(e) => last_err = format!("{}: {e}", p.display()),
            }
        }
        Err(format!("Could not create {CONFIG_NAME} ({last_err})."))
    }

    fn load_config() -> Result<(Config, PathBuf), String> {
        let path = config_path()?;
        let bytes =
            fs::read(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        Ok((Config::parse(&rules::decode_text(&bytes)), path))
    }

    fn warn_config_errors(cfg: &Config, path: &Path) {
        if !cfg.errors.is_empty() {
            message_box(
                &format!(
                    "Some lines in {} were ignored:\n\n{}",
                    path.display(),
                    cfg.errors.join("\n")
                ),
                MB_OK | MB_ICONWARNING,
            );
        }
    }

    /// Resolves what would be run for `url` (None = open the browser bare).
    /// Returns (description of the rule used, program, args).
    fn resolve(cfg: &Config, url: Option<&str>) -> Result<(String, String, Vec<String>), String> {
        let rule = match url {
            Some(u) => cfg.find(u),
            None => cfg.rules.iter().find(|r| r.pattern == rules::Pattern::Any),
        };
        let (target, why) = match rule {
            Some(r) => (
                r.target.as_str(),
                format!("line {}: {}  {}", r.line, r.pattern_text, r.target),
            ),
            None => (LAST_RESORT, format!("no rule matched, using {LAST_RESORT}")),
        };
        let (program, args) = rules::build_command(
            target,
            url,
            &cfg.aliases,
            &win::locate_builtin,
            &win::env_var,
        )
        .map_err(|e| format!("{e}\n\nRule used: {why}"))?;
        refuse_self(&program)?;
        Ok((why, program, args))
    }

    /// Pointing a rule at urlrouter.exe itself would spawn copies forever.
    fn refuse_self(program: &str) -> Result<(), String> {
        let me = exe_path()?;
        let same = match (fs::canonicalize(program), fs::canonicalize(&me)) {
            (Ok(a), Ok(b)) => a == b,
            _ => Path::new(program)
                .file_name()
                .is_some_and(|n| Some(n) == me.file_name()),
        };
        if same {
            return Err(format!(
                "A rule points at URL Router itself ({program}); that would loop forever."
            ));
        }
        Ok(())
    }

    fn launch(url: Option<&str>) -> Result<(), String> {
        if let Some(u) = url {
            // Links always start with a scheme and file paths never start with
            // '-'. Refusing these keeps a crafted "link" from smuggling
            // command-line switches into the browser.
            if u.starts_with('-') {
                return Err(format!(
                    "Refusing to open something that looks like a command-line switch:\n{u}"
                ));
            }
        }
        let (cfg, path) = load_config()?;
        warn_config_errors(&cfg, &path);
        let (_, program, args) = resolve(&cfg, url)?;
        win::allow_child_foreground();
        Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| {
                format!(
                    "Could not start {program}: {e}\n\nURL: {}\nConfig: {}",
                    url.unwrap_or("(none)"),
                    path.display()
                )
            })
    }

    fn test(url: &str) -> Result<(), String> {
        let (cfg, path) = load_config()?;
        warn_config_errors(&cfg, &path);
        let (why, program, args) = resolve(&cfg, Some(url))?;
        let mut cmd = quote(&program);
        for a in &args {
            cmd.push(' ');
            cmd.push_str(&quote(a));
        }
        message_box(
            &format!(
                "URL:\n{url}\n\nRule:\n{why}\n\nWould run:\n{cmd}\n\nConfig: {}",
                path.display()
            ),
            MB_OK | MB_ICONINFORMATION,
        );
        Ok(())
    }

    /// Display-only quoting for the --test dialog; the real launch passes
    /// arguments through std::process::Command, which quotes properly.
    fn quote(s: &str) -> String {
        if s.is_empty() || s.contains(char::is_whitespace) {
            format!("\"{s}\"")
        } else {
            s.to_string()
        }
    }

    fn register() -> Result<(), String> {
        win::register(&exe_path()?)?;
        let path = config_path()?;
        message_box(
            &format!(
                "URL Router is registered as a browser for your account.\n\n\
                 Windows only lets you choose the default browser yourself, so \
                 Settings will open next: pick \"URL Router\" as the web browser \
                 (on Windows 11, press \"Set default\" at the top of its page).\n\n\
                 Rules: {}\n\n\
                 If you move urlrouter.exe, run it again and choose Register.",
                path.display()
            ),
            MB_OK | MB_ICONINFORMATION,
        );
        // registeredAppUser jumps straight to our page on Windows 11; Windows
        // 10 ignores the parameter and shows the general Default apps page.
        win::shell_open(&format!(
            "ms-settings:defaultapps?registeredAppUser={}",
            win::APP_ID
        ))
        .or_else(|_| win::shell_open("ms-settings:defaultapps"))
    }

    fn unregister() -> Result<(), String> {
        win::unregister()?;
        message_box(
            "URL Router has been removed from the list of browsers.\n\n\
             If it was your default, Windows will ask you to pick another \
             browser the next time you open a link.",
            MB_OK | MB_ICONINFORMATION,
        );
        Ok(())
    }

    fn edit_config() -> Result<(), String> {
        let path = config_path()?;
        win::shell_open(&path.display().to_string())
    }

    fn interactive() -> Result<(), String> {
        let path = config_path()?;
        let choice = message_box(
            &format!(
                "URL Router opens each link in the browser chosen by your rules.\n\n\
                 Rules file:\n{}\n\n\
                 Yes  –  register as a browser and open Default apps settings\n\
                 No  –  edit the rules\n\
                 Cancel  –  close\n\n\
                 Run with --help for command-line options.",
                path.display()
            ),
            MB_YESNOCANCEL | MB_ICONQUESTION,
        );
        match choice {
            IDYES => register(),
            IDNO => edit_config(),
            _ => Ok(()),
        }
    }

    fn help_text() -> String {
        "urlrouter.exe <url>            open the URL in the browser its rule picks\n\
         urlrouter.exe --test <url>     show which rule and command would be used\n\
         urlrouter.exe --register       register as a browser (current user, no admin)\n\
         urlrouter.exe --unregister     remove that registration\n\
         urlrouter.exe --edit           open urlrouter.txt\n\
         urlrouter.exe --launch-default open the catch-all (*) browser without a URL\n\
         urlrouter.exe                  menu: register / edit rules"
            .into()
    }
}
