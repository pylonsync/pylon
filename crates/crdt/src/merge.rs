//! Merging a field's other containers into the one holding its key.
//!
//! Two writers that each create a field's container (a client that edits
//! an empty doc offline, and the server's seed) leave two containers under
//! one key of the root map. The key holds one; the other's content is not
//! in the projection. After a client's update is imported, the server
//! merges each such container the update changed, and the one the key
//! moved away from, into the key's container ("the holder"), with ops of
//! its own:
//!
//! - The first merge of a source compares both sides with the server's last
//!   whole write of the field ([`BaseWrite`]). A source that still holds
//!   exactly that write adds nothing. A holder that still holds exactly
//!   that write takes the source's value. Otherwise the source's content is
//!   added and nothing is deleted: the source's author may not have seen
//!   the holder's content.
//! - Text and lists: each element of the source is linked to an element of
//!   the holder, by Loro id ([`Links`]). The first merge links the two by a
//!   diff of their values (text: runs of at least [`MIN_TEXT_RUN`]
//!   characters, unless the holder takes the source's value), and inserts
//!   the source's other elements beside the images of their neighbours. A
//!   later merge deletes the images of the source's elements deleted since,
//!   inserts its new elements, and (a movable list) sets the images of the
//!   ones whose value changed.
//! - Trees: nodes are matched by their `id`. A later merge compares the
//!   source with its nodes at the previous merge.
//! - Counters: increments to any other counter of the field are added to
//!   the holder, and a displaced holder adds its total.
//!
//! A movable list's element moved in the holder since a merge gets a new
//! position id, which the links do not follow: a later delete or set of
//! it from the source does not reach it. A move in the source reaches the
//! holder as a delete and an insert.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use loro::cursor::Side;
use loro::{
    ContainerID, ContainerType, IdSpan, JsonMapOp, JsonOpContent, LoroDoc, LoroValue,
    ValueOrContainer, VersionVector, ID,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    apply_patch, json_to_loro, loro_to_json, project_tree, root_map, CrdtField, CrdtFieldKind,
    ROOT_MAP,
};

/// How long the first merge's diff of two values may take before it
/// settles for a coarser match.
const ALIGN_DEADLINE: Duration = Duration::from_millis(200);

/// The shortest run of equal characters the first merge links, when it adds
/// a source's text to a holder: a shorter one matches by chance.
const MIN_TEXT_RUN: usize = 4;

/// Sources one field merges per push at most.
const MAX_SOURCES: usize = 16;

/// Container chains followed at most (a field's key moved from container
/// to container).
const MAX_CHAIN: usize = 32;

/// The server's last whole write of a text, list, or tree field: the
/// container it wrote, the op span of the write, and how many elements (or
/// tree nodes) it wrote. The elements the write inserted there have ids in
/// that span. For a movable list, the items it wrote (a `set` changes an
/// item and keeps its id). For a tree, the nodes it wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseWrite {
    pub container: String,
    pub peer: u64,
    pub start: i32,
    pub end: i32,
    pub len: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Value>,
}

impl BaseWrite {
    fn wrote(&self, id: ID) -> bool {
        id.peer == self.peer && id.counter >= self.start && id.counter < self.end
    }
}

/// The server's write of `field` by the ops of `peer` from `start` to
/// `end` (exclusive), when the key holds a text, list, or tree.
pub fn base_write(
    doc: &LoroDoc,
    field: &CrdtField,
    peer: u64,
    start: i32,
    end: i32,
) -> Option<BaseWrite> {
    use CrdtFieldKind as K;
    if !matches!(field.kind, K::Text | K::List | K::MovableList | K::Tree) {
        return None;
    }
    let container = holders(doc, std::slice::from_ref(field)).remove(&field.name)?;
    let (len, values, nodes) = match field.kind {
        K::Tree => {
            let nodes = project_tree(&doc.get_tree(container.clone()));
            (nodes.as_array().map_or(0, Vec::len), None, Some(nodes))
        }
        K::MovableList => {
            let values: Vec<Value> = read_elements(doc, &container)
                .into_iter()
                .map(|e| e.value)
                .collect();
            (values.len(), Some(values), None)
        }
        _ => (Seq::of(doc, &container)?.len(), None, None),
    };
    Some(BaseWrite {
        container: container.to_string(),
        peer,
        start,
        end,
        len,
        values,
        nodes,
    })
}

/// A source container's links into the holder, as of its last merge.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Links {
    /// The container the links point into.
    pub target: String,
    /// Text and lists: each source element (in the source's order) and its
    /// image in `target`, none for an element the target's side deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<LinkRun>,
    /// Movable list: each linked element's value at the last merge, in the
    /// order of `runs`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Value>,
    /// Tree: the source's nodes at the last merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Value>,
}

/// `len` links: source ids `s.1..` of peer `s.0` to image ids `t.1..` of
/// peer `t.0` (or none).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkRun {
    pub s: (u64, i32),
    pub t: Option<(u64, i32)>,
    pub len: u32,
}

fn expand(runs: &[LinkRun]) -> Vec<(ID, Option<ID>)> {
    runs.iter()
        .flat_map(|r| {
            (0..r.len as i32).map(move |k| {
                (
                    ID::new(r.s.0, r.s.1 + k),
                    r.t.map(|(p, c)| ID::new(p, c + k)),
                )
            })
        })
        .collect()
}

