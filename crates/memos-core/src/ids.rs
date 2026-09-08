use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A UUIDv7 identifier.
///
/// v7 rather than v4 deliberately: it is time-ordered, so rows insert at the
/// end of the B-tree instead of scattering across it. That keeps index locality
/// good in SQLite and, later, in Postgres — and it means an id sorts
/// chronologically without a separate column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(pub Uuid);

impl Id {
    pub fn new() -> Self {
        Id(Uuid::now_v7())
    }

    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Ok(Id(Uuid::parse_str(s)?))
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for Id {
    fn from(u: Uuid) -> Self {
        Id(u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_time_ordered() {
        let a = Id::new();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = Id::new();
        assert!(a < b, "v7 ids must sort chronologically");
    }
}
