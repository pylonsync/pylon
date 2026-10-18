//! ProseMirror (Tiptap) documents in a Loro doc.
//!
//! A `crdt: "prosemirror"` field keeps its document under its key of the
//! row map, in the layout the `loro-prosemirror` binding writes, so an
//! editor in the browser and the server read and write the same tree:
//!
//! - a node is a [`LoroMap`] with `nodeName` (a string), `attributes` (a
//!   map of the node's non-null attrs) and `children` (a [`LoroList`]);
//! - a child is a node map, or a [`LoroText`] holding a run of consecutive
//!   text nodes;
//! - each mark is a text style named after the mark type, whose value is
//!   the mark's attrs (an empty map for a mark without attrs). A style
//!   holds one value per character, so two marks of one type cannot
//!   overlap; store overlapping anchors (comments) as cursors instead.
//!
//! The projection is ProseMirror's JSON: `{type, attrs?, content?}`, and
//! `{type: "text", text, marks?: [{type, attrs?}]}` for text.

use loro::{
    Container, ExpandType, LoroDoc, LoroList, LoroMap, LoroText, LoroValue, StyleConfig,
    StyleConfigMap, ValueOrContainer,
};
use serde_json::{Map as JsonMap, Value};

use crate::{json_to_loro, loro_to_json};

const NODE_NAME: &str = "nodeName";
const ATTRIBUTES: &str = "attributes";
const CHILDREN: &str = "children";

/// The deepest node nesting a document may have. ProseMirror documents,
/// tables and nested lists included, stay far below it. Writes past it are
/// refused; the projection stops there, so a doc a client built deeper
/// cannot exhaust the server's stack.
pub const MAX_DEPTH: usize = 64;
/// The most nodes (text runs included) one write may hold.
pub const MAX_NODES: usize = 200_000;
/// The most characters of text one write may hold.
pub const MAX_TEXT_CHARS: usize = 5_000_000;

/// Replace the document under `key` of `parent` with `value`, a
/// ProseMirror JSON document. An absolute write, like a text field's: the
/// tree is rebuilt in the container already under the key, so an editor
/// bound to that container keeps its binding, but concurrent edits to the
/// old content are lost.
///
/// The whole value is checked first ([`validate`]); a value that fails
/// leaves the document as it was.
///
/// A mark needs a text style before it can be written, and the style's
/// expand rule is recorded with the mark. Styles are not part of the doc;
/// each peer configures its own. The server has no editor schema, so every
/// mark it writes expands "after", ProseMirror's default for a mark that is
/// `inclusive`, which is what `configureProseMirrorMarks` gives an editor
/// for such a mark. A server function that needs exact behavior for
/// non-inclusive marks (a link, a suggestion) writes through
/// `writeProseMirror` in `@pylonsync/loro/prosemirror` with the schema.
pub(crate) fn write(
    doc: &LoroDoc,
    parent: &LoroMap,
    key: &str,
    value: &Value,
) -> Result<(), String> {
    let node = value.as_object().ok_or_else(|| {
        format!("field {key}: expected a ProseMirror document (a JSON object), got {value}")
    })?;
    validate(node).map_err(|e| format!("field {key}: {e}"))?;
    // Loro ships its own styles for a few names ("highlight", "code" and
    // "link" do not expand), which are not ProseMirror's: there a mark is
    // inclusive unless its schema says otherwise. The server has no editor
    // schema, so every mark it writes expands "after".
    doc.config_text_style(StyleConfigMap::new());
    doc.config_default_text_style(Some(StyleConfig {
        expand: ExpandType::After,
    }));
    let root = match parent.get(key) {
        Some(ValueOrContainer::Container(Container::Map(m))) => m,
        _ => parent
            .insert_container(key, LoroMap::new())
            .map_err(|e| format!("field {key}: create the document: {e}"))?,
    };
    fill_node(&root, node).map_err(|e| format!("field {key}: {e}"))
}

