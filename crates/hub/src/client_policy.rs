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

use crate::client_scope::{
    normalize, search_form, InstructionBlock, InstructionBlocks, Markers, MIN_BLOCK_BYTES,
};
use crate::config::ClientScopeConfig;

/// The prefix of a registered block's accepted-set row.
const BLOCK_ROW: &str = "instruction:";

/// A usable policy generation.
pub struct Policy {
    /// sha256 of (routing bytes ‖ floor bytes): any change to either file is a
    /// new generation.
    pub generation: String,
    /// The path markers, with the registered instruction blocks they skip.
    pub markers: Markers,
    /// sha256 of each normalized path marker: what the hub keeps of the set.
    pub marker_hashes: BTreeSet<String>,
    /// The registered instruction blocks and where each one's text lives
    /// (openspec `client-scope-instruction-subtraction`).
    pub instruction_blocks: Vec<RegisteredBlock>,
    /// The distiller's content terms, normalized. Never used for the flag (Q1:
    /// not at launch); the sweep only COUNTS sessions they would protect.
    pub terms: Vec<String>,
}

impl Policy {
    /// What the hub records as accepted: the marker hashes, and one row per
    /// registered block that pins its whole descriptor (review U1).
    pub fn accepted_set(&self) -> BTreeSet<String> {
        self.marker_hashes
            .iter()
            .cloned()
            .chain(self.instruction_blocks.iter().map(RegisteredBlock::row))
            .collect()
    }
}

/// Which form of the file a block is: as Claude Code injects it (trailing
/// whitespace removed, measured against the archive 2026-11-02), or as the
/// `search_text` column holds it (whitespace runs collapsed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Injected,
    Search,
}

impl Form {
    pub const ALL: [Form; 2] = [Form::Injected, Form::Search];

    pub fn as_str(self) -> &'static str {
        match self {
            Form::Injected => "injected",
            Form::Search => "search",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == s)
    }

    /// This form of a file's text.
    pub fn of(self, file_text: &str) -> String {
        let injected = file_text.trim_end();
        match self {
            Form::Injected => injected.to_string(),
            Form::Search => search_form(injected),
        }
    }
}

/// Where a block's text lives (review U1): a file at a commit of a git
/// repository, in one form. `accept-routing` re-derives the hash from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockSource {
    pub git_repo: String,
    pub git_commit: String,
    pub git_path: String,
    pub form: Form,
}

/// One `[protected].instruction_blocks` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredBlock {
    pub block: InstructionBlock,
    pub source: BlockSource,
}

impl RegisteredBlock {
    /// The accepted-set row: sha256, length and head hash together, so a
    /// changed descriptor is a new row, hence a narrowing.
    pub fn row(&self) -> String {
        format!(
            "{BLOCK_ROW}{}:{}:{}",
            hex(&self.block.sha256),
            self.block.bytes,
            hex(&self.block.head_sha256)
        )
    }

    /// The routing-file line for this entry.
    pub fn entry_line(&self) -> String {
        format!(
            "{{ sha256 = \"{}\", bytes = {}, head_sha256 = \"{}\", git_repo = {}, git_commit = \"{}\", git_path = {}, form = \"{}\" }}",
            hex(&self.block.sha256),
            self.block.bytes,
            hex(&self.block.head_sha256),
            toml::Value::String(self.source.git_repo.clone()),
            self.source.git_commit,
            toml::Value::String(self.source.git_path.clone()),
            self.source.form.as_str()
        )
    }

    /// Re-derive the block from its source and compare (review U1).
    pub fn verify(&self) -> anyhow::Result<()> {
        let text = git_show(
            &self.source.git_repo,
            &self.source.git_commit,
            &self.source.git_path,
        )?;
        if InstructionBlock::of(&self.source.form.of(&text)) == self.block {
            Ok(())
        } else {
            anyhow::bail!(
                "instruction block {} does not match {}:{} ({})",
                &hex(&self.block.sha256)[..12],
                &self.source.git_commit[..12],
                self.source.git_path,
                self.source.form.as_str()
            )
        }
    }
}

fn git(repo: &str, args: &[&str]) -> anyhow::Result<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("git: {e}"))?;
    if !out.status.success() {
        anyhow::bail!("git {} failed in {repo}", args.first().unwrap_or(&""));
    }
    String::from_utf8(out.stdout).map_err(|_| anyhow::anyhow!("git output is not UTF-8"))
}

