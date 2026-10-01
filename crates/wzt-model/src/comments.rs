//! `_` comment keys.
//!
//! JSON has no comments, so any object key starting with `_` is one, at every
//! nesting level. Typed structs collect them in a flattened [`Comments`] map
//! and write them back out, so a TUI edit never erases a hand-written note.
//! Any other unknown key is an error, which is what turns a misspelled field
//! into a failure instead of a silent no-op.
//!
//! Keyed maps (font names in `corrections`, push target names, layer ids in
//! `layer_tweaks`, `params` keys) cannot use `flatten`, because their remaining
//! keys are the real entries. [`Keyed`] splits the two as it deserializes.

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// Whether `key` is a comment key.
pub fn is_comment_key(key: &str) -> bool {
    key.starts_with('_')
}

/// The `_` keys of one object, in file order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Comments(pub Map<String, Value>);

impl Comments {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        let key = key.into();
        debug_assert!(is_comment_key(&key), "comment keys start with `_`");
        self.0.insert(key, value.into());
    }
}

impl Serialize for Comments {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Comments {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CommentsVisitor;

        impl<'de> Visitor<'de> for CommentsVisitor {
            type Value = Comments;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object whose keys all start with `_`")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Comments, A::Error> {
                let mut out = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !is_comment_key(&key) {
                        return Err(de::Error::custom(format!(
                            "unknown field `{key}` (only keys starting with `_` are allowed as comments)"
                        )));
                    }
                    out.insert(key, map.next_value::<Value>()?);
                }
                Ok(Comments(out))
            }
        }

        deserializer.deserialize_map(CommentsVisitor)
    }
}

/// An object whose keys name entries (so none may start with `_`), plus the
/// comment keys that annotate the map itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyed<T> {
    pub entries: BTreeMap<String, T>,
    pub comments: Comments,
}

impl<T> Default for Keyed<T> {
    fn default() -> Self {
        Keyed {
            entries: BTreeMap::new(),
            comments: Comments::default(),
        }
    }
}

impl<T> Keyed<T> {
    pub fn get(&self, key: &str) -> Option<&T> {
        self.entries.get(key)
    }
}

impl<T: Serialize> Serialize for Keyed<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.comments.0.len() + self.entries.len()))?;
        for (k, v) in &self.comments.0 {
            map.serialize_entry(k, v)?;
        }
        for (k, v) in &self.entries {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Keyed<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyedVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for KeyedVisitor<T> {
            type Value = Keyed<T>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map of entries plus optional `_` comments")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Keyed<T>, A::Error> {
                let mut out = Keyed::default();
                while let Some(key) = map.next_key::<String>()? {
                    if is_comment_key(&key) {
                        out.comments.0.insert(key, map.next_value::<Value>()?);
                    } else {
                        out.entries.insert(key, map.next_value::<T>()?);
                    }
                }
                Ok(out)
            }
        }

        deserializer.deserialize_map(KeyedVisitor(PhantomData))
    }
}
