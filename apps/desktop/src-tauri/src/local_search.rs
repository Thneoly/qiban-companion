//! Local search (read-only): model-driven tools confined to the user's
//! authorized directories. L1 name search + L2 content search / slice
//! reading; L3 (execute/modify) is deliberately not a thing here.
//!
//! The engine half is PURE (no Tauri types): roots arrive canonicalized,
//! every model-supplied path is untrusted and must survive
//! canonicalize-containment plus per-entry reparse-point rejection (the
//! execution.rs sandbox idioms), and every budget (nodes, depth, wall
//! clock, bytes) is first-class in our own walk — no walker dependency.
//! The result payload IS the exact string replayed into the model request
//! and stored in the tool audit; field-level caps keep it under 8KB.
use companion_storage::local_search_store::{AuthorizedRoot, AuthorizedRootInsert, ToolOutcome};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs::Metadata;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::Manager;

pub const MAX_ROOTS: usize = 8;
/// Text extensions eligible for content search and slice reading. Binary
/// and oversized files are skipped, never guessed at.
pub const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "json", "toml", "yaml", "yml", "rs", "ts", "tsx", "js", "py", "ini", "cfg",
];
const WALK_NODES: usize = 50_000;
const WALK_DEPTH: usize = 24;
const WALK_TIME: Duration = Duration::from_secs(2);
const SCAN_BYTES: u64 = 8 * 1024 * 1024;
const NAMES_LIMIT: usize = 30;
const LINES_LIMIT: usize = 20;
const LINE_MATCH_CHARS: usize = 1_000; // match against at most this much of a line
const LINE_SHOW_CHARS: usize = 200;
const SLICE_CHARS: usize = 2_000; // caps the payload under 8KB even for CJK
const SLICE_MAX_FILE: u64 = 2 * 1024 * 1024;
const CONTENTS_MAX_FILE: u64 = 512 * 1024;
const PAYLOAD_CAP: usize = 8192;
/// One tool round may carry several calls; three keeps a turn cheap.
pub const MAX_CALLS_PER_ROUND: usize = 3;

/// Tool declaration shared with the provider request body (rendered
/// per-protocol by the chat loop in C2).
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "list_roots",
            description: "列出用户授权检索的目录（只读检索的前提；先调用它了解范围）。",
            parameters: json!({"type":"object","properties":{},"required":[]}),
        },
        ToolSpec {
            name: "search_names",
            description: "在授权目录内按名称子串检索文件与目录（不区分大小写），返回最多30条。",
            parameters: json!({"type":"object","properties":{
                "pattern":{"type":"string","description":"名称子串，1~200字符"}
            },"required":["pattern"]}),
        },
        ToolSpec {
            name: "search_contents",
            description: "在授权目录内的文本文件中按子串检索内容（不区分大小写），返回最多20行 path:行号:内容。",
            parameters: json!({"type":"object","properties":{
                "pattern":{"type":"string","description":"内容子串，1~200字符"}
            },"required":["pattern"]}),
        },
        ToolSpec {
            name: "read_slice",
            description: "读取授权目录内一个文本文件的片段（每页2000字符），返回内容与续读偏移。",
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"文件绝对路径，必须在授权目录内"},
                "offset":{"type":"integer","description":"字符偏移，缺省0"}
            },"required":["path"]}),
        },
    ]
}

/// The engine result: the audit outcome plus the exact payload string that
/// will be replayed into the model request.
pub struct ToolResult {
    pub outcome: ToolOutcome,
    pub payload: String,
}

