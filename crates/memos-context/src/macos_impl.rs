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
//! here fails and the honest result is `None` — which is indistinguishable, from
//! the outside, from an application that simply exposes nothing. Hence
//! [`is_trusted`], and [`probe`] for when that is not enough: a granted
//! permission and a working lookup turned out to be different things, and only
//! the AXError codes tell them apart.
//!
//! Note this is *not* Input Monitoring, which is what the capture shortcut needs.
//! Two permissions, two toggles, adjacent in the same pane, and granting one does
//! nothing for the other.
//!
//! ## Why the API is used rather than Apple Events
//!
//! A URL can also be had by scripting the browser, but Apple Events prompt
//! separately per application, are refused outright by anything sandboxed, and
//! put an entry in a second permission list.
//!
//! There is no single accessibility call that answers it either. Safari sets
//! `kAXDocumentAttribute` on the window; Chrome does not, and puts `AXURL` on
//! the web area inside instead. Both are tried, cheapest first.

use std::ffi::c_void;
use std::time::Duration;

use core_foundation::array::CFArrayRef;
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};

/// An `AXUIElementRef`. Opaque, reference-counted like any CoreFoundation type.
pub type AXUIElementRef = *const c_void;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    /// `Boolean` is an `unsigned char` in C, not a Rust `bool`. Declared as the
    /// byte it really is and compared against zero, because a Rust `bool`
    /// holding anything other than 0 or 1 is undefined behaviour.
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
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

    /// This element's children, up to `limit`.
    ///
    /// Capped by the caller because an accessibility tree is unbounded and this
    /// runs on the capture path — a container with several hundred children is
    /// ordinary in a browser, and reading all of them is a cross-process message
    /// each.
    fn children(&self, limit: usize) -> Vec<AxElement> {
        let Some(value) = self.attribute("AXChildren") else {
            return Vec::new();
        };
        // AXChildren is a CFArray of AXUIElementRef. Checked rather than
        // assumed: an element that answers with something else would otherwise
        // have its value reinterpreted as an array, which is not a mistake that
        // fails safely.
        unsafe {
            if CFGetTypeID(value.0) != CFArrayGetTypeID() {
                return Vec::new();
            }
            let array = value.0 as CFArrayRef;
            let count = CFArrayGetCount(array).min(limit as isize).max(0);
            (0..count)
                .filter_map(|i| {
                    let raw = CFArrayGetValueAtIndex(array, i) as AXUIElementRef;
                    if raw.is_null() {
                        return None;
                    }
                    // The array holds borrowed references; retain so each child
                    // outlives the array it came from.
                    CFRetain(raw as CFTypeRef);
                    AxElement::new(raw)
                })
                .collect()
        }
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

    /// Interpret as a URL.
    ///
    /// `AXURL` is a CFURL, not a CFString, so the string path rejects it as the
    /// wrong type — correctly, and unhelpfully. `CFURLGetString` borrows the
    /// URL's own string representation without copying it.
    fn as_url_string(&self) -> Option<String> {
        unsafe {
            if CFGetTypeID(self.0) != CFURLGetTypeID() {
                // Some builds hand back a plain string here instead.
                return self.as_string();
            }
            let s = CFURLGetString(self.0 as *const c_void);
            if s.is_null() {
                return None;
            }
            Some(CFString::wrap_under_get_rule(s).to_string())
        }
    }
}

extern "C" {
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFURLGetTypeID() -> usize;
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFArrayGetCount(array: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: isize) -> *const c_void;
    fn CFURLGetString(url: *const c_void) -> CFStringRef;
}

/// Does this process have Accessibility permission?
///
/// Worth asking separately because without it every read below returns nothing,
/// which looks exactly like a well-behaved application that exposes nothing —
/// and one of those is fixable by the user in thirty seconds.
pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Walk the same path collection walks, reporting the error at each step.
///
/// Exists because an empty context is silent about *why* it is empty, and the
/// answers are far apart: a permission never granted, a focused application the
/// system will not name, an application that exposes no window, or a window
/// with no document. The AXError codes distinguish all four and nothing else
/// does — -25204 is the API being disabled for this process, -25212 an invalid
/// element, -25205 an attribute the element simply does not have.
///
/// Only run when a capture came back with nothing, so it costs nothing in the
/// normal case.
pub fn probe() -> String {
    fn step(el: &AxElement, name: &str) -> (String, Option<CfValue>) {
        let key = CFString::new(name);
        let mut out: CFTypeRef = std::ptr::null();
        let err =
            unsafe { AXUIElementCopyAttributeValue(el.0, key.as_concrete_TypeRef(), &mut out) };
        if err != AX_SUCCESS {
            return (format!("{name}=err({err})"), None);
        }
        if out.is_null() {
            return (format!("{name}=null"), None);
        }
        let v = CfValue(out);
        let shown = match v.as_string() {
            Some(s) => format!("{name}={:?}", s.chars().take(40).collect::<String>()),
            None => format!("{name}=<non-string>"),
        };
        (shown, Some(v))
    }

    let mut parts = vec![format!("trusted={}", is_trusted())];

    let Some(sys) = system_wide() else {
        parts.push("systemWide=null".into());
        return parts.join(" ");
    };

    let (msg, app_val) = step(&sys, "AXFocusedApplication");
    parts.push(msg);

    // Whichever route produced an element is the one worth reporting on, since
    // the fallback is what actually runs on this system.
    let app = match app_val {
        Some(v) => {
            let raw = v.0 as AXUIElementRef;
            std::mem::forget(v);
            AxElement::new(raw)
        }
        None => match frontmost() {
            Some((pid, name)) => {
                parts.push(format!("frontmost=pid {pid} {name:?}"));
                AxElement::new(unsafe { AXUIElementCreateApplication(pid) })
            }
            None => {
                parts.push("frontmost=none".into());
                None
            }
        },
    };
    let Some(app) = app else {
        return parts.join(" ");
    };

    parts.push(step(&app, "AXTitle").0);

    let (msg, win_val) = step(&app, "AXFocusedWindow");
    parts.push(msg);
    if let Some(win_val) = win_val {
        let raw = win_val.0 as AXUIElementRef;
        std::mem::forget(win_val);
        if let Some(win) = AxElement::new(raw) {
            parts.push(step(&win, "AXTitle").0);
            parts.push(step(&win, "AXDocument").0);
        }
    }

    parts.join(" ")
}

