use anyhow::{Context, Result, bail};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::canvas::{LayerMap, LayerStack, LineData, LineMarker};
use crate::model::{Atom, Coord, Face, LayerId, StyledAtom};
use crate::selection::{CanvasRegion, SelectionBounds, TextRectangle};

pub(crate) fn selected_atoms(map: &LayerMap, bounds: SelectionBounds) -> Vec<Vec<StyledAtom>> {
    (bounds.top..=bounds.bottom)
        .map(|line| {
            (bounds.left..=bounds.right)
                .map(|column| styled_at(map, line, column))
                .collect()
        })
        .collect()
}

pub(crate) fn atoms_in_region(map: &LayerMap, region: CanvasRegion) -> Vec<Vec<StyledAtom>> {
    (0..region.height)
        .map(|row_offset| {
            let line = region
                .top
                .saturating_add(i64::try_from(row_offset).unwrap_or(i64::MAX));
            (0..region.width)
                .map(|column_offset| {
                    let column = region
                        .left
                        .saturating_add(i64::try_from(column_offset).unwrap_or(i64::MAX));
                    let (Ok(line), Ok(column)) = (i16::try_from(line), i16::try_from(column))
                    else {
                        return default_blank();
                    };
                    styled_at(map, line, column)
                })
                .collect()
        })
        .collect()
}

pub(crate) fn from_dense(
    id: LayerId,
    visible: bool,
    lines: &[Vec<StyledAtom>],
) -> Result<LayerMap> {
    for atom in lines.iter().flatten() {
        for grapheme in UnicodeSegmentation::graphemes(atom.contents.as_str(), true) {
            if UnicodeWidthStr::width(grapheme) != 1 {
                bail!("atom {grapheme:?} has display width other than 1");
            }
        }
    }
    from_dense_with_markers(id, visible, lines, &[])
}

pub(crate) fn from_dense_with_markers(
    id: LayerId,
    visible: bool,
    lines: &[Vec<StyledAtom>],
    line_markers: &[LineMarker],
) -> Result<LayerMap> {
    let mut map = LayerMap::new(id, visible);
    for (line_index, row) in lines.iter().enumerate() {
        let line = i16::try_from(line_index).context("canvas line exceeds signed i16 range")?;
        let mut column = 0i16;
        for atom in row {
            for grapheme in UnicodeSegmentation::graphemes(atom.contents.as_str(), true) {
                let width = UnicodeWidthStr::width(grapheme);
                if width == 0 {
                    bail!("atom {grapheme:?} has display width zero");
                }
                map.set_at_untracked(column, line, Atom::new(grapheme)?, &atom.face)?;
                column = column
                    .checked_add(i16::try_from(width).context("atom width exceeds i16")?)
                    .context("canvas column exceeds signed i16 range")?;
            }
        }
    }
    for marker in line_markers {
        map.set_line_data_untracked(
            marker.coord.column,
            marker.coord.line,
            Some(LineData {
                ending: marker.ending,
                base_glyph: marker.base_glyph.clone(),
            }),
        );
    }
    Ok(map)
}

pub(crate) fn to_dense(map: &LayerMap) -> Vec<Vec<StyledAtom>> {
    let height = map
        .rows()
        .last_key_value()
        .and_then(|(&line, _)| usize::try_from(line).ok())
        .map_or(1, |line| line.saturating_add(1));
    (0..height)
        .map(|line| {
            let row = i16::try_from(line)
                .ok()
                .and_then(|line| map.rows().get(&line));
            let width = row.map_or(0, |row| {
                row.iter()
                    .filter_map(|(&column, data)| {
                        usize::try_from(column).ok().map(|column| {
                            column
                                .saturating_add(UnicodeWidthStr::width(data.atom.contents()).max(1))
                        })
                    })
                    .max()
                    .unwrap_or(0)
            });
            let mut atoms = Vec::new();
            let mut column = 0usize;
            while column < width {
                let data = i16::try_from(column)
                    .ok()
                    .and_then(|column| row.and_then(|row| row.get(&column)));
                if let Some(data) = data {
                    column =
                        column.saturating_add(UnicodeWidthStr::width(data.atom.contents()).max(1));
                    atoms.push(styled(data));
                } else {
                    atoms.push(default_blank());
                    column += 1;
                }
            }
            atoms
        })
        .collect()
}

pub(crate) fn composite_region(
    stack: &LayerStack,
    region: CanvasRegion,
) -> Option<Vec<Vec<StyledAtom>>> {
    let left = i16::try_from(region.left).ok()?;
    let top = i16::try_from(region.top).ok()?;
    let width = i16::try_from(region.width).ok()?;
    let height = i16::try_from(region.height).ok()?;
    let mut rows = Vec::with_capacity(region.height);
    for line_offset in 0..height {
        let line = top.checked_add(line_offset)?;
        let mut row = Vec::with_capacity(region.width);
        for column_offset in 0..width {
            let column = left.checked_add(column_offset)?;
            let atom = stack
                .effective_layers()
                .iter()
                .filter(|layer| layer.visible)
                .filter_map(|layer| layer.get(line, column))
                .filter(|data| !data.atom.contents().chars().all(char::is_whitespace))
                .next_back()
                .map_or_else(default_blank, styled);
            row.push(atom);
        }
        rows.push(row);
    }
    Some(rows)
}

pub(crate) fn overwrite_rectangle(
    map: &mut LayerMap,
    origin: Coord,
    rectangle: &TextRectangle,
) -> Result<()> {
    for (row_offset, row) in rectangle.rows.iter().enumerate() {
        let row_offset = i16::try_from(row_offset).context("rectangle exceeds canvas height")?;
        let line = origin
            .line
            .checked_add(row_offset)
            .context("rectangle exceeds canvas height")?;
        for (column_offset, atom) in row.iter().enumerate() {
            atom.validate_cell()?;
            let column_offset =
                i16::try_from(column_offset).context("rectangle exceeds canvas width")?;
            let column = origin
                .column
                .checked_add(column_offset)
                .context("rectangle exceeds canvas width")?;
            map.delete_at(column, line);
            map.set_at(column, line, Atom::new(atom.contents.clone())?, &atom.face)?;
        }
    }
    Ok(())
}

pub(crate) fn overwrite_active_rectangle(
    stack: &mut LayerStack,
    origin: Coord,
    rectangle: &TextRectangle,
) -> Result<()> {
    overwrite_rectangle(stack.active_layer_mut(), origin, rectangle)
}

fn styled_at(map: &LayerMap, line: i16, column: i16) -> StyledAtom {
    map.get(line, column).map_or_else(default_blank, styled)
}

fn styled(data: &crate::canvas::CoordData) -> StyledAtom {
    StyledAtom {
        face: data.face.as_ref().clone(),
        contents: data.atom.contents().to_owned(),
    }
}

fn default_blank() -> StyledAtom {
    StyledAtom {
        face: Face::default(),
        contents: " ".to_owned(),
    }
}
