//! Strict reading of hand-written KDL files: errors name the file, the 1-based line and column, the offending node
//! or property and the names allowed at that place.
//!
//! - [`NodeSchema`] lists the nodes a document accepts, with their properties and children. [`check_document`]
//!   rejects any other node or property.
//! - [`decode`] runs a derived (`#[derive(Decode)]`) decoder. knuffel's derives already reject unexpected nodes,
//!   properties and arguments; when decoding fails, the schema check runs to name the allowed names too.
//! - Hand-walked readers parse with [`crate::parse_ast_line_span`], check the document with [`check_document`] and
//!   read values with the helpers here, which report errors through [`error_at`].

use std::fmt::Display;

use color_eyre::eyre;
use knuffel::ast::{Document, Literal, Node, SpannedNode, Value};
use knuffel::span::{LineSpan, Span};
use knuffel::traits::DecodeChildren;

use crate::Error;

/// The properties and children one node accepts. Arguments are checked by the reader.
pub struct NodeSchema {
    pub name: &'static str,
    pub properties: &'static [&'static str],
    pub children: &'static [NodeSchema],
}

impl NodeSchema {
    /// A node with properties and no children.
    pub const fn leaf(name: &'static str, properties: &'static [&'static str]) -> Self {
        Self { name, properties, children: &[] }
    }
}

/// Decodes a document with a derived decoder. `file_name` starts every error message.
///
/// On a decode error, `schema` (the nodes the decoder accepts) is checked first, so an unknown node or property is
/// reported with the names allowed at its place; any other error is knuffel's, with its line and column.
pub fn decode<T>(file_name: &str, text: &str, schema: &[NodeSchema]) -> eyre::Result<T>
where
    T: DecodeChildren<Span> + 'static,
{
    crate::parse::<T>(file_name, text).map_err(|error| {
        let schema_error = crate::parse_ast_line_span(file_name, text)
            .ok()
            .and_then(|document| check_document(&document, schema).err());
        let report = schema_error.unwrap_or_else(|| describe_error(text, &error));
        in_file(file_name, report)
    })
}

/// Prefixes an error with the file it comes from.
pub fn in_file(file_name: &str, error: eyre::Report) -> eyre::Report {
    eyre::eyre!("{file_name}: {error}")
}

/// Checks that the document's nodes, their properties and their children are the ones `schema` lists.
pub fn check_document(document: &Document<LineSpan>, schema: &[NodeSchema]) -> eyre::Result<()> {
    check_nodes(&document.nodes, None, schema)
}

fn check_nodes<'a>(
    nodes: impl IntoIterator<Item = &'a SpannedNode<LineSpan>>,
    parent: Option<&str>,
    schema: &[NodeSchema],
) -> eyre::Result<()> {
    for node in nodes {
        let node_schema = node_schema(node, parent, schema)?;
        check_properties(node, node_schema.properties)?;
        check_nodes(node.children(), Some(node_schema.name), node_schema.children)?;
    }
    Ok(())
}

/// The schema entry named like `node`, or an error naming the nodes allowed under `parent`.
pub fn node_schema<'a>(
    node: &SpannedNode<LineSpan>,
    parent: Option<&str>,
    schema: &'a [NodeSchema],
) -> eyre::Result<&'a NodeSchema> {
    let name: &str = &node.node_name;
    schema.iter().find(|entry| entry.name == name).ok_or_else(|| {
        let names: Vec<_> = schema.iter().map(|entry| entry.name).collect();
        unknown_node(node, parent, &names)
    })
}

/// An error for a node not among `allowed` under `parent` (`None`: the top level).
pub fn unknown_node(node: &Node<LineSpan>, parent: Option<&str>, allowed: &[&str]) -> eyre::Report {
    let name: &str = &node.node_name;
    let message = match (parent, allowed.is_empty()) {
        (Some(parent), true) => format!("`{parent}` takes no children, found `{name}`"),
        (None, true) => format!("unknown node `{name}`: the file takes no nodes"),
        (Some(parent), false) => format!("unknown node `{name}` in `{parent}`: expected {}", one_of(allowed)),
        (None, false) => format!("unknown node `{name}` at the top level: expected {}", one_of(allowed)),
    };
    error_at(node.node_name.span(), message)
}

