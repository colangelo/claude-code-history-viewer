//! Client scope: which sessions are client material (openspec change
//! `hub-client-read-scope`, design D1, D2, R2-A, R2-K).
//!
//! The markers are the path markers of the distiller's routing file, kept outside
//! the repo. The match rules are the distiller's (`scripts/cchv-distill.py`
//! `normalize` and `Routing.marker_in`), and the shared fixture
//! `scripts/fixtures/client-scope-cases.json` pins that both sides agree.

use serde_json::Value;

/// What the matcher compares (distiller `normalize`).
pub fn normalize(text: &str) -> String {
    text.to_string()
}

/// A compiled set of path markers.
pub struct Markers {
    _markers: Vec<String>,
}

impl Markers {
    pub fn new(markers: &[String]) -> Self {
        Self {
            _markers: markers.to_vec(),
        }
    }

    /// Whether `normalized` (already passed through [`normalize`]) holds a marker.
    pub fn matches(&self, _normalized: &str) -> bool {
        false
    }
}

/// A session's class: the first match in the order of the variants, else
/// `Unreadable` when any part could not be read, else `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Path,
    Metadata,
    Cwd,
    ToolInput,
    ToolResult,
    TextMarker,
    Raw,
    Unreadable,
    None,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Path => "path",
            Class::Metadata => "metadata",
            Class::Cwd => "cwd",
            Class::ToolInput => "tool_input",
            Class::ToolResult => "tool_result",
            Class::TextMarker => "text_marker",
            Class::Raw => "raw",
            Class::Unreadable => "unreadable",
            Class::None => "none",
        }
    }
}

/// The served fields of a session row.
#[derive(Debug, Default, Clone, Copy)]
pub struct SessionFields<'a> {
    pub project_path: &'a str,
    pub summary: Option<&'a str>,
    pub file_path: Option<&'a str>,
    pub entrypoint: Option<&'a str>,
}

/// One stored message, in full (never the clamped `search_text`).
#[derive(Debug, Clone, Copy)]
pub struct MessageFields<'a> {
    pub content: &'a Value,
    pub raw: &'a Value,
}

/// Classify a session from its row and its messages.
pub fn classify<'a>(
    _markers: &Markers,
    _session: SessionFields<'_>,
    _messages: impl IntoIterator<Item = MessageFields<'a>>,
) -> Class {
    Class::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Fixture {
        markers: Vec<String>,
        match_cases: Vec<MatchCase>,
        extract_cases: Vec<ExtractCase>,
    }

    #[derive(Deserialize)]
    struct MatchCase {
        name: String,
        text: String,
        #[serde(rename = "match")]
        expect: bool,
    }

    #[derive(Deserialize)]
    struct ExtractCase {
        name: String,
        project_path: String,
        session: SessionCase,
        messages: Vec<MessageCase>,
        class: String,
    }

    #[derive(Deserialize, Default)]
    struct SessionCase {
        summary: Option<String>,
        file_path: Option<String>,
        entrypoint: Option<String>,
    }

    #[derive(Deserialize)]
    struct MessageCase {
        content: Value,
        raw: Value,
    }

    fn fixture() -> Fixture {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/fixtures/client-scope-cases.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// Shared with the distiller: the same inputs give the same answer.
    #[test]
    fn match_cases_agree_with_the_distiller() {
        let f = fixture();
        let m = Markers::new(&f.markers);
        let wrong: Vec<&str> = f
            .match_cases
            .iter()
            .filter(|c| m.matches(&normalize(&c.text)) != c.expect)
            .map(|c| c.name.as_str())
            .collect();
        assert!(wrong.is_empty(), "cases that disagree: {wrong:?}");
    }

    #[test]
    fn extract_cases_classify_the_whole_stored_form() {
        let f = fixture();
        let m = Markers::new(&f.markers);
        let wrong: Vec<String> = f
            .extract_cases
            .iter()
            .filter_map(|c| {
                let session = SessionFields {
                    project_path: &c.project_path,
                    summary: c.session.summary.as_deref(),
                    file_path: c.session.file_path.as_deref(),
                    entrypoint: c.session.entrypoint.as_deref(),
                };
                let got = classify(
                    &m,
                    session,
                    c.messages.iter().map(|x| MessageFields {
                        content: &x.content,
                        raw: &x.raw,
                    }),
                );
                (got.as_str() != c.class).then(|| format!("{}: got {}", c.name, got.as_str()))
            })
            .collect();
        assert!(wrong.is_empty(), "cases that disagree: {wrong:?}");
    }

    /// Past the nesting limit the extractor gives up, and giving up is client.
    #[test]
    fn nesting_past_the_limit_is_unreadable() {
        let mut v = Value::String("lab".into());
        for _ in 0..80 {
            v = Value::Array(vec![v]);
        }
        let content = serde_json::json!([{"type": "tool_use", "name": "x", "input": v}]);
        let raw = serde_json::json!({});
        let got = classify(
            &Markers::new(&["/clientco".to_string()]),
            SessionFields {
                project_path: "/w/lab",
                ..Default::default()
            },
            [MessageFields {
                content: &content,
                raw: &raw,
            }],
        );
        assert_eq!(got, Class::Unreadable);
    }
}