fn git_show(repo: &str, commit: &str, path: &str) -> anyhow::Result<String> {
    git(repo, &["show", &format!("{commit}:{path}")])
}

/// The entries for a file at a git revision, both forms, each checked
/// against `markers` for safe edges (review U5) and the minimum size.
pub fn entries_from_git(
    repo: &str,
    rev: &str,
    path: &str,
    markers: &Markers,
) -> anyhow::Result<Vec<RegisteredBlock>> {
    let commit = git(
        repo,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )?
    .trim()
    .to_string();
    let text = git_show(repo, &commit, path)?;
    let mut out: Vec<RegisteredBlock> = Vec::new();
    for form in Form::ALL {
        let t = form.of(&text);
        if t.len() < MIN_BLOCK_BYTES {
            anyhow::bail!("{path} at {rev} is under {MIN_BLOCK_BYTES} bytes");
        }
        if !markers.block_edges_safe(&t) {
            anyhow::bail!(
                "{path} at {rev} ({}): an edge could carry part of a marker or an escape; refused",
                form.as_str()
            );
        }
        let block = InstructionBlock::of(&t);
        if out.iter().all(|r| r.block != block) {
            out.push(RegisteredBlock {
                block,
                source: BlockSource {
                    git_repo: repo.to_string(),
                    git_commit: commit.clone(),
                    git_path: path.to_string(),
                    form,
                },
            });
        }
    }
    Ok(out)
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Exactly `2 * N` ASCII hex digits (review U6: `from_str_radix` alone takes `+1`).
fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != 2 * N || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0_u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// `[protected].instruction_blocks`: absent is none; anything but a list of
/// well-formed entries of at least [`MIN_BLOCK_BYTES`], with no two entries
/// giving one sha256 different descriptors, is `None`.
fn instruction_blocks(sect: &toml::Table) -> Option<Vec<RegisteredBlock>> {
    let Some(v) = sect.get("instruction_blocks") else {
        return Some(Vec::new());
    };
    let blocks: Vec<RegisteredBlock> = v
        .as_array()?
        .iter()
        .map(|e| {
            let t = e.as_table()?;
            let text = |k: &str| t.get(k)?.as_str().filter(|s| !s.is_empty());
            let bytes = usize::try_from(t.get("bytes")?.as_integer()?).ok()?;
            (bytes >= MIN_BLOCK_BYTES).then_some(())?;
            let commit = text("git_commit")?;
            unhex::<20>(commit)?;
            Some(RegisteredBlock {
                block: InstructionBlock {
                    sha256: unhex(text("sha256")?)?,
                    bytes,
                    head_sha256: unhex(text("head_sha256")?)?,
                },
                source: BlockSource {
                    git_repo: text("git_repo")?.to_string(),
                    git_commit: commit.to_ascii_lowercase(),
                    git_path: text("git_path")?.to_string(),
                    form: Form::parse(text("form")?)?,
                },
            })
        })
        .collect::<Option<_>>()?;
    let conflicting = blocks.iter().any(|a| {
        blocks
            .iter()
            .any(|b| a.block.sha256 == b.block.sha256 && a.block != b.block)
    });
    (!conflicting).then_some(blocks)
}

/// The outcome of reading the two files. `Unknown` carries a reason that names
/// the problem, never a marker.
pub enum Load {
    Ready(Policy),
    Unknown(String),
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
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
    let Some(blocks) = instruction_blocks(sect) else {
        return unknown("instruction_blocks invalid".into());
    };

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
        markers: Markers::new(&markers).with_instructions(InstructionBlocks::new(
            &blocks.iter().map(|b| b.block.clone()).collect::<Vec<_>>(),
        )),
        instruction_blocks: blocks,
        terms: terms.iter().map(|t| normalize(t)).collect(),
    })
}

/// How a new accepted set relates to the accepted one (R2-D).
#[derive(Debug, PartialEq, Eq)]
pub enum Acceptance {
    /// Nothing accepted yet: the first set is accepted and audited.
    Bootstrap,
    /// Markers added or blocks removed, nothing else: access can only narrow, accept.
    Widened { added: usize },
    /// A marker removed or replaced, or an instruction block added: unknown
    /// until `accept-routing`.
    Narrowed {
        removed: usize,
        added: usize,
        blocks_added: usize,
    },
}

