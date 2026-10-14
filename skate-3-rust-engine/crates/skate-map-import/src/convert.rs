//! Triangles from an FBX scene, written as a portable SKATE09 map.
use crate::scene::{Scene, Triangle};
use serde::{Deserialize, Serialize};
use skate_data::skate_map::{Collision, Geometry, Material, Rail, SkateMap, Texture, Vertex};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImportOptions {
    /// Multiplies the file's real-world size. 1.0 keeps metres.
    pub scale: f32,
    pub name: String,
    /// `full`, or `grid:2` to cluster vertices onto a 2 metre grid.
    pub collision: String,
    pub floor: bool,
    /// Spawn in final map metres, after scale, crop and recentring.
    pub spawn: Option<[f32; 3]>,
    /// Yaw in radians, stored the same way as `SkateMap::heading`. Zero faces +Z.
    pub heading: f32,
    /// Longest texture edge, in pixels. 0 keeps the embedded size.
    pub max_texture: u32,
    /// Physics surface id, 0 through 13. 1 is smooth and 2 is rough.
    /// Audio stays concrete (3), the same default the demo map writer uses.
    pub surface: u32,
    /// Drops triangles whose centre is outside this XZ rectangle, in metres
    /// after scaling and before the map is recentred.
    pub crop: Option<[f32; 4]>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            name: "Imported map".into(),
            collision: "full".into(),
            floor: false,
            spawn: None,
            heading: 0.0,
            max_texture: 0,
            surface: 1,
            crop: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ImportReport {
    pub output: std::path::PathBuf,
    pub name: String,
    pub width_m: f32,
    pub depth_m: f32,
    pub height_m: f32,
    pub triangles: usize,
    pub collision_triangles: usize,
    pub textures: usize,
    pub texture_bytes: usize,
    pub spawn: [f32; 3],
    pub warnings: Vec<String>,
}

impl std::fmt::Display for ImportReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Imported {} ", self.name)?;
        writeln!(f, "  {}", self.output.display())?;
        writeln!(f, "  {:.1} m by {:.1} m, {:.1} m tall", self.width_m, self.depth_m, self.height_m)?;
        writeln!(f, "  {} render triangles, {} collision triangles, {} textures ({} MB)",
            self.triangles, self.collision_triangles, self.textures, self.texture_bytes / 1_048_576)?;
        writeln!(f, "  spawn {:.1}, {:.1}, {:.1}", self.spawn[0], self.spawn[1], self.spawn[2])?;
        for warning in &self.warnings { writeln!(f, "  warning: {warning}")?; }
        Ok(())
    }
}

