---
id: rule-053
paths: "crates/*/src/**/*.rs"
---

When adding new public types (structs, enums) to a crate module, ensure they are exported in lib.rs. New types must be added to the `pub use` statement to be accessible from external crates. Check the crate's lib.rs exports after adding types to internal modules.