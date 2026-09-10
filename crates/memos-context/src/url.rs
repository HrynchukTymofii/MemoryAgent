//! Deciding whether a string is a URL, and storing it the same way twice.
//!
//! Pure string processing, and therefore shared: both platforms read an address
//! out of a widget that may equally be holding a search query or a filename, and
//! both must store the result in the same shape. A page saved on a Mac and the
//! same page saved on Windows have to come out as one string, or they are two
//! different sources for the same memory.

/// Whether a string from an edit control is plausibly a URL.
///
/// Deliberately strict. A false positive here attaches someone's search query
/// or a half-typed sentence to a saved memory as its source, which is worse
/// than having no URL at all.
pub fn looks_like_url(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > 2048 || s.contains(char::is_whitespace) {
        return false;
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return true;
    }
    // Bare host as browsers display it, e.g. "react.dev/learn".
    let host = s.split(['/', '?', '#']).next().unwrap_or(s);
    if host.is_empty() || host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':')
    {
        return false;
    }

    // Development hosts have no dot but are perfectly real addresses.
    let hostname = host.split(':').next().unwrap_or(host);
    if hostname.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if !host.contains('.') {
        return false;
    }

    // A filename looks exactly like a domain. Since we scan every edit control
    // in the window rather than a known address bar, "notes.txt" would
    // otherwise be attached to a memory as its source URL. The trade is
    // deliberate: a bare domain whose TLD collides with a file extension (.md,
    // .sh) is rejected, but the same address typed with a scheme still works,
    // and a wrong source is worse than a missing one.
    const FILE_EXTS: &[&str] = &[
        "txt", "md", "json", "exe", "dll", "pdf", "png", "jpg", "jpeg", "gif", "svg", "doc",
        "docx", "xls", "xlsx", "ppt", "csv", "zip", "rar", "7z", "rs", "ts", "tsx", "js", "jsx",
        "html", "htm", "css", "log", "ini", "cfg", "conf", "yml", "yaml", "toml", "bin", "wav",
        "mp3", "mp4", "mkv", "iso", "bat", "ps1", "sql", "db",
    ];
    let tld = hostname.rsplit('.').next().unwrap_or("");
    if tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    !FILE_EXTS.contains(&tld.to_ascii_lowercase().as_str())
}

/// Browsers hide the scheme in the address bar; restore it so the stored source
/// is a link that actually resolves.
pub fn normalise_url(s: &str) -> String {
    let s = s.trim();
    // Anything that already names a scheme is left exactly as it is. This
    // matters more than it looks: the Windows path only ever sees what a user
    // could type into an address bar, but the macOS one reads the browser's own
    // answer, which is fully-schemed and not always http — `chrome://newtab/`,
    // `file:///Users/...`, `about:blank`. Prefixing those produced
    // `https://chrome://newtab/`, a string that is not a URL of any kind.
    if s.split_once("://").is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
    }) {
        return s.to_string();
    }
    format!("https://{s}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_real_urls() {
        assert!(looks_like_url("https://react.dev/learn/state-as-a-snapshot"));
        assert!(looks_like_url("react.dev/learn"));
        assert!(looks_like_url("localhost:1420"));
    }

    #[test]
    fn rejects_search_queries_and_prose() {
        // The failure that matters: attaching a search box's contents to a
        // memory as its source.
        assert!(!looks_like_url("how to use react state"));
        assert!(!looks_like_url(""));
        assert!(!looks_like_url("save this to react"));
        // A filename is the dangerous false positive: it would be stored as
        // the memory's source.
        assert!(!looks_like_url("notes.txt "));
        assert!(!looks_like_url("README.md"));
        assert!(!looks_like_url("build.ps1"));
        assert!(!looks_like_url("."));
        assert!(!looks_like_url("192.168.1.1.")); // trailing dot
    }

    #[test]
    fn restores_the_hidden_scheme() {
        assert_eq!(normalise_url("react.dev"), "https://react.dev");
        assert_eq!(normalise_url("https://react.dev"), "https://react.dev");
        assert_eq!(normalise_url("http://x.com"), "http://x.com");
    }

    #[test]
    fn a_scheme_that_is_not_http_survives_intact() {
        // What a browser actually answers on macOS. Prefixing any of these
        // produced a string that was not a URL at all.
        assert_eq!(normalise_url("chrome://newtab/"), "chrome://newtab/");
        assert_eq!(
            normalise_url("file:///Users/x/notes.md"),
            "file:///Users/x/notes.md"
        );
        // Not a scheme: a bare host that happens to contain a colon.
        assert_eq!(normalise_url("localhost:1420"), "https://localhost:1420");
    }
}