pub fn import_file(source: &Path, output_dir: &Path, options: &ImportOptions) -> Result<ImportReport, String> {
    let bytes = std::fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let (mut map, mut report) = convert_bytes(&bytes, options)?;
    if load_rails(source, &mut map)? {
        report.warnings.push("Loaded grind rails from the sidecar JSON.".into());
    }
    let encoded = map.to_v9_bytes()?;
    let parsed = SkateMap::parse(&encoded)?;
    parsed.validate_runtime()?;
    std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    let output = output_dir.join(format!("{}.skate", file_stem(&options.name)));
    std::fs::write(&output, &encoded).map_err(|e| format!("{}: {e}", output.display()))?;
    let sidecar = output_dir.join(format!("{}.import.json", file_stem(&options.name)));
    std::fs::write(&sidecar, serde_json::to_vec_pretty(options).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    report.output = output;
    Ok(report)
}

pub fn convert_bytes(bytes: &[u8], options: &ImportOptions) -> Result<(SkateMap, ImportReport), String> {
    if !(0.001..=100.0).contains(&options.scale) {
        return Err("Scale must be between 0.001 and 100".into());
    }
    if options.surface > 13 {
        return Err("Surface must be from 0 to 13".into());
    }
    if !options.heading.is_finite() {
        return Err("Heading must be a finite number of radians".into());
    }
    if options.name.is_empty() {
        return Err("Map name must not be empty".into());
    }
    if options.crop.is_some_and(|crop| crop.iter().any(|value| !value.is_finite())) {
        return Err("Crop must be a finite XZ rectangle".into());
    }
    let scene = crate::scene::read(&crate::fbx::parse(bytes)?)?;
    convert_scene(scene, options)
}

pub(crate) fn convert_scene(scene: Scene, options: &ImportOptions) -> Result<(SkateMap, ImportReport), String> {
    let grid = collision_grid(&options.collision)?;
    let mut warnings = scene.warnings;
    let mut triangles = scene.triangles;
    for triangle in &mut triangles {
        for point in &mut triangle.positions {
            *point = point.map(|value| value * options.scale);
        }
    }
    if let Some([x0, z0, x1, z1]) = options.crop {
        let (min_x, max_x) = (x0.min(x1), x0.max(x1));
        let (min_z, max_z) = (z0.min(z1), z0.max(z1));
        let before = triangles.len();
        triangles.retain(|triangle| {
            let centre_x = triangle.positions.iter().map(|point| point[0]).sum::<f32>() / 3.0;
            let centre_z = triangle.positions.iter().map(|point| point[2]).sum::<f32>() / 3.0;
            (min_x..=max_x).contains(&centre_x) && (min_z..=max_z).contains(&centre_z)
        });
        warnings.push(format!("Crop kept {} of {before} triangles.", triangles.len()));
    }
    if triangles.is_empty() {
        return Err("Nothing is left after the crop".into());
    }
    let (min, max) = bounds(&triangles);
    let centre_x = (min[0] + max[0]) * 0.5;
    let centre_z = (min[2] + max[2]) * 0.5;
    for triangle in &mut triangles {
        for point in &mut triangle.positions {
            point[0] -= centre_x;
            point[1] -= min[1];
            point[2] -= centre_z;
        }
    }
    let (min, max) = bounds(&triangles);
    if triangles.iter().any(|triangle| triangle.positions.iter().flatten().any(|value| !value.is_finite())) {
        return Err("FBX positions are not finite numbers".into());
    }
    if scene.materials.is_empty() {
        return Err("FBX mesh has no material".into());
    }
    if triangles.iter().any(|triangle| triangle.material >= scene.materials.len()) {
        return Err("FBX triangle material is outside the material list".into());
    }
    let surface = riding_surface(options.surface);
    let mut textures = Vec::new();
    let mut materials = Vec::new();
    for (index, material) in scene.materials.iter().enumerate() {
        let albedo = if let Some(texture) = &material.texture {
            let mut rgba = texture.rgba.clone();
            let (width, height) = fit(&mut rgba, texture.width, texture.height, options.max_texture);
            if width > 16384 || height > 16384 {
                return Err(format!(
                    "Texture {} is {width} by {height}. Set max_texture to 16384 or below.",
                    texture.name
                ));
            }
            // PNG rows arrive top-down. The map stores them bottom-up, matching the Blender exporter.
            flip_rows(&mut rgba, width, height);
            let name = if texture.name.is_empty() {
                format!("Texture {}", index + 1)
            } else {
                texture.name.clone()
            };
            textures.push(Texture { name, width, height, color_space: 1, rgba });
            textures.len() as u32
        } else {
            let pixel = material.color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0) as u8);
            textures.push(Texture {
                name: format!("{} color", material.name),
                width: 1, height: 1, color_space: 1,
                rgba: vec![pixel[0], pixel[1], pixel[2], 255],
            });
            textures.len() as u32
        };
        materials.push(portable_material(&material.name, albedo, options.surface));
    }
    let indirect = textures.len() as u32 + 1;
    textures.push(Texture {
        name: "Indirect".into(), width: 1, height: 1, color_space: 0, rgba: vec![210, 210, 210, 255],
    });
    for material in &mut materials {
        material.textures[1] = indirect;
    }
    let (vertices, indices) = dedup(&triangles);
    let mut collision = if let Some(size) = grid {
        grid_collision(&triangles, size, surface)
    } else {
        full_collision(&triangles, surface)
    };
    if collision.len() != triangles.len() {
        warnings.push(format!("Collision kept {} of {} triangles.", collision.len(), triangles.len()));
    }
    if collision.is_empty() {
        return Err("Every collision triangle was degenerate".into());
    }
    if options.floor {
        collision.extend(catch_floor(min, max, surface));
    }
    let spawn = if let Some(spawn) = options.spawn {
        if spawn.iter().any(|value| !value.is_finite()) {
            return Err("Spawn must be a finite position".into());
        }
        spawn
    } else if let Some(spawn) = find_spawn(&collision) {
        spawn
    } else {
        warnings.push("No flat ground under the centre, so the spawn is 1 m up.".into());
        [0.0, 1.0, 0.0]
    };
    let map = SkateMap {
        version: 9,
        name: options.name.clone(),
        spawn,
        heading: options.heading,
        environment: default_environment(),
        materials,
        textures,
        geometry: Geometry { vertices, indices, collision },
        rails: Vec::new(),
        doors: Vec::new(),
        lights: Vec::new(),
        routes: Vec::new(),
        extensions: Vec::new(),
    };
    let report = ImportReport {
        output: std::path::PathBuf::new(),
        name: options.name.clone(),
        width_m: max[0] - min[0],
        depth_m: max[2] - min[2],
        height_m: max[1] - min[1],
        triangles: map.geometry.indices.len() / 3,
        collision_triangles: map.geometry.collision.len(),
        textures: map.textures.len(),
        texture_bytes: map.textures.iter().map(|texture| texture.rgba.len()).sum(),
        spawn,
        warnings,
    };
    Ok((map, report))
}

