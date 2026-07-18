use super::TOOLTIP_ROTATION_INTERVAL;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MainMode {
    #[default]
    Stamp,
    Line,
    Shapes,
    Utilities,
    Layers,
    Colors,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShapeKind {
    #[default]
    Rect,
    RoundedRect,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UtilityKind {
    #[default]
    Push,
    Pull,
    View,
}

impl MainMode {
    pub const ALL: [Self; 4] = [Self::Stamp, Self::Line, Self::Shapes, Self::Utilities];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::Stamp => "Stamp",
            Self::Shapes => "Shape",
            Self::Utilities => "Utils",
            Self::Layers => "Layers",
            Self::Colors => "Colors",
        }
    }

    pub fn tooltip(self) -> Tooltip {
        match self {
            Self::Line => Tooltip::Line,
            Self::Stamp => Tooltip::Stamp,
            Self::Shapes => Tooltip::Shapes,
            Self::Utilities => Tooltip::UtilitiesPush,
            Self::Layers => Tooltip::Layers,
            Self::Colors => Tooltip::Colors,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tooltip {
    #[default]
    None,
    Line,
    Stamp,
    Shapes,
    UtilitiesPush,
    UtilitiesPull,
    UtilitiesView,
    SelectionMoveLift,
    LinePreview,
    ShapePreview,
    SingleReplace,
    LineStroke,
    Text,
    Replace,
    Export,
    Layers,
    Colors,
    Selection,
    Jump,
}

impl Tooltip {
    pub fn text(self) -> String {
        const MISC_TIP: [&str; 7] = [
            "Canvas: u undo; U redo; Ctrl/Cmd-Z undo; Ctrl/Cmd-R redo",
            "Direction keys are ←→↓↑ and hjkl",
            "m opens a directional sector grid for jumping",
            "When drawing/selecting/resizing add Ctrl/Alt/Shift for 5/10 steps",
            "Alt-direction erases",
            "Shift-direction selects",
            "Esc, Ctrl-C, or Ctrl-G cancels the current mode",
        ];
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        let primary = match self {
            Self::None => "",
            Self::Line => "Line: Space starts a preview; Ctrl-direction draws",
            Self::Stamp => "Stamp: Space places; Ctrl-direction draws continuously",
            Self::Shapes => "Shape: Space starts a preview",
            Self::UtilitiesPush => "Push: Ctrl-direction inserts a blank row or column",
            Self::UtilitiesPull => "Pull: Ctrl-direction pulls",
            Self::UtilitiesView => "View: directions pan; Space centers",
            Self::SelectionMoveLift => {
                "Selection move: Alt-direction repositions; direction confirms and moves; Space/Enter confirms"
            }
            Self::LinePreview => {
                "Space anchors; Space again confirms; Backspace removes the last anchor"
            }
            Self::ShapePreview => "Shape preview: Space confirms",
            Self::SingleReplace => "Replace selection: type or paste one character",
            Self::LineStroke => "Line stroke: Ctrl-direction continues; release Ctrl to finish",
            Self::Text => "<Ret> exits text mode; arrows move freely over the canvas",
            Self::Replace => "<Esc>/Ctrl-C/Ctrl-G to exit replace mode",
            Self::Export => {
                "Files/Togls: copy/export, save/load files, clear, or toggle display modes"
            }
            Self::Layers => "Lyrs: select, show, reorder, add, or delete a layer",
            Self::Colors => "Clrs: select the foreground color for future writes",
            Self::Selection => {
                "Selection: Alt-direction lifts and moves; Shift-direction expands; cancel key collapses; Backspace clears; r then KEY replaces"
            }
            Self::Jump => {
                "Jump: direction lands cursor; Shift lands selection; edges pan; Esc cancels"
            }
        };
        if matches!(
            self,
            Self::SelectionMoveLift
                | Self::LinePreview
                | Self::ShapePreview
                | Self::SingleReplace
                | Self::LineStroke
                | Self::Export
                | Self::Layers
                | Self::Colors
                | Self::Selection
                | Self::Jump
        ) {
            return primary.to_string();
        }
        let selector = (timestamp / TOOLTIP_ROTATION_INTERVAL.as_secs() as usize) % MISC_TIP.len();
        let random_tip = MISC_TIP[selector];
        let misc = if primary.len() + random_tip.len() > 80 {
            ""
        } else {
            random_tip
        };
        let secondary = if primary.is_empty() || misc.is_empty() {
            ""
        } else {
            "; "
        };
        format!("{primary}{secondary}{misc}")
    }
}
