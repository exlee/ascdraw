use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

fn default_color() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
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
pub struct StyledAtom {
    pub face: Face,
    pub contents: String,
}

impl StyledAtom {
    pub fn validate_cell(&self) -> anyhow::Result<()> {
        let mut graphemes = UnicodeSegmentation::graphemes(self.contents.as_str(), true);
        let Some(grapheme) = graphemes.next() else {
            anyhow::bail!("cell atom cannot be empty");
        };
        if graphemes.next().is_some() {
            anyhow::bail!("cell atom must contain exactly one grapheme");
        }
        if UnicodeWidthStr::width(grapheme) != 1 {
            anyhow::bail!("cell atom must have display width 1");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atom {
    contents: String,
}

impl Atom {
    pub fn new(contents: impl Into<String>) -> anyhow::Result<Self> {
        let atom = Self {
            contents: contents.into(),
        };
        atom.validate()?;
        Ok(atom)
    }

    pub fn contents(&self) -> &str {
        &self.contents
    }

    fn validate(&self) -> anyhow::Result<()> {
        let mut graphemes = UnicodeSegmentation::graphemes(self.contents.as_str(), true);
        let Some(grapheme) = graphemes.next() else {
            anyhow::bail!("cell atom cannot be empty");
        };
        if graphemes.next().is_some() {
            anyhow::bail!("cell atom must contain exactly one grapheme");
        }
        if UnicodeWidthStr::width(grapheme) != 1 {
            anyhow::bail!("cell atom must have display width 1");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Coord {
    pub line: i16,
    pub column: i16,
}

pub const MAX_CANVAS_WIDTH: usize = 20_000;
pub const MAX_CANVAS_HEIGHT: usize = 20_000;
pub const MAX_LAYERS: usize = 6;
pub const LAYER_SYMBOLS: [&str; 10] = ["α", "β", "γ", "δ", "ε", "ζ", "η", "θ", "ι", "κ"];
pub const BASE_COLORS: [&str; 8] = [
    "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5",
];
pub const BRIGHT_COLORS: [&str; 8] = [
    "#7f7f7f", "#ff0000", "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
];

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ColorId(pub u8);

impl ColorId {
    pub const COUNT: usize = BASE_COLORS.len() + BRIGHT_COLORS.len();

    pub fn hex(self) -> Option<&'static str> {
        let index = usize::from(self.0);
        if index < BASE_COLORS.len() {
            BASE_COLORS.get(index).copied()
        } else {
            BRIGHT_COLORS.get(index - BASE_COLORS.len()).copied()
        }
    }

    pub fn is_valid(self) -> bool {
        usize::from(self.0) < Self::COUNT
    }
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