/// Create the document under `key` when there is none, with the containers
/// every node keeps its content in (an `attributes` map and a `children`
/// list). Editors then only insert into containers the server made, so
/// concurrent inserts merge. When two editors made them, as they did when the
/// server left an empty map, one copy won and the other's edits went with the
/// losing containers. The top node's name is a plain value, not a container:
/// the first editor writes its schema's (two writing the same name lose
/// nothing), so a schema whose top node is not `doc` works too.
pub(crate) fn ensure(parent: &LoroMap, key: &str) -> Result<(), String> {
    let root = match parent.get(key) {
        Some(ValueOrContainer::Container(Container::Map(m))) => m,
        _ => parent
            .insert_container(key, LoroMap::new())
            .map_err(|e| format!("field {key}: create the document: {e}"))?,
    };
    skeleton(&root).map_err(|e| format!("field {key}: create the document: {e}"))
}

/// The containers every document has.
fn skeleton(root: &LoroMap) -> Result<(), String> {
    child_map(root, ATTRIBUTES)?;
    child_list(root, CHILDREN)?;
    Ok(())
}

/// Empty the document under `key`, keeping its containers: the content and
/// the attributes go, the attributes map and the children list stay.
pub(crate) fn clear(parent: &LoroMap, key: &str) -> Result<(), String> {
    match parent.get(key) {
        Some(ValueOrContainer::Container(Container::Map(m))) => {
            skeleton(&m).map_err(|e| format!("field {key}: clear the document: {e}"))?;
            let attrs = child_map(&m, ATTRIBUTES)?;
            let keys: Vec<String> = attrs.keys().map(|k| k.to_string()).collect();
            for k in keys {
                attrs
                    .delete(&k)
                    .map_err(|e| format!("field {key}: clear the document: {e}"))?;
            }
            child_list(&m, CHILDREN)?
                .clear()
                .map_err(|e| format!("field {key}: clear the document: {e}"))
        }
        _ => ensure(parent, key),
    }
}

/// Check a ProseMirror JSON document before anything is written: every
/// node has a string `type`, `attrs` is an object, `content` an array of
/// nodes, a text node has string `text` and no content, each mark has a
/// `type` (without ':') and object `attrs`, nesting stays within
/// [`MAX_DEPTH`], and the size within [`MAX_NODES`] and [`MAX_TEXT_CHARS`].
pub fn validate(node: &JsonMap<String, Value>) -> Result<(), String> {
    let mut nodes = 0usize;
    let mut chars = 0usize;
    validate_node(node, 1, &mut nodes, &mut chars)
}

fn validate_node(
    node: &JsonMap<String, Value>,
    depth: usize,
    nodes: &mut usize,
    chars: &mut usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("the document nests deeper than {MAX_DEPTH} levels"));
    }
    *nodes += 1;
    if *nodes > MAX_NODES {
        return Err(format!("the document has more than {MAX_NODES} nodes"));
    }
    let name = match node.get("type") {
        Some(Value::String(s)) if !s.is_empty() => s.as_str(),
        _ => return Err("every ProseMirror node needs a \"type\" string".into()),
    };
    match node.get("attrs") {
        None | Some(Value::Null) => {}
        Some(Value::Object(attrs)) => {
            for (k, v) in attrs {
                if !v.is_null() && json_to_loro(v).is_none() {
                    return Err(format!("attr {k} of {name} is too deep"));
                }
            }
        }
        Some(other) => {
            return Err(format!(
                "the attrs of {name} must be an object, got {other}"
            ))
        }
    }
    if name == "text" {
        let text = match node.get("text") {
            Some(Value::String(t)) => t,
            _ => return Err("a text node needs a \"text\" string".into()),
        };
        *chars += text.chars().count();
        if *chars > MAX_TEXT_CHARS {
            return Err(format!(
                "the document has more than {MAX_TEXT_CHARS} characters"
            ));
        }
        if node.get("content").is_some_and(|c| !c.is_null()) {
            return Err("a text node cannot have content".into());
        }
        match node.get("marks") {
            None | Some(Value::Null) => {}
            Some(Value::Array(marks)) => {
                for mark in marks {
                    let mark = mark.as_object().ok_or("every mark must be an object")?;
                    let kind = match mark.get("type") {
                        Some(Value::String(t)) if !t.is_empty() => t,
                        _ => return Err("every mark needs a \"type\" string".into()),
                    };
                    if kind.contains(':') {
                        return Err(format!("mark type {kind} cannot contain ':'"));
                    }
                    match mark.get("attrs") {
                        None | Some(Value::Null) => {}
                        Some(a @ Value::Object(_)) => {
                            if json_to_loro(a).is_none() {
                                return Err(format!("the attrs of mark {kind} are too deep"));
                            }
                        }
                        Some(other) => {
                            return Err(format!(
                                "the attrs of mark {kind} must be an object, got {other}"
                            ))
                        }
                    }
                }
            }
            Some(other) => {
                return Err(format!(
                    "the marks of a text node must be an array, got {other}"
                ))
            }
        }
        return Ok(());
    }
    if node.contains_key("marks")
        && !node["marks"].is_null()
        && !node["marks"].as_array().is_some_and(Vec::is_empty)
    {
        return Err(format!(
            "only text nodes carry marks here; {name} has marks"
        ));
    }
    match node.get("content") {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Array(children)) => {
            for child in children {
                let child = child
                    .as_object()
                    .ok_or_else(|| format!("a child of {name} is not an object"))?;
                validate_node(child, depth + 1, nodes, chars)?;
            }
            Ok(())
        }
        Some(other) => Err(format!(
            "the content of {name} must be an array, got {other}"
        )),
    }
}