/// Rejects properties not among `allowed`.
pub fn check_properties(node: &Node<LineSpan>, allowed: &[&str]) -> eyre::Result<()> {
    let node_name: &str = &node.node_name;
    for key in node.properties.keys() {
        let key_name: &str = key;
        if !allowed.contains(&key_name) {
            let message = if allowed.is_empty() {
                format!("`{node_name}` takes no properties, found `{key_name}`")
            } else {
                format!("unknown property `{key_name}` of `{node_name}`: expected {}", one_of(allowed))
            };
            return Err(error_at(key.span(), message));
        }
    }
    Ok(())
}

/// Rejects a children block.
pub fn reject_children(node: &Node<LineSpan>) -> eyre::Result<()> {
    match node.children().next() {
        Some(child) => Err(unknown_node(child, Some(&**node.node_name), &[])),
        None => Ok(()),
    }
}

/// The node's arguments, which must be exactly `N`.
pub fn arguments<const N: usize>(node: &Node<LineSpan>) -> eyre::Result<&[Value<LineSpan>; N]> {
    let node_name: &str = &node.node_name;
    node.arguments.as_slice().try_into().map_err(|_| {
        let span = node.arguments.get(N).map_or(node.node_name.span(), |extra| extra.literal.span());
        error_at(span, format!("`{node_name}` takes {N} arguments, found {}", node.arguments.len()))
    })
}

/// The value of property `name`, if present.
pub fn property<'a>(node: &'a Node<LineSpan>, name: &str) -> Option<&'a Value<LineSpan>> {
    node.properties.iter().find(|(key, _)| key.as_ref() == name).map(|(_, value)| value)
}

/// The value of property `name`, or an error naming the node.
pub fn required_property<'a>(node: &'a Node<LineSpan>, name: &str) -> eyre::Result<&'a Value<LineSpan>> {
    let node_name: &str = &node.node_name;
    property(node, name).ok_or_else(|| error_at(node.node_name.span(), format!("`{node_name}`: missing {name}=")))
}

pub fn value_string(value: &Value<LineSpan>, field: &str) -> eyre::Result<String> {
    match &*value.literal {
        Literal::String(text) => Ok(text.to_string()),
        _ => Err(error_at(value.literal.span(), format!("{field}: expected a string"))),
    }
}

pub fn value_u16(value: &Value<LineSpan>, field: &str) -> eyre::Result<u16> {
    value_integer(value, field, "u16")
}

pub fn value_u32(value: &Value<LineSpan>, field: &str) -> eyre::Result<u32> {
    value_integer(value, field, "u32")
}

fn value_integer<T>(value: &Value<LineSpan>, field: &str, type_name: &str) -> eyre::Result<T>
where
    T: for<'i> TryFrom<&'i knuffel::ast::Integer>,
    for<'i> <T as TryFrom<&'i knuffel::ast::Integer>>::Error: Display,
{
    let span = value.literal.span();
    match &*value.literal {
        Literal::Int(integer) => {
            T::try_from(integer).map_err(|error| error_at(span, format!("{field}: not a {type_name} ({error})")))
        }
        _ => Err(error_at(span, format!("{field}: expected an integer"))),
    }
}

/// A number, integer or decimal.
pub fn value_f32(value: &Value<LineSpan>, field: &str) -> eyre::Result<f32> {
    let span = value.literal.span();
    match &*value.literal {
        Literal::Int(integer) => i64::try_from(integer)
            .map(|integer| integer as f32)
            .map_err(|error| error_at(span, format!("{field}: not a number ({error})"))),
        Literal::Decimal(decimal) => {
            f32::try_from(decimal).map_err(|error| error_at(span, format!("{field}: not a number ({error})")))
        }
        _ => Err(error_at(span, format!("{field}: expected a number"))),
    }
}

