//! The client-scope policy: the routing file's path markers, checked against
//! their floor record, as one generation (openspec `hub-client-read-scope`,
//! D2, D3, R2-C, R2-D).
//!
//! The rules mirror the distiller's `load_routing` and `_floor_problem`: a
//! missing or invalid file, counts below the floor, or a sha256 that differs
//! from the floor record make the policy `Unknown`, and unknown means every
//! session is client for a lab reader.

use std::collections::BTreeSet;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::client_scope::{normalize, Markers};
use crate::config::ClientScopeConfig;

/// A usable policy generation.
pub struct Policy {
    /// sha256 of (routing bytes ‖ floor bytes): any change to either file is a
    /// new generation.
    pub generation: String,
    pub markers: Markers,
    /// sha256 of each normalized path marker: what the hub keeps of the set.
    pub marker_hashes: BTreeSet<String>,
    /// The distiller's content terms, normalized. Never used for the flag (Q1:
    /// not at launch); the sweep only COUNTS sessions they would protect.
    pub terms: Vec<String>,
}

/// The outcome of reading the two files. `Unknown` carries a reason that names
/// the problem, never a marker.
pub enum Load {
    Ready(Policy),
    Unknown(String),
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Read and check the routing file and its floor record.
pub fn load(cfg: &ClientScopeConfig) -> Load {
    load_paths(&cfg.routing_file, &cfg.floor_file)
}

pub fn load_paths(routing_file: &Path, floor_file: &Path) -> Load {
    let unknown = |why: String| Load::Unknown(why);
    let routing = match std::fs::read(routing_file) {
        Ok(b) => b,
        Err(e) => return unknown(format!("routing file unreadable: {}", e.kind())),
    };
    let floor = match std::fs::read(floor_file) {
        Ok(b) => b,
        Err(e) => return unknown(format!("floor file unreadable: {}", e.kind())),
    };
    let Ok(text) = std::str::from_utf8(&routing) else {
        return unknown("routing file is not UTF-8".into());
    };
    let Ok(cfg) = text.parse::<toml::Table>() else {
        return unknown("routing file is not valid TOML".into());
    };
    let Some(sect) = cfg.get("protected").and_then(toml::Value::as_table) else {
        return unknown("routing file has no [protected] table".into());
    };
    let strings = |key: &str| -> Option<Vec<String>> {
        match sect.get(key) {
            None => Some(Vec::new()),
            Some(toml::Value::Array(items)) => items
                .iter()
                .map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string))
                .collect(),
            Some(_) => None,
        }
    };
    let (Some(markers), Some(terms)) = (strings("path_markers"), strings("content_terms")) else {
        return unknown("path_markers/content_terms must be lists of strings".into());
    };
    if sect
        .get("model")
        .and_then(toml::Value::as_str)
        .map_or(true, |m| m.trim().is_empty())
    {
        return unknown("[protected].model missing".into());
    }

    let Ok(rec) = serde_json::from_slice::<serde_json::Value>(&floor) else {
        return unknown("floor record is not JSON".into());
    };
    let int = |k: &str| rec.get(k).and_then(serde_json::Value::as_u64);
    let (Some(sha), Some(plain_min), Some(encoded_min), Some(terms_min)) = (
        rec.get("sha256").and_then(serde_json::Value::as_str),
        int("plain_path_markers"),
        int("encoded_path_markers"),
        int("content_terms"),
    ) else {
        return unknown("floor record is incomplete".into());
    };
    let encoded = markers
        .iter()
        .filter(|m| m.starts_with('-') && !m.contains('/'))
        .count() as u64;
    let plain = markers.len() as u64 - encoded;
    if plain < plain_min || encoded < encoded_min || (terms.len() as u64) < terms_min {
        return unknown("routing file below its floor".into());
    }
    if sha256_hex(&routing) != sha {
        return unknown("routing file differs from its floor record".into());
    }

    let mut both = routing.clone();
    both.extend_from_slice(&floor);
    Load::Ready(Policy {
        generation: sha256_hex(&both),
        marker_hashes: markers
            .iter()
            .map(|m| sha256_hex(normalize(m).as_bytes()))
            .collect(),
        markers: Markers::new(&markers),
        terms: terms.iter().map(|t| normalize(t)).collect(),
    })
}

/// How a new marker set relates to the accepted one (R2-D).
#[derive(Debug, PartialEq, Eq)]
pub enum Acceptance {
    /// Nothing accepted yet: the first set is accepted and audited.
    Bootstrap,
    /// The same set, or only additions: access can only narrow, accept.
    Widened { added: usize },
    /// A marker was removed or replaced: unknown until `accept-routing`.
    Narrowed { removed: usize, added: usize },
}

