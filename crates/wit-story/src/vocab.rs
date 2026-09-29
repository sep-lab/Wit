//! The banned-vocabulary lint (wit-planning/PLAN.md "Vocabulary": banned
//! words are product decisions). Checks Wit's own words only — a musician
//! may name a track "Stage Piano" or a region "Push it", and that is theirs
//! to name.

use crate::types::{Library, Sentence, SpanKind, Story};

/// Words that never appear in Wit's own user-facing text. Matched as whole
/// words, case-insensitively. "diff" is banned as a noun; Wit has no reason
/// to use it as a verb either.
pub const BANNED: &[&str] = &[
    "commit",
    "commits",
    "branch",
    "branches",
    "merge",
    "merged",
    "repo",
    "repository",
    "push",
    "pull",
    "clone",
    "cloned",
    "checkout",
    "stage",
    "staged",
    "diff",
    "diffs",
    "head",
    "hash",
    "hashes",
    "delta",
    "deltas",
    "sync",
    "synced",
    "conflict",
    "conflicts",
    "snapshot",
    "snapshots",
    "upload",
    "uploaded",
    "cloud",
];

/// Multi-word bans.
pub const BANNED_PHRASES: &[&str] = &["version control"];

/// Banned words found in `text`, lowercased, in order of appearance.
pub fn banned_words(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut found: Vec<String> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| BANNED.contains(w))
        .map(str::to_string)
        .collect();
    for phrase in BANNED_PHRASES {
        if lower.contains(phrase) {
            found.push((*phrase).to_string());
        }
    }
    found
}

fn sentence_violations(s: &Sentence, out: &mut Vec<String>) {
    for span in &s.spans {
        if span.kind == SpanKind::Plain {
            for w in banned_words(&span.text) {
                out.push(format!("'{w}' in \"{}\"", s.text));
            }
        }
    }
}

/// Every banned word in Wit's own wording anywhere in a Story. Names the
/// musician chose are skipped: only [`SpanKind::Plain`] spans and the
/// labels Wit renders are checked. (Titles like the song name are the
/// musician's and are not checked.)
pub fn story_violations(story: &Story) -> Vec<String> {
    let mut texts: Vec<&str> = vec![
        &story.header.daw_label,
        &story.header.kept.label,
        &story.header.subtitle,
    ];
    if let Some(l) = &story.header.lineage {
        texts.push(l);
    }
    if let Some(l) = &story.header.family_label {
        texts.push(l);
    }
    for note in &story.capability {
        texts.push(&note.text);
    }
    for session in &story.sessions {
        texts.push(&session.label);
        for m in &session.moments {
            texts.extend([
                m.label.as_str(),
                m.heading.as_str(),
                m.subheading.as_str(),
                m.source_label.as_str(),
            ]);
            if let Some(n) = &m.note {
                texts.push(n);
            }
            if let Some(n) = &m.summary {
                texts.push(n);
            }
        }
    }
    if let Some(o) = &story.overview {
        texts.extend([o.heading.as_str(), o.subheading.as_str()]);
        if let Some(n) = &o.note {
            texts.push(n);
        }
        if let Some(n) = &o.summary {
            texts.push(n);
        }
    }
    if let Some(sr) = &story.send_ready {
        texts.push(&sr.headline);
        for f in &sr.outside_files {
            texts.push(&f.reason);
        }
    }
    let mut out = Vec::new();
    for text in texts {
        for w in banned_words(text) {
            out.push(format!("'{w}' in \"{text}\""));
        }
    }
    for session in &story.sessions {
        for m in &session.moments {
            for s in &m.sentences {
                sentence_violations(s, &mut out);
            }
        }
    }
    if let Some(o) = &story.overview {
        for s in &o.sentences {
            sentence_violations(s, &mut out);
        }
    }
    out
}

/// [`story_violations`] across a whole library, plus the shelf and trust
/// panel wording.
pub fn library_violations(library: &Library) -> Vec<String> {
    let mut out: Vec<String> = library.stories.iter().flat_map(story_violations).collect();
    for card in &library.shelf {
        let mut texts = vec![card.digest.as_str()];
        if let Some(l) = &card.last_worked_label {
            texts.push(l);
        }
        for text in texts {
            for w in banned_words(text) {
                out.push(format!("'{w}' in shelf text \"{text}\""));
            }
        }
    }
    for s in &library.trust.statements {
        for w in banned_words(s) {
            out.push(format!("'{w}' in trust statement \"{s}\""));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_words_only() {
        assert_eq!(banned_words("Pushed the tempo"), Vec::<String>::new());
        assert_eq!(banned_words("push it"), vec!["push"]);
        assert_eq!(banned_words("Headphone mix"), Vec::<String>::new());
        assert_eq!(
            banned_words("Wit is version control"),
            vec!["version control"]
        );
        assert_eq!(banned_words("A Snapshot of HEAD"), vec!["snapshot", "head"]);
    }
}
