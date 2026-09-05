//! Typed `hara:values@0.1.0` Component Model boundary.
//!
//! WIT does not permit recursive value aliases, so the public value is a
//! checked post-order graph (`root`, `nodes`). This module owns that one
//! representation; it never falls back to JSON or an HTA byte frame.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use num_bigint::BigInt;
use wasmtime::component::{Type as ComponentType, Val as ComponentVal};

use crate::core::{
    ExceptionInfo, ExceptionProvenance, ExceptionSite, StructType, StructValue, Value,
};
use crate::lang::data::{
    Cons, Keyword, MapEntry, Metadata, MetadataValue, Pointer, Symbol, TaggedLiteral, Trie,
};
use crate::lang::protocol::IMetadata;

type NodeEntry = (u32, u32);

#[derive(Debug, Clone, Copy)]
enum SequenceKind {
    List,
    Vector,
    Cons,
    Deque,
    Queue,
    Set,
    OrderedSet,
    SortedSet,
}

#[derive(Debug, Clone, Copy)]
enum MappingKind {
    Map,
    OrderedMap,
    SortedMap,
    PriorityMap,
    Trie,
}

#[derive(Debug, Clone)]
enum GraphNode {
    Nil,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Character(char),
    BigInteger(String),
    Regex(String),
    Text(String),
    Bytes(Vec<u8>),
    Keyword(String),
    Symbol {
        text: String,
        metadata: Option<Vec<NodeEntry>>,
    },
    Sequence {
        kind: SequenceKind,
        values: Vec<u32>,
        metadata: Option<Vec<NodeEntry>>,
    },
    Mapping {
        kind: MappingKind,
        entries: Vec<NodeEntry>,
        metadata: Option<Vec<NodeEntry>>,
    },
    Tagged {
        tag: String,
        form: u32,
    },
    MapEntry {
        key: u32,
        value: u32,
        metadata: Option<Vec<NodeEntry>>,
    },
    Pointer {
        context: String,
        fields: Vec<NodeEntry>,
        metadata: Option<Vec<NodeEntry>>,
    },
    Structure {
        name: String,
        fields: Vec<String>,
        values: Vec<u32>,
        metadata: Option<Vec<NodeEntry>>,
    },
    Exception {
        message: String,
        data: u32,
        cause: Option<u32>,
        created_at: Option<ExceptionSite>,
        throws: Vec<ExceptionSite>,
    },
}

impl GraphNode {
    fn children(&self) -> Vec<u32> {
        let metadata_children = |metadata: &Option<Vec<NodeEntry>>| {
            metadata
                .iter()
                .flatten()
                .flat_map(|(key, value)| [*key, *value])
                .collect::<Vec<_>>()
        };
        match self {
            Self::Nil
            | Self::Boolean(_)
            | Self::Integer(_)
            | Self::Float(_)
            | Self::Character(_)
            | Self::BigInteger(_)
            | Self::Regex(_)
            | Self::Text(_)
            | Self::Bytes(_)
            | Self::Keyword(_) => Vec::new(),
            Self::Symbol { metadata, .. } => metadata_children(metadata),
            Self::Sequence {
                values, metadata, ..
            } => {
                let mut children = values.clone();
                children.extend(metadata_children(metadata));
                children
            }
            Self::Mapping {
                entries, metadata, ..
            }
            | Self::Pointer {
                fields: entries,
                metadata,
                ..
            } => {
                let mut children = entries
                    .iter()
                    .flat_map(|(key, value)| [*key, *value])
                    .collect::<Vec<_>>();
                children.extend(metadata_children(metadata));
                children
            }
            Self::Tagged { form, .. } => vec![*form],
            Self::MapEntry {
                key,
                value,
                metadata,
            } => {
                let mut children = vec![*key, *value];
                children.extend(metadata_children(metadata));
                children
            }
            Self::Structure {
                values, metadata, ..
            } => {
                let mut children = values.clone();
                children.extend(metadata_children(metadata));
                children
            }
            Self::Exception { data, cause, .. } => {
                let mut children = vec![*data];
                children.extend(cause.iter().copied());
                children
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Graph {
    root: u32,
    nodes: Vec<GraphNode>,
}

struct GraphBuilder {
    nodes: Vec<GraphNode>,
}

impl GraphBuilder {
    fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    fn finish(self, root: u32) -> Graph {
        Graph {
            root,
            nodes: self.nodes,
        }
    }

    fn push(&mut self, node: GraphNode) -> Result<u32, String> {
        let index = u32::try_from(self.nodes.len())
            .map_err(|_| "extension/value-limit: too many graph nodes".to_owned())?;
        self.nodes.push(node);
        Ok(index)
    }

    fn append_metadata(
        &mut self,
        metadata: Option<&Metadata>,
    ) -> Result<Option<Vec<NodeEntry>>, String> {
        metadata
            .map(|metadata| {
                metadata
                    .entries()
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            self.append_metadata_value(key)?,
                            self.append_metadata_value(value)?,
                        ))
                    })
                    .collect()
            })
            .transpose()
    }

    fn append_metadata_value(&mut self, value: &MetadataValue) -> Result<u32, String> {
        match value {
            MetadataValue::Nil => self.push(GraphNode::Nil),
            MetadataValue::Boolean(value) => self.push(GraphNode::Boolean(*value)),
            MetadataValue::Number(value) => self.push(GraphNode::Integer(*value)),
            MetadataValue::Float(value) if value.is_finite() => self.push(GraphNode::Float(*value)),
            MetadataValue::Float(_) => Err("extension/value-unsupported: non-finite-float".into()),
            MetadataValue::BigInteger(value) => self.push(GraphNode::BigInteger(value.to_string())),
            MetadataValue::Character(value) => self.push(GraphNode::Character(*value)),
            MetadataValue::Regex(value) => self.push(GraphNode::Regex(value.clone())),
            MetadataValue::Tagged(tag, form) => {
                let form = self.append_metadata_value(form)?;
                self.push(GraphNode::Tagged {
                    tag: tag.clone(),
                    form,
                })
            }
            MetadataValue::String(value) => self.push(GraphNode::Text(value.clone())),
            MetadataValue::Keyword(value) => self.push(GraphNode::Keyword(value.as_str().into())),
            MetadataValue::Symbol(value) => {
                let metadata = self.append_metadata(value.meta().map(Rc::as_ref))?;
                self.push(GraphNode::Symbol {
                    text: value.as_str().into(),
                    metadata,
                })
            }
            MetadataValue::Vector(values) => {
                let values = values
                    .iter()
                    .map(|value| self.append_metadata_value(value))
                    .collect::<Result<Vec<_>, _>>()?;
                self.push(GraphNode::Sequence {
                    kind: SequenceKind::Vector,
                    values,
                    metadata: None,
                })
            }
            MetadataValue::List(values) => {
                let values = values
                    .iter()
                    .map(|value| self.append_metadata_value(value))
                    .collect::<Result<Vec<_>, _>>()?;
                self.push(GraphNode::Sequence {
                    kind: SequenceKind::List,
                    values,
                    metadata: None,
                })
            }
            MetadataValue::Set(values) => {
                let values = values
                    .iter()
                    .map(|value| self.append_metadata_value(value))
                    .collect::<Result<Vec<_>, _>>()?;
                self.push(GraphNode::Sequence {
                    kind: SequenceKind::Set,
                    values,
                    metadata: None,
                })
            }
            MetadataValue::Map(entries) => {
                let entries = entries
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            self.append_metadata_value(key)?,
                            self.append_metadata_value(value)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                self.push(GraphNode::Mapping {
                    kind: MappingKind::Map,
                    entries,
                    metadata: None,
                })
            }
        }
    }

