use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationRequest {
    pub id: u64,
    #[serde(flatten)]
    pub command: AutomationCommand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum AutomationCommand {
    Ping,
    Key {
        key: String,
        #[serde(default)]
        modifiers: KeyModifiers,
        #[serde(default)]
        repeat: bool,
        #[serde(default = "default_one")]
        count: u32,
    },
    Text {
        text: String,
    },
    Scroll {
        x: f32,
        y: f32,
        #[serde(default = "default_one")]
        steps: u32,
    },
    Zoom {
        delta: f32,
    },
    State,
    Metrics {
        #[serde(default)]
        reset: bool,
    },
    Screenshot {
        path: PathBuf,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyModifiers {
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub control: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default, rename = "super")]
    pub super_key: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationResponse {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AutomationResponse {
    pub fn success(id: u64, result: impl Into<Value>) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result.into()),
            error: None,
        }
    }

    pub fn error(id: u64, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}

const fn default_one() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{AutomationCommand, AutomationRequest, AutomationResponse, KeyModifiers};

    fn round_trip(request: AutomationRequest) {
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<AutomationRequest>(&json).unwrap(),
            request
        );
        assert!(!json.contains("\"command\":{"));
    }

    #[test]
    fn requests_round_trip_for_every_command() {
        let commands = [
            AutomationCommand::Ping,
            AutomationCommand::Key {
                key: "ArrowLeft".into(),
                modifiers: KeyModifiers {
                    shift: true,
                    control: true,
                    alt: false,
                    super_key: true,
                },
                repeat: true,
                count: 3,
            },
            AutomationCommand::Text {
                text: "hello\nworld".into(),
            },
            AutomationCommand::Scroll {
                x: -1.5,
                y: 2.25,
                steps: 4,
            },
            AutomationCommand::Zoom { delta: 1.0 },
            AutomationCommand::State,
            AutomationCommand::Metrics { reset: true },
            AutomationCommand::Screenshot {
                path: PathBuf::from("/tmp/ascdraw.png"),
            },
            AutomationCommand::Shutdown,
        ];

        for (id, command) in commands.into_iter().enumerate() {
            round_trip(AutomationRequest {
                id: id as u64,
                command,
            });
        }
    }

    #[test]
    fn key_defaults_are_applied() {
        let request: AutomationRequest =
            serde_json::from_value(json!({"id": 7, "command": "key", "key": "a"})).unwrap();

        assert_eq!(
            request.command,
            AutomationCommand::Key {
                key: "a".into(),
                modifiers: KeyModifiers::default(),
                repeat: false,
                count: 1,
            }
        );
    }

    #[test]
    fn key_modifiers_use_protocol_field_names() {
        let request = AutomationRequest {
            id: 16,
            command: AutomationCommand::Key {
                key: "z".into(),
                modifiers: KeyModifiers {
                    super_key: true,
                    ..KeyModifiers::default()
                },
                repeat: false,
                count: 1,
            },
        };

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({
                "id": 16,
                "command": "key",
                "key": "z",
                "modifiers": {
                    "shift": false,
                    "control": false,
                    "alt": false,
                    "super": true
                },
                "repeat": false,
                "count": 1
            })
        );
    }

    #[test]
    fn individual_modifier_fields_default_to_false() {
        let request: AutomationRequest = serde_json::from_value(json!({
            "id": 8,
            "command": "key",
            "key": "x",
            "modifiers": {"control": true}
        }))
        .unwrap();

        let AutomationCommand::Key { modifiers, .. } = request.command else {
            panic!("expected key command");
        };
        assert_eq!(
            modifiers,
            KeyModifiers {
                control: true,
                ..KeyModifiers::default()
            }
        );
    }

    #[test]
    fn scroll_and_metrics_defaults_are_applied() {
        let scroll: AutomationRequest =
            serde_json::from_value(json!({"id": 9, "command": "scroll", "x": 1.0, "y": -2.0}))
                .unwrap();
        let metrics: AutomationRequest =
            serde_json::from_value(json!({"id": 10, "command": "metrics"})).unwrap();

        assert_eq!(
            scroll.command,
            AutomationCommand::Scroll {
                x: 1.0,
                y: -2.0,
                steps: 1,
            }
        );
        assert_eq!(metrics.command, AutomationCommand::Metrics { reset: false });
    }

    #[test]
    fn response_constructors_round_trip_and_omit_empty_fields() {
        let success = AutomationResponse::success(11, json!({"status": "pong"}));
        let error = AutomationResponse::error(12, "bad key");

        let success_json = serde_json::to_value(&success).unwrap();
        let error_json = serde_json::to_value(&error).unwrap();
        assert_eq!(
            success_json,
            json!({"id": 11, "ok": true, "result": {"status": "pong"}})
        );
        assert_eq!(
            error_json,
            json!({"id": 12, "ok": false, "error": "bad key"})
        );
        assert_eq!(
            serde_json::from_value::<AutomationResponse>(success_json).unwrap(),
            success
        );
        assert_eq!(
            serde_json::from_value::<AutomationResponse>(error_json).unwrap(),
            error
        );
    }

    #[test]
    fn response_optional_fields_default_to_none() {
        let response: AutomationResponse =
            serde_json::from_value(json!({"id": 13, "ok": true})).unwrap();

        assert_eq!(response.result, None);
        assert_eq!(response.error, None);
    }

    #[test]
    fn response_result_accepts_any_json_value() {
        for result in [json!(true), json!(42), json!("text"), json!([1, 2])] {
            let response = AutomationResponse::success(14, result.clone());
            let decoded: AutomationResponse =
                serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap();
            assert_eq!(decoded.result, Some(result));
        }

        let null = AutomationResponse::success(15, serde_json::Value::Null);
        let decoded: AutomationResponse =
            serde_json::from_str(&serde_json::to_string(&null).unwrap()).unwrap();
        assert_eq!(decoded.result, None);
    }
}
