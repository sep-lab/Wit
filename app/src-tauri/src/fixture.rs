//! The Story contract's demo library, embedded at compile time so a
//! packaged app never needs to locate the source tree on disk. This lane's
//! brief, item 3: "With no root ... load
//! `crates/wit-story/fixtures/demo-library.json`."

use wit_story::Library;

const DEMO_LIBRARY_JSON: &str =
    include_str!("../../../crates/wit-story/fixtures/demo-library.json");

pub fn demo_library() -> Library {
    serde_json::from_str(DEMO_LIBRARY_JSON)
        .expect("crates/wit-story/fixtures/demo-library.json parses as a Library")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_fixture_parses() {
        let library = demo_library();
        assert_eq!(library.schema_version, wit_story::SCHEMA_VERSION);
        assert!(!library.shelf.is_empty());
    }
}
