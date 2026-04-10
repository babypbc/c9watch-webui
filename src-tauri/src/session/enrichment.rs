use crate::session::{
    determine_status, get_pending_tool_input, get_pending_tool_name, parse_last_n_entries,
    parse_sessions_index, SessionDetector, SessionStatus,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::{LazyLock, Mutex};

/// Combined session information
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub pid: u32,
    pub session_name: String,
    pub custom_title: Option<String>,
    pub project_path: String,
    pub git_branch: Option<String>,
    pub git_status: Option<String>,
    pub first_prompt: String,
    pub summary: Option<String>,
    pub message_count: u32,
    pub modified: String,
    pub status: SessionStatus,
    pub latest_message: String,
    pub latest_user_message: String,
    pub pending_tool_name: Option<String>,
    /// The input/arguments of the pending tool (when status is NeedsPermission)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_tool_input: Option<serde_json::Value>,
    /// Current context window usage (used/max in tokens)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_usage: Option<ContextUsage>,
}

/// Context window usage information
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    /// Tokens currently used in context
    pub used: u64,
    /// Maximum context window size for the model
    pub max: u64,
    /// Percentage used (0-100)
    pub percentage: f64,
}

/// Cache for native custom titles, keyed by file path.
/// Stores (mtime_as_nanos, cached_title) to avoid re-scanning JSONL files every poll cycle.
static NATIVE_TITLE_CACHE: LazyLock<Mutex<HashMap<std::path::PathBuf, (u64, Option<String>)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Look up the native custom title for a session JSONL, using a mtime-based cache.
pub(crate) fn get_cached_native_title(path: &Path) -> Option<String> {
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    if let Ok(mut cache) = NATIVE_TITLE_CACHE.lock() {
        if let Some((cached_mtime, cached_title)) = cache.get(path) {
            if *cached_mtime == mtime {
                crate::debug_log::log_info(&format!(
                    "Native title cache hit for {:?}: title={:?}",
                    path, cached_title
                ));
                return cached_title.clone();
            } else {
                crate::debug_log::log_info(&format!(
                    "Native title cache stale for {:?}: cached_mtime={} != current_mtime={}",
                    path, cached_mtime, mtime
                ));
            }
        }
        // Cache miss or stale — re-scan
        let title = crate::session::parser::get_native_custom_title_from_file(path);
        crate::debug_log::log_info(&format!(
            "Native title cache miss for {:?}: read title={:?}, mtime={}",
            path, title, mtime
        ));
        cache.insert(path.to_path_buf(), (mtime, title.clone()));
        title
    } else {
        // Mutex poisoned — fallback to direct read
        crate::session::parser::get_native_custom_title_from_file(path)
    }
}

/// Detect sessions and enrich them with status and conversation data
pub fn detect_and_enrich_sessions(
) -> Result<(Vec<Session>, crate::session::DetectionDiagnostics), String> {
    let mut detector =
        SessionDetector::new().map_err(|e| format!("Failed to create session detector: {}", e))?;
    detect_and_enrich_sessions_with_detector(&mut detector)
}

