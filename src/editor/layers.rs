use serde::{Deserialize, Serialize};

use crate::model::{LayerId, StyledAtom};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct PersistedLayer {
    pub id: LayerId,
    pub visible: bool,
    pub lines: Vec<Vec<StyledAtom>>,
}
