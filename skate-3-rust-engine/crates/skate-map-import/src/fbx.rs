//! Binary FBX 7.x reader. ASCII FBX is rejected; Blender's binary export is the path we support.
use std::io::Read;

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug)]
pub(crate) enum Prop {
    I16(i16),
    Bool(bool),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Bytes(Vec<u8>),
    F32s(Vec<f32>),
    F64s(Vec<f64>),
    I32s(Vec<i32>),
    I64s(Vec<i64>),
    Bools(Vec<bool>),
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|node| node.name == name)
    }
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |node| node.name == name)
    }
    pub fn string(&self, index: usize) -> Option<String> {
        match self.props.get(index)? {
            Prop::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        }
    }
    pub fn i64(&self, index: usize) -> Option<i64> {
        match self.props.get(index)? {
            Prop::I64(value) => Some(*value),
            Prop::I32(value) => Some(i64::from(*value)),
            _ => None,
        }
    }
    pub fn f64s(&self) -> Option<&[f64]> {
        self.props.iter().find_map(|prop| match prop {
            Prop::F64s(values) => Some(values.as_slice()),
            _ => None,
        })
    }
    pub fn f32s(&self) -> Option<&[f32]> {
        self.props.iter().find_map(|prop| match prop {
            Prop::F32s(values) => Some(values.as_slice()),
            _ => None,
        })
    }
    pub fn i32s(&self) -> Option<&[i32]> {
        self.props.iter().find_map(|prop| match prop {
            Prop::I32s(values) => Some(values.as_slice()),
            _ => None,
        })
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        self.props.iter().find_map(|prop| match prop {
            Prop::Bytes(values) => Some(values.as_slice()),
            _ => None,
        })
    }
}

const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";
const _: () = assert!(MAGIC.len() == 23);

pub(crate) fn parse(data: &[u8]) -> Result<Vec<Node>, String> {
    if data.starts_with(b"; FBX") {
        return Err("This is an ASCII FBX file. Export Binary FBX from Blender and import that.".into());
    }
    if data.len() < 27 || &data[..MAGIC.len()] != MAGIC {
        return Err("Not a binary FBX file".into());
    }
    let version = u32::from_le_bytes(data[23..27].try_into().unwrap());
    if !(7000..8000).contains(&version) {
        return Err(format!("Unsupported FBX version {version}"));
    }
    let big = version >= 7500;
    let mut cursor = 27usize;
    let mut nodes = Vec::new();
    while cursor + header_size(big) <= data.len() {
        match read_node(data, &mut cursor, big)? {
            Some(node) => nodes.push(node),
            None => break,
        }
    }
    if nodes.is_empty() {
        return Err("FBX file has no objects".into());
    }
    Ok(nodes)
}

fn header_size(big: bool) -> usize {
    if big { 25 } else { 13 }
}

fn read_node(data: &[u8], cursor: &mut usize, big: bool) -> Result<Option<Node>, String> {
    let header = header_size(big);
    if *cursor + header > data.len() {
        return Err("Truncated FBX node header".into());
    }
    let (end, count, name_len) = if big {
        let end = u64::from_le_bytes(data[*cursor..*cursor + 8].try_into().unwrap()) as usize;
        let count = u64::from_le_bytes(data[*cursor + 8..*cursor + 16].try_into().unwrap()) as usize;
        let name_len = data[*cursor + 24] as usize;
        (end, count, name_len)
    } else {
        let end = u32::from_le_bytes(data[*cursor..*cursor + 4].try_into().unwrap()) as usize;
        let count = u32::from_le_bytes(data[*cursor + 4..*cursor + 8].try_into().unwrap()) as usize;
        let name_len = data[*cursor + 12] as usize;
        (end, count, name_len)
    };
    *cursor += header;
    if end == 0 {
        return Ok(None);
    }
    if end > data.len() || *cursor + name_len > data.len() {
        return Err("FBX node points past the end of the file".into());
    }
    let name = String::from_utf8_lossy(&data[*cursor..*cursor + name_len]).into_owned();
    *cursor += name_len;
    let mut props = Vec::with_capacity(count);
    for _ in 0..count {
        props.push(read_prop(data, cursor)?);
    }
    let mut children = Vec::new();
    while *cursor + header < end {
        match read_node(data, cursor, big)? {
            Some(child) => children.push(child),
            None => break,
        }
    }
    *cursor = end;
    Ok(Some(Node { name, props, children }))
}

