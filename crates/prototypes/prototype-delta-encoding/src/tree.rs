use serde::de::{self, DeserializeOwned, Deserializer, SeqAccess, Visitor};
use serde::ser::{self, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;

// ── Tree Error ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct TreeError(pub String);

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for TreeError {}
impl serde::ser::Error for TreeError {
    fn custom<T: fmt::Display>(msg: T) -> Self { TreeError(msg.to_string()) }
}
impl serde::de::Error for TreeError {
    fn custom<T: fmt::Display>(msg: T) -> Self { TreeError(msg.to_string()) }
}

// ── Tree (positional-value tree with no string keys) ──────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    None,
    Bool(bool),
    U64(u64),
    I64(i64),
    F64(f64),
    Str(String),
    Bytes(Vec<u8>),
    Seq(Vec<Node>),
    /// Positional struct fields — NO string keys stored.
    Struct(Vec<Node>),
}

pub fn to_node<T: Serialize>(value: &T) -> Node {
    value.serialize(NodeSerializer).unwrap()
}

// ── Node manual (De)serialization for postcard round-trip ────────────────
// Format: tag byte followed by variant data. Tags 7 and 8 use length-prefixed Vec.

impl serde::Serialize for Node {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let len = match self {
            Node::None => 1,
            Node::Bool(_) | Node::U64(_) | Node::I64(_) | Node::F64(_) | Node::Str(_) | Node::Bytes(_) => 2,
            Node::Seq(_) | Node::Struct(_) => 2,
        };
        let mut seq = s.serialize_seq(Some(len))?;
        match self {
            Node::None => seq.serialize_element(&0u8)?,
            Node::Bool(v) => { seq.serialize_element(&1u8)?; seq.serialize_element(v)?; }
            Node::U64(v) => { seq.serialize_element(&2u8)?; seq.serialize_element(v)?; }
            Node::I64(v) => { seq.serialize_element(&3u8)?; seq.serialize_element(v)?; }
            Node::F64(v) => { seq.serialize_element(&4u8)?; seq.serialize_element(v)?; }
            Node::Str(v) => { seq.serialize_element(&5u8)?; seq.serialize_element(v)?; }
            Node::Bytes(v) => { seq.serialize_element(&6u8)?; seq.serialize_element(v)?; }
            Node::Seq(v) => { seq.serialize_element(&7u8)?; seq.serialize_element(v)?; }
            Node::Struct(v) => { seq.serialize_element(&8u8)?; seq.serialize_element(v)?; }
        }
        seq.end()
    }
}

impl<'de> serde::Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Node, D::Error> {
        struct NodeVisitor;
        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = Node;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("Node") }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
                let tag: u8 = seq.next_element()?.ok_or_else(|| de::Error::custom("missing tag"))?;
                match tag {
                    0 => Ok(Node::None),
                    1 => seq.next_element()?.map(Node::Bool).ok_or_else(|| de::Error::custom("missing Bool")),
                    2 => seq.next_element()?.map(Node::U64).ok_or_else(|| de::Error::custom("missing U64")),
                    3 => seq.next_element()?.map(Node::I64).ok_or_else(|| de::Error::custom("missing I64")),
                    4 => seq.next_element()?.map(Node::F64).ok_or_else(|| de::Error::custom("missing F64")),
                    5 => seq.next_element()?.map(Node::Str).ok_or_else(|| de::Error::custom("missing Str")),
                    6 => seq.next_element()?.map(Node::Bytes).ok_or_else(|| de::Error::custom("missing Bytes")),
                    7 => seq.next_element()?.map(Node::Seq).ok_or_else(|| de::Error::custom("missing Seq")),
                    8 => seq.next_element()?.map(Node::Struct).ok_or_else(|| de::Error::custom("missing Struct")),
                    _ => Err(de::Error::custom("unknown Node tag")),
                }
            }
        }
        // Tell postcard this is a variable-length sequence
        d.deserialize_seq(NodeVisitor)
    }
}

// ── Serializer (builds Node from any Serialize impl) ──────────────────────

struct NodeSerializer;

impl Serializer for NodeSerializer {
    type Ok = Node;
    type Error = TreeError;
    type SerializeSeq = NodeBuilder;
    type SerializeTuple = NodeBuilder;
    type SerializeTupleStruct = NodeBuilder;
    type SerializeTupleVariant = NodeBuilder;
    type SerializeMap = MapBuilder;
    type SerializeStruct = NodeBuilder;
    type SerializeStructVariant = NodeBuilder;

