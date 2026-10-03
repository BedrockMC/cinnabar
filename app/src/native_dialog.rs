//! Native consent and status dialogs for the pre-window lifecycle flows, via each OS's stock tooling.

use std::{
    io::{BufRead, IsTerminal, Write},
    process::{Command, Stdio},
};

pub(crate) trait Prompter {
    /// Asks a yes/no question; `false` on decline or when no prompt surface exists.
    fn confirm(&self, title: &str, body: &str) -> bool;
    /// Best-effort, non-blocking progress message.
    fn info(&self, title: &str, body: &str);
    /// Blocking error message.
    fn alert(&self, title: &str, body: &str);
}

pub(crate) struct NativePrompter;

#[derive(Clone, Copy)]
enum Kind {
    Confirm,
    Info,
    Alert,
}

impl Prompter for NativePrompter {
    fn confirm(&self, title: &str, body: &str) -> bool {
        for argv in commands(Kind::Confirm, title, body) {
            if let Some(accepted) = run(&argv) {
                return accepted;
            }
        }
        tty_confirm(title, body)
    }

    fn info(&self, title: &str, body: &str) {
        let surfaces = commands(Kind::Info, title, body);
        let title = title.to_owned();
        let body = body.to_owned();
        std::thread::spawn(move || {
            if !show_message(surfaces) {
                eprintln!("{title}: {body}");
            }
        });
    }

    fn alert(&self, title: &str, body: &str) {
        if show_message(commands(Kind::Alert, title, body)) {
            return;
        }
        eprintln!("{title}: {body}");
    }
}

/// Tries message tools in order, requiring successful delivery before stopping.
fn show_message(commands: Vec<Vec<String>>) -> bool {
    commands.iter().any(|argv| run(argv) == Some(true))
}

/// Waits for a tracked tool; `None` means it could not be launched.
fn run(argv: &[String]) -> Option<bool> {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = crate::lifecycle::children::spawn(&mut command).ok()?;
    Some(child.wait().is_some_and(|status| status.success()))
}

fn tty_confirm(title: &str, body: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("{title}\n{body}\nType `yes` to continue: ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).is_ok() && line.trim().eq_ignore_ascii_case("yes")
}

fn commands(kind: Kind, title: &str, body: &str) -> Vec<Vec<String>> {
    let owned = |parts: &[&str]| {
        parts
            .iter()
            .map(|part| (*part).to_owned())
            .collect::<Vec<_>>()
    };
    if cfg!(target_os = "macos") {
        let (t, b) = (applescript_quote(title), applescript_quote(body));
        let script = match kind {
            Kind::Confirm => format!(
                "display dialog {b} with title {t} buttons {{\"Quit\", \"Continue\"}} default button \"Continue\" cancel button \"Quit\""
            ),
            Kind::Info => format!("display notification {b} with title {t}"),
            Kind::Alert => format!(
                "display dialog {b} with title {t} buttons {{\"OK\"}} default button \"OK\" with icon stop"
            ),
        };
        return vec![vec!["osascript".into(), "-e".into(), script]];
    }
    if cfg!(target_os = "windows") {
        let buttons = match kind {
            Kind::Confirm => "OKCancel",
            Kind::Alert => "OK",
            Kind::Info => return Vec::new(),
        };
        let icon = if matches!(kind, Kind::Alert) {
            "Error"
        } else {
            "Information"
        };
        let script = format!(
            "Add-Type -AssemblyName PresentationFramework; if ([System.Windows.MessageBox]::Show({}, {}, '{buttons}', '{icon}') -ne 'OK') {{ exit 1 }}",
            powershell_quote(body),
            powershell_quote(title),
        );
        return vec![vec![
            "powershell".into(),
            "-NoProfile".into(),
            "-Command".into(),
            script,
        ]];
    }
    match kind {
        Kind::Confirm => vec![
            vec![
                "zenity".into(),
                "--question".into(),
                "--width=520".into(),
                "--ok-label=Continue".into(),
                "--cancel-label=Quit".into(),
                format!("--title={title}"),
                format!("--text={body}"),
            ],
            vec![
                "kdialog".into(),
                "--title".into(),
                title.into(),
                "--yesno".into(),
                body.into(),
            ],
        ],
        Kind::Alert => vec![
            vec![
                "zenity".into(),
                "--error".into(),
                format!("--title={title}"),
                format!("--text={body}"),
            ],
            vec![
                "kdialog".into(),
                "--title".into(),
                title.into(),
                "--error".into(),
                body.into(),
            ],
        ],
        Kind::Info => vec![owned(&["notify-send", title, body])],
    }
}

fn applescript_quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

fn powershell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn review_failed_message_tool_allows_the_next_fallback() {
        let path =
            std::env::temp_dir().join(format!("cinnabar-dialog-fallback-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let first = vec!["sh".into(), "-c".into(), "exit 1".into()];
        let second = vec![
            "sh".into(),
            "-c".into(),
            "touch \"$1\"".into(),
            "test".into(),
            path.to_string_lossy().into_owned(),
        ];
        assert!(show_message(vec![first, second]));
        let delivered = path.exists();
        let _ = std::fs::remove_file(path);
        assert!(delivered);
    }

    #[test]
    fn applescript_quoting_escapes_quotes_backslashes_and_newlines() {
        assert_eq!(applescript_quote("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }

    #[test]
    fn powershell_quoting_doubles_single_quotes() {
        assert_eq!(powershell_quote("it's"), "'it''s'");
    }

    #[test]
    fn every_kind_has_a_command_on_the_supported_platforms() {
        for kind in [Kind::Confirm, Kind::Alert] {
            assert!(!commands(kind, "t", "b").is_empty());
        }
    }
}
