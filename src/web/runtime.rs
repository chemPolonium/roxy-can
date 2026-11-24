// Runtime split by responsibility. `include!` keeps the private web module
// scope, so the pieces share handlers and DTOs without pub(crate) spam.

include!("runtime/app_loop.rs");
include!("runtime/store_assets.rs");
include!("runtime/signal_store.rs");
include!("runtime/signal_helpers.rs");
include!("runtime/catalog.rs");
