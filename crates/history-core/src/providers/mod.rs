use serde::{Deserialize, Serialize};

pub mod aider;
pub mod antigravity;
pub mod claude;
pub mod cline;
pub mod codebuddy;
pub mod codex;
pub mod copilot;
pub mod copilot_cli;
pub mod cursor;
pub mod cursor_agent;
pub mod forgecode;
pub mod gemini;
pub mod kimi;
pub mod kiro;
pub mod opencode;
pub mod vscode;

/// Provider identifier
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Aider,
    Claude,
    Cline,
    Codebuddy,
    Codex,
    /// Unified GitHub Copilot provider covering CLI, Desktop, and the VS Code
    /// Copilot Chat extension. Per-session disambiguation lives in the
    /// `entrypoint` field (`copilot-cli` / `copilot-desktop` / `copilot-vscode`).
    Copilot,
    Cursor,
    #[serde(rename = "cursor-agent")]
    CursorAgent,
    Gemini,
    Kimi,
    ForgeCode,
    Kiro,
    OpenCode,
    Antigravity,
}

impl ProviderId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Aider => "aider",
            Self::Claude => "claude",
            Self::Cline => "cline",
            Self::Codebuddy => "codebuddy",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::Cursor => "cursor",
            Self::CursorAgent => "cursor-agent",
            Self::Gemini => "gemini",
            Self::Kimi => "kimi",
            Self::ForgeCode => "forgecode",
            Self::Kiro => "kiro",
            Self::OpenCode => "opencode",
            Self::Antigravity => "antigravity",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "aider" => Some(Self::Aider),
            "claude" => Some(Self::Claude),
            "cline" => Some(Self::Cline),
            "codebuddy" => Some(Self::Codebuddy),
            "codex" => Some(Self::Codex),
            "copilot" => Some(Self::Copilot),
            "cursor" => Some(Self::Cursor),
            "cursor-agent" => Some(Self::CursorAgent),
            "gemini" => Some(Self::Gemini),
            "kimi" => Some(Self::Kimi),
            "forgecode" => Some(Self::ForgeCode),
            "kiro" => Some(Self::Kiro),
            "opencode" => Some(Self::OpenCode),
            "antigravity" => Some(Self::Antigravity),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Aider => "Aider",
            Self::Claude => "Claude Code",
            Self::Cline => "Cline",
            Self::Codebuddy => "CodeBuddy Code",
            Self::Codex => "Codex CLI",
            Self::Copilot => "Copilot",
            Self::Cursor => "Cursor",
            Self::CursorAgent => "Cursor Agent",
            Self::Gemini => "Gemini CLI",
            Self::Kimi => "Kimi CLI",
            Self::ForgeCode => "ForgeCode",
            Self::Kiro => "Kiro CLI",
            Self::OpenCode => "OpenCode",
            Self::Antigravity => "Antigravity",
        }
    }
}

/// Information about a detected provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub base_path: String,
    pub is_available: bool,
}

/// Detect all available providers on the system
pub fn detect_providers() -> Vec<ProviderInfo> {
    let mut providers = Vec::new();

    if let Some(info) = claude::detect() {
        providers.push(info);
    }
    if let Some(info) = codex::detect() {
        providers.push(info);
    }
    if let Some(info) = gemini::detect() {
        providers.push(info);
    }
    if let Some(info) = kimi::detect() {
        providers.push(info);
    }
    if let Some(info) = forgecode::detect() {
        providers.push(info);
    }
    if let Some(info) = opencode::detect() {
        providers.push(info);
    }
    if let Some(info) = cline::detect() {
        providers.push(info);
    }
    if let Some(info) = cursor::detect() {
        providers.push(info);
    }
    if let Some(info) = cursor_agent::detect() {
        providers.push(info);
    }
    if let Some(info) = aider::detect() {
        providers.push(info);
    }
    if let Some(info) = antigravity::detect() {
        providers.push(info);
    }
    if let Some(info) = codebuddy::detect() {
        providers.push(info);
    }
    if let Some(info) = kiro::detect() {
        providers.push(info);
    }
    if let Some(info) = copilot::detect() {
        providers.push(info);
    }

    providers
}

// ---------------------------------------------------------------------------
// Unified provider dispatch
//
// The canonical "scan/load by provider" surface, used by headless consumers
// (the sync daemon) and available to the desktop. Each provider module exposes
// the same contract; this maps a `ProviderId` to it and stamps the provider on
// the returned records. (The desktop's `multi_provider` adds extra features —
// WSL, custom Claude paths, Copilot path variants, tool-execution merging —
// on top of this baseline.)
// ---------------------------------------------------------------------------

