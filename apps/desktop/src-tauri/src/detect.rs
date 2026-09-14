//! Noticing that a call has started.
//!
//! Windows records, per application, when it last started and stopped using
//! the microphone — the same record behind the "is using your microphone" icon
//! in the taskbar. An entry whose stop time is zero is an application holding
//! the microphone right now. That is the signal: nothing is injected into the
//! meeting app and nothing listens to the audio.
//!
//! Holding the microphone is not the same as being in a call — a voice message,
//! a recorder, this app itself — so only known call apps count, and a browser
//! counts only while one of its windows is a meeting.

use std::time::Duration;

/// How often the record is read. A call is minutes long; noticing it within a
/// few seconds is plenty, and each read is a handful of registry calls.
const POLL: Duration = Duration::from_secs(2);

/// Consecutive reads a change must survive before it is reported, so a
/// three-second voice message does not become a meeting.
const STEADY_READS: u32 = 3;

/// Call apps, by a fragment of the executable path or package name.
const CALL_APPS: &[(&str, &str)] = &[
    ("zoom.exe", "Zoom"),
    ("msteams", "Microsoft Teams"),
    ("ms-teams.exe", "Microsoft Teams"),
    ("#teams.exe", "Microsoft Teams"),
    ("slack", "Slack"),
    ("webex", "Webex"),
    ("ciscocollabhost.exe", "Webex"),
    ("skype", "Skype"),
    ("discord.exe", "Discord"),
    ("whatsapp", "WhatsApp"),
    ("telegram", "Telegram"),
];

const BROWSERS: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
    "arc.exe",
];

/// Window titles a browser shows while it is in a meeting.
const MEETING_TITLES: &[(&str, &str)] = &[
    ("meet - ", "Google Meet"),
    ("meet – ", "Google Meet"),
    ("google meet", "Google Meet"),
    ("microsoft teams", "Microsoft Teams"),
    ("zoom meeting", "Zoom"),
    ("zoom workplace", "Zoom"),
    ("whereby", "Whereby"),
    ("jitsi meet", "Jitsi Meet"),
];

