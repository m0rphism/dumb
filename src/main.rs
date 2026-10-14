pub mod args;
pub mod config;
pub mod file_tree;
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

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{Event as TermEvent, KeyCode, KeyEventKind};
use crossterm::terminal;

use crate::{
    args::Args,
    config::{Config, SortBy},
    file_tree::FileTree,
    raw_mode::RawMode,
    render::{render_state, sorted_entries},
    scan::scan,
    theme::Theme,
};

#[derive(Clone, Debug)]
pub enum Event {
    Term(TermEvent),
    Redraw,
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
}

impl State {
    pub fn cur_subdir_entry_ix(&self) -> usize {
        *self.cur_subdir_entry_ixs.last().unwrap()
    }
    pub fn cur_subdir_entry_ix_mut(&mut self) -> &mut usize {
        self.cur_subdir_entry_ixs.last_mut().unwrap()
    }
}

fn main() {
    if let Err(e) = run() {
        println!("{e}");
        std::fs::write("dumb.log", e.to_string()).unwrap();
    }
}
fn run() -> Result<()> {
    let args = Args::parse();

    let config_dir = dirs::config_dir().expect(
        "The config directory could not be determined. Is your $HOME variable empty for some reason?"
    ).join("dumb");

    let config = {
        let config_path = config_dir.join("config.toml");
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
        match config.theme {
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
                    .context(format!("Failed to parse config file at {theme_path:?}."))?
            }
        }
    };

    let target_path = args
        .target
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    let (sender, receiver) = channel();
    let (done_sender, done_receiver) = channel();

    let file_tree = Arc::new(Mutex::new(FileTree::new(&target_path)));

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
    };

    let mut stdout = stdout();
    let _raw_mode = RawMode::enter()?;
    render_state(&mut stdout, &mut state, &theme)?;

    for event in receiver {
        // println!("Received: {event:?}");
        match event {
            Event::Term(event) => match event {
                TermEvent::Key(key_event) => {
                    if key_event.kind != KeyEventKind::Release {
                        match key_event.code {
                            KeyCode::Char('q') => break,
                            KeyCode::Char('j') | KeyCode::Down => {
                                let num_files_in_cur_subdir = state
                                    .file_tree
                                    .lock()
                                    .unwrap()
                                    .get(&state.cur_subdir)
                                    .map(|node| node.children.len())
                                    .unwrap_or(0);
                                let ix = state.cur_subdir_entry_ix_mut();
                                if *ix + 1 < num_files_in_cur_subdir {
                                    *ix += 1;
                                    render_state(&mut stdout, &mut state, &theme)?;
                                }
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                let ix = state.cur_subdir_entry_ix_mut();
                                if *ix > 0 {
                                    *ix -= 1;
                                    render_state(&mut stdout, &mut state, &theme)?;
                                }
                            }
                            KeyCode::Char('h') | KeyCode::Left => {
                                if let Some(p) = state.cur_subdir.parent() {
                                    state.cur_subdir = p.to_path_buf();
                                    state.cur_subdir_entry_ixs.pop();
                                    if state.cur_subdir_entry_ixs.is_empty() {
                                        state.cur_subdir_entry_ixs.push(0)
                                    }
                                    render_state(&mut stdout, &mut state, &theme)?;
                                }
                            }
                            KeyCode::Char('l') | KeyCode::Right => {
                                let redraw = {
                                    let ix = state.cur_subdir_entry_ix();
                                    let file_tree = state.file_tree.lock().unwrap();
                                    let entries = sorted_entries(&file_tree, &state);
                                    if let Some((cur_name, _node, _inode, _is_dir @ true)) =
                                        entries.get(ix)
                                    {
                                        state.cur_subdir = state.cur_subdir.join(cur_name);
                                        true
                                    } else {
                                        false
                                    }
                                };
                                if redraw {
                                    state.cur_subdir_entry_ixs.push(0);
                                    render_state(&mut stdout, &mut state, &theme)?;
                                }
                            }
                            KeyCode::Char('g') | KeyCode::Home => {
                                let ix = state.cur_subdir_entry_ix_mut();
                                if *ix > 0 {
                                    *ix = 0;
                                    render_state(&mut stdout, &mut state, &theme)?;
                                }
                            }
                            KeyCode::Char('G') | KeyCode::End => {
                                let num_files_in_cur_subdir = state
                                    .file_tree
                                    .lock()
                                    .unwrap()
                                    .get(&state.cur_subdir)
                                    .map(|node| node.children.len())
                                    .unwrap_or(0);
                                let ix = state.cur_subdir_entry_ix_mut();

                                if *ix + 1 < num_files_in_cur_subdir {
                                    *ix = num_files_in_cur_subdir.saturating_sub(1);
                                    render_state(&mut stdout, &mut state, &theme)?;
                                }
                            }
                            KeyCode::Char('s') => {
                                state.sort_by = state.sort_by.next();
                                render_state(&mut stdout, &mut state, &theme)?;
                            }
                            KeyCode::Char('S') => {
                                state.sort_dirs_before_files = !state.sort_dirs_before_files;
                                render_state(&mut stdout, &mut state, &theme)?;
                            }
                            _ => (),
                        }
                    }
                }
                TermEvent::Resize(w, h) => {
                    state.term_size = [w, h];
                    render_state(&mut stdout, &mut state, &theme)?;
                }
                _ => (),
            },
            Event::Redraw => {
                render_state(&mut stdout, &mut state, &theme)?;
            }
        };
    }

    Ok(())
}