impl ToolResult {
    fn ok(payload: Value) -> Self {
        Self::fit(ToolOutcome::Ok, payload)
    }
    fn denied(reason: &str) -> Self {
        Self {
            outcome: ToolOutcome::Denied,
            payload: json!({"error": reason}).to_string(),
        }
    }
    fn error(reason: &str) -> Self {
        Self {
            outcome: ToolOutcome::Error,
            payload: json!({"error": reason}).to_string(),
        }
    }
    /// Serialize with a hard cap: drop list items first, then truncate the
    /// long text fields — never emit a broken JSON string.
    fn fit(outcome: ToolOutcome, mut payload: Value) -> Self {
        loop {
            let text = payload.to_string();
            if text.len() <= PAYLOAD_CAP {
                return Self {
                    outcome,
                    payload: text,
                };
            }
            if let Some(items) = payload.get_mut("matches").and_then(Value::as_array_mut) {
                if items.len() > 1 {
                    items.pop();
                    if let Some(object) = payload.as_object_mut() {
                        object.insert("truncated".into(), Value::Bool(true));
                    }
                    continue;
                }
                // A single monstrous entry (not reachable with real path
                // lengths, but refuse to deadlock): truncate the string.
                if let Some(Value::String(entry)) = items.first_mut() {
                    let cut = entry
                        .char_indices()
                        .nth(SLICE_CHARS)
                        .map(|(index, _)| index)
                        .unwrap_or(entry.len());
                    if cut < entry.len() {
                        entry.truncate(cut);
                        if let Some(object) = payload.as_object_mut() {
                            object.insert("truncated".into(), Value::Bool(true));
                        }
                        continue;
                    }
                }
            }
            let mut shrunk = false;
            if let Some(Value::String(content)) = payload.get_mut("content") {
                // char_indices yields byte offsets at char boundaries, so
                // truncate() is safe even mid-CJK.
                let cut = content
                    .char_indices()
                    .nth(SLICE_CHARS)
                    .map(|(index, _)| index)
                    .unwrap_or(content.len());
                if cut < content.len() {
                    content.truncate(cut);
                    shrunk = true;
                }
            }
            if shrunk {
                if let Some(object) = payload.as_object_mut() {
                    object.insert("truncated".into(), Value::Bool(true));
                }
                continue;
            }
            // Nothing left to shrink safely (should not happen): refuse to
            // emit an oversized payload instead of breaking the JSON.
            return Self {
                outcome: ToolOutcome::Error,
                payload: json!({"error":"结果过长"}).to_string(),
            };
        }
    }
}

/// Execute one model-requested tool call. `roots` must be canonicalized.
pub fn execute_tool(roots: &[PathBuf], name: &str, arguments: &str) -> ToolResult {
    let args: Value = match serde_json::from_str::<Value>(arguments) {
        Ok(value) if value.is_object() => value,
        _ => {
            return ToolResult::error("参数不是合法的 JSON 对象");
        }
    };
    match name {
        "list_roots" => list_roots(roots),
        "search_names" => {
            let Some(pattern) = string_arg(&args, "pattern") else {
                return ToolResult::error("缺少 pattern 参数");
            };
            search_names(roots, &pattern)
        }
        "search_contents" => {
            let Some(pattern) = string_arg(&args, "pattern") else {
                return ToolResult::error("缺少 pattern 参数");
            };
            search_contents(roots, &pattern)
        }
        "read_slice" => {
            let Some(path) = string_arg(&args, "path") else {
                return ToolResult::error("缺少 path 参数");
            };
            let offset = match args.get("offset") {
                None | Some(Value::Null) => 0usize,
                Some(Value::Number(number)) if number.is_u64() => number.as_u64().unwrap() as usize,
                _ => return ToolResult::error("offset 需是非负整数"),
            };
            read_slice(roots, &path, offset)
        }
        _ => ToolResult {
            outcome: ToolOutcome::UnknownTool,
            payload: json!({"error":"未知工具；可用：list_roots / search_names / search_contents / read_slice"}).to_string(),
        },
    }
}

fn string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)?.as_str().map(str::trim).map(str::to_string)
}

fn list_roots(roots: &[PathBuf]) -> ToolResult {
    ToolResult::ok(json!({
        "roots": roots.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
        "notice": "只读检索；授权范围外的路径会被拒绝"
    }))
}

