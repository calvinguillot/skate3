//! Off-thread scene preparation and transactional publish.
//!
//! Assets are built on a worker with *reserved* handles, then inserted on the
//! main thread in one pass. Nothing touches live ECS until commit, so a failed
//! load cannot leave a half-replaced world (RFC 1 §10.2).
//!
//! Retirement is the leak boundary: `MapAssets` owns every asset the scene
//! created, and dropping a map releases exactly those. Unlike the previous
//! design this does not hardcode the list of asset types — each staged group
//! carries its own release behaviour, so adding a renderer asset type needs no
//! change here.
use bevy::{
    asset::{Asset, AssetId, Assets, AssetHandleProvider},
    ecs::world::CommandQueue,
    prelude::*,
};
use skate_data::skate_map::SkateMap;
use std::path::Path;

/// Marks everything the current map owns. Retirement despawns exactly this set,
/// which is why the character and other persistent entities survive a map change.
#[derive(Component)]
pub(crate) struct MapEntity;

/// Somewhere to put an asset that may not be the live `Assets<A>` collection.
pub(crate) trait AssetSink<A: Asset> {
    fn add(&mut self, asset: A) -> Handle<A>;
}

/// Erased release behaviour for one staged asset type.
trait OwnedAssets: Send + Sync {
    fn release(&mut self, world: &mut World);
}

/// Assets built off-thread against reserved handles.
///
/// `reserve_handle` hands out a real, strong handle without needing `&mut
/// Assets<A>`, which is what makes worker-thread construction possible at all.
pub(crate) struct StagedAssets<A: Asset> {
    provider: AssetHandleProvider,
    pending: Vec<(Handle<A>, A)>,
    owned: Vec<AssetId<A>>,
}

impl<A: Asset> StagedAssets<A> {
    fn new(world: &World) -> Self {
        Self {
            provider: world.resource::<Assets<A>>().get_handle_provider(),
            pending: Vec::new(),
            owned: Vec::new(),
        }
    }

    fn publish(&mut self, world: &mut World) {
        let mut assets = world.resource_mut::<Assets<A>>();
        for (handle, asset) in self.pending.drain(..) {
            self.owned.push(handle.id());
            // The handle came from this same provider moments ago, so a stale
            // generation would mean the reservation logic is broken, not that a
            // map failed to load. Say so rather than dropping geometry silently.
            if let Err(error) = assets.insert(handle.id(), asset) {
                error!(
                    "staged {} asset rejected on publish: {error}",
                    std::any::type_name::<A>()
                );
            }
        }
    }
}

impl<A: Asset> AssetSink<A> for StagedAssets<A> {
    fn add(&mut self, asset: A) -> Handle<A> {
        let handle = self.provider.reserve_handle().typed::<A>();
        self.pending.push((handle.clone(), asset));
        handle
    }
}

impl<A: Asset> OwnedAssets for StagedAssets<A> {
    fn release(&mut self, world: &mut World) {
        let mut assets = world.resource_mut::<Assets<A>>();
        for id in self.owned.drain(..) {
            assets.remove(id);
        }
        self.pending.clear();
    }
}

/// Deferred spawns. Every entity created through this gets `MapEntity`, so
/// retirement cannot miss one by omission.
#[derive(Default)]
pub(crate) struct SceneCommands {
    queue: CommandQueue,
}

impl SceneCommands {
    pub(crate) fn spawn<B: Bundle>(&mut self, bundle: B) {
        self.queue.push(move |world: &mut World| {
            world.spawn((bundle, MapEntity));
        });
    }

    pub(crate) fn spawn_marked<B: Bundle, M: Component>(&mut self, bundle: B, marker: M) {
        self.queue.push(move |world: &mut World| {
            world.spawn((bundle, MapEntity, marker));
        });
    }

    pub(crate) fn apply(mut self, world: &mut World) {
        self.queue.apply(world);
    }
}

struct LiveAssets<'a, A: Asset>(&'a mut Assets<A>);
impl<A: Asset> AssetSink<A> for LiveAssets<'_, A> {
    fn add(&mut self, asset: A) -> Handle<A> {
        self.0.add(asset)
    }
}

