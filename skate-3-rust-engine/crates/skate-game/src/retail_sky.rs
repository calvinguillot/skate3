//! Authored camera-relative sky dome, its radial sun gradient, and the fog frame
//! the world shader reads.
//!
//! The dome itself is a conventional one-draw material: a single mesh with a
//! single panorama, so none of the slabbing machinery in `retail_render` applies.
//! What *does* have to reach the world is the authored environment — sun
//! direction and fog ramp. Those live in the per-material storage buffer, which
//! is encoded once while the material table is built, so the sky package is
//! loaded *before* the world and its environment is threaded through
//! `MaterialTable::build` rather than patched into materials afterwards.
use bevy::{
    asset::{AssetPath, RenderAssetUsages, embedded_path},
    prelude::*,
    render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType},
    shader::ShaderRef,
};

use crate::map_render::{AssetSink, SceneCommands};
use std::path::Path;

/// Marks a sky dome spawned for a map that has no authored sky of its own.
/// Retail domes do not carry this, so a preset change never removes them.
#[derive(Component)]
pub(crate) struct ImportedSky {
    pub multiplier: f32,
}

/// Retail packages in `private/native-skies`, then user panoramas in `settings/skies`.
/// A user file that reuses a retail name is left out of the list.
pub(crate) fn available(asset_root: &Path) -> Vec<String> {
    let mut names = list_packages(&asset_root.join("private/native-skies"));
    let retail: std::collections::BTreeSet<String> = names.iter().cloned().collect();
    for name in list_packages(&user_sky_dir(asset_root)) {
        if !retail.contains(&name) {
            names.push(name);
        }
    }
    names.sort();
    names.dedup();
    names
}

pub(crate) fn user_sky_dir(asset_root: &Path) -> std::path::PathBuf {
    asset_root.parent().unwrap_or(asset_root).join("settings/skies")
}

fn list_packages(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return names };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") { continue; }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else { continue };
        if name.is_empty() || name.contains(['/', '\\', ':', '.']) { continue; }
        if dir.join(format!("{name}.rgba")).is_file() {
            names.push(name.to_string());
        }
    }
    names
}

fn package_dir(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    for dir in [root.join("private/native-skies"), user_sky_dir(root)] {
        if dir.join(format!("{name}.json")).is_file() && dir.join(format!("{name}.rgba")).is_file() {
            return Some(dir);
        }
    }
    None
}

pub(crate) fn cycle_preset(current: &str, names: &[String], direction: i32) -> String {
    let mut options = vec!["Off".to_string()];
    options.extend(names.iter().cloned());
    if options.len() == 1 { return "Off".into(); }
    let index = options.iter().position(|name| name == current).unwrap_or(0);
    let next = (index as i32 + direction).rem_euclid(options.len() as i32) as usize;
    options[next].clone()
}

/// What the world shader needs from the sky package.
///
/// The defaults are the no-sky case: the same sun vector the material table used
/// before skies were authored, and a fog ramp that evaluates to zero fog for
/// every distance (`saturate(d*0 + 0)`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SkyEnvironment {
    pub sun_direction: Vec4,
    pub fog_ramp: Vec4,
    pub fog_color: Vec4,
}

impl Default for SkyEnvironment {
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(4., 7., 4.).normalize().extend(0.),
            fog_ramp: Vec4::new(0., 0., 1., 0.),
            fog_color: Vec4::ZERO,
        }
    }
}

#[derive(Clone, Debug, ShaderType)]
pub(crate) struct SkyParams {
    /// anchor height, scene exposure, sky multiplier, sun angular scale (0 disables)
    pub settings: Vec4,
    pub sun_direction: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct SkyMaterial {
    #[uniform(0)]
    pub params: SkyParams,
    #[texture(1)]
    #[sampler(2)]
    pub panorama: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub sun: Option<Handle<Image>>,
}

impl Material for SkyMaterial {
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn vertex_shader() -> ShaderRef {
        AssetPath::from(embedded_path!("retail_sky.wgsl"))
            .with_source("embedded")
            .into()
    }

    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }

