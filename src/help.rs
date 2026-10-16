pub const HELP: &[(&'static str, &'static str, &'static str)] = &[
    ("Keys", "Action", "Scope"),
    ("q", "Quit", "files"),
    ("q", "Back to file tree", "errors, help"),
    ("hjkl/arrow keys", "Basic motions", "files, errors, help"),
    ("g/Home", "Jump to first entry", "files, errors, help"),
    ("G/End", "Jump to last entry", "files, errors, help"),
    ("C-b/PageUp", "Jump one page up", "files, errors, help"),
    ("C-f/PageDown", "Jump one page down", "files, errors, help"),
    ("C-u", "Jump half page up", "files, errors, help"),
    ("C-d", "Jump half page down", "files, errors, help"),
    (
        "e",
        "Toggle error view for currently selected entry",
        "files, errors",
    ),
    ("E", "Toggle error view of all errors", "files, errors"),
    ("s", "Next sort mode (by size/by name/by modtime)", "files"),
    // ("S", "Toggle sorting directories before files", "files"),
    ("?", "Toggle this help screen", "files, errors"),
];

#[derive(Debug, Clone, Copy)]
pub struct HelpMeta {
    pub key_col_width: usize,
    pub action_col_width: usize,
}

impl HelpMeta {
    pub fn new() -> Self {
        let mut key_col_width = 0;
        let mut action_col_width = 0;
        for (k, a, _s) in HELP {
            key_col_width = key_col_width.max(k.len());
            action_col_width = action_col_width.max(a.len());
        }
        Self {
            key_col_width,
            action_col_width,
        }
    }
}
