//! KDL parsing for the workspace, with knuffel's grammar compiled once.
//!
//! knuffel's chumsky grammar is generic over the span type, so `knuffel::parse`, `parse_with_context` and
//! `parse_ast` compile it again in every crate that calls them. Its type names run to about a megabyte each and
//! made up most of the workspace's debug info. The parsers here are not generic and never inlined, so the grammar
//! is compiled only in this crate; callers compile only their own decoders.
//!
//! Derived decoders (`#[derive(Decode)]`) name the `knuffel` crate, so a crate that derives them still depends on
//! `knuffel` directly.

use std::any::Any;

use knuffel::ast::{Document, SpannedNode};
use knuffel::decode::Context;
use knuffel::errors::DecodeError;
use knuffel::span::{LineSpan, Span};

pub use knuffel::{Decode, DecodeChildren, DecodeScalar, Error};
pub use knuffel::{ast, decode, errors, span, traits};

/// Parses KDL text into an AST with byte-offset spans.
#[inline(never)]
pub fn parse_ast(file_name: &str, text: &str) -> Result<Document<Span>, Error> {
    knuffel::parse_ast(file_name, text)
}

/// Parses KDL text into an AST with line and column spans.
#[inline(never)]
pub fn parse_ast_line_span(file_name: &str, text: &str) -> Result<Document<LineSpan>, Error> {
    knuffel::parse_ast(file_name, text)
}

/// Parses KDL text and decodes a Rust value, like `knuffel::parse`.
pub fn parse<T>(file_name: &str, text: &str) -> Result<T, Error>
where
    T: DecodeChildren<Span> + 'static,
{
    parse_with_context(file_name, text, |_| {})
}

/// Parses KDL text and decodes a Rust value with extra decoder context, like `knuffel::parse_with_context`.
///
/// `T` must be `'static`: the decoded value comes back through the non-generic parser as a `Box<dyn Any>`.
pub fn parse_with_context<T, S, F>(file_name: &str, text: &str, set_ctx: F) -> Result<T, Error>
where
    F: FnOnce(&mut Context<S>),
    T: DecodeChildren<S> + 'static,
    S: KdlSpan,
{
    let mut set_ctx = Some(set_ctx);
    let mut set_ctx_once = |ctx: &mut Context<S>| {
        if let Some(set_ctx) = set_ctx.take() {
            set_ctx(ctx);
        }
    };
    let decoded = S::parse_erased(file_name, text, &mut set_ctx_once, DecodeFn(decode_boxed::<T, S>))?;
    Ok(*decoded.downcast::<T>().expect("the decoder returns the requested type"))
}

/// The span types the parsers here support: knuffel's `Span` and `LineSpan`.
pub trait KdlSpan: knuffel::traits::Span + sealed::Sealed {
    #[doc(hidden)]
    fn parse_erased(
        file_name: &str,
        text: &str,
        set_ctx: &mut dyn FnMut(&mut Context<Self>),
        decode: DecodeFn<Self>,
    ) -> Result<Box<dyn Any>, Error>;
}

impl KdlSpan for Span {
    fn parse_erased(
        file_name: &str,
        text: &str,
        set_ctx: &mut dyn FnMut(&mut Context<Self>),
        decode: DecodeFn<Self>,
    ) -> Result<Box<dyn Any>, Error> {
        parse_erased_span(file_name, text, set_ctx, decode)
    }
}

