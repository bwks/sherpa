//! The `use_in_function` lint.

use clippy_utils::diagnostics::span_lint_and_help;
use rustc_ast::{ItemKind, Stmt, StmtKind};
use rustc_lint::{EarlyContext, EarlyLintPass};
use rustc_session::{declare_lint, declare_lint_pass};

declare_lint! {
    /// Refuses a `use` statement written inside a block: a function body, a closure or any
    /// nested block. Sherpa declares every `use` at the top of the file (`AGENTS.md`, "Use").
    ///
    /// A `use` that a macro generates is not reported: its author cannot move it.
    pub USE_IN_FUNCTION,
    Warn,
    "a `use` statement declared inside a function"
}

declare_lint_pass!(UseInFunction => [USE_IN_FUNCTION]);

impl EarlyLintPass for UseInFunction {
    fn check_stmt(&mut self, cx: &EarlyContext<'_>, stmt: &Stmt) {
        let StmtKind::Item(item) = &stmt.kind else {
            return;
        };
        if !matches!(item.kind, ItemKind::Use(_)) || item.span.from_expansion() {
            return;
        }
        span_lint_and_help(
            cx,
            USE_IN_FUNCTION,
            item.span,
            "`use` declared inside a function",
            None,
            "declare it with the other `use` statements at the top of the file",
        );
    }
}
