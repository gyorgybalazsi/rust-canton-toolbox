// Hand-written DAML example/integration tests. These deliberately mirror DAML
// and ledger-command shapes — camelCase field names, many-argument command
// builders, defensive `..Default::default()`, nested same-name modules — so we
// allow the corresponding style lints crate-wide rather than distort the
// examples. (Real-bug lints are still enforced.)
#![allow(
    clippy::needless_update,
    clippy::too_many_arguments,
    clippy::module_inception,
    non_snake_case
)]

pub mod asset;
pub mod full;
pub mod interface_example;
pub mod nested_test;
pub mod optional;
pub mod ticketoffer;
pub mod ticketoffer_explicit_disclosure;
