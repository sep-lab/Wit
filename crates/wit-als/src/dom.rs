//! A minimal, owned XML element tree — just enough of `ElementTree`'s query
//! surface (`find`, `findall`, direct-child and `.//`-descendant paths) for
//! `model.rs`'s whitelist extraction to read the same way
//! `als_semantic_diff.py` does. Not a general XPath engine: only the path
//! shapes this model actually uses are supported (see [`find_path`]).

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub tag: String,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(|s| s.as_str())
    }

    /// First direct child with this tag, in document order.
    pub fn find_child(&self, tag: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.tag == tag)
    }

    /// All direct children with this tag, in document order.
    pub fn find_all_children<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.tag == tag)
    }

    /// First descendant (any depth, not including `self`) with this tag,
    /// pre-order — matches `ElementTree.find(".//tag")`.
    pub fn find_descendant(&self, tag: &str) -> Option<&Element> {
        for c in &self.children {
            if c.tag == tag {
                return Some(c);
            }
            if let Some(found) = c.find_descendant(tag) {
                return Some(found);
            }
        }
        None
    }

    /// Every descendant (any depth) with this tag, pre-order, continuing to
    /// search inside a match's own subtree too — matches
    /// `ElementTree.findall(".//tag")`.
    pub fn find_all_descendants<'a>(&'a self, tag: &str, out: &mut Vec<&'a Element>) {
        for c in &self.children {
            if c.tag == tag {
                out.push(c);
            }
            c.find_all_descendants(tag, out);
        }
    }
}

/// Resolve an `ElementTree`-style path relative to `root`: an optional
/// leading `.//` makes the *first* segment a descendant search (any depth);
/// every other segment (including the first, when there is no `.//` prefix)
/// is a direct-child lookup. This covers every path shape
/// `als_semantic_diff.py` uses (`"Name/EffectiveName"`,
/// `".//SampleRef/FileRef/RelativePath"`, `"Volume/Manual"`, ...).
///
/// Not a full XPath engine: if the first matching element (found via the
/// descendant search) doesn't have the rest of the path, this returns
/// `None` rather than backtracking to try a later match. Every real and
/// synthetic Ableton Live set this model reads has at most one match for
/// each such path, so backtracking would never change the answer — see
/// `docs/FORMATS.md` / the M1 factory for the schema this assumes.
pub fn find_path<'a>(root: &'a Element, path: &str) -> Option<&'a Element> {
    let (leading_descendant, path) = match path.strip_prefix(".//") {
        Some(rest) => (true, rest),
        None => (false, path),
    };
    let mut segments = path.split('/');
    let first = segments.next()?;
    let mut node = if leading_descendant {
        root.find_descendant(first)?
    } else {
        root.find_child(first)?
    };
    for seg in segments {
        node = node.find_child(seg)?;
    }
    Some(node)
}

/// `_val(node, path, attr="Value")` from `als_semantic_diff.py`: resolve
/// `path` from `node` (if present) and return its `Value` attribute.
pub fn val<'a>(node: Option<&'a Element>, path: &str) -> Option<&'a str> {
    node.and_then(|n| find_path(n, path))
        .and_then(|el| el.attr("Value"))
}
