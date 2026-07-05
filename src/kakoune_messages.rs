use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const NOTIFICATION_METHODS: &[&str] = &[
    "draw",
    "draw_status",
    "menu_show",
    "menu_select",
    "menu_hide",
    "info_show",
    "info_hide",
    "set_ui_options",
    "refresh",
];

pub const REQUEST_METHODS: &[&str] = &[
    "keys",
    "paste",
    "resize",
    "scroll",
    "mouse_move",
    "mouse_press",
    "mouse_release",
    "menu_select",
];

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Face {
    #[serde(default = "default_color")]
    pub fg: String,
    #[serde(default = "default_color")]
    pub bg: String,
    #[serde(default = "default_color")]
    pub underline: String,
    #[serde(default)]
    pub attributes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Atom {
    pub face: Face,
    pub contents: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Coord {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MenuStyle {
    Prompt,
    Search,
    Inline,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InfoStyle {
    Prompt,
    Inline,
    InlineAbove,
    InlineBelow,
    MenuDoc,
    Modal,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StatusStyle {
    Status,
    Command,
    Search,
    Prompt,
}

#[derive(Debug, Clone)]
pub enum KakouneNotification {
    Draw {
        lines: Vec<Vec<Atom>>,
        cursor_pos: Coord,
        default_face: Face,
        padding_face: Face,
        widget_columns: usize,
    },
    DrawStatus {
        prompt: Vec<Atom>,
        content: Vec<Atom>,
        cursor_pos: isize,
        mode_line: Vec<Atom>,
        default_face: Face,
        style: StatusStyle,
    },
    MenuShow {
        items: Vec<Vec<Atom>>,
        anchor: Coord,
        selected_face: Face,
        menu_face: Face,
        style: MenuStyle,
    },
    MenuSelect {
        selected: isize,
    },
    MenuHide,
    InfoShow {
        title: Vec<Atom>,
        content: Vec<Vec<Atom>>,
        anchor: Coord,
        face: Face,
        style: InfoStyle,
    },
    InfoHide,
    SetUiOptions {
        options: serde_json::Map<String, Value>,
    },
    Refresh {
        force: bool,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButtonName {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone)]
pub enum KakouneRequest {
    Keys {
        keys: Vec<String>,
    },
    Paste {
        content: String,
    },
    Resize {
        rows: usize,
        columns: usize,
    },
    Scroll {
        amount: i32,
        coord: Coord,
    },
    MouseMove {
        coord: Coord,
    },
    MousePress {
        button: MouseButtonName,
        coord: Coord,
    },
    MouseRelease {
        button: MouseButtonName,
        coord: Coord,
    },
    MenuSelect {
        index: isize,
    },
}

#[derive(Debug, Deserialize)]
struct RpcEnvelope {
    method: String,
    params: Vec<Value>,
}

fn default_color() -> String {
    "default".to_string()
}

pub fn parse_notification(line: &str) -> Result<KakouneNotification> {
    let envelope: RpcEnvelope = serde_json::from_str(line)?;
    match envelope.method.as_str() {
        "draw" => {
            let (lines, cursor_pos, default_face, padding_face, widget_columns): (
                Vec<Vec<Atom>>,
                Coord,
                Face,
                Face,
                usize,
            ) = deserialize_params(envelope.params)?;
            Ok(KakouneNotification::Draw {
                lines,
                cursor_pos,
                default_face,
                padding_face,
                widget_columns,
            })
        }
        "draw_status" => {
            let (prompt, content, cursor_pos, mode_line, default_face, style): (
                Vec<Atom>,
                Vec<Atom>,
                isize,
                Vec<Atom>,
                Face,
                StatusStyle,
            ) = deserialize_params(envelope.params)?;
            Ok(KakouneNotification::DrawStatus {
                prompt,
                content,
                cursor_pos,
                mode_line,
                default_face,
                style,
            })
        }
        "menu_show" => {
            let (items, anchor, selected_face, menu_face, style): (
                Vec<Vec<Atom>>,
                Coord,
                Face,
                Face,
                MenuStyle,
            ) = deserialize_params(envelope.params)?;
            Ok(KakouneNotification::MenuShow {
                items,
                anchor,
                selected_face,
                menu_face,
                style,
            })
        }
        "menu_select" => {
            let (selected,): (isize,) = deserialize_params(envelope.params)?;
            Ok(KakouneNotification::MenuSelect { selected })
        }
        "menu_hide" => Ok(KakouneNotification::MenuHide),
        "info_show" => {
            let (title, content, anchor, face, style): (
                Vec<Atom>,
                Vec<Vec<Atom>>,
                Coord,
                Face,
                InfoStyle,
            ) = deserialize_params(envelope.params)?;
            Ok(KakouneNotification::InfoShow {
                title,
                content,
                anchor,
                face,
                style,
            })
        }
        "info_hide" => Ok(KakouneNotification::InfoHide),
        "set_ui_options" => {
            let (options,): (serde_json::Map<String, Value>,) =
                deserialize_params(envelope.params)?;
            Ok(KakouneNotification::SetUiOptions { options })
        }
        "refresh" => {
            let (force,): (bool,) = deserialize_params(envelope.params)?;
            Ok(KakouneNotification::Refresh { force })
        }
        other => bail!("unsupported rpc method {other}"),
    }
}

fn deserialize_params<T>(params: Vec<Value>) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    Ok(serde_json::from_value(Value::Array(params))?)
}

impl KakouneRequest {
    pub fn to_json_line(&self) -> String {
        let (method, params) = match self {
            KakouneRequest::Keys { keys } => (
                "keys",
                Value::Array(keys.iter().cloned().map(Value::String).collect()),
            ),
            KakouneRequest::Paste { content } => ("paste", json!([content])),
            KakouneRequest::Resize { rows, columns } => ("resize", json!([rows, columns])),
            KakouneRequest::Scroll { amount, coord } => {
                ("scroll", json!([amount, coord.line, coord.column]))
            }
            KakouneRequest::MouseMove { coord } => {
                ("mouse_move", json!([coord.line, coord.column]))
            }
            KakouneRequest::MousePress { button, coord } => {
                ("mouse_press", json!([button, coord.line, coord.column]))
            }
            KakouneRequest::MouseRelease { button, coord } => {
                ("mouse_release", json!([button, coord.line, coord.column]))
            }
            KakouneRequest::MenuSelect { index } => ("menu_select", json!([index])),
        };

        json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_methods_cover_kakoune_json_ui_stdout_surface() {
        assert_eq!(
            NOTIFICATION_METHODS,
            &[
                "draw",
                "draw_status",
                "menu_show",
                "menu_select",
                "menu_hide",
                "info_show",
                "info_hide",
                "set_ui_options",
                "refresh",
            ]
        );
    }

    #[test]
    fn request_methods_cover_kakoune_json_ui_stdin_surface() {
        assert_eq!(
            REQUEST_METHODS,
            &[
                "keys",
                "paste",
                "resize",
                "scroll",
                "mouse_move",
                "mouse_press",
                "mouse_release",
                "menu_select",
            ]
        );
    }

    #[test]
    fn parses_menu_show_payload() {
        let notification = parse_notification(
            r#"{"jsonrpc":"2.0","method":"menu_show","params":[[[{"face":{"fg":"default","bg":"default","underline":"default","attributes":[]},"contents":"item"}]],{"line":1,"column":2},{"fg":"white","bg":"blue","underline":"default","attributes":[]},{"fg":"black","bg":"white","underline":"default","attributes":[]},"prompt"]}"#,
        )
        .unwrap();

        match notification {
            KakouneNotification::MenuShow {
                items,
                anchor,
                style,
                ..
            } => {
                assert_eq!(items.len(), 1);
                assert_eq!(anchor, Coord { line: 1, column: 2 });
                assert_eq!(style, MenuStyle::Prompt);
            }
            other => panic!("unexpected notification: {other:?}"),
        }
    }

    #[test]
    fn parses_info_show_payload() {
        let notification = parse_notification(
            r#"{"jsonrpc":"2.0","method":"info_show","params":[[{"face":{"fg":"default","bg":"default","underline":"default","attributes":[]},"contents":"title"}],[[{"face":{"fg":"default","bg":"default","underline":"default","attributes":[]},"contents":"body"}]],{"line":3,"column":4},{"fg":"white","bg":"black","underline":"default","attributes":[]},"modal"]}"#,
        )
        .unwrap();

        match notification {
            KakouneNotification::InfoShow {
                title,
                content,
                anchor,
                style,
                ..
            } => {
                assert_eq!(title.len(), 1);
                assert_eq!(content.len(), 1);
                assert_eq!(anchor, Coord { line: 3, column: 4 });
                assert_eq!(style, InfoStyle::Modal);
            }
            other => panic!("unexpected notification: {other:?}"),
        }
    }

    #[test]
    fn serializes_mouse_press_request() {
        let request = KakouneRequest::MousePress {
            button: MouseButtonName::Left,
            coord: Coord { line: 7, column: 9 },
        };

        assert_eq!(
            request.to_json_line(),
            r#"{"jsonrpc":"2.0","method":"mouse_press","params":["left",7,9]}"#
        );
    }
}
