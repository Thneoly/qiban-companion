//! Personal memory service: one SQLite archive behind two protocols.
//! `mcp` speaks JSON-RPC over stdio for Claude Code; `serve` exposes the
//! same semantics over loopback HTTP for future clients. Fully
//! self-contained on purpose: the personal memory domain (supersession
//! chains, validity windows) is a different model from the in-app limited
//! memory, and this binary must deploy standalone under ~/.personal-memory.

pub mod errors;
pub mod http;
pub mod mcp;
pub mod store;

pub use http::{app_state, router};
pub use store::{MemoryKind, MemoryStore};
