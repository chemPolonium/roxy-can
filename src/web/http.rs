// HTTP/session host split by concern; all chunks stay in the same private
// module so DTOs and helpers remain visible without widening visibility.

include!("http/startup.rs");
include!("http/session.rs");
include!("http/tools.rs");
include!("http/presenters.rs");