fn fill_node(map: &LoroMap, node: &JsonMap<String, Value>) -> Result<(), String> {
    let name = node
        .get("type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("every ProseMirror node needs a \"type\"")?;
    map.insert(NODE_NAME, name).map_err(|e| e.to_string())?;

    let attrs = child_map(map, ATTRIBUTES)?;
    let old: Vec<String> = attrs.keys().map(|k| k.to_string()).collect();
    for k in old {
        attrs.delete(&k).map_err(|e| e.to_string())?;
    }
    if let Some(Value::Object(given)) = node.get("attrs") {
        for (k, v) in given {
            // The binding leaves null attrs out; so does the server.
            if v.is_null() {
                continue;
            }
            let v = json_to_loro(v).ok_or_else(|| format!("attr {k} of {name} is too deep"))?;
            attrs.insert(k, v).map_err(|e| e.to_string())?;
        }
    }

    let children = child_list(map, CHILDREN)?;
    children.clear().map_err(|e| e.to_string())?;
    let mut run: Vec<&JsonMap<String, Value>> = Vec::new();
    if let Some(content) = node.get("content") {
        let content = content
            .as_array()
            .ok_or_else(|| format!("the content of {name} must be an array"))?;
        for child in content {
            let child = child
                .as_object()
                .ok_or_else(|| format!("a child of {name} is not an object"))?;
            if child.get("type").and_then(Value::as_str) == Some("text") {
                run.push(child);
                continue;
            }
            push_text(&children, &run)?;
            run.clear();
            let m = children
                .push_container(LoroMap::new())
                .map_err(|e| e.to_string())?;
            fill_node(&m, child)?;
        }
    }
    push_text(&children, &run)
}

/// Write a run of consecutive text nodes into one text container, as the
/// binding does.
fn push_text(list: &LoroList, run: &[&JsonMap<String, Value>]) -> Result<(), String> {
    // All the text first, then the marks: a style that expands (Loro's
    // default for "bold", say) would otherwise spread onto the next run.
    let joined: String = run
        .iter()
        .filter_map(|n| n.get("text").and_then(Value::as_str))
        .collect();
    if joined.is_empty() {
        return Ok(());
    }
    let text = list
        .push_container(LoroText::new())
        .map_err(|e| e.to_string())?;
    text.insert(0, &joined).map_err(|e| e.to_string())?;
    let mut at = 0usize;
    for node in run {
        let s = node.get("text").and_then(Value::as_str).unwrap_or("");
        let end = at + s.chars().count();
        if let Some(marks) = node
            .get("marks")
            .and_then(Value::as_array)
            .filter(|_| end > at)
        {
            for mark in marks {
                let kind = mark
                    .get("type")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .ok_or("every mark needs a \"type\"")?;
                if kind.contains(':') {
                    return Err(format!("mark type {kind} cannot contain ':'"));
                }
                let attrs = match mark.get("attrs") {
                    Some(Value::Object(a)) => Value::Object(a.clone()),
                    _ => Value::Object(JsonMap::new()),
                };
                let value = json_to_loro(&attrs)
                    .ok_or_else(|| format!("the attrs of mark {kind} are too deep"))?;
                text.mark(at..end, kind, value)
                    .map_err(|e| format!("mark {kind}: {e}"))?;
            }
        }
        at = end;
    }
    Ok(())
}

