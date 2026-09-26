//! MCP stdio transport: one JSON-RPC frame per line, the same six tools the
//! Python server exposed. Tool names and descriptions are kept verbatim so
//! existing CLAUDE.md instructions keep working.

use crate::store::{MemoryKind, MemoryStore, NewMemory, RecallFilter};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

/// Descriptors copied verbatim from the Python server (names, descriptions
/// and input schemas; the only edits are formatting).
const TOOLS_JSON: &str = r#"[
  {"name": "memory_remember", "description":
   "存储一条记忆（事实/决策/偏好/项目/人物/洞察）。当用户分享了重要信息时调用。",
   "inputSchema": {"type": "object", "properties": {
       "type": {"type": "string", "enum": ["fact","decision","preference","project","person","insight","context"],
                "description": "记忆类型"},
       "title": {"type": "string", "description": "简短标签"},
       "content": {"type": "string", "description": "完整内容"},
       "project": {"type": "string", "description": "所属项目（可选，全局记忆不填）"},
       "importance": {"type": "integer", "description": "1-5（5=极重要，默认3）"},
       "tags": {"type": "string", "description": "逗号分隔标签"}
   }, "required": ["title", "content"]}},
  {"name": "memory_recall", "description":
   "检索活跃记忆。会话开始时或需要回顾历史时调用。支持按项目/类型/关键词过滤。",
   "inputSchema": {"type": "object", "properties": {
       "query": {"type": "string", "description": "搜索关键词"},
       "project": {"type": "string", "description": "按项目过滤"},
       "type": {"type": "string", "description": "按类型过滤"},
       "limit": {"type": "integer", "description": "最多返回条数（默认20）"}
   }}},
  {"name": "memory_supersede", "description":
   "用新记忆取代旧记忆（旧记忆保留但不再活跃）。当信息更新时调用。",
   "inputSchema": {"type": "object", "properties": {
       "old_id": {"type": "integer", "description": "要取代的记忆 ID"},
       "title": {"type": "string"}, "content": {"type": "string"}
   }, "required": ["old_id", "title", "content"]}},
  {"name": "memory_forget", "description":
   "标记记忆过期（不再检索到，但保留历史）。",
   "inputSchema": {"type": "object", "properties": {
       "id": {"type": "integer"}},
   "required": ["id"]}},
  {"name": "memory_stats", "description":
   "查看记忆库统计概览。",
   "inputSchema": {"type": "object", "properties": {}}},
  {"name": "memory_update", "description":
   "直接更新记忆内容（不取代）。",
   "inputSchema": {"type": "object", "properties": {
       "id": {"type": "integer"}, "content": {"type": "string"}
   }, "required": ["id", "content"]}}
]"#;

/// Runs the stdio loop until EOF. Stdout carries protocol frames only; every
/// other diagnostic goes to stderr or nowhere.
pub fn run(store: MemoryStore) {
    let stdin = io::stdin();
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(response) = handle_line(&store, line) {
            // handle_line already returns one serialized JSON object; write
            // it verbatim or it would be double-encoded into a JSON string.
            if output.write_all(response.as_bytes()).is_ok() {
                let _ = writeln!(output);
            }
            let _ = output.flush();
        }
    }
}

/// Handles one input line and returns the serialized response, if any.
/// Messages without an id are notifications and never produce output.
pub fn handle_line(store: &MemoryStore, line: &str) -> Option<String> {
    let request: Value = serde_json::from_str(line).ok()?;
    let id = request.get("id").cloned()?;
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(store, method, &request)
    }));
    let payload = match outcome {
        Ok(Outcome::Rpc(result)) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Ok(Outcome::ToolText(text)) => json!({
            "jsonrpc": "2.0", "id": id,
            "result": {"content": [{"type": "text", "text": text}]}
        }),
        Ok(Outcome::ToolError(message)) => json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "content": [{"type": "text", "text": tool_error_text(&message)}],
                "isError": true
            }
        }),
        Ok(Outcome::RpcError(code, message)) => json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": code, "message": message}
        }),
        // A panicking handler must not take the transport down with it.
        Err(_) => json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": -32603, "message": "internal error"}
        }),
    };
    Some(payload.to_string())
}

fn tool_error_text(message: &str) -> String {
    serde_json::to_string(&json!({"error": message})).unwrap_or_else(|_| "{}".to_string())
}

enum Outcome {
    Rpc(Value),
    ToolText(String),
    ToolError(String),
    RpcError(i64, String),
}

