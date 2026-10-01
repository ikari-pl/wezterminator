//! Enumerate installed font families through fontique.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use fontique::{Blob, Collection, CollectionOptions, SourceKind};

/// A snapshot of family names available for matching.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontCatalog {
    /// Canonical family names as reported by the enumerator.
    pub families: Vec<String>,
}

impl FontCatalog {
    /// Build from an explicit list (tests inject this).
    pub fn from_families(families: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut set = BTreeSet::new();
        for name in families {
            set.insert(name.into());
        }
        Self {
            families: set.into_iter().collect(),
        }
    }

    /// Whether `name` is present, compared case-insensitively.
    pub fn contains(&self, name: &str) -> bool {
        family_present(name, &self.families)
    }
}

/// Case-insensitive membership against a family list.
pub fn family_present(name: &str, installed: &[String]) -> bool {
    installed.iter().any(|f| f.eq_ignore_ascii_case(name))
}

/// Enumerate system fonts via fontique.
pub fn system_catalog() -> FontCatalog {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    collection.load_system_fonts();
    let mut set = BTreeSet::new();
    for name in collection.family_names() {
        set.insert(name.to_string());
    }
    FontCatalog {
        families: set.into_iter().collect(),
    }
}

/// Load one font file into a private collection and return its family names.
///
/// Useful for coverage tests that register a bundled fixture without touching
/// the system catalog.
pub fn catalog_from_font_file(path: &Path) -> Result<FontCatalog, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    catalog_from_font_bytes(&bytes)
}

/// Register in-memory font data and return the family names it declares.
pub fn catalog_from_font_bytes(bytes: &[u8]) -> Result<FontCatalog, String> {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    let blob = Blob::new(Arc::new(bytes.to_vec()));
    let registered = collection.register_fonts(blob, None);
    let mut set = BTreeSet::new();
    for (family_id, _fonts) in registered {
        if let Some(name) = collection.family_name(family_id) {
            set.insert(name.to_string());
        }
    }
    Ok(FontCatalog {
        families: set.into_iter().collect(),
    })
}

/// Read the first available source path for `family`, if the catalog knows it.
///
/// Returns `None` when the family is missing or only available as memory.
pub fn family_source_path(family: &str) -> Option<std::path::PathBuf> {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    collection.load_system_fonts();
    let info = collection.family_by_name(family)?;
    let font = info.default_font()?;
    match font.source().kind() {
        SourceKind::Path(path) => Some(path.as_ref().to_path_buf()),
        SourceKind::Memory(_) => None,
    }
}

/// Load font bytes for a family from the system catalog.
pub fn load_family_bytes(family: &str) -> Option<Vec<u8>> {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    collection.load_system_fonts();
    let info = collection.family_by_name(family)?;
    let font = info.default_font()?;
    let blob = font.load(None)?;
    Some(blob.as_ref().to_vec())
}
