# ascdraw

ascdraw is a work-in-progress native ASCII drawing editor inspired by Monodraw and Emacs Uniline.

The current foundation provides:

- A standalone editable cell grid with no external editor process
- Move/Draw mode with `h`/`j`/`k`/`l` and arrow-key movement
- Shift-movement drawing with connected Unicode lines, corners, tees, and crossings
- Grapheme-aware text insertion, deletion, cursor movement, and mouse positioning
- Skia-based fixed-cell rendering with system-font fallback
- Face-based foreground, background, underline, and text-attribute resolution
- Configurable block, beam, and underline cursors
- Live font size controls and configuration reload
- Native multi-window and macOS menu integration

The checked-in [`ascdraw.toml`](ascdraw.toml) contains bundled defaults. User overrides are loaded from `~/.config/ascdraw/config.toml`, or `$XDG_CONFIG_HOME/ascdraw/config.toml` when `XDG_CONFIG_HOME` is set.

Run `ascdraw --show-config` to print the merged configuration.

## Move/Draw mode

The editor starts in Move/Draw mode. Use `h`, `j`, `k`, `l`, or the arrow keys to move one cell. Hold Shift while moving to draw a thin line. New segments connect to existing lines using Uniline-style rounded corners, tees, and crossings without overwriting ordinary text.

Insert mode, Replace mode, the remaining drawing tools, and a document format are not implemented yet.