fn compress(links: &[(ID, Option<ID>)]) -> Vec<LinkRun> {
    let mut runs: Vec<LinkRun> = Vec::new();
    for &(s, t) in links {
        if let Some(last) = runs.last_mut() {
            let k = last.len as i32;
            let s_next = last.s.0 == s.peer && last.s.1 + k == s.counter;
            let t_next = match (last.t, t) {
                (None, None) => true,
                (Some((p, c)), Some(t)) => p == t.peer && c + k == t.counter,
                _ => false,
            };
            if s_next && t_next {
                last.len += 1;
                continue;
            }
        }
        runs.push(LinkRun {
            s: (s.peer, s.counter),
            t: t.map(|t| (t.peer, t.counter)),
            len: 1,
        });
    }
    runs
}

/// The container under each field's key.
pub fn holders(doc: &LoroDoc, fields: &[CrdtField]) -> HashMap<String, ContainerID> {
    use loro::ContainerTrait;
    let map = root_map(doc);
    fields
        .iter()
        .filter_map(|f| match map.get(&f.name) {
            Some(ValueOrContainer::Container(c)) => Some((f.name.clone(), c.id())),
            _ => None,
        })
        .collect()
}

/// What an import added, read from its ops.
#[derive(Debug, Default)]
pub struct Imported {
    /// Fields whose key, or whose containers, the ops changed.
    pub touched: Vec<String>,
    /// Register fields the ops wrote, with the value the last of those ops
    /// left (null for a delete): the op Loro's map keeps among them.
    pub registers: HashMap<String, Value>,
    /// Counter increments, per container.
    pub increments: HashMap<ContainerID, f64>,
    /// Per field, the containers the ops changed (a tree's node metadata
    /// counts as its tree).
    pub changed: HashMap<String, Vec<ContainerID>>,
}

/// Read what the ops `doc` holds past `before` did.
pub fn read_import(doc: &LoroDoc, fields: &[CrdtField], before: &VersionVector) -> Imported {
    use CrdtFieldKind as K;
    let root = ContainerID::new_root(ROOT_MAP, ContainerType::Map);
    let kinds: HashMap<&str, CrdtFieldKind> =
        fields.iter().map(|f| (f.name.as_str(), f.kind)).collect();
    let mut touched: Vec<String> = Vec::new();
    let mut last_set: HashMap<String, ((u32, u64), Value)> = HashMap::new();
    let mut created: HashMap<ContainerID, String> = HashMap::new();
    let mut increments: HashMap<ContainerID, f64> = HashMap::new();
    let mut containers: Vec<ContainerID> = Vec::new();
    let mut seen: HashSet<ContainerID> = HashSet::new();
    for (&peer, &end) in doc.oplog_vv().iter() {
        let start = before.get(&peer).copied().unwrap_or(0);
        if end <= start {
            continue;
        }
        for change in doc.export_json_in_id_span(IdSpan::new(peer, start, end)) {
            for op in &change.ops {
                if op.container == root {
                    let JsonOpContent::Map(m) = &op.content else {
                        continue;
                    };
                    let (key, value) = match m {
                        JsonMapOp::Insert { key, value } => (key, Some(value)),
                        JsonMapOp::Delete { key } => (key, None),
                    };
                    let Some(&kind) = kinds.get(key.as_str()) else {
                        continue;
                    };
                    if !touched.contains(key) {
                        touched.push(key.clone());
                    }
                    if let Some(LoroValue::Container(cid)) = value {
                        created.insert(cid.clone(), key.clone());
                        continue;
                    }
                    if !matches!(kind, K::LwwString | K::LwwNumber | K::LwwBool | K::LwwJson) {
                        continue;
                    }
                    let value = value
                        .and_then(|v| loro_to_json(v.clone()))
                        .unwrap_or(Value::Null);
                    let lamport = change.lamport + (op.counter - change.id.counter) as u32;
                    let rank = (lamport, peer);
                    if last_set.get(key).is_none_or(|(r, _)| *r < rank) {
                        last_set.insert(key.clone(), (rank, value));
                    }
                    continue;
                }
                if let JsonOpContent::Future(wrapper) = &op.content {
                    if let Some(n) = counter_increment(&wrapper.value) {
                        *increments.entry(op.container.clone()).or_default() += n;
                    }
                }
                if seen.insert(op.container.clone()) {
                    containers.push(op.container.clone());
                }
            }
        }
    }
    // A container the ops created holds the value they wrote, even an empty
    // one (no ops of its own).
    for cid in created.keys() {
        if seen.insert(cid.clone()) {
            containers.push(cid.clone());
        }
    }
    let held: HashMap<ContainerID, String> = holders(doc, fields)
        .into_iter()
        .map(|(field, cid)| (cid, field))
        .collect();
    let mut changed: HashMap<String, Vec<ContainerID>> = HashMap::new();
    for cid in containers {
        let Some((field, top)) = field_of(doc, &cid, &created, &held, &kinds) else {
            continue;
        };
        if !touched.contains(&field) {
            touched.push(field.clone());
        }
        let list = changed.entry(field).or_default();
        if !list.contains(&top) {
            list.push(top);
        }
    }
    Imported {
        touched,
        registers: last_set.into_iter().map(|(k, (_, v))| (k, v)).collect(),
        increments,
        changed,
    }
}