    fn specialize(
        _: &bevy::pbr::MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &bevy::mesh::MeshVertexBufferLayoutRef,
        _: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // The camera sits inside the dome, and the dome is pushed to the far
        // plane in the vertex shader, so it must neither cull nor occlude.
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = false;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Authored package
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct FogFrame {
    ramp: [f32; 4],
    colour: [f32; 4],
}

#[derive(serde::Deserialize)]
struct Environment {
    anchor_height: f32,
    sun_direction: [f32; 3],
    sun_scale: f32,
    multiplier: f32,
    location_chain: Vec<String>,
    #[serde(default)]
    fog_frame: Option<FogFrame>,
}

#[derive(serde::Deserialize)]
struct Sky {
    width: u32,
    height: u32,
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    #[serde(default)]
    sun_width: u32,
    #[serde(default)]
    sun_height: u32,
    environment: Option<Environment>,
}

/// A validated sky package, decoded off the main thread with the rest of the map.
pub(crate) struct SkyPackage {
    sky: Sky,
    rgba: Vec<u8>,
    sun: Option<Vec<u8>>,
}

impl SkyPackage {
    pub(crate) fn load(root: &std::path::Path, name: &str) -> Result<Self, String> {
        if name.is_empty() || name.contains(['/', '\\', ':']) || matches!(name, "." | "..") {
            return Err("Invalid retail sky name".into());
        }
        let base = package_dir(root, name).ok_or_else(|| format!("Sky {name} is not installed"))?;
        let read = |suffix: &str| {
            std::fs::read(base.join(format!("{name}{suffix}"))).map_err(|e| e.to_string())
        };
        let sky: Sky = serde_json::from_slice(&read(".json")?).map_err(|e| e.to_string())?;
        let rgba = read(".rgba")?;
        if sky.width == 0
            || sky.height == 0
            || u64::from(sky.width) * u64::from(sky.height) * 4 != rgba.len() as u64
            || sky.positions.is_empty()
            || sky.positions.len() != sky.uvs.len()
            || sky.indices.is_empty()
            || sky.indices.len() % 3 != 0
            || sky
                .indices
                .iter()
                .any(|&i| i as usize >= sky.positions.len())
            || sky
                .positions
                .iter()
                .flatten()
                .chain(sky.uvs.iter().flatten())
                .any(|v| !v.is_finite())
        {
            return Err("Invalid retail sky dimensions/geometry".into());
        }
        let sun = if let Some(env) = &sky.environment {
            let direction = Vec3::from_array(env.sun_direction);
            if !env.anchor_height.is_finite()
                || !direction.is_finite()
                || !(0.99..=1.01).contains(&direction.length_squared())
                || !env.sun_scale.is_finite()
                || env.sun_scale <= 0.
                || !env.multiplier.is_finite()
                || env.multiplier < 0.
                || sky.sun_width == 0
                || sky.sun_height != 16
            {
                return Err("Invalid authored sky environment".into());
            }
            if let Some(fog) = &env.fog_frame {
                if !fog.ramp.iter().chain(fog.colour.iter()).all(|v| v.is_finite())
                    || fog.ramp[0] < 0.
                    || fog.ramp[2] <= 0.
                    || !(-1. ..=0.).contains(&fog.colour[3])
                {
                    return Err("Invalid authored fog frame".into());
                }
            }
            let bytes = read(".sun.rgba")?;
            if u64::from(sky.sun_width) * u64::from(sky.sun_height) * 4 != bytes.len() as u64 {
                return Err("Invalid sun-gradient payload".into());
            }
            Some(bytes)
        } else {
            None
        };
        Ok(Self { sky, rgba, sun })
    }

    /// The world-facing half of the package. Legacy packages carry no
    /// environment block, which leaves the world on the defaults.
    pub(crate) fn environment(&self) -> SkyEnvironment {
        let Some(env) = &self.sky.environment else {
            return SkyEnvironment::default();
        };
        let mut environment = SkyEnvironment {
            // Only the tangent-sign terms in the world shader read this vector;
            // it adds no directional light energy and no shadow source.
            sun_direction: Vec3::from_array(env.sun_direction).extend(0.),
            ..default()
        };
        if let Some(fog) = &env.fog_frame {
            environment.fog_ramp = Vec4::from_array(fog.ramp);
            environment.fog_color = Vec4::from_array(fog.colour);
        }
        environment
    }

    pub(crate) fn spawn(
        self,
        commands: &mut SceneCommands,
        meshes: &mut impl AssetSink<Mesh>,
        images: &mut impl AssetSink<Image>,
        materials: &mut impl AssetSink<SkyMaterial>,
        imported: bool,
    ) {
        // Legacy sky packages retain their original parameters. The scene
        // exposure is the baseline the tone pass divides back out.
        let mut params = SkyParams {
            settings: Vec4::new(165., 2.5, 1., 0.),
            sun_direction: Vec4::ZERO,
        };
        if let Some(env) = &self.sky.environment {
            params.settings = Vec4::new(env.anchor_height, 2.5, env.multiplier, env.sun_scale);
            params.sun_direction = Vec3::from_array(env.sun_direction).extend(0.);
            info!(
                "SKATE_SKY_READY location={:?} anchor={} sun_scale={} multiplier={} fog={} authored_direction={:?}",
                env.location_chain,
                env.anchor_height,
                env.sun_scale,
                env.multiplier,
                env.fog_frame.is_some(),
                env.sun_direction
            );
        } else {
            warn!(
                "Retail sky has legacy metadata; reconvert skies for authored parameters, sun and fog"
            );
        }
        let sun = self
            .sun
            .map(|bytes| image(bytes, self.sky.sun_width, self.sky.sun_height, false))
            .map(|sun| images.add(sun));
        let panorama = images.add(image(self.rgba, self.sky.width, self.sky.height, true));
        let mesh = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.sky.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.sky.uvs)
        .with_inserted_indices(bevy::mesh::Indices::U32(self.sky.indices));
        let multiplier = params.settings.z;
        let bundle = (
            Name::new("Retail sky dome"),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(SkyMaterial {
                params,
                panorama,
                sun,
            })),
            Transform::default(),
            // The dome follows the camera in the vertex shader, so its authored
            // bounds say nothing about where it will be drawn.
            bevy::camera::visibility::NoFrustumCulling,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        );
        if imported {
            commands.spawn_marked(bundle, ImportedSky { multiplier });
        } else {
            commands.spawn(bundle);
        }
    }
}

