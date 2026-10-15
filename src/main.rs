pub mod args;
pub mod config;
pub mod file_tree;
pub mod motion;
pub mod raw_mode;
pub mod render;
pub mod scan;
pub mod theme;

use std::{
    io::stdout,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc::channel},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use crossterm::{
    event::{Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};

use crate::{
    args::Args,
    config::{Config, SortBy},
    file_tree::FileTree,
    motion::Motion,
    raw_mode::RawMode,
    render::{render_state, sorted_entries},
    scan::scan,
    theme::Theme,
};

fn main() {
    if let Err(e) = run() {
        let err_str = format!("{e:?}");
        eprintln!("{}", err_str);
        std::fs::write("dumb.log", &err_str).unwrap();
    }
}

fn run() -> Result<()> {
    let args = Args::parse();

    let (config, theme) = load_config_and_theme(&args.config)?;

    let target_path = args
        .target
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    let file_tree = Arc::new(Mutex::new(FileTree::new(&target_path)));

    let (sender, receiver) = channel();
    let (done_sender, done_receiver) = channel();

    // Spawn thread that scans the `target_path`.
    let _scan_handle = std::thread::spawn({
        let file_tree = file_tree.clone();
        let target_path = target_path.clone();
        move || -> Result<()> {
            scan(&target_path, |mut scan_event| -> Result<()> {
                let mut file_tree = file_tree.lock().unwrap();
                scan_event
                    .map_path_bufs(|p| *p = p.strip_prefix(&target_path).unwrap().to_path_buf());
                file_tree.handle_scan_event(scan_event);
                Ok(())
            })?;
            done_sender.send(())?;
            Ok(())
        }
    });

    // Spawn thread that periodically sends redraw events until scan completed.
    let _redraw_handle = std::thread::spawn({
        let sender = sender.clone();
        move || -> Result<()> {
            while done_receiver.try_recv().is_err() {
                sender.send(Event::Redraw)?;
                std::thread::sleep(Duration::from_millis(500));
            }
            sender.send(Event::Redraw)?;
            Ok(())
        }
    });

    // Spawn thread that captures terminal events.
    let _input_handle = std::thread::spawn({
        let sender = sender.clone();
        move || {
            while let Ok(e) = crossterm::event::read() {
                if sender.send(Event::Term(e)).is_err() {
                    break;
                }
            }
        }
    });

    let term_size = {
        let (w, h) = terminal::size()?;
        [w, h]
    };

    // cur_subdir_entries: target_path
    //     .read_dir()?
    //     .filter_map(|e| e.ok().map(|e| e.file_name()))
    //     .collect(),
    // state.cur_subdir_entries.sort();
    let mut state = State {
        file_tree,
        cur_subdir: PathBuf::new(),
        cur_subdir_entry_ixs: vec![0],
        target_path,
        term_size,
        frame: 0,
        sort_by: config.default_sort_mode,
        sort_dirs_before_files: true,
        display_mode: DisplayMode::FileTree,
    };

    let mut stdout = stdout();
    let _raw_mode = RawMode::enter()?;
    render_state(&mut stdout, &mut state, &theme)?;

    for event in receiver {
        // println!("Received: {event:?}");
        let redraw = match event {
            Event::Term(event) => match event {
                TermEvent::Key(key_event) => match handle_key_event(&mut state, key_event) {
                    Some(redraw) => redraw,
                    None => break,
                },
                TermEvent::Resize(w, h) => {
                    if state.term_size == [w, h] {
                        false
                    } else {
                        state.term_size = [w, h];
                        true
                    }
                }
                _ => false,
            },
            Event::Redraw => true,
        };
        if redraw {
            render_state(&mut stdout, &mut state, &theme)?;
        }
    }

    Ok(())
}

/// Returns `None` if the program should quit, or `Some(should_redraw)`.
fn handle_key_event(state: &mut State, key_event: KeyEvent) -> Option<bool> {
    let redraw = if key_event.kind != KeyEventKind::Release {
        match &mut state.display_mode {
            DisplayMode::FileTree => match key_event.code {
                KeyCode::Char('q') => return None,
                KeyCode::Char('j') | KeyCode::Down => state.apply_motion(Motion::Down),
                KeyCode::Char('k') | KeyCode::Up => state.apply_motion(Motion::Up),
                KeyCode::Char('h') | KeyCode::Left => {
                    if let Some(p) = state.cur_subdir.parent() {
                        state.cur_subdir = p.to_path_buf();
                        state.cur_subdir_entry_ixs.pop();
                        if state.cur_subdir_entry_ixs.is_empty() {
                            state.cur_subdir_entry_ixs.push(0)
                        }
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    let redraw = {
                        let ix = state.cur_subdir_entry_ix();
                        let file_tree = state.file_tree.lock().unwrap();
                        let entries = sorted_entries(&file_tree, &state);
                        if let Some((cur_name, _node, _inode, _is_dir @ true)) = entries.get(ix) {
                            state.cur_subdir = state.cur_subdir.join(cur_name);
                            true
                        } else {
                            false
                        }
                    };
                    if redraw {
                        state.cur_subdir_entry_ixs.push(0);
                    }
                    redraw
                }
                KeyCode::Char('g') | KeyCode::Home => state.apply_motion(Motion::First),
                KeyCode::Char('G') | KeyCode::End => state.apply_motion(Motion::Last),
                KeyCode::Char('f') if key_event.modifiers == KeyModifiers::CONTROL => {
                    state.apply_motion(Motion::PageDown)
                }
                KeyCode::PageDown => state.apply_motion(Motion::PageDown),
                KeyCode::Char('b') if key_event.modifiers == KeyModifiers::CONTROL => {
                    state.apply_motion(Motion::PageUp)
                }
                KeyCode::PageUp => state.apply_motion(Motion::PageUp),
                KeyCode::Char('d') if key_event.modifiers == KeyModifiers::CONTROL => {
                    state.apply_motion(Motion::HalfPageDown)
                }
                KeyCode::Char('u') if key_event.modifiers == KeyModifiers::CONTROL => {
                    state.apply_motion(Motion::HalfPageUp)
                }
                KeyCode::Char('s') => {
                    state.sort_by = state.sort_by.next();
                    true
                }
                KeyCode::Char('S') => {
                    state.sort_dirs_before_files = !state.sort_dirs_before_files;
                    true
                }
                KeyCode::Char('e') => {
                    let file_tree = state.file_tree.lock().unwrap();
                    let children = sorted_entries(&file_tree, &state);
                    if let Some((name, _, _, _)) = children.get(state.cur_subdir_entry_ix()) {
                        let cur_path = state.cur_subdir.join(name);
                        state.display_mode = DisplayMode::ErrorView(ErrorView {
                            errors: file_tree
                                .errors
                                .iter()
                                .filter(|(path, _)| path.strip_prefix(&cur_path).is_ok())
                                .map(|(path, message)| ErrorInfo {
                                    path: path.to_string_lossy().to_string(),
                                    message: message.clone(),
                                })
                                .collect(),
                            mode: ErrorViewMode::CurEntryErrors,
                            cur_error_ix: 0,
                        });
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Char('E') => {
                    state.display_mode = DisplayMode::ErrorView(ErrorView {
                        errors: state
                            .file_tree
                            .lock()
                            .unwrap()
                            .errors
                            .iter()
                            .map(|(path, message)| ErrorInfo {
                                path: path.to_string_lossy().to_string(),
                                message: message.clone(),
                            })
                            .collect(),
                        mode: ErrorViewMode::AllErrors,
                        cur_error_ix: 0,
                    });
                    true
                }
                _ => false,
            },
            DisplayMode::ErrorView(error_view) => match key_event.code {
                KeyCode::Char('q') | KeyCode::Char('e') | KeyCode::Char('E') => {
                    state.display_mode = DisplayMode::FileTree;
                    true
                }
                KeyCode::Char('j') | KeyCode::Down => error_view.apply_motion(Motion::Down),
                KeyCode::Char('k') | KeyCode::Up => error_view.apply_motion(Motion::Up),
                KeyCode::Char('g') | KeyCode::Home => error_view.apply_motion(Motion::First),
                KeyCode::Char('G') | KeyCode::End => error_view.apply_motion(Motion::Last),
                KeyCode::Char('f') if key_event.modifiers == KeyModifiers::CONTROL => {
                    error_view.apply_motion(Motion::PageDown)
                }
                KeyCode::PageDown => error_view.apply_motion(Motion::PageDown),
                KeyCode::Char('b') if key_event.modifiers == KeyModifiers::CONTROL => {
                    error_view.apply_motion(Motion::PageUp)
                }
                KeyCode::PageUp => error_view.apply_motion(Motion::PageUp),
                KeyCode::Char('d') if key_event.modifiers == KeyModifiers::CONTROL => {
                    error_view.apply_motion(Motion::HalfPageDown)
                }
                KeyCode::Char('u') if key_event.modifiers == KeyModifiers::CONTROL => {
                    error_view.apply_motion(Motion::HalfPageUp)
                }
                _ => false,
            },
        }
    } else {
        false
    };
    Some(redraw)
}

fn load_config_and_theme(config_path: &Option<PathBuf>) -> Result<(Config, Theme)> {
    let (config_dir, config_path) = if let Some(config_path) = config_path {
        let config_path = config_path
            .canonicalize()
            .context("Failed to canonicalize config path '{config_path:?}'")?;
        let config_dir = config_path
            .parent()
            .unwrap_or_else(|| panic!("Config path '{config_path:?}' has no parent directory."));
        if !config_path.exists() {
            bail!("Config path '{config_path:?}' does not exist.")
        }
        (config_dir.to_path_buf(), config_path.clone())
    } else {
        let config_dir = dirs::config_dir().expect(
            "The config directory could not be determined. Is your $HOME variable empty for some reason?"
        ).join("dumb");
        let config_path = config_dir.join("config.toml");
        (config_dir, config_path)
    };

    let config = {
        if !config_dir.exists() {
            Config::default()
        } else {
            let content = std::fs::read_to_string(&config_path)
                .context(format!("Couldn't read config file at {config_path:?}."))?;
            toml::from_str(&content)
                .context(format!("Failed to parse config file at {config_path:?}."))?
        }
    };

    let theme = {
        match &config.theme {
            None => Theme::default(),
            Some(theme_path) => {
                let theme_path = if theme_path.is_relative() {
                    config_dir.join(theme_path)
                } else {
                    theme_path.clone()
                };
                let content = std::fs::read_to_string(&theme_path)
                    .context(format!("Couldn't read theme file at {theme_path:?}."))?;
                toml::from_str(&content)
                    .context(format!("Failed to parse theme file at {theme_path:?}."))?
            }
        }
    };

    Ok((config, theme))
}

#[derive(Clone, Debug)]
pub enum Event {
    Term(TermEvent),
    Redraw,
}

#[derive(Debug, Clone)]
pub enum DisplayMode {
    FileTree,
    ErrorView(ErrorView),
}

#[derive(Debug, Clone)]
pub struct ErrorView {
    pub errors: Vec<ErrorInfo>,
    pub mode: ErrorViewMode,
    pub cur_error_ix: usize,
}

impl ErrorView {
    pub fn apply_motion(&mut self, motion: Motion) -> bool {
        motion.apply_check(&mut self.cur_error_ix, self.errors.len(), 10)
    }
}

#[derive(Debug, Clone)]
pub struct ErrorInfo {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorViewMode {
    AllErrors,
    CurEntryErrors,
}

#[derive(Debug)]
pub struct State {
    pub target_path: PathBuf,
    pub file_tree: Arc<Mutex<FileTree>>,
    pub cur_subdir: PathBuf,
    pub cur_subdir_entry_ixs: Vec<usize>,
    pub term_size: [u16; 2],
    pub frame: usize,
    pub sort_by: SortBy,
    pub sort_dirs_before_files: bool,
    pub display_mode: DisplayMode,
}

impl State {
    pub fn cur_subdir_entry_ix(&self) -> usize {
        *self.cur_subdir_entry_ixs.last().unwrap()
    }
    pub fn cur_subdir_entry_ix_mut(&mut self) -> &mut usize {
        self.cur_subdir_entry_ixs.last_mut().unwrap()
    }
    pub fn cur_subdir_entry_len(&self) -> usize {
        self.file_tree
            .lock()
            .unwrap()
            .get(&self.cur_subdir)
            .map(|node| node.children.len())
            .unwrap_or(0)
    }
    /// Apply the motion to the file list and return true iff the motion actually changed the
    /// selected entry.
    pub fn apply_motion(&mut self, motion: Motion) -> bool {
        let num_entries = self.cur_subdir_entry_len();
        let height = self.term_size[1] as usize - 3;
        let i = self.cur_subdir_entry_ix_mut();
        motion.apply_check(i, num_entries, height)
    }
}
