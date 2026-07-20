use crate::app::CursorMode;
use crate::drawing::{LineEnding, is_line_glyph};
#[cfg(test)]
use crate::model::StyledAtom;
use crate::model::{Coord, Direction};
use crate::selection::CanvasSelection;
use crate::toolbar::RoutingMode;

use super::routing::{RouteStep, route_steps};
use super::{Editor, adjacent_coord};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoutedSegment {
    end: Coord,
    routing: RoutingMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LinePreview {
    start: Coord,
    segments: Vec<RoutedSegment>,
    end: Coord,
    pub(super) source_canvas: crate::canvas::LayerStack,
    pub(super) source_cursor: Coord,
    pub(super) source_selection: CanvasSelection,
    rendered_canvas: crate::canvas::LayerStack,
}

impl LinePreview {
    pub(super) fn has_committed_segments(&self) -> bool {
        !self.segments.is_empty()
    }

    fn anchor(&self) -> Coord {
        self.segments
            .last()
            .map_or(self.start, |segment| segment.end)
    }
}

impl Editor {
    pub fn has_line_preview(&self) -> bool {
        self.line_preview.is_some()
    }

    pub fn line_preview_anchor(&self) -> Option<Coord> {
        self.line_preview.as_ref().map(LinePreview::anchor)
    }

    pub fn start_or_advance_line_preview(&mut self) -> bool {
        if self.cursor_mode != CursorMode::MoveDraw {
            return false;
        }
        let Some(preview) = self.line_preview.as_ref() else {
            self.start_line_preview();
            return false;
        };
        if preview.end == preview.anchor() {
            self.finish_line_preview();
            return false;
        }

        let routing = self.toolbar.routing_mode();
        let preview = self
            .line_preview
            .as_mut()
            .expect("line preview remains active while committing");
        preview.segments.push(RoutedSegment {
            end: preview.end,
            routing,
        });
        self.apply_composed_line_preview(false)
    }

    pub fn finish_line_preview(&mut self) -> bool {
        let changed = self
            .line_preview
            .as_ref()
            .is_some_and(|preview| !preview.segments.is_empty());
        if changed {
            self.apply_composed_line_preview(false);
            self.line_preview = None;
            self.active_stroke = None;
        } else {
            self.cancel_line_preview();
        }
        changed
    }

    fn start_line_preview(&mut self) {
        self.end_stroke();
        self.shape_preview = None;
        self.cancel_move_lift();
        self.toolbar.cancel_shortcut();
        self.commit_canvas();
        let source_selection = self.selection;
        self.selection.collapse(self.grid.cursor_pos);
        self.line_preview = Some(LinePreview {
            start: self.grid.cursor_pos,
            segments: Vec::new(),
            end: self.grid.cursor_pos,
            source_canvas: self.canvas.clone(),
            source_cursor: self.grid.cursor_pos,
            source_selection,
            rendered_canvas: self.canvas.clone(),
        });
    }

    pub fn move_line_preview(&mut self, direction: Direction) -> bool {
        if self.line_preview.is_none() {
            return false;
        }
        let Some(prepended) = self.prepare_adjacent(direction) else {
            return false;
        };
        let to = adjacent_coord(self.grid.cursor_pos, direction)
            .expect("canvas edge was structurally extended");
        self.set_line_preview_end(to);
        prepended
    }

    pub fn move_line_preview_to(&mut self, target: Coord) -> bool {
        let target = super::clamp_canvas_coord(target);
        if self.line_preview.is_none() || self.grid.cursor_pos == target {
            return false;
        }
        self.set_line_preview_end(target);
        true
    }

    fn set_line_preview_end(&mut self, target: Coord) {
        self.grid.cursor_pos = target;
        self.selection.collapse(target);
        self.line_preview
            .as_mut()
            .expect("preview remains active while moving")
            .end = target;
        self.refresh_line_preview_render();
    }

    pub fn remove_line_preview_anchor(&mut self) -> bool {
        let Some(preview) = self.line_preview.as_mut() else {
            return false;
        };
        let Some(_) = preview.segments.pop() else {
            return false;
        };
        preview.end = preview.anchor();
        self.apply_composed_line_preview(false);
        self.refresh_line_preview_render();
        true
    }

    pub fn cancel_line_preview(&mut self) -> bool {
        let Some(preview) = self.line_preview.as_ref() else {
            return false;
        };
        if preview.segments.is_empty() {
            let preview = self.line_preview.take().expect("preview exists");
            self.canvas = preview.source_canvas;
            self.grid.cursor_pos = preview.source_cursor;
            self.selection = preview.source_selection;
        } else {
            self.apply_composed_line_preview(false);
            self.line_preview = None;
            self.active_stroke = None;
        }
        true
    }

    #[cfg(test)]
    pub(super) fn lines_with_line_preview(&self) -> Option<Vec<Vec<StyledAtom>>> {
        self.line_preview
            .as_ref()
            .map(|preview| preview.rendered_canvas.active_dense_lines())
    }

    pub(crate) fn line_preview_render_canvas(&self) -> Option<&crate::canvas::LayerStack> {
        self.line_preview
            .as_ref()
            .map(|preview| &preview.rendered_canvas)
    }

    pub(super) fn refresh_line_preview_render(&mut self) {
        let Some(composed) = self.composed_line_preview_state(true) else {
            return;
        };
        if let Some(preview) = self.line_preview.as_mut() {
            preview.rendered_canvas = composed.canvas;
        }
    }

    fn apply_composed_line_preview(&mut self, include_active: bool) -> bool {
        let Some(composed) = self.composed_line_preview_state(include_active) else {
            return false;
        };
        let changed = self.canvas != composed.canvas;
        self.grid.cursor_pos = composed.grid.cursor_pos;
        self.selection.collapse(self.grid.cursor_pos);
        self.canvas = composed.canvas;
        self.active_stroke = None;
        changed
    }

    fn composed_line_preview_state(&self, include_active: bool) -> Option<Self> {
        let preview = self.line_preview.as_ref()?;
        let adjust = |coord: Coord| coord;

        let mut composed = self.clone();
        composed.line_preview = None;
        composed.shape_preview = None;
        composed.move_lift = None;
        composed.active_stroke = None;
        composed.canvas = preview.source_canvas.clone();
        let start = adjust(preview.start);
        composed.grid.cursor_pos = start;
        composed.selection.collapse(start);

        let final_end = if include_active && preview.end != preview.anchor() {
            Some(adjust(preview.end))
        } else {
            preview.segments.last().map(|segment| adjust(segment.end))
        };
        let start_was_existing = composed
            .cell_contents(start)
            .is_some_and(is_routed_line_glyph);
        let end_was_existing = final_end.is_some_and(|end| {
            composed
                .cell_contents(end)
                .is_some_and(is_routed_line_glyph)
        });
        let mut first_direction = None;
        let mut last_direction = None;
        let mut previous_step = None;
        for segment in &preview.segments {
            if let Some((first, last, final_step)) = draw_routed_body(
                &mut composed,
                adjust(segment.end),
                segment.routing,
                previous_step,
            ) {
                first_direction.get_or_insert(first);
                last_direction = Some(last);
                previous_step = Some(final_step);
            }
        }
        if include_active
            && preview.end != preview.anchor()
            && let Some((first, last, _)) = draw_routed_body(
                &mut composed,
                adjust(preview.end),
                self.toolbar.routing_mode(),
                previous_step,
            )
        {
            first_direction.get_or_insert(first);
            last_direction = Some(last);
        }
        if let (Some(end), Some(first), Some(last)) = (final_end, first_direction, last_direction) {
            apply_routed_ending(
                &mut composed,
                start,
                self.toolbar.line_start(),
                first,
                start_was_existing,
            );
            apply_routed_ending(
                &mut composed,
                end,
                self.toolbar.line_end(),
                last,
                end_was_existing,
            );
        }
        composed.end_stroke();
        Some(composed)
    }
}

fn draw_routed_body(
    state: &mut Editor,
    end: Coord,
    routing: RoutingMode,
    preceding_step: Option<RouteStep>,
) -> Option<(Direction, Direction, RouteStep)> {
    let start = state.grid.cursor_pos;
    let steps = route_steps(start, end, routing);
    let directions = steps.first().zip(steps.last()).map(|(first, last)| {
        (
            step_direction(*first, false),
            step_direction(*last, true),
            *last,
        )
    });
    let mut previous = preceding_step;
    for step in &steps {
        let diagonal_corner = previous.is_some_and(|previous| {
            previous != *step
                && (matches!(previous, RouteStep::Diagonal { .. })
                    || matches!(step, RouteStep::Diagonal { .. }))
        });
        match step {
            RouteStep::Orthogonal(direction) => {
                if diagonal_corner {
                    state.write_diagonal_cell(state.grid.cursor_pos, "·", true, true);
                }
                state.move_or_draw_routed(*direction);
            }
            RouteStep::Diagonal {
                horizontal,
                vertical,
            } => draw_diagonal_step(state, *horizontal, *vertical, diagonal_corner),
        }
        previous = Some(*step);
    }
    state.end_stroke();
    directions
}

fn draw_diagonal_step(
    state: &mut Editor,
    horizontal: Direction,
    vertical: Direction,
    starts_at_corner: bool,
) {
    let glyph = if matches!(horizontal, Direction::Right) == matches!(vertical, Direction::Down) {
        "╲"
    } else {
        "╱"
    };
    let from = state.grid.cursor_pos;
    let overwrite_active_endpoint = state
        .active_stroke
        .as_ref()
        .is_some_and(|stroke| stroke.end == from);
    state.end_stroke();
    state.write_diagonal_cell(
        from,
        if starts_at_corner { "·" } else { glyph },
        overwrite_active_endpoint || starts_at_corner,
        true,
    );
    let mut target = state.grid.cursor_pos;
    target.column = match horizontal {
        Direction::Right => target.column + 1,
        Direction::Left => target.column - 1,
        _ => unreachable!(),
    };
    target.line = match vertical {
        Direction::Down => target.line + 1,
        Direction::Up => target.line - 1,
        _ => unreachable!(),
    };
    state.write_diagonal_cell(target, glyph, false, false);
    state.grid.cursor_pos = target;
    state.selection.collapse(target);
    state.end_stroke();
}

fn step_direction(step: RouteStep, reverse: bool) -> Direction {
    let direction = match step {
        RouteStep::Orthogonal(direction) => direction,
        RouteStep::Diagonal { horizontal, .. } => horizontal,
    };
    if reverse {
        direction.opposite()
    } else {
        direction
    }
}

fn is_routed_line_glyph(contents: &str) -> bool {
    is_line_glyph(contents) || matches!(contents, "╱" | "╲")
}

fn apply_routed_ending(
    state: &mut Editor,
    coord: Coord,
    ending: LineEnding,
    connected_direction: Direction,
    was_existing: bool,
) {
    if ending == LineEnding::None || was_existing {
        return;
    }
    let Some(base_glyph) = state
        .cell_contents(coord)
        .filter(|contents| is_routed_line_glyph(contents))
        .map(str::to_owned)
    else {
        return;
    };
    state.apply_line_ending(
        coord,
        ending,
        connected_direction,
        state.toolbar.line_style(),
        &base_glyph,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppConfig;
    use crate::model::ColorId;
    use crate::toolbar::{MainMode, ToolbarAction};

    fn line_editor() -> Editor {
        let mut state = Editor::new(&AppConfig::default().theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
        state
    }

    #[test]
    fn every_route_uses_the_same_composition_for_preview_and_commit() {
        for option in 0..5 {
            let mut state = line_editor();
            state.apply_toolbar_action(ToolbarAction::SelectSubmenu { submenu: 3, option });
            assert!(!state.start_or_advance_line_preview());
            assert!(state.move_line_preview_to(Coord { line: 3, column: 5 }));
            let preview = state
                .lines_with_line_preview()
                .expect("active route is rendered");
            assert!(state.start_or_advance_line_preview());
            assert_eq!(state.lines_for_test(), preview);
        }
    }

    #[test]
    fn route_changes_apply_only_to_the_next_segment_and_cancel_keeps_committed_work() {
        let mut state = line_editor();
        state.start_or_advance_line_preview();
        state.move_line_preview_to(Coord { line: 2, column: 3 });
        assert!(state.start_or_advance_line_preview());
        let first_segment = state.lines_for_test();

        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 3,
            option: 2,
        });
        state.move_line_preview_to(Coord { line: 5, column: 5 });
        let live = state
            .lines_with_line_preview()
            .expect("changed route has a preview");
        assert_ne!(live, first_segment);
        assert!(
            live.iter()
                .flatten()
                .any(|atom| atom.contents == "╲" || atom.contents == "╱")
        );

        assert!(state.cancel_line_preview());
        assert_eq!(state.lines_for_test(), first_segment);
    }

    #[test]
    fn diagonal_routing_reaches_a_secondary_dominant_target() {
        let mut state = line_editor();
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 3,
            option: 2,
        });
        state.start_or_advance_line_preview();
        let target = Coord { line: 6, column: 2 };
        state.move_line_preview_to(target);
        assert!(state.start_or_advance_line_preview());
        assert_eq!(state.grid.cursor_pos, target);
        assert!(
            state
                .lines_for_test()
                .iter()
                .flatten()
                .any(|atom| atom.contents == "╲")
        );
    }

    #[test]
    fn diagonal_routes_use_dots_where_the_direction_changes() {
        let cases = [
            (
                2,
                Coord { line: 2, column: 5 },
                Coord { line: 0, column: 3 },
                "╲",
            ),
            (
                3,
                Coord { line: 2, column: 5 },
                Coord { line: 2, column: 2 },
                "╴",
            ),
        ];

        for (option, target, corner, endpoint) in cases {
            let mut state = line_editor();
            state.apply_toolbar_action(ToolbarAction::SelectSubmenu { submenu: 3, option });
            state.start_or_advance_line_preview();
            state.move_line_preview_to(target);
            assert!(state.start_or_advance_line_preview());

            assert_eq!(state.cell_contents(corner), Some("·"));
            assert_eq!(state.cell_contents(target), Some(endpoint));
        }
    }

    #[test]
    fn changing_diagonal_direction_at_an_anchor_uses_a_dot() {
        let mut state = line_editor();
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 3,
            option: 2,
        });
        state.start_or_advance_line_preview();
        let anchor = Coord { line: 2, column: 2 };
        state.move_line_preview_to(anchor);
        assert!(state.start_or_advance_line_preview());

        state.move_line_preview_to(Coord { line: 4, column: 0 });
        assert!(state.start_or_advance_line_preview());

        assert_eq!(state.cell_contents(anchor), Some("·"));
    }

    #[test]
    fn mixed_diagonal_route_colors_every_cell_and_decorates_only_true_endpoints() {
        let mut state = line_editor();
        let color = ColorId(9);
        let expected_foreground = color.hex().expect("test color is valid").to_owned();
        state.apply_toolbar_action(ToolbarAction::Toggle(
            crate::toolbar::ToggleKind::MultiColorMode,
        ));
        state.apply_toolbar_action(ToolbarAction::SelectColor(color));
        for submenu in [0, 1] {
            state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
                submenu,
                option: 11,
            });
        }
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 3,
            option: 2,
        });

        state.start_or_advance_line_preview();
        let target = Coord { line: 5, column: 2 };
        state.move_line_preview_to(target);
        let preview = state
            .lines_with_line_preview()
            .expect("mixed route is previewed");
        assert_eq!(
            preview
                .iter()
                .flatten()
                .filter(|atom| atom.contents == "◆")
                .count(),
            2
        );
        assert!(preview.iter().flatten().any(|atom| atom.contents == "╲"));
        assert!(
            preview
                .iter()
                .flatten()
                .any(|atom| is_line_glyph(&atom.contents))
        );
        assert!(preview.iter().flatten().all(|atom| {
            atom.contents.chars().all(char::is_whitespace) || atom.face.fg == expected_foreground
        }));

        assert!(state.start_or_advance_line_preview());
        assert_eq!(state.lines_for_test(), preview);
        assert_eq!(state.line_markers_for_test().len(), 2);
        assert_eq!(state.line_markers_for_test()[0].coord, Coord::default());
        assert_eq!(state.line_markers_for_test()[1].coord, target);
        assert!(
            state
                .line_markers_for_test()
                .iter()
                .all(|marker| marker.ending == LineEnding::Fixed('◆'))
        );
    }

    #[test]
    fn mixed_routing_across_anchors_decorates_only_the_overall_line_endpoints() {
        let mut state = line_editor();
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 11,
        });
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 1,
            option: 12,
        });
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 3,
            option: 2,
        });

        state.start_or_advance_line_preview();
        let anchor = Coord { line: 2, column: 5 };
        state.move_line_preview_to(anchor);
        assert!(state.start_or_advance_line_preview());

        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 3,
            option: 1,
        });
        let endpoint = Coord { line: 5, column: 8 };
        state.move_line_preview_to(endpoint);
        let preview = state
            .lines_with_line_preview()
            .expect("both routed segments are previewed");
        assert!(preview.iter().flatten().any(|atom| atom.contents == "╲"));
        assert_eq!(
            preview
                .iter()
                .flatten()
                .filter(|atom| matches!(atom.contents.as_str(), "◆" | "◊"))
                .count(),
            2
        );

        assert!(state.start_or_advance_line_preview());
        assert_eq!(state.lines_for_test(), preview);
        assert_eq!(state.line_markers_for_test().len(), 2);
        assert_eq!(state.line_markers_for_test()[0].coord, Coord::default());
        assert_eq!(
            state.line_markers_for_test()[0].ending,
            LineEnding::Fixed('◆')
        );
        assert_eq!(state.line_markers_for_test()[1].coord, endpoint);
        assert_eq!(
            state.line_markers_for_test()[1].ending,
            LineEnding::Fixed('◊')
        );
        assert!(
            !state
                .line_markers_for_test()
                .iter()
                .any(|marker| marker.coord == anchor)
        );
        assert!(
            state
                .cell_contents(Coord { line: 5, column: 5 })
                .is_some_and(is_line_glyph)
        );
    }
}
