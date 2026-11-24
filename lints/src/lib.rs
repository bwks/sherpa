//! Prototype Dylint lints for Sherpa's own code rules:
//!
//! - `error_text_match`, `error_text_match_in_test`: no decision is taken from an error's text.
//! - `field_comment`: struct fields and enum variants carry no comments, except on clap types.
//! - `version_history`: comments and string literals carry no version-history narrative.
//! - `single_use_closure`: a closure is not bound to a name to be used once.
//! - `use_in_function`: a `use` statement is never declared inside a function.
//! - `code_in_mod_rs`: a `mod.rs` file holds only docs, `mod` declarations and `pub use`
//!   re-exports.
//!
//! Nothing runs this library in CI or in the pre-commit hook.

#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_hir;
extern crate rustc_lexer;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

mod code_in_mod_rs;
mod error_text_match;
mod field_comment;
mod single_use_closure;
mod use_in_function;
mod version_history;

use rustc_lint::LintStore;
use rustc_session::Session;

use code_in_mod_rs::{CODE_IN_MOD_RS, CodeInModRs};
use error_text_match::{ERROR_TEXT_MATCH, ERROR_TEXT_MATCH_IN_TEST, ErrorTextMatch};
use field_comment::{FIELD_COMMENT, FieldComment};
use single_use_closure::{SINGLE_USE_CLOSURE, SingleUseClosure};
use use_in_function::{USE_IN_FUNCTION, UseInFunction};
use version_history::{VERSION_HISTORY, VersionHistory};

dylint_linting::dylint_library!();

/// Registers every lint with the compiler. Dylint calls this once per crate it lints.
#[unsafe(no_mangle)]
pub fn register_lints(sess: &Session, lint_store: &mut LintStore) {
    dylint_linting::init_config(sess);
    lint_store.register_lints(&[
        CODE_IN_MOD_RS,
        ERROR_TEXT_MATCH,
        ERROR_TEXT_MATCH_IN_TEST,
        FIELD_COMMENT,
        SINGLE_USE_CLOSURE,
        USE_IN_FUNCTION,
        VERSION_HISTORY,
    ]);
    lint_store.register_early_lint_pass(Box::new(|| Box::new(UseInFunction)));
    lint_store.register_early_lint_pass(Box::new(|| Box::new(CodeInModRs)));
    lint_store.register_early_lint_pass(Box::new(|| Box::new(VersionHistory)));
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(ErrorTextMatch::default())));
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(FieldComment::default())));
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(SingleUseClosure)));
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}
