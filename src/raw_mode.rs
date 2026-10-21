use std::io::{Write, stdout};

use anyhow::Result;
use crossterm::{
    QueueableCommand, cursor,
    terminal::{
        DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};

/// A guard that enters raw mode, hides the cursor, enables alternate screen, and disables line
/// wrapping. Dropping the guard reverses these effects, which is important for proper error
/// display.
///
/// To also exit raw mode before printing panic messages, run [`RawMode::set_panic_hook`].
pub struct RawMode;

/// Returns `true` if errors and panics should be logged to `./dumb.log`. This is for debugging in
/// case the program crashes without being able to properly display the error messages e.g. due to
/// raw mode failing to deactivate.
pub(crate) fn error_log_to_file_enabled() -> bool {
    std::env::var_os("DUMB_LOG_ERROR_TO_FILE").is_some()
}

impl RawMode {
    pub fn enter() -> Result<Self> {
        let mut stdout = stdout();
        enable_raw_mode()?;
        stdout.queue(cursor::Hide)?;
        stdout.queue(EnterAlternateScreen)?;
        stdout.queue(DisableLineWrap)?;
        Ok(Self {})
    }

    fn exit() {
        let mut stdout = stdout();
        let _ = stdout.queue(cursor::Show);
        let _ = stdout.queue(LeaveAlternateScreen);
        let _ = stdout.queue(EnableLineWrap);
        let _ = disable_raw_mode();
        let _ = stdout.flush();
    }

    /// Set a panic hook that restores the terminal, lets the default hook print
    /// the panic, and, if [`error_log_to_file_enabled`], records the panic in `dumb.log`.
    pub fn set_panic_hook() {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            Self::exit();
            default(info);
            if error_log_to_file_enabled() {
                let payload = info.payload();
                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    (*s).to_string()
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    String::from("<non-string panic payload>")
                };
                let location = info
                    .location()
                    .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                    .unwrap_or_else(|| "unknown location".to_string());
                let backtrace = std::backtrace::Backtrace::capture();
                let _ = std::fs::write(
                    "dumb.log",
                    format!("Panic at {location}:\n{msg}\n\n{backtrace}\n"),
                );
            }
        }));
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        Self::exit();
    }
}