fn read_prop(data: &[u8], cursor: &mut usize) -> Result<Prop, String> {
    let code = *data.get(*cursor).ok_or("Truncated FBX property")?;
    *cursor += 1;
    let take = |cursor: &mut usize, n: usize| -> Result<&[u8], String> {
        let end = cursor.checked_add(n).ok_or("FBX property size overflow")?;
        let bytes = data.get(*cursor..end).ok_or("Truncated FBX property")?;
        *cursor = end;
        Ok(bytes)
    };
    Ok(match code {
        b'Y' => Prop::I16(i16::from_le_bytes(take(cursor, 2)?.try_into().unwrap())),
        b'C' => Prop::Bool(take(cursor, 1)?[0] != 0),
        b'I' => Prop::I32(i32::from_le_bytes(take(cursor, 4)?.try_into().unwrap())),
        b'F' => Prop::F32(f32::from_le_bytes(take(cursor, 4)?.try_into().unwrap())),
        b'D' => Prop::F64(f64::from_le_bytes(take(cursor, 8)?.try_into().unwrap())),
        b'L' => Prop::I64(i64::from_le_bytes(take(cursor, 8)?.try_into().unwrap())),
        b'S' | b'R' => {
            let n = u32::from_le_bytes(take(cursor, 4)?.try_into().unwrap()) as usize;
            Prop::Bytes(take(cursor, n)?.to_vec())
        }
        b'f' | b'd' | b'i' | b'l' | b'b' => read_array(code, data, cursor, take)?,
        _ => return Err(format!("Unsupported FBX property type '{}'", code as char)),
    })
}

fn read_array<'a>(
    code: u8,
    _data: &'a [u8],
    cursor: &mut usize,
    take: impl Fn(&mut usize, usize) -> Result<&'a [u8], String>,
) -> Result<Prop, String> {
    let count = u32::from_le_bytes(take(cursor, 4)?.try_into().unwrap()) as usize;
    let encoding = u32::from_le_bytes(take(cursor, 4)?.try_into().unwrap());
    let compressed = u32::from_le_bytes(take(cursor, 4)?.try_into().unwrap()) as usize;
    let payload = take(cursor, compressed)?.to_vec();
    let raw = match encoding {
        0 => payload,
        1 => inflate(&payload)?,
        _ => return Err(format!("Unsupported FBX array encoding {encoding}")),
    };
    let width = match code {
        b'f' | b'i' => 4,
        b'd' | b'l' => 8,
        b'b' => 1,
        _ => 0,
    };
    if raw.len() != count * width {
        return Err("FBX array decompressed to an unexpected size".into());
    }
    Ok(match code {
        b'f' => Prop::F32s(raw.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect()),
        b'd' => Prop::F64s(raw.chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap())).collect()),
        b'i' => Prop::I32s(raw.chunks_exact(4).map(|b| i32::from_le_bytes(b.try_into().unwrap())).collect()),
        b'l' => Prop::I64s(raw.chunks_exact(8).map(|b| i64::from_le_bytes(b.try_into().unwrap())).collect()),
        b'b' => Prop::Bools(raw.iter().map(|b| *b != 0).collect()),
        _ => unreachable!(),
    })
}

/// Test helper. Writes the same binary layout `parse` reads.
#[cfg(test)]
pub(crate) fn encode(nodes: &[Node], version: u32) -> Vec<u8> {
    let big = version >= 7500;
    let mut data = MAGIC.to_vec();
    data.extend_from_slice(&version.to_le_bytes());
    for node in nodes {
        write_node(&mut data, node, big);
    }
    data.extend(std::iter::repeat(0).take(header_size(big)));
    data
}

#[cfg(test)]
fn write_node(data: &mut Vec<u8>, node: &Node, big: bool) {
    let start = data.len();
    let header = header_size(big);
    data.extend(std::iter::repeat(0).take(header));
    let name_at = start + header - 1;
    data[name_at] = u8::try_from(node.name.len()).expect("FBX node name fits in one byte");
    data.extend_from_slice(node.name.as_bytes());
    let props_at = data.len();
    for prop in &node.props {
        write_prop(data, prop);
    }
    let props_len = data.len() - props_at;
    for child in &node.children {
        write_node(data, child, big);
    }
    data.extend(std::iter::repeat(0).take(header));
    let end = data.len();
    if big {
        data[start..start + 8].copy_from_slice(&(end as u64).to_le_bytes());
        data[start + 8..start + 16].copy_from_slice(&(node.props.len() as u64).to_le_bytes());
        data[start + 16..start + 24].copy_from_slice(&(props_len as u64).to_le_bytes());
    } else {
        data[start..start + 4].copy_from_slice(&(end as u32).to_le_bytes());
        data[start + 4..start + 8].copy_from_slice(&(node.props.len() as u32).to_le_bytes());
        data[start + 8..start + 12].copy_from_slice(&(props_len as u32).to_le_bytes());
    }
}