use crate::models::{ClaudeMessage, ClaudeProject, ClaudeSession};

fn tag_projects(
    out: &mut Vec<ClaudeProject>,
    res: Result<Vec<ClaudeProject>, String>,
    id: ProviderId,
) {
    match res {
        Ok(mut ps) => {
            for p in &mut ps {
                if p.provider.is_none() {
                    p.provider = Some(id.as_str().to_string());
                }
            }
            out.append(&mut ps);
        }
        Err(e) => log::warn!("scan_projects failed for {}: {e}", id.as_str()),
    }
}

/// Scan projects across every supported provider at its default location,
/// tagging each project with its provider. Errors from one provider are logged
/// and skipped (a missing/unreadable provider never fails the whole scan).
#[must_use]
pub fn scan_all_projects() -> Vec<ClaudeProject> {
    let mut out: Vec<ClaudeProject> = Vec::new();
    if let Some(base) = claude::get_base_path() {
        tag_projects(&mut out, claude::scan_projects(&base), ProviderId::Claude);
    }
    tag_projects(&mut out, codex::scan_projects(), ProviderId::Codex);
    tag_projects(&mut out, copilot::scan_projects(), ProviderId::Copilot);
    tag_projects(&mut out, gemini::scan_projects(), ProviderId::Gemini);
    tag_projects(&mut out, kimi::scan_projects(), ProviderId::Kimi);
    tag_projects(&mut out, forgecode::scan_projects(), ProviderId::ForgeCode);
    tag_projects(&mut out, opencode::scan_projects(), ProviderId::OpenCode);
    tag_projects(&mut out, cline::scan_projects(), ProviderId::Cline);
    tag_projects(&mut out, cursor::scan_projects(), ProviderId::Cursor);
    tag_projects(
        &mut out,
        cursor_agent::scan_projects(),
        ProviderId::CursorAgent,
    );
    tag_projects(&mut out, aider::scan_projects(), ProviderId::Aider);
    tag_projects(
        &mut out,
        antigravity::scan_projects(),
        ProviderId::Antigravity,
    );
    tag_projects(&mut out, codebuddy::scan_projects(), ProviderId::Codebuddy);
    tag_projects(&mut out, kiro::scan_projects(), ProviderId::Kiro);
    out
}

/// Load the sessions of a project for the given provider (provider-stamped).
pub fn load_sessions(
    provider: ProviderId,
    project_path: &str,
    exclude_sidechain: bool,
) -> Result<Vec<ClaudeSession>, String> {
    let mut sessions = match provider {
        ProviderId::Claude => claude::load_sessions(project_path, Some(exclude_sidechain))?,
        ProviderId::Codex => codex::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::Copilot => copilot::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::Gemini => gemini::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::Kimi => kimi::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::ForgeCode => forgecode::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::OpenCode => opencode::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::Cline => cline::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::Cursor => cursor::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::CursorAgent => cursor_agent::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::Aider => aider::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::Antigravity => antigravity::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::Codebuddy => codebuddy::load_sessions(project_path, exclude_sidechain)?,
        ProviderId::Kiro => kiro::load_sessions(project_path, exclude_sidechain)?,
    };
    for s in &mut sessions {
        if s.provider.is_none() {
            s.provider = Some(provider.as_str().to_string());
        }
    }
    Ok(sessions)
}

/// Load the messages of a session for the given provider (provider-stamped).
pub fn load_messages(
    provider: ProviderId,
    session_path: &str,
) -> Result<Vec<ClaudeMessage>, String> {
    let mut messages = match provider {
        ProviderId::Claude => claude::load_messages(session_path)?,
        ProviderId::Codex => codex::load_messages(session_path)?,
        ProviderId::Copilot => copilot::load_messages(session_path)?,
        ProviderId::Gemini => gemini::load_messages(session_path)?,
        ProviderId::Kimi => kimi::load_messages(session_path)?,
        ProviderId::ForgeCode => forgecode::load_messages(session_path)?,
        ProviderId::OpenCode => opencode::load_messages(session_path)?,
        ProviderId::Cline => cline::load_messages(session_path)?,
        ProviderId::Cursor => cursor::load_messages(session_path)?,
        ProviderId::CursorAgent => cursor_agent::load_messages(session_path)?,
        ProviderId::Aider => aider::load_messages(session_path)?,
        ProviderId::Antigravity => antigravity::load_messages(session_path)?,
        ProviderId::Codebuddy => codebuddy::load_messages(session_path)?,
        ProviderId::Kiro => kiro::load_messages(session_path)?,
    };
    for m in &mut messages {
        if m.provider.is_none() {
            m.provider = Some(provider.as_str().to_string());
        }
    }
    Ok(messages)
}
