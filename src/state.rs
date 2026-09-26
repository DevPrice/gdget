use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::manifest::AddonName;

const STATE_FILE: &str = "state.toml";

/// What gdget remembers about a project outside the addon folders: the override links it
/// created. Links can't carry a marker without writing into the linked dev folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    /// Addon name to the absolute directory its link points at.
    pub links: BTreeMap<AddonName, PathBuf>,
}

impl State {
    pub fn load(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join(STATE_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        Self::parse(&text).with_context(|| format!("invalid {}", path.display()))
    }

    fn parse(text: &str) -> Result<Self> {
        let doc: DocumentMut = text.parse()?;
        let mut state = Self::default();
        if let Some(links) = doc.get("links") {
            let links = links
                .as_table_like()
                .ok_or_else(|| anyhow!("`links` must be a table"))?;
            for (name, target) in links.iter() {
                let target = target
                    .as_str()
                    .ok_or_else(|| anyhow!("`links.{name}` must be a string"))?;
                state.links.insert(name.parse()?, PathBuf::from(target));
            }
        }
        Ok(state)
    }

    pub fn save(&self, state_dir: &Path) -> Result<()> {
        let mut doc = DocumentMut::new();
        doc.decor_mut()
            .set_prefix("# Written by gdget. Do not edit.\n");
        let mut links = Table::new();
        for (name, target) in &self.links {
            links.insert(name.as_str(), value(target.to_string_lossy().as_ref()));
        }
        doc["links"] = Item::Table(links);
        let path = state_dir.join(STATE_FILE);
        std::fs::create_dir_all(state_dir)
            .with_context(|| format!("cannot create {}", state_dir.display()))?;
        crate::fsutil::write_atomic(&path, doc.to_string().as_bytes())
            .with_context(|| format!("cannot write {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_defaults_to_empty() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".gdget");
        assert_eq!(State::load(&dir).unwrap(), State::default());

        let mut state = State::default();
        state.links.insert(
            "godot-slang".parse().unwrap(),
            PathBuf::from("C:\\dev\\godot-slang\\addons\\godot-slang"),
        );
        state.save(&dir).unwrap();
        assert_eq!(State::load(&dir).unwrap(), state);
    }
}