/// Detect sessions using an existing detector (avoids recreating System each call)
pub fn detect_and_enrich_sessions_with_detector(
    detector: &mut SessionDetector,
) -> Result<(Vec<Session>, crate::session::DetectionDiagnostics), String> {
    let (detected_sessions, diagnostics) = detector
        .detect_sessions()
        .map_err(|e| format!("Failed to detect sessions: {}", e))?;

    let custom_names = crate::session::CustomNames::load();
    let custom_titles = crate::session::CustomTitles::load();
    let mut sessions = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    for detected in detected_sessions {
        // Get session ID - if not found, skip this session
        let session_id = match &detected.session_id {
            Some(id) => id.clone(),
            None => {
                continue;
            }
        };

        // Skip duplicate session IDs (same session can appear in multiple project dirs)
        if seen_ids.contains(&session_id) {
            continue;
        }
        seen_ids.insert(session_id.clone());

        // Try to parse sessions-index.json to get basic info (optional)
        // sessions-index.json is in the actual project's .claude/ directory
        let index_path = detected.cwd.join(".claude").join("sessions-index.json");
        let sessions_index = parse_sessions_index(&index_path).ok();

        // Find the matching entry in the index (if index exists)
        let session_entry = sessions_index.as_ref().and_then(|index| {
            index
                .entries
                .iter()
                .find(|entry| entry.session_id == session_id)
        });

        let (first_prompt, summary, message_count, modified, git_branch, git_status) = match session_entry {
            Some(entry) => {
                // Guard: if sessions-index first_prompt is a system command, try JSONL fallback
                let fp = if crate::session::parser::is_system_content(&entry.first_prompt) {
                    let session_file_path =
                        detected.project_path.join(format!("{}.jsonl", session_id));
                    get_first_prompt_from_jsonl(&session_file_path)
                        .unwrap_or_else(|| entry.first_prompt.clone())
                } else {
                    entry.first_prompt.clone()
                };
                // Get git status from the actual project directory (sessions-index doesn't track it)
                let git_status = get_git_status_summary(&detected.cwd);
                (
                    fp,
                    entry.summary.clone(),
                    entry.message_count,
                    entry.modified.clone(),
                    Some(entry.git_branch.clone()),
                    git_status,
                )
            }
            None => {
                // Session not in index or index doesn't exist - use fallback values
                let session_file_path =
                    detected.project_path.join(format!("{}.jsonl", session_id));

                // Try to get first prompt from JSONL file
                let first_prompt = get_first_prompt_from_jsonl(&session_file_path)
                    .unwrap_or_else(|| "(Active session)".to_string());

                // Count messages in the file
                let message_count = count_messages_in_jsonl(&session_file_path);

                // Get file modification time
                let modified = std::fs::metadata(&session_file_path)
                    .and_then(|m| m.modified())
                    .ok()
                    .map(|t| {
                        let datetime: DateTime<Utc> = t.into();
                        datetime.to_rfc3339()
                    })
                    .unwrap_or_default();

                // Get git branch and status from the actual project directory (detected.cwd)
                crate::debug_log::log_info(&format!(
                    "Session {}: fetching git info from cwd={:?}",
                    session_id, detected.cwd
                ));
                let git_branch = get_git_branch(&detected.cwd);
                let git_status = get_git_status_summary(&detected.cwd);

                (first_prompt, None, message_count, modified, git_branch, git_status)
            }
        };

        // Parse the session JSONL file to determine status and get latest message
        let session_file_path = detected.project_path.join(format!("{}.jsonl", session_id));
        let entries = match parse_last_n_entries(&session_file_path, 200) {
            Ok(entries) => entries,
            Err(e) => {
                crate::debug_log::log_warn(&format!(
                    "Failed to parse session file for {}: {}",
                    session_id, e
                ));
                vec![]
            }
        };

        let status = if entries.is_empty() {
            SessionStatus::Connecting
        } else {
            let raw_status = determine_status(&entries);
            // Override WaitingForInput if the JSONL file was recently modified.
            if raw_status == SessionStatus::WaitingForInput
                && is_file_recently_modified(&session_file_path, 8)
            {
                SessionStatus::Working
            } else {
                raw_status
            }
        };

        let latest_message = get_latest_assistant_message(&entries);
        let latest_user_message = get_latest_user_message(&entries);
        let pending_tool_name = get_pending_tool_name(&entries);
        let pending_tool_input = get_pending_tool_input(&entries);

        // Skip empty sessions (0 messages)
        if message_count == 0 {
            continue;
        }

        // Use custom name if available, otherwise use detected project name
        let session_name = custom_names
            .get(&session_id)
            .cloned()
            .unwrap_or(detected.project_name);

        // Get custom title: Claude Code native /rename takes priority over c9watch's own.
        // Uses a static cache keyed by (path, mtime) to avoid re-scanning the JSONL every cycle.
        let native_title = get_cached_native_title(&session_file_path);
        crate::debug_log::log_info(&format!(
            "Session {}: native_title={:?}, custom_titles={:?}",
            session_id, native_title, custom_titles.get(&session_id)
        ));
        crate::debug_log::log_info(&format!(
            "Session {}: git_branch={:?}, git_status={:?}",
            session_id, git_branch, git_status
        ));
        let context_usage = get_context_usage(&session_file_path);
        crate::debug_log::log_info(&format!(
            "Session {}: context_usage={:?}",
            session_id, context_usage
        ));
        let custom_title =
            native_title.or_else(|| custom_titles.get(&session_id).cloned());

        sessions.push(Session {
            id: session_id,
            pid: detected.pid,
            session_name,
            custom_title,
            project_path: detected.cwd.to_string_lossy().to_string(),
            git_branch,
            git_status,
            first_prompt,
            summary,
            message_count,
            modified,
            status,
            latest_message: latest_message,
            latest_user_message: latest_user_message,
            pending_tool_name,
            pending_tool_input,
            context_usage,
        });
    }

    Ok((sessions, diagnostics))
}

