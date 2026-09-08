//! What the user was looking at when they pressed the shortcut.
//!
//! Spec section 7. Without this, "save **this**" has no referent — the command
//! is meaningless on its own, and the whole premise of capturing without
//! explaining depends on the system already knowing what "this" is.
//!
//! ## Two properties shape the implementation
//!
//! **It must never block the capture.** Every field is best-effort and
//! independently optional: UI Automation can be slow, absent, or refused by a
//! hardened application, and a missing URL must degrade the capture rather than
//! fail it. Collection runs behind a deadline and returns whatever it has.
//!
//! **It runs in parallel with the speech, not after it.** The user speaks for
//! two to four seconds; gathering context inside that window costs nothing on
//! the timeline (section 4, stage 3). Doing it at release would add directly to
//! the latency the whole design exists to protect.

pub mod capture;
pub mod readable;

/// The Windows implementation, public so diagnostics can ask it direct
/// questions — "what does this window actually expose" is not answerable from
/// the trimmed [`Context`] alone.
#[cfg(windows)]
pub mod windows_impl;
pub use capture::{collect, start, Pending};

use serde::{Deserialize, Serialize};

/// The normalized context object from spec section 7.
///
/// Every field is optional on purpose. A partial context is useful; a failed
/// capture is not.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Context {
    pub active_application: Option<String>,
    pub active_window_title: Option<String>,
    pub current_url: Option<String>,
    pub selected_text: Option<String>,
    /// The readable text of the page, when there is one and it has prose in it.
    ///
    /// Trimmed by [`readable::extract`] before it gets here — the raw
    /// accessibility text is half navigation, and that half is identical across
    /// every page a user saves.
    pub page_text: Option<String>,
    pub clipboard_text: Option<String>,
    pub captured_at: Option<String>,
    /// How long collection took. Watched because this runs on the capture path
    /// and a regression here is invisible until the product feels slow.
    pub elapsed_ms: u32,
}

impl Context {
    /// Whether there is anything worth attaching to a memory.
    ///
    /// A window title alone is weak evidence of intent; a selection, a page or
    /// a URL is a real referent for the word "this".
    pub fn has_referent(&self) -> bool {
        self.selected_text.as_deref().is_some_and(|s| !s.trim().is_empty())
            || self.page_text.as_deref().is_some_and(|s| !s.trim().is_empty())
            || self.current_url.is_some()
    }

    /// What "this" refers to, in order of how directly the user chose it.
    ///
    /// A selection is an explicit choice and always wins. The page is what they
    /// were looking at. The URL is the last resort — it names the memory
    /// without containing any of it, which is a bookmark rather than a memory.
    pub fn referent(&self) -> Option<&str> {
        self.selected_text
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.page_text.as_deref().filter(|s| !s.trim().is_empty()))
            .or(self.current_url.as_deref())
    }

    /// What the correction log is allowed to remember about this situation.
    ///
    /// ADR-0006 calls that log the most valuable table in the system and, in
    /// the same breath, the most sensitive: it is a record of what the user
    /// said and did. The router needs to know *what kind* of thing was on
    /// screen when a command was spoken — that is what makes a past command
    /// comparable to the present one — and it never needs a second copy of the
    /// article, the selection or the clipboard.
    ///
    /// So the text fields become booleans and nothing else changes. A memory
    /// the user deliberately saved keeps its content, in `knowledge_items`,
    /// where they can see and delete it. A command they merely spoke does not
    /// quietly acquire one too.
    pub fn digest(&self) -> ContextDigest {
        fn present(s: &Option<String>) -> bool {
            s.as_deref().is_some_and(|v| !v.trim().is_empty())
        }
        ContextDigest {
            active_application: self.active_application.clone(),
            active_window_title: self.active_window_title.clone(),
            current_url: self.current_url.clone(),
            had_selection: present(&self.selected_text),
            had_page: present(&self.page_text),
            had_clipboard: present(&self.clipboard_text),
        }
    }

    /// The best available title for a memory captured from here.
    pub fn suggested_title(&self) -> Option<String> {
        // Browser titles carry the site name as a suffix; strip it so a saved
        // item reads as the article rather than the browser.
        if let Some(t) = &self.active_window_title {
            let cleaned = t
                .rsplit_once(" - ")
                .map(|(head, _)| head)
                .unwrap_or(t)
                .trim();
            if !cleaned.is_empty() {
                return Some(cleaned.to_string());
            }
        }
        None
    }
}

/// A situation, without its contents. See [`Context::digest`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextDigest {
    pub active_application: Option<String>,
    pub active_window_title: Option<String>,
    pub current_url: Option<String>,
    pub had_selection: bool,
    pub had_page: bool,
    pub had_clipboard: bool,
}

/// Which fields the user has allowed. Spec section 7 requires context
/// acquisition to be permission-aware, and clipboard and selection are the two
/// that read content the user did not explicitly hand over.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ContextPermissions {
    pub window: bool,
    pub url: bool,
    pub selection: bool,
    /// Read the whole page, not only what is selected.
    ///
    /// On by default: it is the difference between saving an article and saving
    /// a link to one, and it only ever runs on a capture the user asked for.
    pub page: bool,
    pub clipboard: bool,
}

impl Default for ContextPermissions {
    fn default() -> Self {
        Self {
            window: true,
            url: true,
            selection: true,
            page: true,
            // Off by default. The clipboard frequently holds passwords and
            // tokens the user never intended to share, and unlike a selection
            // it is not evidence of present intent — it may be hours old.
            clipboard: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_title_loses_the_browser_suffix() {
        let c = Context {
            active_window_title: Some("React - State as a Snapshot - Google Chrome".into()),
            ..Default::default()
        };
        assert_eq!(
            c.suggested_title().as_deref(),
            Some("React - State as a Snapshot")
        );
    }

    #[test]
    fn a_title_alone_is_not_a_referent() {
        let c = Context {
            active_window_title: Some("Notepad".into()),
            ..Default::default()
        };
        assert!(!c.has_referent(), "a window title is not what 'this' means");
    }

    #[test]
    fn selection_or_url_counts_as_a_referent() {
        let c = Context {
            selected_text: Some("state is a snapshot".into()),
            ..Default::default()
        };
        assert!(c.has_referent());

        let c2 = Context {
            current_url: Some("https://react.dev".into()),
            ..Default::default()
        };
        assert!(c2.has_referent());
    }

    #[test]
    fn whitespace_selection_is_not_a_referent() {
        let c = Context {
            selected_text: Some("   \n ".into()),
            ..Default::default()
        };
        assert!(!c.has_referent());
    }

    #[test]
    fn clipboard_is_off_by_default() {
        assert!(!ContextPermissions::default().clipboard);
    }
}