    fn serialize_bool(self, v: bool) -> Result<Node, TreeError> { Ok(Node::Bool(v)) }
    fn serialize_i8(self, v: i8) -> Result<Node, TreeError> { Ok(Node::I64(v as i64)) }
    fn serialize_i16(self, v: i16) -> Result<Node, TreeError> { Ok(Node::I64(v as i64)) }
    fn serialize_i32(self, v: i32) -> Result<Node, TreeError> { Ok(Node::I64(v as i64)) }
    fn serialize_i64(self, v: i64) -> Result<Node, TreeError> { Ok(Node::I64(v)) }
    fn serialize_u8(self, v: u8) -> Result<Node, TreeError> { Ok(Node::U64(v as u64)) }
    fn serialize_u16(self, v: u16) -> Result<Node, TreeError> { Ok(Node::U64(v as u64)) }
    fn serialize_u32(self, v: u32) -> Result<Node, TreeError> { Ok(Node::U64(v as u64)) }
    fn serialize_u64(self, v: u64) -> Result<Node, TreeError> { Ok(Node::U64(v)) }
    fn serialize_f32(self, v: f32) -> Result<Node, TreeError> { Ok(Node::F64(v as f64)) }
    fn serialize_f64(self, v: f64) -> Result<Node, TreeError> { Ok(Node::F64(v)) }
    fn serialize_char(self, v: char) -> Result<Node, TreeError> { Ok(Node::Str(v.to_string())) }
    fn serialize_str(self, v: &str) -> Result<Node, TreeError> { Ok(Node::Str(v.to_owned())) }
    fn serialize_bytes(self, v: &[u8]) -> Result<Node, TreeError> { Ok(Node::Bytes(v.to_vec())) }
    fn serialize_none(self) -> Result<Node, TreeError> { Ok(Node::None) }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Result<Node, TreeError> { v.serialize(self) }
    fn serialize_unit(self) -> Result<Node, TreeError> { Ok(Node::None) }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Node, TreeError> { Ok(Node::None) }
    fn serialize_unit_variant(self, _: &'static str, idx: u32, _: &'static str) -> Result<Node, TreeError> { Ok(Node::U64(idx as u64)) }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _: &'static str, v: &T) -> Result<Node, TreeError> { v.serialize(self) }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(self, _: &'static str, idx: u32, _: &'static str, v: &T) -> Result<Node, TreeError> {
        Ok(Node::Struct(vec![Node::U64(idx as u64), v.serialize(self)?]))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<NodeBuilder, TreeError> { Ok(NodeBuilder(Vec::new())) }
    fn serialize_tuple(self, len: usize) -> Result<NodeBuilder, TreeError> { Ok(NodeBuilder(Vec::with_capacity(len))) }
    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<NodeBuilder, TreeError> { Ok(NodeBuilder(Vec::with_capacity(len))) }
    fn serialize_tuple_variant(self, _: &'static str, idx: u32, _: &'static str, len: usize) -> Result<NodeBuilder, TreeError> {
        let mut b = NodeBuilder(Vec::with_capacity(len + 1));
        b.0.push(Node::U64(idx as u64));
        Ok(b)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<MapBuilder, TreeError> { Ok(MapBuilder(Vec::new())) }
    fn serialize_struct(self, _: &'static str, len: usize) -> Result<NodeBuilder, TreeError> { Ok(NodeBuilder(Vec::with_capacity(len))) }
    fn serialize_struct_variant(self, _: &'static str, idx: u32, _: &'static str, len: usize) -> Result<NodeBuilder, TreeError> {
        let mut b = NodeBuilder(Vec::with_capacity(len + 1));
        b.0.push(Node::U64(idx as u64));
        Ok(b)
    }
}

struct NodeBuilder(Vec<Node>);
struct MapBuilder(Vec<(Node, Node)>);

impl ser::SerializeSeq for NodeBuilder {
    type Ok = Node; type Error = TreeError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), TreeError> {
        self.0.push(v.serialize(NodeSerializer)?); Ok(())
    }
    fn end(self) -> Result<Node, TreeError> { Ok(Node::Seq(self.0)) }
}
impl ser::SerializeTuple for NodeBuilder {
    type Ok = Node; type Error = TreeError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), TreeError> {
        self.0.push(v.serialize(NodeSerializer)?); Ok(())
    }
    fn end(self) -> Result<Node, TreeError> { Ok(Node::Seq(self.0)) }
}
impl ser::SerializeTupleStruct for NodeBuilder {
    type Ok = Node; type Error = TreeError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), TreeError> {
        self.0.push(v.serialize(NodeSerializer)?); Ok(())
    }
    fn end(self) -> Result<Node, TreeError> { Ok(Node::Seq(self.0)) }
}
impl ser::SerializeTupleVariant for NodeBuilder {
    type Ok = Node; type Error = TreeError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), TreeError> {
        self.0.push(v.serialize(NodeSerializer)?); Ok(())
    }
    fn end(self) -> Result<Node, TreeError> { Ok(Node::Seq(self.0)) }
}
impl ser::SerializeMap for MapBuilder {
    type Ok = Node; type Error = TreeError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), TreeError> {
        self.0.push((key.serialize(NodeSerializer)?, Node::None)); Ok(())
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), TreeError> {
        self.0.last_mut().unwrap().1 = v.serialize(NodeSerializer)?; Ok(())
    }
    fn end(self) -> Result<Node, TreeError> {
        let flat: Vec<Node> = self.0.into_iter().flat_map(|(k, v)| vec![k, v]).collect();
        Ok(Node::Seq(flat))
    }
}
impl ser::SerializeStruct for NodeBuilder {
    type Ok = Node; type Error = TreeError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, _name: &'static str, v: &T) -> Result<(), TreeError> {
        self.0.push(v.serialize(NodeSerializer)?); Ok(())
    }
    fn end(self) -> Result<Node, TreeError> { Ok(Node::Struct(self.0)) }
}
impl ser::SerializeStructVariant for NodeBuilder {
    type Ok = Node; type Error = TreeError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, _name: &'static str, v: &T) -> Result<(), TreeError> {
        self.0.push(v.serialize(NodeSerializer)?); Ok(())
    }
    fn end(self) -> Result<Node, TreeError> { Ok(Node::Seq(self.0)) }
}

