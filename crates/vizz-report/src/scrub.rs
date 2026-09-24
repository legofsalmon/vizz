//! Take the person out of a report before it is written anywhere.
//!
//! Runs on the device, on every string a report carries, before the
//! report reaches the queue — so what sits in the outbox is already what
//! would be sent, and a report that is never sent was never anything
//! else either. The rules are the intake contract's:
//!
//! 1. the home directory becomes `~`;
//! 2. the user name is stripped from any other path that carries one
//!    (`/Users/<name>/`, `/home/<name>/`, `C:\Users\<name>\`), which
//!    catches a home that was not ours — another account's, a mounted
//!    volume's — as well as ours spelled differently;
//! 3. anything after `?` in a URL is dropped, since query strings are
//!    where tokens, keys and emails travel;
//! 4. the result is cut to the field's limit, on a character boundary.

/// What stands in for a stripped user name.
pub const USER: &str = "<user>";

/// The home directory and user name of whoever is running this, read
/// once. Either may be missing; the rules that need it are skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub home: Option<String>,
    pub user: Option<String>,
}

impl Identity {
    pub fn current() -> Self {
        let home = std::env::home_dir()
            .map(|h| h.to_string_lossy().into_owned())
            .filter(|h| h.len() > 1);
        let user = ["USER", "USERNAME", "LOGNAME"]
            .iter()
            .find_map(|v| std::env::var(v).ok())
            .or_else(|| {
                home.as_deref()
                    .and_then(|h| h.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next())
                    .map(str::to_string)
            })
            .filter(|u| !u.is_empty());
        Identity { home, user }
    }
}

/// Apply every rule, then cut to `limit` characters.
pub fn scrub(text: &str, who: &Identity, limit: usize) -> String {
    let mut out = text.to_string();
    if let Some(home) = who.home.as_deref() {
        // Both separators: a Windows home shows up with either, depending
        // on who formatted the path.
        out = out.replace(home, "~");
        let alt = if home.contains('\\') { home.replace('\\', "/") } else { home.replace('/', "\\") };
        if alt != home {
            out = out.replace(&alt, "~");
        }
    }
    out = strip_user_dirs(&out);
    if let Some(user) = who.user.as_deref()
        && user.len() >= 3
    {
        // A name that survives the path rules — in a volume name, a
        // temp path, a hostname-looking segment — goes too. Three
        // characters or more, so a user called "a" does not blank every
        // "a" in a backtrace.
        out = replace_word(&out, user, USER);
    }
    out = strip_queries(&out);
    cut(&out, limit)
}

/// `/Users/<x>/`, `/home/<x>/` and `\Users\<x>\` with the segment
/// replaced, whoever `<x>` is.
fn strip_user_dirs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'outer: loop {
        let mut best: Option<(usize, &str)> = None;
        for marker in ["/Users/", "/home/", "\\Users\\", "\\home\\"] {
            if let Some(at) = rest.find(marker)
                && best.is_none_or(|(b, _)| at < b)
            {
                best = Some((at, marker));
            }
        }
        let Some((at, marker)) = best else {
            out.push_str(rest);
            break 'outer;
        };
        let after = at + marker.len();
        out.push_str(&rest[..after]);
        let seg_end = rest[after..]
            .find(|c: char| c == '/' || c == '\\' || c.is_whitespace() || c == '"' || c == '\'' || c == ')')
            .map_or(rest.len(), |i| after + i);
        let segment = &rest[after..seg_end];
        // `/Users/Shared` is not a person, and `~` means we already did.
        if segment.is_empty() || segment == "Shared" || segment == "~" || segment == USER {
            out.push_str(segment);
        } else {
            out.push_str(USER);
        }
        rest = &rest[seg_end..];
    }
    out
}

/// Replace `word` where it is not part of a longer identifier.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(word) {
        let before_ok = rest[..at].chars().next_back().is_none_or(|c| !is_ident(c));
        let after_ok = rest[at + word.len()..].chars().next().is_none_or(|c| !is_ident(c));
        out.push_str(&rest[..at]);
        out.push_str(if before_ok && after_ok { with } else { word });
        rest = &rest[at + word.len()..];
    }
    out.push_str(rest);
    out
}