fn valid_pattern(pattern: &str) -> Option<String> {
    let trimmed = pattern.trim();
    (1..=200)
        .contains(&trimmed.chars().count())
        .then(|| trimmed.to_lowercase())
}

fn search_names(roots: &[PathBuf], pattern: &str) -> ToolResult {
    let Some(needle) = valid_pattern(pattern) else {
        return ToolResult::error("pattern 需在1~200字符之间");
    };
    let mut matches: Vec<String> = Vec::new();
    let mut truncated = false;
    for root in roots {
        walk(root, |path, _metadata, budget_hit| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if name.contains(&needle) {
                if matches.len() < NAMES_LIMIT {
                    matches.push(path.to_string_lossy().into_owned());
                } else {
                    truncated = true;
                }
            }
            if matches.len() >= NAMES_LIMIT || budget_hit {
                Walk::Stop
            } else {
                Walk::Continue
            }
        });
        if matches.len() >= NAMES_LIMIT {
            truncated = true;
            break;
        }
    }
    ToolResult::ok(json!({"matches": matches, "truncated": truncated}))
}

fn search_contents(roots: &[PathBuf], pattern: &str) -> ToolResult {
    let Some(needle) = valid_pattern(pattern) else {
        return ToolResult::error("pattern 需在1~200字符之间");
    };
    let mut matches: Vec<String> = Vec::new();
    let mut truncated = false;
    let mut scanned: u64 = 0;
    for root in roots {
        walk(root, |path, metadata, budget_hit| {
            if budget_hit || matches.len() >= LINES_LIMIT {
                truncated = true;
                return Walk::Stop;
            }
            if !metadata.is_file()
                || metadata.len() > CONTENTS_MAX_FILE
                || !is_text_file(path)
                || scanned >= SCAN_BYTES
            {
                return Walk::Continue;
            }
            scanned += metadata.len();
            if let Ok(file) = std::fs::File::open(path) {
                for (index, line) in BufReader::new(file).lines().enumerate() {
                    let Ok(line) = line else { break };
                    if line.chars().count() > LINE_MATCH_CHARS {
                        continue; // minified monsters do not eat the budget
                    }
                    if line.to_lowercase().contains(&needle) {
                        if matches.len() < LINES_LIMIT {
                            let shown: String = line.chars().take(LINE_SHOW_CHARS).collect();
                            matches.push(format!(
                                "{}:{}:{}",
                                path.to_string_lossy(),
                                index + 1,
                                shown
                            ));
                        } else {
                            truncated = true;
                            break;
                        }
                    }
                }
            }
            if matches.len() >= LINES_LIMIT {
                truncated = true;
                Walk::Stop
            } else {
                Walk::Continue
            }
        });
        if matches.len() >= LINES_LIMIT {
            break;
        }
    }
    ToolResult::ok(json!({"matches": matches, "truncated": truncated}))
}

fn read_slice(roots: &[PathBuf], input: &str, offset: usize) -> ToolResult {
    let resolved = match resolve_in_roots(roots, input) {
        Ok(path) => path,
        Err(()) => return ToolResult::denied("路径不在授权目录内或不可访问"),
    };
    let metadata = match std::fs::symlink_metadata(&resolved) {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return ToolResult::denied("不是常规文件"),
    };
    if is_link(&metadata) {
        return ToolResult::denied("拒绝链接文件");
    }
    if !is_text_file(&resolved) {
        return ToolResult::error("仅支持文本扩展名");
    }
    if metadata.len() > SLICE_MAX_FILE {
        return ToolResult::error("文件超过 2MB 上限");
    }
    let content = match std::fs::read_to_string(&resolved) {
        Ok(content) => content,
        Err(_) => return ToolResult::error("读取失败（内容可能不是 UTF-8）"),
    };
    let total = content.chars().count();
    if offset > total {
        return ToolResult::error("offset 超出文件长度");
    }
    let slice: String = content.chars().skip(offset).take(SLICE_CHARS).collect();
    let next = offset + slice.chars().count();
    ToolResult::ok(json!({
        "path": resolved.to_string_lossy(),
        "content": slice,
        "offset": offset,
        "nextOffset": if next < total { Value::from(next) } else { Value::Null },
        "totalChars": total,
        "truncated": next < total
    }))
}