// ── Deserializer (Node → any Deserialize impl) ────────────────────────────

impl<'de> Deserializer<'de> for Node {
    type Error = TreeError;
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TreeError> {
        match self {
            Node::None => visitor.visit_unit(),
            Node::Bool(v) => visitor.visit_bool(v),
            Node::U64(v) => visitor.visit_u64(v),
            Node::I64(v) => visitor.visit_i64(v),
            Node::F64(v) => visitor.visit_f64(v),
            Node::Str(v) => visitor.visit_string(v),
            Node::Bytes(v) => visitor.visit_byte_buf(v),
            Node::Seq(v) | Node::Struct(v) => visitor.visit_seq(SeqIter(v.into_iter())),
        }
    }
    fn deserialize_struct<V: Visitor<'de>>(self, _name: &'static str, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, TreeError> {
        self.deserialize_any(visitor)
    }
    fn deserialize_enum<V: Visitor<'de>>(self, _name: &'static str, _variants: &'static [&'static str], visitor: V) -> Result<V::Value, TreeError> {
        match self {
            Node::Struct(mut items) | Node::Seq(mut items) if !items.is_empty() => {
                let tag = items.remove(0);
                let idx = match tag {
                    Node::U64(v) => v,
                    Node::I64(v) => v as u64,
                    _ => return Err(TreeError("expected U64 variant tag".into())),
                };
                visitor.visit_enum(EnumAccessor { idx, items })
            }
            ref other => {
                other.clone().deserialize_any(visitor)
            }
        }
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, TreeError> {
        self.deserialize_any(visitor)
    }
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TreeError> {
        self.deserialize_any(visitor)
    }
    fn deserialize_tuple<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, TreeError> {
        self.deserialize_any(visitor)
    }
    fn deserialize_tuple_struct<V: Visitor<'de>>(self, _name: &'static str, _len: usize, visitor: V) -> Result<V::Value, TreeError> {
        self.deserialize_any(visitor)
    }
    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TreeError> {
        self.deserialize_any(visitor)
    }
    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TreeError> {
        self.deserialize_any(visitor)
    }
    fn deserialize_bool<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_i8<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_i16<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_i32<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_i64<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_u8<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_u16<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_u32<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_u64<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_f32<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_f64<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_char<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_str<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_string<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_bytes<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_byte_buf<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_option<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_unit<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_unit_struct<V: Visitor<'de>>(self, _n: &'static str, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
    fn deserialize_identifier<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TreeError> { self.deserialize_any(v) }
}

