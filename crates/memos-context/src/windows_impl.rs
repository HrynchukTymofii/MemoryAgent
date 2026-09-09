//! Windows context acquisition: foreground window, selection, URL, clipboard.
//!
//! UI Automation is the mechanism for the last three. It is genuinely unreliable
//! per-application — some apps expose nothing, some are slow, some refuse — so
//! every accessor here returns `Option` and failure is normal rather than
//! exceptional.

use windows::core::{Interface, BSTR, VARIANT};
use windows::Win32::Foundation::{BOOL, CloseHandle, HANDLE, HWND, LPARAM, MAX_PATH};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use crate::url::{looks_like_url, normalise_url};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationValuePattern, TreeScope_Descendants, UIA_ControlTypePropertyId,
    UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_TextPatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId,
};

/// Initialise COM once per process for this thread's use of UI Automation.
///
/// Multi-threaded apartment: context collection runs on a worker thread, and an
/// STA would require a message pump we have no reason to run.
pub fn init_com() {
    unsafe {
        // Already-initialised is a success case, not an error: Tauri may have
        // initialised COM on this thread first.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

pub fn foreground_window() -> Option<HWND> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        None
    } else {
        Some(hwnd)
    }
}

/// Find a top-level window whose title contains `needle`, case-insensitively.
///
/// For diagnostics: it makes "what would we capture from *that* window" a
/// question you can ask without focusing it, which matters because focusing a
/// window to inspect it is exactly what changes what is on screen.
pub fn find_window(needle: &str) -> Option<HWND> {
    struct Search {
        needle: String,
        found: Option<HWND>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        if let Some(title) = window_title(hwnd) {
            if title.to_lowercase().contains(&search.needle) {
                search.found = Some(hwnd);
                return BOOL(0); // stop enumerating
            }
        }
        BOOL(1)
    }

    let mut search = Search {
        needle: needle.to_lowercase(),
        found: None,
    };
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
}

pub fn window_title(hwnd: HWND) -> Option<String> {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..n as usize]))
    }
}

/// Executable name of the window's owning process, e.g. `chrome.exe`.
pub fn process_name(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        // LIMITED_INFORMATION rather than QUERY_INFORMATION: it is the least
        // privilege that answers the question, and it succeeds against
        // higher-integrity processes where the fuller right would be refused.
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; MAX_PATH as usize];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        if ok.is_err() || len == 0 {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Some(
            full.rsplit(['\\', '/'])
                .next()
                .unwrap_or(&full)
                .to_string(),
        )
    }
}

pub fn automation() -> Option<IUIAutomation> {
    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() }
}

/// Text currently selected in the focused control.
///
/// Uses the focused element rather than the foreground window: the selection
/// lives in whatever control has keyboard focus, which may be nested many
/// levels inside the window.
pub fn selected_text(uia: &IUIAutomation) -> Option<String> {
    unsafe {
        let focused: IUIAutomationElement = uia.GetFocusedElement().ok()?;
        let pattern = focused.GetCurrentPattern(UIA_TextPatternId).ok()?;
        let text: IUIAutomationTextPattern = pattern.cast().ok()?;
        let ranges = text.GetSelection().ok()?;
        let count = ranges.Length().ok()?;
        if count == 0 {
            return None;
        }
        let mut out = String::new();
        for i in 0..count {
            if let Ok(range) = ranges.GetElement(i) {
                // -1 means "no limit"; a selection larger than this is not a
                // selection, it is a document, and we should not swallow it.
                if let Ok(s) = range.GetText(100_000) {
                    out.push_str(&s.to_string());
                }
            }
        }
        let trimmed = out.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

/// The whole document's text, for "save this page".
///
/// Finds the first Document element in the window and reads its full range.
/// Browsers expose the rendered page this way, which is what makes capturing an
/// article possible with no extension and no network fetch — the text is
/// already in the accessibility tree because a screen reader needs it there.
///
/// `max_chars` is passed to UIA rather than applied afterwards: marshalling a
/// megabyte of text across the process boundary and then throwing most of it
/// away is the expensive half of this call.
pub fn document_text(uia: &IUIAutomation, hwnd: HWND, max_chars: i32) -> Option<String> {
    unsafe {
        let root = uia.ElementFromHandle(hwnd).ok()?;
        let cond = uia
            .CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(UIA_DocumentControlTypeId.0),
            )
            .ok()?;

        // FindFirst, not FindAll: a page has one document, and walking every
        // descendant of a complex application is exactly the call that blows
        // the collection deadline.
        let doc = root.FindFirst(TreeScope_Descendants, &cond).ok()?;
        let pattern = doc.GetCurrentPattern(UIA_TextPatternId).ok()?;
        let text: IUIAutomationTextPattern = pattern.cast().ok()?;
        let range = text.DocumentRange().ok()?;
        let raw = range.GetText(max_chars).ok()?.to_string();

        if raw.trim().is_empty() {
            None
        } else {
            Some(raw)
        }
    }
}

/// Best-effort browser URL.
///
/// There is no standard accessibility property for "the address bar", so this
/// walks the foreground window for edit controls and takes the first whose
/// value looks like a URL. Heuristic by necessity: Chrome, Edge and Firefox all
/// expose the address bar as a plain edit control with no stable identifier
/// across versions.
pub fn current_url(uia: &IUIAutomation, hwnd: HWND) -> Option<String> {
    unsafe {
        let root = uia.ElementFromHandle(hwnd).ok()?;
        let cond = uia
            .CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(UIA_EditControlTypeId.0),
            )
            .ok()?;
        let edits = root.FindAll(TreeScope_Descendants, &cond).ok()?;
        let n = edits.Length().ok()?;
        // Address bars sit near the top of the tree; a handful of candidates is
        // plenty, and walking every edit control in a complex app is slow.
        for i in 0..n.min(12) {
            let Ok(el) = edits.GetElement(i) else { continue };
            let Ok(p) = el.GetCurrentPattern(UIA_ValuePatternId) else {
                continue;
            };
            let Ok(vp) = p.cast::<IUIAutomationValuePattern>() else {
                continue;
            };
            let Ok(v): Result<BSTR, _> = vp.CurrentValue() else {
                continue;
            };
            let s = v.to_string();
            if looks_like_url(&s) {
                return Some(normalise_url(&s));
            }
        }
        None
    }
}

/// Current clipboard text, if any.
///
/// Read-only, and never by synthesising Ctrl+C: sending keystrokes to another
/// application to harvest its selection would destroy whatever the user already
/// had on the clipboard and is indistinguishable from input injection.
pub fn clipboard_text() -> Option<String> {
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
            return None;
        }
        // The clipboard is a single global resource; another app may hold it.
        // Failing quietly is correct — this is a best-effort field.
        if OpenClipboard(None).is_err() {
            return None;
        }
        let result = (|| {
            let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
            let ptr = GlobalLock(windows::Win32::Foundation::HGLOBAL(handle.0)) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let mut len = 0usize;
            while *ptr.add(len) != 0 && len < 1_000_000 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(handle.0));
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        })();
        let _ = CloseClipboard();
        result
    }
}
