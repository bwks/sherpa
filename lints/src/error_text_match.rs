//! The `error_text_match` and `error_text_match_in_test` lints.

use std::ops::ControlFlow;

use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::macros::{macro_backtrace, root_macro_call};
use clippy_utils::paths::{PathNS, lookup_path_str};
use clippy_utils::res::MaybeResPath;
use clippy_utils::ty::implements_trait;
use clippy_utils::visitors::for_each_expr;
use clippy_utils::{find_binding_init, is_in_test, sym};
use rustc_hir::def_id::DefId;
use rustc_hir::{BinOpKind, Expr, ExprKind, UnOp};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_middle::ty::{self, Ty};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::Span;

const MAX_DEPTH: u8 = 8;

/// Methods that hand on the text they are called on, so the result is still an error's text.
const PASS_THROUGH: &[&str] = &[
    "as_str",
    "as_ref",
    "borrow",
    "clone",
    "to_owned",
    "to_string",
    "to_lowercase",
    "to_uppercase",
    "to_ascii_lowercase",
    "to_ascii_uppercase",
    "trim",
    "trim_start",
    "trim_end",
];

/// Methods that decide something from the text they are called on.
const TEXT_TESTS: &[&str] = &[
    "contains",
    "starts_with",
    "ends_with",
    "find",
    "rfind",
    "matches",
    "strip_prefix",
    "strip_suffix",
    "split",
    "split_once",
    "rsplit_once",
    "eq",
    "ne",
    "eq_ignore_ascii_case",
    "parse",
];

/// Text tests that also decide from their argument, as in `list.contains(&error.to_string())`.
const ARGUMENT_TESTS: &[&str] = &["contains", "eq", "ne", "eq_ignore_ascii_case"];

const COMPARE_MACROS: &[&str] = &[
    "assert_eq_macro",
    "assert_ne_macro",
    "debug_assert_eq_macro",
    "debug_assert_ne_macro",
];

declare_lint! {
    /// Refuses a decision taken from an error's text: a comparison, a `match` or a string test
    /// such as `contains` or `starts_with` on what `to_string()` or `format!` made of a value
    /// whose type is an error (it implements `std::error::Error`, or is `anyhow::Error` or a
    /// boxed error). The text is followed through `let` bindings and through methods that keep
    /// it text (`as_str`, `to_lowercase`, `trim`, ...).
    ///
    /// Match on the error's type or variant instead, or downcast to it.
    pub ERROR_TEXT_MATCH,
    Warn,
    "a decision taken from the text of an error"
}

declare_lint! {
    /// The same check as `error_text_match`, for code in a test: a `#[test]` function, a
    /// `#[cfg(test)]` module or a file under a `tests` folder. It has its own name so that the
    /// two can be given different levels.
    pub ERROR_TEXT_MATCH_IN_TEST,
    Warn,
    "a test that checks an error by its text"
}

#[derive(Default)]
pub struct ErrorTextMatch {
    anyhow_errors: Vec<DefId>,
}

impl_lint_pass!(ErrorTextMatch => [ERROR_TEXT_MATCH, ERROR_TEXT_MATCH_IN_TEST]);

impl<'tcx> LateLintPass<'tcx> for ErrorTextMatch {
    fn check_crate(&mut self, cx: &LateContext<'tcx>) {
        self.anyhow_errors = lookup_path_str(cx.tcx, PathNS::Type, "anyhow::Error");
    }

    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        match expr.kind {
            ExprKind::MethodCall(segment, receiver, arguments, _)
                if !expr.span.from_expansion() =>
            {
                let name = segment.ident.name.as_str();
                if !TEXT_TESTS.contains(&name) {
                    return;
                }
                let from_argument = ARGUMENT_TESTS.contains(&name)
                    && arguments
                        .iter()
                        .any(|argument| self.is_error_text(cx, argument, 0));
                if from_argument || self.is_error_text(cx, receiver, 0) {
                    report(cx, expr, expr.span);
                }
            }
            ExprKind::Binary(operator, left, right)
                if !expr.span.from_expansion()
                    && matches!(operator.node, BinOpKind::Eq | BinOpKind::Ne) =>
            {
                if self.is_error_text(cx, left, 0) || self.is_error_text(cx, right, 0) {
                    report(cx, expr, expr.span);
                }
            }
            ExprKind::Match(scrutinee, ..) if !expr.span.from_expansion() => {
                if self.is_error_text(cx, scrutinee, 0) {
                    report(cx, expr, scrutinee.span);
                }
            }
            ExprKind::Match(scrutinee, ..) => {
                let Some(call) = root_macro_call(expr.span) else {
                    return;
                };
                let compares = cx
                    .tcx
                    .get_diagnostic_name(call.def_id)
                    .is_some_and(|name| COMPARE_MACROS.contains(&name.as_str()));
                if !compares {
                    return;
                }
                let ExprKind::Tup(sides) = scrutinee.kind else {
                    return;
                };
                if sides.iter().any(|side| self.is_error_text(cx, side, 0)) {
                    report(cx, expr, call.span);
                }
            }
            _ => {}
        }
    }
}