/// Checks if a file was modified within the last N seconds
pub fn is_file_recently_modified(path: &Path, seconds: u64) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(|modified| {
            modified
                .elapsed()
                .map(|elapsed| elapsed.as_secs() < seconds)
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Get the current git branch name for a directory
pub fn get_git_branch(project_path: &Path) -> Option<String> {
    std::process::Command::new("git")
        .arg("branch")
        .arg("--show-current")
        .current_dir(project_path)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_string()
                    .into()
            } else {
                None
            }
        })
}

/// Get the max context window size for a model (in tokens)
fn get_model_context_window(model: &str) -> u64 {
    // Source: Anthropic API documentation
    if model.contains("claude-sonnet-4") || model.contains("claude-sonnet4") {
        200_000
    } else if model.contains("claude-opus-4-6") || model.contains("claude-opus-4-5") {
        200_000
    } else if model.contains("claude-opus") {
        200_000
    } else if model.contains("claude-haiku-4-5") || model.contains("claude-haiku4") {
        200_000
    } else if model.contains("claude-haiku") {
        200_000
    } else if model.contains("claude-3-5-sonnet") {
        200_000
    } else if model.contains("claude-3-opus") {
        200_000
    } else if model.contains("claude-3-haiku") {
        200_000
    } else if model.contains("kimi") {
        256_000  // Kimi K2.5 supports 256K
    } else {
        200_000  // Default to 200K for unknown models
    }
}

/// Calculate context window usage from session JSONL file
/// Uses the last assistant message's usage data, which represents the current context state
pub fn get_context_usage(session_file_path: &Path) -> Option<ContextUsage> {
    let file = File::open(session_file_path).ok()?;
    let reader = BufReader::new(file);

    let mut last_cache_read = 0u64;
    let mut last_cache_creation = 0u64;
    let mut last_model = String::new();

    // We only need the last assistant message's usage, which represents current context
    for line in reader.lines().map_while(Result::ok) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                continue;
            }

            if let Some(msg) = value.get("message") {
                if let Some(model) = msg.get("model").and_then(|m| m.as_str()) {
                    last_model = model.to_string();
                }

                if let Some(usage) = msg.get("usage") {
                    last_cache_read = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    last_cache_creation = usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                }
            }
        }
    }

    // Current context = cached tokens + new input tokens (cache_creation)
    // This represents what's currently in the model's context window
    let used = last_cache_read + last_cache_creation;

    if used == 0 {
        return None;
    }

    let max_tokens = get_model_context_window(&last_model);
    let percentage = (used as f64 / max_tokens as f64) * 100.0;

    Some(ContextUsage {
        used,
        max: max_tokens,
        percentage: percentage.min(100.0),
    })
}

/// Get git status summary as "+N -M" (changed/deleted files) for a directory
pub fn get_git_status_summary(project_path: &Path) -> Option<String> {
    std::process::Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .current_dir(project_path)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let status = String::from_utf8_lossy(&output.stdout);
                if status.trim().is_empty() {
                    return None;
                }

                let mut added = 0u32;
                let mut deleted = 0u32;

                for line in status.lines() {
                    if line.len() >= 2 {
                        let x = line.as_bytes()[0] as char;
                        let y = line.as_bytes()[1] as char;

                        // Count based on status codes
                        match (x, y) {
                            // Modified, Renamed, Copied, Untracked - count as added
                            ('M' | 'A' | 'R' | 'C' | '?', _) => added += 1,
                            // Deleted
                            ('D', _) => deleted += 1,
                            // Second column: D means deleted
                            (_, 'D') => deleted += 1,
                            // Modified in working tree
                            (_, 'M') => added += 1,
                            _ => {}
                        }
                    }
                }

                if added > 0 || deleted > 0 {
                    Some(format!("+{} -{}", added, deleted))
                } else {
                    None
                }
            } else {
                None
            }
        })
}

/// Extract the first user prompt from a session JSONL file (truncated to 100 chars).
pub fn get_first_prompt_from_jsonl(path: &Path) -> Option<String> {
    get_first_prompt_from_jsonl_raw(path).map(|s| truncate_string(&s, 100))
}

