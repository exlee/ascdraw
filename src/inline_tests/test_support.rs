use unicode_width::UnicodeWidthStr;

use crate::canvas::LayerMap;
use crate::model::{Face, StyledAtom};

pub(crate) fn dense_layer(map: &LayerMap) -> Vec<Vec<StyledAtom>> {
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
                if let Some(data) = i16::try_from(column)
                    .ok()
                    .and_then(|column| row.and_then(|row| row.get(&column)))
                {
                    column =
                        column.saturating_add(UnicodeWidthStr::width(data.atom.contents()).max(1));
                    atoms.push(StyledAtom {
                        face: data.face.as_ref().clone(),
                        contents: data.atom.contents().to_owned(),
                    });
                } else {
                    atoms.push(StyledAtom {
                        face: Face::default(),
                        contents: " ".to_owned(),
                    });
                    column += 1;
                }
            }
            atoms
        })
        .collect()
}