pub fn compare(accepted: &BTreeSet<String>, new: &BTreeSet<String>) -> Acceptance {
    if accepted.is_empty() {
        return Acceptance::Bootstrap;
    }
    let removed = accepted.difference(new).count();
    let added = new.difference(accepted).count();
    if removed == 0 {
        Acceptance::Widened { added }
    } else {
        Acceptance::Narrowed { removed, added }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Files {
        dir: std::path::PathBuf,
    }

    impl Files {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "cchv-client-policy-{tag}-{}",
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        /// Write a routing file and a floor record that matches it (unless
        /// `floor` overrides the record).
        fn write(
            &self,
            routing: &str,
            floor: Option<serde_json::Value>,
        ) -> (std::path::PathBuf, std::path::PathBuf) {
            let r = self.dir.join("routing.toml");
            let f = self.dir.join("floor.json");
            std::fs::write(&r, routing).unwrap();
            let rec = floor.unwrap_or_else(|| {
                serde_json::json!({
                    "sha256": sha256_hex(routing.as_bytes()),
                    "plain_path_markers": 2, "encoded_path_markers": 1, "content_terms": 1,
                })
            });
            std::fs::write(&f, rec.to_string()).unwrap();
            (r, f)
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const GOOD: &str = r#"
[protected]
model = "claude-x"
path_markers = ["/clientco", "/work/acme_client", "-users-ac--sync-dev--clientco"]
content_terms = ["clientword"]
"#;

    fn reason(l: Load) -> String {
        match l {
            Load::Ready(_) => "ready".into(),
            Load::Unknown(r) => r,
        }
    }

    #[test]
    fn a_file_that_passes_its_floor_is_ready() {
        let fs = Files::new("good");
        let (r, f) = fs.write(GOOD, None);
        let Load::Ready(p) = load_paths(&r, &f) else {
            panic!("expected ready");
        };
        assert_eq!(p.marker_hashes.len(), 3);
        assert!(p.markers.matches(&normalize("cd /x/ClientCo/y")));
        assert_eq!(p.generation.len(), 64);
    }

    #[test]
    fn every_way_of_not_knowing_is_unknown() {
        let fs = Files::new("bad");
        let (r, f) = fs.write(GOOD, None);
        assert_eq!(
            reason(load_paths(&fs.dir.join("missing.toml"), &f)),
            "routing file unreadable: entity not found"
        );
        assert_eq!(
            reason(load_paths(&r, &fs.dir.join("missing.json"))),
            "floor file unreadable: entity not found"
        );

        let (r, f) = fs.write("not = [toml", None);
        assert_eq!(reason(load_paths(&r, &f)), "routing file is not valid TOML");

        let (r, f) = fs.write("[other]\nx = 1\n", None);
        assert_eq!(
            reason(load_paths(&r, &f)),
            "routing file has no [protected] table"
        );

        let (r, f) = fs.write("[protected]\npath_markers = [\"/a\"]\n", None);
        assert_eq!(reason(load_paths(&r, &f)), "[protected].model missing");

        let (r, f) = fs.write("[protected]\nmodel = \"m\"\npath_markers = [1]\n", None);
        assert_eq!(
            reason(load_paths(&r, &f)),
            "path_markers/content_terms must be lists of strings"
        );

        // One plain marker fewer than the floor.
        let thin = GOOD.replace("\"/work/acme_client\", ", "");
        let (r, f) = fs.write(
            &thin,
            Some(serde_json::json!({
                "sha256": sha256_hex(thin.as_bytes()),
                "plain_path_markers": 2, "encoded_path_markers": 1, "content_terms": 1,
            })),
        );
        assert_eq!(reason(load_paths(&r, &f)), "routing file below its floor");

        // Same counts, different bytes than recorded.
        let (r, f) = fs.write(
            GOOD,
            Some(serde_json::json!({
                "sha256": sha256_hex(b"something else"),
                "plain_path_markers": 2, "encoded_path_markers": 1, "content_terms": 1,
            })),
        );
        assert_eq!(
            reason(load_paths(&r, &f)),
            "routing file differs from its floor record"
        );

        let (r, f) = fs.write(GOOD, Some(serde_json::json!({"sha256": "x"})));
        assert_eq!(reason(load_paths(&r, &f)), "floor record is incomplete");
    }

    #[test]
    fn any_change_to_either_file_is_a_new_generation() {
        let fs = Files::new("gen");
        let (r, f) = fs.write(GOOD, None);
        let Load::Ready(a) = load_paths(&r, &f) else {
            panic!()
        };
        let more = GOOD.replace(
            "content_terms = [\"clientword\"]",
            "content_terms = [\"clientword\", \"other\"]",
        );
        let (r, f) = fs.write(&more, None);
        let Load::Ready(b) = load_paths(&r, &f) else {
            panic!()
        };
        assert_ne!(a.generation, b.generation, "a changed routing file");
        assert_eq!(
            a.marker_hashes, b.marker_hashes,
            "the path markers did not change"
        );
    }

    #[test]
    fn removing_or_swapping_a_marker_is_a_narrowing() {
        let set = |xs: &[&str]| xs.iter().map(|s| (*s).to_string()).collect::<BTreeSet<_>>();
        let accepted = set(&["a", "b", "c"]);
        assert_eq!(compare(&BTreeSet::new(), &accepted), Acceptance::Bootstrap);
        assert_eq!(
            compare(&accepted, &accepted),
            Acceptance::Widened { added: 0 }
        );
        assert_eq!(
            compare(&accepted, &set(&["a", "b", "c", "d"])),
            Acceptance::Widened { added: 1 }
        );
        assert_eq!(
            compare(&accepted, &set(&["a", "b"])),
            Acceptance::Narrowed {
                removed: 1,
                added: 0
            }
        );
        // The same count with one marker replaced is still a narrowing.
        assert_eq!(
            compare(&accepted, &set(&["a", "b", "x"])),
            Acceptance::Narrowed {
                removed: 1,
                added: 1
            }
        );
    }
}