/// Model-supplied paths are untrusted: reject empties/oversize/NUL, resolve
/// through the filesystem (junctions and symlinks land on their real
/// target), then demand containment inside a canonical root.
pub(crate) fn resolve_in_roots(roots: &[PathBuf], input: &str) -> Result<PathBuf, ()> {
    if input.is_empty() || input.len() > 4096 || input.contains('\0') {
        return Err(());
    }
    let canonical = std::fs::canonicalize(Path::new(input)).map_err(|_| ())?;
    if roots.iter().any(|root| canonical.starts_with(root)) {
        Ok(canonical)
    } else {
        Err(())
    }
}

fn is_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| TEXT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[derive(PartialEq)]
enum Walk {
    Continue,
    Stop,
}

/// Deterministic depth-first walk with first-class budgets (node count,
/// depth, wall clock). Reparse points (junctions/symlinks) are never
/// entered; the visitor sees `budget_hit` so it can flag truncation.
fn walk(root: &Path, mut visit: impl FnMut(&Path, &Metadata, bool) -> Walk) {
    let started = Instant::now();
    let mut nodes = 0usize;
    fn descend(
        dir: &Path,
        depth: usize,
        nodes: &mut usize,
        started: &Instant,
        visit: &mut dyn FnMut(&Path, &Metadata, bool) -> Walk,
    ) -> Walk {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Walk::Continue;
        };
        let mut names: Vec<(String, PathBuf)> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                )
            })
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, path) in names {
            *nodes += 1;
            let budget_hit =
                *nodes >= WALK_NODES || depth >= WALK_DEPTH || started.elapsed() >= WALK_TIME;
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if is_link(&metadata) {
                continue; // links are reported never entered
            }
            if visit(&path, &metadata, budget_hit) == Walk::Stop {
                return Walk::Stop;
            }
            if budget_hit {
                return Walk::Stop;
            }
            if metadata.is_dir()
                && depth + 1 < WALK_DEPTH
                && descend(&path, depth + 1, nodes, started, visit) == Walk::Stop
            {
                return Walk::Stop;
            }
        }
        Walk::Continue
    }
    descend(root, 0, &mut nodes, &started, &mut visit);
}

// ===== Tauri commands =====

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum LocalRootsAddResult {
    Cancelled,
    Added { root: AuthorizedRoot },
    Invalid { reason: String },
}

#[tauri::command]
pub fn local_roots_get(
    state: tauri::State<'_, crate::chat::ChatState>,
) -> Result<Vec<AuthorizedRoot>, String> {
    let inner = state.inner().0.lock().map_err(|_| "会话状态不可用")?;
    let store = inner.store.as_ref().ok_or("本地存储不可用")?;
    store
        .authorized_roots()
        .map_err(|_| "本地存储不可用".to_string())
}

