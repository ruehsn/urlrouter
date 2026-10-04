//! Rewrites Slack web links into `slack://` deep links, so a rule can send
//! them straight to the Slack desktop app instead of a browser tab that then
//! asks "Open in Slack?".
//!
//! Deep links need the workspace's team ID (`T…`/`E…`). `app.slack.com` URLs
//! carry it in the path; `<workspace>.slack.com` URLs don't, so those need a
//! `slack-team <workspace> <ID>` line in the config. A URL we can't convert
//! returns None and the router falls through to the next matching rule, so an
//! unknown link still opens in a browser rather than being dropped.

use std::collections::HashMap;

/// Subdomains of slack.com that are Slack's own services, not workspaces.
/// Their pages are web-only (docs, status, sign-in), so they stay in the browser.
const NOT_WORKSPACES: &[&str] = &[
    "www",
    "api",
    "status",
    "slackhq",
    "hooks",
    "edgeapi",
    "downloads",
    "a",
    "ca",
    "my",
    "get",
];

/// `id` must be uppercase alphanumeric starting with one of `prefixes`. Every
/// ID is interpolated into the deep link's query string, so this is also what
/// keeps a crafted URL from smuggling extra `&param=` values into it.
fn is_id(id: &str, prefixes: &str) -> bool {
    id.len() >= 2
        && id.starts_with(|c| prefixes.contains(c))
        && id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

pub fn is_team_id(id: &str) -> bool {
    is_id(id, "TE")
}

/// Slack message timestamps look like `1712345678.123456`.
fn is_ts(ts: &str) -> bool {
    match ts.split_once('.') {
        Some((a, b)) => {
            !a.is_empty()
                && !b.is_empty()
                && a.bytes().all(|c| c.is_ascii_digit())
                && b.bytes().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Permalinks encode the timestamp as `p` + digits with the dot removed:
/// `p1712345678123456` is message `1712345678.123456`.
fn ts_from_permalink(p: &str) -> Option<String> {
    let digits = p.strip_prefix('p')?;
    if digits.len() <= 6 || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (secs, micros) = digits.split_at(digits.len() - 6);
    Some(format!("{secs}.{micros}"))
}

fn channel(team: &str, id: &str, message: Option<String>) -> String {
    match message {
        Some(ts) => format!("slack://channel?team={team}&id={id}&message={ts}"),
        None => format!("slack://channel?team={team}&id={id}"),
    }
}

/// Converts a Slack web URL to a `slack://` deep link, or None if it isn't one
/// we can map. `teams` maps lowercased workspace subdomains (`acme`,
/// `acme.enterprise`) to team IDs.
pub fn deep_link(url: &str, teams: &HashMap<String, String>) -> Option<String> {
    let scheme = url.split("://").next()?.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let (host, rest) = crate::rules::split_host(url)?;
    let sub = host.strip_suffix(".slack.com")?;
    let path = rest.split(['?', '#']).next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    match sub {
        // app.slack.com/client/<team>/<channel>[/<ts> | /thread/<channel>-<ts>]
        //                              [/user_profile/<user>]
        "app" => {
            if segs.first() != Some(&"client") {
                return None;
            }
            let team = *segs.get(1)?;
            if !is_team_id(team) {
                return None;
            }
            match segs.get(2..).unwrap_or(&[]) {
                [_, "user_profile", user, ..] | ["user_profile", user, ..] if is_id(user, "UW") => {
                    Some(format!("slack://user?team={team}&id={user}"))
                }
                [ch, "thread", thread, ..] if is_id(ch, "CGD") => {
                    let ts = thread
                        .split_once('-')
                        .map(|(_, ts)| ts)
                        .filter(|ts| is_ts(ts));
                    Some(channel(team, ch, ts.map(str::to_string)))
                }
                [ch, ts, ..] if is_id(ch, "CGD") && is_ts(ts) => {
                    Some(channel(team, ch, Some(ts.to_string())))
                }
                [ch, ..] if is_id(ch, "CGD") => Some(channel(team, ch, None)),
                // Any other page of the web client (activity, search, ...) is
                // still "this workspace", so open the workspace in the app.
                _ => Some(format!("slack://open?team={team}")),
            }
        }
        // files.slack.com/files-pri/<team>-<file>/<name>
        "files" => {
            let (team, file) = segs.get(1)?.split_once('-')?;
            (matches!(segs.first(), Some(&("files-pri" | "files-tmb")))
                && is_team_id(team)
                && is_id(file, "F"))
            .then(|| format!("slack://file?team={team}&id={file}"))
        }
        _ if NOT_WORKSPACES.contains(&sub) => None,
        workspace => {
            let team = teams.get(workspace)?;
            match segs.as_slice() {
                [] => Some(format!("slack://open?team={team}")),
                ["archives" | "messages", ch] if is_id(ch, "CGD") => Some(channel(team, ch, None)),
                ["archives", ch, p] if is_id(ch, "CGD") => {
                    ts_from_permalink(p).map(|ts| channel(team, ch, Some(ts)))
                }
                ["team", user] if is_id(user, "UW") => {
                    Some(format!("slack://user?team={team}&id={user}"))
                }
                ["files", _, file, ..] if is_id(file, "F") => {
                    Some(format!("slack://file?team={team}&id={file}"))
                }
                // Sign-in, admin, customize, apps, ...: web-only pages.
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn teams() -> HashMap<String, String> {
        HashMap::from([
            ("acme".to_string(), "T0ACME".to_string()),
            ("bigco.enterprise".to_string(), "E0BIGCO".to_string()),
        ])
    }

    fn link(url: &str) -> Option<String> {
        deep_link(url, &teams())
    }

    #[test]
    fn workspace_channel_and_message_permalinks() {
        assert_eq!(
            link("https://acme.slack.com/archives/C0123ABC").as_deref(),
            Some("slack://channel?team=T0ACME&id=C0123ABC")
        );
        assert_eq!(
            link("https://acme.slack.com/archives/C0123ABC/p1712345678123456").as_deref(),
            Some("slack://channel?team=T0ACME&id=C0123ABC&message=1712345678.123456")
        );
        // Thread replies carry extra query parameters; the message is the reply.
        assert_eq!(
            link("https://ACME.slack.com/archives/C0123ABC/p1712345678123456?thread_ts=1712345600.000100&cid=C0123ABC")
                .as_deref(),
            Some("slack://channel?team=T0ACME&id=C0123ABC&message=1712345678.123456")
        );
        assert_eq!(
            link("https://acme.slack.com/messages/G0PRIV").as_deref(),
            Some("slack://channel?team=T0ACME&id=G0PRIV")
        );
        assert_eq!(
            link("https://bigco.enterprise.slack.com/archives/D0DM").as_deref(),
            Some("slack://channel?team=E0BIGCO&id=D0DM")
        );
    }

    #[test]
    fn workspace_users_files_and_root() {
        assert_eq!(
            link("https://acme.slack.com/").as_deref(),
            Some("slack://open?team=T0ACME")
        );
        assert_eq!(
            link("https://acme.slack.com/team/U0USER").as_deref(),
            Some("slack://user?team=T0ACME&id=U0USER")
        );
        assert_eq!(
            link("https://acme.slack.com/files/U0USER/F0FILE/report.pdf").as_deref(),
            Some("slack://file?team=T0ACME&id=F0FILE")
        );
    }

    #[test]
    fn app_slack_com_carries_its_own_team_id() {
        let none = HashMap::new();
        let l = |u: &str| deep_link(u, &none);
        assert_eq!(
            l("https://app.slack.com/client/T0XYZ/C0CHAN").as_deref(),
            Some("slack://channel?team=T0XYZ&id=C0CHAN")
        );
        assert_eq!(
            l("https://app.slack.com/client/T0XYZ/C0CHAN/1712345678.123456").as_deref(),
            Some("slack://channel?team=T0XYZ&id=C0CHAN&message=1712345678.123456")
        );
        assert_eq!(
            l("https://app.slack.com/client/T0XYZ/C0CHAN/thread/C0CHAN-1712345678.123456")
                .as_deref(),
            Some("slack://channel?team=T0XYZ&id=C0CHAN&message=1712345678.123456")
        );
        assert_eq!(
            l("https://app.slack.com/client/T0XYZ/C0CHAN/user_profile/U0ME").as_deref(),
            Some("slack://user?team=T0XYZ&id=U0ME")
        );
        assert_eq!(
            l("https://app.slack.com/client/T0XYZ/activity-page").as_deref(),
            Some("slack://open?team=T0XYZ")
        );
        assert_eq!(
            l("https://app.slack.com/client/T0XYZ").as_deref(),
            Some("slack://open?team=T0XYZ")
        );
        assert_eq!(
            l("https://files.slack.com/files-pri/T0XYZ-F0FILE/image.png").as_deref(),
            Some("slack://file?team=T0XYZ&id=F0FILE")
        );
    }

    #[test]
    fn web_only_and_unknown_links_are_not_converted() {
        for url in [
            "https://slack.com/",
            "https://slack.com/help/articles/123",
            "https://api.slack.com/methods",
            "https://status.slack.com/",
            "https://app.slack.com/plans",
            "https://acme.slack.com/signin",
            "https://acme.slack.com/admin",
            "https://acme.slack.com/archives/C0123/notapermalink/extra",
            "https://unknown.slack.com/archives/C0123ABC", // no slack-team line
            "https://acme.slack.com.evil.example/archives/C0123ABC",
            "https://notslack.com/archives/C0123ABC",
            "slack://channel?team=T0ACME&id=C0123ABC",
            "ftp://acme.slack.com/",
        ] {
            assert_eq!(link(url), None, "{url}");
        }
    }

    #[test]
    fn ids_cannot_inject_query_parameters() {
        assert_eq!(
            link("https://acme.slack.com/archives/C01&team=T0EVIL"),
            None
        );
        assert_eq!(
            deep_link(
                "https://app.slack.com/client/T0X&id=C1/C0CHAN",
                &HashMap::new()
            ),
            None
        );
        assert_eq!(link("https://acme.slack.com/archives/c0lowercase"), None);
        assert_eq!(
            link("https://acme.slack.com/archives/C0123/p12345x"),
            None,
            "a malformed permalink is not a channel link either"
        );
    }

    #[test]
    fn permalink_timestamps() {
        assert_eq!(
            ts_from_permalink("p1712345678123456").as_deref(),
            Some("1712345678.123456")
        );
        assert_eq!(ts_from_permalink("p123456"), None);
        assert_eq!(ts_from_permalink("1712345678123456"), None);
    }
}