    fn append(&mut self, value: &Value) -> Result<u32, String> {
        match value {
            Value::Nil => self.push(GraphNode::Nil),
            Value::Bool(value) => self.push(GraphNode::Boolean(*value)),
            Value::Number(value) => self.push(GraphNode::Integer(*value)),
            Value::Float(value) if value.is_finite() => self.push(GraphNode::Float(*value)),
            Value::Float(_) => Err("extension/value-unsupported: non-finite-float".into()),
            Value::BigInteger(value) => self.push(GraphNode::BigInteger(value.to_string())),
            Value::Character(value) => self.push(GraphNode::Character(*value)),
            Value::Regex(value) => self.push(GraphNode::Regex(value.clone())),
            Value::String(value) => self.push(GraphNode::Text(value.clone())),
            Value::Bytes(value) => self.push(GraphNode::Bytes(value.clone())),
            Value::Keyword(value) => self.push(GraphNode::Keyword(value.as_str().into())),
            Value::Symbol(value) => {
                let metadata = self.append_metadata(value.meta().map(Rc::as_ref))?;
                self.push(GraphNode::Symbol {
                    text: value.as_str().into(),
                    metadata,
                })
            }
            Value::Tagged(value) => {
                let form = self.append(value.form())?;
                self.push(GraphNode::Tagged {
                    tag: value.tag().as_str().into(),
                    form,
                })
            }
            Value::Map(values) => self.append_mapping(
                MappingKind::Map,
                values.iter().map(|(key, value)| (key, value)),
                values.meta().map(Rc::as_ref),
            ),
            Value::OrderedMap(values) => self.append_mapping(
                MappingKind::OrderedMap,
                values.iter().map(|(key, value)| (key, value)),
                values.meta().map(Rc::as_ref),
            ),
            Value::SortedMap(values) => self.append_mapping(
                MappingKind::SortedMap,
                values.iter().map(|(key, value)| (key, value)),
                values.meta().map(Rc::as_ref),
            ),
            Value::PriorityMap(values) => {
                let entries = values
                    .iter()
                    .map(|(key, value)| Ok((self.append(&key)?, self.append(&value)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                let metadata = self.append_metadata(values.meta().map(Rc::as_ref))?;
                self.push(GraphNode::Mapping {
                    kind: MappingKind::PriorityMap,
                    entries,
                    metadata,
                })
            }
            Value::Trie(values) => {
                let entries = values
                    .entries()
                    .iter()
                    .map(|(key, value)| {
                        let key = Value::String(key.clone());
                        Ok((self.append(&key)?, self.append(*value)?))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let metadata = self.append_metadata(values.meta().map(Rc::as_ref))?;
                self.push(GraphNode::Mapping {
                    kind: MappingKind::Trie,
                    entries,
                    metadata,
                })
            }
            Value::List(values) => self.append_sequence(
                SequenceKind::List,
                values.iter(),
                values.meta().map(Rc::as_ref),
            ),
            Value::Cons(values) => {
                let elements = values
                    .iter()
                    .map(|value| self.append(&value))
                    .collect::<Result<Vec<_>, _>>()?;
                let metadata = self.append_metadata(values.meta().map(Rc::as_ref))?;
                self.push(GraphNode::Sequence {
                    kind: SequenceKind::Cons,
                    values: elements,
                    metadata,
                })
            }
            Value::Deque(values) => self.append_sequence(
                SequenceKind::Deque,
                values.iter(),
                values.meta().map(Rc::as_ref),
            ),
            Value::Queue(values) => self.append_sequence(
                SequenceKind::Queue,
                values.iter(),
                values.meta().map(Rc::as_ref),
            ),
            Value::Vector(values) => self.append_sequence(
                SequenceKind::Vector,
                values.iter(),
                values.meta().map(Rc::as_ref),
            ),
            // Tuple is Hara's compact internal vector representation. The
            // Component boundary intentionally exposes its public vector kind.
            Value::Tuple(values) => self.append_sequence(
                SequenceKind::Vector,
                values.iter(),
                values.meta().map(Rc::as_ref),
            ),
            Value::Set(values) => self.append_sequence(
                SequenceKind::Set,
                values.iter(),
                values.meta().map(Rc::as_ref),
            ),
            Value::OrderedSet(values) => self.append_sequence(
                SequenceKind::OrderedSet,
                values.iter(),
                values.meta().map(Rc::as_ref),
            ),
            Value::SortedSet(values) => self.append_sequence(
                SequenceKind::SortedSet,
                values.iter(),
                values.meta().map(Rc::as_ref),
            ),
            Value::MapEntry(value) => {
                let key = self.append(value.key())?;
                let value_index = self.append(value.value())?;
                let metadata = self.append_metadata(value.meta().map(Rc::as_ref))?;
                self.push(GraphNode::MapEntry {
                    key,
                    value: value_index,
                    metadata,
                })
            }
            Value::Pointer(value) => {
                let fields =
                    self.append_pairs(value.fields().iter().map(|(key, value)| (key, value)))?;
                let metadata = self.append_metadata(value.meta().map(Rc::as_ref))?;
                self.push(GraphNode::Pointer {
                    context: value.context().as_str().into(),
                    fields,
                    metadata,
                })
            }
            Value::Struct(value) => {
                let values = value
                    .ordered_values()
                    .into_iter()
                    .map(|value| self.append(value))
                    .collect::<Result<Vec<_>, _>>()?;
                let metadata = self.append_metadata(value.metadata.as_deref())?;
                self.push(GraphNode::Structure {
                    name: value.ty.name.clone(),
                    fields: value.ty.fields.clone(),
                    values,
                    metadata,
                })
            }
            Value::ExceptionInfo(value) => {
                let data = self.append(value.data.as_ref())?;
                let cause = value
                    .cause
                    .as_deref()
                    .map(|value| self.append(value))
                    .transpose()?;
                let provenance = value.provenance.borrow();
                self.push(GraphNode::Exception {
                    message: value.message.clone(),
                    data,
                    cause,
                    created_at: provenance.created_at.clone(),
                    throws: provenance.throws.clone(),
                })
            }
            Value::ByteBuffer(_) => Err("extension/value-unsupported: byte-buffer".into()),
            Value::Array(_) | Value::Object(_) => {
                Err("extension/value-unsupported: mutable-object".into())
            }
            Value::Promise(_) => Err("extension/value-unsupported: promise".into()),
            Value::Atom(_) => Err("extension/value-unsupported: atom".into()),
            Value::Recur(_) => Err("extension/value-unsupported: recur".into()),
            Value::Function(_) => Err("extension/value-unsupported: function".into()),
            Value::MutableCollection(_) => {
                Err("extension/value-unsupported: mutable-collection".into())
            }
            Value::Seq(_) | Value::Iterator(_) => {
                Err("extension/value-unsupported: lazy-sequence".into())
            }
            Value::Var(_) => Err("extension/value-unsupported: var".into()),
            Value::Namespace(_) => Err("extension/value-unsupported: namespace".into()),
            Value::Extension(_) => Err("extension/value-unsupported: extension".into()),
            Value::StructType(_) | Value::MutableType(_) | Value::NativeType(_) => {
                Err("extension/value-unsupported: type".into())
            }
            Value::Mutable(_) => Err("extension/value-unsupported: mutable".into()),
            Value::Protocol(_) => Err("extension/value-unsupported: protocol".into()),
            Value::Schema(_) => Err("extension/value-unsupported: schema".into()),
            Value::Coroutine(_) | Value::Stream(_) => {
                Err("extension/value-unsupported: stream".into())
            }
            Value::Result(_) => Err("extension/value-unsupported: result".into()),
        }
    }

    fn append_sequence<'a>(
        &mut self,
        kind: SequenceKind,
        values: impl Iterator<Item = &'a Value>,
        metadata: Option<&Metadata>,
    ) -> Result<u32, String> {
        let values = values
            .map(|value| self.append(value))
            .collect::<Result<Vec<_>, _>>()?;
        let metadata = self.append_metadata(metadata)?;
        self.push(GraphNode::Sequence {
            kind,
            values,
            metadata,
        })
    }

    fn append_mapping<'a>(
        &mut self,
        kind: MappingKind,
        entries: impl Iterator<Item = (&'a Value, &'a Value)>,
        metadata: Option<&Metadata>,
    ) -> Result<u32, String> {
        let entries = self.append_pairs(entries)?;
        let metadata = self.append_metadata(metadata)?;
        self.push(GraphNode::Mapping {
            kind,
            entries,
            metadata,
        })
    }

    fn append_pairs<'a>(
        &mut self,
        entries: impl Iterator<Item = (&'a Value, &'a Value)>,
    ) -> Result<Vec<NodeEntry>, String> {
        entries
            .map(|(key, value)| Ok((self.append(key)?, self.append(value)?)))
            .collect()
    }
}

/// Lowers a supported Hara value to the exact `hara:values` record expected
/// by the Component function parameter.
pub fn lower(ty: &ComponentType, value: &Value) -> Result<ComponentVal, String> {
    let mut builder = GraphBuilder::new();
    let root = builder.append(value)?;
    lower_graph(ty, &builder.finish(root))
}

/// Lifts a `hara:values` Component result, validates its graph invariants, and
/// reconstructs the original Hara persistent value category.
pub fn lift(value: ComponentVal) -> Result<Value, String> {
    let graph = parse_graph(&value)?;
    graph.validate()?;
    graph.materialize()
}

fn lower_graph(ty: &ComponentType, graph: &Graph) -> Result<ComponentVal, String> {
    let fields = record_field_types(ty, &["root", "nodes"])?;
    let node_type = list_element_type(&fields[1])?;
    let nodes = lower_list_values(
        &fields[1],
        graph
            .nodes
            .iter()
            .map(|node| lower_node(&node_type, node))
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    let root = match fields[0] {
        ComponentType::U32 => ComponentVal::U32(graph.root),
        _ => return Err("extension/value-wire-invalid: value.root must be u32".into()),
    };
    make_record(ty, &["root", "nodes"], vec![root, nodes])
}

fn lower_node(ty: &ComponentType, node: &GraphNode) -> Result<ComponentVal, String> {
    let (case, payload) = match node {
        GraphNode::Nil => ("nil", None),
        GraphNode::Boolean(value) => ("boolean", Some(ComponentVal::Bool(*value))),
        GraphNode::Integer(value) => ("integer", Some(ComponentVal::S64(*value))),
        GraphNode::Float(value) => ("float", Some(ComponentVal::Float64(*value))),
        GraphNode::Character(value) => ("character", Some(ComponentVal::Char(*value))),
        GraphNode::BigInteger(value) => (
            "big-integer",
            Some(ComponentVal::String(value.clone().into())),
        ),
        GraphNode::Regex(value) => ("regex", Some(ComponentVal::String(value.clone().into()))),
        GraphNode::Text(value) => ("text", Some(ComponentVal::String(value.clone().into()))),
        GraphNode::Bytes(values) => {
            let payload = lower_list_values_for_case(
                ty,
                "byte-vector",
                values.iter().copied().map(ComponentVal::U8).collect(),
            )?;
            ("byte-vector", Some(payload))
        }
        GraphNode::Keyword(value) => ("keyword", Some(ComponentVal::String(value.clone().into()))),
        GraphNode::Symbol { text, metadata } => {
            let payload_type = variant_payload_type(ty, "symbol")?;
            let fields = record_field_types(&payload_type, &["text", "metadata"])?;
            let payload = make_record(
                &payload_type,
                &["text", "metadata"],
                vec![
                    ComponentVal::String(text.clone().into()),
                    lower_metadata(&fields[1], metadata)?,
                ],
            )?;
            ("symbol", Some(payload))
        }
        GraphNode::Sequence {
            kind,
            values,
            metadata,
        } => {
            let case = match kind {
                SequenceKind::List => "linear-list",
                SequenceKind::Vector => "vector",
                SequenceKind::Cons => "cons",
                SequenceKind::Deque => "deque",
                SequenceKind::Queue => "queue",
                SequenceKind::Set => "set",
                SequenceKind::OrderedSet => "ordered-set",
                SequenceKind::SortedSet => "sorted-set",
            };
            (
                case,
                Some(lower_sequence(
                    &variant_payload_type(ty, case)?,
                    values,
                    metadata,
                )?),
            )
        }
        GraphNode::Mapping {
            kind,
            entries,
            metadata,
        } => {
            let case = match kind {
                MappingKind::Map => "dictionary",
                MappingKind::OrderedMap => "ordered-map",
                MappingKind::SortedMap => "sorted-map",
                MappingKind::PriorityMap => "priority-map",
                MappingKind::Trie => "trie",
            };
            (
                case,
                Some(lower_mapping(
                    &variant_payload_type(ty, case)?,
                    entries,
                    metadata,
                )?),
            )
        }
        GraphNode::Tagged { tag, form } => {
            let payload_type = variant_payload_type(ty, "tagged")?;
            let fields = record_field_types(&payload_type, &["tag", "form"])?;
            let payload = make_record(
                &payload_type,
                &["tag", "form"],
                vec![
                    ComponentVal::String(tag.clone().into()),
                    lower_u32(&fields[1], *form, "tagged.form")?,
                ],
            )?;
            ("tagged", Some(payload))
        }
        GraphNode::MapEntry {
            key,
            value,
            metadata,
        } => {
            let payload_type = variant_payload_type(ty, "map-entry")?;
            let fields = record_field_types(&payload_type, &["key", "value", "metadata"])?;
            let payload = make_record(
                &payload_type,
                &["key", "value", "metadata"],
                vec![
                    lower_u32(&fields[0], *key, "map-entry.key")?,
                    lower_u32(&fields[1], *value, "map-entry.value")?,
                    lower_metadata(&fields[2], metadata)?,
                ],
            )?;
            ("map-entry", Some(payload))
        }
        GraphNode::Pointer {
            context,
            fields: entries,
            metadata,
        } => {
            let payload_type = variant_payload_type(ty, "pointer")?;
            let fields = record_field_types(&payload_type, &["context", "fields", "metadata"])?;
            let payload = make_record(
                &payload_type,
                &["context", "fields", "metadata"],
                vec![
                    ComponentVal::String(context.clone().into()),
                    lower_entries(&fields[1], entries)?,
                    lower_metadata(&fields[2], metadata)?,
                ],
            )?;
            ("pointer", Some(payload))
        }
        GraphNode::Structure {
            name,
            fields: names,
            values,
            metadata,
        } => {
            let payload_type = variant_payload_type(ty, "structure")?;
            let fields =
                record_field_types(&payload_type, &["name", "fields", "values", "metadata"])?;
            let names = lower_list_values(
                &fields[1],
                names
                    .iter()
                    .map(|name| ComponentVal::String(name.clone().into()))
                    .collect(),
            )?;
            let values = lower_u32_list(&fields[2], values)?;
            let payload = make_record(
                &payload_type,
                &["name", "fields", "values", "metadata"],
                vec![
                    ComponentVal::String(name.clone().into()),
                    names,
                    values,
                    lower_metadata(&fields[3], metadata)?,
                ],
            )?;
            ("structure", Some(payload))
        }
        GraphNode::Exception {
            message,
            data,
            cause,
            created_at,
            throws,
        } => {
            let payload_type = variant_payload_type(ty, "exception")?;
            let fields =
                record_field_types(&payload_type, &["message", "data", "cause", "provenance"])?;
            let payload = make_record(
                &payload_type,
                &["message", "data", "cause", "provenance"],
                vec![
                    ComponentVal::String(message.clone().into()),
                    lower_u32(&fields[1], *data, "exception.data")?,
                    lower_optional_u32(&fields[2], *cause)?,
                    lower_provenance(&fields[3], created_at.as_ref(), throws)?,
                ],
            )?;
            ("exception", Some(payload))
        }
    };
    let ComponentType::Variant(variant) = ty else {
        return Err("extension/value-wire-invalid: node must be a variant".into());
    };
    variant
        .new_val(case, payload)
        .map_err(|error| format!("extension/value-wire-invalid: node {case}: {error}"))
}

fn lower_sequence(
    ty: &ComponentType,
    values: &[u32],
    metadata: &Option<Vec<NodeEntry>>,
) -> Result<ComponentVal, String> {
    let fields = record_field_types(ty, &["values", "metadata"])?;
    make_record(
        ty,
        &["values", "metadata"],
        vec![
            lower_u32_list(&fields[0], values)?,
            lower_metadata(&fields[1], metadata)?,
        ],
    )
}

fn lower_mapping(
    ty: &ComponentType,
    entries: &[NodeEntry],
    metadata: &Option<Vec<NodeEntry>>,
) -> Result<ComponentVal, String> {
    let fields = record_field_types(ty, &["entries", "metadata"])?;
    make_record(
        ty,
        &["entries", "metadata"],
        vec![
            lower_entries(&fields[0], entries)?,
            lower_metadata(&fields[1], metadata)?,
        ],
    )
}

fn lower_entries(ty: &ComponentType, entries: &[NodeEntry]) -> Result<ComponentVal, String> {
    let entry_type = list_element_type(ty)?;
    let values = entries
        .iter()
        .map(|(key, value)| {
            let fields = record_field_types(&entry_type, &["key", "value"])?;
            make_record(
                &entry_type,
                &["key", "value"],
                vec![
                    lower_u32(&fields[0], *key, "entry.key")?,
                    lower_u32(&fields[1], *value, "entry.value")?,
                ],
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    lower_list_values(ty, values)
}

fn lower_metadata(
    ty: &ComponentType,
    metadata: &Option<Vec<NodeEntry>>,
) -> Result<ComponentVal, String> {
    let ComponentType::Option(option) = ty else {
        return Err(
            "extension/value-wire-invalid: metadata must be option<list<node-entry>>".into(),
        );
    };
    option
        .new_val(match metadata {
            Some(entries) => Some(lower_entries(&option.ty(), entries)?),
            None => None,
        })
        .map_err(|error| format!("extension/value-wire-invalid: metadata: {error}"))
}

fn lower_u32(ty: &ComponentType, value: u32, label: &str) -> Result<ComponentVal, String> {
    matches!(ty, ComponentType::U32)
        .then_some(ComponentVal::U32(value))
        .ok_or_else(|| format!("extension/value-wire-invalid: {label} must be u32"))
}

fn lower_u32_list(ty: &ComponentType, values: &[u32]) -> Result<ComponentVal, String> {
    let element = list_element_type(ty)?;
    if !matches!(element, ComponentType::U32) {
        return Err("extension/value-wire-invalid: graph references must be u32".into());
    }
    lower_list_values(ty, values.iter().copied().map(ComponentVal::U32).collect())
}

fn lower_optional_u32(ty: &ComponentType, value: Option<u32>) -> Result<ComponentVal, String> {
    let ComponentType::Option(option) = ty else {
        return Err(
            "extension/value-wire-invalid: optional graph reference must be option<u32>".into(),
        );
    };
    if !matches!(option.ty(), ComponentType::U32) {
        return Err(
            "extension/value-wire-invalid: optional graph reference must be option<u32>".into(),
        );
    }
    option
        .new_val(value.map(ComponentVal::U32))
        .map_err(|error| format!("extension/value-wire-invalid: optional graph reference: {error}"))
}

fn lower_provenance(
    ty: &ComponentType,
    created_at: Option<&ExceptionSite>,
    throws: &[ExceptionSite],
) -> Result<ComponentVal, String> {
    let fields = record_field_types(ty, &["created-at", "throws"])?;
    let ComponentType::Option(option) = &fields[0] else {
        return Err("extension/value-wire-invalid: exception created-at must be optional".into());
    };
    let created_at = option
        .new_val(
            created_at
                .map(|site| lower_site(&option.ty(), site))
                .transpose()?,
        )
        .map_err(|error| format!("extension/value-wire-invalid: exception created-at: {error}"))?;
    let site_type = list_element_type(&fields[1])?;
    let throws = lower_list_values(
        &fields[1],
        throws
            .iter()
            .map(|site| lower_site(&site_type, site))
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    make_record(ty, &["created-at", "throws"], vec![created_at, throws])
}

fn lower_site(ty: &ComponentType, site: &ExceptionSite) -> Result<ComponentVal, String> {
    let fields = record_field_types(ty, &["namespace", "source-resource", "line", "column"])?;
    make_record(
        ty,
        &["namespace", "source-resource", "line", "column"],
        vec![
            lower_optional_string(&fields[0], site.namespace.as_deref())?,
            lower_optional_string(&fields[1], site.resource.as_deref())?,
            lower_u64(&fields[2], site.line as u64, "exception-site.line")?,
            lower_u64(&fields[3], site.column as u64, "exception-site.column")?,
        ],
    )
}

fn lower_optional_string(ty: &ComponentType, value: Option<&str>) -> Result<ComponentVal, String> {
    let ComponentType::Option(option) = ty else {
        return Err("extension/value-wire-invalid: optional string must use option<string>".into());
    };
    if !matches!(option.ty(), ComponentType::String) {
        return Err("extension/value-wire-invalid: optional string must use option<string>".into());
    }
    option
        .new_val(value.map(|value| ComponentVal::String(value.into())))
        .map_err(|error| format!("extension/value-wire-invalid: optional string: {error}"))
}

fn lower_u64(ty: &ComponentType, value: u64, label: &str) -> Result<ComponentVal, String> {
    matches!(ty, ComponentType::U64)
        .then_some(ComponentVal::U64(value))
        .ok_or_else(|| format!("extension/value-wire-invalid: {label} must be u64"))
}

fn lower_list_values(
    ty: &ComponentType,
    values: Vec<ComponentVal>,
) -> Result<ComponentVal, String> {
    let ComponentType::List(list) = ty else {
        return Err("extension/value-wire-invalid: expected Component list".into());
    };
    list.new_val(values.into_boxed_slice())
        .map_err(|error| format!("extension/value-wire-invalid: list: {error}"))
}

fn lower_list_values_for_case(
    variant: &ComponentType,
    case: &str,
    values: Vec<ComponentVal>,
) -> Result<ComponentVal, String> {
    lower_list_values(&variant_payload_type(variant, case)?, values)
}

fn list_element_type(ty: &ComponentType) -> Result<ComponentType, String> {
    let ComponentType::List(list) = ty else {
        return Err("extension/value-wire-invalid: expected Component list".into());
    };
    Ok(list.ty())
}

fn variant_payload_type(ty: &ComponentType, case: &str) -> Result<ComponentType, String> {
    let ComponentType::Variant(variant) = ty else {
        return Err("extension/value-wire-invalid: node must be a Component variant".into());
    };
    variant
        .cases()
        .find(|candidate| candidate.name == case)
        .and_then(|candidate| candidate.ty)
        .ok_or_else(|| format!("extension/value-wire-invalid: node case {case} has no payload"))
}

fn record_field_types(ty: &ComponentType, names: &[&str]) -> Result<Vec<ComponentType>, String> {
    let ComponentType::Record(record) = ty else {
        return Err("extension/value-wire-invalid: expected Component record".into());
    };
    let fields = record.fields().collect::<Vec<_>>();
    if fields
        .iter()
        .map(|field| field.name)
        .ne(names.iter().copied())
    {
        return Err(format!(
            "extension/value-wire-invalid: expected record fields {:?}, got {:?}",
            names,
            fields.iter().map(|field| field.name).collect::<Vec<_>>()
        ));
    }
    Ok(fields.into_iter().map(|field| field.ty).collect())
}

fn make_record(
    ty: &ComponentType,
    names: &[&str],
    values: Vec<ComponentVal>,
) -> Result<ComponentVal, String> {
    let ComponentType::Record(record) = ty else {
        return Err("extension/value-wire-invalid: expected Component record".into());
    };
    record_field_types(ty, names)?;
    record
        .new_val(names.iter().copied().zip(values))
        .map_err(|error| format!("extension/value-wire-invalid: record: {error}"))
}

fn parse_graph(value: &ComponentVal) -> Result<Graph, String> {
    let fields = record_values(value, &["root", "nodes"])?;
    let root = as_u32(fields[0], "value.root")?;
    let nodes = as_list(fields[1], "value.nodes")?
        .iter()
        .map(parse_node)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Graph { root, nodes })
}

fn parse_node(value: &ComponentVal) -> Result<GraphNode, String> {
    let ComponentVal::Variant(variant) = value else {
        return Err("extension/value-wire-invalid: graph node must be a variant".into());
    };
    let case = variant.discriminant();
    let payload = |case| {
        variant
            .payload()
            .ok_or_else(|| format!("extension/value-wire-invalid: node {case} requires a payload"))
    };
    match case {
        "nil" if variant.payload().is_none() => Ok(GraphNode::Nil),
        "boolean" => Ok(GraphNode::Boolean(as_bool(payload(case)?, "boolean")?)),
        "integer" => Ok(GraphNode::Integer(as_s64(payload(case)?, "integer")?)),
        "float" => {
            let value = as_f64(payload(case)?, "float")?;
            value
                .is_finite()
                .then_some(GraphNode::Float(value))
                .ok_or_else(|| "extension/value-wire-invalid: float must be finite".into())
        }
        "character" => Ok(GraphNode::Character(as_char(payload(case)?, "character")?)),
        "big-integer" => Ok(GraphNode::BigInteger(as_string(
            payload(case)?,
            "big-integer",
        )?)),
        "regex" => Ok(GraphNode::Regex(as_string(payload(case)?, "regex")?)),
        "text" => Ok(GraphNode::Text(as_string(payload(case)?, "text")?)),
        "byte-vector" => Ok(GraphNode::Bytes(
            as_list(payload(case)?, "byte-vector")?
                .iter()
                .map(|value| as_u8(value, "byte-vector item"))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        "keyword" => Ok(GraphNode::Keyword(as_string(payload(case)?, "keyword")?)),
        "symbol" => {
            let fields = record_values(payload(case)?, &["text", "metadata"])?;
            Ok(GraphNode::Symbol {
                text: as_string(fields[0], "symbol.text")?,
                metadata: parse_optional_entries(fields[1], "symbol.metadata")?,
            })
        }
        "linear-list" => parse_sequence(payload(case)?, SequenceKind::List),
        "vector" => parse_sequence(payload(case)?, SequenceKind::Vector),
        "cons" => parse_sequence(payload(case)?, SequenceKind::Cons),
        "deque" => parse_sequence(payload(case)?, SequenceKind::Deque),
        "queue" => parse_sequence(payload(case)?, SequenceKind::Queue),
        "set" => parse_sequence(payload(case)?, SequenceKind::Set),
        "ordered-set" => parse_sequence(payload(case)?, SequenceKind::OrderedSet),
        "sorted-set" => parse_sequence(payload(case)?, SequenceKind::SortedSet),
        "dictionary" => parse_mapping(payload(case)?, MappingKind::Map),
        "ordered-map" => parse_mapping(payload(case)?, MappingKind::OrderedMap),
        "sorted-map" => parse_mapping(payload(case)?, MappingKind::SortedMap),
        "priority-map" => parse_mapping(payload(case)?, MappingKind::PriorityMap),
        "trie" => parse_mapping(payload(case)?, MappingKind::Trie),
        "tagged" => {
            let fields = record_values(payload(case)?, &["tag", "form"])?;
            Ok(GraphNode::Tagged {
                tag: as_string(fields[0], "tagged.tag")?,
                form: as_u32(fields[1], "tagged.form")?,
            })
        }
        "map-entry" => {
            let fields = record_values(payload(case)?, &["key", "value", "metadata"])?;
            Ok(GraphNode::MapEntry {
                key: as_u32(fields[0], "map-entry.key")?,
                value: as_u32(fields[1], "map-entry.value")?,
                metadata: parse_optional_entries(fields[2], "map-entry.metadata")?,
            })
        }
        "pointer" => {
            let fields = record_values(payload(case)?, &["context", "fields", "metadata"])?;
            Ok(GraphNode::Pointer {
                context: as_string(fields[0], "pointer.context")?,
                fields: parse_entries(fields[1], "pointer.fields")?,
                metadata: parse_optional_entries(fields[2], "pointer.metadata")?,
            })
        }
        "structure" => {
            let fields = record_values(payload(case)?, &["name", "fields", "values", "metadata"])?;
            Ok(GraphNode::Structure {
                name: as_string(fields[0], "structure.name")?,
                fields: as_list(fields[1], "structure.fields")?
                    .iter()
                    .map(|value| as_string(value, "structure field"))
                    .collect::<Result<Vec<_>, _>>()?,
                values: parse_u32_list(fields[2], "structure.values")?,
                metadata: parse_optional_entries(fields[3], "structure.metadata")?,
            })
        }
        "exception" => {
            let fields =
                record_values(payload(case)?, &["message", "data", "cause", "provenance"])?;
            let provenance = parse_provenance(fields[3])?;
            Ok(GraphNode::Exception {
                message: as_string(fields[0], "exception.message")?,
                data: as_u32(fields[1], "exception.data")?,
                cause: parse_optional_u32(fields[2], "exception.cause")?,
                created_at: provenance.0,
                throws: provenance.1,
            })
        }
        _ => Err(format!(
            "extension/value-wire-invalid: unknown node case {case}"
        )),
    }
}

fn parse_sequence(value: &ComponentVal, kind: SequenceKind) -> Result<GraphNode, String> {
    let fields = record_values(value, &["values", "metadata"])?;
    Ok(GraphNode::Sequence {
        kind,
        values: parse_u32_list(fields[0], "sequence.values")?,
        metadata: parse_optional_entries(fields[1], "sequence.metadata")?,
    })
}

fn parse_mapping(value: &ComponentVal, kind: MappingKind) -> Result<GraphNode, String> {
    let fields = record_values(value, &["entries", "metadata"])?;
    Ok(GraphNode::Mapping {
        kind,
        entries: parse_entries(fields[0], "mapping.entries")?,
        metadata: parse_optional_entries(fields[1], "mapping.metadata")?,
    })
}

fn parse_entries(value: &ComponentVal, label: &str) -> Result<Vec<NodeEntry>, String> {
    as_list(value, label)?
        .iter()
        .map(|value| {
            let fields = record_values(value, &["key", "value"])?;
            Ok((
                as_u32(fields[0], "entry.key")?,
                as_u32(fields[1], "entry.value")?,
            ))
        })
        .collect()
}

fn parse_optional_entries(
    value: &ComponentVal,
    label: &str,
) -> Result<Option<Vec<NodeEntry>>, String> {
    let ComponentVal::Option(option) = value else {
        return Err(format!(
            "extension/value-wire-invalid: {label} must be an option"
        ));
    };
    option
        .value()
        .map(|value| parse_entries(value, label))
        .transpose()
}

fn parse_u32_list(value: &ComponentVal, label: &str) -> Result<Vec<u32>, String> {
    as_list(value, label)?
        .iter()
        .map(|value| as_u32(value, label))
        .collect()
}

fn parse_provenance(
    value: &ComponentVal,
) -> Result<(Option<ExceptionSite>, Vec<ExceptionSite>), String> {
    let fields = record_values(value, &["created-at", "throws"])?;
    let ComponentVal::Option(created_at) = fields[0] else {
        return Err("extension/value-wire-invalid: exception created-at must be an option".into());
    };
    let created_at = created_at.value().map(parse_site).transpose()?;
    let throws = as_list(fields[1], "exception.throws")?
        .iter()
        .map(parse_site)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((created_at, throws))
}

fn parse_site(value: &ComponentVal) -> Result<ExceptionSite, String> {
    let fields = record_values(value, &["namespace", "source-resource", "line", "column"])?;
    Ok(ExceptionSite {
        namespace: parse_optional_string(fields[0], "exception-site.namespace")?,
        resource: parse_optional_string(fields[1], "exception-site.source-resource")?,
        line: usize::try_from(as_u64(fields[2], "exception-site.line")?).map_err(|_| {
            "extension/value-wire-invalid: exception-site.line exceeds usize".to_owned()
        })?,
        column: usize::try_from(as_u64(fields[3], "exception-site.column")?).map_err(|_| {
            "extension/value-wire-invalid: exception-site.column exceeds usize".to_owned()
        })?,
    })
}

fn parse_optional_string(value: &ComponentVal, label: &str) -> Result<Option<String>, String> {
    let ComponentVal::Option(option) = value else {
        return Err(format!(
            "extension/value-wire-invalid: {label} must be an option"
        ));
    };
    option
        .value()
        .map(|value| as_string(value, label))
        .transpose()
}

fn parse_optional_u32(value: &ComponentVal, label: &str) -> Result<Option<u32>, String> {
    let ComponentVal::Option(option) = value else {
        return Err(format!(
            "extension/value-wire-invalid: {label} must be an option"
        ));
    };
    option.value().map(|value| as_u32(value, label)).transpose()
}

fn record_values<'a>(
    value: &'a ComponentVal,
    names: &[&str],
) -> Result<Vec<&'a ComponentVal>, String> {
    let ComponentVal::Record(record) = value else {
        return Err("extension/value-wire-invalid: expected Component record".into());
    };
    let fields = record.fields().collect::<Vec<_>>();
    if fields
        .iter()
        .map(|(name, _)| *name)
        .ne(names.iter().copied())
    {
        return Err(format!(
            "extension/value-wire-invalid: expected record fields {:?}, got {:?}",
            names,
            fields.iter().map(|(name, _)| *name).collect::<Vec<_>>()
        ));
    }
    Ok(fields.into_iter().map(|(_, value)| value).collect())
}

fn as_list<'a>(value: &'a ComponentVal, label: &str) -> Result<&'a [ComponentVal], String> {
    let ComponentVal::List(values) = value else {
        return Err(format!(
            "extension/value-wire-invalid: {label} must be a list"
        ));
    };
    Ok(values)
}

fn as_bool(value: &ComponentVal, label: &str) -> Result<bool, String> {
    match value {
        ComponentVal::Bool(value) => Ok(*value),
        _ => Err(format!(
            "extension/value-wire-invalid: {label} must be bool"
        )),
    }
}

fn as_u8(value: &ComponentVal, label: &str) -> Result<u8, String> {
    match value {
        ComponentVal::U8(value) => Ok(*value),
        _ => Err(format!("extension/value-wire-invalid: {label} must be u8")),
    }
}

fn as_u32(value: &ComponentVal, label: &str) -> Result<u32, String> {
    match value {
        ComponentVal::U32(value) => Ok(*value),
        _ => Err(format!("extension/value-wire-invalid: {label} must be u32")),
    }
}

fn as_u64(value: &ComponentVal, label: &str) -> Result<u64, String> {
    match value {
        ComponentVal::U64(value) => Ok(*value),
        _ => Err(format!("extension/value-wire-invalid: {label} must be u64")),
    }
}

fn as_s64(value: &ComponentVal, label: &str) -> Result<i64, String> {
    match value {
        ComponentVal::S64(value) => Ok(*value),
        _ => Err(format!("extension/value-wire-invalid: {label} must be s64")),
    }
}

fn as_f64(value: &ComponentVal, label: &str) -> Result<f64, String> {
    match value {
        ComponentVal::Float64(value) => Ok(*value),
        _ => Err(format!("extension/value-wire-invalid: {label} must be f64")),
    }
}

fn as_char(value: &ComponentVal, label: &str) -> Result<char, String> {
    match value {
        ComponentVal::Char(value) => Ok(*value),
        _ => Err(format!(
            "extension/value-wire-invalid: {label} must be char"
        )),
    }
}

fn as_string(value: &ComponentVal, label: &str) -> Result<String, String> {
    match value {
        ComponentVal::String(value) => Ok(value.to_string()),
        _ => Err(format!(
            "extension/value-wire-invalid: {label} must be string"
        )),
    }
}

impl Graph {
    fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() {
            return Err("extension/value-wire-invalid: graph must contain a root node".into());
        }
        let root = usize::try_from(self.root)
            .map_err(|_| "extension/value-wire-invalid: root exceeds usize".to_owned())?;
        if root != self.nodes.len() - 1 {
            return Err(
                "extension/value-wire-invalid: root must be the final post-order node".into(),
            );
        }
        for (index, node) in self.nodes.iter().enumerate() {
            for child in node.children() {
                if usize::try_from(child)
                    .ok()
                    .map_or(true, |child| child >= index)
                {
                    return Err(format!(
                        "extension/value-wire-invalid: node {index} references non-prior child {child}"
                    ));
                }
            }
        }
        let mut seen = HashSet::new();
        let mut pending = vec![self.root];
        while let Some(index) = pending.pop() {
            if seen.insert(index) {
                let node = self.nodes.get(index as usize).ok_or_else(|| {
                    format!("extension/value-wire-invalid: graph index {index} is out of range")
                })?;
                pending.extend(node.children());
            }
        }
        if seen.len() != self.nodes.len() {
            return Err("extension/value-wire-invalid: graph contains unreachable nodes".into());
        }
        Ok(())
    }

    fn materialize(&self) -> Result<Value, String> {
        let mut values = Vec::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.iter().enumerate() {
            let value = match node {
                GraphNode::Nil => Value::Nil,
                GraphNode::Boolean(value) => Value::Bool(*value),
                GraphNode::Integer(value) => Value::Number(*value),
                GraphNode::Float(value) => Value::Float(*value),
                GraphNode::Character(value) => Value::Character(*value),
                GraphNode::BigInteger(value) => {
                    Value::BigInteger(value.parse::<BigInt>().map_err(|_| {
                        "extension/value-wire-invalid: invalid big-integer".to_owned()
                    })?)
                }
                GraphNode::Regex(value) => Value::Regex(value.clone()),
                GraphNode::Text(value) => Value::String(value.clone()),
                GraphNode::Bytes(value) => Value::Bytes(value.clone()),
                GraphNode::Keyword(value) => {
                    Value::Keyword(Keyword::parse(value).map_err(|error| {
                        format!("extension/value-wire-invalid: keyword: {error}")
                    })?)
                }
                GraphNode::Symbol { text, metadata } => attach_metadata(
                    Value::Symbol(Symbol::parse(text)),
                    materialize_metadata(&values, metadata)?,
                )?,
                GraphNode::Sequence {
                    kind,
                    values: refs,
                    metadata,
                } => {
                    let elements = materialize_values(&values, refs, index)?;
                    let value = match kind {
                        SequenceKind::List => Value::List(elements.into_iter().collect()),
                        SequenceKind::Vector => Value::Vector(elements.into_iter().collect()),
                        SequenceKind::Cons => {
                            let Some((first, rest)) = elements.split_first() else {
                                return Err(
                                    "extension/value-wire-invalid: cons cannot be empty".into()
                                );
                            };
                            Value::Cons(Box::new(Cons::new(
                                first.clone(),
                                rest.iter().cloned().collect(),
                            )))
                        }
                        SequenceKind::Deque => {
                            Value::Deque(Box::new(elements.into_iter().collect()))
                        }
                        SequenceKind::Queue => {
                            Value::Queue(Box::new(elements.into_iter().collect()))
                        }
                        SequenceKind::Set => Value::Set(elements.into_iter().collect()),
                        SequenceKind::OrderedSet => {
                            Value::OrderedSet(Box::new(elements.into_iter().collect()))
                        }
                        SequenceKind::SortedSet => {
                            Value::SortedSet(Box::new(elements.into_iter().collect()))
                        }
                    };
                    attach_metadata(value, materialize_metadata(&values, metadata)?)?
                }
                GraphNode::Mapping {
                    kind,
                    entries,
                    metadata,
                } => {
                    let entries = materialize_pairs(&values, entries, index)?;
                    let value = match kind {
                        MappingKind::Map => Value::Map(entries.into_iter().collect()),
                        MappingKind::OrderedMap => {
                            Value::OrderedMap(Box::new(entries.into_iter().collect()))
                        }
                        MappingKind::SortedMap => {
                            Value::SortedMap(Box::new(entries.into_iter().collect()))
                        }
                        MappingKind::PriorityMap => {
                            Value::PriorityMap(Box::new(entries.into_iter().collect()))
                        }
                        MappingKind::Trie => Value::Trie(Box::new(entries.into_iter().try_fold(
                            Trie::new(),
                            |trie, (key, value)| match key {
                                Value::String(key) => Ok(trie.assoc_value(key, value)),
                                _ => Err("extension/value-wire-invalid: trie key must be string"),
                            },
                        )?)),
                    };
                    attach_metadata(value, materialize_metadata(&values, metadata)?)?
                }
                GraphNode::Tagged { tag, form } => Value::Tagged(Box::new(TaggedLiteral::new(
                    Symbol::parse(tag),
                    materialize_value(&values, *form, index)?,
                ))),
                GraphNode::MapEntry {
                    key,
                    value,
                    metadata,
                } => attach_metadata(
                    Value::MapEntry(Box::new(MapEntry::new(
                        materialize_value(&values, *key, index)?,
                        materialize_value(&values, *value, index)?,
                    ))),
                    materialize_metadata(&values, metadata)?,
                )?,
                GraphNode::Pointer {
                    context,
                    fields,
                    metadata,
                } => attach_metadata(
                    Value::Pointer(Pointer::new(
                        Keyword::parse(context).map_err(|error| {
                            format!("extension/value-wire-invalid: pointer context: {error}")
                        })?,
                        materialize_pairs(&values, fields, index)?
                            .into_iter()
                            .collect(),
                    )),
                    materialize_metadata(&values, metadata)?,
                )?,
                GraphNode::Structure {
                    name,
                    fields,
                    values: refs,
                    metadata,
                } => {
                    if fields.len() != refs.len() {
                        return Err("extension/value-wire-invalid: structure fields and values differ in length".into());
                    }
                    let structure = StructValue::from_values(
                        Rc::new(StructType::detached(name.clone(), fields.clone())),
                        materialize_values(&values, refs, index)?,
                        materialize_metadata(&values, metadata)?,
                    )?;
                    Value::Struct(Rc::new(structure))
                }
                GraphNode::Exception {
                    message,
                    data,
                    cause,
                    created_at,
                    throws,
                } => Value::ExceptionInfo(Rc::new(ExceptionInfo {
                    message: message.clone(),
                    data: Box::new(materialize_value(&values, *data, index)?),
                    cause: cause
                        .map(|cause| materialize_value(&values, cause, index).map(Box::new))
                        .transpose()?,
                    provenance: Rc::new(RefCell::new(ExceptionProvenance {
                        created_at: created_at.clone(),
                        throws: throws.clone(),
                    })),
                })),
            };
            values.push(value);
        }
        values
            .get(self.root as usize)
            .cloned()
            .ok_or_else(|| "extension/value-wire-invalid: root is out of range".into())
    }
}

fn materialize_value(values: &[Value], reference: u32, owner: usize) -> Result<Value, String> {
    let index = usize::try_from(reference)
        .map_err(|_| "extension/value-wire-invalid: graph reference exceeds usize".to_owned())?;
    if index >= owner {
        return Err(format!(
            "extension/value-wire-invalid: node {owner} references non-prior child {reference}"
        ));
    }
    values.get(index).cloned().ok_or_else(|| {
        format!("extension/value-wire-invalid: graph reference {reference} is out of range")
    })
}

fn materialize_values(
    values: &[Value],
    references: &[u32],
    owner: usize,
) -> Result<Vec<Value>, String> {
    references
        .iter()
        .map(|reference| materialize_value(values, *reference, owner))
        .collect()
}

fn materialize_pairs(
    values: &[Value],
    entries: &[NodeEntry],
    owner: usize,
) -> Result<Vec<(Value, Value)>, String> {
    entries
        .iter()
        .map(|(key, value)| {
            Ok((
                materialize_value(values, *key, owner)?,
                materialize_value(values, *value, owner)?,
            ))
        })
        .collect()
}

fn materialize_metadata(
    values: &[Value],
    entries: &Option<Vec<NodeEntry>>,
) -> Result<Option<Rc<Metadata>>, String> {
    entries
        .as_ref()
        .map(|entries| {
            entries
                .iter()
                .map(|(key, value)| {
                    let key = values.get(*key as usize).cloned().ok_or_else(|| {
                        format!("extension/value-wire-invalid: metadata key {key} is out of range")
                    })?;
                    let value = values.get(*value as usize).cloned().ok_or_else(|| {
                        format!(
                            "extension/value-wire-invalid: metadata value {value} is out of range"
                        )
                    })?;
                    Ok((metadata_value(key)?, metadata_value(value)?))
                })
                .collect::<Result<Vec<_>, String>>()
                .map(Metadata::new)
        })
        .transpose()
}

fn metadata_value(value: Value) -> Result<MetadataValue, String> {
    if metadata_of(&value).is_some() && !matches!(value, Value::Symbol(_)) {
        return Err(
            "extension/value-wire-invalid: metadata values cannot themselves carry metadata".into(),
        );
    }
    match value {
        Value::Nil => Ok(MetadataValue::Nil),
        Value::Bool(value) => Ok(MetadataValue::Boolean(value)),
        Value::Number(value) => Ok(MetadataValue::Number(value)),
        Value::Float(value) => Ok(MetadataValue::Float(value)),
        Value::BigInteger(value) => Ok(MetadataValue::BigInteger(value)),
        Value::Character(value) => Ok(MetadataValue::Character(value)),
        Value::Regex(value) => Ok(MetadataValue::Regex(value)),
        Value::Tagged(value) => Ok(MetadataValue::Tagged(
            value.tag().as_str().into(),
            Box::new(metadata_value(value.form().clone())?),
        )),
        Value::String(value) => Ok(MetadataValue::String(value)),
        Value::Keyword(value) => Ok(MetadataValue::Keyword(value)),
        Value::Symbol(value) => Ok(MetadataValue::Symbol(value)),
        Value::Vector(values) => Ok(MetadataValue::Vector(
            values
                .iter()
                .cloned()
                .map(metadata_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Value::Tuple(values) => Ok(MetadataValue::Vector(
            values
                .iter()
                .cloned()
                .map(metadata_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Value::List(values) => Ok(MetadataValue::List(
            values
                .iter()
                .cloned()
                .map(metadata_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Value::Set(values) => Ok(MetadataValue::Set(
            values
                .iter()
                .cloned()
                .map(metadata_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Value::Map(values) => Ok(MetadataValue::Map(
            values
                .iter()
                .map(|(key, value)| {
                    Ok((metadata_value(key.clone())?, metadata_value(value.clone())?))
                })
                .collect::<Result<Vec<_>, String>>()?,
        )),
        _ => Err("extension/value-wire-invalid: unsupported metadata value".into()),
    }
}

fn metadata_of(value: &Value) -> Option<Rc<Metadata>> {
    match value {
        Value::Symbol(value) => value.meta().cloned(),
        Value::Map(value) => value.meta().cloned(),
        Value::OrderedMap(value) => value.meta().cloned(),
        Value::SortedMap(value) => value.meta().cloned(),
        Value::Trie(value) => value.meta().cloned(),
        Value::Set(value) => value.meta().cloned(),
        Value::OrderedSet(value) => value.meta().cloned(),
        Value::SortedSet(value) => value.meta().cloned(),
        Value::List(value) => value.meta().cloned(),
        Value::Cons(value) => value.meta().cloned(),
        Value::Deque(value) => value.meta().cloned(),
        Value::Queue(value) => value.meta().cloned(),
        Value::PriorityMap(value) => value.meta().cloned(),
        Value::Pointer(value) => value.meta().cloned(),
        Value::Tuple(value) => value.meta().cloned(),
        Value::Vector(value) => value.meta().cloned(),
        Value::MapEntry(value) => value.meta().cloned(),
        Value::Struct(value) => value.metadata.clone(),
        _ => None,
    }
}

fn attach_metadata(value: Value, metadata: Option<Rc<Metadata>>) -> Result<Value, String> {
    match value {
        Value::Symbol(value) => Ok(Value::Symbol(value.with_meta(metadata))),
        Value::Map(value) => Ok(Value::Map(value.with_meta(metadata))),
        Value::OrderedMap(value) => Ok(Value::OrderedMap(Box::new(value.with_meta(metadata)))),
        Value::SortedMap(value) => Ok(Value::SortedMap(Box::new(value.with_meta(metadata)))),
        Value::Trie(value) => Ok(Value::Trie(Box::new(value.with_meta(metadata)))),
        Value::Set(value) => Ok(Value::Set(value.with_meta(metadata))),
        Value::OrderedSet(value) => Ok(Value::OrderedSet(Box::new(value.with_meta(metadata)))),
        Value::SortedSet(value) => Ok(Value::SortedSet(Box::new(value.with_meta(metadata)))),
        Value::List(value) => Ok(Value::List(value.with_meta(metadata))),
        Value::Cons(value) => Ok(Value::Cons(Box::new(value.with_meta(metadata)))),
        Value::Deque(value) => Ok(Value::Deque(Box::new(value.with_meta(metadata)))),
        Value::Queue(value) => Ok(Value::Queue(Box::new(value.with_meta(metadata)))),
        Value::PriorityMap(value) => Ok(Value::PriorityMap(Box::new(value.with_meta(metadata)))),
        Value::Pointer(value) => Ok(Value::Pointer(value.with_meta(metadata))),
        Value::Tuple(value) => Ok(Value::Tuple(Box::new(value.with_meta(metadata)))),
        Value::Vector(value) => Ok(Value::Vector(value.with_meta(metadata))),
        Value::MapEntry(value) => Ok(Value::MapEntry(Box::new(value.with_meta(metadata)))),
        value if metadata.is_none() => Ok(value),
        _ => Err("extension/value-wire-invalid: metadata attached to unsupported value".into()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::core::Value;
    use crate::lang::data::{Deque, Keyword, MapEntry, Pointer, PriorityMap, Symbol};

    use super::{Graph, GraphBuilder, GraphNode};

    #[test]
    fn canonical_graph_round_trips_persistent_kinds_and_rejects_nonportable_values() {
        let priority = PriorityMap::new()
            .assoc_value(Value::String("second".into()), Value::Number(2))
            .assoc_value(Value::String("first".into()), Value::Number(1));
        let value = Value::OrderedMap(Box::new(
            [
                (
                    Value::Keyword(Keyword::from("big")),
                    Value::BigInteger("9223372036854775808".parse().unwrap()),
                ),
                (
                    Value::Keyword(Keyword::from("deque")),
                    Value::Deque(Box::new(
                        [Value::Number(1), Value::Number(2)]
                            .into_iter()
                            .collect::<Deque<_>>(),
                    )),
                ),
                (
                    Value::Keyword(Keyword::from("priority")),
                    Value::PriorityMap(Box::new(priority)),
                ),
                (
                    Value::Keyword(Keyword::from("entry")),
                    Value::MapEntry(Box::new(MapEntry::new(
                        Value::Symbol(Symbol::parse("key")),
                        Value::String("value".into()),
                    ))),
                ),
                (
                    Value::Keyword(Keyword::from("pointer")),
                    Value::Pointer(Pointer::new(
                        Keyword::from("fixture"),
                        [(Value::Keyword(Keyword::from("id")), Value::Number(7))]
                            .into_iter()
                            .collect(),
                    )),
                ),
            ]
            .into_iter()
            .collect(),
        ));

        let mut builder = GraphBuilder::new();
        let root = builder.append(&value).unwrap();
        let graph = builder.finish(root);
        graph.validate().unwrap();
        assert_eq!(graph.materialize().unwrap(), value);

        let mut builder = GraphBuilder::new();
        assert!(builder
            .append(&Value::ByteBuffer(Rc::new(RefCell::new(vec![1]))))
            .unwrap_err()
            .contains("byte-buffer"));
        assert!(builder
            .append(&Value::Float(f64::NAN))
            .unwrap_err()
            .contains("non-finite-float"));

        let malformed = Graph {
            root: 1,
            nodes: vec![GraphNode::Nil, GraphNode::Nil],
        };
        assert!(malformed.validate().unwrap_err().contains("unreachable"));
    }
}