/// Report a call starting (`Some(app)`) and ending (`None`), on a thread of
/// its own, for as long as the process runs.
///
/// Whatever is true at launch is taken as the starting point rather than
/// announced: a meeting app that crashed can leave its entry claiming the
/// microphone for ever, and that must not greet every launch with a prompt.
pub fn watch(on_change: impl Fn(Option<&'static str>) + Send + 'static) {
    std::thread::Builder::new()
        .name("call-watch".into())
        .spawn(move || {
            let mut steady = Steady::new(current_call());
            loop {
                std::thread::sleep(POLL);
                if let Some(change) = steady.read(current_call()) {
                    tracing::info!(call = ?change, "call state changed");
                    on_change(change);
                }
            }
        })
        .expect("spawn call watcher");
}

/// Turns noisy reads into changes that held.
struct Steady {
    reported: Option<&'static str>,
    candidate: Option<&'static str>,
    reads: u32,
}

impl Steady {
    fn new(initial: Option<&'static str>) -> Self {
        Self {
            reported: initial,
            candidate: initial,
            reads: 0,
        }
    }

    fn read(&mut self, now: Option<&'static str>) -> Option<Option<&'static str>> {
        if now != self.candidate {
            self.candidate = now;
            self.reads = 0;
        }
        self.reads += 1;
        if self.candidate != self.reported && self.reads >= STEADY_READS {
            self.reported = self.candidate;
            return Some(self.reported);
        }
        None
    }
}

/// The call in progress, named for a person, if there is one.
pub fn current_call() -> Option<&'static str> {
    let holders = microphone_holders();
    holders.iter().find_map(|h| call_app(h, &has_window_titled))
}

/// Which call a microphone holder is in, if it is one.
///
/// `holder` is the registry's name for the application: a package name, or an
/// executable path with `#` for each separator.
fn call_app(holder: &str, has_window: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    let holder = holder.to_lowercase();
    if let Some((_, name)) = CALL_APPS.iter().find(|(key, _)| holder.contains(key)) {
        return Some(name);
    }
    if BROWSERS.iter().any(|b| holder.ends_with(b)) {
        return MEETING_TITLES
            .iter()
            .find(|(title, _)| has_window(title))
            .map(|(_, name)| *name);
    }
    None
}

#[cfg(windows)]
fn has_window_titled(fragment: &str) -> bool {
    memos_context::windows_impl::find_window(fragment).is_some()
}

#[cfg(not(windows))]
fn has_window_titled(_fragment: &str) -> bool {
    false
}

/// Applications holding the microphone right now.
#[cfg(windows)]
fn microphone_holders() -> Vec<String> {
    use windows::core::{w, PCWSTR, PWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        RRF_RT_REG_QWORD,
    };

    /// The subkeys of `key` whose stop time is zero and start time is not.
    unsafe fn holding(key: HKEY, out: &mut Vec<String>) {
        let mut index = 0;
        loop {
            let mut name = [0u16; 512];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(
                key,
                index,
                PWSTR(name.as_mut_ptr()),
                &mut len,
                None,
                PWSTR::null(),
                None,
                None,
            ) != ERROR_SUCCESS
            {
                break;
            }
            index += 1;
            let sub = PCWSTR(name.as_ptr());
            let qword = |value: PCWSTR| -> Option<u64> {
                let mut v = 0u64;
                let mut size = std::mem::size_of::<u64>() as u32;
                (RegGetValueW(
                    key,
                    sub,
                    value,
                    RRF_RT_REG_QWORD,
                    None,
                    Some(&mut v as *mut u64 as *mut _),
                    Some(&mut size),
                ) == ERROR_SUCCESS)
                    .then_some(v)
            };
            if qword(w!("LastUsedTimeStop")) == Some(0)
                && qword(w!("LastUsedTimeStart")).unwrap_or(0) != 0
            {
                out.push(String::from_utf16_lossy(&name[..len as usize]));
            }
        }
    }

    let mut out = Vec::new();
    for path in [
        w!(
            r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone"
        ),
        w!(
            r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone\NonPackaged"
        ),
    ] {
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, path, 0, KEY_READ, &mut key) == ERROR_SUCCESS {
                holding(key, &mut out);
                let _ = RegCloseKey(key);
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn microphone_holders() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_windows(_: &str) -> bool {
        false
    }

    #[test]
    fn call_apps_are_recognised_by_path_or_package() {
        assert_eq!(
            call_app("C:#Program Files#Zoom#bin#Zoom.exe", &no_windows),
            Some("Zoom")
        );
        assert_eq!(
            call_app("MSTeams_8wekyb3d8bbwe", &no_windows),
            Some("Microsoft Teams")
        );
    }

    #[test]
    fn an_app_that_merely_records_is_not_a_call() {
        assert_eq!(
            call_app(
                "C:#Program Files#obs-studio#bin#64bit#obs64.exe",
                &no_windows
            ),
            None
        );
        assert_eq!(
            call_app(
                "C:#Users#me#AppData#Local#WisprFlow#Wispr Flow.exe",
                &no_windows
            ),
            None
        );
        assert_eq!(
            call_app("C:#Users#me#target#debug#memos-desktop.exe", &no_windows),
            None
        );
    }

    #[test]
    fn a_browser_is_a_call_only_with_a_meeting_open() {
        let chrome = "C:#Program Files#Google#Chrome#Application#chrome.exe";
        assert_eq!(call_app(chrome, &no_windows), None);
        let meet = |t: &str| t == "meet - ";
        assert_eq!(call_app(chrome, &meet), Some("Google Meet"));
    }

    #[test]
    fn a_short_blip_is_not_reported() {
        let mut s = Steady::new(None);
        assert_eq!(s.read(Some("Telegram")), None);
        assert_eq!(s.read(None), None);
        assert_eq!(s.read(None), None);
        assert_eq!(s.read(None), None);
    }

    #[test]
    fn a_call_that_holds_is_reported_once_and_so_is_its_end() {
        let mut s = Steady::new(None);
        assert_eq!(s.read(Some("Zoom")), None);
        assert_eq!(s.read(Some("Zoom")), None);
        assert_eq!(s.read(Some("Zoom")), Some(Some("Zoom")));
        assert_eq!(s.read(Some("Zoom")), None);
        assert_eq!(s.read(None), None);
        assert_eq!(s.read(None), None);
        assert_eq!(s.read(None), Some(None));
    }

    #[test]
    fn what_is_true_at_launch_is_not_announced() {
        let mut s = Steady::new(Some("Zoom"));
        for _ in 0..5 {
            assert_eq!(s.read(Some("Zoom")), None);
        }
    }
}