impl ErrorTextMatch {
    /// Whether `expr` holds text made from an error by `to_string()` or `format!`.
    fn is_error_text<'tcx>(
        &self,
        cx: &LateContext<'tcx>,
        expr: &'tcx Expr<'tcx>,
        depth: u8,
    ) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }
        let expr = peel(expr);
        if macro_backtrace(expr.span)
            .any(|call| cx.tcx.is_diagnostic_item(sym::format_macro, call.def_id))
        {
            return for_each_expr(cx.tcx, expr, |part| {
                if self.is_error_type(cx, cx.typeck_results().expr_ty(part)) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .is_some();
        }
        match expr.kind {
            ExprKind::MethodCall(segment, receiver, arguments, _) => {
                let name = segment.ident.name.as_str();
                if name == "to_string"
                    && arguments.is_empty()
                    && self.is_error_type(cx, cx.typeck_results().expr_ty(receiver))
                {
                    return true;
                }
                PASS_THROUGH.contains(&name) && self.is_error_text(cx, receiver, depth + 1)
            }
            ExprKind::Path(_) => expr
                .res_local_id()
                .and_then(|id| find_binding_init(cx, id))
                .is_some_and(|init| self.is_error_text(cx, init, depth + 1)),
            _ => false,
        }
    }

    /// Whether `ty`, behind any references and boxes, is an error type.
    fn is_error_type<'tcx>(&self, cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> bool {
        let ty = ty.peel_refs();
        match ty.kind() {
            ty::Adt(adt, arguments) => {
                if adt.is_box() {
                    return self.is_error_type(cx, arguments.type_at(0));
                }
                if self.anyhow_errors.contains(&adt.did()) {
                    return true;
                }
            }
            ty::Dynamic(predicates, ..) => {
                return predicates
                    .principal_def_id()
                    .is_some_and(|id| cx.tcx.is_diagnostic_item(sym::Error, id));
            }
            _ => {}
        }
        cx.tcx
            .get_diagnostic_item(sym::Error)
            .is_some_and(|error_trait| implements_trait(cx, ty, error_trait, &[]))
    }
}

/// Strips borrows, dereferences and temporaries-scope wrappers from `expr`.
fn peel<'tcx>(mut expr: &'tcx Expr<'tcx>) -> &'tcx Expr<'tcx> {
    loop {
        expr = match expr.kind {
            ExprKind::AddrOf(_, _, inner)
            | ExprKind::Unary(UnOp::Deref, inner)
            | ExprKind::DropTemps(inner) => inner,
            _ => return expr,
        };
    }
}

/// Reports `span` under the lint for test code or for the rest, by where `expr` is.
fn report(cx: &LateContext<'_>, expr: &Expr<'_>, span: Span) {
    let location = cx.sess().source_map().span_to_diagnostic_string(span);
    let in_test = is_in_test(cx.tcx, expr.hir_id)
        || location.starts_with("tests/")
        || location.contains("/tests/");
    if in_test {
        span_lint_and_help(
            cx,
            ERROR_TEXT_MATCH_IN_TEST,
            span,
            "test checks an error by its text",
            None,
            "check the error's type or variant; keep a text check for the wording of one message only",
        );
    } else {
        span_lint_and_help(
            cx,
            ERROR_TEXT_MATCH,
            span,
            "decision taken from the text of an error",
            None,
            "match on the error's type or variant, or downcast to it",
        );
    }
}
