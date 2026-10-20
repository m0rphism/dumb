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
pub mod snapshot;
pub mod theme;

use std::{
    ffi::OsString,
    io::stdout,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, channel},
    },
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
    config::{Config, SizeMetric, SortBy},
    file_tree::FileTree,
    help::{HELP, HelpMeta},
    layout::Layout,
    motion::Motion,
    raw_mode::RawMode,
    render::{fixed_columns_width, render_state},
    scan::scan,
    snapshot::{FileEntry, Snapshot, ViewParams},
    theme::Theme,
};

fn main() -> ExitCode {
    RawMode::set_panic_hook();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let err_str = format!("{e:?}");
            let _ = std::fs::write("dumb.log", &err_str);
            eprintln!("{err_str}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();

    let (config, theme) = load_config_and_theme(&args.config)?;

    let root_dir = match args.root_dir {
        Some(path) => path,
        None => {
            std::env::current_dir().context("Failed to determine the current working directory.")?
        }
    };

    let file_tree = Arc::new(Mutex::new(FileTree::new(&root_dir)?));

    let (sender, receiver) = channel();
    let (done_sender, done_receiver) = channel();

    // The version of the file tree. This is used to check if the `Snapshot` is stale.
    // The version is kept outside of the file tree mutex such that it can be read lock-free.
    //
    // It is bumped under the tree lock whenever the file tree gets mutated, i.e. after
    // - the file tree handled a `ScanEvent` sent by the scan thread
    // - the user deleted a file via keybinding
    //
    // The purpose of the staleness checks is to prevent
    // - a synchronous `Snapshot` rebuild triggered by a deletion to be overwritten by an
    //   asynchronous rebuild that was started before the deletion but arrived after the synchronous rebuild.
    // - unnecessary `Snapshot` rebuilds (although those are rare and practically only
    //   happen with slow network mounts, where the file tree does not get updated between redraw
    //   ticks)
    let tree_version = Arc::new(AtomicU64::new(0));

    // Spawn thread that scans the `root_dir`.
    let _scan_handle = std::thread::spawn({
        let file_tree = file_tree.clone();
        let tree_version = tree_version.clone();
        let root_dir = root_dir.clone();
        move || -> Result<()> {
            scan(
                &root_dir,
                |mut scan_event| -> Result<()> {
                    let mut file_tree = file_tree.lock().unwrap();
                    scan_event
                        .map_path_bufs(|p| *p = p.strip_prefix(&root_dir).unwrap().to_path_buf());
                    file_tree.handle_scan_event(scan_event);
                    tree_version.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                },
                |p: &Path| {
                    let rel = p.strip_prefix(&root_dir).unwrap_or(p);
                    file_tree.lock().unwrap().is_deleted_path(rel)
                },
            )?;
            done_sender.send(())?;
            Ok(())
        }
    });

    // Spawn thread that periodically sends redraw events until the scan completed.
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

    // Channel for requesting builds of the view snapshot.
    let (build_sender, build_receiver) = channel::<ViewParams>();

    // Spawn the snapshot builder thread.
    let _builder_handle = Snapshot::spawn_snapshot_builder_thread(
        file_tree.clone(),
        tree_version.clone(),
        build_receiver,
        move |snapshot| {
            sender
                .send(Event::SnapshotReady(snapshot))
                .map_err(|e| e.into())
        },
    );

    let term_size = {
        let (w, h) = terminal::size()?;
        [w, h]
    };

    // cur_subdir_entries: root_dir
    //     .read_dir()?
    //     .filter_map(|e| e.ok().map(|e| e.file_name()))
    //     .collect(),
    // state.cur_subdir_entries.sort();
    let mut state = State {
        file_tree,
        tree_version: tree_version.clone(),
        file_view: FileView::new(&config),
        root_dir,
        term_size,
        layout: Layout::new(term_size, 1),
        spinner_frame: 0,
        display_mode: DisplayMode::FileTree,
        help_return_mode: DisplayMode::FileTree,
        help_meta: HelpMeta::new(),
        status: if config.initial_statusbar_message.is_empty() {
            None
        } else {
            Some(config.initial_statusbar_message.clone())
        },
    };

    let mut stdout = stdout();
    let _raw_mode = RawMode::enter()?;

    // Improve perceived startup time by giving the scanner a brief moment to deliver the first entries.
    std::thread::sleep(Duration::from_millis(50));

    let mut snapshot = Snapshot::default();
    snapshot.sync_rebuild(&state.file_view.params(), &state.file_tree, &tree_version);
    state.file_view.resolve_current_selection(&snapshot);
    state.layout = render_state(&mut stdout, &state, &snapshot, &theme, &config)?;

    for event in receiver {
        let redraw = match event {
            Event::Term(event) => match event {
                TermEvent::Key(key_event) => {
                    match handle_key_event(
                        &mut state,
                        &mut snapshot,
                        &config,
                        key_event,
                        &build_sender,
                    ) {
                        Some(redraw) => redraw,
                        None => break,
                    }
                }
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
            Event::Redraw => {
                state.spinner_frame += 1;
                // Request a new `Snapshot` if the tree advanced since previous snapshot.
                if tree_version.load(Ordering::Relaxed) != snapshot.version {
                    let _ = build_sender.send(state.file_view.params());
                }
                true
            }
            Event::SnapshotReady(new) => {
                // Use the new snapshot only if it matches the current view and is not older than the
                // current snapshot. Without this check, it could happen that a synchronous snapshot
                // update, e.g. when the user changed the directory, is overwritten by a delayed
                // asynchronous snapshot.
                if new.params == state.file_view.params() && new.version >= snapshot.version {
                    snapshot = new;
                    state.file_view.resolve_current_selection(&snapshot);
                }
                true
            }
        };
        if redraw {
            state.layout = render_state(&mut stdout, &state, &snapshot, &theme, &config)?;
        }
    }

    Ok(())
}

/// Returns `None` if the program should quit, or `Some(should_redraw)`.
fn handle_key_event(
    state: &mut State,
    snapshot: &mut Snapshot,
    config: &Config,
    key_event: KeyEvent,
    build_sender: &mpsc::Sender<ViewParams>,
) -> Option<bool> {
    let redraw = if key_event.kind != KeyEventKind::Release {
        // Any key press clears the status message.
        state.status = None;

        let layout = state.layout;
        match &mut state.display_mode {
            DisplayMode::FileTree => match key_event.code {
                KeyCode::Char('q') => return None,
                KeyCode::Char('j') | KeyCode::Down => {
                    state
                        .file_view
                        .apply_motion(Motion::Down, &snapshot, &layout)
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.file_view.apply_motion(Motion::Up, &snapshot, &layout)
                }
                KeyCode::Char('h') | KeyCode::Left => {
                    if state.file_view.leave_subdir() {
                        snapshot.sync_rebuild(
                            &state.file_view.params(),
                            &state.file_tree,
                            &state.tree_version,
                        );
                        state.file_view.resolve_current_selection(&snapshot);
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    if let Some((_, entry)) = state.file_view.cur_entry(snapshot)
                        && entry.is_dir
                    {
                        let name = entry.name.clone();
                        state.file_view.enter_subdir(&name);
                        snapshot.sync_rebuild(
                            &state.file_view.params(),
                            &state.file_tree,
                            &state.tree_version,
                        );
                        state.file_view.resolve_current_selection(&snapshot);
                        true
                    } else {
                        false
                    }
                }
                KeyCode::Char('g') | KeyCode::Home => {
                    state
                        .file_view
                        .apply_motion(Motion::First, &snapshot, &layout)
                }
                KeyCode::Char('G') | KeyCode::End => {
                    state
                        .file_view
                        .apply_motion(Motion::Last, &snapshot, &layout)
                }
                KeyCode::Char('f') if key_event.modifiers == KeyModifiers::CONTROL => state
                    .file_view
                    .apply_motion(Motion::PageDown, &snapshot, &layout),
                KeyCode::PageDown => {
                    state
                        .file_view
                        .apply_motion(Motion::PageDown, &snapshot, &layout)
                }
                KeyCode::Char('b') if key_event.modifiers == KeyModifiers::CONTROL => state
                    .file_view
                    .apply_motion(Motion::PageUp, &snapshot, &layout),
                KeyCode::PageUp => state
                    .file_view
                    .apply_motion(Motion::PageUp, &snapshot, &layout),
                KeyCode::Char('d') if key_event.modifiers == KeyModifiers::CONTROL => state
                    .file_view
                    .apply_motion(Motion::HalfPageDown, &snapshot, &layout),
                KeyCode::Char('u') if key_event.modifiers == KeyModifiers::CONTROL => state
                    .file_view
                    .apply_motion(Motion::HalfPageUp, &snapshot, &layout),
                KeyCode::Char('s') => {
                    state.file_view.sort_by = state.file_view.sort_by.next();
                    let _ = build_sender.send(state.file_view.params());
                    true
                }
                KeyCode::Char('S') => {
                    state.file_view.sort_dirs_before_files =
                        !state.file_view.sort_dirs_before_files;
                    let _ = build_sender.send(state.file_view.params());
                    true
                }
                KeyCode::Char('m') => {
                    state.file_view.size_metric = state.file_view.size_metric.next();
                    let _ = build_sender.send(state.file_view.params());
                    true
                }
                KeyCode::Char('c') => {
                    state.file_view.show_size_cur_column = !state.file_view.show_size_cur_column;
                    true
                }
                KeyCode::Char('+') => state.file_view.adjust_name_col_width(
                    1,
                    state.term_size[0] as i64,
                ),
                KeyCode::Char('-') => state.file_view.adjust_name_col_width(
                    -1,
                    state.term_size[0] as i64,
                ),
                KeyCode::Char('e') => {
                    if let Some((_, entry)) = state.file_view.cur_entry(snapshot) {
                        let cur_path = state.file_view.cur_subdir.join(&*entry.name);
                        let errors: Vec<_> = state
                            .file_tree
                            .lock()
                            .unwrap()
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
                    if let Some((_, entry)) = state.file_view.cur_entry(snapshot) {
                        let rel_path = state.file_view.cur_subdir.join(&*entry.name);
                        if config.require_delete_confirmation {
                            state.display_mode = DisplayMode::DeleteDialog(rel_path);
                        } else {
                            state.try_delete(&rel_path, &mut *snapshot);
                        }
                    }
                    true
                }
                KeyCode::Char('?') => {
                    state.help_return_mode = DisplayMode::FileTree;
                    state.display_mode = DisplayMode::HelpView(HelpView::default());
                    true
                }
                _ => false,
            },
            DisplayMode::DeleteDialog(target) => {
                let target = target.clone();
                state.display_mode = DisplayMode::FileTree;
                if let KeyCode::Char('y' | 'Y') = key_event.code {
                    state.try_delete(&target, &mut *snapshot);
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
                    state.help_return_mode = DisplayMode::ErrorView(error_view.clone());
                    state.display_mode = DisplayMode::HelpView(HelpView::default());
                    true
                }
                _ => false,
            },
            DisplayMode::HelpView(help_view) => match key_event.code {
                KeyCode::Char('q') | KeyCode::Char('?') => {
                    state.display_mode = state.help_return_mode.clone();
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

#[derive(Debug)]
pub enum Event {
    /// Terminal events
    Term(TermEvent),
    /// Sent in a fixed interval by the redraw thread while the scan is running.
    Redraw,
    /// Sent by the snapshot builder thread after building a snapshot.
    SnapshotReady(Snapshot),
}

#[derive(Debug, Clone)]
pub enum DisplayMode {
    FileTree,
    ErrorView(ErrorView),
    HelpView(HelpView),
    /// `root_dir`-relative path of the entry to delete.
    DeleteDialog(PathBuf),
}

/// The state of the file tree view: the currently viewed subdirectory, the selection within it,
/// and the view's display options.
#[derive(Debug, Clone)]
pub struct FileView {
    /// The current subdirectory selected in the file tree view. Relative to [`State::root_dir`].
    pub cur_subdir: PathBuf,
    /// The name of the currently selected entry in [`FileView::cur_subdir`].
    /// `None` if the current subdir has no entries.
    pub selected: Option<OsString>,
    /// How the file tree view is sorted.
    pub sort_by: SortBy,
    /// The size-metric with which the entries of the file tree are displayed.
    pub size_metric: SizeMetric,
    /// Whether the file tree view should sort directories before files (only relevant for [`SortBy::Name`].
    pub sort_dirs_before_files: bool,
    /// Whether to display the "Size Cur" column in the file tree view (only relevant if the file
    /// tree view is currently in a subdirectory of [`State::root_dir`].
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
            selected: None,
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

    /// The currently selected entry within the snapshot.
    ///
    /// Returns `None` if there is no selection (e.g. no entries in directory) or
    /// if the selection could not be found in the snapshot's entry list.
    pub fn cur_entry<'a>(&self, snapshot: &'a Snapshot) -> Option<(usize, &'a FileEntry)> {
        self.selected.as_ref().and_then(|name| {
            snapshot
                .entries
                .iter()
                .enumerate()
                .find(|(_, e)| e.name.as_os_str() == name.as_os_str())
        })
    }

    /// Set the current selection to the first entry if it was previously `None`.
    pub fn resolve_current_selection(&mut self, snapshot: &Snapshot) {
        if self.selected.is_none()
            && let Some(first) = snapshot.entries.first()
        {
            self.selected = Some(first.name.as_ref().clone());
        }
    }

    /// The view parameters that determine the snapshot.
    pub fn params(&self) -> ViewParams {
        ViewParams {
            subdir: self.cur_subdir.clone(),
            sort_by: self.sort_by,
            sort_dirs_before_files: self.sort_dirs_before_files,
            size_metric: self.size_metric,
        }
    }

    pub fn enter_subdir(&mut self, name: &OsString) {
        self.cur_subdir = self.cur_subdir.join(name);
        // The selection gets resolved after the snapshot is built.
        self.selected = None;
    }

    pub fn leave_subdir(&mut self) -> bool {
        match self.cur_subdir.parent() {
            Some(p) => {
                self.selected = Some(self.cur_subdir.file_name().unwrap().to_os_string());
                self.cur_subdir = p.to_path_buf();
                true
            }
            None => false,
        }
    }

    /// Apply the motion to the file list and return true iff the motion actually changed the
    /// selected entry.
    pub fn apply_motion(&mut self, motion: Motion, snapshot: &Snapshot, layout: &Layout) -> bool {
        let cur = self.cur_entry(snapshot).map(|(i, _)| i).unwrap_or(0);
        let new = motion.apply(
            cur,
            snapshot.entries.len(),
            layout.num_file_list_lines() as usize,
        );
        if new == cur {
            return false;
        }
        self.selected = snapshot.entries.get(new).map(|e| e.name.as_ref().clone());
        true
    }

    pub fn show_size_cur(&self) -> bool {
        self.show_size_cur_column && !self.cur_subdir.to_string_lossy().is_empty()
    }

    pub fn auto_name_col_width(&self, term_width: i64) -> i64 {
        let leftover = term_width - fixed_columns_width(self.show_size_cur());
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
    ) -> bool {
        let auto = self.auto_name_col_width(term_width);
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
    pub root_dir: PathBuf,
    /// The file tree of the root directory. This is accessible by the main thread while it is being
    /// built by the scan thread.
    pub file_tree: Arc<Mutex<FileTree>>,
    /// The version of the file tree (see its creation in `run`).
    pub tree_version: Arc<AtomicU64>,
    /// The state of the file tree view.
    pub file_view: FileView,
    /// The current terminal size.
    pub term_size: [u16; 2],
    /// The layout of a rendered frame.
    /// This is produced during rendering by [`crate::render::render_state`]
    /// such that layout dependent actions, e.g. the "PageDown" keybinding, can refer to it.
    pub layout: Layout,
    /// The current frame of the spinner animation. Incremented by the redraw thread.
    pub spinner_frame: usize,
    /// Determines what is currently rendered (e.g. file tree view or help view), and
    /// correspondingly also which keybindings are active.
    pub display_mode: DisplayMode,
    /// The display mode that the help view was opened from, which is restored when the help view
    /// is closed. Only meaningful while the help view is displayed.
    pub help_return_mode: DisplayMode,
    /// Cached data for rendering the help view.
    pub help_meta: HelpMeta,
    /// The message that should be displayed in a status bar at the bottom.
    pub status: Option<String>,
}

impl State {
    /// Try to delete the `root_dir`-relative entry at `rel` from the file system and from the file
    /// tree, then rebuild `snapshot` synchronously so that the next frame no longer shows the
    /// deleted entry, and update the selection.
    pub fn try_delete(&mut self, rel: &Path, snapshot: &mut Snapshot) {
        let abs = self.root_dir.join(rel);

        if let Err(e) = remove_file_or_dir(&abs) {
            self.status = Some(format!("Failed to delete {abs:?}: {e}"));
            return;
        }

        // Remember where the selected entry sits so that, after the deletion, the selection can be
        // moved to the entry that shifts up into its place.
        let old_ix = self.file_view.cur_entry(&snapshot).map(|(i, _)| i);

        let stat_nlink = |p: &Path| {
            std::fs::symlink_metadata(self.root_dir.join(p))
                .ok()
                .map(|m| m.nlink())
        };

        {
            let mut tree = self.file_tree.lock().unwrap();
            if tree.delete_path(rel, &stat_nlink).is_none() {
                drop(tree);
                self.status = Some(format!(
                    "Failed to delete {abs:?}: entry not found in file tree."
                ));
                return;
            }
            self.tree_version.fetch_add(1, Ordering::Relaxed);
        }

        // The deletion modified the file tree: rebuild the snapshot synchronously so that the next
        // frame reflects it and the selection can be resolved against the fresh list.
        snapshot.sync_rebuild(
            &self.file_view.params(),
            &self.file_tree,
            &self.tree_version,
        );

        let new_ix = old_ix
            .map(|old| old.min(snapshot.entries.len().saturating_sub(1)))
            .unwrap_or(0);
        self.file_view.selected = snapshot
            .entries
            .get(new_ix)
            .map(|e| e.name.as_ref().clone());
    }
}

pub fn remove_file_or_dir(p: &Path) -> Result<(), std::io::Error> {
    if p.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}