/// Extract the first user prompt from a session JSONL file (full text, no truncation).
pub fn get_first_prompt_from_jsonl_raw(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().map_while(Result::ok).take(50) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if value.get("type").and_then(|t| t.as_str()) == Some("user") {
                if let Some(message) = value.get("message") {
                    if let Some(content) = message.get("content") {
                        if let Some(text) = content.as_str() {
                            // Skip system-generated local command messages
                            if crate::session::parser::is_system_content(text) {
                                continue;
                            }
                            return Some(text.to_string());
                        } else if let Some(arr) = content.as_array() {
                            for item in arr {
                                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    if let Some(text) =
                                        item.get("text").and_then(|t| t.as_str())
                                    {
                                        return Some(text.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Truncate a string to a maximum length (character-safe for UTF-8)
pub fn truncate_string(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}

/// Extract the latest user message content from session entries
pub fn get_latest_user_message(
    entries: &[crate::session::parser::SessionEntry],
) -> String {
    if entries.is_empty() {
        return String::new();
    }

    for entry in entries.iter().rev() {
        if let crate::session::parser::SessionEntry::User { message, .. } = entry {
            // Skip system-generated command messages
            if crate::session::parser::is_system_content(&message.content) {
                crate::debug_log::log_info(&format!(
                    "Skipping user message (system content): {}",
                    truncate_string(&message.content, 50)
                ));
                continue;
            }
            // Skip tool result messages (they are not actual user input)
            if message.is_tool_result {
                crate::debug_log::log_info(&format!(
                    "Skipping user message (is_tool_result): {}",
                    truncate_string(&message.content, 50)
                ));
                continue;
            }
            crate::debug_log::log_info(&format!(
                "Found user message (content_len={}): {}",
                message.content.len(),
                truncate_string(&message.content, 50)
            ));
            return truncate_string(&message.content, 200);
        }
    }

    crate::debug_log::log_info("No user message found in entries");
    String::new()
}

/// Extract the latest assistant message content from session entries
pub fn get_latest_assistant_message(
    entries: &[crate::session::parser::SessionEntry],
) -> String {
    if entries.is_empty() {
        return String::new();
    }

    for entry in entries.iter().rev() {
        if let crate::session::parser::SessionEntry::Assistant { message, .. } = entry {
            for content in message.content.iter().rev() {
                match content {
                    crate::session::parser::MessageContent::Text { text } => {
                        return truncate_string(text, 200);
                    }
                    crate::session::parser::MessageContent::Thinking { thinking, .. } => {
                        return truncate_string(thinking, 200);
                    }
                    crate::session::parser::MessageContent::ToolUse { name, .. } => {
                        return format!("Executing {}...", name);
                    }
                    _ => continue,
                }
            }
        }
    }

    String::new()
}

/// Count user/assistant messages in a JSONL file.
/// Skips system-injected user messages (local commands, slash commands, etc.)
pub fn count_messages_in_jsonl(path: &Path) -> u32 {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return 0,
    };
    let reader = BufReader::new(file);
    let mut count = 0u32;

    for line in reader.lines().map_while(Result::ok) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(msg_type) = value.get("type").and_then(|t| t.as_str()) {
                match msg_type {
                    "assistant" => count += 1,
                    "user" => {
                        // Skip system-injected user messages
                        if let Some(content) = value
                            .get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            if !crate::session::parser::is_system_content(content) {
                                count += 1;
                            }
                        } else {
                            // Array content (tool results) — still count them
                            count += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_string_no_truncation() {
        assert_eq!(truncate_string("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_string_exact_boundary() {
        assert_eq!(truncate_string("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_string_over_boundary() {
        assert_eq!(truncate_string("hello world", 5), "hello...");
    }

    #[test]
    fn test_truncate_string_empty() {
        assert_eq!(truncate_string("", 10), "");
    }

    #[test]
    fn test_truncate_string_zero_max() {
        assert_eq!(truncate_string("hello", 0), "...");
        assert!(!truncate_string("hello", 0).contains('h'));
    }

    #[test]
    fn test_truncate_string_single_char_limit() {
        assert_eq!(truncate_string("hello", 1), "h...");
    }

    #[test]
    fn test_truncate_string_utf8_accented() {
        assert_eq!(truncate_string("héllo", 3), "hél...");
    }

    #[test]
    fn test_truncate_string_utf8_cjk() {
        assert_eq!(truncate_string("你好世界", 2), "你好...");
    }

    #[test]
    fn test_truncate_string_utf8_emoji() {
        assert_eq!(truncate_string("Hello 👋 World", 7), "Hello 👋...");
    }
}
