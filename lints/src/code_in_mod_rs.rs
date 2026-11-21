//! The `code_in_mod_rs` lint.

use std::ffi::OsStr;

use clippy_utils::diagnostics::span_lint_and_help;
use rustc_ast::{Crate, Inline, Item, ItemKind, ModKind, VisibilityKind};
use rustc_lint::{EarlyContext, EarlyLintPass, LintContext};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::{FileName, Span};

const MOD_FILE_NAME: &str = "mod.rs";

declare_lint! {
    /// Refuses any item in a `mod.rs` file other than a `mod` declaration (`mod name;`) and a
    /// re-export (`pub use`, of any visibility). Sherpa keeps code out of `mod.rs` files
    /// (`AGENTS.md`, "Mod files"). Docs and attributes are not items, so they pass.
    ///
    /// An item that a macro generates is not reported.
    pub CODE_IN_MOD_RS,
    Warn,
    "an item other than a `mod` declaration or a `pub use` in a `mod.rs` file"
}

declare_lint_pass!(CodeInModRs => [CODE_IN_MOD_RS]);

impl EarlyLintPass for CodeInModRs {
    fn check_crate(&mut self, cx: &EarlyContext<'_>, krate: &Crate) {
        if is_mod_rs(cx, krate.spans.inner_span) {
            check_items(cx, &krate.items);
        }
    }

    fn check_item(&mut self, cx: &EarlyContext<'_>, item: &Item) {
        if let ItemKind::Mod(_, _, ModKind::Loaded(items, Inline::No { .. }, spans)) = &item.kind
            && is_mod_rs(cx, spans.inner_span)
        {
            check_items(cx, items);
        }
    }
}

/// Whether the span lies in a file named `mod.rs`.
fn is_mod_rs(cx: &EarlyContext<'_>, span: Span) -> bool {
    match cx.sess().source_map().span_to_filename(span) {
        FileName::Real(name) => name
            .local_path()
            .is_some_and(|path| path.file_name() == Some(OsStr::new(MOD_FILE_NAME))),
        _ => false,
    }
}

/// Reports every item of a `mod.rs` file that is neither a `mod` declaration nor a re-export.
fn check_items(cx: &EarlyContext<'_>, items: &[Box<Item>]) {
    for item in items {
        if item.span.from_expansion() || is_allowed(item) {
            continue;
        }
        span_lint_and_help(
            cx,
            CODE_IN_MOD_RS,
            cx.sess().source_map().guess_head_span(item.span),
            "code in a `mod.rs` file",
            None,
            "a `mod.rs` holds only docs, `mod` declarations and `pub use` re-exports; move this to its own module",
        );
    }
}

/// Whether a `mod.rs` may hold the item: `mod name;` or a `use` with a visibility.
fn is_allowed(item: &Item) -> bool {
    match &item.kind {
        ItemKind::Mod(_, _, ModKind::Unloaded | ModKind::Loaded(_, Inline::No { .. }, _)) => true,
        ItemKind::Use(_) => !matches!(item.vis.kind, VisibilityKind::Inherited),
        _ => false,
    }
}
