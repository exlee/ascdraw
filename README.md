# ascdraw

ascdraw is a native editor for drawing diagrams with Unicode text and line characters. It is inspired by Monodraw and Emacs Uniline and is currently under active development.

## Quick start

The editor opens in **Line** mode.

| Action | Keys |
| --- | --- |
| Move the cursor | Arrow keys or `h` `j` `k` `l` |
| Draw a line | Shift + direction |
| Clear the current cell | Space or Backspace |
| Enter or leave text entry | Return |
| Choose a tool directly | `1`, then its displayed number |
| Choose an option directly | Its displayed three-digit path |

Lines connect automatically into corners, tees, and crossings. Starting a stroke on an existing line extends that connection; starting on an endpoint marker moves the marker to the new endpoint.

## Toolbar

The first two rows select a tool directly: press `1`, then the number under **Line**, **Stamp**, **Shape**, or **Utils**. The option rows show hierarchical paths such as `2.1.3`: press the category number, page number, and option number in sequence. On each page, `1` through `9` select the first nine options and `0` selects the tenth.

### Line

Line mode is fully usable. Its options control:

- **Line Start** — none, arrow, diamond, or circle
- **Line End** — none, arrow, diamond, or circle
- **Line Width** — thin, heavy, or double

Hold Shift while moving with the arrow keys or `h` `j` `k` `l` to draw. Move without Shift to navigate.

### Text entry

Text entry is independent of the selected toolbar tool. Press Return to enter it and Return again to return to the selected tool. Type to insert text; the arrow keys or `h` `j` `k` `l` move freely over the canvas. Backspace and Delete edit text, and Tab inserts four spaces.

### Stamp, Shape, and Utils

These tools expose their planned options and canvas navigation, but their editing actions are not implemented yet.

## Configuration

Bundled application defaults live in [`ascdraw.toml`](ascdraw.toml), while the bundled stylesheet lives in [`theme.toml`](theme.toml). Put personal overrides in:

```text
~/.config/ascdraw/config.toml
```

If `XDG_CONFIG_HOME` is set, ascdraw reads `$XDG_CONFIG_HOME/ascdraw/config.toml` instead. Run `ascdraw --show-config` to print the merged configuration.

Default application shortcuts:

| Action | macOS shortcut |
| --- | --- |
| Increase font size | Command + `=` |
| Decrease font size | Command + `-` |
| Reset font size | Command + `0` |
| New window | Command + `N` |
| Close window | Command + `W` |

All shortcuts can be changed in the `[keys]` section of the configuration file.

Theme overrides use `[theme.<face>]` tables. The semantic faces are `default`, `selection`, `selection-highlight` (a pending menu prefix), `cursor-drawing`, `cursor-block`, and `tooltip`. Unspecified face colors inherit from `default`.

## Current scope

ascdraw currently provides an editable grapheme-aware cell grid, connected Unicode line drawing, text entry, font fallback, configurable themes and cursor shapes, live font scaling, multiple native windows, and macOS menu integration.

Document save/load and the Stamp, Shape, and Utils editing operations are still to come.
