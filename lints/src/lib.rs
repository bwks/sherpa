//! Prototype Dylint lints for two of Sherpa's code rules in `AGENTS.md`:
//!
//! - `use_in_function`: a `use` statement is never declared inside a function.
//! - `code_in_mod_rs`: a `mod.rs` file holds only docs, `mod` declarations and `pub use`
//!   re-exports.
//!
//! Nothing runs this library in CI or in the pre-commit hook. See
//! `docs/investigations/dylint.md`.

#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_lint;
extern crate rustc_session;
extern crate rustc_span;

mod code_in_mod_rs;
mod use_in_function;

use rustc_lint::LintStore;
use rustc_session::Session;

use code_in_mod_rs::{CODE_IN_MOD_RS, CodeInModRs};
use use_in_function::{USE_IN_FUNCTION, UseInFunction};

dylint_linting::dylint_library!();

/// Registers both lints with the compiler. Dylint calls this once per crate it lints.
#[unsafe(no_mangle)]
pub fn register_lints(sess: &Session, lint_store: &mut LintStore) {
    dylint_linting::init_config(sess);
    lint_store.register_lints(&[USE_IN_FUNCTION, CODE_IN_MOD_RS]);
    lint_store.register_early_lint_pass(Box::new(|| Box::new(UseInFunction)));
    lint_store.register_early_lint_pass(Box::new(|| Box::new(CodeInModRs)));
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}
