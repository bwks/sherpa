//! The `field_comment` lint.

use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::paths::{PathNS, lookup_path_str};
use clippy_utils::tokenize_with_text;
use clippy_utils::ty::implements_trait;
use rustc_hir::def_id::DefId;
use rustc_hir::{Item, ItemKind};
use rustc_lexer::TokenKind;
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::{BytePos, Span};

/// The traits clap's derives implement. clap builds the command line help from the doc comments
/// of a type that implements one.
const CLAP_TRAITS: &[&str] = &[
    "clap::Parser",
    "clap::Subcommand",
    "clap::Args",
    "clap::ValueEnum",
];

declare_lint! {
    /// Refuses a comment inside the body of a struct, an enum or a union: a doc comment, a plain
    /// comment, a block comment, a trailing comment or a `#[doc = "..."]` attribute on a field
    /// or a variant. A field or a variant is named for what it holds.
    ///
    /// A type that implements clap's `Parser`, `Subcommand`, `Args` or `ValueEnum` is not
    /// checked, however the implementation was written or the derive was imported. A type that
    /// a macro generates is not checked.
    pub FIELD_COMMENT,
    Warn,
    "a comment on a struct field or an enum variant"
}

#[derive(Default)]
pub struct FieldComment {
    clap_traits: Vec<DefId>,
}

impl_lint_pass!(FieldComment => [FIELD_COMMENT]);

impl<'tcx> LateLintPass<'tcx> for FieldComment {
    fn check_crate(&mut self, cx: &LateContext<'tcx>) {
        self.clap_traits = CLAP_TRAITS
            .iter()
            .flat_map(|path| lookup_path_str(cx.tcx, PathNS::Type, path))
            .collect();
    }

    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx Item<'tcx>) {
        let (ItemKind::Struct(ident, generics, _)
        | ItemKind::Enum(ident, generics, _)
        | ItemKind::Union(ident, generics, _)) = item.kind
        else {
            return;
        };
        if item.span.from_expansion() || ident.span.from_expansion() {
            return;
        }
        let ty = cx
            .tcx
            .type_of(item.owner_id)
            .instantiate_identity()
            .skip_norm_wip();
        if self
            .clap_traits
            .iter()
            .any(|&clap_trait| implements_trait(cx, ty, clap_trait, &[]))
        {
            return;
        }
        let Ok(text) = cx.sess().source_map().span_to_snippet(item.span) else {
            return;
        };
        let mut header_end = ident.span.hi();
        if item.span.contains(generics.span) {
            header_end = header_end.max(generics.span.hi());
        }
        let header_len = (header_end - item.span.lo()).0 as usize;

        let mut in_body = false;
        let mut recent = [(TokenKind::Whitespace, ""); 3];
        let mut attribute_start = 0;
        for (kind, token_text, range) in tokenize_with_text(&text) {
            if !in_body {
                in_body = range.start >= header_len
                    && matches!(kind, TokenKind::OpenBrace | TokenKind::OpenParen);
                continue;
            }
            match kind {
                TokenKind::Whitespace => continue,
                TokenKind::LineComment { .. } | TokenKind::BlockComment { .. } => {
                    report(cx, span_in(item.span, range.start, range.end));
                    continue;
                }
                TokenKind::Pound => attribute_start = range.start,
                TokenKind::Eq
                    if matches!(
                        recent,
                        [
                            (TokenKind::Pound, _),
                            (TokenKind::OpenBracket, _),
                            (TokenKind::Ident, "doc")
                        ]
                    ) =>
                {
                    report(cx, span_in(item.span, attribute_start, range.end));
                }
                _ => {}
            }
            recent = [recent[1], recent[2], (kind, token_text)];
        }
    }
}

/// The part of `outer` from byte `start` to byte `end` of its text.
fn span_in(outer: Span, start: usize, end: usize) -> Span {
    let lo = outer.lo() + BytePos(u32::try_from(start).unwrap_or(0));
    let hi = outer.lo() + BytePos(u32::try_from(end).unwrap_or(0));
    outer.with_lo(lo).with_hi(hi)
}

/// Reports a comment at `span`.
fn report(cx: &LateContext<'_>, span: Span) {
    span_lint_and_help(
        cx,
        FIELD_COMMENT,
        span,
        "comment on a struct field or an enum variant",
        None,
        "name the field or the variant for what it holds, and say the rest in the type's doc comment",
    );
}
