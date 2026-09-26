//! Config parsing, URL matching and command-line building.
//!
//! Everything here is pure logic with no Win32 calls, so `cargo test` runs it
//! on any platform. The Windows-only pieces (registry, message boxes, locating
//! installed browsers) live in `win.rs` and are injected as closures.

use std::collections::HashMap;

/// What the left-hand side of a rule line matches.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `*` — matches everything. Conventionally the last line (first match wins).
    Any,
    /// Contains `://` — a literal, case-insensitive prefix of the whole URL.
    /// `https://github.com/work-org` matches only that scheme and path.
    Prefix(String),
    /// Bare `host[/path]` — that host *or any subdomain of it*, on any scheme,
    /// optionally restricted to URLs whose path starts with `path`.
    /// `youtube.com` therefore catches `www.`, `m.` and `music.youtube.com`
    /// without also catching `notyoutube.com`.
    Host { host: String, path: String },
}

impl Pattern {
    pub fn parse(text: &str) -> Result<Pattern, String> {
        if text == "*" {
            return Ok(Pattern::Any);
        }
        if text.contains("://") {
            return Ok(Pattern::Prefix(text.to_lowercase()));
        }
        // `*.example.com` is accepted as a courtesy; host patterns already
        // cover subdomains, so the wildcard adds nothing.
        let t = text.strip_prefix("*.").unwrap_or(text);
        let (host, path) = match t.find('/') {
            Some(i) => (&t[..i], &t[i..]),
            None => (t, ""),
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            return Err(format!("`{text}` has no host name"));
        }
        Ok(Pattern::Host {
            host,
            path: path.to_lowercase(),
        })
    }

    pub fn matches(&self, url: &str) -> bool {
        match self {
            Pattern::Any => true,
            Pattern::Prefix(p) => url.to_lowercase().starts_with(p.as_str()),
            Pattern::Host { host, path } => {
                let Some((url_host, rest)) = split_host(url) else {
                    return false;
                };
                // Suffix match only on a label boundary, so `youtube.com`
                // never matches `evilyoutube.com`.
                let host_ok = url_host == *host
                    || (url_host.len() > host.len()
                        && url_host.ends_with(host.as_str())
                        && url_host.as_bytes()[url_host.len() - host.len() - 1] == b'.');
                host_ok && (path.is_empty() || rest.to_lowercase().starts_with(path.as_str()))
            }
        }
    }
}

/// Splits `scheme://[user@]host[:port]/rest` into (lowercased host, "/rest").
/// Returns None for things that aren't URLs with an authority, such as local
/// file paths handed over by an .html file association.
fn split_host(url: &str) -> Option<(String, &str)> {
    let i = url.find("://")?;
    let scheme = &url[..i];
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return None;
    }
    let after = &url[i + 3..];
    let end = after.find(['/', '?', '#', '\\']).unwrap_or(after.len());
    let mut auth = &after[..end];
    // Drop userinfo: in `https://youtube.com@evil.example/` the host is
    // evil.example, and that is what the browser will actually load.
    if let Some(at) = auth.rfind('@') {
        auth = &auth[at + 1..];
    }
    let host = if auth.starts_with('[') {
        // IPv6 literal: the port colon comes after the closing bracket.
        auth.find(']').map_or(auth, |e| &auth[..=e])
    } else {
        auth.split(':').next().unwrap_or("")
    };
    Some((
        host.trim_end_matches('.').to_ascii_lowercase(),
        &after[end..],
    ))
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub pattern: Pattern,
    pub pattern_text: String,
    pub target: String,
    pub line: usize,
}

#[derive(Debug, Default)]
pub struct Config {
    pub rules: Vec<Rule>,
    /// `browser <name> <command>` lines, keyed by lowercased name.
    pub aliases: HashMap<String, String>,
    /// Problems found while parsing, already prefixed with the line number.
    /// Bad lines are skipped rather than fatal so one typo doesn't stop every
    /// link from opening.
    pub errors: Vec<String>,
}

impl Config {
    pub fn parse(text: &str) -> Config {
        let mut cfg = Config::default();
        for (i, raw) in text.lines().enumerate() {
            let line_no = i + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let (first, rest) = split_first_word(line);
            match first.to_ascii_lowercase().as_str() {
                "browser" => {
                    let (name, cmd) = split_first_word(rest);
                    if name.is_empty() || cmd.is_empty() {
                        cfg.errors.push(format!(
                            "line {line_no}: expected `browser <name> <program> [arguments]`"
                        ));
                    } else {
                        cfg.aliases.insert(name.to_lowercase(), cmd.to_string());
                    }
                }
                "default" => cfg.push_rule(Pattern::Any, "*", rest, line_no),
                _ => match Pattern::parse(first) {
                    Ok(p) => cfg.push_rule(p, first, rest, line_no),
                    Err(e) => cfg.errors.push(format!("line {line_no}: {e}")),
                },
            }
        }
        cfg
    }

