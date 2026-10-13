//! SKATE09 writer. Field order matches the reader in the parent module.
use super::{Collision, Material, Rail, SkateMap, Texture, Vertex};
use std::io::Write;

impl SkateMap {
    /// Encode a portable map. Version-9 packages have no doors, lights, routes,
    /// extensions, retail material definitions or v11+ collision edge bytes.
    pub fn to_v9_bytes(&self) -> Result<Vec<u8>, String> {
        if self.environment.len() != 45 {
            return Err(format!(
                "SKATE09 needs 45 environment floats, found {}",
                self.environment.len()
            ));
        }
        if self.materials.is_empty() || self.geometry.vertices.is_empty() || self.geometry.indices.is_empty() {
            return Err("SKATE09 needs materials and render geometry".into());
        }
        if self.geometry.indices.len() % 3 != 0 {
            return Err("SKATE09 indices are not triangles".into());
        }
        if self.geometry.collision.is_empty() {
            return Err("SKATE09 needs collision triangles".into());
        }
        if self.geometry.collision.iter().any(|triangle| triangle.surface == 0) {
            return Err("SKATE09 collision surface must be non-zero".into());
        }
        if !self.doors.is_empty() || !self.lights.is_empty() || !self.routes.is_empty() || !self.extensions.is_empty() {
            return Err("SKATE09 writer does not emit doors, lights, routes or extensions".into());
        }
        if self.materials.iter().any(|m| m.depth_layer.is_some() || m.retail_definition.is_some()) {
            return Err("SKATE09 writer does not emit retail material definitions".into());
        }
        if self.geometry.vertices.iter().any(|v| v.decal_uv.is_some() || v.tangent_frame.is_some()) {
            return Err("SKATE09 vertices cannot carry decal UVs or tangent frames".into());
        }
        if self.geometry.collision.iter().any(|c| c.native_edges.is_some()) || self.rails.iter().any(|r| r.native.is_some()) {
            return Err("SKATE09 writer does not emit native collision edges or rail curves".into());
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"SKATE09\0");
        u32s(&mut out, &[0x1234_5678]);
        string(&mut out, &self.name)?;
        floats(&mut out, &self.spawn);
        floats(&mut out, &[self.heading]);
        floats(&mut out, &self.environment);
        u32s(&mut out, &[
            self.materials.len() as u32,
            self.textures.len() as u32,
            self.geometry.vertices.len() as u32,
            self.geometry.indices.len() as u32,
            self.geometry.collision.len() as u32,
            self.rails.len() as u32,
            0,
            0,
            0,
        ]);
        for material in &self.materials {
            write_material(&mut out, material)?;
        }
        for texture in &self.textures {
            write_texture(&mut out, texture)?;
        }
        stored(&mut out, &vertex_bytes(&self.geometry.vertices));
        stored(&mut out, &index_bytes(&self.geometry.indices));
        stored(&mut out, &collision_bytes(&self.geometry.collision));
        for rail in &self.rails {
            write_rail(&mut out, rail)?;
        }
        Ok(out)
    }

    /// Portable checks the game applies before it will simulate a map.
    ///
    /// This matches the failure cases in the game's `validate_runtime` for a
    /// package this writer can emit: collision, embedded textures, and no
    /// hinged doors. Retail extension adapters stay in the game crate.
    pub fn validate_runtime(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("Empty SKATE map name".into());
        }
        if self.materials.is_empty() || self.geometry.vertices.is_empty() || self.geometry.indices.is_empty() {
            return Err("SKATE requires materials and render geometry".into());
        }
        if self.geometry.collision.is_empty()
            && !self.extensions.iter().any(|extension| {
                extension.tag == *b"RWCM" && extension.schema == 1 && !extension.payload.is_empty()
            })
        {
            return Err("SKATE map has no collision geometry".into());
        }
        if !self.doors.is_empty() {
            return Err(format!(
                "Map '{}' contains {} hinged doors. This imported game has no door body/controller adapter yet; refusing to drop their geometry or turn them into static walls.",
                self.name,
                self.doors.len()
            ));
        }
        if self.textures.iter().any(|texture| texture.width == 0) {
            return Err(
                "SKATE contains external texture placeholders; supply a package with embedded textures".into(),
            );
        }
        Ok(())
    }
}

fn write_material(out: &mut Vec<u8>, material: &Material) -> Result<(), String> {
    string(out, &material.name)?;
    u32s(out, &[material.flags]);
    floats(out, &[
        material.friction,
        material.restitution,
        material.color[0],
        material.color[1],
        material.color[2],
        material.roughness,
        material.emissive,
    ]);
    u32s(out, &[material.textures[0], material.textures[1]]);
    floats(out, &[material.indirect_strength]);
    u32s(out, &[material.textures[2], material.textures[3], material.textures[4], material.alpha_mode]);
    floats(out, &[material.alpha_cutoff]);
    u32s(out, &[material.audio, material.physics, material.pattern]);
    Ok(())
}

