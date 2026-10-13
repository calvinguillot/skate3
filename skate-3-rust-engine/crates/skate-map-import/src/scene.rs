//! FBX object graph to triangles in Y-up metres.
use crate::fbx::{Node, Prop};

#[derive(Clone, Debug)]
pub(crate) struct Triangle {
    pub positions: [[f32; 3]; 3],
    pub normals: [[f32; 3]; 3],
    pub uvs: [[f32; 2]; 3],
    pub material: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct TexturePixels {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct Material {
    pub name: String,
    pub color: [f32; 3],
    pub texture: Option<TexturePixels>,
}

#[derive(Clone, Debug)]
pub(crate) struct Scene {
    pub materials: Vec<Material>,
    pub triangles: Vec<Triangle>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
struct Mat4 {
    m: [[f64; 4]; 4],
}

impl Mat4 {
    fn identity() -> Self {
        let mut m = [[0.0; 4]; 4];
        for i in 0..4 { m[i][i] = 1.0; }
        Self { m }
    }
    fn mul(self, other: Self) -> Self {
        let mut m = [[0.0; 4]; 4];
        for row in 0..4 {
            for col in 0..4 {
                m[row][col] = (0..4).map(|k| self.m[row][k] * other.m[k][col]).sum();
            }
        }
        Self { m }
    }
    fn translation(x: f64, y: f64, z: f64) -> Self {
        let mut m = Self::identity();
        m.m[0][3] = x;
        m.m[1][3] = y;
        m.m[2][3] = z;
        m
    }
    fn scale(x: f64, y: f64, z: f64) -> Self {
        let mut m = Self::identity();
        m.m[0][0] = x;
        m.m[1][1] = y;
        m.m[2][2] = z;
        m
    }
    fn rotation_x(degrees: f64) -> Self {
        let (s, c) = (degrees.to_radians().sin(), degrees.to_radians().cos());
        let mut m = Self::identity();
        m.m[1][1] = c; m.m[1][2] = -s; m.m[2][1] = s; m.m[2][2] = c;
        m
    }
    fn rotation_y(degrees: f64) -> Self {
        let (s, c) = (degrees.to_radians().sin(), degrees.to_radians().cos());
        let mut m = Self::identity();
        m.m[0][0] = c; m.m[0][2] = s; m.m[2][0] = -s; m.m[2][2] = c;
        m
    }
    fn rotation_z(degrees: f64) -> Self {
        let (s, c) = (degrees.to_radians().sin(), degrees.to_radians().cos());
        let mut m = Self::identity();
        m.m[0][0] = c; m.m[0][1] = -s; m.m[1][0] = s; m.m[1][1] = c;
        m
    }
    /// FBX euler orders 0..5: XYZ, XZY, YZX, YXZ, ZXY, ZYX. Anything else is XYZ.
    fn euler(order: i32, degrees: [f64; 3]) -> Self {
        let rx = Self::rotation_x(degrees[0]);
        let ry = Self::rotation_y(degrees[1]);
        let rz = Self::rotation_z(degrees[2]);
        match order {
            1 => rx.mul(rz).mul(ry),
            2 => ry.mul(rz).mul(rx),
            3 => ry.mul(rx).mul(rz),
            4 => rz.mul(rx).mul(ry),
            5 => rz.mul(ry).mul(rx),
            _ => rx.mul(ry).mul(rz),
        }
    }
    fn inverse_rotation(self) -> Self {
        let mut out = Self::identity();
        for row in 0..3 {
            for col in 0..3 {
                out.m[row][col] = self.m[col][row];
            }
        }
        out
    }
    fn point(self, p: [f64; 3]) -> [f64; 3] {
        let mut out = [self.m[0][3], self.m[1][3], self.m[2][3]];
        for row in 0..3 {
            out[row] += (0..3).map(|col| self.m[row][col] * p[col]).sum::<f64>();
        }
        out
    }
    fn det3(self) -> f64 {
        let m = &self.m;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }
    fn inverse_transpose_vector(self, v: [f64; 3]) -> [f64; 3] {
        let m = &self.m;
        let a = [
            m[1][1] * m[2][2] - m[1][2] * m[2][1],
            m[1][2] * m[2][0] - m[1][0] * m[2][2],
            m[1][0] * m[2][1] - m[1][1] * m[2][0],
        ];
        let b = [
            m[0][2] * m[2][1] - m[0][1] * m[2][2],
            m[0][0] * m[2][2] - m[0][2] * m[2][0],
            m[0][1] * m[2][0] - m[0][0] * m[2][1],
        ];
        let c = [
            m[0][1] * m[1][2] - m[0][2] * m[1][1],
            m[0][2] * m[1][0] - m[0][0] * m[1][2],
            m[0][0] * m[1][1] - m[0][1] * m[1][0],
        ];
        normalize([
            a[0] * v[0] + b[0] * v[1] + c[0] * v[2],
            a[1] * v[0] + b[1] * v[1] + c[1] * v[2],
            a[2] * v[0] + b[2] * v[1] + c[2] * v[2],
        ])
    }
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length < 1e-12 { [0.0, 1.0, 0.0] } else { v.map(|n| n / length) }
}

struct Axis {
    to_engine: Mat4,
    metres_per_unit: f64,
}

fn axis(roots: &[Node]) -> Axis {
    let mut up = 1i32;
    let mut up_sign = 1i32;
    let mut front = 2i32;
    let mut front_sign = 1i32;
    let mut coord = 0i32;
    let mut coord_sign = 1i32;
    let mut unit = 1.0f64;
    if let Some(settings) = roots.iter().find(|node| node.name == "GlobalSettings") {
        for (name, value) in integers(settings) {
            match name.as_str() {
                "UpAxis" => up = value,
                "UpAxisSign" => up_sign = value,
                "FrontAxis" => front = value,
                "FrontAxisSign" => front_sign = value,
                "CoordAxis" => coord = value,
                "CoordAxisSign" => coord_sign = value,
                _ => {}
            }
        }
        for (name, value) in doubles(settings) {
            if name == "UnitScaleFactor" {
                unit = value;
            }
        }
    }
    let mut columns = [[0.0; 3]; 3];
    let put = |columns: &mut [[f64; 3]; 3], file_axis: i32, sign: i32, engine: [f64; 3]| {
        if (0..3).contains(&file_axis) {
            columns[file_axis as usize] = engine.map(|n| n * f64::from(sign));
        }
    };
    put(&mut columns, coord, coord_sign, [1.0, 0.0, 0.0]);
    put(&mut columns, up, up_sign, [0.0, 1.0, 0.0]);
    put(&mut columns, front, front_sign, [0.0, 0.0, 1.0]);
    let mut to_engine = Mat4::identity();
    for row in 0..3 {
        for col in 0..3 {
            to_engine.m[row][col] = columns[col][row];
        }
    }
    Axis { to_engine, metres_per_unit: unit / 100.0 }
}

fn integers(node: &Node) -> Vec<(String, i32)> {
    properties(node).filter_map(|(name, props)| {
        props.iter().find_map(|prop| match prop {
            Prop::I32(value) => Some((name.clone(), *value)),
            Prop::I64(value) => Some((name.clone(), *value as i32)),
            _ => None,
        })
    }).collect()
}

fn doubles(node: &Node) -> Vec<(String, f64)> {
    properties(node).filter_map(|(name, props)| {
        let values: Vec<f64> = props.iter().filter_map(|prop| match prop {
            Prop::F64(value) => Some(*value),
            Prop::F32(value) => Some(f64::from(*value)),
            _ => None,
        }).collect();
        (values.len() == 1).then(|| (name, values[0]))
    }).collect()
}

fn vec3(node: &Node, name: &str) -> [f64; 3] {
    properties(node).find(|(property, _)| property == name).map(|(_, props)| {
        let values: Vec<f64> = props.iter().filter_map(|prop| match prop {
            Prop::F64(value) => Some(*value),
            Prop::F32(value) => Some(f64::from(*value)),
            _ => None,
        }).collect();
        [values.first().copied().unwrap_or(0.0), values.get(1).copied().unwrap_or(0.0), values.get(2).copied().unwrap_or(0.0)]
    }).unwrap_or([0.0; 3])
}

fn properties(node: &Node) -> impl Iterator<Item = (String, &[Prop])> {
    node.child("Properties70").into_iter().flat_map(|block| {
        block.children_named("P").filter_map(|property| {
            let name = property.string(0)?;
            Some((name, property.props.as_slice()))
        })
    })
}

fn rotation_order(model: &Node) -> i32 {
    integers(model)
        .into_iter()
        .find(|(name, _)| name == "RotationOrder")
        .map(|(_, value)| value)
        .unwrap_or(0)
}

fn local_matrix(model: &Node) -> Mat4 {
    let order = rotation_order(model);
    let t = vec3(model, "Lcl Translation");
    let r = vec3(model, "Lcl Rotation");
    let s = {
        let value = vec3(model, "Lcl Scaling");
        if value == [0.0; 3] { [1.0; 3] } else { value }
    };
    let pre = vec3(model, "PreRotation");
    let post = vec3(model, "PostRotation");
    let pivot = vec3(model, "RotationPivot");
    let offset = vec3(model, "RotationOffset");
    let scale_pivot = vec3(model, "ScalingPivot");
    let scale_offset = vec3(model, "ScalingOffset");
    // T * Roff * Rp * Rpre * R * Rpost^-1 * Rp^-1 * Soff * Sp * S * Sp^-1
    let rotation = Mat4::euler(order, pre)
        .mul(Mat4::euler(order, r))
        .mul(Mat4::euler(order, post).inverse_rotation());
    Mat4::translation(t[0], t[1], t[2])
        .mul(Mat4::translation(offset[0], offset[1], offset[2]))
        .mul(Mat4::translation(pivot[0], pivot[1], pivot[2]))
        .mul(rotation)
        .mul(Mat4::translation(-pivot[0], -pivot[1], -pivot[2]))
        .mul(Mat4::translation(scale_offset[0], scale_offset[1], scale_offset[2]))
        .mul(Mat4::translation(scale_pivot[0], scale_pivot[1], scale_pivot[2]))
        .mul(Mat4::scale(s[0], s[1], s[2]))
        .mul(Mat4::translation(-scale_pivot[0], -scale_pivot[1], -scale_pivot[2]))
}

fn geometric_matrix(model: &Node, order: i32) -> Mat4 {
    let t = vec3(model, "GeometricTranslation");
    let r = vec3(model, "GeometricRotation");
    let s = vec3(model, "GeometricScaling");
    let s = if s == [0.0; 3] { [1.0; 3] } else { s };
    Mat4::translation(t[0], t[1], t[2])
        .mul(Mat4::euler(order, r))
        .mul(Mat4::scale(s[0], s[1], s[2]))
}

struct Link {
    kind: String,
    child: i64,
    parent: i64,
}

fn links(roots: &[Node]) -> Vec<Link> {
    roots.iter().find(|node| node.name == "Connections").map(|node| {
        node.children_named("C").filter_map(|link| {
            Some(Link {
                kind: link.string(0)?,
                child: link.i64(1)?,
                parent: link.i64(2)?,
            })
        }).collect()
    }).unwrap_or_default()
}

fn object_id(node: &Node) -> Option<i64> {
    node.i64(0)
}

fn object_name(node: &Node) -> String {
    node.string(1).unwrap_or_default().split(['\0', '\u{1}']).next().unwrap_or("").to_string()
}

fn by_id<'a>(nodes: &'a [&Node], id: i64) -> Option<&'a Node> {
    nodes.iter().copied().find(|node| object_id(node) == Some(id))
}

pub(crate) fn read(roots: &[Node]) -> Result<Scene, String> {
    let objects = roots.iter().find(|node| node.name == "Objects").ok_or("FBX file has no objects")?;
    let models: Vec<&Node> = objects.children_named("Model").collect();
    let geometries: Vec<&Node> = objects.children_named("Geometry").collect();
    let materials: Vec<&Node> = objects.children_named("Material").collect();
    let videos: Vec<&Node> = objects.children_named("Video").collect();
    let links = links(roots);
    let axis = axis(roots);
    let mut warnings = Vec::new();
    let mut scene_materials: Vec<Material> = Vec::new();
    let mut material_ids: Vec<i64> = Vec::new();
    let mut triangles = Vec::new();
    let meshes: Vec<&Node> = models.iter().copied().filter(|model| model.string(2).as_deref() == Some("Mesh")).collect();
    if meshes.len() > 1 {
        warnings.push(format!("Combining {} meshes into one map.", meshes.len()));
    }
    if meshes.is_empty() {
        return Err("FBX file has no mesh".into());
    }
    for model in meshes {
        let id = object_id(model).ok_or("FBX mesh has no id")?;
        let order = rotation_order(model);
        if !(0..=5).contains(&order) {
            warnings.push(format!(
                "{} uses FBX rotation order {order}; treating it as XYZ.",
                object_name(model)
            ));
        }
        let world = world_matrix(&models, &links, id).mul(geometric_matrix(model, order));
        let placed = axis.to_engine.mul(Mat4::scale(axis.metres_per_unit, axis.metres_per_unit, axis.metres_per_unit)).mul(world);
        let flip = placed.det3() < 0.0;
        let model_materials: Vec<i64> = links.iter().filter(|link| link.parent == id && materials.iter().any(|m| object_id(m) == Some(link.child))).map(|link| link.child).collect();
        let mut local_material = |source: i64| -> usize {
            if let Some(index) = material_ids.iter().position(|id| *id == source) {
                return index;
            }
            let node = by_id(&materials, source);
            let color = node.map(diffuse).unwrap_or([0.7, 0.7, 0.7]);
            let texture = node.and_then(|material| texture_of(material, &links, &videos));
            material_ids.push(source);
            scene_materials.push(Material {
                name: node.map(object_name).filter(|name| !name.is_empty()).unwrap_or_else(|| format!("Material {}", scene_materials.len() + 1)),
                color,
                texture,
            });
            scene_materials.len() - 1
        };
        if model_materials.is_empty() {
            local_material(0);
        }
        for geometry in links.iter().filter(|link| link.parent == id).filter_map(|link| by_id(&geometries, link.child)) {
            if geometry.child("LayerElementColor").is_some() && !warnings.iter().any(|warning| warning.contains("Vertex colours")) {
                warnings.push("Vertex colours are ignored.".into());
            }
            triangles.extend(geometry_triangles(geometry, placed, flip, &model_materials, &mut local_material)?);
        }
    }
    if triangles.is_empty() {
        return Err("FBX mesh has no triangles".into());
    }
    Ok(Scene { materials: scene_materials, triangles, warnings })
}

fn world_matrix(models: &[&Node], links: &[Link], id: i64) -> Mat4 {
    let mut chain = Vec::new();
    let mut current = id;
    for _ in 0..64 {
        let Some(model) = models.iter().copied().find(|model| object_id(model) == Some(current)) else { break };
        chain.push(local_matrix(model));
        match links.iter().find(|link| link.child == current && link.kind == "OO" && models.iter().any(|m| object_id(m) == Some(link.parent))) {
            Some(link) => current = link.parent,
            None => break,
        }
    }
    chain.into_iter().rev().fold(Mat4::identity(), Mat4::mul)
}

fn diffuse(material: &Node) -> [f32; 3] {
    let color = properties(material).find(|(name, _)| name.contains("Diffuse") || name == "DiffuseColor").map(|(_, props)| {
        let values: Vec<f64> = props.iter().filter_map(|prop| match prop {
            Prop::F64(value) => Some(*value),
            Prop::F32(value) => Some(f64::from(*value)),
            _ => None,
        }).collect();
        values
    }).unwrap_or_default();
    if color.len() >= 3 {
        [color[0] as f32, color[1] as f32, color[2] as f32]
    } else {
        [0.7, 0.7, 0.7]
    }
}

fn texture_of(material: &Node, links: &[Link], videos: &[&Node]) -> Option<TexturePixels> {
    let material_id = object_id(material)?;
    let texture_id = links.iter().find(|link| link.parent == material_id)?.child;
    let video_id = links.iter().find(|link| link.parent == texture_id).map(|link| link.child).unwrap_or(texture_id);
    let video = videos.iter().copied().find(|video| object_id(video) == Some(video_id))?;
    let bytes = video.child("Content").and_then(Node::bytes)?;
    let start = bytes.windows(8).position(|window| window == b"\x89PNG\r\n\x1a\n" || window.starts_with(&[0xff, 0xd8, 0xff]))?;
    let image = image::load_from_memory(&bytes[start..]).ok()?;
    let rgba = image.to_rgba8();
    Some(TexturePixels {
        name: object_name(video),
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

fn geometry_triangles(
    geometry: &Node,
    transform: Mat4,
    flip: bool,
    model_materials: &[i64],
    local_material: &mut impl FnMut(i64) -> usize,
) -> Result<Vec<Triangle>, String> {
    let vertex_node = geometry.child("Vertices").ok_or("FBX mesh has no vertices")?;
    let mut vertex_backing = None;
    let vertices = floats_of(vertex_node, &mut vertex_backing).ok_or("FBX mesh vertices are not a float array")?;
    if vertices.len() % 3 != 0 {
        return Err("FBX vertex list is not a list of positions".into());
    }
    let indices = geometry.child("PolygonVertexIndex").and_then(Node::i32s).ok_or("FBX mesh has no polygons")?;
    let uv = layer(geometry, "LayerElementUV", &["UV", "UVIndex"]);
    let normals = layer(geometry, "LayerElementNormal", &["Normals", "NormalsIndex"]);
    let material_layer = layer(geometry, "LayerElementMaterial", &["Materials", "Materials"]);
    let mut triangles = Vec::new();
    let mut polygon = Vec::new();
    let mut polygon_index = 0usize;
    for (corner, &encoded) in indices.iter().enumerate() {
        let end = encoded < 0;
        let index = if end { !encoded } else { encoded } as usize;
        polygon.push((index, corner));
        if !end { continue; }
        if polygon.len() >= 3 {
            let first_vertex = polygon[0].0;
            let first_corner = polygon[0].1;
            let material_slot = material_index(&material_layer, polygon_index, first_corner, first_vertex)
                .min(model_materials.len().saturating_sub(1));
            let material = match model_materials.get(material_slot).copied() {
                Some(source) => local_material(source),
                None => local_material(0),
            };
            for fan in 1..polygon.len() - 1 {
                let mut corners = [0, fan, fan + 1];
                if flip { corners.swap(1, 2); }
                let sample = |which: usize| -> Result<([f32; 3], [f32; 3], [f32; 2]), String> {
                    let (vertex, corner) = polygon[which];
                    if vertex >= vertices.len() / 3 {
                        return Err("FBX polygon index is outside the vertex list".into());
                    }
                    let position = transform.point([vertices[vertex * 3], vertices[vertex * 3 + 1], vertices[vertex * 3 + 2]]);
                    let normal = match element_f64(&normals, corner, vertex, polygon_index) {
                        Some(value) => transform.inverse_transpose_vector([value[0], value[1], value[2]]),
                        None => [0.0, 0.0, 0.0],
                    };
                    let uv = element_f64(&uv, corner, vertex, polygon_index)
                        .map(|value| [value[0] as f32, value[1] as f32])
                        .unwrap_or([0.0, 0.0]);
                    Ok(([position[0] as f32, position[1] as f32, position[2] as f32], [normal[0] as f32, normal[1] as f32, normal[2] as f32], uv))
                };
                let (p0, n0, u0) = sample(corners[0])?;
                let (p1, n1, u1) = sample(corners[1])?;
                let (p2, n2, u2) = sample(corners[2])?;
                let face = face_normal(p0, p1, p2);
                let normals = [n0, n1, n2].map(|normal| if normal == [0.0, 0.0, 0.0] { face } else { normal });
                triangles.push(Triangle { positions: [p0, p1, p2], normals, uvs: [u0, u1, u2], material });
            }
        }
        polygon.clear();
        polygon_index += 1;
    }
    Ok(triangles)
}

fn floats_of<'a>(node: &'a Node, backing: &'a mut Option<Vec<f64>>) -> Option<&'a [f64]> {
    if let Some(values) = node.f64s() {
        return Some(values);
    }
    let values = node.f32s()?;
    *backing = Some(values.iter().copied().map(f64::from).collect());
    backing.as_deref()
}

enum Floats<'a> {
    F64(&'a [f64]),
    F32(&'a [f32]),
}

impl Floats<'_> {
    fn get(&self, index: usize) -> Option<f64> {
        match self {
            Floats::F64(values) => values.get(index).copied(),
            Floats::F32(values) => values.get(index).copied().map(f64::from),
        }
    }
}

struct Layer<'a> {
    mapping: String,
    reference: String,
    values: Floats<'a>,
    indices: Option<&'a [i32]>,
    width: usize,
}

fn layer<'a>(geometry: &'a Node, name: &str, value_names: &[&str]) -> Option<Layer<'a>> {
    let element = geometry.children.iter().find(|node| node.name == name)?;
    let mapping = element.child("MappingInformationType").and_then(|node| node.string(0)).unwrap_or_else(|| "ByPolygonVertex".into());
    let reference = element.child("ReferenceInformationType").and_then(|node| node.string(0)).unwrap_or_else(|| "Direct".into());
    let direct = element.children.iter().find(|node| node.name == value_names[0])
        .or_else(|| element.children.iter().find(|node| node.name.ends_with("Index") && value_names[0] == value_names[1]))?;
    let values = if name.contains("Material") {
        Floats::F64(&[])
    } else if let Some(values) = direct.f64s() {
        Floats::F64(values)
    } else if let Some(values) = direct.f32s() {
        Floats::F32(values)
    } else {
        return None;
    };
    let indices = if value_names[0] != value_names[1] {
        element.child(value_names[1]).and_then(Node::i32s)
            .or_else(|| element.children.iter().find(|node| node.name.ends_with("Index")).and_then(Node::i32s))
    } else {
        direct.i32s()
    };
    let width = if name.contains("UV") { 2 } else if name.contains("Material") { 1 } else { 3 };
    Some(Layer { mapping, reference, values, indices, width })
}

fn mapped_slot(mapping: &str, corner: usize, vertex: usize, polygon: usize) -> usize {
    match mapping {
        "AllSame" => 0,
        "ByPolygon" => polygon,
        "ByVertice" | "ByVertex" => vertex,
        _ => corner,
    }
}

fn material_index(layer: &Option<Layer<'_>>, polygon: usize, corner: usize, vertex: usize) -> usize {
    let Some(layer) = layer else { return 0 };
    let Some(indices) = layer.indices else { return 0 };
    let slot = mapped_slot(&layer.mapping, corner, vertex, polygon);
    indices.get(slot).copied().unwrap_or(0).max(0) as usize
}

fn element_f64(layer: &Option<Layer<'_>>, corner: usize, vertex: usize, polygon: usize) -> Option<[f64; 3]> {
    let layer = layer.as_ref()?;
    let slot = mapped_slot(&layer.mapping, corner, vertex, polygon);
    let index = if layer.reference != "Direct" {
        layer.indices?.get(slot).copied().unwrap_or(0).max(0) as usize
    } else {
        slot
    };
    let mut out = [0.0; 3];
    for component in 0..layer.width.min(3) {
        out[component] = layer.values.get(index * layer.width + component)?;
    }
    Some(out)
}

fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = normalize([
        f64::from(u[1] * v[2] - u[2] * v[1]),
        f64::from(u[2] * v[0] - u[0] * v[2]),
        f64::from(u[0] * v[1] - u[1] * v[0]),
    ]);
    [n[0] as f32, n[1] as f32, n[2] as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blender_scale_and_x_rotation_land_in_metres() {
        // Blender writes centimetres with a scale of 100 and a -90 degree X rotation
        // when the object scale is 1. A vertex at (0, -10, 0) file units becomes
        // 10 metres along engine Z.
        let model = Node {
            name: "Model".into(),
            props: vec![Prop::I64(1), Prop::Bytes(b"Ground\0\x01Mesh".to_vec()), Prop::Bytes(b"Mesh".to_vec())],
            children: vec![Node {
                name: "Properties70".into(),
                props: Vec::new(),
                children: vec![
                    property("Lcl Rotation", &[-90.0, 0.0, 0.0]),
                    property("Lcl Scaling", &[100.0, 100.0, 100.0]),
                ],
            }],
        };
        let roots = [
            Node { name: "GlobalSettings".into(), props: Vec::new(), children: vec![Node {
                name: "Properties70".into(), props: Vec::new(),
                children: vec![
                    int_property("UpAxis", 1), int_property("UpAxisSign", 1),
                    int_property("FrontAxis", 2), int_property("FrontAxisSign", 1),
                    int_property("CoordAxis", 0), int_property("CoordAxisSign", 1),
                    property("UnitScaleFactor", &[1.0]),
                ],
            }]},
            Node { name: "Objects".into(), props: Vec::new(), children: vec![
                model,
                Node { name: "Geometry".into(), props: vec![Prop::I64(2)], children: vec![
                    Node { name: "Vertices".into(), props: vec![Prop::F64s(vec![0.0, -10.0, 0.0, 1.0, -10.0, 0.0, 0.0, -10.0, 1.0])], children: Vec::new() },
                    Node { name: "PolygonVertexIndex".into(), props: vec![Prop::I32s(vec![0, 1, !2])], children: Vec::new() },
                ]},
                Node { name: "Material".into(), props: vec![Prop::I64(3), Prop::Bytes(b"Paint".to_vec())], children: Vec::new() },
            ]},
            Node { name: "Connections".into(), props: Vec::new(), children: vec![
                connection(2, 1),
                connection(3, 1),
            ]},
        ];
        let encoded = crate::fbx::encode(&roots, 7400);
        let scene = read(&roots).unwrap();
        let triangle = &scene.triangles[0];
        let z: Vec<f32> = triangle.positions.iter().map(|p| p[2]).collect();
        assert!(z.iter().all(|z| (z - 10.0).abs() < 0.001), "{triangle:?}");
        assert_eq!(scene.materials[0].name, "Paint");
        let via_bytes = read(&crate::fbx::parse(&encoded).unwrap()).unwrap();
        assert!((via_bytes.triangles[0].positions[0][2] - 10.0).abs() < 0.001);
    }

    #[test]
    fn rotation_order_zyx_differs_from_xyz() {
        let point = |order: i32| {
            let scene = read(&metre_mesh(vec![
                int_property("RotationOrder", order),
                property("Lcl Rotation", &[90.0, 90.0, 0.0]),
            ], vec![0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0], vec![0, 1, !2], Vec::new())).unwrap();
            scene.triangles[0].positions[0]
        };
        let xyz = point(0);
        let zyx = point(5);
        assert!((xyz[0] - 1.0).abs() < 1e-4 && xyz[1].abs() < 1e-4 && xyz[2].abs() < 1e-4, "{xyz:?}");
        assert!(zyx[0].abs() < 1e-4 && (zyx[1] + 1.0).abs() < 1e-4 && zyx[2].abs() < 1e-4, "{zyx:?}");
    }

    #[test]
    fn negative_scale_flips_winding_and_parent_translation_is_inherited() {
        let scene = read(&parented_mesh(vec![property("Lcl Scaling", &[-1.0, 1.0, 1.0])], [5.0, 0.0, 0.0])).unwrap();
        let [a, b, c] = scene.triangles[0].positions;
        let normal_z = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        assert!(normal_z > 0.5, "{a:?} {b:?} {c:?}");
        assert!(scene.triangles[0].positions.iter().all(|p| p[0] > 3.0), "{:?}", scene.triangles[0].positions);
    }

    #[test]
    fn index_to_direct_uvs_and_missing_normals_are_filled() {
        let uv = layer_element("LayerElementUV", "ByPolygonVertex", "IndexToDirect", "UV", "UVIndex",
            Prop::F64s(vec![0.0, 0.0, 1.0, 0.5]), Prop::I32s(vec![1, 0, 1]));
        let materials = layer_element("LayerElementMaterial", "AllSame", "IndexToDirect", "Materials", "Materials",
            Prop::I32s(vec![0]), Prop::I32s(vec![]));
        let scene = read(&metre_mesh(Vec::new(), vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 0.0], vec![0, 1, !2], vec![uv, materials])).unwrap();
        let triangle = &scene.triangles[0];
        assert!((triangle.uvs[0][0] - 1.0).abs() < 1e-5 && (triangle.uvs[1][0]).abs() < 1e-5, "{:?}", triangle.uvs);
        assert!(triangle.normals.iter().all(|normal| (normal[2] - 1.0).abs() < 1e-4), "{:?}", triangle.normals);
    }