/// Assets owned by the live scene, released on the next map change.
#[derive(Resource, Default)]
pub(crate) struct MapAssets {
    groups: Vec<Box<dyn OwnedAssets>>,
}

impl MapAssets {
    /// Despawn the current map and release its assets. Safe to call with no map
    /// loaded, which is the case on the very first transition.
    pub(crate) fn retire(world: &mut World) {
        let entities: Vec<Entity> = world
            .query_filtered::<Entity, With<MapEntity>>()
            .iter(world)
            .collect();
        for entity in entities {
            world.despawn(entity);
        }
        if let Some(mut assets) = world.remove_resource::<MapAssets>() {
            for group in &mut assets.groups {
                group.release(world);
            }
        }
    }
}

/// A whole scene, built without touching the live world.
pub(crate) struct PreparedScene {
    meshes: StagedAssets<Mesh>,
    images: StagedAssets<Image>,
    standard: StagedAssets<StandardMaterial>,
    world_materials: StagedAssets<crate::retail_render::WorldMaterial>,
    sky: StagedAssets<crate::retail_sky::SkyMaterial>,
    params: StagedAssets<bevy::render::storage::ShaderStorageBuffer>,
    commands: SceneCommands,
    pub stats: crate::skate_world::SceneStats,
    pub retail: bool,
}

impl PreparedScene {
    pub(crate) fn new(world: &World) -> Self {
        Self {
            meshes: StagedAssets::new(world),
            images: StagedAssets::new(world),
            standard: StagedAssets::new(world),
            world_materials: StagedAssets::new(world),
            sky: StagedAssets::new(world),
            params: StagedAssets::new(world),
            commands: SceneCommands::default(),
            stats: default(),
            retail: false,
        }
    }

    /// Worker-thread body. Builds all geometry, materials and images for the map.
    pub(crate) fn prepare(&mut self, map: Option<&SkateMap>, asset_root: &Path) {
        let Some(map) = map else {
            crate::world::spawn_test_world(&mut self.commands, &mut self.meshes, &mut self.standard);
            return;
        };
        self.retail = crate::retail_render::RetailScene::for_map(map);
        let tuning = crate::retail_render::MaterialTuning::load(asset_root);
        // The sky is loaded before the world because its authored fog frame and
        // sun direction are encoded into the per-material storage buffer while
        // the material table is built, not patched in afterwards.
        let sky = self.retail.then(|| {
            crate::retail_sky::SkyPackage::load(asset_root, &map.name)
                .inspect_err(|error| warn!("Retail sky unavailable for {}: {error}", map.name))
                .ok()
        }).flatten();
        let environment = sky
            .as_ref()
            .map(crate::retail_sky::SkyPackage::environment)
            .unwrap_or_default();
        self.stats = crate::skate_world::spawn(
            map,
            &tuning,
            &environment,
            &mut self.commands,
            &mut self.meshes,
            &mut self.world_materials,
            &mut self.images,
            &mut self.params,
        );
        if let Some(sky) = sky {
            sky.spawn(
                &mut self.commands,
                &mut self.meshes,
                &mut self.images,
                &mut self.sky,
                false,
            );
        }
    }

    /// Main-thread commit. Assets first, then entities: a spawned entity must
    /// never reference a handle whose asset has not been inserted yet.
    pub(crate) fn publish(&mut self, world: &mut World) {
        self.meshes.publish(world);
        self.images.publish(world);
        self.standard.publish(world);
        self.world_materials.publish(world);
        self.sky.publish(world);
        self.params.publish(world);
        std::mem::take(&mut self.commands.queue).apply(world);
        world.insert_resource(MapAssets {
            groups: vec![
                Box::new(std::mem::replace(&mut self.meshes, StagedAssets::new(world))),
                Box::new(std::mem::replace(&mut self.images, StagedAssets::new(world))),
                Box::new(std::mem::replace(&mut self.standard, StagedAssets::new(world))),
                Box::new(std::mem::replace(&mut self.world_materials, StagedAssets::new(world))),
                Box::new(std::mem::replace(&mut self.sky, StagedAssets::new(world))),
                Box::new(std::mem::replace(&mut self.params, StagedAssets::new(world))),
            ],
        });
    }
}

