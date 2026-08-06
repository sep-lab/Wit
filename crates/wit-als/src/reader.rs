//! Hardened reading: bounded gunzip, DOCTYPE rejection, depth-capped XML
//! parsing into an owned [`crate::dom::Element`] tree.
//!
//! PLAN.md M1: "Security: gzip cap, DOCTYPE reject, depth cap." A `.als` is
//! an untrusted file by construction — Wit reads project files a musician
//! downloaded, was AirDropped, or found in someone else's shared folder.

use crate::dom::Element;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;

/// Ceiling on decompressed XML size — a gzip-bomb defense. `.als` files in
/// the wild run from a few hundred KB to low tens of MB; 512 MiB is
/// generous headroom with no realistic legitimate file anywhere near it.
pub const MAX_DECOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

/// Ceiling on XML element nesting depth. Real Live sets nest a few dozen
/// levels deep at most; this guards against a crafted file using recursive
/// nesting to exhaust the stack or force quadratic tree-walk costs.
pub const MAX_DEPTH: usize = 256;

#[derive(Debug)]
pub enum AlsError {
    Io(std::io::Error),
    DecompressedTooLarge { limit: u64 },
    DoctypeRejected,
    TooDeep { limit: usize },
    Xml(String),
    MissingRoot,
}

impl fmt::Display for AlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AlsError::Io(e) => write!(f, "I/O error reading .als: {e}"),
            AlsError::DecompressedTooLarge { limit } => write!(
                f,
                "decompressed .als exceeds the {limit}-byte limit — refusing \
                 to read further (possible gzip bomb)"
            ),
            AlsError::DoctypeRejected => {
                write!(f, "the .als XML declares a DOCTYPE — refusing to parse it")
            }
            AlsError::TooDeep { limit } => write!(
                f,
                "the .als XML nests more than {limit} elements deep — refusing \
                 to parse further"
            ),
            AlsError::Xml(msg) => write!(f, "malformed .als XML: {msg}"),
            AlsError::MissingRoot => write!(f, "the .als XML has no root element"),
        }
    }
}

impl std::error::Error for AlsError {}

impl From<std::io::Error> for AlsError {
    fn from(e: std::io::Error) -> Self {
        AlsError::Io(e)
    }
}

/// Decompress a gzip stream, refusing to produce more than `limit` bytes.
/// Reads in bounded chunks rather than trusting any size hint in the gzip
/// header, which an attacker fully controls.
pub fn bounded_gunzip(bytes: &[u8], limit: u64) -> Result<Vec<u8>, AlsError> {
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = decoder.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > limit {
            return Err(AlsError::DecompressedTooLarge { limit });
        }
        out.extend_from_slice(&chunk[..n]);
    }
    Ok(out)
}

/// Parse decompressed XML bytes into an owned [`Element`] tree.
pub fn parse_xml(xml: &[u8], max_depth: usize) -> Result<Element, AlsError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| AlsError::Xml(e.to_string()))?;
        match event {
            // Rejected outright, never resolved — the DOCTYPE/XXE defense.
            // quick-xml does not expand external entities regardless, but
            // refusing the declaration itself is the guardrail PLAN.md asks
            // for, and it is cheap insurance against a parser change later.
            Event::DocType(_) => return Err(AlsError::DoctypeRejected),
            Event::Start(e) => {
                if stack.len() >= max_depth {
                    return Err(AlsError::TooDeep { limit: max_depth });
                }
                stack.push(element_from_start(&e)?);
            }
            Event::Empty(e) => {
                if stack.len() >= max_depth {
                    return Err(AlsError::TooDeep { limit: max_depth });
                }
                let el = element_from_start(&e)?;
                attach(&mut stack, &mut root, el);
            }
            Event::End(_) => {
                let el = stack
                    .pop()
                    .ok_or_else(|| AlsError::Xml("unbalanced closing tag".to_string()))?;
                attach(&mut stack, &mut root, el);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if !stack.is_empty() {
        // quick-xml does not itself error on a document that ends with
        // elements still open — it just stops. A truncated .als (a
        // half-written file, a crafted attack input) must not silently
        // parse into a partial tree.
        return Err(AlsError::Xml(
            "unexpected end of document (unclosed element)".to_string(),
        ));
    }
    root.ok_or(AlsError::MissingRoot)
}

fn attach(stack: &mut [Element], root: &mut Option<Element>, el: Element) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(el);
    } else {
        *root = Some(el);
    }
}

fn element_from_start(e: &BytesStart) -> Result<Element, AlsError> {
    let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut attrs = BTreeMap::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|err| AlsError::Xml(err.to_string()))?;
        let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Explicit1_0)
            .map_err(|err| AlsError::Xml(err.to_string()))?
            .into_owned();
        attrs.insert(key, value);
    }
    Ok(Element {
        tag,
        attrs,
        children: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn bounded_gunzip_refuses_past_the_limit() {
        let big = vec![b'a'; 10_000];
        let gz = gzip(&big);
        let err = bounded_gunzip(&gz, 100).unwrap_err();
        assert!(matches!(err, AlsError::DecompressedTooLarge { limit: 100 }));
    }

    #[test]
    fn bounded_gunzip_accepts_within_the_limit() {
        let small = b"<a/>".to_vec();
        let gz = gzip(&small);
        let out = bounded_gunzip(&gz, 1000).unwrap();
        assert_eq!(out, small);
    }

    #[test]
    fn parse_xml_rejects_a_doctype() {
        let xml = b"<!DOCTYPE foo><a/>";
        let err = parse_xml(xml, 256).unwrap_err();
        assert!(matches!(err, AlsError::DoctypeRejected));
    }

    #[test]
    fn parse_xml_refuses_past_the_depth_limit() {
        let mut xml = String::new();
        for _ in 0..10 {
            xml.push_str("<a>");
        }
        xml.push_str("</a>".repeat(10).as_str());
        let err = parse_xml(xml.as_bytes(), 5).unwrap_err();
        assert!(matches!(err, AlsError::TooDeep { limit: 5 }));
    }

    #[test]
    fn parse_xml_builds_a_tree_with_attributes_and_children() {
        let xml = br#"<Root A="1"><Child B="2"/></Root>"#;
        let root = parse_xml(xml, 256).unwrap();
        assert_eq!(root.tag, "Root");
        assert_eq!(root.attr("A"), Some("1"));
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].tag, "Child");
        assert_eq!(root.children[0].attr("B"), Some("2"));
    }

    #[test]
    fn parse_xml_rejects_a_truncated_document() {
        let xml = b"<Root><Child>";
        let err = parse_xml(xml, 256).unwrap_err();
        assert!(matches!(err, AlsError::Xml(_)));
    }
}