#[tauri::command]
pub async fn local_roots_add(
    app: tauri::AppHandle,
    window: tauri::Window,
    path: Option<String>,
) -> Result<LocalRootsAddResult, String> {
    #[cfg(windows)]
    let parent = window.hwnd().map_err(|_| "窗口不可用")?.0 as isize;
    #[cfg(not(windows))]
    let parent = 0;
    let picked = match path {
        Some(manual) => Some(std::path::PathBuf::from(manual)),
        None => tauri::async_runtime::spawn_blocking(move || choose_folder(parent))
            .await
            .map_err(|_| "目录选择失败")?
            .map_err(|_| "目录选择失败")?,
    };
    let Some(raw) = picked else {
        return Ok(LocalRootsAddResult::Cancelled);
    };
    let canonical = match std::fs::canonicalize(&raw) {
        Ok(path) if path.is_dir() => path,
        _ => {
            return Ok(LocalRootsAddResult::Invalid {
                reason: "目录不存在或不可访问".into(),
            })
        }
    };
    let private_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|_| "应用目录不可用")?;
    let private_canonical = std::fs::canonicalize(&private_dir).unwrap_or(private_dir);
    if canonical.starts_with(&private_canonical) {
        return Ok(LocalRootsAddResult::Invalid {
            reason: "应用自身的数据目录不能加入检索范围".into(),
        });
    }
    let state = app.state::<crate::chat::ChatState>();
    let mut inner = state.inner().0.lock().map_err(|_| "会话状态不可用")?;
    let store = inner.store.as_mut().ok_or("本地存储不可用")?;
    let existing = store
        .authorized_roots()
        .map_err(|_| "本地存储不可用".to_string())?;
    if existing.len() >= MAX_ROOTS {
        return Ok(LocalRootsAddResult::Invalid {
            reason: format!("最多 {MAX_ROOTS} 个目录"),
        });
    }
    for root in &existing {
        let other = PathBuf::from(&root.path);
        if canonical.starts_with(&other) || other.starts_with(&canonical) {
            return Ok(LocalRootsAddResult::Invalid {
                reason: "与已有授权目录重叠".into(),
            });
        }
    }
    let path_string = canonical.to_string_lossy().into_owned();
    let root = store
        .authorized_root_add(&AuthorizedRootInsert {
            path: &path_string,
            label: None,
        })
        .map_err(|_| "保存失败")?;
    Ok(LocalRootsAddResult::Added { root })
}

#[tauri::command]
pub fn local_roots_remove(
    state: tauri::State<'_, crate::chat::ChatState>,
    id: i64,
) -> Result<Vec<AuthorizedRoot>, String> {
    let mut inner = state.inner().0.lock().map_err(|_| "会话状态不可用")?;
    let store = inner.store.as_mut().ok_or("本地存储不可用")?;
    store
        .authorized_root_remove(id)
        .map_err(|_| "本地存储不可用".to_string())?;
    store
        .authorized_roots()
        .map_err(|_| "本地存储不可用".to_string())
}

/// Native folder picker on a dedicated STA thread (memory_export.rs pattern,
/// IFileOpenDialog + FOS_PICKFOLDERS instead of the save dialog).
#[cfg(windows)]
fn choose_folder(parent: isize) -> Result<Option<PathBuf>, ()> {
    use windows::{
        core::{w, HRESULT},
        Win32::{
            Foundation::{ERROR_CANCELLED, HWND},
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{
                FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR,
                FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
            },
        },
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    let run = || -> windows::core::Result<Option<PathBuf>> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            let _apartment = Apartment;
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetTitle(w!("选择允许栖伴检索的目录"))?;
            dialog.SetOptions(
                FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR,
            )?;
            if let Err(error) = dialog.Show(Some(HWND(parent as *mut _))) {
                if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                    return Ok(None);
                }
                return Err(error);
            }
            let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let value = name.to_string();
            CoTaskMemFree(Some(name.0.cast()));
            Ok(Some(PathBuf::from(value?)))
        }
    };
    run().map_err(|_| ())
}

