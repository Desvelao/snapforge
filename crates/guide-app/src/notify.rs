//! Desktop notifications through a user-configured command (see `guide_core::parse_command`).
//!
//! Automatic notifications are best effort: the copy or save already succeeded, so a missing
//! program is not worth an error. The settings page's test button reports the outcome instead.

use std::{
    io::Read,
    process::{Command, Stdio},
    sync::mpsc::{channel, Receiver},
    thread,
};

use guide_storage::NotificationSettings;

/// What a worker thread needs to send a notification; cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct NotifyCfg {
    pub enabled: bool,
    pub command: String,
}

impl From<&NotificationSettings> for NotifyCfg {
    fn from(s: &NotificationSettings) -> Self {
        Self {
            enabled: s.enabled,
            command: s.command.clone(),
        }
    }
}

/// Sends a notification and returns at once; does nothing when notifications are off.
pub fn send(cfg: &NotifyCfg, title: &str, message: &str) {
    if !cfg.enabled {
        return;
    }
    let Ok(argv) = guide_core::parse_command(&cfg.command, title, message) else {
        return;
    };
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(mut child) = cmd.spawn() {
        thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Runs the command for the settings page's test button, even when notifications are switched
/// off. The result arrives on the receiver: `Ok` when the command exited with success.
pub fn test(command: &str, title: &str, message: &str) -> Receiver<Result<(), String>> {
    let (tx, rx) = channel();
    let argv = guide_core::parse_command(command, title, message);
    thread::spawn(move || {
        let result = argv.and_then(|argv| run_and_wait(&argv));
        let _ = tx.send(result);
    });
    rx
}

fn run_and_wait(argv: &[String]) -> Result<(), String> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", argv[0]))?;
    let mut err = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.by_ref().take(2000).read_to_string(&mut err);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        let err = err.trim();
        Err(if err.is_empty() {
            format!("{} exited with {status}", argv[0])
        } else {
            err.to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(rx: Receiver<Result<(), String>>) -> Result<(), String> {
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("the test finishes")
    }

    #[test]
    fn a_command_that_succeeds_passes_the_test() {
        assert_eq!(wait(test("true {title} {message}", "t", "m")), Ok(()));
    }

    #[test]
    fn a_missing_program_fails_the_test_and_names_it() {
        let e = wait(test("snapforge-no-such-program {message}", "t", "m")).unwrap_err();
        assert!(e.contains("snapforge-no-such-program"), "{e}");
    }

    #[test]
    fn a_failing_command_reports_what_it_printed() {
        let e = wait(test("sh -c 'echo boom >&2; exit 3' x {message}", "t", "m")).unwrap_err();
        assert!(e.contains("boom"), "{e}");
    }

    #[test]
    fn an_unparsable_command_fails_the_test() {
        assert!(wait(test("echo 'oops", "t", "m")).is_err());
    }

    #[test]
    fn the_message_reaches_the_program_unchanged() {
        let dir = std::env::temp_dir().join(format!("snapforge-notify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out.txt");
        let cmd = format!(
            "sh -c 'printf \"%s|%s\" \"$0\" \"$1\" > \"$2\"' {{title}} {{message}} {}",
            out.display()
        );
        wait(test(&cmd, "T t", "a; b $(c)")).unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "T t|a; b $(c)");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_is_run_when_notifications_are_off() {
        // Would fail loudly if it ran: the program does not exist and `send` would try it.
        send(
            &NotifyCfg {
                enabled: false,
                command: "snapforge-no-such-program".into(),
            },
            "t",
            "m",
        );
    }
}