fn image(rgba: Vec<u8>, width: u32, height: u32, repeat: bool) -> Image {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        // Squared in the shader, matching the world path's approximate decode.
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    let mut sampler = bevy::image::ImageSamplerDescriptor::linear();
    if repeat {
        // The panorama wraps in azimuth only.
        sampler.address_mode_u = bevy::image::ImageAddressMode::Repeat;
    }
    image.sampler = bevy::image::ImageSampler::Descriptor(sampler);
    image
}

struct SkyJob {
    rx: std::sync::Mutex<std::sync::mpsc::Receiver<Result<String, String>>>,
}

/// Background import of a panorama onto the existing sky dome.
#[derive(Resource, Default)]
pub(crate) struct SkyImport {
    job: Option<SkyJob>,
}

impl SkyImport {
    pub(crate) fn busy(&self) -> bool { self.job.is_some() }

    pub(crate) fn begin(&mut self, asset_root: std::path::PathBuf) {
        if self.job.is_some() { return; }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let Some(path) = rfd::FileDialog::new()
                .set_title("Import a sky image")
                .add_filter("Sky image", &["png", "jpg", "jpeg", "rgba"])
                .pick_file()
            else {
                let _ = tx.send(Err("cancelled".into()));
                return;
            };
            let _ = tx.send(import_sky(&path, &asset_root));
        });
        self.job = Some(SkyJob { rx: std::sync::Mutex::new(rx) });
    }
}

pub(crate) fn poll_import(
    mut jobs: ResMut<SkyImport>,
    mut menu: Option<ResMut<crate::graphics_menu::Menu>>,
) {
    let Some(job) = jobs.job.as_ref() else { return };
    let result = match job.rx.lock().expect("sky import").try_recv() {
        Ok(result) => result,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("Sky import stopped before it finished".into()),
    };
    jobs.job = None;
    let Some(menu) = menu.as_mut() else { return };
    match result {
        Ok(name) => {
            let saved = menu.set_sky_preset(name.clone());
            menu.set_status(match saved {
                Ok(()) => format!("Sky {name} is selected. Imported maps use it immediately."),
                Err(error) => format!("Imported {name}, but the choice was not saved: {error}"),
            });
        }
        Err(error) if error == "cancelled" => menu.set_status("Sky import cancelled".into()),
        Err(error) => menu.set_status(format!("Sky import failed: {error}")),
    }
}