/// An error at a 1-based line and column.
pub fn error_at(span: &LineSpan, message: impl Display) -> eyre::Report {
    eyre::eyre!("line {}, column {}: {message}", span.0.line + 1, span.0.column + 1)
}

/// knuffel's syntax or decode errors with their 1-based line and column.
pub fn describe_error(text: &str, error: &Error) -> eyre::Report {
    use miette::Diagnostic;

    let mut messages = Vec::new();
    for diagnostic in error.related().into_iter().flatten() {
        let labels: Vec<_> = diagnostic.labels().into_iter().flatten().collect();
        match labels.first() {
            Some(label) => {
                let (line, column) = line_column(text, label.offset());
                let detail = label.label().map(|label| format!(" ({label})")).unwrap_or_default();
                messages.push(format!("line {line}, column {column}: {diagnostic}{detail}"));
            }
            None => messages.push(diagnostic.to_string()),
        }
    }
    if messages.is_empty() {
        messages.push(error.to_string());
    }
    eyre::eyre!("invalid KDL: {}", messages.join("; "))
}

/// 1-based line and column (in characters) of a byte offset.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    (before.matches('\n').count() + 1, before[line_start..].chars().count() + 1)
}

/// `a`, `a or b`, `one of a, b, c`.
fn one_of(names: &[&str]) -> String {
    match names {
        [name] => format!("`{name}`"),
        [first, second] => format!("`{first}` or `{second}`"),
        _ => format!("one of {}", names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(knuffel::Decode, Debug)]
    struct Item {
        #[knuffel(argument)]
        #[allow(dead_code)]
        name: String,
        #[knuffel(property)]
        #[allow(dead_code)]
        count: u32,
    }

    #[derive(knuffel::Decode, Debug)]
    struct Items {
        #[knuffel(children(name = "item"))]
        #[allow(dead_code)]
        items: Vec<Item>,
    }

    const SCHEMA: &[NodeSchema] = &[NodeSchema::leaf("item", &["count"])];

    fn decode_error(text: &str) -> String {
        decode::<Items>("items.kdl", text, SCHEMA).expect_err("error").to_string()
    }

    #[test]
    fn decode_names_unknown_nodes_and_properties_with_the_allowed_names() {
        assert_eq!(
            decode_error("item \"a\" count=1\nfoo 1\n"),
            "items.kdl: line 2, column 1: unknown node `foo` at the top level: expected `item`"
        );
        assert_eq!(
            decode_error("item \"a\" count=1 size=2\n"),
            "items.kdl: line 1, column 18: unknown property `size` of `item`: expected `count`"
        );
        assert_eq!(
            decode_error("item \"a\" count=1 {\n    part 1\n}\n"),
            "items.kdl: line 2, column 5: `item` takes no children, found `part`"
        );
    }

    #[test]
    fn decode_reports_other_errors_with_their_position() {
        let missing = decode_error("item \"a\"\n");
        assert!(missing.starts_with("items.kdl: invalid KDL: line 1, column 1:"), "{missing}");
        assert!(missing.contains("count"), "{missing}");

        let extra = decode_error("item \"a\" \"b\" count=1\n");
        assert!(extra.starts_with("items.kdl: invalid KDL: line 1, column 10: unexpected argument"), "{extra}");
    }

    #[test]
    fn hand_walked_helpers_report_positions() {
        let text = "item 1 2\n";
        let document = crate::parse_ast_line_span("items.kdl", text).expect("parse");
        let node = &document.nodes[0];
        assert_eq!(
            arguments::<1>(node).expect_err("extra argument").to_string(),
            "line 1, column 8: `item` takes 1 arguments, found 2"
        );
        assert_eq!(
            required_property(node, "count").expect_err("missing").to_string(),
            "line 1, column 1: `item`: missing count="
        );
        assert_eq!(
            value_string(&node.arguments[0], "name").expect_err("not a string").to_string(),
            "line 1, column 6: name: expected a string"
        );
    }
}
