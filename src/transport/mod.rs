//! Transport layer. Both submodules share the same `Memory` and the same
//! `protocol::dispatch` function — they differ only in framing.

pub mod http;
pub mod stdio;
