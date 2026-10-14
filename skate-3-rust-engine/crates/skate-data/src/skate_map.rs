//! Little-endian SKATE01..15 reader. Layout follows the supplied
//! SK8R15/Source/tools/blender_owned_map/SKATE_FORMAT.md and
//! owned/world/src/owned_map_package.cpp. No geometry or physics is inferred.
use std::{io::Read, path::Path, time::Instant};
mod texture_decode;
mod storage_v15;
mod writer;

#[derive(Debug, PartialEq)]
pub struct SkateMap {
    pub version: u8,
    pub name: String,
    pub spawn: [f32; 3],
    pub heading: f32,
    /// Authored environment fields in wire order (12, 14, or 45 floats).
    pub environment: Vec<f32>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub geometry: Geometry,
    pub rails: Vec<Rail>,
    pub doors: Vec<Door>,
    pub lights: Vec<Light>,
    pub routes: Vec<Route>,
    pub extensions: Vec<Extension>,
}
#[derive(Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub flags: u32,
    pub friction: f32,
    pub restitution: f32,
    pub color: [f32; 3],
    pub roughness: f32,
    pub emissive: f32,
    /// Albedo, indirect, normal, ORM, emissive. One-based, zero absent.
    pub textures: [u32; 5],
    pub indirect_strength: f32,
    pub alpha_mode: u32,
    pub alpha_cutoff: f32,
    pub audio: u32,
    pub physics: u32,
    pub pattern: u32,
    pub depth_layer: Option<u32>,
    /// Complete v12+ retail definition bytes; distinct from portable PBR fields.
    pub retail_definition: Option<Vec<u8>>,
}
#[derive(Debug, PartialEq)]
pub struct Texture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub color_space: u32,
    pub rgba: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub lightmap_uv: [f32; 2],
    pub material: u32,
    pub decal_uv: Option<[f32; 2]>,
    pub tangent_frame: Option<[u8; 4]>,
}
#[derive(Debug, PartialEq)]
pub struct Collision {
    pub points: [[f32; 3]; 3],
    pub surface: u32,
    pub material: u32,
    pub native_edges: Option<[u8; 3]>,
}
#[derive(Debug, PartialEq)]
pub struct Geometry {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub collision: Vec<Collision>,
}
#[derive(Debug, PartialEq)]
pub struct Rail {
    pub name: String,
    pub closed: bool,
    pub points: Vec<[f32; 3]>,
    pub native: Option<Vec<u8>>,
}
#[derive(Debug, PartialEq)]
pub struct Door {
    pub name: String,
    /// Hinge position/axis, closed width/depth axes, local min/max.
    pub frame: [[f32; 3]; 6],
    /// Minimum/maximum/initial angle, mass, damping.
    pub motion: [f32; 5],
    /// Spring, max angular speed, contact impulse scale (absent in v4).
    pub response: Option<[f32; 3]>,
    pub friction: f32,
    pub restitution: f32,
    pub surface: u32,
    pub geometry: Geometry,
}
#[derive(Debug, PartialEq)]
pub struct Light {
    pub name: String,
    pub kind: u32,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
    pub radius: f32,
    pub inner_cos: f32,
    pub outer_cos: f32,
}
#[derive(Debug, PartialEq)]
pub struct Route {
    pub rail: Rail,
    pub skaters: u32,
    pub speed: f32,
    pub spacing: f32,
}
#[derive(Debug, PartialEq)]
pub struct Extension {
    pub tag: [u8; 4],
    pub schema: u32,
    pub payload: Vec<u8>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

/// MOBJ schema 3 stores editor ownership of ranges in the base geometry.
/// Physics-disabled objects can use those unchanged static arrays. Explicit
/// body types still require a runtime adapter and must not be flattened.
pub fn validate_static_objects(map: &SkateMap, extension: &Extension) -> Result<(), String> {
    if extension.schema != 3 { return Err("Unsupported MOBJ schema".into()); }
    let mut r = Reader { bytes: &extension.payload, at: 0 };
    let count = r.u()?;
    r.check_count(count, 80)?;
    let mut ids = std::collections::HashSet::new();
    for _ in 0..count {
        if !ids.insert(r.u()?) { return Err("Duplicate MOBJ identity".into()); }
        let name = r.string()?;
        r.floats::<3>()?;
        for limit in [map.geometry.indices.len(), map.geometry.collision.len()] {
            let first = r.u()? as usize;
            let length = r.u()? as usize;
            if first.checked_add(length).is_none_or(|end| end > limit) {
                return Err(format!("MOBJ {name} geometry range is invalid"));
            }
        }
        let rails = r.u()?;
        r.check_count(rails, 4)?;
        for _ in 0..rails {
            if r.u()? as usize >= map.rails.len() { return Err(format!("MOBJ {name} rail index is invalid")); }
        }
        if r.u()? != 0 {
            return Err(format!("MOBJ {name} requests object physics, which requires a body adapter"));
        }
        if r.u()? > 2 { return Err(format!("MOBJ {name} collision shape is invalid")); }
        r.floats::<6>()?;
        for _ in 0..2 {
            if r.u()? > 1 { return Err(format!("MOBJ {name} has an invalid boolean")); }
        }
    }
    if r.at != r.bytes.len() { return Err("MOBJ has trailing data".into()); }
    Ok(())
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).ok_or("SKATE offset overflow")?;
        let result = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| format!("Truncated SKATE at byte {} (need {n})", self.at))?;
        self.at = end;
        Ok(result)
    }
    fn u(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f(&mut self) -> Result<f32, String> {
        let value = f32::from_bits(self.u()?);
        if !value.is_finite() {
            return Err(format!("Non-finite SKATE float at {}", self.at - 4));
        }
        Ok(value)
    }
    fn floats<const N: usize>(&mut self) -> Result<[f32; N], String> {
        let mut result = [0.; N];
        for v in &mut result {
            *v = self.f()?;
        }
        Ok(result)
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.u()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "Invalid SKATE UTF-8".into())
    }
    fn check_count(&self, count: u32, minimum_bytes: usize) -> Result<(), String> {
        if count as usize > (self.bytes.len() - self.at) / minimum_bytes {
            return Err("SKATE count exceeds remaining data".into());
        }
        Ok(())
    }
    fn points(&mut self) -> Result<Vec<[f32; 3]>, String> {
        let n = self.u()?;
        self.check_count(n, 12)?;
        (0..n).map(|_| self.floats()).collect()
    }
    fn stored_block(&mut self, expected: usize) -> Result<StoredBlock<'a>, String> {
        if expected > 2_147_483_648 {
            return Err("SKATE decoded block exceeds 2 GiB reader limit".into());
        }
        let method = self.u()?;
        let n = self.u()? as usize;
        let bytes = self.take(n)?;
        Ok(StoredBlock { expected, method, bytes })
    }
    fn stored(&mut self, expected: usize) -> Result<Vec<u8>, String> {
        self.stored_block(expected)?.decode()
    }
    fn geometry(
        &mut self,
        counts: [u32; 3],
        materials: usize,
        version: u8,
    ) -> Result<Geometry, String> {
        let [nv, ni, nc] = counts;
        self.check_count(nv, if version >= 12 { 56 } else { 44 })?;
        let vertices: Vec<Vertex> = (0..nv)
            .map(|_| {
                Ok(Vertex {
                    position: self.floats()?,
                    normal: self.floats()?,
                    uv: self.floats()?,
                    lightmap_uv: self.floats()?,
                    material: self.u()?,
                    decal_uv: if version >= 12 {
                        Some(self.floats()?)
                    } else {
                        None
                    },
                    tangent_frame: if version >= 12 {
                        Some(self.take(4)?.try_into().unwrap())
                    } else {
                        None
                    },
                })
            })
            .collect::<Result<_, String>>()?;
        self.check_count(ni, 4)?;
        let indices: Vec<u32> = (0..ni).map(|_| self.u()).collect::<Result<_, _>>()?;
        self.check_count(nc, if version >= 11 { 48 } else { 44 })?;
        let collision: Vec<Collision> = (0..nc)
            .map(|_| {
                let points = [self.floats()?, self.floats()?, self.floats()?];
                let surface = self.u()?;
                let material = self.u()?;
                let native_edges = if version >= 11 {
                    let bytes = self.take(4)?;
                    if bytes[3] != 0 {
                        Some(bytes[..3].try_into().unwrap())
                    } else {
                        None
                    }
                } else {
                    None
                };
                Ok(Collision {
                    points,
                    surface,
                    material,
                    native_edges,
                })
            })
            .collect::<Result<_, String>>()?;
        let valid_material = |id: u32| id > 0 && id as usize <= materials;
        if vertices.iter().any(|v| !valid_material(v.material))
            || collision
                .iter()
                .any(|v| !valid_material(v.material) || v.surface == 0)
        {
            return Err("SKATE invalid material/surface reference".into());
        }
        if indices.len() % 3 != 0 || indices.iter().any(|&i| i as usize >= vertices.len()) {
            return Err("SKATE invalid triangle indices".into());
        }
        for tri in indices.chunks_exact(3) {
            if tri
                .iter()
                .any(|&i| vertices[i as usize].material != vertices[tri[0] as usize].material)
            {
                return Err("SKATE triangle mixes materials".into());
            }
        }
        for tri in &collision {
            let [a, b, c] = tri.points;
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cross = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let area = cross.iter().map(|x| x * x).sum::<f32>();
            if !area.is_finite() || area <= 0. {
                return Err("SKATE degenerate collision triangle".into());
            }
        }
        Ok(Geometry {
            vertices,
            indices,
            collision,
        })
    }
}
/// Borrow compressed payloads while scanning; decompression does not mutate
/// the package or depend on any other texture.
#[derive(Clone, Copy)]
struct StoredBlock<'a> { expected: usize, method: u32, bytes: &'a [u8] }
impl StoredBlock<'_> {
    fn decode(&self) -> Result<Vec<u8>, String> {
        let Self { expected, method, bytes } = *self;
        let decoded = match method {
            0 => bytes.to_vec(),
            1 => {
                let mut result = Vec::with_capacity(expected.min(8 * 1024 * 1024));
                let mut decoder = flate2::read::ZlibDecoder::new(bytes);
                decoder
                    .by_ref()
                    .take(expected as u64 + 1)
                    .read_to_end(&mut result)
                    .map_err(|e| format!("SKATE DEFLATE: {e}"))?;
                if decoder.total_in() as usize != bytes.len() {
                    return Err("SKATE compressed block has unused input".into());
                }
                result
            }
            2 => {
                let mut result = Vec::with_capacity(expected.min(8 * 1024 * 1024));
                zstd::stream::read::Decoder::new(bytes)
                    .map_err(|e| e.to_string())?
                    .take(expected as u64 + 1)
                    .read_to_end(&mut result)
                    .map_err(|e| format!("SKATE Zstandard: {e}"))?;
                result
            }
            3..=10 => return storage_v15::decode(method, bytes, expected),
            _ => return Err(format!("Unsupported SKATE storage method {method}")),
        };
        if decoded.len() != expected {
            return Err("SKATE decoded block size mismatch".into());
        }
        Ok(decoded)
    }
}
impl SkateMap {
    pub fn load(path: &Path) -> Result<Self, String> {
        // File length, then the header counts. Both are known before textures
        // and geometry are decoded, which is the allocation that exhausts the GPU.
        let file_bytes = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
        if let Some(reason) = crate::load_budget::reject_package_file(file_bytes) {
            return Err(format!("{}: {reason}", path.display()));
        }
        let (vertices, triangles, collision) = geometry_counts(path)?;
        if let Some(reason) = crate::load_budget::reject_package_shape(vertices, triangles, collision) {
            return Err(format!("{}: {reason}", path.display()));
        }
        let started = Instant::now();
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let disk = started.elapsed();
        let parse_started = Instant::now();
        let map = Self::parse(&data).map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("MAP_READ_TIMING name={:?} disk_ms={} parse_ms={} file_bytes={}",
            map.name, disk.as_millis(), parse_started.elapsed().as_millis(), data.len());
        Ok(map)
    }
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
        Self::parse_with_decode_workers(data, workers)
    }
    /// Data tools can request one worker for reproducible serial comparisons.
    /// The decoder caps concurrency at eight and skips threading for small maps.
    pub fn parse_with_decode_workers(data: &[u8], workers: usize) -> Result<Self, String> {
        Self::parse_inner(data, workers, true)
    }

    /// Decode a supplemental presentation package without requiring collision.
    /// Playable map loading continues to use `parse`/`load`.
    pub fn parse_render_only(data: &[u8]) -> Result<Self, String> {
        let map = Self::parse_inner(data, 4, false)?;
        if !map.geometry.collision.is_empty() || !map.rails.is_empty()
            || !map.doors.is_empty() || !map.lights.is_empty() || !map.routes.is_empty()
            || map.extensions.iter().any(|e| e.tag != *b"WMET")
        {
            return Err("SKATE render-only package contains non-presentation data".into());
        }
        Ok(map)
    }
    fn parse_inner(data: &[u8], workers: usize, require_collision: bool) -> Result<Self, String> {
        let parse_started = Instant::now();
        let mut r = Reader { bytes: data, at: 0 };
        let magic = r.take(8)?;
        if &magic[..5] != b"SKATE"
            || magic[7] != 0
            || !magic[5].is_ascii_digit()
            || !magic[6].is_ascii_digit()
        {
            return Err("Invalid SKATE magic/version".into());
        }
        let version = (magic[5] - b'0') * 10 + magic[6] - b'0';
        if !(1..=15).contains(&version) {
            return Err("Unsupported SKATE version: reader supports 01 through 15; newer packages require an update".into());
        }
        if r.u()? != 0x12345678 {
            return Err("Invalid SKATE endian marker".into());
        }
        let name = r.string()?;
        if name.is_empty() {
            return Err("Empty SKATE map name".into());
        }
        let spawn = r.floats()?;
        let heading = r.f()?;
        let ne = if version >= 6 {
            45
        } else if version >= 3 {
            14
        } else {
            12
        };
        let environment = (0..ne).map(|_| r.f()).collect::<Result<_, _>>()?;
        let mut counts = [0; 9];
        let n = if version >= 8 {
            9
        } else if version >= 7 {
            8
        } else if version >= 4 {
            7
        } else {
            6
        };
        for c in &mut counts[..n] {
            *c = r.u()?;
        }
        let material_bytes = if version >= 15 {
            let size = r.u()? as usize;
            Some(r.stored(size)?)
        } else { None };
        let mut package_reader = None;
        if let Some(bytes) = &material_bytes {
            package_reader = Some(r);
            r = Reader { bytes, at: 0 };
        }
        r.check_count(counts[0], if version >= 2 { 80 } else { 48 })?;
        let mut materials = Vec::new();
        for _ in 0..counts[0] {
            let mut m = Material {
                name: r.string()?,
                flags: r.u()?,
                friction: r.f()?,
                restitution: r.f()?,
                color: r.floats()?,
                roughness: r.f()?,
                emissive: r.f()?,
                textures: [r.u()?, r.u()?, 0, 0, 0],
                indirect_strength: r.f()?,
                alpha_mode: 0,
                alpha_cutoff: 0.5,
                audio: 3,
                physics: 1,
                pattern: 0,
                depth_layer: None,
                retail_definition: None,
            };
            if version >= 2 {
                for id in &mut m.textures[2..] {
                    *id = r.u()?;
                }
                m.alpha_mode = r.u()?;
                m.alpha_cutoff = r.f()?;
                m.audio = r.u()?;
                m.physics = r.u()?;
                m.pattern = r.u()?;
            }
            if version >= 13 {
                let layer = r.u()?;
                if layer > 3 {
                    return Err("Invalid SKATE depth layer".into());
                }
                m.depth_layer = Some(layer);
            }
            if version >= 12 && r.u()? != 0 {
                let start = r.at;
                r.take(16)?;
                r.string()?;
                r.take(8)?;
                let bindings = r.u()?;
                r.check_count(bindings, 20)?;
                for _ in 0..bindings {
                    r.string()?;
                    let id = r.u()?;
                    let uv = r.u()?;
                    let au = r.u()?;
                    let av = r.u()?;
                    if id > counts[1] || uv > 2 || au > 1 || av > 1 {
                        return Err("Invalid SKATE retail texture binding".into());
                    }
                }
                let parameters = r.u()?;
                r.check_count(parameters, 8)?;
                for _ in 0..parameters {
                    r.string()?;
                    let n = r.u()?;
                    r.check_count(n, 4)?;
                    for _ in 0..n {
                        r.string()?;
                    }
                }
                r.string()?;
                m.retail_definition = Some(r.bytes[start..r.at].to_vec());
            }
            if m.name.is_empty()
                || m.textures.iter().any(|&id| id > counts[1])
                || m.alpha_mode > 2
                || m.audio > 127
                || m.physics > 13
                || m.pattern > 15
                || m.friction < 0.
                || m.restitution < 0.
                || !(0.0..=1.0).contains(&m.roughness)
                || !(0.0..=1.0).contains(&m.alpha_cutoff)
                || m.indirect_strength < 0.
            {
                return Err("Invalid SKATE material".into());
            }
            materials.push(m);
        }
        if let Some(package) = package_reader {
            if r.at != r.bytes.len() { return Err("SKATE material block has trailing bytes".into()); }
            r = package;
        }
        let texture_started = Instant::now();
        r.check_count(counts[1], 20)?;
        let mut textures = Vec::new();
        let mut texture_blocks: Vec<StoredBlock<'_>> = Vec::new();
        for _ in 0..counts[1] {
            let name = r.string()?;
            let width = r.u()?;
            let height = r.u()?;
            let color_space = r.u()?;
            if (width == 0) != (height == 0) || width > 16384 || height > 16384 || color_space > 1 {
                return Err("Invalid SKATE embedded texture".into());
            }
            let expected = width as usize * height as usize * 4;
            let block = if version >= 9 {
                r.stored_block(expected)?
            } else {
                let bytes = r.u()? as usize;
                if bytes != expected {
                    return Err("Invalid SKATE embedded texture size".into());
                }
                StoredBlock { expected, method: 0, bytes: r.take(bytes)? }
            };
            let block = if version >= 15 && block.method == 11 {
                let index = u32::from_le_bytes(block.bytes.try_into()
                    .map_err(|_| "Invalid SKATE texture reference size")?) as usize;
                let source = *texture_blocks.get(index).ok_or("Invalid SKATE forward texture reference")?;
                if source.expected != expected { return Err("SKATE texture reference size mismatch".into()); }
                source
            } else { block };
            texture_blocks.push(block);
            textures.push(Texture {
                name,
                width,
                height,
                color_space,
                rgba: Vec::new(),
            });
        }
        let texture_workers = texture_decode::decode(&mut textures, &texture_blocks, workers)?;
        let texture_time = texture_started.elapsed();
        let geometry_started = Instant::now();
        let geometry_counts = [counts[2], counts[3], counts[4]];
        let geometry = if version >= 9 {
            let mut bytes = r.stored(counts[2] as usize * if version >= 12 { 56 } else { 44 })?;
            bytes.extend(r.stored(counts[3] as usize * 4)?);
            bytes.extend(r.stored(counts[4] as usize * if version >= 11 { 48 } else { 44 })?);
            Reader {
                bytes: &bytes,
                at: 0,
            }
            .geometry(geometry_counts, materials.len(), version)?
        } else {
            r.geometry(geometry_counts, materials.len(), version)?
        };
        let geometry_time = geometry_started.elapsed();
        if materials.is_empty()
            || geometry.vertices.is_empty()
            || geometry.indices.is_empty()
        {
            return Err("SKATE requires materials and render geometry".into());
        }
        r.check_count(counts[5], 12)?;
        let mut rails = Vec::new();
        for _ in 0..counts[5] {
            let name = r.string()?;
            let closed = r.u()? != 0;
            let representation = if version >= 10 { r.u()? } else { 0 };
            let (points, native) = match representation {
                0 => (r.points()?, None),
                1 => {
                    let start = r.at;
                    r.take(24)?;
                    let n = r.u()?;
                    r.check_count(n, 120)?;
                    r.take(n as usize * 120)?;
                    (vec![], Some(r.bytes[start..r.at].to_vec()))
                }
                _ => return Err("Unsupported SKATE rail representation".into()),
            };
            rails.push(Rail {
                name,
                closed,
                points,
                native,
            });
        }
        r.check_count(counts[6], 120)?;
        let mut doors = Vec::new();
        for _ in 0..counts[6] {
            let name = r.string()?;
            let frame = [
                r.floats()?,
                r.floats()?,
                r.floats()?,
                r.floats()?,
                r.floats()?,
                r.floats()?,
            ];
            let motion = r.floats()?;
            let response = if version >= 5 {
                Some(r.floats()?)
            } else {
                None
            };
            let friction = r.f()?;
            let restitution = r.f()?;
            let surface = r.u()?;
            let counts = [r.u()?, r.u()?, r.u()?];
            doors.push(Door {
                name,
                frame,
                motion,
                response,
                friction,
                restitution,
                surface,
                geometry: r.geometry(counts, materials.len(), version)?,
            });
        }
        r.check_count(counts[7], 64)?;
        let mut lights = Vec::new();
        for _ in 0..counts[7] {
            let light = Light {
                name: r.string()?,
                kind: r.u()?,
                position: r.floats()?,
                direction: r.floats()?,
                color: r.floats()?,
                intensity: r.f()?,
                range: r.f()?,
                radius: r.f()?,
                inner_cos: r.f()?,
                outer_cos: r.f()?,
            };
            if light.kind > 2
                || light.range <= 0.
                || light.intensity < 0.
                || light.radius < 0.
                || !(-1.0..=1.0).contains(&light.inner_cos)
                || !(-1.0..=1.0).contains(&light.outer_cos)
            {
                return Err("Invalid SKATE light".into());
            }
            lights.push(light);
        }
        r.check_count(counts[8], 24)?;
        let mut routes = Vec::new();
        for _ in 0..counts[8] {
            let name = r.string()?;
            let closed = r.u()? != 0;
            let skaters = r.u()?;
            let speed = r.f()?;
            let spacing = r.f()?;
            let points = r.points()?;
            routes.push(Route {
                rail: Rail {
                    name,
                    closed,
                    points,
                    native: None,
                },
                skaters,
                speed,
                spacing,
            });
        }
        let extension_started = Instant::now();
        let mut extensions = Vec::new();
        if version >= 12 {
            let n = r.u()?;
            r.check_count(n, 20)?;
            for _ in 0..n {
                let tag = r.take(4)?.try_into().unwrap();
                let schema = r.u()?;
                let bytes = r.u()? as usize;
                extensions.push(Extension {
                    tag,
                    schema,
                    payload: r.stored(bytes)?,
                });
            }
        }
        if require_collision && geometry.collision.is_empty() && !extensions.iter().any(|e| e.tag == *b"RWCM" && e.schema == 1 && !e.payload.is_empty()) {
            return Err("SKATE requires triangle collision or an embedded RWCM archive".into());
        }
        if r.at != data.len() {
            return Err(format!(
                "SKATE contains {} unexplained trailing bytes",
                data.len() - r.at
            ));
        }
        eprintln!("MAP_DECODE_TIMING name={:?} textures_ms={} texture_workers={} geometry_ms={} extensions_ms={} total_ms={}",
            name, texture_time.as_millis(), texture_workers, geometry_time.as_millis(),
            extension_started.elapsed().as_millis(), parse_started.elapsed().as_millis());
        Ok(Self {
            version,
            name,
            spawn,
            heading,
            environment,
            materials,
            textures,
            geometry,
            rails,
            doors,
            lights,
            routes,
            extensions,
        })
    }
}

