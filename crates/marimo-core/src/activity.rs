use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// パネルは要約を 1 行で出し、ツールチップで全文を出す。そのための元の文章を
/// 保存時に切りすぎないよう、上限は表示の都合でなく状態ファイルの大きさで決める。
pub const TEXT_MAX_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityKind {
    Tool,
    Message,
    Error,
    /// 一行表示が文字列だけだった古い形式の状態ファイルから読んだもの。
    Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Activity {
    pub kind: ActivityKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Activity {
    fn tool(tool: &str, summary: String, detail: Option<String>) -> Self {
        Self {
            kind: ActivityKind::Tool,
            tool: Some(tool.to_owned()),
            summary: clip(&collapse(&summary)),
            detail: detail
                .filter(|d| !d.trim().is_empty())
                .map(|d| clip(d.trim())),
        }
    }

    pub fn message(text: &str) -> Option<Self> {
        let summary = clip(&collapse(text));
        (!summary.is_empty()).then_some(Self {
            kind: ActivityKind::Message,
            tool: None,
            summary,
            detail: None,
        })
    }

    pub fn error(text: &str) -> Self {
        Self {
            kind: ActivityKind::Error,
            tool: None,
            summary: clip(&collapse(text)),
            detail: None,
        }
    }
}

/// 古い形式では `line` に文字列が入っていた。新しい形式の `activity` と同じ項目として
/// 読み、文字列なら要約だけを持つ Activity にする。
pub fn deserialize_compat<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Activity>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Compat {
        Structured(Activity),
        Legacy(String),
        // 想定外の値でもファイル全体を読めなくしないよう、ここで受け止めて捨てる。
        Other(serde::de::IgnoredAny),
    }
    Ok(match Option::<Compat>::deserialize(d)? {
        Some(Compat::Structured(a)) => Some(a),
        Some(Compat::Legacy(s)) if !s.trim().is_empty() => Some(Activity {
            kind: ActivityKind::Text,
            tool: None,
            summary: clip(&collapse(&s)),
            detail: None,
        }),
        _ => None,
    })
}

/// `tool_input` の項目名は https://code.claude.com/docs/en/hooks の PreToolUse input にある、
/// ツールごとの表に従う。
pub fn tool_activity(tool_name: &str, tool_input: Option<&Value>, cwd: Option<&str>) -> Activity {
    let get = |key: &str| {
        tool_input
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
    };
    let rel = |p: &str| relative_path(p, cwd);
    let name = if tool_name.is_empty() {
        "Tool"
    } else {
        tool_name
    };

    match name {
        "Bash" | "PowerShell" => {
            let command = get("command");
            let summary = get("description")
                .map(str::to_owned)
                .or_else(|| command.map(first_line))
                .unwrap_or_else(|| name.to_owned());
            Activity::tool(name, summary, command.map(str::to_owned))
        }
        "Edit" | "Write" | "Read" | "NotebookEdit" => {
            let verb = match name {
                "Edit" => "編集",
                "Write" => "書き込み",
                "Read" => "読み込み",
                _ => "ノートブック編集",
            };
            let path = get("file_path").or(get("notebook_path"));
            let summary = match path {
                Some(p) => format!("{verb}: {}", rel(p)),
                None => verb.to_owned(),
            };
            Activity::tool(name, summary, path.map(str::to_owned))
        }
        "Grep" | "Glob" => {
            let verb = if name == "Grep" {
                "検索"
            } else {
                "ファイル検索"
            };
            let summary = match get("pattern") {
                Some(p) => format!("{verb}: {p}"),
                None => verb.to_owned(),
            };
            let scope: Vec<String> = [get("path").map(rel), get("glob").map(str::to_owned)]
                .into_iter()
                .flatten()
                .collect();
            Activity::tool(name, summary, (!scope.is_empty()).then(|| scope.join("  ")))
        }
        "WebFetch" => {
            let url = get("url");
            let summary = match url {
                Some(u) => format!("取得: {}", host_and_path(u)),
                None => "取得".to_owned(),
            };
            Activity::tool(name, summary, url.map(str::to_owned))
        }
        "WebSearch" => {
            let summary = match get("query") {
                Some(q) => format!("Web 検索: {q}"),
                None => "Web 検索".to_owned(),
            };
            Activity::tool(name, summary, None)
        }
        "Agent" => {
            let summary = match get("description") {
                Some(d) => format!("サブエージェント: {d}"),
                None => "サブエージェント".to_owned(),
            };
            Activity::tool(name, summary, get("subagent_type").map(str::to_owned))
        }
        "AskUserQuestion" => {
            let first = tool_input.and_then(|v| v.pointer("/questions/0"));
            let summary = match first
                .and_then(|q| q.get("question"))
                .and_then(Value::as_str)
            {
                Some(q) => format!("質問: {q}"),
                None => "質問".to_owned(),
            };
            let options: Vec<&str> = first
                .and_then(|q| q.get("options"))
                .and_then(Value::as_array)
                .map(|opts| {
                    opts.iter()
                        .filter_map(|o| o.get("label").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default();
            Activity::tool(
                name,
                summary,
                (!options.is_empty()).then(|| options.join(" / ")),
            )
        }
        "ExitPlanMode" => Activity::tool(
            name,
            "計画の承認を依頼".to_owned(),
            get("planFilePath").map(str::to_owned),
        ),
        _ => {
            let summary = match mcp_parts(name) {
                Some((server, tool)) => format!("{server}: {}", tool.replace('_', " ")),
                None => name.to_owned(),
            };
            Activity::tool(name, summary, compact_args(tool_input))
        }
    }
}

// MCP のツール名は mcp__<server>__<tool> の形になる（hooks のドキュメントの Match MCP tools）。
fn mcp_parts(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    (!server.is_empty() && !tool.is_empty()).then_some((server, tool))
}

fn compact_args(tool_input: Option<&Value>) -> Option<String> {
    let obj = tool_input?.as_object()?;
    if obj.is_empty() {
        return None;
    }
    if let Some(s) = obj
        .values()
        .find_map(Value::as_str)
        .filter(|_| obj.len() == 1)
    {
        return Some(s.to_owned());
    }
    serde_json::to_string(obj).ok()
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(text)
        .trim()
        .to_owned()
}

// Windows のパスは \ 区切りで届く（hooks のドキュメントの PreToolUse input）ので、
// 両方の区切りを受け付ける。
pub fn relative_path(path: &str, cwd: Option<&str>) -> String {
    let Some(cwd) = cwd
        .map(|c| c.trim_end_matches(['/', '\\']))
        .filter(|c| !c.is_empty())
    else {
        return path.to_owned();
    };
    match path.strip_prefix(cwd) {
        Some(rest) if rest.starts_with(['/', '\\']) && rest.len() > 1 => rest[1..].to_owned(),
        _ => path.to_owned(),
    }
}

fn host_and_path(url: &str) -> String {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let end = without_scheme
        .find(['?', '#'])
        .unwrap_or(without_scheme.len());
    without_scheme[..end].trim_end_matches('/').to_owned()
}

pub fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// バイトでなく文字で数え、日本語でも見た目の長さで切る。
pub fn clip(text: &str) -> String {
    if text.chars().count() <= TEXT_MAX_CHARS {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(TEXT_MAX_CHARS - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn act(name: &str, input: Value) -> (String, Option<String>) {
        let a = tool_activity(name, Some(&input), Some("/w/proj"));
        assert_eq!(a.kind, ActivityKind::Tool);
        assert_eq!(a.tool.as_deref(), Some(name));
        (a.summary, a.detail)
    }

    #[test]
    fn tool_activity_reads_documented_input_fields() {
        let cases: Vec<(&str, Value, &str, Option<&str>)> = vec![
            (
                "Bash",
                json!({"command": "cargo test --workspace -- --nocapture", "description": "Run the test suite"}),
                "Run the test suite",
                Some("cargo test --workspace -- --nocapture"),
            ),
            (
                "Bash",
                json!({"command": "\n  cd app &&\n npm run build\n"}),
                "cd app &&",
                Some("cd app &&\n npm run build"),
            ),
            (
                "PowerShell",
                json!({"command": "Get-ChildItem"}),
                "Get-ChildItem",
                Some("Get-ChildItem"),
            ),
            (
                "Edit",
                json!({"file_path": "/w/proj/src/foo.rs", "old_string": "a", "new_string": "b"}),
                "編集: src/foo.rs",
                Some("/w/proj/src/foo.rs"),
            ),
            (
                "Write",
                json!({"file_path": "/elsewhere/x.txt"}),
                "書き込み: /elsewhere/x.txt",
                Some("/elsewhere/x.txt"),
            ),
            (
                "Read",
                json!({"file_path": "/w/proj/README"}),
                "読み込み: README",
                Some("/w/proj/README"),
            ),
            (
                "NotebookEdit",
                json!({"notebook_path": "/w/proj/a.ipynb"}),
                "ノートブック編集: a.ipynb",
                Some("/w/proj/a.ipynb"),
            ),
            (
                "Grep",
                json!({"pattern": "TODO.*fix", "path": "/w/proj/src", "glob": "*.rs"}),
                "検索: TODO.*fix",
                Some("src  *.rs"),
            ),
            (
                "Glob",
                json!({"pattern": "**/*.ts"}),
                "ファイル検索: **/*.ts",
                None,
            ),
            (
                "WebFetch",
                json!({"url": "https://code.claude.com/docs/en/hooks?x=1#top", "prompt": "p"}),
                "取得: code.claude.com/docs/en/hooks",
                Some("https://code.claude.com/docs/en/hooks?x=1#top"),
            ),
            (
                "WebSearch",
                json!({"query": "tauri autostart"}),
                "Web 検索: tauri autostart",
                None,
            ),
            (
                "Agent",
                json!({"description": "Find endpoints", "subagent_type": "Explore", "prompt": "..."}),
                "サブエージェント: Find endpoints",
                Some("Explore"),
            ),
            (
                "AskUserQuestion",
                json!({"questions": [{"question": "Which?", "options": [{"label": "A"}, {"label": "B"}]}]}),
                "質問: Which?",
                Some("A / B"),
            ),
            ("ExitPlanMode", json!({}), "計画の承認を依頼", None),
            (
                "mcp__claude-in-chrome__navigate_page",
                json!({"url": "https://example.com"}),
                "claude-in-chrome: navigate page",
                Some("https://example.com"),
            ),
            (
                "mcp__memory__create_entities",
                json!({"a": 1, "b": "x"}),
                "memory: create entities",
                Some(r#"{"a":1,"b":"x"}"#),
            ),
            ("SomeTool", json!({}), "SomeTool", None),
        ];
        for (name, input, summary, detail) in cases {
            assert_eq!(
                act(name, input.clone()),
                (summary.to_owned(), detail.map(str::to_owned)),
                "{name} {input}"
            );
        }
        assert_eq!(
            relative_path("C:\\w\\proj\\src\\a.ts", Some("C:\\w\\proj")),
            "src\\a.ts"
        );
        assert_eq!(
            relative_path("/w/project2/a", Some("/w/proj")),
            "/w/project2/a"
        );
    }

    #[test]
    fn long_text_is_clipped_by_chars() {
        let long = "あ".repeat(900);
        let (summary, detail) = act("Bash", json!({"command": long, "description": long}));
        assert_eq!(summary.chars().count(), TEXT_MAX_CHARS);
        assert!(summary.ends_with('…'));
        assert_eq!(detail.unwrap().chars().count(), TEXT_MAX_CHARS);
    }
}