/// A counter op's increment. Loro keeps the op's value type private; its
/// serde form is `{"type": "counter", "value_type": "f64", "value": n}`.
fn counter_increment(op: &loro::JsonFutureOp) -> Option<f64> {
    let v = serde_json::to_value(op).ok()?;
    if v.get("type")?.as_str()? != "counter" {
        return None;
    }
    v.get("value")?.as_f64()
}

/// The field a container belongs to, and the container under the field's
/// key it is part of (a tree node's metadata map: its tree). `created` and
/// `held` map containers the import created, and the ones the keys hold, to
/// their fields; any other container is found through the op that created
/// it.
fn field_of(
    doc: &LoroDoc,
    cid: &ContainerID,
    created: &HashMap<ContainerID, String>,
    held: &HashMap<ContainerID, String>,
    kinds: &HashMap<&str, CrdtFieldKind>,
) -> Option<(String, ContainerID)> {
    let root = ContainerID::new_root(ROOT_MAP, ContainerType::Map);
    let mut cid = cid.clone();
    for _ in 0..MAX_CHAIN {
        if let Some(field) = created.get(&cid).or_else(|| held.get(&cid)) {
            return Some((field.clone(), cid));
        }
        let ContainerID::Normal { peer, counter, .. } = &cid else {
            return None;
        };
        let (peer, counter) = (*peer, *counter);
        let changes = doc.export_json_in_id_span(IdSpan::new(peer, counter, counter + 1));
        let op = changes
            .iter()
            .flat_map(|c| &c.ops)
            .find(|op| op.counter <= counter && counter < op.counter + op.content.op_len() as i32)?
            .clone();
        match &op.content {
            JsonOpContent::Map(JsonMapOp::Insert {
                key,
                value: LoroValue::Container(child),
            }) if op.container == root && child == &cid => {
                return kinds.contains_key(key.as_str()).then(|| (key.clone(), cid));
            }
            // A tree node's metadata map shares its node's id: the op
            // that created the node, in its tree.
            JsonOpContent::Tree(_) => cid = op.container.clone(),
            _ => return None,
        }
    }
    None
}

/// The container type a field's kind keeps under its key; none for a
/// register.
fn container_type(kind: CrdtFieldKind) -> Option<ContainerType> {
    use CrdtFieldKind as K;
    match kind {
        K::Text => Some(ContainerType::Text),
        K::List => Some(ContainerType::List),
        K::MovableList => Some(ContainerType::MovableList),
        K::Tree => Some(ContainerType::Tree),
        K::Counter => Some(ContainerType::Counter),
        K::LwwString | K::LwwNumber | K::LwwBool | K::LwwJson => None,
    }
}

/// Whether [`merge_after_import`] has anything to merge: a field whose key
/// moved to another container of its kind, or another container of its
/// kind the import changed.
pub fn has_sources(
    doc: &LoroDoc,
    fields: &[CrdtField],
    imported: &Imported,
    prior: &HashMap<String, ContainerID>,
) -> bool {
    let now = holders(doc, fields);
    fields.iter().any(|f| {
        let (Some(kind), Some(holder)) = (container_type(f.kind), now.get(&f.name)) else {
            return false;
        };
        holder.container_type() == kind
            && prior
                .get(&f.name)
                .into_iter()
                .chain(imported.changed.get(&f.name).into_iter().flatten())
                .any(|c| c != holder && c.container_type() == kind)
    })
}

/// After an import: merge, per text, list, tree, or counter field, the
/// containers of its kind the import changed, and the container the key
/// moved from, into the one holding the key. `prior` is the holders before
/// the import; `links` is the row's links, updated in place. Returns the
/// sources whose links changed or were dropped.
pub fn merge_after_import(
    doc: &LoroDoc,
    fields: &[CrdtField],
    imported: &Imported,
    prior: &HashMap<String, ContainerID>,
    bases: &HashMap<String, BaseWrite>,
    links: &mut HashMap<String, Links>,
) -> Result<Vec<String>, String> {
    use CrdtFieldKind as K;
    let now = holders(doc, fields);
    let mut updated = Vec::new();
    for f in fields {
        let (Some(kind), Some(holder)) = (container_type(f.kind), now.get(&f.name)) else {
            continue;
        };
        // A key holding another type of container (a client can write any)
        // is left as Loro merged it; a source of another type is skipped.
        if holder.container_type() != kind {
            continue;
        }
        let displaced = prior
            .get(&f.name)
            .filter(|p| *p != holder && p.container_type() == kind);
        let changed: Vec<&ContainerID> = imported
            .changed
            .get(&f.name)
            .into_iter()
            .flatten()
            .filter(|c| *c != holder && c.container_type() == kind)
            .collect();
        if f.kind == K::Counter {
            let mut add: f64 = changed
                .iter()
                .filter(|c| Some(**c) != displaced)
                .filter_map(|c| imported.increments.get(*c))
                .sum();
            if let Some(p) = displaced {
                add += counter_value(doc, p);
            }
            if add != 0.0 && add.is_finite() {
                doc.get_counter(holder.clone())
                    .increment(add)
                    .map_err(|e| format!("counter {}: {e}", f.name))?;
            }
            continue;
        }
        // The displaced container first: links into it from earlier merges
        // go on through its own links.
        let mut sources: Vec<&ContainerID> = displaced.into_iter().collect();
        for c in changed {
            if !sources.contains(&c) {
                sources.push(c);
            }
        }
        if sources.len() > MAX_SOURCES {
            tracing::warn!(
                field = %f.name,
                sources = sources.len(),
                "an update changed more containers of one field than a push merges; \
                 the rest merge when they change again"
            );
            sources.truncate(MAX_SOURCES);
        }
        for source in sources {
            let key = source.to_string();
            let earlier = links
                .get(&key)
                .cloned()
                .and_then(|l| translate(l, holder, links));
            let base = bases.get(&f.name);
            let merged = if f.kind == K::Tree {
                merge_tree(doc, f, source, holder, earlier, base)
            } else {
                // Only a first merge compares with the base write.
                let (in_source, in_holder) = if earlier.is_none() {
                    (base_in(base, source, links), base_in(base, holder, links))
                } else {
                    (BaseIn::Nothing, BaseIn::Nothing)
                };
                merge_seq(doc, source, holder, earlier, base, &in_source, &in_holder)
            };
            match merged {
                Ok(merged) => {
                    links.insert(key.clone(), merged);
                }
                // What a failed merge inserted is not linked: dropping the
                // source's links makes the next merge a first merge, whose
                // diff links it.
                Err(e) => {
                    tracing::warn!(
                        field = %f.name,
                        source = %key,
                        "merge a container into the one holding its key: {e}"
                    );
                    links.remove(&key);
                }
            }
            updated.push(key);
        }
    }
    doc.commit();
    Ok(updated)
}