/// Vertex, triangle and collision counts from the package header.
///
/// Reads at most 64 KB. Geometry and texture blocks stay on disk.
fn geometry_counts(path: &Path) -> Result<(u64, u64, u64), String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut header = Vec::new();
    file.by_ref()
        .take(64 * 1024)
        .read_to_end(&mut header)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    geometry_counts_from_header(&header).map_err(|e| format!("{}: {e}", path.display()))
}

fn geometry_counts_from_header(bytes: &[u8]) -> Result<(u64, u64, u64), String> {
    let mut cursor = Header { bytes, at: 0 };
    let magic = cursor.take(8)?;
    if &magic[..5] != b"SKATE"
        || magic[7] != 0
        || !magic[5].is_ascii_digit()
        || !magic[6].is_ascii_digit()
    {
        return Err("Invalid SKATE magic/version".into());
    }
    let version = (magic[5] - b'0') * 10 + (magic[6] - b'0');
    if !(1..=15).contains(&version) {
        return Err("Unsupported SKATE version: reader supports 01 through 15; newer packages require an update".into());
    }
    if cursor.u32()? != 0x1234_5678 {
        return Err("Invalid SKATE endian marker".into());
    }
    let name_len = cursor.u32()? as usize;
    if name_len > 4096 {
        return Err("SKATE map name is too long to inspect".into());
    }
    cursor.take(name_len)?;
    cursor.take(16)?;
    let environment = if version >= 6 {
        45
    } else if version >= 3 {
        14
    } else {
        12
    };
    cursor.take(environment * 4)?;
    let count_len = if version >= 8 {
        9
    } else if version >= 7 {
        8
    } else if version >= 4 {
        7
    } else {
        6
    };
    let mut counts = [0u32; 9];
    for count in &mut counts[..count_len] {
        *count = cursor.u32()?;
    }
    let indices = u64::from(counts[3]);
    Ok((u64::from(counts[2]), indices / 3, u64::from(counts[4])))
}

