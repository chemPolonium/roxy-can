// Feature-area includes keep the test module one lexical scope, so the
// shared helpers and `use super::*` imports in the first chunk stay visible
// to every later chunk.

include!("app_tests/record_replay.rs");
include!("app_tests/traffic_export.rs");
include!("app_tests/protocols.rs");
include!("app_tests/state_scripts.rs");
