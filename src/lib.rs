//! Blazing-ART-MCP library crate. The binary at `src/main.rs` is a thin shell
//! that wires `Memory` (an ART-backed store) to one of two transports.
//!
//! See `BENCHMARKS.md` for the proof-of-value numbers vs `BTreeMap` and `HashMap`.

pub mod ingest;
pub mod memory;
pub mod protocol;
pub mod transport;