struct Header<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Header<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).ok_or("Truncated SKATE header")?;
        let slice = self.bytes.get(self.at..end).ok_or("Truncated SKATE header")?;
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}

#[cfg(test)]
mod budget_load_tests {
    use super::*;

    fn header(version: u8, name: &str, vertices: u32, indices: u32, collision: u32) -> Vec<u8> {
        let mut out = format!("SKATE{version:02}\0").into_bytes();
        out.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&[0u8; 16]);
        let environment = if version >= 6 { 45 } else if version >= 3 { 14 } else { 12 };
        out.extend_from_slice(&vec![0u8; environment * 4]);
        let count_len = if version >= 8 { 9 } else if version >= 7 { 8 } else if version >= 4 { 7 } else { 6 };
        let mut counts = [0u32; 9];
        counts[2] = vertices;
        counts[3] = indices;
        counts[4] = collision;
        for count in &counts[..count_len] {
            out.extend_from_slice(&count.to_le_bytes());
        }
        out
    }

    #[test]
    fn load_refuses_a_city_header_before_decoding_a_body() {
        let dir = std::env::temp_dir().join(format!("skate-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let city = dir.join("city.skate");
        std::fs::write(&city, header(9, "Helsinki", 10_972_270, 27_685_644, 9_228_550)).unwrap();
        let error = SkateMap::load(&city).unwrap_err();
        assert!(error.contains("10972270 vertices"), "{error}");
        assert!(!error.contains("Truncated"), "{error}");
        let downtown = dir.join("DownTown.skate");
        std::fs::write(&downtown, header(14, "DownTown", 2_626_756, 5_666_682, 0)).unwrap();
        let error = SkateMap::load(&downtown).unwrap_err();
        assert!(!error.contains("limit") && !error.contains("vertices. The limit"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn installed_retail_headers_fit_and_the_city_package_is_not_read() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../skate3rust-windows-x64/data/installations");
        let Ok(entries) = std::fs::read_dir(&root) else { return };
        let Some(maps) = entries.flatten().map(|entry| entry.path().join("maps")).find(|path| path.join("StartPark.skate").is_file()) else {
            return;
        };
        assert_eq!(super::geometry_counts(&maps.join("StartPark.skate")).unwrap(), (15_957, 13_176, 0));
        assert_eq!(super::geometry_counts(&maps.join("DownTown.skate")).unwrap(), (2_626_756, 1_888_894, 0));
        let started = std::time::Instant::now();
        let error = SkateMap::load(&maps.join("helsinki_kallio_blend.skate")).unwrap_err();
        assert!(started.elapsed().as_secs() < 3, "{:?}", started.elapsed());
        assert!(error.contains("1120 MB"), "{error}");
        assert!(error.contains("not read"), "{error}");
    }
}
