//! Hierarchical names for heads (branches) and name-keyed metadata.
//!
//! A [`Name`] is a non-empty sequence of segments, displayed joined by
//! [`SEP`], which is `:`. Nesting is structural rather than a substring
//! convention. `synth:filter` is the child segment `filter` under the root
//! `synth`.

use crate::{CaHash, Hasher};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str};

/// A hierarchical name. One or more segments, displayed joined by [`SEP`].
///
/// Ordering is segment-wise, so all of a name's descendants sort directly
/// after it. For example, `["a"] < ["a", "b"] < ["ab"]`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Name(Vec<String>);

/// The separator between segments in a [`Name`]'s display form.
pub const SEP: char = ':';

impl Name {
    /// A root name with the given single segment.
    pub fn root(segment: impl Into<String>) -> Self {
        Self(vec![segment.into()])
    }

    /// The name's segments. There is always at least one.
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    /// This name with `segment` appended as a child.
    pub fn child(&self, segment: impl Into<String>) -> Self {
        let mut segments = self.0.clone();
        segments.push(segment.into());
        Self(segments)
    }

    /// The parent name, or `None` for a root name.
    pub fn parent(&self) -> Option<Name> {
        (self.0.len() > 1).then(|| Self(self.0[..self.0.len() - 1].to_vec()))
    }

    /// Nesting depth. `0` for a root name.
    pub fn depth(&self) -> usize {
        self.0.len() - 1
    }

    /// Whether the name has a parent.
    pub fn is_nested(&self) -> bool {
        self.0.len() > 1
    }

    /// Whether `prefix` is this name or an ancestor of it.
    pub fn starts_with(&self, prefix: &Name) -> bool {
        self.0.len() >= prefix.0.len() && self.0[..prefix.0.len()] == prefix.0[..]
    }
}

impl CaHash for Name {
    fn hash(&self, hasher: &mut Hasher) {
        self.0.hash(hasher);
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut segments = self.0.iter();
        if let Some(first) = segments.next() {
            write!(f, "{first}")?;
        }
        for segment in segments {
            write!(f, "{SEP}{segment}")?;
        }
        Ok(())
    }
}

impl From<Vec<String>> for Name {
    /// Constructs the name from raw segments. An empty vec becomes the name
    /// with a single empty segment. This keeps the at-least-one-segment
    /// invariant without a failure case.
    fn from(segments: Vec<String>) -> Self {
        if segments.is_empty() {
            Self(vec![String::new()])
        } else {
            Self(segments)
        }
    }
}

impl str::FromStr for Name {
    type Err = std::convert::Infallible;
    /// Splits on [`SEP`]. Every string is a valid name. An empty string is
    /// the root name with one empty segment.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.split(SEP).map(str::to_string).collect()))
    }
}

impl Serialize for Name {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(s.parse().expect("infallible"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_display_round_trip() {
        for s in ["synth", "synth:filter", "a:b:c", "", "a::b"] {
            let name: Name = s.parse().unwrap();
            assert_eq!(name.to_string(), s);
        }
    }

    #[test]
    fn hierarchy() {
        let root = Name::root("synth");
        let child = root.child("filter");
        assert_eq!(child.to_string(), "synth:filter");
        assert_eq!(child.parent(), Some(root.clone()));
        assert_eq!(root.parent(), None);
        assert_eq!(root.depth(), 0);
        assert_eq!(child.depth(), 1);
        assert!(!root.is_nested());
        assert!(child.is_nested());
        assert!(child.starts_with(&root));
        assert!(child.starts_with(&child));
        assert!(!root.starts_with(&child));
    }

    #[test]
    fn descendants_sort_adjacent() {
        let mut names: Vec<Name> = ["ab", "a", "a:b", "b"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        names.sort();
        let displayed: Vec<String> = names.iter().map(Name::to_string).collect();
        assert_eq!(displayed, ["a", "a:b", "ab", "b"]);
    }

    #[test]
    fn prefix_is_segment_wise_not_substring() {
        let ab: Name = "ab".parse().unwrap();
        let a = Name::root("a");
        assert!(!ab.starts_with(&a));
    }

    #[test]
    fn serde_as_string() {
        let name: Name = "synth:filter".parse().unwrap();
        let ron = ron::to_string(&name).unwrap();
        assert_eq!(ron, "\"synth:filter\"");
        let back: Name = ron::de::from_str(&ron).unwrap();
        assert_eq!(back, name);
    }
}
