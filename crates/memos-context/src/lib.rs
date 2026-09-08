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
pub use capture::{collect, start, Pending};
#[cfg(windows)]
mod windows_impl;

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
    pub clipboard_text: Option<String>,
    pub captured_at: Option<String>,
    /// How long collection took. Watched because this runs on the capture path
    /// and a regression here is invisible until the product feels slow.
    pub elapsed_ms: u32,
}

impl Context {
    /// Whether there is anything worth attaching to a memory.
    ///
    /// A window title alone is weak evidence of intent; a selection or a URL is
    /// a real referent for the word "this".
    pub fn has_referent(&self) -> bool {
        self.selected_text.as_deref().is_some_and(|s| !s.trim().is_empty())
            || self.current_url.is_some()
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

/// Which fields the user has allowed. Spec section 7 requires context
/// acquisition to be permission-aware, and clipboard and selection are the two
/// that read content the user did not explicitly hand over.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ContextPermissions {
    pub window: bool,
    pub url: bool,
    pub selection: bool,
    pub clipboard: bool,
}

impl Default for ContextPermissions {
    fn default() -> Self {
        Self {
            window: true,
            url: true,
            selection: true,
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