/// A counter's value, 0 for a container the doc does not hold or that is
/// not a counter (Loro asserts both).
pub fn counter_value(doc: &LoroDoc, id: &ContainerID) -> f64 {
    if id.container_type() == ContainerType::Counter && doc.has_container(id) {
        doc.get_counter(id.clone()).get_value()
    } else {
        0.0
    }
}

/// `links` pointed into a container that was itself merged on: follow its
/// links on to `holder`. None when the chain does not reach it.
fn translate(
    mut links: Links,
    holder: &ContainerID,
    all: &HashMap<String, Links>,
) -> Option<Links> {
    let holder = holder.to_string();
    for _ in 0..MAX_CHAIN {
        if links.target == holder {
            return Some(links);
        }
        let next = all.get(&links.target)?;
        if links.nodes.is_none() {
            let onward: HashMap<ID, Option<ID>> = expand(&next.runs).into_iter().collect();
            let moved: Vec<(ID, Option<ID>)> = expand(&links.runs)
                .into_iter()
                .map(|(s, t)| (s, t.and_then(|t| onward.get(&t).copied().flatten())))
                .collect();
            links.runs = compress(&moved);
        }
        links.target = next.target.clone();
    }
    None
}

/// Which elements of a container came from the base write: in the container
/// it wrote, those in its span; in a container that one was merged into
/// (through any chain), their images.
enum BaseIn {
    Nothing,
    Span { peer: u64, start: i32, end: i32 },
    Images(HashSet<ID>),
}

impl BaseIn {
    fn contains(&self, id: ID) -> bool {
        match self {
            BaseIn::Nothing => false,
            BaseIn::Span { peer, start, end } => {
                id.peer == *peer && id.counter >= *start && id.counter < *end
            }
            BaseIn::Images(ids) => ids.contains(&id),
        }
    }
}

fn base_in(
    base: Option<&BaseWrite>,
    container: &ContainerID,
    all: &HashMap<String, Links>,
) -> BaseIn {
    let Some(base) = base else {
        return BaseIn::Nothing;
    };
    if base.container == container.to_string() {
        return BaseIn::Span {
            peer: base.peer,
            start: base.start,
            end: base.end,
        };
    }
    let Some(links) = all
        .get(&base.container)
        .cloned()
        .and_then(|l| translate(l, container, all))
    else {
        return BaseIn::Nothing;
    };
    BaseIn::Images(
        expand(&links.runs)
            .into_iter()
            .filter(|(s, _)| base.wrote(*s))
            .filter_map(|(_, t)| t)
            .collect(),
    )
}

/// One element of a text or list.
struct Elem {
    id: ID,
    value: Value,
}