fn child_map(map: &LoroMap, key: &str) -> Result<LoroMap, String> {
    match map.get(key) {
        Some(ValueOrContainer::Container(Container::Map(m))) => Ok(m),
        _ => map
            .insert_container(key, LoroMap::new())
            .map_err(|e| e.to_string()),
    }
}

fn child_list(map: &LoroMap, key: &str) -> Result<LoroList, String> {
    match map.get(key) {
        Some(ValueOrContainer::Container(Container::List(l))) => Ok(l),
        _ => map
            .insert_container(key, LoroList::new())
            .map_err(|e| e.to_string()),
    }
}

/// The document in a node map as ProseMirror JSON. Null for a map that is
/// not a node (no `nodeName`), such as a container no editor has written.
/// Nodes nested deeper than [`MAX_DEPTH`] are left out, so a doc a client
/// built deeper cannot exhaust the stack.
///
/// A document with no content and no attributes is empty and reads as null.
pub(crate) fn project(map: &LoroMap) -> Value {
    let node = project_node(map, 1);
    match &node {
        Value::Object(o) if o.len() == 1 && o.contains_key("type") => Value::Null,
        _ => node,
    }
}

fn project_node(map: &LoroMap, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return Value::Null;
    }
    let Some(ValueOrContainer::Value(LoroValue::String(name))) = map.get(NODE_NAME) else {
        return Value::Null;
    };
    let mut out = JsonMap::new();
    out.insert("type".into(), Value::String(name.to_string()));
    if let Some(ValueOrContainer::Container(Container::Map(attrs))) = map.get(ATTRIBUTES) {
        if let Some(Value::Object(a)) = loro_to_json(attrs.get_value()) {
            if !a.is_empty() {
                out.insert("attrs".into(), Value::Object(a));
            }
        }
    }
    let mut content = Vec::new();
    if let Some(ValueOrContainer::Container(Container::List(children))) = map.get(CHILDREN) {
        for i in 0..children.len() {
            match children.get(i) {
                Some(ValueOrContainer::Container(Container::Map(child))) => {
                    let node = project_node(&child, depth + 1);
                    if !node.is_null() {
                        content.push(node);
                    }
                }
                Some(ValueOrContainer::Container(Container::Text(text))) => {
                    content.extend(project_text(&text));
                }
                _ => {}
            }
        }
    }
    if !content.is_empty() {
        out.insert("content".into(), Value::Array(content));
    }
    Value::Object(out)
}