fn dispatch(store: &MemoryStore, method: &str, request: &Value) -> Outcome {
    match method {
        "initialize" => Outcome::Rpc(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "personal-memory", "version": env!("CARGO_PKG_VERSION")}
        })),
        "tools/list" => Outcome::Rpc(json!({
            "tools": serde_json::from_str::<Value>(TOOLS_JSON)
                .expect("embedded tool descriptors are valid JSON")
        })),
        "tools/call" => {
            let name = request
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = request
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call_tool(store, name, &arguments) {
                Ok(payload) => Outcome::ToolText(
                    serde_json::to_string(&payload).expect("tool payload serializes"),
                ),
                Err(message) => Outcome::ToolError(message),
            }
        }
        other => Outcome::RpcError(-32601, format!("method not found: {other}")),
    }
}

fn call_tool(store: &MemoryStore, name: &str, arguments: &Value) -> Result<Value, String> {
    match name {
        "memory_remember" => call_remember(store, arguments),
        "memory_recall" => call_recall(store, arguments),
        "memory_supersede" => call_supersede(store, arguments),
        "memory_forget" => call_forget(store, arguments),
        "memory_stats" => call_stats(store),
        "memory_update" => call_update(store, arguments),
        _ => Err(format!("unknown tool: {name}")),
    }
}

fn call_remember(store: &MemoryStore, arguments: &Value) -> Result<Value, String> {
    let args = parse_args::<RememberArgs>(arguments)?;
    let kind = MemoryKind::parse(&args.r#type).ok_or("unknown memory type")?;
    let draft = NewMemory {
        kind,
        title: args.title.clone(),
        content: args.content.clone(),
        project: args.project.clone(),
        importance: args.importance.unwrap_or(3),
        tags: args
            .tags
            .as_deref()
            .map(|tags| tags.split(',').map(|t| t.to_string()).collect())
            .unwrap_or_default(),
    };
    let id = store.remember(&draft).map_err(|error| error.to_string())?;
    Ok(json!({"stored": true, "id": id, "title": args.title}))
}

fn call_recall(store: &MemoryStore, arguments: &Value) -> Result<Value, String> {
    let args = parse_args::<RecallArgs>(arguments)?;
    let kind = match args.r#type.as_deref() {
        None => None,
        Some(value) => Some(MemoryKind::parse(value).ok_or("unknown memory type")?),
    };
    let filter = RecallFilter {
        query: args.query,
        project: args.project,
        kind,
        limit: args.limit,
    };
    let records = store.recall(&filter).map_err(|error| error.to_string())?;
    Ok(json!({
        "count": records.len(),
        "memories": records.iter().map(recall_item).collect::<Vec<_>>()
    }))
}

fn recall_item(record: &crate::store::MemoryRecord) -> Value {
    json!({
        "id": record.id,
        "type": record.kind.as_str(),
        "project": record.project,
        "title": record.title,
        "content": record.content,
        "importance": record.importance,
        "created_at": record.created_at,
        "tags": record.tags,
    })
}

fn call_supersede(store: &MemoryStore, arguments: &Value) -> Result<Value, String> {
    let args = parse_args::<SupersedeArgs>(arguments)?;
    let (old_id, new_id) = store
        .supersede(args.old_id, &args.title, &args.content)
        .map_err(|error| error.to_string())?;
    let old_title = store
        .get(old_id)
        .ok()
        .flatten()
        .map(|record| record.title)
        .unwrap_or_default();
    Ok(json!({
        "superseded": old_id, "new_id": new_id,
        "old_title": old_title, "new_title": args.title
    }))
}

fn call_forget(store: &MemoryStore, arguments: &Value) -> Result<Value, String> {
    let args = parse_args::<ForgetBody>(arguments)?;
    store.forget(args.id).map_err(|error| error.to_string())?;
    Ok(json!({"forgotten": args.id}))
}

fn call_stats(store: &MemoryStore) -> Result<Value, String> {
    let stats = store.stats().map_err(|error| error.to_string())?;
    Ok(json!({
        "total": stats.total,
        "active": stats.active,
        "superseded": stats.superseded,
        "by_type": stats.by_type
            .into_iter()
            .map(|(k, v)| (k, json!(v)))
            .collect::<serde_json::Map<String, Value>>(),
        "by_project": stats.by_project
            .into_iter()
            .map(|(k, v)| (k, json!(v)))
            .collect::<serde_json::Map<String, Value>>(),
    }))
}

fn call_update(store: &MemoryStore, arguments: &Value) -> Result<Value, String> {
    let args = parse_args::<ForgetBody>(arguments)?;
    store
        .update(args.id, &args.content)
        .map_err(|error| error.to_string())?;
    Ok(json!({"updated": args.id}))
}

fn parse_args<T: serde::de::DeserializeOwned>(arguments: &Value) -> Result<T, String> {
    serde_json::from_value(arguments.clone()).map_err(|error| error.to_string())
}

