use serde::{Deserialize, Serialize};

fn default_color() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Face {
    pub fg: String,
    pub bg: String,
    pub underline: String,
    pub attributes: Vec<String>,
}

impl Default for Face {
    fn default() -> Self {
        Self {
            fg: default_color(),
            bg: default_color(),
            underline: default_color(),
            attributes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Atom {
    pub face: Face,
    pub contents: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Coord {
    pub line: usize,
    pub column: usize,
}

pub const MAX_LAYERS: usize = 6;
pub const LAYER_SYMBOLS: [&str; 10] = ["⍺", "β", "γ", "δ", "ε", "ζ", "η", "θ", "ι", "κ"];

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct LayerId(pub u8);

impl LayerId {
    pub fn symbol(self) -> &'static str {
        LAYER_SYMBOLS[usize::from(self.0)]
    }

    pub fn is_valid(self) -> bool {
        usize::from(self.0) < LAYER_SYMBOLS.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerSummary {
    pub id: LayerId,
    pub visible: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Right,
    Down,
    Left,
}

impl Direction {
    pub fn opposite(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Right => Self::Left,
            Self::Down => Self::Up,
            Self::Left => Self::Right,
        }
    }
}
