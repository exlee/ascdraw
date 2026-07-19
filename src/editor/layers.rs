use serde::{Deserialize, Serialize};

use crate::model::{Atom, LayerId};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct PersistedLayer {
    pub id: LayerId,
    pub visible: bool,
    pub lines: Vec<Vec<Atom>>,
}

#[derive(Debug, Clone)]
pub struct LayerView {
    pub id: LayerId,
    pub visible: bool,
    pub lines: Vec<Vec<Atom>>,
}
