//! The wezterminator data model.
//!
//! - [`model`]: typed mirrors of the JSON documents in `schema/`, with `_`
//!   comment keys kept through read and write ([`comments`]).
//! - [`resolve`]: layer resolution, the Rust twin of `plugin/wzt/resolve.lua`.
//! - [`paths`]: XDG paths (Known Folders on Windows) via `etcetera`.
//! - [`io`]: reading documents and atomic writes.
//! - [`version`]: the `schema_version` gate.
//!
//! `docs/data-model.md` is the contract and `tests/fixtures/resolution/` is
//! the arbiter when the two runtimes disagree.

pub mod comments;
pub mod error;
pub mod io;
pub mod json;
pub mod loader;
pub mod model;
pub mod paths;
pub mod resolve;
pub mod version;

pub use comments::{Comments, Keyed, is_comment_key};
pub use error::{Error, Result};
pub use io::{read_document, write_atomic, write_document};
pub use model::*;
pub use paths::Paths;
pub use resolve::{Resolution, ResolveInput, resolve};
pub use version::SUPPORTED_SCHEMA_VERSION;
