//! What the user was looking at, read through the macOS Accessibility API.
//!
//! The counterpart of [`crate::windows_impl`], and the same shape: small,
//! independently-optional lookups that a caller composes behind a deadline. Every
//! function here returns `Option` and none of them are allowed to block for long,
//! because all of this runs while the user is speaking.
//!
//! ## The permission, which has no prompt worth the name
//!
//! Reading another application's window requires **Accessibility**, and there is
//! no API that asks for it in place. `AXIsProcessTrustedWithOptions` can raise a
//! dialogue whose only button sends the user to System Settings, where they must
//! find the app in a list and turn it on themselves. Until they do, every call
//! here returns `kAXErrorAPIDisabled` and the honest result is `None` — which is
//! indistinguishable, from the outside, from an application that simply exposes
//! nothing. Hence [`is_trusted`]: the difference matters enough to ask directly.
//!
//! Note this is *not* Input Monitoring, which is what the capture shortcut needs.
//! Two permissions, two toggles, adjacent in the same pane, and granting one does
//! nothing for the other.
//!
//! ## Why the API is used rather than Apple Events
//!
//! A URL can also be had by scripting the browser, but Apple Events prompt
//! separately per application, are refused outright by anything sandboxed, and
//! put an entry in a second permission list. `kAXDocumentAttribute` is one call
//! that works across Safari, Chrome and their derivatives.

use std::ffi::c_void;
use std::time::Duration;

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};

/// An `AXUIElementRef`. Opaque, reference-counted like any CoreFoundation type.
pub type AXUIElementRef = *const c_void;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    /// Bounds how long a single call may block. Without it one unresponsive
    /// application stalls the whole capture — the default is six seconds, which
    /// is twenty times the budget this crate collects under.
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
}

const AX_SUCCESS: i32 = 0;

/// Everything here runs while the user is mid-sentence, so no single lookup may
/// cost more than a fraction of the collection deadline.
const MESSAGING_TIMEOUT: Duration = Duration::from_millis(250);

/// An owned `AXUIElementRef` that releases on drop.
///
/// The Copy-rule types come back with a +1 retain count and leak without this.
/// Capture runs on every shortcut press, so a leak per press is a leak per use
/// of the product.
pub struct AxElement(AXUIElementRef);

impl Drop for AxElement {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0 as CFTypeRef) };
        }
    }
}

impl AxElement {
    fn new(raw: AXUIElementRef) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw))
    }

    /// Read one attribute, as a raw CoreFoundation value.
    fn attribute(&self, name: &str) -> Option<CfValue> {
        let key = CFString::new(name);
        let mut out: CFTypeRef = std::ptr::null();
        let err = unsafe {
            AXUIElementCopyAttributeValue(self.0, key.as_concrete_TypeRef(), &mut out)
        };
        if err != AX_SUCCESS || out.is_null() {
            return None;
        }
        Some(CfValue(out))
    }

    fn string(&self, name: &str) -> Option<String> {
        self.attribute(name)?.as_string()
    }

    /// Follow an attribute that yields another element.
    fn element(&self, name: &str) -> Option<AxElement> {
        let v = self.attribute(name)?;
        let raw = v.0 as AXUIElementRef;
        // Ownership moves into the new wrapper rather than being released here.
        std::mem::forget(v);
        AxElement::new(raw)
    }
}

/// A CoreFoundation value from a Copy-rule call. Released on drop.
struct CfValue(CFTypeRef);

impl Drop for CfValue {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

impl CfValue {
    /// Interpret as a string, when it is one.
    ///
    /// Type-checked rather than assumed: `kAXValueAttribute` is a string in a
    /// text field and a number in a slider, and reading a number as a string is
    /// how a crash gets into the capture path.
    fn as_string(&self) -> Option<String> {
        unsafe {
            if CFGetTypeID(self.0) != CFStringGetTypeID() {
                return None;
            }
            Some(CFString::wrap_under_get_rule(self.0 as CFStringRef).to_string())
        }
    }
}

extern "C" {
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
}

/// Does this process have Accessibility permission?
///
/// Worth asking separately because without it every read below returns nothing,
/// which looks exactly like a well-behaved application that exposes nothing —
/// and one of those is fixable by the user in thirty seconds.
pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// The system-wide element, with a bounded messaging timeout.
fn system_wide() -> Option<AxElement> {
    let el = AxElement::new(unsafe { AXUIElementCreateSystemWide() })?;
    unsafe { AXUIElementSetMessagingTimeout(el.0, MESSAGING_TIMEOUT.as_secs_f32()) };
    Some(el)
}

/// The application element for the frontmost app.
///
/// Reached through the system-wide element's focused application rather than by
/// asking NSWorkspace and then building an element from the pid. Both work; this
/// one is a single call and cannot disagree with the focused-element lookups that
/// follow it, which the two-step version can when focus moves mid-capture.
pub fn focused_application() -> Option<AxElement> {
    let app = system_wide()?.element("AXFocusedApplication")?;
    unsafe { AXUIElementSetMessagingTimeout(app.0, MESSAGING_TIMEOUT.as_secs_f32()) };
    Some(app)
}

/// The name of the frontmost application.
pub fn application_name(app: &AxElement) -> Option<String> {
    app.string("AXTitle").filter(|s| !s.trim().is_empty())
}

/// The focused window's title.
pub fn window_title(app: &AxElement) -> Option<String> {
    let window = app.element("AXFocusedWindow")?;
    window.string("AXTitle").filter(|s| !s.trim().is_empty())
}

/// The selected text, wherever the focus is.
///
/// `kAXSelectedTextAttribute` on the focused *element*, not the window: the
/// window does not have a selection, the text field inside it does.
pub fn selected_text(app: &AxElement) -> Option<String> {
    let focused = app.element("AXFocusedUIElement")?;
    focused
        .string("AXSelectedText")
        .filter(|s| !s.trim().is_empty())
}

/// The URL of the document in the focused window, when it is showing one.
///
/// `kAXDocumentAttribute` is what a browser sets to the address of the page it
/// is displaying. It also has a second, useful meaning: in a text editor it is
/// the `file://` path of the open document, which is just as good an answer to
/// "what was I looking at".
pub fn current_url(app: &AxElement) -> Option<String> {
    let window = app.element("AXFocusedWindow")?;
    let raw = window.string("AXDocument")?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Shared with the Windows path so a URL is stored the same way whichever
    // platform captured it — otherwise the same page saved twice is two
    // different strings.
    Some(crate::url::normalise_url(trimmed))
}

/// The clipboard's text, if it holds any.
pub fn clipboard_text() -> Option<String> {
    use objc2_app_kit::NSPasteboard;

    // The general pasteboard is a shared system resource and reading it is a
    // plain read — no permission, no prompt, and no effect on what is in it.
    let text = unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.stringForType(objc2_app_kit::NSPasteboardTypeString)
    }?;
    let s = text.to_string();
    (!s.trim().is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without permission every lookup must answer `None` rather than panic or
    /// hang — the state of every machine before the user visits System Settings,
    /// and the one this crate is most likely to run in.
    #[test]
    fn every_lookup_is_safe_without_permission() {
        let started = std::time::Instant::now();
        if let Some(app) = focused_application() {
            let _ = application_name(&app);
            let _ = window_title(&app);
            let _ = selected_text(&app);
            let _ = current_url(&app);
        }
        let _ = clipboard_text();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "lookups took {:?}, which would blow the collection deadline",
            started.elapsed()
        );
    }
}