    #[test]
    fn z_up_files_become_y_up_metres() {
        let mut roots = metre_mesh(Vec::new(), vec![0.0, 0.0, 5.0, 1.0, 0.0, 5.0, 0.0, 1.0, 5.0], vec![0, 1, !2], Vec::new());
        let settings = &mut roots[0].children[0].children;
        let up = settings.iter().position(|node| node.string(0).as_deref() == Some("UpAxis")).unwrap();
        let front = settings.iter().position(|node| node.string(0).as_deref() == Some("FrontAxis")).unwrap();
        settings[up] = int_property("UpAxis", 2);
        settings[front] = int_property("FrontAxis", 1);
        let scene = read(&roots).unwrap();
        let point = scene.triangles[0].positions[0];
        assert!(point[0].abs() < 1e-4 && (point[1] - 5.0).abs() < 1e-4 && point[2].abs() < 1e-4, "{point:?}");
    }

    fn metre_mesh(extra: Vec<Node>, vertices: Vec<f64>, polygons: Vec<i32>, geometry_children: Vec<Node>) -> [Node; 3] {
        let mut properties = vec![property("UnitScaleFactor", &[100.0])];
        properties.extend([
            int_property("UpAxis", 1), int_property("UpAxisSign", 1),
            int_property("FrontAxis", 2), int_property("FrontAxisSign", 1),
            int_property("CoordAxis", 0), int_property("CoordAxisSign", 1),
        ]);
        let mut model_props = extra;
        model_props.push(property("Lcl Scaling", &[1.0, 1.0, 1.0]));
        let mut geometry = vec![
            Node { name: "Vertices".into(), props: vec![Prop::F64s(vertices)], children: Vec::new() },
            Node { name: "PolygonVertexIndex".into(), props: vec![Prop::I32s(polygons)], children: Vec::new() },
        ];
        geometry.extend(geometry_children);
        [
            Node { name: "GlobalSettings".into(), props: Vec::new(), children: vec![Node {
                name: "Properties70".into(), props: Vec::new(), children: properties,
            }]},
            Node { name: "Objects".into(), props: Vec::new(), children: vec![
                Node {
                    name: "Model".into(),
                    props: vec![Prop::I64(1), Prop::Bytes(b"Ground".to_vec()), Prop::Bytes(b"Mesh".to_vec())],
                    children: vec![Node { name: "Properties70".into(), props: Vec::new(), children: model_props }],
                },
                Node { name: "Geometry".into(), props: vec![Prop::I64(2)], children: geometry },
                Node { name: "Material".into(), props: vec![Prop::I64(3), Prop::Bytes(b"Paint".to_vec())], children: Vec::new() },
            ]},
            Node { name: "Connections".into(), props: Vec::new(), children: vec![connection(2, 1), connection(3, 1)] },
        ]
    }