    fn push_rule(&mut self, pattern: Pattern, text: &str, target: &str, line: usize) {
        if target.is_empty() {
            self.errors
                .push(format!("line {line}: `{text}` needs a browser after it"));
            return;
        }
        self.rules.push(Rule {
            pattern,
            pattern_text: text.to_string(),
            target: target.to_string(),
            line,
        });
    }

    /// First matching rule wins, top to bottom.
    pub fn find(&self, url: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.pattern.matches(url))
    }
}

fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim()),
        None => (s, ""),
    }
}

/// Decodes the config file bytes. Notepad may save UTF-8 with a BOM or, if
/// someone picks "UTF-16 LE", UTF-16 — both are handled rather than producing
/// a confusing "no rules" failure.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// A browser the config can name without a path.
pub struct Builtin {
    pub name: &'static str,
    /// Looked up under the registry's `App Paths` key first.
    pub exe: &'static str,
    /// Fallback install locations, with %VARS% expanded at runtime.
    pub paths: &'static [&'static str],
}

pub const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "chrome",
        exe: "chrome.exe",
        paths: &[
            r"%ProgramFiles%\Google\Chrome\Application\chrome.exe",
            r"%ProgramFiles(x86)%\Google\Chrome\Application\chrome.exe",
            r"%LOCALAPPDATA%\Google\Chrome\Application\chrome.exe",
        ],
    },
    Builtin {
        name: "firefox",
        exe: "firefox.exe",
        paths: &[
            r"%ProgramFiles%\Mozilla Firefox\firefox.exe",
            r"%ProgramFiles(x86)%\Mozilla Firefox\firefox.exe",
            r"%LOCALAPPDATA%\Mozilla Firefox\firefox.exe",
        ],
    },
    Builtin {
        name: "edge",
        exe: "msedge.exe",
        paths: &[
            r"%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe",
            r"%ProgramFiles%\Microsoft\Edge\Application\msedge.exe",
        ],
    },
    Builtin {
        name: "brave",
        exe: "brave.exe",
        paths: &[
            r"%ProgramFiles%\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"%ProgramFiles(x86)%\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"%LOCALAPPDATA%\BraveSoftware\Brave-Browser\Application\brave.exe",
        ],
    },
    Builtin {
        name: "vivaldi",
        exe: "vivaldi.exe",
        paths: &[
            r"%LOCALAPPDATA%\Vivaldi\Application\vivaldi.exe",
            r"%ProgramFiles%\Vivaldi\Application\vivaldi.exe",
        ],
    },
    Builtin {
        name: "opera",
        exe: "opera.exe",
        paths: &[
            r"%LOCALAPPDATA%\Programs\Opera\opera.exe",
            r"%ProgramFiles%\Opera\opera.exe",
        ],
    },
    Builtin {
        name: "librewolf",
        exe: "librewolf.exe",
        paths: &[r"%ProgramFiles%\LibreWolf\librewolf.exe"],
    },
];

pub fn builtin(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name.eq_ignore_ascii_case(name))
}

/// Splits a command line on whitespace, with double quotes grouping (and being
/// removed), which is how people naturally write Windows paths with spaces.
pub fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut have_token = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                have_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if have_token {
                    out.push(std::mem::take(&mut cur));
                    have_token = false;
                }
            }
            c => {
                cur.push(c);
                have_token = true;
            }
        }
    }
    if have_token {
        out.push(cur);
    }
    out
}

/// Expands `%NAME%` environment variables. Unknown names and the `%URL%`
/// placeholder are left untouched.
pub fn expand_env(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('%') {
            let name = &after[..end];
            if !name.is_empty() && !name.eq_ignore_ascii_case("URL") {
                if let Some(v) = lookup(name) {
                    out.push_str(&v);
                    rest = &after[end + 1..];
                    continue;
                }
            }
        }
        // Not a variable we expand: keep the '%' and rescan from the next
        // character, since the closing '%' may open the next variable.
        out.push('%');
        rest = after;
    }
    out.push_str(rest);
    out
}

fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

