# ascdraw

ascdraw is a work-in-progress native ASCII drawing editor inspired by Monodraw and Emacs Uniline.

The current foundation provides:

- A standalone editable cell grid with no external editor process
- Grapheme-aware text insertion, deletion, cursor movement, and mouse positioning
- Skia-based fixed-cell rendering with system-font fallback
- Face-based foreground, background, underline, and text-attribute resolution
- Configurable block, beam, and underline cursors
- Live font size controls and configuration reload
- Native multi-window and macOS menu integration

The checked-in [`ascdraw.toml`](ascdraw.toml) contains bundled defaults. User overrides are loaded from `~/.config/ascdraw/config.toml`, or `$XDG_CONFIG_HOME/ascdraw/config.toml` when `XDG_CONFIG_HOME` is set.

Run `ascdraw --show-config` to print the merged configuration.

## Status

The drawing-specific tools and document format are not implemented yet. This first stage establishes a standalone editor and retains the cell, face, and cursor rendering foundation.