/// The system-wide element, with a bounded messaging timeout.
fn system_wide() -> Option<AxElement> {
    let el = AxElement::new(unsafe { AXUIElementCreateSystemWide() })?;
    unsafe { AXUIElementSetMessagingTimeout(el.0, MESSAGING_TIMEOUT.as_secs_f32()) };
    Some(el)
}

/// The frontmost application, as the window server understands it.
///
/// `NSWorkspace` rather than the Accessibility API, because the obvious
/// accessibility route does not work: `kAXFocusedApplicationAttribute` on the
/// system-wide element answers `kAXErrorNoValue` (-25212) here — not "denied",
/// not "no such attribute", simply nothing — and since every other lookup hangs
/// off that element, the entire context came back empty while the permission
/// was granted and every individual call was correct.
///
/// Returns the process id and the name together because both come from this one
/// answer, and the name is better than anything the AX tree offers: an
/// application element frequently has no `AXTitle` at all.
fn frontmost() -> Option<(i32, Option<String>)> {
    use objc2_app_kit::NSWorkspace;

    // Reading the frontmost application is a snapshot read and is safe off the
    // main thread; this runs on the capture's collection thread.
    let workspace = NSWorkspace::sharedWorkspace();
    let app = workspace.frontmostApplication()?;
    let pid = app.processIdentifier();
    let name = app.localizedName()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty());
    Some((pid, name))
}

/// The application element for the frontmost app.
///
/// The system-wide element is still tried first: when it answers, it accounts
/// for keyboard focus, which is a better question than "what is frontmost". It
/// is the fallback that does the work in practice.
pub fn focused_application() -> Option<AxElement> {
    if let Some(app) = system_wide().and_then(|s| s.element("AXFocusedApplication")) {
        unsafe { AXUIElementSetMessagingTimeout(app.0, MESSAGING_TIMEOUT.as_secs_f32()) };
        return Some(app);
    }
    let (pid, _) = frontmost()?;
    let app = AxElement::new(unsafe { AXUIElementCreateApplication(pid) })?;
    unsafe { AXUIElementSetMessagingTimeout(app.0, MESSAGING_TIMEOUT.as_secs_f32()) };
    Some(app)
}

/// The name of the frontmost application.
///
/// The workspace's name wins over `AXTitle`: it is the name the user sees in
/// the Dock, and an application element often has no title at all.
pub fn application_name(app: &AxElement) -> Option<String> {
    frontmost()
        .and_then(|(_, name)| name)
        .or_else(|| app.string("AXTitle").filter(|s| !s.trim().is_empty()))
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
    let raw = window
        .string("AXDocument")
        // Chrome and its derivatives do not set AXDocument on the window. They
        // put the address on the web area inside it, as AXURL, so a browser
        // that answers nothing above is worth one bounded look downward.
        .or_else(|| web_area_url(&window))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Shared with the Windows path so a URL is stored the same way whichever
    // platform captured it — otherwise the same page saved twice is two
    // different strings.
    Some(crate::url::normalise_url(trimmed))
}

/// How deep the search for a web area goes, and how wide at each level.
///
/// Bounded rather than exhaustive, and not as a micro-optimisation: this runs
/// while the user is speaking, every lookup is a cross-process message, and a
/// page with a thousand elements would otherwise be walked in full for a string
/// that lives three or four levels down. A browser puts its web area near the
/// top of the window; if it is not there, it is not worth the rest of the
/// capture's budget to find out.
const URL_SEARCH_DEPTH: u32 = 5;
const URL_SEARCH_BREADTH: usize = 12;

/// Find the address of the page inside a browser window.
///
/// `AXURL` comes back as a CFURL rather than a CFString, so it is read as a
/// description rather than through the string path — which is why the
/// type-checked `as_string` would answer `None` for a perfectly good value.
fn web_area_url(element: &AxElement) -> Option<String> {
    fn walk(el: &AxElement, depth: u32) -> Option<String> {
        if el.string("AXRole").as_deref() == Some("AXWebArea") {
            if let Some(url) = el.attribute("AXURL").and_then(|v| v.as_url_string()) {
                return Some(url);
            }
        }
        if depth == 0 {
            return None;
        }
        for child in el.children(URL_SEARCH_BREADTH) {
            if let Some(found) = walk(&child, depth - 1) {
                return Some(found);
            }
        }
        None
    }
    walk(element, URL_SEARCH_DEPTH)
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