// ---------------------------------------------------------------------------
// Day/night for non-retail maps
//
// Retail maps are lightmap-baked and keep their authored lighting, so these only
// drive the procedural test/custom world.
// ---------------------------------------------------------------------------

#[derive(Component)]
pub(crate) struct DayEnvironment {
    pub hour: f32,
    pub speed: f32,
}

/// Sun or moon disc, positioned opposite each other on the same arc.
#[derive(Component)]
pub(crate) struct CelestialBody {
    pub moon: bool,
}

pub(crate) fn advance_day(
    time: Res<Time<Virtual>>,
    mut lights: Query<(&mut DayEnvironment, &mut DirectionalLight, &mut Transform)>,
) {
    for (mut environment, mut light, mut transform) in &mut lights {
        environment.hour =
            (environment.hour + time.delta_secs() * environment.speed / 3600.0).rem_euclid(24.0);
        let angle = (environment.hour / 24.0) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let direction = Vec3::new(angle.cos(), angle.sin(), 0.25).normalize();
        *transform = Transform::from_translation(direction * 100.0)
            .looking_at(Vec3::ZERO, Vec3::Y);
        // Fade out below the horizon rather than snapping to black.
        light.illuminance = 11_000.0 * direction.y.max(0.0).powf(0.4);
    }
}

pub(crate) fn position_celestial_bodies(
    environments: Query<&DayEnvironment>,
    mut bodies: Query<(&CelestialBody, &mut Transform, &mut Visibility)>,
) {
    let Ok(environment) = environments.single() else { return };
    let angle = (environment.hour / 24.0) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
    for (body, mut transform, mut visibility) in &mut bodies {
        let phase = if body.moon { angle + std::f32::consts::PI } else { angle };
        let direction = Vec3::new(phase.cos(), phase.sin(), 0.25).normalize();
        transform.translation = direction * 400.0;
        *visibility = if direction.y > -0.05 { Visibility::Inherited } else { Visibility::Hidden };
    }
}

/// Sun direction for a 24-hour clock. Noon is mostly upward; midnight points down.
pub(crate) fn sun_direction(hour: f32) -> Vec3 {
    let angle = (hour / 24.0) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
    Vec3::new(angle.cos(), angle.sin(), 0.25).normalize()
}

/// Directional light spawned for maps that are not lightmap-baked.
#[derive(Component)]
pub(crate) struct ImportedSun;

/// Which preset the live imported dome was built from. `Off` and empty are distinct:
/// empty means "retail map, no imported dome".
#[derive(Resource)]
struct ActiveImportedSky {
    name: String,
    /// The package failed to load. Don't try again until the preset changes.
    missing: bool,
}

