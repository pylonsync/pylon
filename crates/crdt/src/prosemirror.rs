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
    ValueOrContainer,
};
use serde_json::{Map as JsonMap, Value};

use crate::{json_to_loro, loro_to_json};

const NODE_NAME: &str = "nodeName";
const ATTRIBUTES: &str = "attributes";
const CHILDREN: &str = "children";

/// Replace the document under `key` of `parent` with `value`, a
/// ProseMirror JSON document. An absolute write, like a text field's: the
/// tree is rebuilt in the container already under the key, so an editor
/// bound to that container keeps its binding, but concurrent edits to the
/// old content are lost.
///
/// A mark needs a text style before it can be written. Styles are not part
/// of the doc; each peer configures its own. The server marks with
/// "do not expand" for any mark type it has no style for. Clients set
/// theirs from the editor schema (`configureProseMirrorMarks` in
/// `@pylonsync/loro/prosemirror`).
pub(crate) fn write(
    doc: &LoroDoc,
    parent: &LoroMap,
    key: &str,
    value: &Value,
) -> Result<(), String> {
    let node = value.as_object().ok_or_else(|| {
        format!("field {key}: expected a ProseMirror document (a JSON object), got {value}")
    })?;
    doc.config_default_text_style(Some(StyleConfig {
        expand: ExpandType::None,
    }));
    let root = match parent.get(key) {
        Some(ValueOrContainer::Container(Container::Map(m))) => m,
        _ => parent
            .insert_container(key, LoroMap::new())
            .map_err(|e| format!("field {key}: create the document: {e}"))?,
    };
    fill_node(&root, node).map_err(|e| format!("field {key}: {e}"))
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
pub(crate) fn project(map: &LoroMap) -> Value {
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
                    let node = project(&child);
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
    use crate::{apply_patch, field_kind, project_doc_to_json, root_map, CrdtField, CrdtFieldKind};
    use loro::LoroDoc;
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
}
