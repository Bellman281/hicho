//! Infrastructure layer: concrete adapters implementing domain ports.
//!
//! - [`InMemoryPasteRepository`] — thread-safe in-memory store (test double /
//!   local runs), PR #3.
//! - `SqlitePasteRepository` — the production sqlx adapter (PR #4).

mod memory;

pub use memory::InMemoryPasteRepository;
