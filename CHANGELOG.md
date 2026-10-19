# Changelog

## 0.1.0 - 2026-10-19

Initial release.

- Interactive TUI for exploring the disk usage of a directory tree
- Live file tree updates while the directory is being scanned
- Sizes reflect the space that deleting an entry would free,
  accounting for hardlinks and actual block sizes
- In-process deletion of entries, with confirmation by default
- TOML-based configuration and theming
- Vim and regular keybindings
