//! Web frontend host and HTTP API.
//!
//! The bus core is unchanged: this module starts an `App` on a manual drive in
//! its own thread, publishes a small JSON snapshot, and serves the Vite build
//! embedded at compile time.
//!// Web host split into HTTP/session plumbing, runtime commands, state/DTOs
// and tests. `include!` keeps one private module, so handlers can keep using
// the same internal types without widening visibility.

include!("web/http.rs");
include!("web/runtime.rs");
include!("web/state.rs");
include!("web/tests.rs");
