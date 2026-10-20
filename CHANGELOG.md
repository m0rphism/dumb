# Changelog

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
