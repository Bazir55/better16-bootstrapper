//! Turning a `better16://...` link into the client's command line.
//!
//! Two link shapes are in the wild:
//!
//!     better16://placeId=1818&key=<ticket>[&jobId=<guid>]
//!     better16:1+launchmode:play+gameinfo:<ticket>+placelauncherurl:<url>
//!
//! The first is what the current site emits. The second is the old site's
//! and still turns up in bookmarks. Both are split on `&` and `+`, and each
//! piece is cut at whichever of `=` or `:` comes first.
//!
//! Any web page can open one of these links, so everything that ends up on
//! the client's command line is checked against a narrow character set
//! first. A value with a space or a quote in it could otherwise smuggle in
//! extra arguments.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::config;

#[derive(Debug, PartialEq)]
pub struct Launch {
    /// Where the client asks for a server (PlaceLauncher.ashx).
    pub join_url: String,
    /// One-shot login ticket. The client spends it on Negotiate.ashx and
    /// is then logged in by cookie, which is also how PlaceLauncher knows
    /// who is asking. It must not be put into the join URL too: the server
    /// refuses a ticket the second time it sees it.
    pub ticket: String,
    pub browser_tracker: Option<String>,
}

impl Launch {
    pub fn client_args(&self) -> String {
        let mut args = format!(
            "--play -a {}/Login/Negotiate.ashx -t {} -j {}",
            config::BASE_URL,
            self.ticket,
            self.join_url
        );
        if let Some(id) = &self.browser_tracker {
            args.push_str(" -b ");
            args.push_str(id);
        }
        args
    }
}

/// Splits the part after the scheme into lowercase keys and raw values.
pub fn parse(uri: &str, scheme: &str) -> Option<Vec<(String, String)>> {
    let prefix = uri.get(..scheme.len() + 1)?;
    if !prefix.eq_ignore_ascii_case(&format!("{scheme}:")) {
        return None;
    }
    let rest = uri[scheme.len() + 1..].trim_start_matches('/');
    // Some browsers add a trailing slash of their own.
    let rest = rest.strip_suffix('/').unwrap_or(rest);

    let params = rest
        .split(['&', '+'])
        .filter(|p| !p.is_empty())
        .map(|piece| match piece.find([':', '=']) {
            Some(i) => (piece[..i].to_ascii_lowercase(), piece[i + 1..].to_string()),
            None => (piece.to_ascii_lowercase(), String::new()),
        })
        .collect();
    Some(params)
}

pub fn launch_from(params: &[(String, String)]) -> Result<Launch, &'static str> {
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    };

    let browser_tracker = get("browsertrackerid").filter(|v| is_digits(v)).map(String::from);

    if let Some(place) = get("placeid") {
        if !is_digits(place) {
            return Err("the place id in the link is not a number");
        }
        let ticket = get("key").ok_or("the link has no login ticket")?;
        if !is_token(ticket) {
            return Err("the login ticket in the link has unexpected characters");
        }

        // RequestGameJob asks for one particular server. Plain RequestGame
        // with a job id tacked on would just get any server.
        let join_url = match get("jobid") {
            Some(job) if is_guid(job) => format!(
                "{}/Game/PlaceLauncher.ashx?request=RequestGameJob&placeId={place}&jobId={job}",
                config::BASE_URL
            ),
            Some(_) => return Err("the server id in the link is not valid"),
            None => format!(
                "{}/Game/PlaceLauncher.ashx?request=RequestGame&placeId={place}",
                config::BASE_URL
            ),
        };

        return Ok(Launch { join_url, ticket: String::from(ticket), browser_tracker });
    }

    if get("launchmode") == Some("play") {
        let ticket = get("gameinfo").ok_or("the link has no login ticket")?;
        if !is_token(ticket) {
            return Err("the login ticket in the link has unexpected characters");
        }
        let url = get("placelauncherurl").ok_or("the link doesn't say which game to join")?;
        let join_url = percent_decode(url);
        // The client would run whatever join script that URL hands back,
        // so it has to be our own site.
        if !join_url.starts_with(&format!("{}/", config::BASE_URL)) || !is_url_safe(&join_url) {
            return Err("the link points at a different site");
        }
        return Ok(Launch { join_url, ticket: String::from(ticket), browser_tracker });
    }

    Err("the link doesn't name a game")
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_guid(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// Tickets are base64 or hex, possibly percent-encoded.
fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 4096
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"%-_.~+/=".contains(&b))
}

fn is_url_safe(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_alphanumeric() || b":/?&=%-_.~+".contains(&b))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(uri: &str) -> Result<Launch, &'static str> {
        launch_from(&parse(uri, "better16").expect("scheme should match"))
    }

    #[test]
    fn current_site_link() {
        let l = launch("better16://placeId=1818&key=abc%2Bdef%3D").unwrap();
        assert_eq!(
            l.join_url,
            format!("{}/Game/PlaceLauncher.ashx?request=RequestGame&placeId=1818", config::BASE_URL)
        );
        // Passed through still encoded; the client appends it to a URL.
        assert_eq!(l.ticket, "abc%2Bdef%3D");
    }

    #[test]
    fn link_with_job_and_trailing_slash() {
        let l = launch("Better16://placeId=5&key=t0k&jobId=6f1c2e44-0b7a-4c55-9a7e-1d2f3a4b5c6d/").unwrap();
        assert!(l.join_url.contains("request=RequestGameJob"));
        assert!(l.join_url.ends_with("&jobId=6f1c2e44-0b7a-4c55-9a7e-1d2f3a4b5c6d"));
    }

    #[test]
    fn legacy_link() {
        let url = format!("{}/Game/PlaceLauncher.ashx?request=RequestGame%26placeId=7", config::BASE_URL)
            .replace(':', "%3A")
            .replace('/', "%2F");
        let l = launch(&format!("better16:1+launchmode:play+gameinfo:TICKET+placelauncherurl:{url}")).unwrap();
        assert_eq!(l.ticket, "TICKET");
        assert!(l.join_url.ends_with("request=RequestGame&placeId=7"));
    }

    #[test]
    fn rejects_injection() {
        assert!(launch("better16://placeId=1 -evil&key=x").is_err());
        assert!(launch("better16://placeId=1&key=x\" --evil").is_err());
        assert!(launch("better16://placeId=1&key=x&jobId=../../").is_err());
        assert!(launch("better16:1+launchmode:play+gameinfo:x+placelauncherurl:http%3A%2F%2Fevil.example%2F").is_err());
    }

    #[test]
    fn other_schemes_are_not_ours() {
        assert!(parse("roblox://placeId=1", "better16").is_none());
        assert!(parse("better16", "better16").is_none());
    }
}