/// Compare accepted sets ([`Policy::accepted_set`]). A marker row lowers
/// access when added; an `instruction:` row raises it when added.
pub fn compare(accepted: &BTreeSet<String>, new: &BTreeSet<String>) -> Acceptance {
    let is_block = |h: &&String| h.starts_with(BLOCK_ROW);
    if accepted.is_empty() {
        // Review U2: a first set with blocks is never accepted by the hub.
        let blocks_added = new.iter().filter(is_block).count();
        if blocks_added == 0 {
            return Acceptance::Bootstrap;
        }
        return Acceptance::Narrowed {
            removed: 0,
            added: new.len() - blocks_added,
            blocks_added,
        };
    }
    let removed = accepted.difference(new).filter(|h| !is_block(h)).count();
    let added = new.difference(accepted).filter(|h| !is_block(h)).count();
    let blocks_added = new.difference(accepted).filter(is_block).count();
    if removed == 0 && blocks_added == 0 {
        Acceptance::Widened { added }
    } else {
        Acceptance::Narrowed {
            removed,
            added,
            blocks_added,
        }
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
                added: 0,
                blocks_added: 0
            }
        );
        // The same count with one marker replaced is still a narrowing.
        assert_eq!(
            compare(&accepted, &set(&["a", "b", "x"])),
            Acceptance::Narrowed {
                removed: 1,
                added: 1,
                blocks_added: 0
            }
        );
    }

    /// The entries for `text` in both forms, with a source the loader only
    /// checks for shape (`accept-routing` verifies it).
    fn block_entry(text: &str) -> String {
        Form::ALL
            .into_iter()
            .map(|form| {
                RegisteredBlock {
                    block: InstructionBlock::of(&form.of(text)),
                    source: BlockSource {
                        git_repo: "/r".into(),
                        git_commit: "a".repeat(40),
                        git_path: "CLAUDE.md".into(),
                        form,
                    },
                }
                .entry_line()
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn with_blocks(entries: &str) -> String {
        GOOD.replace(
            "content_terms = [\"clientword\"]",
            &format!("content_terms = [\"clientword\"]\ninstruction_blocks = [{entries}]"),
        )
    }

    #[test]
    fn registered_instruction_blocks_load_and_subtract() {
        let fs = Files::new("blocks");
        let text = format!("## Rules\n{}see /clientco/notes\n", "lab line. ".repeat(30));
        let (r, f) = fs.write(&with_blocks(&block_entry(&text)), None);
        let Load::Ready(p) = load_paths(&r, &f) else {
            panic!("expected ready");
        };
        assert_eq!(
            p.instruction_blocks.len(),
            2,
            "the injected form and the search_text form"
        );
        assert!(
            !p.markers.hits_message_text(&text),
            "the block is subtracted"
        );
        assert!(p.markers.hits_message_text(&format!("{text}/clientco")));
        assert!(
            !p.markers
                .hits_message_text(&format!("q {}", search_form(&text))),
            "the search_text form is subtracted"
        );
        assert_eq!(
            p.accepted_set().len(),
            5,
            "3 markers and 2 blocks in the accepted set"
        );
        // No key: no blocks, as before.
        let (r, f) = fs.write(GOOD, None);
        let Load::Ready(p) = load_paths(&r, &f) else {
            panic!("expected ready");
        };
        assert!(p.instruction_blocks.is_empty());
    }

    #[test]
    fn a_bad_instruction_entry_is_unknown() {
        let fs = Files::new("badblocks");
        let good_text = format!("{}end", "lab line. ".repeat(30));
        let good = block_entry(&good_text);
        let one = good.split(" }, ").next().unwrap().to_string() + " }";
        let sha = hex(&InstructionBlock::of(&good_text).sha256);
        let bad = [
            block_entry("## Rules\nsee /clientco\n"),
            one.replace("sha256 = \"", "sha256 = \"zz"),
            one.replace(&sha, &"+1".repeat(32)), // review U6
            one.replace(&sha, &sha[..63]),
            one.replace("head_sha256", "head"),
            one.replace("bytes = 303", "bytes = \"303\""),
            one.replace(&"a".repeat(40), "main"),
            one.replace("form = \"injected\"", "form = \"other\""),
            one.replace("git_path = \"CLAUDE.md\", ", ""),
            // One sha256 with two descriptors (review U1).
            format!("{one}, {}", one.replace("bytes = 303", "bytes = 304")),
            "\"just a string\"".to_string(),
        ];
        assert!(
            bad.iter().all(|b| *b != good && *b != one),
            "every bad entry differs"
        );
        for entry in bad {
            let (r, f) = fs.write(&with_blocks(&entry), None);
            assert_eq!(
                reason(load_paths(&r, &f)),
                "instruction_blocks invalid",
                "{entry}"
            );
        }
        let (r, f) = fs.write(
            &GOOD.replace("model", "instruction_blocks = 1\nmodel"),
            None,
        );
        assert_eq!(reason(load_paths(&r, &f)), "instruction_blocks invalid");
        // Upper-case hex is still hex.
        let (r, f) = fs.write(&with_blocks(&one.replace(&sha, &sha.to_uppercase())), None);
        assert_eq!(reason(load_paths(&r, &f)), "ready");
    }

    /// A block lowers what marks: adding one is a narrowing, removing one is not.
    #[test]
    fn adding_an_instruction_block_is_a_narrowing() {
        let set = |xs: &[&str]| xs.iter().map(|s| (*s).to_string()).collect::<BTreeSet<_>>();
        let accepted = set(&["a", "b", "instruction:x"]);
        assert_eq!(
            compare(
                &accepted,
                &set(&["a", "b", "instruction:x", "instruction:y"])
            ),
            Acceptance::Narrowed {
                removed: 0,
                added: 0,
                blocks_added: 1
            }
        );
        assert_eq!(
            compare(&accepted, &set(&["a", "b"])),
            Acceptance::Widened { added: 0 },
            "a removed block marks more"
        );
        assert_eq!(
            compare(&accepted, &set(&["a", "b", "c", "instruction:x"])),
            Acceptance::Widened { added: 1 }
        );
        // Review U2: an empty accepted set bootstraps markers only.
        assert_eq!(
            compare(&BTreeSet::new(), &set(&["a", "instruction:x"])),
            Acceptance::Narrowed {
                removed: 0,
                added: 1,
                blocks_added: 1
            }
        );
        assert_eq!(
            compare(&BTreeSet::new(), &set(&["a"])),
            Acceptance::Bootstrap
        );
    }

    /// Review U1: a changed length or head hash for an accepted sha256 is a new
    /// row, so a narrowing.
    #[test]
    fn a_changed_descriptor_is_a_new_row() {
        let text = format!("{}end", "lab line. ".repeat(30));
        let rb = RegisteredBlock {
            block: InstructionBlock::of(&text),
            source: BlockSource {
                git_repo: "/r".into(),
                git_commit: "a".repeat(40),
                git_path: "CLAUDE.md".into(),
                form: Form::Injected,
            },
        };
        let mut other = rb.clone();
        other.block.bytes += 1;
        let accepted: BTreeSet<String> = ["m".to_string(), rb.row()].into();
        let new: BTreeSet<String> = ["m".to_string(), other.row()].into();
        assert!(matches!(
            compare(&accepted, &new),
            Acceptance::Narrowed {
                blocks_added: 1,
                ..
            }
        ));
    }

    /// A throwaway git repository holding `CLAUDE.md` with `text`, committed.
    fn repo_with(text: &str) -> (Files, String) {
        let fs = Files::new("git");
        let repo = fs.dir.to_str().unwrap().to_string();
        let run = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(["-C", &repo, "-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .output()
                .unwrap();
            assert!(ok.status.success(), "{args:?}");
        };
        run(&["init", "-q"]);
        std::fs::write(fs.dir.join("CLAUDE.md"), text).unwrap();
        run(&["add", "CLAUDE.md"]);
        run(&["commit", "-q", "-m", "v1"]);
        (fs, repo)
    }

    /// Review U1: entries come from git, and `verify` re-derives each one.
    #[test]
    fn entries_from_git_verify_and_a_tampered_one_does_not() {
        let text = format!("## Rules\n{}see /clientco/notes\n", "lab line. ".repeat(30));
        let (_fs, repo) = repo_with(&text);
        let m = Markers::new(&["/clientco".to_string()]);
        let entries = entries_from_git(&repo, "HEAD", "CLAUDE.md", &m).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].source.git_commit.len(), 40);
        for e in &entries {
            e.verify().unwrap();
        }
        assert_eq!(entries[0].block, InstructionBlock::of(text.trim_end()));
        let mut forged = entries[0].clone();
        forged.block = InstructionBlock::of(&format!("{}\nmore", text.trim_end()));
        assert!(forged.verify().is_err());
        // Review U5: a file whose end could start a marker is refused.
        let (_fs2, repo2) = repo_with(&format!("{text}see /cli"));
        assert!(entries_from_git(&repo2, "HEAD", "CLAUDE.md", &m).is_err());
    }
}
