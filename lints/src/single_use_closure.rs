//! The `single_use_closure` lint.

use clippy_utils::diagnostics::span_lint_and_then;
use clippy_utils::visitors::local_used_once;
use rustc_hir::{ClosureKind, ExprKind, LetStmt, PatKind};
use rustc_lint::{LateContext, LateLintPass};
use rustc_session::{declare_lint, declare_lint_pass};

declare_lint! {
    /// Refuses a closure bound to a name with `let` and then used exactly once in the function.
    /// Write the code where it is used, or make it a function if it deserves a name.
    ///
    /// An `async` block bound to a name is not a closure and is not reported, nor is a closure
    /// that a macro generates.
    pub SINGLE_USE_CLOSURE,
    Warn,
    "a closure bound with `let` and used once"
}

declare_lint_pass!(SingleUseClosure => [SINGLE_USE_CLOSURE]);

impl<'tcx> LateLintPass<'tcx> for SingleUseClosure {
    fn check_local(&mut self, cx: &LateContext<'tcx>, local: &'tcx LetStmt<'tcx>) {
        let Some(init) = local.init else {
            return;
        };
        let ExprKind::Closure(closure) = init.kind else {
            return;
        };
        if local.span.from_expansion() || !matches!(closure.kind, ClosureKind::Closure) {
            return;
        }
        let PatKind::Binding(_, id, ident, None) = local.pat.kind else {
            return;
        };
        let owner = cx.tcx.hir_enclosing_body_owner(local.hir_id);
        let body = cx.tcx.hir_body_owned_by(owner);
        let Some(only_use) = local_used_once(cx, body.value, id) else {
            return;
        };
        span_lint_and_then(
            cx,
            SINGLE_USE_CLOSURE,
            local.pat.span,
            format!("closure `{}` is used once", ident.name),
            |diag| {
                diag.span_note(only_use.span, "its only use");
                diag.help("write the code where it is used, or make it a function");
            },
        );
    }
}
