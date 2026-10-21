# Changelog

## 0.2.0

### Breaking Changes

- Keybinding: Change keybinding for toggling "Size Cur" column from `c` to `C`
  as modtime column is also toggled by an uppercase letter (`M`). (f284b6c)

- Remove header layout option in favor of a compact form of the multi-line
  layout. This change also breaks very long paths properly. (019e252)

### Features

- Refuse to delete an entry that is a mount point or contains a mount point
  This only works on linux: if `/proc/self/mounts` does not exist, the check is
  skipped. (05fc46c)

- Add `C-c` keybinding that quits the program independently from which view is
  active. (6a97055)

- Remove progress column and instead use a style to highlight the currently
  scanned entry in the file tree. (00a339c)

- Add config option and keybinding to toggle the "Last Modified" column of the
  file tree. (cb54dcf)

- Add config option to set the startup statusbar message and make it
  default to `"Press ? to show keybindings"`. (53e90b0)

- Add config option to display the home directory as `~`. The option is enabled
  by default. (47770b4)

- If the config directory exists but does not contain a config file, then show
  a warning and use defaults instead of stopping with a hard error. (b5eade2)

### Fixes

- Prevent `dumb` from spinning forever if it is backgrounded or started with
  `nohup` and then the terminal is closed. (be839bc)

- Starting dumb on a directory symlink `dumb <symlink-to-directory>` now works
  correctly. (4588c09)

- Fixed two bugs leading to incorrect sizes displayed after deleting files with
  hardlinks. (4b8bbc3, 3c114d8)

- Themes: For styles that are merged on other styles the boolean attributes
  (bold, italic, etc.) are now merged correctly. In particular setting, e.g.
  `bold = false` is treated differently from not setting `bold` at all.
  (8e3de2e)

- Rendering: Human readable byte rendering now extends until zetabytes and
  renders the edge case of `1024 B` as `1.00 KB`. (92f8df4)

- Rendering: Prevent the '/' suffix of directory names from rendering past the
  name column width. (132dcce)

- Rendering: Fixed off-by-one error which caused the currently selected line to
  not be exactly vertically centered. (4be3310)

- Rendering: Display `-` for modtimes that could not be read instead of
  defaulting to `UNIX_EPOCH`. (1f33027)

- Errors and panics are no longer written to a `dumb.log` file in the current
  working directory by default. Set the `DUMB_LOG_ERROR_TO_FILE` environment
  variable to restore this behavior. (3aad024)


## 0.1.1 - 2026-10-20

- Significant performance and responsiveness improvements for rendering and
  navigating in large directories while they are being scanned.
  (0b890fd, 09ab811)
- Comment out `theme` option in `config.default.toml` to match actual default
  behavior. (a1f3896)
- Add install instructions to `README.md`. (e6d646f)


## 0.1.0 - 2026-10-19

Initial release.

- Interactive TUI for exploring the disk usage of a directory tree
- Live file tree updates while the directory is being scanned
- Sizes reflect the space that deleting an entry would free,
  accounting for hardlinks and actual block sizes
- In-process deletion of entries, with confirmation by default
- TOML-based configuration and theming
- Vim and regular keybindings
