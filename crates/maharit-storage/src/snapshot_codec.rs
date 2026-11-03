//! Encoder / decoder for the v4 snapshot payload.
//!
//! v4 files were written with `bincode` 1 (`bincode::serialize`), which is no
//! longer maintained. This module reads and writes exactly the same bytes, so
//! existing database files stay readable and newly written ones are identical:
//!
//! - integers and floats: fixed width, little endian (`f64` as its bits)
//! - `bool`: one byte, `0` or `1`
//! - `String` / `Vec` / `HashMap`: `u64` element count, then the elements
//!   (a string's elements are its UTF-8 bytes)
//! - enum: `u32` variant index, then the variant's fields
//! - struct: its fields in declaration order
//!
//! Decoding checks every length against the bytes that remain, so a corrupted
//! or hostile file fails with an error instead of a huge allocation.

use std::collections::HashMap;

use maharit_core::PropertyValue;

/// A node as stored in the snapshot.
#[derive(Debug)]
pub(crate) struct SNode {
    pub id: u64,
    pub labels: Vec<String>,
    pub properties: HashMap<String, PropertyValue>,
}

/// A relationship as stored in the snapshot.
#[derive(Debug)]
pub(crate) struct SEdge {
    pub id: u64,
    pub label: String,
    pub from: u64,
    pub to: u64,
    pub properties: HashMap<String, PropertyValue>,
}

/// A property index definition as stored in the snapshot.
#[derive(Debug)]
pub(crate) struct SIndex {
    pub label: String,
    pub property: String,
    pub unique: bool,
}

/// The whole v4 payload.
#[derive(Debug)]
pub(crate) struct SSnapshot {
    pub nodes: Vec<SNode>,
    pub edges: Vec<SEdge>,
    pub indexes: Vec<SIndex>,
}

// Variant indexes of `PropertyValue`, in declaration order (serde's derive).
const PV_NULL: u32 = 0;
const PV_BOOL: u32 = 1;
const PV_INT: u32 = 2;
const PV_FLOAT: u32 = 3;
const PV_STRING: u32 = 4;
const PV_DATE: u32 = 5;
const PV_DATETIME: u32 = 6;
const PV_DURATION: u32 = 7;

// ==================== encoding ====================

pub(crate) fn encode(snapshot: &SSnapshot) -> Vec<u8> {
    let mut out = Vec::new();
    put_len(&mut out, snapshot.nodes.len());
    for n in &snapshot.nodes {
        put_u64(&mut out, n.id);
        put_len(&mut out, n.labels.len());
        for l in &n.labels {
            put_str(&mut out, l);
        }
        put_props(&mut out, &n.properties);
    }
    put_len(&mut out, snapshot.edges.len());
    for e in &snapshot.edges {
        put_u64(&mut out, e.id);
        put_str(&mut out, &e.label);
        put_u64(&mut out, e.from);
        put_u64(&mut out, e.to);
        put_props(&mut out, &e.properties);
    }
    put_len(&mut out, snapshot.indexes.len());
    for i in &snapshot.indexes {
        put_str(&mut out, &i.label);
        put_str(&mut out, &i.property);
        out.push(i.unique as u8);
    }
    out
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_len(out: &mut Vec<u8>, len: usize) {
    put_u64(out, len as u64);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_len(out, s.len());
    out.extend_from_slice(s.as_bytes());
}

fn put_props(out: &mut Vec<u8>, props: &HashMap<String, PropertyValue>) {
    put_len(out, props.len());
    for (k, v) in props {
        put_str(out, k);
        put_value(out, v);
    }
}

fn put_value(out: &mut Vec<u8>, v: &PropertyValue) {
    let tag = |out: &mut Vec<u8>, t: u32| out.extend_from_slice(&t.to_le_bytes());
    match v {
        PropertyValue::Null => tag(out, PV_NULL),
        PropertyValue::Bool(b) => {
            tag(out, PV_BOOL);
            out.push(*b as u8);
        }
        PropertyValue::Int(n) => {
            tag(out, PV_INT);
            out.extend_from_slice(&n.to_le_bytes());
        }
        PropertyValue::Float(f) => {
            tag(out, PV_FLOAT);
            out.extend_from_slice(&f.to_bits().to_le_bytes());
        }
        PropertyValue::String(s) => {
            tag(out, PV_STRING);
            put_str(out, s);
        }
        PropertyValue::Date(d) => {
            tag(out, PV_DATE);
            out.extend_from_slice(&d.to_le_bytes());
        }
        PropertyValue::DateTime(ms) => {
            tag(out, PV_DATETIME);
            out.extend_from_slice(&ms.to_le_bytes());
        }
        PropertyValue::Duration {
            months,
            days,
            millis,
        } => {
            tag(out, PV_DURATION);
            out.extend_from_slice(&months.to_le_bytes());
            out.extend_from_slice(&days.to_le_bytes());
            out.extend_from_slice(&millis.to_le_bytes());
        }
    }
}

// ==================== decoding ====================

/// Decode a v4 payload. The error describes where the data went wrong.
pub(crate) fn decode(bytes: &[u8]) -> Result<SSnapshot, String> {
    let mut d = Decoder { bytes, pos: 0 };
    // Minimum encoded sizes, used to bound element counts before allocating.
    const MIN_NODE: usize = 8 + 8 + 8; // id, label count, property count
    const MIN_EDGE: usize = 8 + 8 + 8 + 8 + 8; // id, label, from, to, property count
    const MIN_INDEX: usize = 8 + 8 + 1;

    let node_count = d.count(MIN_NODE, "node list")?;
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let id = d.u64()?;
        let label_count = d.count(8, "label list")?;
        let mut labels = Vec::with_capacity(label_count);
        for _ in 0..label_count {
            labels.push(d.string()?);
        }
        let properties = d.props()?;
        nodes.push(SNode {
            id,
            labels,
            properties,
        });
    }

    let edge_count = d.count(MIN_EDGE, "edge list")?;
    let mut edges = Vec::with_capacity(edge_count);
    for _ in 0..edge_count {
        edges.push(SEdge {
            id: d.u64()?,
            label: d.string()?,
            from: d.u64()?,
            to: d.u64()?,
            properties: d.props()?,
        });
    }

    let index_count = d.count(MIN_INDEX, "index list")?;
    let mut indexes = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        indexes.push(SIndex {
            label: d.string()?,
            property: d.string()?,
            unique: d.bool()?,
        });
    }

    if d.pos != bytes.len() {
        return Err(format!(
            "{} trailing byte(s) after the snapshot",
            bytes.len() - d.pos
        ));
    }
    Ok(SSnapshot {
        nodes,
        edges,
        indexes,
    })
}