/// Portable materials read these channels from the frame the retail shader already uploads.
/// Retail maps leave the buffer alone so baked lightmaps and ocean PCA stay intact.
pub(crate) fn apply_imported_light(
    retail: Res<crate::retail_render::RetailScene>,
    mut state: ResMut<crate::retail_render::FrameStateData>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut menu: Option<ResMut<crate::graphics_menu::Menu>>,
    time: Res<Time<Virtual>>,
    mut commands: Commands,
    mut suns: Query<(Entity, &mut DirectionalLight, &mut Transform), With<ImportedSun>>,
    skies: Query<(&crate::retail_sky::ImportedSky, &MeshMaterial3d<crate::retail_sky::SkyMaterial>)>,
    mut materials: ResMut<Assets<crate::retail_sky::SkyMaterial>>,
    mut restored: Local<bool>,
) {
    if retail.0 {
        for (entity, _, _) in suns.iter() {
            commands.entity(entity).despawn();
        }
        if *restored {
            ambient.brightness = 350.;
            ambient.color = Color::WHITE;
            *restored = false;
        }
        return;
    }
    let (hour, brightness, fill) = if let Some(menu) = menu.as_deref_mut() {
        let hour = menu.advance_day(time.delta_secs());
        (hour, menu.world_brightness(), menu.ambient_fill(sun_direction(hour).y))
    } else {
        (12., 1., 0.45)
    };
    let sun = sun_direction(hour);
    state.shadow = Vec4::new(sun.x, sun.y, sun.z, 0.);
    state.clock.w = brightness;
    state.pca[0].w = fill;
    ambient.color = Color::WHITE;
    ambient.brightness = (80. + 420. * fill) * brightness;
    *restored = true;
    if suns.is_empty() {
        commands.spawn((
            ImportedSun,
            DirectionalLight {
                illuminance: 11_000.,
                shadows_enabled: true,
                ..default()
            },
            Transform::default(),
        ));
    }
    let up = if sun.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
    let aim = Transform::default().looking_to(-sun, up);
    let illuminance = 11_000. * brightness * sun.y.max(0.).powf(0.4);
    for (_, mut light, mut transform) in &mut suns {
        light.illuminance = illuminance;
        light.shadows_enabled = true;
        *transform = aim;
    }
    let sun4 = sun.extend(0.);
    for (sky, handle) in &skies {
        let Some(material) = materials.get_mut(&handle.0) else { continue };
        let multiplier = sky.multiplier * brightness;
        if (material.params.settings.z - multiplier).abs() > 0.001
            || material.params.sun_direction.distance(sun4) > 0.001
        {
            material.params.settings.z = multiplier;
            material.params.sun_direction = sun4;
        }
    }
}

/// Swap the imported sky dome when the preset changes. Retail skies are spawned
/// with the map and are not touched here. Runs after map retirement, so a dome
/// spawned this frame is not deleted by the same transition.
pub(crate) fn sync_imported_sky(world: &mut World) {
    let retail = world.resource::<crate::retail_render::RetailScene>().0;
    let want = world
        .get_resource::<crate::graphics_menu::Menu>()
        .map(|menu| menu.sky_preset().to_string())
        .unwrap_or_else(|| "University".into());
    let current = world.get_resource::<ActiveImportedSky>().map(|sky| (sky.name.clone(), sky.missing));
    let mut query = world.query::<(Entity, &crate::retail_sky::ImportedSky)>();
    let present: Vec<Entity> = query.iter(world).map(|(entity, _)| entity).collect();
    if retail {
        if present.is_empty() && current.as_ref().is_some_and(|(name, _)| name.is_empty()) {
            return;
        }
        for entity in present {
            world.despawn(entity);
        }
        world.insert_resource(ActiveImportedSky { name: String::new(), missing: false });
        return;
    }
    let settled = current.as_ref().is_some_and(|(name, missing)| {
        name == &want && (*missing || want == "Off" || !present.is_empty())
    });
    if settled {
        return;
    }
    for entity in present {
        world.despawn(entity);
    }
    let mut missing = false;
    if want != "Off" {
        let root = world.resource::<crate::config::Config>().asset_root.clone();
        match crate::retail_sky::SkyPackage::load(&root, &want) {
            Ok(package) => spawn_imported_sky(world, package),
            Err(error) => {
                warn!("Sky preset {want} unavailable: {error}");
                missing = true;
            }
        }
    }
    world.insert_resource(ActiveImportedSky { name: want, missing });
}

fn spawn_imported_sky(world: &mut World, package: crate::retail_sky::SkyPackage) {
    let Some(mut meshes) = world.remove_resource::<Assets<Mesh>>() else { return };
    let Some(mut images) = world.remove_resource::<Assets<Image>>() else {
        world.insert_resource(meshes);
        return;
    };
    let Some(mut materials) = world.remove_resource::<Assets<crate::retail_sky::SkyMaterial>>() else {
        world.insert_resource(meshes);
        world.insert_resource(images);
        return;
    };
    let mut commands = SceneCommands::default();
    package.spawn(
        &mut commands,
        &mut LiveAssets(&mut meshes),
        &mut LiveAssets(&mut images),
        &mut LiveAssets(&mut materials),
        true,
    );
    world.insert_resource(meshes);
    world.insert_resource(images);
    world.insert_resource(materials);
    commands.apply(world);
}