fn import_sky(source: &Path, asset_root: &Path) -> Result<String, String> {
    let (width, height, rgba) = read_sky_image(source)?;
    let (width, height, rgba) = fit_sky(width, height, rgba);
    let name = sky_name(source.file_stem().and_then(|stem| stem.to_str()).unwrap_or("Sky"))?;
    if list_packages(&asset_root.join("private/native-skies")).iter().any(|existing| existing.eq_ignore_ascii_case(&name)) {
        return Err(format!("{name} is a retail sky. Rename the file and try again."));
    }
    let (dir, donor) = donor_sky(asset_root)?;
    let mut value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.join(format!("{donor}.json"))).map_err(|error| error.to_string())?,
    ).map_err(|error| error.to_string())?;
    let object = value.as_object_mut().ok_or("Retail sky JSON is not an object")?;
    object.insert("width".into(), serde_json::json!(width));
    object.insert("height".into(), serde_json::json!(height));
    object.remove("source_sha256");
    let sun = dir.join(format!("{donor}.sun.rgba"));
    if !sun.is_file() {
        object.remove("environment");
        object.remove("sun_width");
        object.remove("sun_height");
    }
    let dest = user_sky_dir(asset_root);
    std::fs::create_dir_all(&dest).map_err(|error| error.to_string())?;
    std::fs::write(dest.join(format!("{name}.rgba")), &rgba).map_err(|error| error.to_string())?;
    std::fs::write(dest.join(format!("{name}.json")), serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    if sun.is_file() {
        std::fs::copy(&sun, dest.join(format!("{name}.sun.rgba"))).map_err(|error| error.to_string())?;
    }
    Ok(name)
}

fn donor_sky(asset_root: &Path) -> Result<(std::path::PathBuf, String), String> {
    let dir = asset_root.join("private/native-skies");
    let ready = |name: &str| dir.join(format!("{name}.json")).is_file() && dir.join(format!("{name}.sun.rgba")).is_file();
    if ready("University") {
        return Ok((dir, "University".into()));
    }
    let mut names = list_packages(&dir);
    names.sort();
    for name in names {
        if ready(&name) {
            return Ok((dir, name));
        }
    }
    Err("A retail sky is needed as the dome for a custom panorama".into())
}

fn read_sky_image(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase();
    if extension == "rgba" {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        if bytes.len() < 4 || bytes.len() % 4 != 0 {
            return Err(format!("{} is not raw RGBA8", path.display()));
        }
        let (width, height) = rgba_sidecar(path, bytes.len() / 4)?;
        return Ok((width, height, bytes));
    }
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg") {
        return Err("Choose a PNG, JPEG, or raw RGBA file".into());
    }
    let image = image::open(path).map_err(|error| error.to_string())?.into_rgba8();
    Ok((image.width(), image.height(), image.into_raw()))
}

fn rgba_sidecar(path: &Path, pixels: usize) -> Result<(u32, u32), String> {
    let json_path = path.with_extension("json");
    let text = std::fs::read_to_string(&json_path).map_err(|_| {
        format!("{} needs a JSON sidecar with width and height", path.display())
    })?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let width = value.get("width").and_then(|value| value.as_u64()).ok_or("The sidecar has no width")? as u32;
    let height = value.get("height").and_then(|value| value.as_u64()).ok_or("The sidecar has no height")? as u32;
    if width == 0 || height == 0 || width as usize * height as usize != pixels {
        return Err(format!("{width}×{height} does not match {pixels} pixels"));
    }
    Ok((width, height))
}

fn sky_name(stem: &str) -> Result<String, String> {
    let name: String = stem.chars().map(|ch| {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }
    }).collect();
    let name = name.trim_matches('_').to_string();
    if name.is_empty() || name.len() > 64 || name.contains(['/', '\\', ':', '.']) {
        return Err("The file name cannot be used as a sky name".into());
    }
    Ok(name)
}

const SKY_EDGE: u32 = 4096;