/// A text container as text nodes, one per run of equal marks. Marks are
/// sorted by type, so the projection does not depend on map order.
fn project_text(text: &LoroText) -> Vec<Value> {
    let mut nodes = Vec::new();
    for delta in text.to_delta() {
        let loro::TextDelta::Insert { insert, attributes } = delta else {
            continue;
        };
        if insert.is_empty() {
            continue;
        }
        let mut node = JsonMap::new();
        node.insert("type".into(), Value::String("text".into()));
        node.insert("text".into(), Value::String(insert));
        let mut marks: Vec<(String, Value)> = attributes
            .unwrap_or_default()
            .into_iter()
            // A null style is a removed mark.
            .filter(|(_, v)| !matches!(v, LoroValue::Null))
            .map(|(k, v)| {
                let mut mark = JsonMap::new();
                mark.insert("type".into(), Value::String(k.clone()));
                if let Some(Value::Object(a)) = loro_to_json(v) {
                    if !a.is_empty() {
                        mark.insert("attrs".into(), Value::Object(a));
                    }
                }
                (k, Value::Object(mark))
            })
            .collect();
        marks.sort_by(|a, b| a.0.cmp(&b.0));
        if !marks.is_empty() {
            node.insert(
                "marks".into(),
                Value::Array(marks.into_iter().map(|(_, m)| m).collect()),
            );
        }
        nodes.push(Value::Object(node));
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::{ATTRIBUTES, CHILDREN, NODE_NAME};
    use crate::{apply_patch, field_kind, project_doc_to_json, root_map, CrdtField, CrdtFieldKind};
    use loro::{Container, LoroDoc, LoroList, LoroMap, LoroText, ValueOrContainer};
    use serde_json::json;

    fn fields() -> Vec<CrdtField> {
        vec![CrdtField {
            name: "body".into(),
            kind: CrdtFieldKind::ProseMirror,
        }]
    }

    fn sample() -> serde_json::Value {
        json!({
            "type": "doc",
            "content": [
                {"type": "heading", "attrs": {"level": 1}, "content": [{"type": "text", "text": "Launch plan"}]},
                {"type": "paragraph", "content": [
                    {"type": "text", "text": "We ship "},
                    {"type": "text", "text": "Friday", "marks": [{"type": "bold"}, {"type": "suggestionInsert", "attrs": {"id": "s1"}}]},
                    {"type": "text", "text": " after review."}
                ]},
                {"type": "table", "content": [{"type": "tableRow", "content": [
                    {"type": "tableCell", "attrs": {"colspan": 1, "rowspan": 1}, "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "Owner"}]}
                    ]}
                ]}]}
            ]
        })
    }

    #[test]
    fn a_document_round_trips_through_the_doc() {
        let doc = LoroDoc::new();
        apply_patch(&doc, &fields(), &json!({"body": sample()})).unwrap();
        doc.commit();
        assert_eq!(project_doc_to_json(&doc, &fields())["body"], sample());
    }

    #[test]
    fn null_attrs_are_left_out_and_empty_text_is_skipped() {
        let doc = LoroDoc::new();
        let value = json!({"type": "doc", "content": [
            {"type": "paragraph", "attrs": {"align": null}, "content": [{"type": "text", "text": ""}]}
        ]});
        apply_patch(&doc, &fields(), &json!({"body": value})).unwrap();
        assert_eq!(
            project_doc_to_json(&doc, &fields())["body"],
            json!({"type": "doc", "content": [{"type": "paragraph"}]})
        );
    }

    #[test]
    fn a_second_write_replaces_the_content_in_the_same_container() {
        let doc = LoroDoc::new();
        apply_patch(&doc, &fields(), &json!({"body": sample()})).unwrap();
        let before = root_map(&doc).get("body").unwrap();
        let next = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "New"}]}]});
        apply_patch(&doc, &fields(), &json!({"body": next.clone()})).unwrap();
        let after = root_map(&doc).get("body").unwrap();
        // An editor bound to the container keeps its binding.
        assert_eq!(format!("{before:?}"), format!("{after:?}"));
        assert_eq!(project_doc_to_json(&doc, &fields())["body"], next);
    }

    #[test]
    fn null_removes_the_document_and_bad_values_are_refused() {
        let doc = LoroDoc::new();
        apply_patch(&doc, &fields(), &json!({"body": sample()})).unwrap();
        apply_patch(&doc, &fields(), &json!({"body": null})).unwrap();
        assert_eq!(project_doc_to_json(&doc, &fields())["body"], json!(null));
        let err = apply_patch(&doc, &fields(), &json!({"body": "text"})).unwrap_err();
        assert!(err.contains("expected a ProseMirror document"), "{err}");
        let err = apply_patch(&doc, &fields(), &json!({"body": {"content": []}})).unwrap_err();
        assert!(err.contains("needs a \"type\""), "{err}");
    }

    /// A snapshot the JavaScript `loro-prosemirror` binding wrote at
    /// `row.body` (crates/crdt/tests/fixtures/prosemirror-binding.bin). The
    /// server must read what the browser editor writes.
    #[test]
    fn a_document_written_by_the_js_binding_projects() {
        let doc = LoroDoc::new();
        doc.import(include_bytes!("../tests/fixtures/prosemirror-binding.bin"))
            .unwrap();
        let body = &project_doc_to_json(&doc, &fields())["body"];
        assert_eq!(
            *body,
            json!({"type": "doc", "content": [
                {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Plan"}]},
                {"type": "paragraph", "content": [
                    {"type": "text", "text": "Ship "},
                    {"type": "text", "text": "Friday", "marks": [{"type": "bold"}]},
                    {"type": "text", "text": " "},
                    {"type": "text", "text": "draft", "marks": [{"type": "suggestionInsert", "attrs": {"id": "s1"}}]},
                    {"type": "text", "text": " café"}
                ]},
                {"type": "table", "content": [{"type": "tableRow", "content": [
                    {"type": "tableCell", "attrs": {"colspan": 1, "rowspan": 1}, "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "A1"}]}
                    ]}
                ]}]}
            ]})
        );
        // And a server write over it keeps working on the binding's tree.
        let next = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Rewritten"}]}]});
        apply_patch(&doc, &fields(), &json!({"body": next.clone()})).unwrap();
        assert_eq!(project_doc_to_json(&doc, &fields())["body"], next);
    }

    #[test]
    fn the_annotation_is_for_richtext_and_json_only() {
        use pylon_kernel::CrdtAnnotation;
        let a = Some(CrdtAnnotation::ProseMirror);
        assert_eq!(field_kind("richtext", a), Ok(CrdtFieldKind::ProseMirror));
        assert_eq!(field_kind("json", a), Ok(CrdtFieldKind::ProseMirror));
        assert!(field_kind("string", a).unwrap_err().contains("prosemirror"));
        assert!(field_kind("int", a).is_err());
    }

    /// A document nested `levels` deep. Built by hand: `json!` copies an
    /// interpolated value, which makes a deep build quadratic.
    fn deep_json(levels: usize) -> serde_json::Value {
        let mut node = serde_json::Value::Object(
            [("type".to_string(), json!("paragraph"))]
                .into_iter()
                .collect(),
        );
        for _ in 0..levels {
            let mut m = serde_json::Map::new();
            m.insert("type".into(), json!("blockquote"));
            m.insert("content".into(), serde_json::Value::Array(vec![node]));
            node = serde_json::Value::Object(m);
        }
        let mut doc = serde_json::Map::new();
        doc.insert("type".into(), json!("doc"));
        doc.insert("content".into(), serde_json::Value::Array(vec![node]));
        serde_json::Value::Object(doc)
    }

    #[test]
    fn a_write_nested_past_the_limit_is_refused() {
        // Building and dropping a 10,000-deep serde_json value recurses in
        // serde_json itself, so the test runs on a large stack. (JSON that
        // arrives over HTTP is parsed with serde_json's 128-level limit.)
        // The check under test stops at MAX_DEPTH whatever the stack.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(nested_write_is_refused)
            .unwrap()
            .join()
            .unwrap();
    }

    fn nested_write_is_refused() {
        let doc = LoroDoc::new();
        apply_patch(&doc, &fields(), &json!({"body": sample()})).unwrap();
        // 10,000 levels: the check must refuse it without exhausting the stack.
        let mut patch = serde_json::Map::new();
        patch.insert("body".into(), deep_json(10_000));
        let err = apply_patch(&doc, &fields(), &serde_json::Value::Object(patch)).unwrap_err();
        assert!(err.contains("deeper than"), "{err}");
        assert_eq!(project_doc_to_json(&doc, &fields())["body"], sample());
        // Within the limit is fine.
        apply_patch(
            &doc,
            &fields(),
            &json!({"body": deep_json(super::MAX_DEPTH - 3)}),
        )
        .unwrap();
    }

    /// A client update with a 10,000-deep chain of node maps under
    /// `row.body` (tests/fixtures/deep-chain.bin.gz, written once: building
    /// the chain through Loro's API takes minutes in a debug build). The
    /// client made 1,000 other ops first, so its write of `row.body`
    /// outranks a server's earlier one. The runtime's tests read it too.
    fn deep_chain_update() -> Vec<u8> {
        use std::io::Read;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&include_bytes!("../tests/fixtures/deep-chain.bin.gz")[..])
            .read_to_end(&mut out)
            .unwrap();
        out
    }

    /// A client can send any Loro update. The chain must project without
    /// exhausting the stack; nodes past the limit are left out.
    #[test]
    fn a_doc_nested_past_the_limit_projects_without_overflow() {
        let update = deep_chain_update();
        // Projected on a thread with a small stack, as a server worker might be.
        let projected = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || {
                let server = LoroDoc::new();
                server.import(&update).unwrap();
                project_doc_to_json(&server, &fields())["body"].clone()
            })
            .unwrap()
            .join()
            .expect("projection overflowed the stack");
        let mut depth = 0;
        let mut at = &projected;
        while let Some(child) = at.get("content").and_then(|c| c.get(0)) {
            depth += 1;
            at = child;
        }
        assert_eq!(depth, super::MAX_DEPTH - 1);
    }

    /// The expand rule is recorded with a mark. A mark the server writes
    /// must behave like an inclusive mark the JS binding writes: text typed
    /// right after it takes the mark.
    #[test]
    fn server_marks_expand_like_the_bindings_inclusive_marks() {
        fn mark_after_typing(doc: &LoroDoc, word_end: usize) -> serde_json::Value {
            let Some(loro::ValueOrContainer::Container(loro::Container::Map(body))) =
                root_map(doc).get("body")
            else {
                panic!("no body");
            };
            let Some(loro::ValueOrContainer::Container(loro::Container::List(blocks))) =
                body.get("children")
            else {
                panic!("no children");
            };
            let Some(loro::ValueOrContainer::Container(loro::Container::Map(para))) = blocks.get(1)
            else {
                panic!("no paragraph");
            };
            let Some(loro::ValueOrContainer::Container(loro::Container::List(runs))) =
                para.get("children")
            else {
                panic!("no runs");
            };
            let Some(loro::ValueOrContainer::Container(loro::Container::Text(text))) = runs.get(0)
            else {
                panic!("no text");
            };
            text.insert(word_end, "X").unwrap();
            doc.commit();
            let projected = project_doc_to_json(doc, &fields())["body"].clone();
            projected["content"][1]["content"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["text"].as_str().is_some_and(|t| t.contains('X')))
                .unwrap()
                .clone()
        }
        // The binding's fixture: "Ship Friday ..." with Friday in bold
        // (inclusive, configured "after" by the binding).
        let js = LoroDoc::new();
        js.import(include_bytes!("../tests/fixtures/prosemirror-binding.bin"))
            .unwrap();
        let js_node = mark_after_typing(&js, "Ship Friday".chars().count());
        assert_eq!(js_node["marks"][0]["type"], "bold", "{js_node}");

        // The same shape written by the server, with "highlight", which
        // Loro's own styles would not expand.
        let server = LoroDoc::new();
        let value = json!({"type": "doc", "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Plan"}]},
            {"type": "paragraph", "content": [
                {"type": "text", "text": "Ship "},
                {"type": "text", "text": "Friday", "marks": [{"type": "highlight"}]},
                {"type": "text", "text": " later"}
            ]}
        ]});
        apply_patch(&server, &fields(), &json!({"body": value})).unwrap();
        server.commit();
        let server_node = mark_after_typing(&server, "Ship Friday".chars().count());
        assert_eq!(
            server_node["marks"][0]["type"],
            "highlight",
            "{server_node} in {}",
            project_doc_to_json(&server, &fields())["body"]
        );
        assert_eq!(server_node["text"], js_node["text"]);
    }

    #[test]
    fn a_bad_write_leaves_the_document_unchanged() {
        let doc = LoroDoc::new();
        apply_patch(&doc, &fields(), &json!({"body": sample()})).unwrap();
        let bad = [
            (
                json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text"}]}]}),
                "\"text\" string",
            ),
            (
                json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": 5}]}]}),
                "\"text\" string",
            ),
            (
                json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a", "marks": {"type": "bold"}}]}]}),
                "must be an array",
            ),
            (
                json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a", "marks": [{"attrs": {}}]}]}]}),
                "needs a \"type\"",
            ),
            (
                json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a", "marks": [{"type": "link", "attrs": "x"}]}]}]}),
                "must be an object",
            ),
            (
                json!({"type": "doc", "content": [{"type": "heading", "attrs": [1]}]}),
                "must be an object",
            ),
            (
                json!({"type": "doc", "content": "text"}),
                "must be an array",
            ),
            (
                json!({"type": "doc", "content": [{"type": "paragraph"}, 3]}),
                "is not an object",
            ),
            (
                json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}, {"type": "paragraph", "content": [{"type": "text", "text": "x".repeat(super::MAX_TEXT_CHARS)}]}]}),
                "characters",
            ),
        ];
        for (value, expected) in bad {
            let err = apply_patch(&doc, &fields(), &json!({"body": value})).unwrap_err();
            assert!(err.contains(expected), "{err} (expected {expected})");
            assert_eq!(project_doc_to_json(&doc, &fields())["body"], sample());
        }
    }

    #[test]
    fn every_patch_leaves_the_container_in_place() {
        // A row inserted without a document still gets the container, so a
        // client never has to create it.
        let doc = LoroDoc::new();
        apply_patch(&doc, &fields(), &json!({})).unwrap();
        let first = format!("{:?}", root_map(&doc).get("body").expect("container"));
        assert_eq!(project_doc_to_json(&doc, &fields())["body"], json!(null));
        apply_patch(&doc, &fields(), &json!({"body": sample()})).unwrap();
        apply_patch(&doc, &fields(), &json!({"body": null})).unwrap();
        assert_eq!(format!("{:?}", root_map(&doc).get("body").unwrap()), first);
        assert_eq!(project_doc_to_json(&doc, &fields())["body"], json!(null));
    }

    fn body_map(doc: &LoroDoc) -> LoroMap {
        match root_map(doc).get("body") {
            Some(ValueOrContainer::Container(Container::Map(m))) => m,
            other => panic!("no body map: {other:?}"),
        }
    }

    #[test]
    fn an_empty_document_has_its_containers_and_reads_as_null() {
        // Editors insert into these; they never create them.
        let doc = LoroDoc::new();
        apply_patch(&doc, &fields(), &json!({})).unwrap();
        let body = body_map(&doc);
        // The top node's name is the first editor's to write.
        assert!(body.get(NODE_NAME).is_none());
        assert!(matches!(
            body.get(ATTRIBUTES),
            Some(ValueOrContainer::Container(Container::Map(_)))
        ));
        let children = match body.get(CHILDREN) {
            Some(ValueOrContainer::Container(Container::List(l))) => l,
            other => panic!("no children list: {other:?}"),
        };
        assert_eq!(project_doc_to_json(&doc, &fields())["body"], json!(null));

        // Null empties the document and keeps the same children list.
        apply_patch(&doc, &fields(), &json!({"body": sample()})).unwrap();
        apply_patch(&doc, &fields(), &json!({"body": null})).unwrap();
        let after = match body_map(&doc).get(CHILDREN) {
            Some(ValueOrContainer::Container(Container::List(l))) => l,
            other => panic!("no children list: {other:?}"),
        };
        assert_eq!(
            loro::ContainerTrait::id(&after),
            loro::ContainerTrait::id(&children)
        );
        assert_eq!(after.len(), 0);
        assert_eq!(project_doc_to_json(&doc, &fields())["body"], json!(null));
    }

    #[test]
    fn two_editors_starting_an_empty_document_keep_both_paragraphs() {
        // The server's empty document, copied to two peers that each add a
        // paragraph before they see the other's.
        let server = LoroDoc::new();
        apply_patch(&server, &fields(), &json!({})).unwrap();
        server.commit();
        let snapshot = server.export(loro::ExportMode::Snapshot).unwrap();
        let peers: Vec<LoroDoc> = (0..2)
            .map(|_| {
                let d = LoroDoc::new();
                d.import(&snapshot).unwrap();
                d
            })
            .collect();
        for (i, peer) in peers.iter().enumerate() {
            let children = match body_map(peer).get(CHILDREN) {
                Some(ValueOrContainer::Container(Container::List(l))) => l,
                other => panic!("no children list: {other:?}"),
            };
            body_map(peer).insert(NODE_NAME, "doc").unwrap();
            let para = children.insert_container(0, LoroMap::new()).unwrap();
            para.insert(NODE_NAME, "paragraph").unwrap();
            let runs = para.insert_container(CHILDREN, LoroList::new()).unwrap();
            let text = runs.insert_container(0, LoroText::new()).unwrap();
            text.insert(0, &format!("from peer {i}")).unwrap();
            peer.commit();
        }
        for peer in &peers {
            server
                .import(&peer.export(loro::ExportMode::all_updates()).unwrap())
                .unwrap();
        }
        let body = project_doc_to_json(&server, &fields())["body"].to_string();
        assert!(
            body.contains("from peer 0") && body.contains("from peer 1"),
            "{body}"
        );
    }
}