fn read_elements(doc: &LoroDoc, cid: &ContainerID) -> Vec<Elem> {
    match cid.container_type() {
        ContainerType::Text => {
            let text = doc.get_text(cid.clone());
            text.to_string()
                .chars()
                .enumerate()
                .filter_map(|(i, ch)| {
                    Some(Elem {
                        id: text.get_cursor(i, Side::Middle)?.id?,
                        value: Value::String(ch.to_string()),
                    })
                })
                .collect()
        }
        ContainerType::List => {
            let list = doc.get_list(cid.clone());
            (0..list.len())
                .filter_map(|i| {
                    Some(Elem {
                        id: list.get_cursor(i, Side::Middle)?.id?,
                        value: item_json(list.get(i)),
                    })
                })
                .collect()
        }
        ContainerType::MovableList => {
            let list = doc.get_movable_list(cid.clone());
            (0..list.len())
                .filter_map(|i| {
                    Some(Elem {
                        id: list.get_cursor(i, Side::Middle)?.id?,
                        value: item_json(list.get(i)),
                    })
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn item_json(item: Option<ValueOrContainer>) -> Value {
    match item {
        Some(ValueOrContainer::Value(v)) => loro_to_json(v).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// A text or list the merge edits.
enum Seq {
    Text(loro::LoroText),
    List(loro::LoroList),
    Movable(loro::LoroMovableList),
}

impl Seq {
    fn of(doc: &LoroDoc, cid: &ContainerID) -> Option<Seq> {
        match cid.container_type() {
            ContainerType::Text => Some(Seq::Text(doc.get_text(cid.clone()))),
            ContainerType::List => Some(Seq::List(doc.get_list(cid.clone()))),
            ContainerType::MovableList => Some(Seq::Movable(doc.get_movable_list(cid.clone()))),
            _ => None,
        }
    }

    fn len(&self) -> usize {
        match self {
            Seq::Text(t) => t.len_unicode(),
            Seq::List(l) => l.len(),
            Seq::Movable(l) => l.len(),
        }
    }

    fn id_at(&self, pos: usize) -> Option<ID> {
        match self {
            Seq::Text(t) => t.get_cursor(pos, Side::Middle)?.id,
            Seq::List(l) => l.get_cursor(pos, Side::Middle)?.id,
            Seq::Movable(l) => l.get_cursor(pos, Side::Middle)?.id,
        }
    }

    fn delete(&self, pos: usize) -> Result<(), String> {
        match self {
            Seq::Text(t) => t.delete(pos, 1),
            Seq::List(l) => l.delete(pos, 1),
            Seq::Movable(l) => l.delete(pos, 1),
        }
        .map_err(|e| format!("delete at {pos}: {e}"))
    }

    /// Insert `values` (one-character strings for a text) at `pos`; returns
    /// the new elements' ids.
    fn insert(&self, pos: usize, values: &[Value]) -> Result<Vec<ID>, String> {
        let item =
            |v: &Value| json_to_loro(v).ok_or_else(|| format!("an item a list cannot hold: {v}"));
        match self {
            Seq::Text(t) => {
                let s: String = values.iter().filter_map(Value::as_str).collect();
                t.insert(pos, &s)
                    .map_err(|e| format!("insert at {pos}: {e}"))?;
            }
            Seq::List(l) => {
                for (k, v) in values.iter().enumerate() {
                    l.insert(pos + k, item(v)?)
                        .map_err(|e| format!("insert at {}: {e}", pos + k))?;
                }
            }
            Seq::Movable(l) => {
                for (k, v) in values.iter().enumerate() {
                    l.insert(pos + k, item(v)?)
                        .map_err(|e| format!("insert at {}: {e}", pos + k))?;
                }
            }
        }
        (0..values.len())
            .map(|k| {
                self.id_at(pos + k)
                    .ok_or_else(|| format!("no element at {}", pos + k))
            })
            .collect()
    }

    fn set(&self, pos: usize, value: &Value) -> Result<(), String> {
        if let Seq::Movable(l) = self {
            let v = json_to_loro(value)
                .ok_or_else(|| format!("an item a list cannot hold: {value}"))?;
            l.set(pos, v).map_err(|e| format!("set at {pos}: {e}"))?;
        }
        Ok(())
    }
}

/// Pairs of equal elements of `a` and `b` (by index), in runs of at least
/// `min_run`. A text is diffed by words, then by characters within the
/// spans that differ, so a long text with many changes still links; a list
/// item by item.
fn align(a: &[Elem], b: &[Elem], text: bool, min_run: usize) -> Vec<(usize, usize)> {
    let deadline = Instant::now() + ALIGN_DEADLINE;
    let mut out = Vec::new();
    if text {
        let ca: Vec<&str> = a.iter().map(|e| e.value.as_str().unwrap_or("")).collect();
        let cb: Vec<&str> = b.iter().map(|e| e.value.as_str().unwrap_or("")).collect();
        align_text(
            &ca,
            0..ca.len(),
            &cb,
            0..cb.len(),
            Level::Word,
            deadline,
            min_run,
            &mut out,
        );
    } else {
        let ka: Vec<String> = a.iter().map(|e| e.value.to_string()).collect();
        let kb: Vec<String> = b.iter().map(|e| e.value.to_string()).collect();
        for (oa, ob, len) in equal_runs(similar::Algorithm::Myers, &ka, &kb, deadline) {
            if len >= min_run {
                out.extend((0..len).map(|k| (oa + k, ob + k)));
            }
        }
    }
    out
}

/// Equal runs `(a index, b index, len)` of two token sequences. Patience
/// anchors on tokens found once on each side (words); Myers, for
/// characters, finds the shortest edit.
fn equal_runs(
    algorithm: similar::Algorithm,
    a: &[String],
    b: &[String],
    deadline: Instant,
) -> Vec<(usize, usize, usize)> {
    similar::capture_diff_slices_deadline(algorithm, a, b, Some(deadline))
        .into_iter()
        .filter_map(|op| match op {
            similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => Some((old_index, new_index, len)),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Word,
    Char,
}

/// The largest span, in characters per side, diffed character by
/// character.
const MAX_CHAR_SPAN: usize = 4096;

/// Split `chars[range]` into tokens (as char ranges): words (a run of
/// letters and digits, or one other character) or characters.
fn tokens(
    chars: &[&str],
    range: std::ops::Range<usize>,
    level: Level,
) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = range.start;
    for i in range.clone() {
        let ends = match level {
            Level::Word => {
                let word = |c: &str| c.chars().all(char::is_alphanumeric);
                !word(chars[i]) || i + 1 == range.end || !word(chars[i + 1])
            }
            Level::Char => true,
        };
        if ends {
            out.push(start..i + 1);
            start = i + 1;
        }
    }
    if start < range.end {
        out.push(start..range.end);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn align_text(
    a: &[&str],
    ar: std::ops::Range<usize>,
    b: &[&str],
    br: std::ops::Range<usize>,
    level: Level,
    deadline: Instant,
    min_run: usize,
    out: &mut Vec<(usize, usize)>,
) {
    let mut emit = |oa: usize, ob: usize, len: usize| {
        if len >= min_run {
            out.extend((0..len).map(|k| (oa + k, ob + k)));
        }
    };
    // The common start and end, exactly.
    let head = a[ar.clone()]
        .iter()
        .zip(&b[br.clone()])
        .take_while(|(x, y)| x == y)
        .count();
    let (ar, br) = (ar.start + head..ar.end, br.start + head..br.end);
    let tail = a[ar.clone()]
        .iter()
        .rev()
        .zip(b[br.clone()].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    emit(ar.start - head, br.start - head, head);
    emit(ar.end - tail, br.end - tail, tail);
    let (ar, br) = (ar.start..ar.end - tail, br.start..br.end - tail);
    if ar.is_empty() || br.is_empty() || Instant::now() > deadline {
        return;
    }
    if level == Level::Char {
        if ar.len() > MAX_CHAR_SPAN || br.len() > MAX_CHAR_SPAN {
            return;
        }
        let ka: Vec<String> = a[ar.clone()].iter().map(|c| c.to_string()).collect();
        let kb: Vec<String> = b[br.clone()].iter().map(|c| c.to_string()).collect();
        for (oa, ob, len) in equal_runs(similar::Algorithm::Myers, &ka, &kb, deadline) {
            emit(ar.start + oa, br.start + ob, len);
        }
        return;
    }
    let ta = tokens(a, ar.clone(), level);
    let tb = tokens(b, br.clone(), level);
    let key = |chars: &[&str], t: &std::ops::Range<usize>| chars[t.clone()].concat();
    let ka: Vec<String> = ta.iter().map(|t| key(a, t)).collect();
    let kb: Vec<String> = tb.iter().map(|t| key(b, t)).collect();
    let next = Level::Char;
    // Equal token runs link; the spans between them are diffed finer.
    let (mut ia, mut ib) = (0, 0);
    let runs = equal_runs(similar::Algorithm::Patience, &ka, &kb, deadline);
    for (oa, ob, len) in runs
        .into_iter()
        .chain(std::iter::once((ta.len(), tb.len(), 0)))
    {
        let gap_a = ta.get(ia).map_or(ar.end, |t| t.start)..ta.get(oa).map_or(ar.end, |t| t.start);
        let gap_b = tb.get(ib).map_or(br.end, |t| t.start)..tb.get(ob).map_or(br.end, |t| t.start);
        if !gap_a.is_empty() && !gap_b.is_empty() {
            align_text(a, gap_a, b, gap_b, next, deadline, min_run, out);
        }
        if len > 0 {
            let start_a = ta[oa].start;
            let start_b = tb[ob].start;
            let chars = ta[oa + len - 1].end - start_a;
            if chars >= min_run {
                out.extend((0..chars).map(|k| (start_a + k, start_b + k)));
            }
        }
        (ia, ib) = (oa + len, ob + len);
    }
}

/// Whether `elems` are exactly the base write's elements: as many as it
/// wrote, all from it, and for a movable list with the values it wrote.
fn is_base(elems: &[Elem], base: Option<&BaseWrite>, ids: &BaseIn) -> bool {
    let Some(base) = base else {
        return false;
    };
    base.len == elems.len()
        && elems.iter().all(|e| ids.contains(e.id))
        && base
            .values
            .as_ref()
            .is_none_or(|values| elems.iter().map(|e| &e.value).eq(values.iter()))
}

/// An edit to the holder, at a position in the holder as read before any.
enum Edit {
    Delete(usize),
    Set(usize, Value),
    /// Insert the source's elements `i..j` before position `at`.
    Insert {
        at: usize,
        i: usize,
        j: usize,
    },
}

impl Edit {
    /// Edits run from the highest key down, so an edit leaves the
    /// positions of the ones still to run as they were. A delete or set of
    /// the element at `p` runs before an insert before `p`; inserts before
    /// one position run last source run first.
    fn key(&self) -> (usize, usize) {
        match self {
            Edit::Delete(p) | Edit::Set(p, _) => (2 * p + 1, 0),
            Edit::Insert { at, i, .. } => (2 * at, *i),
        }
    }
}

/// Merge the text or list `source` into `holder` (see the module docs).
fn merge_seq(
    doc: &LoroDoc,
    source: &ContainerID,
    holder: &ContainerID,
    earlier: Option<Links>,
    base: Option<&BaseWrite>,
    base_in_source: &BaseIn,
    base_in_holder: &BaseIn,
) -> Result<Links, String> {
    let seq = Seq::of(doc, holder).ok_or_else(|| format!("{holder} is not a text or list"))?;
    let movable = matches!(seq, Seq::Movable(_));
    let src = read_elements(doc, source);
    let dst = read_elements(doc, holder);
    // Where each element of the holder is; an image not here was deleted.
    let at: HashMap<ID, usize> = dst.iter().enumerate().map(|(p, e)| (e.id, p)).collect();
    let mut link: HashMap<ID, Option<ID>> = HashMap::new();
    let mut edits: Vec<Edit> = Vec::new();
    match earlier {
        Some(earlier) => {
            let alive: HashSet<ID> = src.iter().map(|e| e.id).collect();
            let expanded = expand(&earlier.runs);
            let was: HashMap<ID, Value> = if movable {
                expanded
                    .iter()
                    .map(|(s, _)| *s)
                    .zip(earlier.values)
                    .collect()
            } else {
                HashMap::new()
            };
            for (s, t) in expanded {
                if alive.contains(&s) {
                    link.insert(s, t);
                } else if let Some(&p) = t.as_ref().and_then(|t| at.get(t)) {
                    edits.push(Edit::Delete(p));
                }
            }
            for e in src.iter().filter(|_| movable) {
                let (Some(Some(t)), Some(was)) = (link.get(&e.id), was.get(&e.id)) else {
                    continue;
                };
                if was != &e.value {
                    if let Some(&p) = at.get(t) {
                        edits.push(Edit::Set(p, e.value.clone()));
                    }
                }
            }
        }
        None if is_base(&src, base, base_in_source) => {
            for e in &src {
                link.insert(e.id, None);
            }
        }
        None => {
            let replace = is_base(&dst, base, base_in_holder);
            let text = matches!(seq, Seq::Text(_));
            let min_run = if replace || !text { 1 } else { MIN_TEXT_RUN };
            let mut in_holder = vec![false; dst.len()];
            for (i, j) in align(&src, &dst, text, min_run) {
                link.insert(src[i].id, Some(dst[j].id));
                in_holder[j] = true;
            }
            if replace {
                edits.extend(
                    in_holder
                        .iter()
                        .enumerate()
                        .filter(|(_, linked)| !**linked)
                        .map(|(p, _)| Edit::Delete(p)),
                );
            }
        }
    }
    // The source's unlinked elements, a run at a time, beside the images of
    // their nearest neighbours still in the holder: after the nearest
    // earlier one, else before the nearest later one, else at the end.
    let image: Vec<Option<usize>> = src
        .iter()
        .map(|e| {
            link.get(&e.id)
                .copied()
                .flatten()
                .and_then(|t| at.get(&t))
                .copied()
        })
        .collect();
    // Per index k: the image of the nearest source element before k, and
    // from k on.
    let mut before = vec![None; src.len() + 1];
    for k in 0..src.len() {
        before[k + 1] = image[k].or(before[k]);
    }
    let mut from = vec![None; src.len() + 1];
    for k in (0..src.len()).rev() {
        from[k] = image[k].or(from[k + 1]);
    }
    let mut i = 0;
    while i < src.len() {
        if link.contains_key(&src[i].id) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < src.len() && !link.contains_key(&src[j].id) {
            j += 1;
        }
        let at = before[i].map(|p| p + 1).or(from[j]).unwrap_or(dst.len());
        edits.push(Edit::Insert { at, i, j });
        i = j;
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.key()));
    for edit in edits {
        match edit {
            Edit::Delete(p) => seq.delete(p)?,
            Edit::Set(p, value) => seq.set(p, &value)?,
            Edit::Insert { at, i, j } => {
                let values: Vec<Value> = src[i..j].iter().map(|e| e.value.clone()).collect();
                let ids = seq.insert(at, &values)?;
                for (e, id) in src[i..j].iter().zip(ids) {
                    link.insert(e.id, Some(id));
                }
            }
        }
    }
    let pairs: Vec<(ID, Option<ID>)> = src
        .iter()
        .map(|e| (e.id, link.get(&e.id).copied().flatten()))
        .collect();
    Ok(Links {
        target: holder.to_string(),
        runs: compress(&pairs),
        values: if movable {
            src.into_iter().map(|e| e.value).collect()
        } else {
            Vec::new()
        },
        nodes: None,
    })
}

fn node_id(node: &Value) -> Option<&str> {
    node.get("id")?.as_str()
}

fn node_parent(node: &Value) -> Option<&str> {
    node.get("parent")?.as_str()
}

fn nodes_of(nodes: Option<&Value>) -> Vec<Value> {
    nodes.and_then(Value::as_array).cloned().unwrap_or_default()
}

fn by_id(nodes: &[Value]) -> HashMap<String, Value> {
    nodes
        .iter()
        .filter_map(|n| Some((node_id(n)?.to_string(), n.clone())))
        .collect()
}

/// Merge the tree `source` into `holder` by node id (see the module docs).
fn merge_tree(
    doc: &LoroDoc,
    field: &CrdtField,
    source: &ContainerID,
    holder: &ContainerID,
    earlier: Option<Links>,
    base: Option<&BaseWrite>,
) -> Result<Links, String> {
    let src_value = project_tree(&doc.get_tree(source.clone()));
    let src = nodes_of(Some(&src_value));
    let src_map = by_id(&src);
    let dst = nodes_of(Some(&project_tree(&doc.get_tree(holder.clone()))));
    let dst_map = by_id(&dst);
    let (changed, removed): (Vec<Value>, HashSet<String>) = match earlier.and_then(|l| l.nodes) {
        Some(last) => {
            let last = by_id(&nodes_of(Some(&last)));
            let changed = src
                .iter()
                .filter(|n| node_id(n).is_some_and(|id| last.get(id) != Some(*n)))
                .cloned()
                .collect();
            let removed = last
                .keys()
                .filter(|id| !src_map.contains_key(*id))
                .cloned()
                .collect();
            (changed, removed)
        }
        None => {
            let base_nodes = nodes_of(base.and_then(|b| b.nodes.as_ref()));
            let base_map = by_id(&base_nodes);
            let same = |a: &HashMap<String, Value>| base.is_some() && *a == base_map;
            if same(&src_map) {
                (Vec::new(), HashSet::new())
            } else if same(&dst_map) {
                // The holder takes the source's nodes.
                let removed = dst_map
                    .keys()
                    .filter(|id| !src_map.contains_key(*id))
                    .cloned()
                    .collect();
                (src.clone(), removed)
            } else {
                // The source's nodes that differ from the base write
                // are added; nothing is removed.
                let changed = src
                    .iter()
                    .filter(|n| node_id(n).is_some_and(|id| base_map.get(id) != Some(*n)))
                    .cloned()
                    .collect();
                (changed, HashSet::new())
            }
        }
    };
    let mut target = dst.clone();
    // A removed node takes its descendants with it, as in Loro.
    let mut gone = removed;
    loop {
        let more: Vec<String> = target
            .iter()
            .filter(|n| {
                node_id(n).is_some_and(|id| !gone.contains(id))
                    && node_parent(n).is_some_and(|p| gone.contains(p))
            })
            .filter_map(|n| node_id(n).map(str::to_string))
            .collect();
        if more.is_empty() {
            break;
        }
        gone.extend(more);
    }
    target.retain(|n| node_id(n).is_none_or(|id| !gone.contains(id)));
    for n in changed {
        match target.iter_mut().find(|m| node_id(m) == node_id(&n)) {
            Some(m) => *m = n,
            None => target.push(n),
        }
    }
    // A node whose parent the target lacks: the parent comes from the
    // source when it has it, else the node becomes a root.
    loop {
        let ids: HashSet<String> = target
            .iter()
            .filter_map(|n| node_id(n).map(str::to_string))
            .collect();
        let missing: Vec<String> = target
            .iter()
            .filter_map(|n| node_parent(n).map(str::to_string))
            .filter(|p| !ids.contains(p))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if missing.is_empty() {
            break;
        }
        for p in missing {
            match src_map.get(&p) {
                Some(parent) => target.push(parent.clone()),
                None => {
                    for n in target
                        .iter_mut()
                        .filter(|n| node_parent(n) == Some(p.as_str()))
                    {
                        n["parent"] = Value::Null;
                    }
                }
            }
        }
    }
    break_cycles(&mut target);
    if by_id(&target) != dst_map {
        apply_patch(
            doc,
            std::slice::from_ref(field),
            &serde_json::json!({ &field.name: target }),
        )?;
    }
    Ok(Links {
        target: holder.to_string(),
        runs: Vec::new(),
        values: Vec::new(),
        nodes: Some(src_value),
    })
}

/// Two sides' moves can make a cycle (each put a node under the other): a
/// node on one becomes a root.
fn break_cycles(nodes: &mut [Value]) {
    loop {
        let parent: HashMap<String, String> = nodes
            .iter()
            .filter_map(|n| Some((node_id(n)?.to_string(), node_parent(n)?.to_string())))
            .collect();
        let on_cycle = parent.keys().find(|start| {
            let mut seen = HashSet::new();
            let mut at = start.as_str();
            while let Some(p) = parent.get(at) {
                if p == *start || !seen.insert(p.as_str()) {
                    return p == *start;
                }
                at = p;
            }
            false
        });
        let Some(id) = on_cycle.cloned() else {
            return;
        };
        for n in nodes.iter_mut().filter(|n| node_id(n) == Some(id.as_str())) {
            n["parent"] = Value::Null;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elems(s: &str) -> Vec<Elem> {
        s.chars()
            .enumerate()
            .map(|(i, c)| Elem {
                id: ID::new(1, i as i32),
                value: Value::String(c.to_string()),
            })
            .collect()
    }

    /// Every line of a long text changed: the words still link.
    #[test]
    fn align_links_a_long_text_whose_every_line_changed() {
        let line = |i: usize, tag: &str| format!("line {i} of the document {tag}\n");
        let a: String = (0..2000).map(|i| line(i, "edited")).collect();
        let b: String = (0..2000).map(|i| line(i, "")).collect();
        let started = Instant::now();
        let pairs = align(&elems(&a), &elems(&b), true, MIN_TEXT_RUN);
        let took = started.elapsed();
        assert!(
            pairs.len() >= b.chars().count() - 2000,
            "{} of {} in {took:?}",
            pairs.len(),
            b.chars().count()
        );
    }
}