/// Drop the query from every `scheme://…?…` in the text, keeping the
/// `?` so it is visible that something was removed.
fn strip_queries(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, token) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        match (token.contains("://"), token.find('?')) {
            (true, Some(q)) => {
                out.push_str(&token[..=q]);
                // Keep a closing bracket or quote that belonged to the
                // surrounding text rather than to the URL.
                let tail: String = token[q + 1..]
                    .chars()
                    .rev()
                    .take_while(|c| matches!(c, ')' | ']' | '"' | '\'' | ',' | '.'))
                    .collect();
                out.extend(tail.chars().rev());
            }
            _ => out.push_str(token),
        }
    }
    out
}

/// At most `limit` characters — not bytes, since the contract counts
/// characters and a byte cut can land inside one.
pub fn cut(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((at, _)) => text[..at].to_string(),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colm() -> Identity {
        Identity { home: Some("/Users/colm".into()), user: Some("colm".into()) }
    }

    #[test]
    fn the_home_directory_becomes_a_tilde() {
        let s = scrub("at /Users/colm/.cargo/registry/src/x.rs:10", &colm(), 1000);
        assert_eq!(s, "at ~/.cargo/registry/src/x.rs:10");
    }

    #[test]
    fn a_user_name_in_any_other_home_path_is_stripped() {
        let who = colm();
        let s = scrub("open /Users/alice/Shows/night.json and /home/bob/x", &who, 1000);
        assert!(!s.contains("alice") && !s.contains("bob"), "{s}");
        assert!(s.contains("/Users/<user>/Shows"), "{s}");
        let s = scrub(r"C:\Users\Dana\AppData\vizz\settings.json", &Identity::default(), 1000);
        assert_eq!(s, r"C:\Users\<user>\AppData\vizz\settings.json");
        // Shared is a macOS system folder, not a person.
        assert_eq!(scrub("/Users/Shared/vizz", &Identity::default(), 100), "/Users/Shared/vizz");
    }

    #[test]
    fn a_windows_home_is_caught_with_either_separator() {
        let who = Identity { home: Some(r"C:\Users\dana".into()), user: Some("dana".into()) };
        assert_eq!(scrub(r"C:\Users\dana\x.rs", &who, 100), r"~\x.rs");
        assert_eq!(scrub("C:/Users/dana/x.rs", &who, 100), "~/x.rs");
    }

    #[test]
    fn the_user_name_goes_wherever_else_it_appears_as_a_word() {
        let who = colm();
        let s = scrub("volume /Volumes/colm-backup and colmhewson and colm", &who, 1000);
        assert!(s.contains("/Volumes/<user>-backup"), "{s}");
        // Part of a longer word: not a name, left alone.
        assert!(s.contains("colmhewson"), "{s}");
        assert!(s.ends_with("and <user>"), "{s}");
        // A very short name would blank letters all over a trace.
        let who = Identity { home: None, user: Some("a".into()) };
        assert_eq!(scrub("a panic at a place", &who, 100), "a panic at a place");
    }

    #[test]
    fn url_queries_are_dropped() {
        let s = scrub(
            "GET https://letissier.ie/api/x?key=LT-VIZZ-1234&email=a@b.c failed (https://h.test/y?t=1)",
            &Identity::default(),
            1000,
        );
        assert_eq!(s, "GET https://letissier.ie/api/x? failed (https://h.test/y?)");
        // A question mark that is not in a URL is ordinary punctuation.
        assert_eq!(scrub("why? because", &Identity::default(), 100), "why? because");
    }

    #[test]
    fn cutting_counts_characters_and_never_splits_one() {
        assert_eq!(cut("abcdef", 3), "abc");
        assert_eq!(cut("ab", 3), "ab");
        let s = cut("ééééé", 2);
        assert_eq!(s, "éé");
        assert_eq!(scrub(&"x".repeat(500), &Identity::default(), 300).chars().count(), 300);
    }
}