struct SeqIter(std::vec::IntoIter<Node>);

impl<'de> SeqAccess<'de> for SeqIter {
    type Error = TreeError;
    fn next_element_seed<T: de::DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, TreeError> {
        match self.0.next() {
            Some(node) => seed.deserialize(node).map(Some),
            None => Ok(None),
        }
    }
}

// ── Enum accessor for Node ───────────────────────────────────────────────

struct EnumAccessor {
    idx: u64,
    items: Vec<Node>,
}

impl<'de> de::EnumAccess<'de> for EnumAccessor {
    type Error = TreeError;
    type Variant = VariantAccessor;
    fn variant_seed<V: de::DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, VariantAccessor), TreeError> {
        let val = seed.deserialize(Node::U64(self.idx))?;
        Ok((val, VariantAccessor(self.items.into_iter())))
    }
}

struct VariantAccessor(std::vec::IntoIter<Node>);

impl<'de> de::VariantAccess<'de> for VariantAccessor {
    type Error = TreeError;

    fn unit_variant(self) -> Result<(), TreeError> { Ok(()) }

    fn newtype_variant_seed<T: de::DeserializeSeed<'de>>(mut self, seed: T) -> Result<T::Value, TreeError> {
        let node = self.0.next().ok_or_else(|| TreeError("missing newtype data".into()))?;
        seed.deserialize(node)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, TreeError> {
        visitor.visit_seq(SeqIter(self.0))
    }

    fn struct_variant<V: Visitor<'de>>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, TreeError> {
        visitor.visit_seq(SeqIter(self.0))
    }
}

// ── Diff / Patch ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Patch {
    Same,
    Replace(Node),
    Struct(Vec<Option<Patch>>),
    Seq(Vec<Patch>),
}

pub fn diff(old: &Node, new: &Node) -> Patch {
    match (old, new) {
        (Node::Struct(a), Node::Struct(b)) => field_diff(a, b, true),
        (Node::Seq(a), Node::Seq(b)) => {
            match field_diff(a, b, false) {
                Patch::Struct(f) => Patch::Seq(f.into_iter().map(|o| o.unwrap_or(Patch::Same)).collect()),
                other => other,
            }
        }
        (a, b) if a == b => Patch::Same,
        (_, b) => Patch::Replace(b.clone()),
    }
}

fn field_diff(a: &[Node], b: &[Node], _is_struct: bool) -> Patch {
    let max = a.len().max(b.len());
    let mut fields: Vec<Option<Patch>> = Vec::with_capacity(max);
    for i in 0..max {
        match (a.get(i), b.get(i)) {
            (_, None) => {}
            (None, Some(v)) => fields.push(Some(Patch::Replace(v.clone()))),
            (Some(pa), Some(pb)) => {
                let p = diff(pa, pb);
                fields.push(if p == Patch::Same { None } else { Some(p) });
            }
        }
    }
    while fields.last() == Some(&None) { fields.pop(); }
    if fields.iter().all(|f| f.is_none()) { Patch::Same }
    else { Patch::Struct(fields) }
}

pub fn apply(node: &mut Node, patch: &Patch) {
    match (node, patch) {
        (_, Patch::Same) => {}
        (n, Patch::Replace(v)) => *n = v.clone(),
        (Node::Struct(fields), Patch::Struct(patches))
        | (Node::Seq(fields), Patch::Struct(patches)) => {
            let max = fields.len().max(patches.len());
            if max > fields.len() { fields.resize(max, Node::None); }
            for (i, p) in patches.iter().enumerate() {
                if let Some(p) = p { apply(&mut fields[i], p); }
            }
        }
        (Node::Seq(fields), Patch::Seq(patches)) => {
            let max = fields.len().max(patches.len());
            if max > fields.len() { fields.resize(max, Node::None); }
            for i in 0..patches.len() { apply(&mut fields[i], &patches[i]); }
        }
        _ => {}
    }
}

// ── Convenience ──────────────────────────────────────────────────────────

pub fn diff_values<T: Serialize>(old: &T, new: &T) -> Patch {
    diff(&to_node(old), &to_node(new))
}

