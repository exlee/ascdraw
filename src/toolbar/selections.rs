use serde::{Deserialize, Serialize};

use crate::model::ColorId;

use super::{
    LINE_OPTIONS, MainMode, RoutingMode, SHAPE_OPTIONS, STAMP_LABELS, STAMP_OPTIONS, ToolbarState,
    UTILITY_OPTIONS,
};

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct DurableMenuSelections {
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    main_mode: Option<String>,
    #[serde(default)]
    line: DurableLineSelections,
    #[serde(default)]
    stamp: DurableStampSelections,
    #[serde(default)]
    shape: DurableShapeSelections,
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    utility: Option<String>,
    #[serde(default)]
    toggles: DurableToggleSelections,
    #[serde(default)]
    active_color: ColorId,
}

impl DurableMenuSelections {
    pub fn active_color(&self) -> ColorId {
        self.active_color
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
struct DurableToggleSelections {
    dark_mode: bool,
    multi_color_mode: bool,
    multi_layer_mode: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
struct DurableLineSelections {
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub start: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub end: Option<String>,
    #[serde(
        alias = "width",
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub style: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub routing: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub corner: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
struct DurableStampSelections {
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub active_family: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub decorators: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub arrows: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub fills: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub blocks: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
struct DurableShapeSelections {
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub kind: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub line: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub fill: Option<String>,
}

fn optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrIgnored {
        String(String),
        Ignored(serde::de::IgnoredAny),
    }

    Ok(match StringOrIgnored::deserialize(deserializer)? {
        StringOrIgnored::String(value) => Some(value),
        StringOrIgnored::Ignored(_) => None,
    })
}

impl ToolbarState {
    pub fn durable_selections(&self) -> DurableMenuSelections {
        DurableMenuSelections {
            main_mode: Some(main_mode_name(self.main_mode).to_owned()),
            line: DurableLineSelections {
                start: selected_value(&LINE_OPTIONS, 0, self.line_selected[0]),
                end: selected_value(&LINE_OPTIONS, 1, self.line_selected[1]),
                style: selected_value(&LINE_OPTIONS, 2, self.line_selected[2]),
                routing: Some(self.routing_mode().durable_name().to_owned()),
                corner: selected_value(&LINE_OPTIONS, 4, self.line_selected[4]),
            },
            stamp: DurableStampSelections {
                active_family: STAMP_LABELS
                    .get(self.stamp_active_category)
                    .map(|value| (*value).to_owned()),
                decorators: selected_value(&STAMP_OPTIONS, 0, self.stamp_selected[0]),
                arrows: selected_value(&STAMP_OPTIONS, 1, self.stamp_selected[1]),
                fills: selected_value(&STAMP_OPTIONS, 2, self.stamp_selected[2]),
                blocks: selected_value(&STAMP_OPTIONS, 3, self.stamp_selected[3]),
            },
            shape: DurableShapeSelections {
                kind: selected_value(&SHAPE_OPTIONS, 0, self.shape_selected[0]),
                line: selected_value(&SHAPE_OPTIONS, 1, self.shape_selected[1]),
                fill: selected_value(&SHAPE_OPTIONS, 2, self.shape_selected[2]),
            },
            utility: selected_value(&UTILITY_OPTIONS, 0, self.utility_selected),
            toggles: DurableToggleSelections {
                dark_mode: self.dark_mode(),
                multi_color_mode: self.multi_color_mode(),
                multi_layer_mode: self.multi_layer_mode(),
            },
            active_color: self.active_color(),
        }
    }

    pub fn restore_durable_selections(&mut self, selections: &DurableMenuSelections) {
        // Resetting first deliberately drops every transient menu interaction.
        *self = Self::default();

        restore_selected(
            &mut self.line_selected[0],
            LINE_OPTIONS[0],
            &selections.line.start,
        );
        restore_selected(
            &mut self.line_selected[1],
            LINE_OPTIONS[1],
            &selections.line.end,
        );
        restore_line_style(&mut self.line_selected[2], &selections.line.style);
        if let Some(routing) = selections
            .line
            .routing
            .as_deref()
            .and_then(RoutingMode::from_durable_name)
        {
            self.line_selected[3] = routing.index();
        }
        restore_selected(
            &mut self.line_selected[4],
            LINE_OPTIONS[4],
            &selections.line.corner,
        );

        let stamp_values = [
            &selections.stamp.decorators,
            &selections.stamp.arrows,
            &selections.stamp.fills,
            &selections.stamp.blocks,
        ];
        for (selected, (options, value)) in self
            .stamp_selected
            .iter_mut()
            .zip(STAMP_OPTIONS.iter().zip(stamp_values))
        {
            restore_selected(selected, options, value);
        }
        if let Some(active_family) = selections.stamp.active_family.as_deref()
            && let Some(index) = STAMP_LABELS
                .iter()
                .position(|label| *label == active_family)
        {
            self.stamp_active_category = index;
        }

        restore_selected(
            &mut self.shape_selected[0],
            SHAPE_OPTIONS[0],
            &selections.shape.kind,
        );
        restore_selected(
            &mut self.shape_selected[1],
            SHAPE_OPTIONS[1],
            &selections.shape.line,
        );
        restore_selected(
            &mut self.shape_selected[2],
            SHAPE_OPTIONS[2],
            &selections.shape.fill,
        );
        restore_selected(
            &mut self.utility_selected,
            UTILITY_OPTIONS[0],
            &selections.utility,
        );
        self.toggles = [
            selections.toggles.dark_mode,
            selections.toggles.multi_color_mode,
            selections.toggles.multi_layer_mode,
        ];
        if selections.active_color.is_valid() {
            self.active_color = selections.active_color;
        }

        if let Some(main_mode) = selections.main_mode.as_deref().and_then(parse_main_mode)
            && self.available_modes().contains(&main_mode)
        {
            self.main_mode = main_mode;
        }
    }
}

fn selected_value(options: &[&[&str]], category: usize, selected: usize) -> Option<String> {
    options
        .get(category)
        .and_then(|options| options.get(selected))
        .map(|value| (*value).to_owned())
}

fn restore_selected(selected: &mut usize, options: &[&str], value: &Option<String>) {
    if let Some(value) = value.as_deref()
        && let Some(index) = options.iter().position(|option| *option == value)
    {
        *selected = index;
    }
}

fn restore_line_style(selected: &mut usize, value: &Option<String>) {
    let value = value
        .as_deref()
        .map(|style| if style == "┄" { "╴" } else { style });
    if let Some(index) =
        value.and_then(|style| LINE_OPTIONS[2].iter().position(|item| *item == style))
    {
        *selected = index;
    }
}

fn main_mode_name(mode: MainMode) -> &'static str {
    match mode {
        MainMode::Line => "line",
        MainMode::Stamp => "stamp",
        MainMode::Shapes => "shapes",
        MainMode::Utilities => "utilities",
        MainMode::Layers => "layers",
        MainMode::Colors => "colors",
    }
}

fn parse_main_mode(value: &str) -> Option<MainMode> {
    match value {
        "line" => Some(MainMode::Line),
        "stamp" => Some(MainMode::Stamp),
        "shapes" => Some(MainMode::Shapes),
        "utilities" => Some(MainMode::Utilities),
        "layers" => Some(MainMode::Layers),
        "colors" => Some(MainMode::Colors),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::{ToolbarAction, UtilityKind};
    use winit::keyboard::{Key, ModifiersState};

    #[test]
    fn durable_selections_round_trip_every_field_and_drop_transients() {
        let mut source = ToolbarState::default();
        for (mode, selections) in [
            (MainMode::Line, &[26, 25, 3, 4, 1][..]),
            (MainMode::Stamp, &[19, 21, 3, 14][..]),
            (MainMode::Shapes, &[1, 2, 4][..]),
        ] {
            source.apply_action(ToolbarAction::SelectMain(mode));
            for (submenu, option) in selections.iter().copied().enumerate() {
                source.apply_action(ToolbarAction::SelectSubmenu { submenu, option });
            }
        }
        source.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        source.apply_action(ToolbarAction::SelectSubmenu {
            submenu: 2,
            option: 3,
        });
        source.apply_action(ToolbarAction::SelectMain(MainMode::Utilities));
        source.apply_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 0,
        });
        assert_eq!(source.utility_kind(), UtilityKind::Push);
        source.apply_action(ToolbarAction::Toggle(
            crate::toolbar::ToggleKind::MultiColorMode,
        ));
        source.apply_action(ToolbarAction::SelectColor(ColorId(15)));
        source.apply_action(ToolbarAction::SelectMain(MainMode::Colors));
        let expected = source.durable_selections();

        source.handle_shortcut(&Key::Character("0".into()), ModifiersState::empty());
        assert!(source.export_menu_open());
        let mut restored = source;
        restored.restore_durable_selections(&expected);

        assert_eq!(restored.durable_selections(), expected);
        assert_eq!(restored.routing_mode(), RoutingMode::Stairs);
        assert!(!restored.export_menu_open());
        assert_eq!(restored.pending_shortcut(), None);
        assert_eq!(restored.pending_export_action, None);
        assert_eq!(restored.active_color(), ColorId(15));
        assert_eq!(restored.main_mode(), MainMode::Colors);
    }

    #[test]
    fn partial_invalid_values_fall_back_field_by_field() {
        let defaults = ToolbarState::default().durable_selections();
        let partial: DurableMenuSelections = toml::from_str(
            r#"
main-mode = 42
utility = "View"

[line]
start = "¤"
end = 5

[stamp]
active-family = "Arrows"
arrows = "↔"
blocks = "not-a-block"

[shape]
kind = "Round"
line = "not-a-line"
"#,
        )
        .unwrap();
        let mut toolbar = ToolbarState::default();
        toolbar.restore_durable_selections(&partial);
        let restored = toolbar.durable_selections();

        assert_eq!(restored.main_mode, defaults.main_mode);
        assert_eq!(restored.line.start.as_deref(), Some("¤"));
        assert_eq!(restored.line.end, defaults.line.end);
        assert_eq!(restored.line.style, defaults.line.style);
        assert_eq!(restored.line.routing, defaults.line.routing);
        assert_eq!(restored.stamp.active_family.as_deref(), Some("Arrows"));
        assert_eq!(restored.stamp.arrows.as_deref(), Some("↔"));
        assert_eq!(restored.stamp.blocks, defaults.stamp.blocks);
        assert_eq!(restored.shape.kind.as_deref(), Some("Round"));
        assert_eq!(restored.shape.line, defaults.shape.line);
        assert_eq!(restored.utility.as_deref(), Some("View"));
    }

    #[test]
    fn legacy_line_width_restores_as_style_and_serializes_with_the_new_name() {
        let selections: DurableMenuSelections = toml::from_str(
            r#"
[line]
width = "┄"
"#,
        )
        .unwrap();
        let mut toolbar = ToolbarState::default();

        toolbar.restore_durable_selections(&selections);

        assert_eq!(toolbar.line_style(), crate::drawing::LineStyle::Dashed);
        let serialized = toml::to_string(&toolbar.durable_selections()).unwrap();
        assert!(serialized.contains("style = \"╴\""));
        assert!(!serialized.contains("width ="));
    }

    #[test]
    fn routing_uses_stable_typed_durable_names() {
        let selections: DurableMenuSelections = toml::from_str(
            r#"
[line]
routing = "vertical-diagonal"
"#,
        )
        .unwrap();
        let mut toolbar = ToolbarState::default();
        toolbar.restore_durable_selections(&selections);

        assert_eq!(toolbar.routing_mode(), RoutingMode::VerticalDiagonal);
        let serialized = toml::to_string(&toolbar.durable_selections()).unwrap();
        assert!(serialized.contains("routing = \"vertical-diagonal\""));
    }
}
