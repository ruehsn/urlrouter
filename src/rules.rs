//! Config parsing, URL matching and command-line building.
//!
//! Everything here is pure logic with no OS calls, so `cargo test` runs it on
//! any platform. The platform pieces (registry or LaunchServices, dialogs,
//! locating installed browsers) live in `win.rs` / `mac.rs` and are injected
//! as closures.

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
pub(crate) fn split_host(url: &str) -> Option<(String, &str)> {
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

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub pattern: Pattern,
    pub pattern_text: String,
    pub target: String,
    pub line: usize,
}

impl Rule {
    /// A bare `slack` target asks for the link to be opened in the Slack app.
    /// A user-defined `browser slack ...` alias takes precedence, so this is
    /// checked against the aliases too.
    fn wants_slack_app(&self, aliases: &HashMap<String, String>) -> bool {
        self.target.trim().eq_ignore_ascii_case("slack") && !aliases.contains_key("slack")
    }
}

/// What to do with a URL once the rules have been applied.
#[derive(Debug, PartialEq)]
pub enum Decision<'a> {
    /// Open the URL with the browser described by `rule.target`.
    Browser(&'a Rule),
    /// Open this `slack://` deep link, which the Slack app handles.
    Slack { rule: &'a Rule, link: String },
    /// No rule matched.
    NoMatch,
}

#[derive(Debug, Default)]
pub struct Config {
    pub rules: Vec<Rule>,
    /// `browser <name> <command>` lines, keyed by lowercased name.
    pub aliases: HashMap<String, String>,
    /// `slack-team <workspace> <ID>` lines: lowercased workspace subdomain
    /// (`acme` for acme.slack.com) to team ID.
    pub slack_teams: HashMap<String, String>,
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
                "slack-team" => {
                    let (name, id) = split_first_word(rest);
                    // Accept the workspace written as a full host, too.
                    let name = name.to_ascii_lowercase();
                    let name = name.strip_suffix(".slack.com").unwrap_or(&name);
                    if name.is_empty() || !crate::slack::is_team_id(id) {
                        cfg.errors.push(format!(
                            "line {line_no}: expected `slack-team <workspace> <team ID>`, \
                             e.g. `slack-team acme T0123ABCD`"
                        ));
                    } else {
                        cfg.slack_teams.insert(name.to_string(), id.to_string());
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

    /// First matching rule, top to bottom, without `decide`'s Slack
    /// fall-through. Tests use it to check pattern matching on its own.
    #[cfg(test)]
    pub fn find(&self, url: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.pattern.matches(url))
    }

    /// Like `find`, but a matching `slack` rule only claims the URL if it can
    /// be turned into a Slack deep link. Otherwise matching carries on down
    /// the list, so Slack's web-only pages (and workspaces with no
    /// `slack-team` line) still reach a browser.
    pub fn decide(&self, url: &str) -> Decision<'_> {
        for rule in self.rules.iter().filter(|r| r.pattern.matches(url)) {
            if !rule.wants_slack_app(&self.aliases) {
                return Decision::Browser(rule);
            }
            if let Some(link) = crate::slack::deep_link(url, &self.slack_teams) {
                return Decision::Slack { rule, link };
            }
        }
        Decision::NoMatch
    }

    /// The catch-all rule, used to open "the browser" with no URL. Skips
    /// `slack` rules, which only make sense with a link.
    pub fn catch_all(&self) -> Option<&Rule> {
        self.rules
            .iter()
            .find(|r| r.pattern == Pattern::Any && !r.wants_slack_app(&self.aliases))
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
    /// Windows: looked up under the registry's `App Paths` key first.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub exe: &'static str,
    /// Windows: fallback install locations, with %VARS% expanded at runtime.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub paths: &'static [&'static str],
    /// macOS: bundle identifier, resolved through LaunchServices wherever the
    /// app is installed. Empty if the browser has no Mac version.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub mac: &'static str,
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
        mac: "com.google.Chrome",
    },
    Builtin {
        name: "firefox",
        exe: "firefox.exe",
        paths: &[
            r"%ProgramFiles%\Mozilla Firefox\firefox.exe",
            r"%ProgramFiles(x86)%\Mozilla Firefox\firefox.exe",
            r"%LOCALAPPDATA%\Mozilla Firefox\firefox.exe",
        ],
        mac: "org.mozilla.firefox",
    },
    Builtin {
        name: "edge",
        exe: "msedge.exe",
        paths: &[
            r"%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe",
            r"%ProgramFiles%\Microsoft\Edge\Application\msedge.exe",
        ],
        mac: "com.microsoft.edgemac",
    },
    Builtin {
        name: "brave",
        exe: "brave.exe",
        paths: &[
            r"%ProgramFiles%\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"%ProgramFiles(x86)%\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"%LOCALAPPDATA%\BraveSoftware\Brave-Browser\Application\brave.exe",
        ],
        mac: "com.brave.Browser",
    },
    Builtin {
        name: "vivaldi",
        exe: "vivaldi.exe",
        paths: &[
            r"%LOCALAPPDATA%\Vivaldi\Application\vivaldi.exe",
            r"%ProgramFiles%\Vivaldi\Application\vivaldi.exe",
        ],
        mac: "com.vivaldi.Vivaldi",
    },
    Builtin {
        name: "opera",
        exe: "opera.exe",
        paths: &[
            r"%LOCALAPPDATA%\Programs\Opera\opera.exe",
            r"%ProgramFiles%\Opera\opera.exe",
        ],
        mac: "com.operasoftware.Opera",
    },
    Builtin {
        name: "librewolf",
        exe: "librewolf.exe",
        paths: &[r"%ProgramFiles%\LibreWolf\librewolf.exe"],
        mac: "",
    },
    Builtin {
        name: "safari",
        exe: "",
        paths: &[],
        mac: "com.apple.Safari",
    },
    Builtin {
        name: "arc",
        exe: "",
        paths: &[],
        mac: "company.thebrowser.Browser",
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
                "`{}` was requested but it could not be found on this computer.",
                b.name
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

/// The concrete action for one URL, ready for a platform launcher.
#[derive(Debug, PartialEq)]
pub enum Plan {
    /// Start `program` with `args` (the URL is already among them).
    Run { program: String, args: Vec<String> },
    /// Hand this `slack://` link to the OS, which opens it in the Slack app.
    OpenSlack(String),
}

#[derive(Debug)]
pub struct Resolved {
    /// The rule used, for error messages and `--test`.
    pub why: String,
    pub plan: Plan,
}

/// Applies the rules to `url` (None = open the catch-all browser with no
/// link). `last_resort` is the browser used when nothing matches, so a config
/// without a `*` line still opens links somewhere instead of dropping them.
pub fn resolve(
    cfg: &Config,
    url: Option<&str>,
    last_resort: &str,
    locate: &dyn Fn(&Builtin) -> Option<String>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Resolved, String> {
    let describe = |r: &Rule| format!("line {}: {}  {}", r.line, r.pattern_text, r.target);
    let decision = match url {
        Some(u) => cfg.decide(u),
        None => cfg.catch_all().map_or(Decision::NoMatch, Decision::Browser),
    };
    let (target, why) = match decision {
        Decision::Slack { rule, link } => {
            return Ok(Resolved {
                why: describe(rule),
                plan: Plan::OpenSlack(link),
            })
        }
        Decision::Browser(rule) => (rule.target.as_str(), describe(rule)),
        Decision::NoMatch => (last_resort, format!("no rule matched, using {last_resort}")),
    };
    let (program, args) = build_command(target, url, &cfg.aliases, locate, env)
        .map_err(|e| format!("{e}\n\nRule used: {why}"))?;
    Ok(Resolved {
        why,
        plan: Plan::Run { program, args },
    })
}

/// macOS: the arguments for /usr/bin/open that carry out `plan`.
///
/// With no extra arguments the URL is handed over as an Apple Event, which
/// a running browser opens in a new tab. Extra arguments (profiles and the
/// like) only reach a browser through a fresh launch (`-n ... --args`);
/// Chromium- and Firefox-based browsers then forward the link to the
/// already-running instance themselves.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn mac_open_args(plan: &Plan, url: Option<&str>) -> Vec<String> {
    match plan {
        // LaunchServices sends slack: links to the Slack app.
        Plan::OpenSlack(link) => vec![link.clone()],
        Plan::Run { program, args } => {
            let mut v = vec!["-a".to_string(), program.clone()];
            match url {
                Some(u) if args.len() == 1 && args[0] == u => v.push(u.to_string()),
                _ if args.is_empty() => {}
                _ => {
                    v.insert(0, "-n".into());
                    v.push("--args".into());
                    v.extend(args.iter().cloned());
                }
            }
            v
        }
    }
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
        assert!(err.contains("`opera`"), "{err}");
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

    #[test]
    fn slack_rules_claim_only_links_they_can_convert() {
        let c = Config::parse("slack-team acme T0ACME\nslack.com slack\n* chrome");
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        match c.decide("https://acme.slack.com/archives/C0123") {
            Decision::Slack { rule, link } => {
                assert_eq!(link, "slack://channel?team=T0ACME&id=C0123");
                assert_eq!(rule.line, 2);
            }
            d => panic!("{d:?}"),
        }
        // Web-only pages and workspaces without a slack-team line fall
        // through to the next matching rule.
        for u in [
            "https://acme.slack.com/admin",
            "https://other.slack.com/archives/C0123",
            "https://slack.com/pricing",
        ] {
            assert!(
                matches!(c.decide(u), Decision::Browser(r) if r.target == "chrome"),
                "{u}"
            );
        }
        assert!(matches!(
            c.decide("https://youtube.com/"),
            Decision::Browser(r) if r.target == "chrome"
        ));
        assert_eq!(
            Config::parse("slack.com slack").decide("https://slack.com/"),
            Decision::NoMatch
        );
    }

    #[test]
    fn slack_alias_overrides_the_slack_target() {
        let c = Config::parse("browser slack safari\nslack.com slack");
        assert!(matches!(
            c.decide("https://app.slack.com/client/T1/C1"),
            Decision::Browser(_)
        ));
    }

    #[test]
    fn catch_all_skips_slack_rules() {
        let c = Config::parse("* slack\n* edge");
        assert_eq!(c.catch_all().unwrap().target, "edge");
        assert!(Config::parse("youtube.com firefox").catch_all().is_none());
    }

    #[test]
    fn slack_team_lines_are_validated() {
        let c =
            Config::parse("slack-team ACME.slack.com T0ACME\nslack-team bad t0lower\nslack-team");
        assert_eq!(
            c.slack_teams.get("acme").map(String::as_str),
            Some("T0ACME")
        );
        assert_eq!(c.errors.len(), 2, "{:?}", c.errors);
        assert!(c.errors[0].starts_with("line 2:"));
    }

    #[test]
    fn default_mac_config_parses_cleanly() {
        let c = Config::parse(crate::DEFAULT_CONFIG_MAC);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert!(matches!(
            c.decide("https://app.slack.com/client/T0X/C0Y"),
            Decision::Slack { .. }
        ));
        assert!(matches!(
            c.decide("https://www.youtube.com/watch"),
            Decision::Browser(r) if r.target == "firefox"
        ));
        assert!(matches!(
            c.decide("https://example.com/"),
            Decision::Browser(r) if r.target == "chrome"
        ));
    }

    #[test]
    fn resolve_builds_a_plan_per_decision() {
        let c =
            Config::parse("slack-team acme T0ACME\nslack.com slack\nyoutube.com firefox\n* chrome");
        let r = |u: Option<&str>| resolve(&c, u, "edge", &fake_locate, &fake_env).unwrap();

        let slack = r(Some("https://acme.slack.com/archives/C01"));
        assert_eq!(
            slack.plan,
            Plan::OpenSlack("slack://channel?team=T0ACME&id=C01".into())
        );
        assert_eq!(slack.why, "line 2: slack.com  slack");

        let yt = r(Some("https://youtube.com/x"));
        assert_eq!(
            yt.plan,
            Plan::Run {
                program: r"C:\B\firefox.exe".into(),
                args: vec!["https://youtube.com/x".into()]
            }
        );

        // No URL: the catch-all browser, opened bare.
        assert_eq!(
            r(None).plan,
            Plan::Run {
                program: r"C:\B\chrome.exe".into(),
                args: vec![]
            }
        );

        // Nothing matches: the last-resort browser.
        let none = Config::parse("youtube.com firefox");
        let lr = resolve(&none, Some("https://x/"), "edge", &fake_locate, &fake_env).unwrap();
        assert_eq!(lr.why, "no rule matched, using edge");
        assert!(matches!(lr.plan, Plan::Run { ref program, .. } if program == r"C:\B\edge.exe"));

        // Errors name the rule that was used.
        let err = resolve(
            &Config::parse("* opera"),
            Some("u"),
            "edge",
            &fake_locate,
            &fake_env,
        )
        .unwrap_err();
        assert!(err.contains("Rule used: line 1: *  opera"), "{err}");
    }

    #[test]
    fn mac_open_args_per_plan() {
        let run = |args: &[&str]| Plan::Run {
            program: "/Applications/Firefox.app".into(),
            args: args.iter().map(|a| a.to_string()).collect(),
        };
        let u = "https://x/";
        // Plain link: an Apple Event to the (possibly running) browser.
        assert_eq!(
            mac_open_args(&run(&[u]), Some(u)),
            ["-a", "/Applications/Firefox.app", u]
        );
        // Extra arguments need a fresh launch to reach the browser.
        assert_eq!(
            mac_open_args(&run(&["-P", "Work", u]), Some(u)),
            [
                "-n",
                "-a",
                "/Applications/Firefox.app",
                "--args",
                "-P",
                "Work",
                u
            ]
        );
        assert_eq!(
            mac_open_args(&run(&["--app=https://x/"]), Some(u)),
            [
                "-n",
                "-a",
                "/Applications/Firefox.app",
                "--args",
                "--app=https://x/"
            ]
        );
        // No URL: just open the app.
        assert_eq!(
            mac_open_args(&run(&[]), None),
            ["-a", "/Applications/Firefox.app"]
        );
        assert_eq!(
            mac_open_args(&Plan::OpenSlack("slack://open?team=T1".into()), Some(u)),
            ["slack://open?team=T1"]
        );
    }
}