pub fn apply_patch<T: DeserializeOwned + Serialize>(value: &T, patch: &Patch) -> T {
    let mut node = to_node(value);
    apply(&mut node, patch);
    T::deserialize(node).unwrap()
}

pub fn patch_size(patch: &Patch) -> usize {
    postcard::to_allocvec(patch).unwrap().len()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
    struct Inner { values: Vec<f32>, name: String }

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
    struct Outer { id: u32, inner: Inner, active: bool }

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
    struct Snapshot { tick: u32, entities: Vec<Outer> }

    #[test]
    fn identical() {
        let a = Outer { id: 1, inner: Inner { values: vec![1.0, 2.0], name: "a".into() }, active: true };
        assert_eq!(diff_values(&a, &a), Patch::Same);
    }

    #[test]
    fn field_change() {
        let a = Outer { id: 1, inner: Inner { values: vec![1.0;3], name: "x".into() }, active: true };
        let b = Outer { id: 2, inner: Inner { values: vec![1.0;3], name: "x".into() }, active: true };
        assert_eq!(apply_patch(&a, &diff_values(&a, &b)), b);
    }

    #[test]
    fn nested_change() {
        let a = Outer { id: 1, inner: Inner { values: vec![1.0, 2.0], name: "hi".into() }, active: true };
        let b = Outer { id: 1, inner: Inner { values: vec![1.0, 9.0], name: "there".into() }, active: true };
        assert_eq!(apply_patch(&a, &diff_values(&a, &b)), b);
    }

    #[test]
    fn vec_appended() {
        let mk = |n| Outer { id: n, inner: Inner { values: vec![], name: "".into() }, active: true };
        let a = Snapshot { tick: 0, entities: vec![mk(1)] };
        let b = Snapshot { tick: 0, entities: vec![mk(1), mk(2)] };
        assert_eq!(apply_patch(&a, &diff_values(&a, &b)), b);
    }

    #[test]
    fn vec_strings_changed() {
        let a: Vec<String> = vec!["a".into(), "b".into()];
        let b: Vec<String> = vec!["a".into(), "c".into()];
        assert_eq!(apply_patch(&a, &diff_values(&a, &b)), b);
    }

    #[test]
    fn no_string_keys_in_tree() {
        let v = Outer { id: 42, inner: Inner { values: vec![], name: "t".into() }, active: false };
        let n = to_node(&v);
        match &n {
            Node::Struct(f) => {
                assert_eq!(f.len(), 3);
                assert_eq!(f[0], Node::U64(42));
                assert!(matches!(f[1], Node::Struct(_)));
                assert_eq!(f[2], Node::Bool(false));
            }
            _ => panic!("expected Struct"),
        }
    }

    #[test]
    fn patch_is_tiny_when_identical() {
        let a = Outer { id: 1, inner: Inner { values: vec![0.0; 100], name: "x".into() }, active: true };
        let sz = patch_size(&diff_values(&a, &a));
        assert!(sz <= 3, "identical patch should be ≤3 bytes (Same variant), got {sz}");
    }

    #[test]
    fn patch_sparse_is_small() {
        let a = Outer { id: 1, inner: Inner { values: vec![0.0; 100], name: "unchanged".into() }, active: true };
        let mut b = a.clone(); b.id = 99;
        let sz = patch_size(&diff_values(&a, &b));
        assert!(sz < 30, "sparse patch should be small, got {sz}");
    }

    #[test]
    fn round_trip_postcard() {
        let v = Outer { id: 42, inner: Inner { values: vec![1.0, 2.0], name: "test".into() }, active: true };
        let n = to_node(&v);
        let bytes = postcard::to_allocvec(&n).unwrap();
        let n2: Node = postcard::from_bytes(&bytes).unwrap();
        let back: Outer = Outer::deserialize(n2).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn apply_after_postcard_round_trip() {
        let a = Outer { id: 1, inner: Inner { values: vec![1.0, 2.0], name: "hi".into() }, active: true };
        let b = Outer { id: 9, inner: Inner { values: vec![1.0, 2.0], name: "hi".into() }, active: true };
        let p = diff_values(&a, &b);
        let p_bytes = postcard::to_allocvec(&p).unwrap();
        let p_restored: Patch = postcard::from_bytes(&p_bytes).unwrap();
        assert_eq!(apply_patch(&a, &p_restored), b);
    }
}
