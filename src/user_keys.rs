use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use crate::app::bundled_default_keys;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontSizeAction {
    Increase,
    Decrease,
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UserAction {
    FontSize(FontSizeAction),
    WindowNew,
    WindowClose,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "kebab-case")]
pub struct UserKeysConfig {
    pub font_scale_up: String,
    pub font_scale_down: String,
    pub font_scale_reset: String,
    pub window_new: String,
    pub window_close: String,
}

impl Default for UserKeysConfig {
    fn default() -> Self {
        bundled_default_keys()
    }
}

#[derive(Debug, Clone)]
pub struct UserKeys {
    bindings: Vec<(Binding, UserAction)>,
}

impl UserKeys {
    pub fn from_config(config: &UserKeysConfig) -> Result<Self> {
        Ok(Self {
            bindings: vec![
                (
                    Binding::parse(&config.font_scale_up)?,
                    UserAction::FontSize(FontSizeAction::Increase),
                ),
                (
                    Binding::parse(&config.font_scale_down)?,
                    UserAction::FontSize(FontSizeAction::Decrease),
                ),
                (
                    Binding::parse(&config.font_scale_reset)?,
                    UserAction::FontSize(FontSizeAction::Reset),
                ),
                (Binding::parse(&config.window_new)?, UserAction::WindowNew),
                (
                    Binding::parse(&config.window_close)?,
                    UserAction::WindowClose,
                ),
            ],
        })
    }

    pub fn action_for_event(
        &self,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> Option<UserAction> {
        let key = event.key_without_modifiers();
        self.bindings
            .iter()
            .find_map(|(binding, action)| binding.matches(&key, modifiers).then_some(*action))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding {
    modifiers: BindingModifiers,
    key: BindingKey,
}

impl Binding {
    fn parse(text: &str) -> Result<Self> {
        let (modifier_text, key_text) = split_binding(text)?;
        let mut modifiers = BindingModifiers::default();

        for token in modifier_text.split('-').filter(|token| !token.is_empty()) {
            modifiers.apply_token(token)?;
        }

        Ok(Self {
            modifiers,
            key: BindingKey::parse(key_text)?,
        })
    }

    fn matches(&self, key: &Key, modifiers: ModifiersState) -> bool {
        self.modifiers.matches(modifiers) && self.key.matches(key)
    }
}

fn split_binding(text: &str) -> Result<(&str, &str)> {
    if text.is_empty() {
        bail!("key binding cannot be empty");
    }

    if text == "-" {
        return Ok(("", "-"));
    }

    if let Some((modifier_text, key_text)) = text.rsplit_once('-') {
        if key_text.is_empty() {
            if modifier_text.is_empty() {
                return Ok(("", "-"));
            }
            return Ok((modifier_text.trim_end_matches('-'), "-"));
        }

        return Ok((modifier_text, key_text));
    }

    Ok(("", text))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BindingModifiers {
    shift: bool,
    alt: bool,
    control: bool,
    super_key: bool,
}

impl BindingModifiers {
    fn apply_token(&mut self, token: &str) -> Result<()> {
        match token {
            "Shift" => self.shift = true,
            "Alt" | "Option" => self.alt = true,
            "Ctrl" | "Control" => self.control = true,
            "Cmd" | "Super" => self.super_key = true,
            _ => bail!("unsupported modifier {token:?}"),
        }
        Ok(())
    }

    fn matches(&self, modifiers: ModifiersState) -> bool {
        self.shift == modifiers.shift_key()
            && self.alt == modifiers.alt_key()
            && self.control == modifiers.control_key()
            && self.super_key == modifiers.super_key()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BindingKey {
    Character(char),
    Named(NamedKey),
}

impl BindingKey {
    fn parse(token: &str) -> Result<Self> {
        let mut chars = token.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => Ok(Self::Character(ch)),
            _ => match token {
                "Enter" => Ok(Self::Named(NamedKey::Enter)),
                "Tab" => Ok(Self::Named(NamedKey::Tab)),
                "Space" => Ok(Self::Named(NamedKey::Space)),
                "Escape" => Ok(Self::Named(NamedKey::Escape)),
                "Up" => Ok(Self::Named(NamedKey::ArrowUp)),
                "Down" => Ok(Self::Named(NamedKey::ArrowDown)),
                "Left" => Ok(Self::Named(NamedKey::ArrowLeft)),
                "Right" => Ok(Self::Named(NamedKey::ArrowRight)),
                "Backspace" => Ok(Self::Named(NamedKey::Backspace)),
                "Delete" => Ok(Self::Named(NamedKey::Delete)),
                "Home" => Ok(Self::Named(NamedKey::Home)),
                "End" => Ok(Self::Named(NamedKey::End)),
                "PageUp" => Ok(Self::Named(NamedKey::PageUp)),
                "PageDown" => Ok(Self::Named(NamedKey::PageDown)),
                _ => bail!("unsupported key {token:?}"),
            },
        }
    }

    fn matches(&self, key: &Key) -> bool {
        match (self, key) {
            (Self::Character(expected), Key::Character(text)) => {
                let mut chars = text.chars();
                matches!(
                    (chars.next(), chars.next()),
                    (Some(ch), None) if ch.eq_ignore_ascii_case(expected)
                )
            }
            (Self::Named(expected), Key::Named(actual)) => expected == actual,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_expected_shortcuts() {
        let config = UserKeysConfig::default();
        assert_eq!(config.font_scale_up, "Cmd-=");
        assert_eq!(config.font_scale_down, "Cmd--");
        assert_eq!(config.font_scale_reset, "Cmd-0");
        assert_eq!(config.window_new, "Cmd-N");
        assert_eq!(config.window_close, "Cmd-W");
    }

    #[test]
    fn parses_cmd_minus_binding() {
        assert_eq!(
            Binding::parse("Cmd--").unwrap(),
            Binding {
                modifiers: BindingModifiers {
                    super_key: true,
                    ..BindingModifiers::default()
                },
                key: BindingKey::Character('-'),
            }
        );
    }

    #[test]
    fn rejects_unknown_modifier() {
        assert!(Binding::parse("Hyper-0").is_err());
    }

    #[test]
    fn parses_plain_minus_binding() {
        assert_eq!(split_binding("-").unwrap(), ("", "-"));
    }

    #[test]
    fn matches_default_reset_binding() {
        let binding = Binding::parse("Cmd-0").unwrap();

        assert!(binding.matches(&Key::Character("0".into()), ModifiersState::SUPER));
    }

    #[test]
    fn matches_default_window_shortcuts() {
        let keys = UserKeys::from_config(&UserKeysConfig::default()).unwrap();

        let new_binding = Binding::parse("Cmd-N").unwrap();
        let close_binding = Binding::parse("Cmd-W").unwrap();

        assert!(new_binding.matches(&Key::Character("n".into()), ModifiersState::SUPER));
        assert!(close_binding.matches(&Key::Character("w".into()), ModifiersState::SUPER));
        assert_eq!(
            keys.bindings
                .iter()
                .find(|(_, action)| *action == UserAction::WindowNew)
                .map(|(_, action)| *action),
            Some(UserAction::WindowNew)
        );
    }
}