pub fn load_rails(source: &Path, map: &mut SkateMap) -> Result<bool, String> {
    let Some(path) = rails_file(source) else { return Ok(false) };
    #[derive(Deserialize)]
    struct Authored { name: String, #[serde(default)] closed: bool, points: Vec<[f32; 3]> }
    let rails: Vec<Authored> = serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    for rail in rails {
        if rail.name.is_empty() { return Err("Rail name must not be empty".into()); }
        if rail.points.len() < 2 { return Err(format!("Rail {} needs two points", rail.name)); }
        if rail.points.iter().any(|point| point.iter().any(|value| !value.is_finite())) {
            return Err(format!("Rail {} has a non-finite point", rail.name));
        }
        map.rails.push(Rail { name: rail.name, closed: rail.closed, points: rail.points, native: None });
    }
    Ok(true)
}

fn rails_file(source: &Path) -> Option<std::path::PathBuf> {
    let stem = source.file_stem()?.to_string_lossy();
    let path = source.with_file_name(format!("{stem}.rails.json"));
    path.is_file().then_some(path)
}

fn collision_grid(value: &str) -> Result<Option<f32>, String> {
    if value.eq_ignore_ascii_case("full") { return Ok(None); }
    let Some(size) = value.get(5..).filter(|_| value.len() >= 5 && value[..5].eq_ignore_ascii_case("grid:")) else {
        return Err("Collision must be 'full' or 'grid:METRES'".into());
    };
    let size: f32 = size.parse().map_err(|_| "Grid size must be a number of metres")?;
    if !(0.05..=50.0).contains(&size) { return Err("Grid size must be from 0.05 m to 50 m".into()); }
    Ok(Some(size))
}

/// Wheel probes decode the collision tag as `audio | (physics << 7) | (pattern << 12)`.
/// Audio 3 is concrete, which is what `tools/make_skate_demo.py` writes.
fn riding_surface(physics: u32) -> u32 {
    3 | (physics << 7)
}

fn portable_material(name: &str, albedo: u32, surface: u32) -> Material {
    Material {
        name: name.to_string(),
        flags: 1,
        friction: 0.6,
        restitution: 0.1,
        color: [1.0, 1.0, 1.0],
        roughness: 0.85,
        emissive: 0.0,
        textures: [albedo, 0, 0, 0, 0],
        indirect_strength: 1.0,
        alpha_mode: 0,
        alpha_cutoff: 0.5,
        audio: 3,
        physics: surface,
        pattern: 0,
        depth_layer: None,
        retail_definition: None,
    }
}

fn default_environment() -> Vec<f32> {
    let mut values = vec![0.2; 45];
    values[0] = 0.09;
    values[1] = 0.34;
    values[2] = 0.72;
    values
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct CornerKey {
    position: [u32; 3],
    normal: [u32; 3],
    uv: [u32; 2],
    material: u32,
}

fn dedup(triangles: &[Triangle]) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::with_capacity(triangles.len() * 3);
    let mut lookup = HashMap::<CornerKey, u32>::new();
    for triangle in triangles {
        let material = triangle.material as u32 + 1;
        for corner in 0..3 {
            let key = CornerKey {
                position: triangle.positions[corner].map(f32::to_bits),
                normal: triangle.normals[corner].map(f32::to_bits),
                uv: triangle.uvs[corner].map(f32::to_bits),
                material,
            };
            let index = *lookup.entry(key).or_insert_with(|| {
                let index = vertices.len() as u32;
                vertices.push(Vertex {
                    position: triangle.positions[corner],
                    normal: triangle.normals[corner],
                    uv: triangle.uvs[corner],
                    lightmap_uv: [0.5, 0.5],
                    material,
                    decal_uv: None,
                    tangent_frame: None,
                });
                index
            });
            indices.push(index);
        }
    }
    (vertices, indices)
}

fn full_collision(triangles: &[Triangle], surface: u32) -> Vec<Collision> {
    triangles.iter().filter_map(|triangle| {
        solid(triangle.positions).then(|| Collision {
            points: triangle.positions,
            surface,
            material: triangle.material as u32 + 1,
            native_edges: None,
        })
    }).collect()
}

fn grid_collision(triangles: &[Triangle], size: f32, surface: u32) -> Vec<Collision> {
    let mut cells: HashMap<[i32; 3], ([f32; 3], u32)> = HashMap::new();
    let key = |point: [f32; 3]| point.map(|value| (value / size).round() as i32);
    for triangle in triangles {
        for point in triangle.positions {
            let entry = cells.entry(key(point)).or_insert(([0.0; 3], 0));
            for axis in 0..3 { entry.0[axis] += point[axis]; }
            entry.1 += 1;
        }
    }
    let mut collision = Vec::new();
    let mut seen = HashSet::new();
    for triangle in triangles {
        let points = triangle.positions.map(|point| {
            let (sum, count) = cells[&key(point)];
            sum.map(|value| value / count as f32)
        });
        if !solid(points) || !seen.insert(points.map(|point| point.map(f32::to_bits))) { continue; }
        collision.push(Collision {
            points,
            surface,
            material: triangle.material as u32 + 1,
            native_edges: None,
        });
    }
    collision
}

fn catch_floor(min: [f32; 3], max: [f32; 3], surface: u32) -> Vec<Collision> {
    let pad = ((max[0] - min[0]).max(max[2] - min[2]) * 0.1).max(20.0);
    let y = -0.25;
    let (x0, x1) = (min[0] - pad, max[0] + pad);
    let (z0, z1) = (min[2] - pad, max[2] + pad);
    vec![
        Collision { points: [[x0, y, z0], [x0, y, z1], [x1, y, z0]], surface, material: 1, native_edges: None },
        Collision { points: [[x1, y, z0], [x0, y, z1], [x1, y, z1]], surface, material: 1, native_edges: None },
    ]
}

fn find_spawn(collision: &[Collision]) -> Option<[f32; 3]> {
    for radius in [0.0, 2.0, 8.0, 32.0, 128.0, 512.0, 2048.0, 8192.0] {
        for step in 0..8 {
            let angle = step as f32 / 8.0 * std::f32::consts::TAU;
            let (x, z) = if radius == 0.0 { (0.0, 0.0) } else { (angle.cos() * radius, angle.sin() * radius) };
            let mut heights = Vec::new();
            for triangle in collision {
                if triangle.points.iter().all(|point| point[1] < -0.1) { continue; }
                if let Some(y) = ground_y(triangle.points, x, z) {
                    heights.push(y);
                }
            }
            if let Some(y) = lowest_open(&heights) {
                return Some([x, y, z]);
            }
        }
    }
    let mut nearest: Option<(f32, f32, f32)> = None;
    for triangle in collision {
        if triangle.points.iter().all(|point| point[1] < -0.1) { continue; }
        let x = triangle.points.iter().map(|point| point[0]).sum::<f32>() / 3.0;
        let z = triangle.points.iter().map(|point| point[2]).sum::<f32>() / 3.0;
        if ground_y(triangle.points, x, z).is_none() { continue; }
        let distance = x * x + z * z;
        if nearest.is_none_or(|(best, _, _)| distance < best) {
            nearest = Some((distance, x, z));
        }
    }
    let (_, x, z) = nearest?;
    let heights: Vec<f32> = collision.iter().filter_map(|triangle| {
        if triangle.points.iter().all(|point| point[1] < -0.1) { return None; }
        ground_y(triangle.points, x, z)
    }).collect();
    lowest_open(&heights).map(|y| [x, y, z])
}

/// Lowest upward hit with two metres of clearance. The catch floor is ignored while real ground exists.
fn lowest_open(heights: &[f32]) -> Option<f32> {
    let raised: Vec<f32> = heights.iter().copied().filter(|y| *y >= -0.1).collect();
    let set = if raised.is_empty() { heights } else { raised.as_slice() };
    set.iter().copied().filter(|y| {
        !set.iter().any(|other| *other > *y + 0.05 && *other < *y + 2.0)
    }).reduce(f32::min)
}

fn ground_y(points: [[f32; 3]; 3], x: f32, z: f32) -> Option<f32> {
    let [a, b, c] = points;
    let up = {
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let length = cross.iter().map(|value| value * value).sum::<f32>().sqrt();
        if length < 1e-8 { return None; }
        cross[1] / length
    };
    if up < 0.35 { return None; }
    let (abx, abz) = (b[0] - a[0], b[2] - a[2]);
    let (acx, acz) = (c[0] - a[0], c[2] - a[2]);
    let det = abx * acz - abz * acx;
    if det.abs() < 1e-8 { return None; }
    let (px, pz) = (x - a[0], z - a[2]);
    let u = (px * acz - pz * acx) / det;
    let v = (abx * pz - abz * px) / det;
    if u < -0.001 || v < -0.001 || u + v > 1.001 { return None; }
    Some(a[1] + u * (b[1] - a[1]) + v * (c[1] - a[1]))
}

fn solid(points: [[f32; 3]; 3]) -> bool {
    let [a, b, c] = points;
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    cross.iter().map(|value| value * value).sum::<f32>() > 1e-8
}

fn bounds(triangles: &[Triangle]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for triangle in triangles {
        for point in triangle.positions {
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
    }
    (min, max)
}

fn flip_rows(rgba: &mut [u8], width: u32, height: u32) {
    let row = width as usize * 4;
    let height = height as usize;
    for y in 0..height / 2 {
        let (top, bottom) = (y * row, (height - 1 - y) * row);
        for offset in 0..row {
            rgba.swap(top + offset, bottom + offset);
        }
    }
}

fn fit(rgba: &mut Vec<u8>, width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    if max_edge == 0 || width.max(height) <= max_edge { return (width, height); }
    let scale = max_edge as f32 / width.max(height) as f32;
    let (new_w, new_h) = ((width as f32 * scale).round().max(1.0) as u32, (height as f32 * scale).round().max(1.0) as u32);
    let mut resized = vec![0u8; new_w as usize * new_h as usize * 4];
    for y in 0..new_h {
        for x in 0..new_w {
            let source_x = (x as f32 / new_w as f32 * width as f32) as usize;
            let source_y = (y as f32 / new_h as f32 * height as f32) as usize;
            let from = (source_y * width as usize + source_x) * 4;
            let to = (y as usize * new_w as usize + x as usize) * 4;
            resized[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
    *rgba = resized;
    (new_w, new_h)
}

fn file_stem(name: &str) -> String {
    let stem: String = name.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect();
    if stem.is_empty() { "Imported_map".into() } else { stem }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recentres_a_triangle_and_adds_a_floor() {
        let scene = Scene {
            materials: vec![crate::scene::Material { name: "Ground".into(), color: [1.0, 1.0, 1.0], texture: None }],
            triangles: vec![Triangle {
                positions: [[0.0, 2.0, 0.0], [0.0, 2.0, 4.0], [4.0, 2.0, 0.0]],
                normals: [[0.0, 1.0, 0.0]; 3],
                uvs: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                material: 0,
            }],
            warnings: Vec::new(),
        };
        let options = ImportOptions { scale: 0.5, name: "Yard".into(), floor: true, ..ImportOptions::default() };
        let (map, report) = convert_scene(scene, &options).unwrap();
        assert_eq!(map.geometry.collision.len(), 3);
        assert!((report.width_m - 2.0).abs() < 0.001, "{}", report.width_m);
        assert!(map.spawn[1].abs() < 0.001, "spawn should sit on the mesh, got {}", map.spawn[1]);
        let floor = Collision {
            points: [[-4.0, 0.0, -4.0], [0.0, 0.0, 4.0], [4.0, 0.0, -4.0]],
            surface: 1, material: 1, native_edges: None,
        };
        let roof = Collision {
            points: [[-4.0, 30.0, -4.0], [0.0, 30.0, 4.0], [4.0, 30.0, -4.0]],
            surface: 1, material: 1, native_edges: None,
        };
        let chosen = find_spawn(&[roof, floor]).unwrap();
        assert!(chosen[1].abs() < 0.05, "{chosen:?}");
        let far = Collision {
            points: [[1990.0, 12.0, -4.0], [2000.0, 12.0, 4.0], [2010.0, 12.0, -4.0]],
            surface: 1, material: 1, native_edges: None,
        };
        let distant = find_spawn(&[far]).unwrap();
        assert!((distant[0] - 2000.0).abs() < 1.0, "{distant:?}");
        assert!((distant[1] - 12.0).abs() < 0.05, "{distant:?}");
        assert!(map.geometry.vertices.iter().all(|vertex| vertex.position[1].abs() < 0.001));
        assert_eq!(map.geometry.collision[0].surface, 3 | (1 << 7));
        assert_eq!(map.materials[0].physics, 1);
        assert_eq!(map.materials[0].audio, 3);
        assert_eq!(map.materials[0].textures[1], 2);
        let parsed = SkateMap::parse(&map.to_v9_bytes().unwrap()).unwrap();
        parsed.validate_runtime().unwrap();
        assert_eq!(parsed.name, "Yard");
        assert_eq!(parsed.geometry.indices.len(), 3);
    }

    #[test]
    fn rough_surface_is_packed_into_the_collision_tag() {
        let (map, _) = convert_scene(ground(), &ImportOptions { surface: 2, ..ImportOptions::default() }).unwrap();
        assert_eq!(map.materials[0].physics, 2);
        assert_eq!(map.geometry.collision[0].surface, 3 | (2 << 7));
    }

    #[test]
    fn crop_and_grid_drop_what_they_should() {
        let inside = ImportOptions { crop: Some([0.0, 0.0, 10.0, 10.0]), ..ImportOptions::default() };
        assert_eq!(convert_scene(ground(), &inside).unwrap().0.geometry.indices.len(), 3);
        let outside = ImportOptions { crop: Some([10.0, 10.0, 12.0, 12.0]), ..ImportOptions::default() };
        assert!(convert_scene(ground(), &outside).unwrap_err().contains("crop"));
        let kept = ImportOptions { collision: "grid:0.5".into(), ..ImportOptions::default() };
        assert_eq!(convert_scene(ground(), &kept).unwrap().0.geometry.collision.len(), 1);
        let collapsed = ImportOptions { collision: "GRID:50".into(), ..ImportOptions::default() };
        assert!(convert_scene(ground(), &collapsed).unwrap_err().contains("degenerate"));
    }

    #[test]
    fn flips_texture_rows_and_writes_the_package() {
        let mut scene = ground();
        scene.materials[0].texture = Some(crate::scene::TexturePixels {
            name: "Albedo".into(),
            width: 1,
            height: 2,
            rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
        });
        let (map, _) = convert_scene(scene, &ImportOptions::default()).unwrap();
        assert_eq!(&map.textures[0].rgba[..4], &[5, 6, 7, 8]);
        let dir = std::env::temp_dir().join(format!("skate-map-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("yard.fbx");
        std::fs::write(&source, b"not an fbx").unwrap();
        let error = import_file(&source, &dir, &ImportOptions::default()).unwrap_err();
        assert!(error.contains("binary FBX") || error.contains("Not a binary"), "{error}");
        std::fs::write(&source, sample_fbx()).unwrap();
        std::fs::write(dir.join("yard.rails.json"), br#"[{"name":"ledge","points":[[0,1,0],[2,1,0]]}]"#).unwrap();
        let options = ImportOptions { name: "Yard".into(), spawn: Some([1.0, 2.0, 3.0]), ..ImportOptions::default() };
        let report = import_file(&source, &dir, &options).unwrap();
        let saved = std::fs::read(dir.join("Yard.import.json")).unwrap();
        let loaded: ImportOptions = serde_json::from_slice(&saved).unwrap();
        assert_eq!(loaded.spawn, Some([1.0, 2.0, 3.0]));
        let map = SkateMap::parse(&std::fs::read(&report.output).unwrap()).unwrap();
        map.validate_runtime().unwrap();
        assert_eq!(map.rails.len(), 1);
        assert_eq!(map.spawn, [1.0, 2.0, 3.0]);
        assert!(report.width_m > 0.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "reads the 71 MB laru FBX; optional and memory-heavy"]
    fn imports_laru_fbx_when_present() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fbx/laru_LOD_40cm_scale_1.fbx");
        if !path.is_file() {
            eprintln!("laru FBX is not in the workspace, skipping");
            return;
        }
        let dir = std::env::temp_dir().join("skate-laru-import");
        let _ = std::fs::remove_dir_all(&dir);
        let options = ImportOptions { name: "Laru".into(), max_texture: 64, ..ImportOptions::default() };
        let report = import_file(&path, &dir, &options).unwrap();
        assert!(report.width_m > 1000.0, "{}", report.width_m);
        assert!(report.triangles > 100_000, "{}", report.triangles);
        assert!(report.textures > 100, "{}", report.textures);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn ground() -> Scene {
        Scene {
            materials: vec![crate::scene::Material { name: "Ground".into(), color: [1.0, 1.0, 1.0], texture: None }],
            triangles: vec![Triangle {
                positions: [[0.0, 2.0, 0.0], [0.0, 2.0, 4.0], [4.0, 2.0, 0.0]],
                normals: [[0.0, 1.0, 0.0]; 3],
                uvs: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                material: 0,
            }],
            warnings: Vec::new(),
        }
    }

    fn sample_fbx() -> Vec<u8> {
        use crate::fbx::{Node, Prop};
        let property = |name: &str, values: &[f64]| {
            let mut props = vec![Prop::Bytes(name.as_bytes().to_vec())];
            props.extend(values.iter().copied().map(Prop::F64));
            Node { name: "P".into(), props, children: Vec::new() }
        };
        let integer = |name: &str, value: i32| Node {
            name: "P".into(),
            props: vec![Prop::Bytes(name.as_bytes().to_vec()), Prop::I32(value)],
            children: Vec::new(),
        };
        let link = |child: i64, parent: i64| Node {
            name: "C".into(),
            props: vec![Prop::Bytes(b"OO".to_vec()), Prop::I64(child), Prop::I64(parent)],
            children: Vec::new(),
        };
        crate::fbx::encode(&[
            Node { name: "GlobalSettings".into(), props: Vec::new(), children: vec![Node {
                name: "Properties70".into(), props: Vec::new(),
                children: vec![
                    property("UnitScaleFactor", &[100.0]),
                    integer("UpAxis", 1), integer("UpAxisSign", 1),
                    integer("FrontAxis", 2), integer("FrontAxisSign", 1),
                    integer("CoordAxis", 0), integer("CoordAxisSign", 1),
                ],
            }]},
            Node { name: "Objects".into(), props: Vec::new(), children: vec![
                Node {
                    name: "Model".into(),
                    props: vec![Prop::I64(1), Prop::Bytes(b"Ground".to_vec()), Prop::Bytes(b"Mesh".to_vec())],
                    children: vec![Node { name: "Properties70".into(), props: Vec::new(), children: vec![
                        property("Lcl Scaling", &[1.0, 1.0, 1.0]),
                    ]}],
                },
                Node { name: "Geometry".into(), props: vec![Prop::I64(2)], children: vec![
                    Node { name: "Vertices".into(), props: vec![Prop::F32s(vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0])], children: Vec::new() },
                    Node { name: "PolygonVertexIndex".into(), props: vec![Prop::I32s(vec![0, 1, !2])], children: Vec::new() },
                ]},
                Node { name: "Material".into(), props: vec![Prop::I64(3), Prop::Bytes(b"Paint".to_vec())], children: Vec::new() },
            ]},
            Node { name: "Connections".into(), props: Vec::new(), children: vec![link(2, 1), link(3, 1)] },
        ], 7400)
    }
}