fn fit_sky(width: u32, height: u32, rgba: Vec<u8>) -> (u32, u32, Vec<u8>) {
    if width.max(height) <= SKY_EDGE {
        return (width, height, rgba);
    }
    let scale = SKY_EDGE as f32 / width.max(height) as f32;
    let (new_w, new_h) = ((width as f32 * scale).round().max(1.) as u32, (height as f32 * scale).round().max(1.) as u32);
    let Some(image) = image::RgbaImage::from_raw(width, height, rgba) else {
        return (width, height, Vec::new());
    };
    let resized = image::imageops::resize(&image, new_w, new_h, image::imageops::FilterType::Triangle);
    (new_w, new_h, resized.into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(fog: Option<[[f32; 4]; 2]>) -> SkyPackage {
        SkyPackage {
            sky: Sky {
                width: 1,
                height: 1,
                positions: vec![[0.; 3]],
                uvs: vec![[0.; 2]],
                indices: vec![0, 0, 0],
                sun_width: 8,
                sun_height: 16,
                environment: Some(Environment {
                    anchor_height: 165.,
                    sun_direction: [0., 1., 0.],
                    sun_scale: 0.5,
                    multiplier: 1.,
                    location_chain: vec![],
                    fog_frame: fog.map(|[ramp, colour]| FogFrame { ramp, colour }),
                }),
            },
            rgba: vec![0; 4],
            sun: None,
        }
    }

    /// A map with no authored fog must evaluate to *no* fog rather than to a
    /// black ramp: `fog_ramp.z` is an exponent the world shader raises the ramp
    /// to, and zero there would collapse every distance to full fog.
    #[test]
    fn the_default_environment_disables_fog() {
        let default = SkyEnvironment::default();
        assert_eq!(default.fog_ramp, Vec4::new(0., 0., 1., 0.));
        assert_eq!(default.fog_color, Vec4::ZERO);
        assert!((default.sun_direction.xyz().length() - 1.).abs() < 1e-5);
        assert_eq!(default.sun_direction.w, 0.);
        assert_eq!(package(None).environment().fog_ramp, default.fog_ramp);
    }

    #[test]
    fn an_authored_fog_frame_reaches_the_world() {
        let environment = package(Some([[0.001, -0.2, 1.5, 0.], [0.3, 0.4, 0.5, -0.8]]))
            .environment();
        assert_eq!(environment.fog_ramp, Vec4::new(0.001, -0.2, 1.5, 0.));
        assert_eq!(environment.fog_color, Vec4::new(0.3, 0.4, 0.5, -0.8));
        assert_eq!(environment.sun_direction, Vec4::new(0., 1., 0., 0.));
    }

    #[test]
    fn a_user_sky_joins_the_list_without_replacing_a_retail_name() {
        let root = std::env::temp_dir().join(format!("skate-skies-{}", std::process::id()));
        let assets = root.join("assets");
        let native = assets.join("private/native-skies");
        let user = user_sky_dir(&assets);
        std::fs::create_dir_all(&native).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        for name in ["University", "Custom"] {
            let dir = if name == "University" { &native } else { &user };
            std::fs::write(dir.join(format!("{name}.json")), b"{}").unwrap();
            std::fs::write(dir.join(format!("{name}.rgba")), [0u8; 4]).unwrap();
        }
        std::fs::write(user.join("University.json"), b"{}").unwrap();
        std::fs::write(user.join("University.rgba"), [0u8; 4]).unwrap();
        let names = available(&assets);
        assert!(names.contains(&"Custom".to_string()), "{names:?}");
        assert_eq!(names.iter().filter(|name| *name == "University").count(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rgba_sidecar_must_match_the_byte_length() {
        let dir = std::env::temp_dir().join(format!("skate-rgba-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Pic.rgba");
        std::fs::write(&path, vec![0u8; 8]).unwrap();
        std::fs::write(dir.join("Pic.json"), br#"{"width":2,"height":2}"#).unwrap();
        let error = rgba_sidecar(&path, 2).unwrap_err();
        assert!(error.contains("does not match"), "{error}");
        std::fs::write(dir.join("Pic.json"), br#"{"width":2,"height":1}"#).unwrap();
        assert_eq!(rgba_sidecar(&path, 2).unwrap(), (2, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