#[cfg(not(windows))]
fn choose_folder(_parent: isize) -> Result<Option<PathBuf>, ()> {
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("qiban-local-search-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("projects/quant")).unwrap();
        fs::create_dir_all(root.join("projects/notes")).unwrap();
        fs::write(
            root.join("projects/quant/main.rs"),
            "fn main() {\n    let alpha = 1;\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("projects/notes/todo.md"),
            "# 待办\n- 复核 Alpha 因子\n- 买菜\n",
        )
        .unwrap();
        fs::write(root.join("projects/binary.bin"), [0u8; 8]).unwrap();
        root
    }

    fn canonical(root: &Path) -> PathBuf {
        fs::canonicalize(root).unwrap()
    }

    #[test]
    fn specs_cover_the_four_tools_with_required_params() {
        let specs = tool_specs();
        let names: Vec<_> = specs.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "list_roots",
                "search_names",
                "search_contents",
                "read_slice"
            ]
        );
        for spec in &specs {
            assert!(spec.description.chars().count() > 5);
            assert_eq!(spec.parameters["type"], "object");
        }
    }

    #[test]
    fn list_roots_reports_canonical_paths() {
        let root = canonical(&tree("list"));
        let result = execute_tool(std::slice::from_ref(&root), "list_roots", "{}");
        assert_eq!(result.outcome, ToolOutcome::Ok);
        let value: Value = serde_json::from_str(&result.payload).unwrap();
        assert_eq!(value["roots"][0].as_str().unwrap(), root.to_string_lossy());
    }

    #[test]
    fn search_names_is_case_insensitive_and_bounded() {
        let root = canonical(&tree("names"));
        let result = execute_tool(&[root], "search_names", r#"{"pattern":"QUANT"}"#);
        assert_eq!(result.outcome, ToolOutcome::Ok);
        let value: Value = serde_json::from_str(&result.payload).unwrap();
        let matches = value["matches"].as_array().unwrap();
        assert!(!matches.is_empty());
        assert!(matches
            .iter()
            .all(|m| m.as_str().unwrap().to_lowercase().contains("quant")));
        // Empty or oversize patterns are refused, not guessed at.
        for bad in ["", "  ", "x".repeat(201).as_str()] {
            let arguments = serde_json::to_string(&json!({ "pattern": bad })).unwrap();
            let result = execute_tool(&[], "search_names", &arguments);
            assert_eq!(result.outcome, ToolOutcome::Error, "{bad:?}");
        }
    }

    #[test]
    fn search_contents_finds_lines_and_skips_binaries() {
        let root = canonical(&tree("contents"));
        let result = execute_tool(
            std::slice::from_ref(&root),
            "search_contents",
            r#"{"pattern":"alpha"}"#,
        );
        assert_eq!(result.outcome, ToolOutcome::Ok);
        let value: Value = serde_json::from_str(&result.payload).unwrap();
        let text = value.to_string();
        assert!(text.contains("main.rs"), "hit inside the rust file");
        // .bin files are never opened, so a pattern that only exists there finds nothing.
        let none = execute_tool(&[root], "search_contents", r#"{"pattern":"not-there"}"#);
        let value: Value = serde_json::from_str(&none.payload).unwrap();
        assert_eq!(value["matches"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn read_slice_pages_by_chars_and_reports_bounds() {
        let root = canonical(&tree("slice"));
        let main = root.join("projects/quant/main.rs");
        let first = execute_tool(
            std::slice::from_ref(&root),
            "read_slice",
            &serde_json::to_string(&json!({"path": main.to_string_lossy()})).unwrap(),
        );
        assert_eq!(first.outcome, ToolOutcome::Ok);
        let page: Value = serde_json::from_str(&first.payload).unwrap();
        let total = page["totalChars"].as_u64().unwrap() as usize;
        let content_len = page["content"].as_str().unwrap().chars().count();
        assert!(content_len <= SLICE_CHARS);
        assert_eq!(page["truncated"], Value::Bool(total > SLICE_CHARS));
        if total > SLICE_CHARS {
            let next = page["nextOffset"].as_u64().unwrap() as usize;
            assert_eq!(next, SLICE_CHARS);
            let second = execute_tool(
                &[root],
                "read_slice",
                &serde_json::to_string(&json!({"path": main.to_string_lossy(), "offset": next}))
                    .unwrap(),
            );
            let page2: Value = serde_json::from_str(&second.payload).unwrap();
            assert!(page2["content"].as_str().unwrap().chars().count() > 0);
        }
    }

    #[test]
    fn escape_attempts_are_denied_and_auditable() {
        let root = canonical(&tree("escape"));
        for path in [
            "C:\\Windows\\win.ini".to_string(),
            format!("{}\\..\\..\\Windows\\win.ini", root.to_string_lossy()),
            "x".repeat(4097),
            String::new(),
        ] {
            let arguments = serde_json::to_string(&json!({ "path": path, "offset": 0 })).unwrap();
            let result = execute_tool(std::slice::from_ref(&root), "read_slice", &arguments);
            assert_eq!(result.outcome, ToolOutcome::Denied, "{path:.40}");
            let value: Value = serde_json::from_str(&result.payload).unwrap();
            assert!(value["error"].is_string());
        }
        // An escaped NUL inside the JSON string is unreadable, not a crash.
        let result = execute_tool(&[root], "read_slice", r#"{"path":"a\u0000b"}"#);
        assert_eq!(result.outcome, ToolOutcome::Denied);
    }

    #[test]
    fn non_text_files_are_refused() {
        let root = canonical(&tree("gates"));
        let bin = root.join("projects/binary.bin");
        let result = execute_tool(
            &[root],
            "read_slice",
            &serde_json::to_string(&json!({"path": bin.to_string_lossy()})).unwrap(),
        );
        assert_eq!(result.outcome, ToolOutcome::Error);
        let value: Value = serde_json::from_str(&result.payload).unwrap();
        assert_eq!(value["error"].as_str().unwrap(), "仅支持文本扩展名");
    }

    #[test]
    fn unknown_tool_and_bad_arguments_stay_replayable() {
        let root = canonical(&tree("meta"));
        let unknown = execute_tool(std::slice::from_ref(&root), "delete_everything", "{}");
        assert_eq!(unknown.outcome, ToolOutcome::UnknownTool);
        serde_json::from_str::<Value>(&unknown.payload).unwrap();
        for bad in ["not json", "[]", r#"{"pattern":123}"#] {
            let result = execute_tool(std::slice::from_ref(&root), "search_names", bad);
            assert_eq!(result.outcome, ToolOutcome::Error, "{bad}");
            serde_json::from_str::<Value>(&result.payload).unwrap();
        }
    }

    #[test]
    fn payloads_stay_under_the_cap_as_valid_json() {
        // Drive fit() directly for the list path (an OS-valid filename cannot
        // be long enough to overflow on its own; many long entries do).
        let wide_items: Vec<String> = std::iter::repeat_n("很".repeat(3000), 10).collect();
        let wide = json!({"matches": wide_items, "truncated": false});
        let fitted = ToolResult::fit(ToolOutcome::Ok, wide);
        assert!(
            fitted.payload.len() <= PAYLOAD_CAP,
            "{}",
            fitted.payload.len()
        );
        let value: Value = serde_json::from_str(&fitted.payload).unwrap();
        assert_eq!(value["truncated"], Value::Bool(true));
        assert!(value["matches"].as_array().unwrap().len() < 10);

        // Content path: a real file whose slice overflows the cap gets
        // truncated at a char boundary and stays valid JSON.
        let root =
            std::env::temp_dir().join(format!("qiban-local-search-cap-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let long = "很".repeat(9_000);
        fs::write(root.join("deep.md"), &long).unwrap();
        let canon = canonical(&root);
        let arguments =
            serde_json::to_string(&json!({"path": root.join("deep.md").to_string_lossy()}))
                .unwrap();
        let slice = execute_tool(&[canon], "read_slice", &arguments);
        assert!(
            slice.payload.len() <= PAYLOAD_CAP,
            "{}",
            slice.payload.len()
        );
        let page: Value = serde_json::from_str(&slice.payload).unwrap();
        assert_eq!(page["truncated"], Value::Bool(true));
        assert!(page["content"].as_str().unwrap().chars().count() <= SLICE_CHARS);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_rejects_nul_and_empty_and_accepts_root() {
        let root = canonical(&tree("resolve"));
        assert!(resolve_in_roots(std::slice::from_ref(&root), "").is_err());
        assert!(resolve_in_roots(std::slice::from_ref(&root), "a\0b").is_err());
        let self_path = root.to_string_lossy().into_owned();
        assert!(resolve_in_roots(&[root], &self_path).is_ok());
    }
}