struct Decoder<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Decoder<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let end = self.pos.checked_add(N).filter(|&e| e <= self.bytes.len());
        let Some(end) = end else {
            return Err(format!("unexpected end of data at byte {}", self.pos));
        };
        let mut buf = [0u8; N];
        buf.copy_from_slice(&self.bytes[self.pos..end]);
        self.pos = end;
        Ok(buf)
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.take::<4>().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, String> {
        self.take::<8>().map(u64::from_le_bytes)
    }

    fn i32(&mut self) -> Result<i32, String> {
        self.take::<4>().map(i32::from_le_bytes)
    }

    fn i64(&mut self) -> Result<i64, String> {
        self.take::<8>().map(i64::from_le_bytes)
    }

    fn bool(&mut self) -> Result<bool, String> {
        match self.take::<1>()?[0] {
            0 => Ok(false),
            1 => Ok(true),
            b => Err(format!("invalid bool byte {b} at {}", self.pos - 1)),
        }
    }

    /// An element count whose elements take at least `min_size` bytes each.
    fn count(&mut self, min_size: usize, what: &str) -> Result<usize, String> {
        let at = self.pos;
        let n = self.u64()?;
        let remaining = (self.bytes.len() - self.pos) as u64;
        if n.saturating_mul(min_size as u64) > remaining {
            return Err(format!(
                "{what} at byte {at} claims {n} element(s) but only {remaining} byte(s) remain"
            ));
        }
        Ok(n as usize)
    }

    fn string(&mut self) -> Result<String, String> {
        let at = self.pos;
        let len = self.count(1, "string")?;
        let raw = &self.bytes[self.pos..self.pos + len];
        self.pos += len;
        String::from_utf8(raw.to_vec()).map_err(|_| format!("invalid UTF-8 in string at byte {at}"))
    }

    fn props(&mut self) -> Result<HashMap<String, PropertyValue>, String> {
        // Key length prefix + variant tag.
        let n = self.count(8 + 4, "property map")?;
        let mut map = HashMap::with_capacity(n);
        for _ in 0..n {
            let key = self.string()?;
            let value = self.value()?;
            map.insert(key, value);
        }
        Ok(map)
    }

    fn value(&mut self) -> Result<PropertyValue, String> {
        let at = self.pos;
        Ok(match self.u32()? {
            PV_NULL => PropertyValue::Null,
            PV_BOOL => PropertyValue::Bool(self.bool()?),
            PV_INT => PropertyValue::Int(self.i64()?),
            PV_FLOAT => PropertyValue::Float(f64::from_bits(self.u64()?)),
            PV_STRING => PropertyValue::String(self.string()?),
            PV_DATE => PropertyValue::Date(self.i32()?),
            PV_DATETIME => PropertyValue::DateTime(self.i64()?),
            PV_DURATION => PropertyValue::Duration {
                months: self.i32()?,
                days: self.i32()?,
                millis: self.i64()?,
            },
            tag => return Err(format!("unknown property type {tag} at byte {at}")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_many_properties() {
        let props: HashMap<String, PropertyValue> = (0..50)
            .map(|i| (format!("k{i}"), PropertyValue::Int(i)))
            .chain([("s".to_string(), PropertyValue::String(String::new()))])
            .collect();
        let snapshot = SSnapshot {
            nodes: vec![SNode {
                id: 7,
                labels: vec!["A".into(), "B".into()],
                properties: props.clone(),
            }],
            edges: vec![SEdge {
                id: 3,
                label: "R".into(),
                from: 7,
                to: 7,
                properties: HashMap::new(),
            }],
            indexes: vec![SIndex {
                label: "A".into(),
                property: "k1".into(),
                unique: false,
            }],
        };
        let decoded = decode(&encode(&snapshot)).unwrap();
        assert_eq!(decoded.nodes[0].properties, props);
        assert_eq!(decoded.nodes[0].labels, vec!["A", "B"]);
        assert_eq!(decoded.edges[0].from, 7);
        assert!(!decoded.indexes[0].unique);
    }

    #[test]
    fn rejects_invalid_bool_and_utf8() {
        let mut bytes = encode(&SSnapshot {
            nodes: vec![],
            edges: vec![],
            indexes: vec![SIndex {
                label: "L".into(),
                property: "p".into(),
                unique: true,
            }],
        });
        let last = bytes.len() - 1;
        bytes[last] = 2;
        assert!(decode(&bytes).unwrap_err().contains("invalid bool"));
        bytes[last] = 1;
        let label_byte = 8 + 8 + 8 + 8; // node count, edge count, index count, label length
        bytes[label_byte] = 0xff;
        assert!(decode(&bytes).unwrap_err().contains("UTF-8"));
    }
}
