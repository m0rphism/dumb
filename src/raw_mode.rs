use std::io::{Write, stdout};

use anyhow::Result;
use crossterm::{
    QueueableCommand, cursor,
    terminal::{
        DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};

pub struct RawMode {}

impl RawMode {
    pub fn enter() -> Result<Self> {
        let mut stdout = stdout();
        enable_raw_mode()?;
        stdout.queue(cursor::Hide)?;
        stdout.queue(EnterAlternateScreen)?;
        stdout.queue(DisableLineWrap)?;
        Ok(Self {})
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let mut stdout = stdout();
        stdout.queue(cursor::Show).unwrap();
        stdout.queue(LeaveAlternateScreen).unwrap();
        stdout.queue(EnableLineWrap).unwrap();
        disable_raw_mode().unwrap();
        stdout.flush().unwrap();
    }
}