fn write_texture(out: &mut Vec<u8>, texture: &Texture) -> Result<(), String> {
    let expected = texture.width as usize * texture.height as usize * 4;
    if texture.rgba.len() != expected {
        return Err(format!(
            "Texture {} is {} bytes, expected {expected}",
            texture.name,
            texture.rgba.len()
        ));
    }
    string(out, &texture.name)?;
    u32s(out, &[texture.width, texture.height, texture.color_space]);
    stored(out, &texture.rgba);
    Ok(())
}

fn write_rail(out: &mut Vec<u8>, rail: &Rail) -> Result<(), String> {
    string(out, &rail.name)?;
    u32s(out, &[u32::from(rail.closed), rail.points.len() as u32]);
    for point in &rail.points {
        floats(out, point);
    }
    Ok(())
}

fn vertex_bytes(vertices: &[Vertex]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vertices.len() * 44);
    for vertex in vertices {
        floats(&mut bytes, &vertex.position);
        floats(&mut bytes, &vertex.normal);
        floats(&mut bytes, &vertex.uv);
        floats(&mut bytes, &vertex.lightmap_uv);
        u32s(&mut bytes, &[vertex.material]);
    }
    bytes
}

fn index_bytes(indices: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(indices.len() * 4);
    u32s(&mut bytes, indices);
    bytes
}

fn collision_bytes(collision: &[Collision]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(collision.len() * 44);
    for triangle in collision {
        for point in &triangle.points {
            floats(&mut bytes, point);
        }
        u32s(&mut bytes, &[triangle.surface, triangle.material]);
    }
    bytes
}

/// Method 1 is zlib, which the reader checks down to the last input byte.
fn stored(out: &mut Vec<u8>, raw: &[u8]) {
    let compressed = zlib(raw);
    u32s(out, &[1, compressed.len() as u32]);
    out.extend_from_slice(&compressed);
}

fn zlib(raw: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(raw).expect("zlib encoder accepts bytes");
    encoder.finish().expect("zlib encoder finishes")
}

fn string(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("SKATE name must not be empty".into());
    }
    u32s(out, &[value.len() as u32]);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn u32s(out: &mut Vec<u8>, values: &[u32]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn floats(out: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    fn demo() -> SkateMap {
        let rgba = vec![220, 220, 220, 255, 50, 90, 150, 255, 50, 90, 150, 255, 220, 220, 220, 255];
        SkateMap {
            version: 9,
            name: "Round trip".into(),
            spawn: [1.0, 2.0, 3.0],
            heading: 0.5,
            environment: (0..45).map(|i| i as f32 * 0.01).collect(),
            materials: vec![Material {
                name: "Floor".into(),
                flags: 1,
                friction: 0.6,
                restitution: 0.1,
                color: [1.0, 1.0, 1.0],
                roughness: 0.85,
                emissive: 0.0,
                textures: [1, 2, 0, 0, 0],
                indirect_strength: 1.0,
                alpha_mode: 0,
                alpha_cutoff: 0.5,
                audio: 3,
                physics: 1,
                pattern: 0,
                depth_layer: None,
                retail_definition: None,
            }],
            textures: vec![
                Texture { name: "Albedo".into(), width: 2, height: 2, color_space: 1, rgba: rgba.clone() },
                Texture { name: "Indirect".into(), width: 1, height: 1, color_space: 0, rgba: vec![64, 64, 64, 255] },
            ],
            geometry: Geometry {
                vertices: (0..4).map(|i| Vertex {
                    position: [i as f32, 0.0, 1.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, i as f32],
                    lightmap_uv: [0.5, 0.5],
                    material: 1,
                    decal_uv: None,
                    tangent_frame: None,
                }).collect(),
                indices: vec![0, 1, 2, 0, 2, 3],
                collision: vec![Collision {
                    points: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                    surface: 1,
                    material: 1,
                    native_edges: None,
                }],
            },
            rails: vec![Rail {
                name: "Ledge".into(),
                closed: false,
                points: vec![[0.0, 1.0, 0.0], [4.0, 1.0, 0.0]],
                native: None,
            }],
            doors: Vec::new(),
            lights: Vec::new(),
            routes: Vec::new(),
            extensions: Vec::new(),
        }
    }

    #[test]
    fn v9_bytes_round_trip() {
        let map = demo();
        let bytes = map.to_v9_bytes().unwrap();
        let parsed = SkateMap::parse(&bytes).unwrap();
        parsed.validate_runtime().unwrap();
        assert_eq!(parsed.version, 9);
        assert_eq!(parsed.name, map.name);
        assert_eq!(parsed.spawn, map.spawn);
        assert_eq!(parsed.heading, map.heading);
        assert_eq!(parsed.environment, map.environment);
        assert_eq!(parsed.materials, map.materials);
        assert_eq!(parsed.textures, map.textures);
        assert_eq!(parsed.geometry, map.geometry);
        assert_eq!(parsed.rails, map.rails);
    }
}