/// Turns a rule's target (`firefox`, `work -private-window`,
/// `"C:\Apps\x.exe" --flag %URL%`) into a program path and argument list.
///
/// The URL is substituted *after* environment expansion so percent-encoded
/// URLs (`%41PPDATA%`...) can never be expanded into something else. With
/// `url: None` (opening the browser without a link) any argument carrying the
/// `%URL%` placeholder is dropped.
pub fn build_command(
    target: &str,
    url: Option<&str>,
    aliases: &HashMap<String, String>,
    locate: &dyn Fn(&Builtin) -> Option<String>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<(String, Vec<String>), String> {
    let mut tokens = tokenize(target);
    if tokens.is_empty() {
        return Err("empty browser command".into());
    }
    // One level of alias expansion; aliases may name built-ins but not other
    // aliases, which rules out loops.
    if let Some(cmd) = aliases.get(&tokens[0].to_lowercase()) {
        let mut expanded = tokenize(cmd);
        if expanded.is_empty() {
            return Err(format!("browser `{}` has an empty command", tokens[0]));
        }
        expanded.extend(tokens.drain(1..));
        tokens = expanded;
    }

    let program = match builtin(&tokens[0]) {
        Some(b) => locate(b).ok_or_else(|| {
            format!(
                "`{}` was requested but {} could not be found on this PC.",
                b.name, b.exe
            )
        })?,
        None => expand_env(&tokens[0], env),
    };

    let mut args = Vec::new();
    let mut placed = false;
    for raw in &tokens[1..] {
        let arg = expand_env(raw, env);
        match find_ci(&arg, "%URL%") {
            Some(i) => {
                placed = true;
                if let Some(u) = url {
                    args.push(format!("{}{}{}", &arg[..i], u, &arg[i + 5..]));
                }
            }
            None => args.push(arg),
        }
    }
    if let (false, Some(u)) = (placed, url) {
        args.push(u.to_string());
    }
    Ok((program, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# comment
; also a comment

browser work \"C:\\Program Files\\Mozilla Firefox\\firefox.exe\" -P Work
youtube.com          firefox
youtu.be             firefox
https://github.com/acme   work
docs.google.com/spreadsheets  edge
*                    chrome
example.org          brave
";

    fn cfg() -> Config {
        Config::parse(SAMPLE)
    }

    fn target_for(url: &str) -> Option<String> {
        cfg().find(url).map(|r| r.target.clone())
    }

    fn fake_locate(b: &Builtin) -> Option<String> {
        match b.name {
            "opera" => None,
            n => Some(format!("C:\\B\\{n}.exe")),
        }
    }

    fn fake_env(name: &str) -> Option<String> {
        match name.to_ascii_uppercase().as_str() {
            "LOCALAPPDATA" => Some("C:\\Users\\me\\AppData\\Local".into()),
            "APPDATA" => Some("C:\\Users\\me\\AppData\\Roaming".into()),
            _ => None,
        }
    }

    fn build(target: &str, url: Option<&str>) -> Result<(String, Vec<String>), String> {
        build_command(target, url, &cfg().aliases, &fake_locate, &fake_env)
    }

    #[test]
    fn parses_rules_and_aliases() {
        let c = cfg();
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert_eq!(c.rules.len(), 6);
        assert_eq!(c.rules[0].line, 5);
        assert_eq!(c.rules[0].target, "firefox");
        assert!(c.aliases.contains_key("work"));
    }

    #[test]
    fn host_patterns_cover_scheme_www_and_subdomains() {
        for url in [
            "https://www.youtube.com/watch?v=abc",
            "http://youtube.com",
            "https://m.youtube.com/",
            "https://music.youtube.com/x",
            "HTTPS://WWW.YOUTUBE.COM/",
            "https://youtube.com:443/",
            "https://youtu.be/abc",
            "https://www.youtube.com./",
        ] {
            assert_eq!(target_for(url).as_deref(), Some("firefox"), "{url}");
        }
    }

    #[test]
    fn host_patterns_respect_label_boundaries_and_userinfo() {
        for url in [
            "https://notyoutube.com/",
            "https://youtube.com.evil.example/",
            "https://youtube.com@evil.example/",
            "https://evil.example/?u=https://youtube.com/",
            "https://evil.example/#youtube.com",
        ] {
            assert_eq!(target_for(url).as_deref(), Some("chrome"), "{url}");
        }
    }

    #[test]
    fn host_pattern_with_path() {
        assert_eq!(
            target_for("https://docs.google.com/spreadsheets/d/1").as_deref(),
            Some("edge")
        );
        assert_eq!(
            target_for("https://docs.google.com/document/d/1").as_deref(),
            Some("chrome")
        );
    }

    #[test]
    fn full_prefix_patterns() {
        assert_eq!(
            target_for("https://github.com/acme/repo").as_deref(),
            Some("work")
        );
        assert_eq!(
            target_for("https://GitHub.com/Acme").as_deref(),
            Some("work")
        );
        assert_eq!(
            target_for("http://github.com/acme").as_deref(),
            Some("chrome")
        );
        assert_eq!(
            target_for("https://github.com/other").as_deref(),
            Some("chrome")
        );
    }

    #[test]
    fn first_match_wins() {
        // example.org sits below `*`, so it can never be reached.
        assert_eq!(
            target_for("https://example.org/").as_deref(),
            Some("chrome")
        );
    }

    #[test]
    fn non_urls_fall_through_to_catch_all() {
        assert_eq!(
            target_for(r"C:\Users\me\page.html").as_deref(),
            Some("chrome")
        );
        assert!(Config::parse("youtube.com firefox")
            .find(r"C:\x.html")
            .is_none());
    }

    #[test]
    fn ipv6_host() {
        let c = Config::parse("[::1] firefox\n* chrome");
        assert_eq!(c.find("http://[::1]:8080/").unwrap().target, "firefox");
    }

    #[test]
    fn default_keyword_is_catch_all() {
        let c = Config::parse("default edge");
        assert_eq!(c.find("https://anything/").unwrap().target, "edge");
    }

    #[test]
    fn bad_lines_are_reported_and_skipped() {
        let c = Config::parse("youtube.com\n/path firefox\nbrowser x\n* chrome");
        assert_eq!(c.rules.len(), 1);
        assert_eq!(c.errors.len(), 3);
        assert!(c.errors[0].starts_with("line 1:"));
        assert!(c.errors[1].starts_with("line 2:"));
        assert!(c.errors[2].starts_with("line 3:"));
    }

    #[test]
    fn decodes_bom_and_utf16() {
        assert_eq!(decode_text(b"\xEF\xBB\xBF* chrome"), "* chrome");
        let mut utf16 = vec![0xFF, 0xFE];
        for u in "* edge".encode_utf16() {
            utf16.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_text(&utf16), "* edge");
        assert_eq!(Config::parse(&decode_text(&utf16)).rules.len(), 1);
    }

    #[test]
    fn tokenizes_quotes() {
        assert_eq!(
            tokenize(r#""C:\Program Files\x.exe" -a  "b c" d"e"f """#),
            vec![r"C:\Program Files\x.exe", "-a", "b c", "def", ""]
        );
        assert!(tokenize("   ").is_empty());
    }

    #[test]
    fn builtin_names_resolve_and_url_is_appended() {
        let (p, a) = build("firefox", Some("https://youtube.com/")).unwrap();
        assert_eq!(p, r"C:\B\firefox.exe");
        assert_eq!(a, vec!["https://youtube.com/"]);
        let (p, _) = build("Chrome", Some("u")).unwrap();
        assert_eq!(p, r"C:\B\chrome.exe");
    }

    #[test]
    fn aliases_expand_and_keep_extra_args() {
        let (p, a) = build("work -new-tab", Some("https://x/")).unwrap();
        assert_eq!(p, r"C:\Program Files\Mozilla Firefox\firefox.exe");
        assert_eq!(a, vec!["-P", "Work", "-new-tab", "https://x/"]);
    }

    #[test]
    fn url_placeholder_is_substituted_not_appended() {
        let (_, a) = build("chrome --app=%url% --x", Some("https://x/")).unwrap();
        assert_eq!(a, vec!["--app=https://x/", "--x"]);
        // Without a URL the placeholder argument is dropped entirely.
        let (_, a) = build("chrome --app=%URL% --x", None).unwrap();
        assert_eq!(a, vec!["--x"]);
        let (_, a) = build("chrome --x", None).unwrap();
        assert_eq!(a, vec!["--x"]);
    }

    #[test]
    fn env_vars_expand_in_paths_but_never_inside_the_url() {
        let (p, a) = build(
            r#""%LOCALAPPDATA%\Thing\t.exe" --dir=%APPDATA%\p"#,
            Some("https://x/%APPDATA%/%41"),
        )
        .unwrap();
        assert_eq!(p, r"C:\Users\me\AppData\Local\Thing\t.exe");
        assert_eq!(
            a,
            vec![
                r"--dir=C:\Users\me\AppData\Roaming\p",
                "https://x/%APPDATA%/%41"
            ]
        );
    }

    #[test]
    fn expand_env_edge_cases() {
        assert_eq!(expand_env("100% %NOPE% %%", &fake_env), "100% %NOPE% %%");
        assert_eq!(expand_env("%URL%", &fake_env), "%URL%");
        assert_eq!(
            expand_env("a%APPDATA%b", &fake_env),
            r"aC:\Users\me\AppData\Roamingb"
        );
        assert_eq!(
            expand_env("%x%APPDATA%", &fake_env),
            r"%xC:\Users\me\AppData\Roaming"
        );
    }

    #[test]
    fn missing_builtin_is_a_clear_error() {
        let err = build("opera", Some("u")).unwrap_err();
        assert!(err.contains("opera.exe"), "{err}");
    }

    #[test]
    fn default_config_parses_cleanly() {
        let c = Config::parse(crate::DEFAULT_CONFIG);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert_eq!(
            c.find("https://www.youtube.com/watch").unwrap().target,
            "firefox"
        );
        assert_eq!(c.find("https://example.com/").unwrap().target, "chrome");
    }
}