#[cfg(test)]
fn write_prop(data: &mut Vec<u8>, prop: &Prop) {
    match prop {
        Prop::I16(value) => {
            data.push(b'Y');
            data.extend_from_slice(&value.to_le_bytes());
        }
        Prop::Bool(value) => {
            data.push(b'C');
            data.push(u8::from(*value));
        }
        Prop::I32(value) => {
            data.push(b'I');
            data.extend_from_slice(&value.to_le_bytes());
        }
        Prop::I64(value) => {
            data.push(b'L');
            data.extend_from_slice(&value.to_le_bytes());
        }
        Prop::F32(value) => {
            data.push(b'F');
            data.extend_from_slice(&value.to_le_bytes());
        }
        Prop::F64(value) => {
            data.push(b'D');
            data.extend_from_slice(&value.to_le_bytes());
        }
        Prop::Bytes(value) => {
            data.push(b'S');
            data.extend_from_slice(&(value.len() as u32).to_le_bytes());
            data.extend_from_slice(value);
        }
        Prop::F32s(values) => write_array(data, b'f', 4, values.len(), values.iter().flat_map(|v| v.to_le_bytes())),
        Prop::F64s(values) => write_array(data, b'd', 8, values.len(), values.iter().flat_map(|v| v.to_le_bytes())),
        Prop::I32s(values) => write_array(data, b'i', 4, values.len(), values.iter().flat_map(|v| v.to_le_bytes())),
        Prop::I64s(values) => write_array(data, b'l', 8, values.len(), values.iter().flat_map(|v| v.to_le_bytes())),
        Prop::Bools(values) => write_array(data, b'b', 1, values.len(), values.iter().map(|v| u8::from(*v))),
    }
}

#[cfg(test)]
fn write_array(data: &mut Vec<u8>, code: u8, width: usize, count: usize, bytes: impl IntoIterator<Item = u8>) {
    let raw: Vec<u8> = bytes.into_iter().collect();
    debug_assert_eq!(raw.len(), count * width);
    data.push(code);
    data.extend_from_slice(&(count as u32).to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    data.extend_from_slice(&raw);
}

fn inflate(payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut zlib = Vec::new();
    if flate2::read::ZlibDecoder::new(payload).read_to_end(&mut zlib).is_ok() {
        return Ok(zlib);
    }
    let mut raw = Vec::new();
    flate2::read::DeflateDecoder::new(payload)
        .read_to_end(&mut raw)
        .map_err(|e| format!("FBX array decompression failed: {e}"))?;
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop_f64s(values: &[f64]) -> Vec<u8> {
        let mut bytes = vec![b'd'];
        bytes.extend_from_slice(&(values.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        bytes.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&raw);
        bytes
    }

    #[test]
    fn reads_a_vertex_node() {
        let props = prop_f64s(&[1.0, 2.0, 3.0]);
        let name = b"Vertices";
        let body_end = 27 + 13 + name.len() + props.len() + 13;
        let mut data = MAGIC.to_vec();
        data.extend_from_slice(&7400u32.to_le_bytes());
        data.extend_from_slice(&(body_end as u32).to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&(props.len() as u32).to_le_bytes());
        data.push(name.len() as u8);
        data.extend_from_slice(name);
        data.extend_from_slice(&props);
        data.extend_from_slice(&[0u8; 13]);
        data.extend_from_slice(&[0u8; 13]);
        let nodes = parse(&data).unwrap();
        assert_eq!(nodes[0].name, "Vertices");
        assert_eq!(nodes[0].f64s().unwrap(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn rejects_ascii() {
        let error = parse(b"; FBX 7.4.0 project file").unwrap_err();
        assert!(error.contains("ASCII"), "{error}");
    }

    #[test]
    fn reads_a_zlib_compressed_array() {
        let values = [1.0f64, 2.0, 3.0];
        let raw: Vec<u8> = values.iter().flat_map(|value| value.to_le_bytes()).collect();
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &raw).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut prop = vec![b'd'];
        prop.extend_from_slice(&(values.len() as u32).to_le_bytes());
        prop.extend_from_slice(&1u32.to_le_bytes());
        prop.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        prop.extend_from_slice(&compressed);
        let name = b"Vertices";
        let body_end = 27 + 13 + name.len() + prop.len() + 13;
        let mut data = MAGIC.to_vec();
        data.extend_from_slice(&7400u32.to_le_bytes());
        data.extend_from_slice(&(body_end as u32).to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&(prop.len() as u32).to_le_bytes());
        data.push(name.len() as u8);
        data.extend_from_slice(name);
        data.extend_from_slice(&prop);
        data.extend_from_slice(&[0u8; 13]);
        data.extend_from_slice(&[0u8; 13]);
        let nodes = parse(&data).unwrap();
        assert_eq!(nodes[0].f64s().unwrap(), &values);
    }

    #[test]
    fn round_trips_nested_64_bit_nodes() {
        let tree = Node {
            name: "Objects".into(),
            props: Vec::new(),
            children: vec![Node {
                name: "Model".into(),
                props: vec![Prop::I64(7), Prop::F32s(vec![1.5, -2.0])],
                children: Vec::new(),
            }],
        };
        let nodes = parse(&encode(&[tree], 7500)).unwrap();
        assert_eq!(nodes[0].name, "Objects");
        assert_eq!(nodes[0].children[0].name, "Model");
        assert_eq!(nodes[0].children[0].i64(0), Some(7));
        assert_eq!(nodes[0].children[0].f32s().unwrap(), &[1.5, -2.0]);
    }
}