#[derive(Deserialize)]
struct RememberArgs {
    #[serde(default = "default_kind")]
    r#type: String,
    title: String,
    content: String,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    importance: Option<i64>,
    #[serde(default)]
    tags: Option<String>,
}

fn default_kind() -> String {
    "fact".to_string()
}

#[derive(Deserialize)]
struct RecallArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct SupersedeArgs {
    old_id: i64,
    title: String,
    content: String,
}

#[derive(Deserialize)]
struct ForgetBody {
    id: i64,
    #[serde(default)]
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> MemoryStore {
        crate::store::test_support::memory_store("mcp")
    }

    fn call(store: &MemoryStore, name: &str, arguments: Value) -> Value {
        let line = json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        })
        .to_string();
        let response: Value =
            serde_json::from_str(&handle_line(store, &line).expect("tool call responds")).unwrap();
        assert!(
            response.get("error").is_none(),
            "unexpected rpc error: {response}"
        );
        let result = response["result"].clone();
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("missing text payload for {name}: {response}"));
        serde_json::from_str(text)
            .unwrap_or_else(|error| panic!("bad payload for {name}: {error}: {text}"))
    }

    #[test]
    fn notifications_never_produce_output() {
        let store = store();
        assert_eq!(
            handle_line(&store, r#"{"method":"notifications/initialized"}"#),
            None
        );
        assert_eq!(
            handle_line(
                &store,
                r#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#
            ),
            None
        );
    }

    #[test]
    fn malformed_lines_are_skipped_silently() {
        let store = store();
        assert_eq!(handle_line(&store, "not json"), None);
        assert_eq!(handle_line(&store, ""), None);
    }

    #[test]
    fn unknown_method_with_id_returns_32601() {
        let store = store();
        let response: Value = serde_json::from_str(
            &handle_line(&store, r#"{"jsonrpc":"2.0","id":1,"method":"wat"}"#).unwrap(),
        )
        .unwrap();
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn initialize_and_tools_list_match_the_python_contract() {
        let store = store();
        let init: Value = serde_json::from_str(
            &handle_line(&store, r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#).unwrap(),
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(init["result"]["serverInfo"]["name"], "personal-memory");
        let listed: Value = serde_json::from_str(
            &handle_line(&store, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap(),
        )
        .unwrap();
        let names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                "memory_remember",
                "memory_recall",
                "memory_supersede",
                "memory_forget",
                "memory_stats",
                "memory_update"
            ]
        );
    }

    #[test]
    fn six_tools_round_trip_with_python_field_names() {
        let store = store();
        let stored = call(
            &store,
            "memory_remember",
            json!({"type": "decision", "title": "论文投 JAAMAS", "content": "决定投 JAAMAS。", "project": "R2R", "importance": 5, "tags": "论文,投稿"}),
        );
        assert_eq!(stored["stored"], true);
        let id = stored["id"].as_i64().unwrap();

        let recalled = call(&store, "memory_recall", json!({"query": "JAAMAS"}));
        assert_eq!(recalled["count"], 1);
        let item = &recalled["memories"][0];
        assert_eq!(item["id"], id);
        assert_eq!(item["type"], "decision");
        assert_eq!(item["tags"], json!(["论文", "投稿"]));

        let superseded = call(
            &store,
            "memory_supersede",
            json!({"old_id": id, "title": "论文改投 COINE", "content": "改投决定。"}),
        );
        assert_eq!(superseded["superseded"], id);
        assert_eq!(superseded["old_title"], "论文投 JAAMAS");
        let new_id = superseded["new_id"].as_i64().unwrap();
        assert_ne!(new_id, id);

        let forgotten = call(&store, "memory_forget", json!({"id": new_id}));
        assert_eq!(forgotten["forgotten"], new_id);

        let stats = call(&store, "memory_stats", json!({}));
        assert_eq!(stats["total"], 2);
        assert_eq!(stats["active"], 0);
        assert_eq!(stats["superseded"], 1);

        // Superseded and forgotten rows are not recalled.
        let empty = call(&store, "memory_recall", json!({}));
        assert_eq!(empty["count"], 0);

        let updated = call(
            &store,
            "memory_update",
            json!({"id": id, "content": "更新后的内容。"}),
        );
        assert_eq!(updated["updated"], id);
    }

    #[test]
    fn update_of_unknown_id_reports_failure_instead_of_fake_success() {
        let store = store();
        let payload = call(&store, "memory_update", json!({"id": 99, "content": "x"}));
        assert!(
            payload.get("error").is_some(),
            "expected error payload: {payload}"
        );
    }

    #[test]
    fn missing_required_argument_is_a_tool_error() {
        let store = store();
        let payload = call(&store, "memory_remember", json!({"content": "no title"}));
        assert!(payload.get("error").is_some());
    }
}
