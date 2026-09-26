use memory_service::{mcp, store::MemoryStore};
use std::{net::SocketAddr, path::PathBuf};

fn main() {
    let command = std::env::args().nth(1);
    match command.as_deref() {
        Some("mcp") => run_mcp(),
        Some("serve") => run_serve(),
        _ => {
            eprintln!("usage: memory-service <mcp|serve>");
            eprintln!("  mcp    JSON-RPC over stdio (Claude Code MCP transport)");
            eprintln!("  serve  loopback HTTP API (see docs/development/personal-memory.md)");
            std::process::exit(2);
        }
    }
}

fn database_path() -> PathBuf {
    if let Ok(path) = std::env::var("QIBAN_MEMORY_DB") {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".personal-memory").join("memory.db")
}

fn run_mcp() {
    // Stdout carries protocol frames only: no banner, no logging, ever.
    match MemoryStore::open(&database_path(), "mcp") {
        Ok(store) => mcp::run(store),
        Err(error) => {
            eprintln!("memory-service: cannot open archive: {error}");
            std::process::exit(1);
        }
    }
}

fn run_serve() {
    let addr: SocketAddr = std::env::var("QIBAN_MEMORY_ADDR")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 4322)));
    // v1 has no authentication; binding beyond loopback belongs to the
    // Phase 2 LAN plan together with an auth layer.
    if !addr.ip().is_loopback() {
        eprintln!(
            "memory-service: refusing non-loopback bind {addr}; \
             LAN exposure is a Phase 2 feature (docs/development/personal-memory.md)"
        );
        std::process::exit(1);
    }
    let store = match MemoryStore::open(&database_path(), "http") {
        Ok(store) => std::sync::Arc::new(store),
        Err(error) => {
            eprintln!("memory-service: cannot open archive: {error}");
            std::process::exit(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime builds");
    if let Err(error) = runtime.block_on(memory_service::http::serve(addr, store)) {
        eprintln!("memory-service: {error}");
        std::process::exit(1);
    }
}
