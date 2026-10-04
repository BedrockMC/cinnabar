use std::process::{Command, Stdio};

const SIGN_IN_PAGE: &str = "https://www.microsoft.com/link?otc=";

/// Opens a separate browser window without blocking the menu on browser startup.
pub(super) fn open(code: &str) -> bool {
    let Some(url) = sign_in_url(code) else {
        return false;
    };
    std::thread::Builder::new()
        .name("sign-in-browser".into())
        .spawn(move || {
            if !launch(&url) {
                bevy::log::warn!("Microsoft sign-in browser could not open");
            }
        })
        .is_ok()
}

fn sign_in_url(code: &str) -> Option<String> {
    (!code.is_empty()
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    .then(|| format!("{SIGN_IN_PAGE}{code}"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Browser {
    Chromium,
    Firefox,
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn browser_kind(desktop: &str) -> Option<Browser> {
    let desktop = desktop.to_ascii_lowercase();
    if desktop.contains("firefox") {
        Some(Browser::Firefox)
    } else if ["chromium", "chrome", "edge", "brave", "vivaldi"]
        .iter()
        .any(|name| desktop.contains(name))
    {
        Some(Browser::Chromium)
    } else {
        None
    }
}

fn popup_args(browser: Browser, url: &str) -> Vec<String> {
    match browser {
        Browser::Chromium => vec![format!("--app={url}"), "--window-size=520,720".into()],
        Browser::Firefox => vec!["--new-window".into(), url.into()],
    }
}

fn spawn_browser(program: &str, args: &[String]) -> bool {
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // The user's browser may share an existing session and must outlive the client.
    child.wait().is_ok_and(|status| status.success())
}

#[cfg(target_os = "linux")]
fn launch(url: &str) -> bool {
    let desktop = Command::new("xdg-settings")
        .args(["get", "default-web-browser"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok());
    let programs = linux_browsers(desktop.as_deref());
    for (program, browser) in programs {
        if spawn_browser(program, &popup_args(browser, url)) {
            return true;
        }
    }
    spawn_browser("xdg-open", &[url.into()])
}

#[cfg(any(target_os = "linux", test))]
fn linux_browsers(desktop: Option<&str>) -> Vec<(&'static str, Browser)> {
    let mut browsers = vec![
        ("chromium", Browser::Chromium),
        ("chromium-browser", Browser::Chromium),
        ("google-chrome", Browser::Chromium),
        ("google-chrome-stable", Browser::Chromium),
        ("microsoft-edge", Browser::Chromium),
        ("microsoft-edge-stable", Browser::Chromium),
        ("brave-browser", Browser::Chromium),
        ("vivaldi", Browser::Chromium),
        ("firefox", Browser::Firefox),
    ];
    if let Some(desktop) = desktop {
        let kind = browser_kind(desktop);
        let desktop = desktop.to_ascii_lowercase();
        browsers.sort_by_key(|(program, browser)| {
            let name = program
                .trim_end_matches("-stable")
                .trim_end_matches("-browser");
            (!desktop.contains(name), Some(*browser) != kind)
        });
    }
    browsers
}

#[cfg(target_os = "macos")]
fn launch(url: &str) -> bool {
    for browser in ["Google Chrome", "Microsoft Edge", "Chromium"] {
        let mut args = vec!["-na".into(), browser.into(), "--args".into()];
        args.extend(popup_args(Browser::Chromium, url));
        if spawn_browser("open", &args) {
            return true;
        }
    }
    let mut args = vec!["-a".into(), "Firefox".into(), "--args".into()];
    args.extend(popup_args(Browser::Firefox, url));
    spawn_browser("open", &args) || spawn_browser("open", &[url.into()])
}

#[cfg(target_os = "windows")]
fn launch(url: &str) -> bool {
    for program in ["msedge.exe", "chrome.exe", "firefox.exe"] {
        let browser = browser_kind(program).unwrap_or(Browser::Chromium);
        if spawn_browser(program, &popup_args(browser, url)) {
            return true;
        }
    }
    for variable in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        let Some(root) = std::env::var_os(variable) else {
            continue;
        };
        for (relative, browser) in [
            ("Microsoft/Edge/Application/msedge.exe", Browser::Chromium),
            ("Google/Chrome/Application/chrome.exe", Browser::Chromium),
            ("Mozilla Firefox/firefox.exe", Browser::Firefox),
        ] {
            let program = std::path::PathBuf::from(&root).join(relative);
            if let Some(program) = program.to_str()
                && spawn_browser(program, &popup_args(browser, url))
            {
                return true;
            }
        }
    }
    spawn_browser(
        "cmd",
        &["/C".into(), "start".into(), String::new(), url.into()],
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn launch(url: &str) -> bool {
    spawn_browser("xdg-open", &[url.into()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_prefills_the_device_code() {
        assert_eq!(
            sign_in_url("AB12-CD34"),
            Some(format!("{SIGN_IN_PAGE}AB12-CD34"))
        );
    }

    #[test]
    fn invalid_codes_never_open_a_browser() {
        for code in [
            "",
            "abc&otc=other",
            "https://example.test",
            "abc def",
            "abc\n",
            "é",
        ] {
            assert!(sign_in_url(code).is_none());
            assert!(!open(code));
        }
    }

    #[test]
    fn browser_commands_open_separate_windows() {
        let url = sign_in_url("AB1234").unwrap();
        assert_eq!(
            popup_args(Browser::Firefox, &url),
            ["--new-window", url.as_str()]
        );
        let args = popup_args(Browser::Chromium, &url);
        assert_eq!(args[0], format!("--app={url}"));
        assert!(args.iter().any(|arg| arg.starts_with("--window-size=")));
    }

    #[test]
    fn installed_default_browser_has_priority() {
        for (desktop, executable) in [
            ("org.mozilla.firefox.desktop", "firefox"),
            ("google-chrome.desktop", "google-chrome"),
            ("microsoft-edge.desktop", "microsoft-edge"),
            ("brave-browser.desktop", "brave-browser"),
        ] {
            assert_eq!(linux_browsers(Some(desktop)).first().unwrap().0, executable);
        }
    }
}
