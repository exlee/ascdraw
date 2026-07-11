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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Coord {
    pub line: usize,
    pub column: usize,
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