impl KdlSpan for LineSpan {
    fn parse_erased(
        file_name: &str,
        text: &str,
        set_ctx: &mut dyn FnMut(&mut Context<Self>),
        decode: DecodeFn<Self>,
    ) -> Result<Box<dyn Any>, Error> {
        parse_erased_line_span(file_name, text, set_ctx, decode)
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for knuffel::span::Span {}
    impl Sealed for knuffel::span::LineSpan {}
}

/// The caller's decoder, passed to the non-generic parser through the decode context.
#[doc(hidden)]
pub struct DecodeFn<S: knuffel::traits::ErrorSpan>(
    fn(&[SpannedNode<S>], &mut Context<S>) -> Result<Box<dyn Any>, DecodeError<S>>,
);

impl<S: knuffel::traits::ErrorSpan> Clone for DecodeFn<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: knuffel::traits::ErrorSpan> Copy for DecodeFn<S> {}

fn decode_boxed<T, S>(nodes: &[SpannedNode<S>], ctx: &mut Context<S>) -> Result<Box<dyn Any>, DecodeError<S>>
where
    T: DecodeChildren<S> + 'static,
    S: knuffel::traits::ErrorSpan,
{
    T::decode_children(nodes, ctx).map(|value| Box::new(value) as Box<dyn Any>)
}

/// The document as decoded by the caller's decoder, which it reads from the context.
struct Decoded(Box<dyn Any>);

impl<S: knuffel::traits::ErrorSpan> DecodeChildren<S> for Decoded {
    fn decode_children(nodes: &[SpannedNode<S>], ctx: &mut Context<S>) -> Result<Self, DecodeError<S>> {
        let decode = *ctx.get::<DecodeFn<S>>().expect("the parser sets the decoder");
        (decode.0)(nodes, ctx).map(Decoded)
    }
}

// knuffel builds the error value itself (its fields are private), so decoding runs inside `knuffel::parse_with_context`,
// called here with concrete types only.
#[inline(never)]
fn parse_erased_span(
    file_name: &str,
    text: &str,
    set_ctx: &mut dyn FnMut(&mut Context<Span>),
    decode: DecodeFn<Span>,
) -> Result<Box<dyn Any>, Error> {
    knuffel::parse_with_context::<Decoded, Span, _>(file_name, text, |ctx| {
        set_ctx(ctx);
        ctx.set(decode);
    })
    .map(|decoded| decoded.0)
}

#[inline(never)]
fn parse_erased_line_span(
    file_name: &str,
    text: &str,
    set_ctx: &mut dyn FnMut(&mut Context<LineSpan>),
    decode: DecodeFn<LineSpan>,
) -> Result<Box<dyn Any>, Error> {
    knuffel::parse_with_context::<Decoded, LineSpan, _>(file_name, text, |ctx| {
        set_ctx(ctx);
        ctx.set(decode);
    })
    .map(|decoded| decoded.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(knuffel::Decode, Debug, PartialEq)]
    struct Item {
        #[knuffel(argument)]
        name: String,
        #[knuffel(property)]
        count: u32,
    }

    #[derive(knuffel::Decode, Debug, PartialEq)]
    struct Items {
        #[knuffel(children(name = "item"))]
        items: Vec<Item>,
    }

    #[derive(Debug, PartialEq)]
    struct Prefix(&'static str);

    #[derive(Debug, PartialEq)]
    struct Prefixed(String);

    impl<S: knuffel::traits::ErrorSpan> DecodeChildren<S> for Prefixed {
        fn decode_children(nodes: &[SpannedNode<S>], ctx: &mut Context<S>) -> Result<Self, DecodeError<S>> {
            let prefix = ctx.get::<Prefix>().expect("context value").0;
            Ok(Prefixed(format!("{prefix}{}", &**nodes[0].node_name)))
        }
    }

    const VALID: &str = "item \"a\" count=1\nitem \"b\" count=2\n";
    const SYNTAX_ERROR: &str = "item \"a\" count=1\nitem \"b\" count=\n";
    const DECODE_ERROR: &str = "item \"a\" count=\"one\"\nitem count=2\n";

    fn error_text(error: &Error) -> String {
        format!("{error:?}")
    }

    #[test]
    fn parse_decodes_like_knuffel() {
        let ours: Items = parse("items.kdl", VALID).expect("parse");
        let theirs: Items = knuffel::parse("items.kdl", VALID).expect("parse");
        assert_eq!(ours, theirs);
        assert_eq!(ours.items[1], Item { name: "b".into(), count: 2 });
    }

    #[test]
    fn parse_reports_the_same_errors_as_knuffel() {
        for text in [SYNTAX_ERROR, DECODE_ERROR] {
            let ours = parse::<Items>("items.kdl", text).expect_err("error");
            let theirs = knuffel::parse::<Items>("items.kdl", text).expect_err("error");
            assert_eq!(error_text(&ours), error_text(&theirs));
        }
    }

    #[test]
    fn parse_with_context_passes_the_context() {
        let ours: Prefixed =
            parse_with_context("items.kdl", VALID, |ctx: &mut Context<LineSpan>| ctx.set(Prefix("x-"))).expect("parse");
        assert_eq!(ours, Prefixed("x-item".into()));
    }

    #[test]
    fn parse_ast_matches_knuffel() {
        for text in [VALID, SYNTAX_ERROR] {
            let ours = parse_ast("items.kdl", text).map(|document| format!("{document:?}"));
            let theirs = knuffel::parse_ast::<Span>("items.kdl", text).map(|document| format!("{document:?}"));
            assert_eq!(ours.map_err(|error| error_text(&error)), theirs.map_err(|error| error_text(&error)));

            let ours = parse_ast_line_span("items.kdl", text).map(|document| format!("{document:?}"));
            let theirs = knuffel::parse_ast::<LineSpan>("items.kdl", text).map(|document| format!("{document:?}"));
            assert_eq!(ours.map_err(|error| error_text(&error)), theirs.map_err(|error| error_text(&error)));
        }
    }
}
