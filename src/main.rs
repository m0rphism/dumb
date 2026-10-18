pub mod args;
pub mod config;
pub mod file_tree;
pub mod frame;
pub mod help;
pub mod layout;
pub mod motion;
pub mod raw_mode;
pub mod render;
pub mod scan;
pub mod theme;

use std::{
    ffi::OsString,
    io::stdout,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
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
    config::{Config, HeaderLayout, SizeMetric, SortBy},
    file_tree::{FileState, FileTree},
    help::{HELP, HelpMeta},
    layout::Layout,
    motion::Motion,
    raw_mode::RawMode,
    render::{fixed_columns_width, render_state, sorted_entries},
    scan::{Inode, scan},
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
            scan(
                &target_path,
                |mut scan_event| -> Result<()> {
                    let mut file_tree = file_tree.lock().unwrap();
                    scan_event.map_path_bufs(|p| {
                        *p = p.strip_prefix(&target_path).unwrap().to_path_buf()
                    });
                    file_tree.handle_scan_event(scan_event);
                    Ok(())
                },
                |p: &Path| {
                    let rel = p.strip_prefix(&target_path).unwrap_or(p);
                    file_tree.lock().unwrap().is_deleted_path(rel)
                },
            )?;
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
        file_view: FileView::new(&config),
        target_path,
        term_size,
        layout: Layout::new(term_size, 1),
        frame: 0,
        header_layout: config.default_header_layout,
        display_mode: DisplayMode::FileTree,
        help_meta: HelpMeta::new(),
        status: None,
    };

    let mut stdout = stdout();
    let _raw_mode = RawMode::enter()?;
    state.layout = render_state(&mut stdout, &state, &theme)?;
    state.frame += 1;

    for event in receiver {
        // println!("Received: {event:?}");
        let redraw = match event {
            Event::Term(event) => match event {
                TermEvent::Key(key_event) => match handle_key_event(&mut state, &config, key_event)
                {
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
            state.layout = render_state(&mut stdout, &state, &theme)?;
            state.frame += 1;
        }
    }

    Ok(())
}

/// Returns `None` if the program should quit, or `Some(should_redraw)`.
fn handle_key_event(state: &mut State, config: &Config, key_event: KeyEvent) -> Option<bool> {
    let redraw = if key_event.kind != KeyEventKind::Release {
        // Any key press clears the status message.
        state.status = None;

        let layout = state.layout;
        match &mut state.display_mode {
            DisplayMode::FileTree => match key_event.code {
                KeyCode::Char('q') => return None,
                KeyCode::Char('j') | KeyCode::Down => state.file_view.apply_motion(
                    Motion::Down,
                    state.cur_subdir_entry_len(),
                    &layout,
                ),
                KeyCode::Char('k') | KeyCode::Up => {
                    state
                        .file_view
                        .apply_motion(Motion::Up, state.cur_subdir_entry_len(), &layout)
                }
                KeyCode::Char('h') | KeyCode::Left => state.file_view.leave_subdir(),
                KeyCode::Char('l') | KeyCode::Right => {
                    let file_tree = state.file_tree.lock().unwrap();
                    let entries = sorted_entries(&file_tree, state);
                    match entries.get(state.file_view.cur_entry_ix()) {
                        Some((cur_name, _, _, true, _)) => {
                            state.file_view.enter_subdir(cur_name);
                            true
                        }
                        _ => false,
                    }
                }
                KeyCode::Char('g') | KeyCode::Home => state.file_view.apply_motion(
                    Motion::First,
                    state.cur_subdir_entry_len(),
                    &layout,
                ),
                KeyCode::Char('G') | KeyCode::End => state.file_view.apply_motion(
                    Motion::Last,
                    state.cur_subdir_entry_len(),
                    &layout,
                ),
                KeyCode::Char('f') if key_event.modifiers == KeyModifiers::CONTROL => state
                    .file_view
                    .apply_motion(Motion::PageDown, state.cur_subdir_entry_len(), &layout),
                KeyCode::PageDown => state.file_view.apply_motion(
                    Motion::PageDown,
                    state.cur_subdir_entry_len(),
                    &layout,
                ),
                KeyCode::Char('b') if key_event.modifiers == KeyModifiers::CONTROL => state
                    .file_view
                    .apply_motion(Motion::PageUp, state.cur_subdir_entry_len(), &layout),
                KeyCode::PageUp => state.file_view.apply_motion(
                    Motion::PageUp,
                    state.cur_subdir_entry_len(),
                    &layout,
                ),
                KeyCode::Char('d') if key_event.modifiers == KeyModifiers::CONTROL => state
                    .file_view
                    .apply_motion(Motion::HalfPageDown, state.cur_subdir_entry_len(), &layout),
                KeyCode::Char('u') if key_event.modifiers == KeyModifiers::CONTROL => state
                    .file_view
                    .apply_motion(Motion::HalfPageUp, state.cur_subdir_entry_len(), &layout),
                KeyCode::Char('s') => {
                    state.file_view.sort_by = state.file_view.sort_by.next();
                    true
                }
                KeyCode::Char('S') => {
                    state.file_view.sort_dirs_before_files =
                        !state.file_view.sort_dirs_before_files;
                    true
                }
                KeyCode::Char('L') => {
                    state.header_layout = state.header_layout.next();
                    true
                }
                KeyCode::Char('m') => {
                    state.file_view.size_metric = state.file_view.size_metric.next();
                    true
                }
                KeyCode::Char('c') => {
                    state.file_view.show_size_cur_column = !state.file_view.show_size_cur_column;
                    true
                }
                KeyCode::Char('+') => state.file_view.adjust_name_col_width(
                    1,
                    state.term_size[0] as i64,
                    state.is_scan_done(),
                ),
                KeyCode::Char('-') => state.file_view.adjust_name_col_width(
                    -1,
                    state.term_size[0] as i64,
                    state.is_scan_done(),
                ),
                KeyCode::Char('e') => {
                    let file_tree = state.file_tree.lock().unwrap();
                    let children = sorted_entries(&file_tree, state);
                    if let Some((name, _, _, _, _)) = children.get(state.file_view.cur_entry_ix()) {
                        let cur_path = state.file_view.cur_subdir.join(name);
                        let errors: Vec<_> = file_tree
                            .errors
                            .iter()
                            .filter(|(path, _)| path.strip_prefix(&cur_path).is_ok())
                            .map(|(path, message)| ErrorInfo {
                                path: path.to_string_lossy().to_string(),
                                message: message.clone(),
                            })
                            .collect();
                        if !errors.is_empty() {
                            state.display_mode = DisplayMode::ErrorView(ErrorView {
                                errors,
                                mode: ErrorViewMode::CurEntryErrors,
                                cur_error_ix: 0,
                            });
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                KeyCode::Char('E') => {
                    let errors: Vec<_> = state
                        .file_tree
                        .lock()
                        .unwrap()
                        .errors
                        .iter()
                        .map(|(path, message)| ErrorInfo {
                            path: path.to_string_lossy().to_string(),
                            message: message.clone(),
                        })
                        .collect();
                    if !errors.is_empty() {
                        state.display_mode = DisplayMode::ErrorView(ErrorView {
                            errors,
                            mode: ErrorViewMode::AllErrors,
                            cur_error_ix: 0,
                        });
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Char('d') if config.enable_delete_binding => {
                    if let Some(rel) = state.cur_entry_path() {
                        if config.require_delete_confirmation {
                            state.display_mode = DisplayMode::DeleteDialog(rel);
                        } else {
                            state.try_delete(&rel);
                        }
                    }
                    true
                }
                KeyCode::Char('?') => {
                    state.display_mode = DisplayMode::HelpView(HelpView::default());
                    true
                }
                _ => false,
            },
            DisplayMode::DeleteDialog(target) => {
                let target = target.clone();
                state.display_mode = DisplayMode::FileTree;
                if let KeyCode::Char('y' | 'Y') = key_event.code {
                    state.try_delete(&target);
                }
                true
            }
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
                KeyCode::Char('?') => {
                    state.display_mode = DisplayMode::HelpView(HelpView::default());
                    true
                }
                _ => false,
            },
            DisplayMode::HelpView(help_view) => match key_event.code {
                KeyCode::Char('q') | KeyCode::Char('?') => {
                    state.display_mode = DisplayMode::FileTree;
                    true
                }
                KeyCode::Char('j') | KeyCode::Down => help_view.apply_motion(Motion::Down, &layout),
                KeyCode::Char('k') | KeyCode::Up => help_view.apply_motion(Motion::Up, &layout),
                KeyCode::Char('g') | KeyCode::Home => {
                    help_view.apply_motion(Motion::First, &layout)
                }
                KeyCode::Char('G') | KeyCode::End => help_view.apply_motion(Motion::Last, &layout),
                KeyCode::Char('f') if key_event.modifiers == KeyModifiers::CONTROL => {
                    help_view.apply_motion(Motion::PageDown, &layout)
                }
                KeyCode::PageDown => help_view.apply_motion(Motion::PageDown, &layout),
                KeyCode::Char('b') if key_event.modifiers == KeyModifiers::CONTROL => {
                    help_view.apply_motion(Motion::PageUp, &layout)
                }
                KeyCode::PageUp => help_view.apply_motion(Motion::PageUp, &layout),
                KeyCode::Char('d') if key_event.modifiers == KeyModifiers::CONTROL => {
                    help_view.apply_motion(Motion::HalfPageDown, &layout)
                }
                KeyCode::Char('u') if key_event.modifiers == KeyModifiers::CONTROL => {
                    help_view.apply_motion(Motion::HalfPageUp, &layout)
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
    HelpView(HelpView),
    /// Target-relative path of the entry to delete.
    DeleteDialog(PathBuf),
}

/// The state of the file tree view: the currently viewed subdirectory, the selection within it,
/// and the view's display options.
#[derive(Debug, Clone)]
pub struct FileView {
    /// The current subdirectory selected in the file tree view. Relative to [`State::target_path`].
    pub cur_subdir: PathBuf,
    /// The stack of file tree entry indexes. The last entry of the stack represents the index for
    /// the currently selected file tree entry in [`FileView::cur_subdir`].
    pub entry_ixs: Vec<usize>,
    /// How the file tree view is sorted.
    pub sort_by: SortBy,
    /// The size-metric with which the entries of the file tree are displayed.
    pub size_metric: SizeMetric,
    /// Whether the file tree view should sort directories before files (only relevant for [`SortBy::Name`].
    pub sort_dirs_before_files: bool,
    /// Whether to display the "Size Cur" column in the file tree view (only relevant if the file
    /// tree view is currently in a subdirectory of [`State::target_path`].
    pub show_size_cur_column: bool,
    /// The width of the name column in the file tree view. If set to 0, the name column is
    /// auto-sized, i.e. it fills the entire left-over width from the other columns.
    pub name_col_width: i64,
    /// Maximum width chosen when auto-sizing the "Name" column width. 0 means no maximum.
    pub max_name_column_width: i64,
}

impl FileView {
    pub fn new(config: &Config) -> Self {
        Self {
            cur_subdir: PathBuf::new(),
            entry_ixs: vec![0],
            sort_by: config.default_sort_mode,
            size_metric: config.default_size_metric,
            sort_dirs_before_files: true,
            show_size_cur_column: config.default_show_size_cur_column,
            name_col_width: if config.default_name_column_width == 0 {
                0
            } else {
                config.default_name_column_width.max(4) as i64
            },
            max_name_column_width: config.max_name_column_width as i64,
        }
    }

    pub fn cur_entry_ix(&self) -> usize {
        *self.entry_ixs.last().unwrap()
    }
    pub fn cur_entry_ix_mut(&mut self) -> &mut usize {
        self.entry_ixs.last_mut().unwrap()
    }

    pub fn enter_subdir(&mut self, name: &OsString) {
        self.cur_subdir = self.cur_subdir.join(name);
        self.entry_ixs.push(0);
    }

    pub fn leave_subdir(&mut self) -> bool {
        match self.cur_subdir.parent() {
            Some(p) => {
                self.cur_subdir = p.to_path_buf();
                self.entry_ixs.pop();
                true
            }
            None => false,
        }
    }

    /// Apply the motion to the file list and return true iff the motion actually changed the
    /// selected entry.
    pub fn apply_motion(&mut self, motion: Motion, num_entries: usize, layout: &Layout) -> bool {
        let i = self.cur_entry_ix_mut();
        motion.apply_check(i, num_entries, layout.num_file_list_lines() as usize)
    }

    pub fn show_size_cur(&self) -> bool {
        self.show_size_cur_column && !self.cur_subdir.to_string_lossy().is_empty()
    }

    pub fn auto_name_col_width(&self, term_width: i64, is_scan_done: bool) -> i64 {
        let leftover = term_width - fixed_columns_width(is_scan_done, self.show_size_cur());
        let w = if self.max_name_column_width > 0 {
            leftover.min(self.max_name_column_width)
        } else {
            leftover
        };
        w.max(4)
    }

    pub fn adjust_name_col_width(
        &mut self,
        delta: i64,
        term_width: i64,
        is_scan_done: bool,
    ) -> bool {
        let auto = self.auto_name_col_width(term_width, is_scan_done);
        let adjusted = if self.name_col_width == 0 {
            // Switch from auto to a fixed width.
            auto + delta
        } else {
            self.name_col_width + delta
        };
        let adjusted = adjusted.max(4);
        let new = if adjusted == auto {
            // Switch from fixed width to auto width.
            0
        } else {
            adjusted
        };
        if new == self.name_col_width {
            false
        } else {
            self.name_col_width = new;
            true
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HelpView {
    pub cur_line_ix: usize,
}

impl HelpView {
    pub fn apply_motion(&mut self, motion: Motion, layout: &Layout) -> bool {
        motion.apply_check(
            &mut self.cur_line_ix,
            HELP.len(),
            layout.num_content_lines() as usize,
        )
    }
}

#[derive(Debug, Clone)]
pub struct ErrorView {
    pub errors: Vec<ErrorInfo>,
    pub mode: ErrorViewMode,
    pub cur_error_ix: usize,
}

/// The number of errors per page in the error view. Each error entry spans a
/// variable number of lines, so the page size is in entries, not lines.
const ERROR_VIEW_PAGE_SIZE: usize = 10;

impl ErrorView {
    pub fn apply_motion(&mut self, motion: Motion) -> bool {
        motion.apply_check(
            &mut self.cur_error_ix,
            self.errors.len(),
            ERROR_VIEW_PAGE_SIZE,
        )
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
    /// The root directory to scan recursively.
    pub target_path: PathBuf,
    /// The file tree of the root directory. This is accessible by the main thread while it is being
    /// built by the scan thread.
    pub file_tree: Arc<Mutex<FileTree>>,
    /// The state of the file tree view.
    pub file_view: FileView,
    /// The current terminal size.
    pub term_size: [u16; 2],
    /// The layout of a rendered frame.
    /// This is produced during rendering by [`crate::render::render_state`]
    /// such that layout dependent actions, e.g. the "PageDown" keybinding, can refer to it.
    pub layout: Layout,
    /// The current frame index. Used to render the spinner during scanning.
    pub frame: usize,
    /// Determines what is currently rendered (e.g. file tree view or help view), and
    /// correspondingly also which keybindings are active.
    pub display_mode: DisplayMode,
    /// Cached data for rendering the help view.
    pub help_meta: HelpMeta,
    /// How the header should be rendered.
    pub header_layout: HeaderLayout,
    /// The message that should be displayed in a status bar at the bottom.
    pub status: Option<String>,
}

impl State {
    pub fn cur_subdir_entry_len(&self) -> usize {
        self.file_tree
            .lock()
            .unwrap()
            .get(&self.file_view.cur_subdir)
            .map(|node| node.children.len())
            .unwrap_or(0)
    }

    pub fn is_scan_done(&self) -> bool {
        !matches!(self.file_tree.lock().unwrap().root.state, FileState::Queued)
    }

    pub fn cur_entry(&self) -> Option<(OsString, Inode)> {
        let ix = self.file_view.cur_entry_ix();
        let file_tree = self.file_tree.lock().unwrap();
        sorted_entries(&file_tree, self)
            .into_iter()
            .nth(ix)
            .map(|(name, _, inode, _, _)| (name.clone(), inode))
    }

    /// Target-relative path of the currently selected entry.
    pub fn cur_entry_path(&self) -> Option<PathBuf> {
        let (name, _) = self.cur_entry()?;
        Some(self.file_view.cur_subdir.join(&name))
    }

    pub fn try_delete(&mut self, rel: &Path) {
        let target = self.target_path.clone();
        let abs = target.join(rel);
        if let Err(e) = remove_file_or_dir(&abs) {
            self.status = Some(format!("Failed to delete {abs:?}: {e}"));
            return;
        }
        let stat_nlink = |p: &Path| {
            std::fs::symlink_metadata(target.join(p))
                .ok()
                .map(|m| m.nlink())
        };
        let mut tree = self.file_tree.lock().unwrap();
        if tree.delete_path(rel, &stat_nlink).is_none() {
            drop(tree);
            self.status = Some(format!(
                "Failed to delete {abs:?}: entry not found in file tree."
            ));
            return;
        }
        drop(tree);

        // Make sure that the selection index of the entry list is still in bounds.
        // This might not be the case if the last entry was deleted.
        let len = self.cur_subdir_entry_len();
        let ix = self.file_view.cur_entry_ix_mut();
        if *ix >= len {
            *ix = len.saturating_sub(1);
        }
    }
}

pub fn remove_file_or_dir(p: &Path) -> Result<(), std::io::Error> {
    if p.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}