    fn parented_mesh(child_props: Vec<Node>, parent_translation: [f64; 3]) -> [Node; 3] {
        let mut roots = metre_mesh(child_props, vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], vec![0, 1, !2], Vec::new());
        let objects = &mut roots[1];
        objects.children.insert(0, Node {
            name: "Model".into(),
            props: vec![Prop::I64(9), Prop::Bytes(b"Parent".to_vec()), Prop::Bytes(b"Null".to_vec())],
            children: vec![Node {
                name: "Properties70".into(), props: Vec::new(),
                children: vec![property("Lcl Translation", &parent_translation)],
            }],
        });
        roots[2].children.push(connection(1, 9));
        roots
    }

    fn layer_element(name: &str, mapping: &str, reference: &str, direct: &str, index: &str, values: Prop, indices: Prop) -> Node {
        let mut children = vec![
            Node { name: "MappingInformationType".into(), props: vec![Prop::Bytes(mapping.as_bytes().to_vec())], children: Vec::new() },
            Node { name: "ReferenceInformationType".into(), props: vec![Prop::Bytes(reference.as_bytes().to_vec())], children: Vec::new() },
            Node { name: direct.into(), props: vec![values], children: Vec::new() },
        ];
        if direct != index {
            children.push(Node { name: index.into(), props: vec![indices], children: Vec::new() });
        }
        Node { name: name.into(), props: Vec::new(), children }
    }

    fn property(name: &str, values: &[f64]) -> Node {
        let mut props = vec![Prop::Bytes(name.as_bytes().to_vec())];
        props.extend(values.iter().copied().map(Prop::F64));
        Node { name: "P".into(), props, children: Vec::new() }
    }
    fn int_property(name: &str, value: i32) -> Node {
        Node { name: "P".into(), props: vec![Prop::Bytes(name.as_bytes().to_vec()), Prop::I32(value)], children: Vec::new() }
    }
    fn connection(child: i64, parent: i64) -> Node {
        Node { name: "C".into(), props: vec![Prop::Bytes(b"OO".to_vec()), Prop::I64(child), Prop::I64(parent)], children: Vec::new() }
    }
}
