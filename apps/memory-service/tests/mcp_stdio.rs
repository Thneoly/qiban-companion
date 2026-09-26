//! Spawns the real binary in `mcp` mode and drives the stdio protocol,
//! including a cross-process WAL visibility check.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};

struct McpChild {
    child: Child,
    stdout: BufReader<ChildStdout>,
}

impl McpChild {
    fn start(database: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_memory-service"))
            .arg("mcp")
            .env("QIBAN_MEMORY_DB", database)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn memory-service mcp");
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
        Self { child, stdout }
    }

    fn send(&mut self, request: &Value) {
        let stdin = self.child.stdin.as_mut().expect("stdin open");
        stdin
            .write_all(format!("{request}\n").as_bytes())
            .expect("write request");
        stdin.flush().expect("flush request");
    }

    /// Reads the next response line; every stdout byte must be one complete
    /// JSON object per line, nothing else.
    fn receive(&mut self) -> Value {
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("read response line");
        let trimmed = line.trim();
        assert!(!trimmed.is_empty(), "expected a response line, got EOF");
        let value: Value = serde_json::from_str(trimmed)
            .unwrap_or_else(|error| panic!("stdout must be pure JSON lines: {error}: {trimmed}"));
        value
    }

    fn request(&mut self, id: i64, method: &str, params: Option<Value>) -> Value {
        let mut request = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if let Some(params) = params {
            request["params"] = params;
        }
        self.send(&request);
        self.receive()
    }

    fn call_tool(&mut self, id: i64, name: &str, arguments: Value) -> Value {
        let response = self.request(
            id,
            "tools/call",
            Some(json!({"name": name, "arguments": arguments})),
        );
        assert!(response.get("error").is_none(), "rpc error: {response}");
        let result = &response["result"];
        assert!(
            !result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "tool error: {result}"
        );
        let text = result["content"][0]["text"].as_str().expect("text payload");
        serde_json::from_str(text).expect("tool payload is JSON")
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn temp_db(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("{name}-{}.db", uuid::Uuid::new_v4()));
    path
}

#[test]
fn stdio_round_trip_matches_the_python_contract() {
    let database = temp_db("mcp-stdio");
    let mut child = McpChild::start(&database);

    let init = child.request(1, "initialize", None);
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(init["result"]["serverInfo"]["name"], "personal-memory");

    // A notification must produce no output at all: the next response we get
    // belongs to the request that follows it.
    child.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let listed = child.request(2, "tools/list", None);
    assert_eq!(listed["id"], 2);
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

    let stored = child.call_tool(
        3,
        "memory_remember",
        json!({"type": "project", "title": "Memory Service 改造", "content": "Rust 双协议重写。", "project": "Game", "importance": 5, "tags": "memory,rust"}),
    );
    assert_eq!(stored["stored"], true);
    let id = stored["id"].as_i64().unwrap();

    let recalled = child.call_tool(4, "memory_recall", json!({"query": "双协议"}));
    assert_eq!(recalled["count"], 1);
    assert_eq!(recalled["memories"][0]["id"], id);
    assert_eq!(recalled["memories"][0]["tags"], json!(["memory", "rust"]));

    let superseded = child.call_tool(
        5,
        "memory_supersede",
        json!({"old_id": id, "title": "改造完成", "content": "已完成。"}),
    );
    assert_eq!(superseded["superseded"], id);
    let new_id = superseded["new_id"].as_i64().unwrap();

    let forgotten = child.call_tool(6, "memory_forget", json!({"id": new_id}));
    assert_eq!(forgotten["forgotten"], new_id);

    let stats = child.call_tool(7, "memory_stats", json!({}));
    assert_eq!(stats["total"], 2);
    assert_eq!(stats["active"], 0);

    let updated = child.call_tool(
        8,
        "memory_update",
        json!({"id": id, "content": "修订内容。"}),
    );
    assert_eq!(updated["updated"], id);

    // Unknown method with an id yields the JSON-RPC error envelope.
    let unknown = child.request(9, "bogus/method", None);
    assert_eq!(unknown["error"]["code"], -32601);

    // Malformed input lines are skipped silently: the next valid request
    // still gets the next response.
    {
        let stdin = child.child.stdin.as_mut().unwrap();
        stdin.write_all(b"this is not json\n").unwrap();
        stdin.flush().unwrap();
    }
    let after_garbage = child.request(10, "tools/list", None);
    assert_eq!(after_garbage["id"], 10);

    drop(child);
    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(database.with_extension("db.v1.bak"));
}

#[test]
fn external_writes_are_visible_to_the_running_child() {
    let database = temp_db("mcp-wal");
    let mut child = McpChild::start(&database);
    child.request(1, "initialize", None);

    let stats = child.call_tool(2, "memory_stats", json!({}));
    assert_eq!(stats["total"], 0);

    // A second writer (the HTTP daemon's role) commits while the MCP child
    // stays alive; WAL mode must make it visible without restart.
    {
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute(
                "INSERT INTO memories(seq,type,project,title,content,importance,created_at,updated_at,tags,origin) \
                 VALUES((SELECT next_seq FROM memory_meta), 'fact', NULL, '外部写入', '来自另一进程。', 3, \
                 strftime('%Y-%m-%d %H:%M:%S','now'), strftime('%Y-%m-%d %H:%M:%S','now'), '', 'http')",
                [],
            )
            .unwrap();
        connection
            .execute("UPDATE memory_meta SET next_seq = next_seq + 1", [])
            .unwrap();
    }

    let recalled = child.call_tool(3, "memory_recall", json!({}));
    assert_eq!(recalled["count"], 1, "cross-process WAL visibility");
    assert_eq!(recalled["memories"][0]["title"], "外部写入");

    drop(child);
    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_file(database.with_extension("db.v1.bak"));
}
