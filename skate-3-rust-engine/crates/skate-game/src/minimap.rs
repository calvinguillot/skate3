//! Top-down map overview, the corner HUD, and the pause-menu click-to-spawn view.
//!
//! The picture is painted once per loaded world from collision triangles. The
//! corner widget scrolls that picture around the skater. The pause-menu view
//! shows the whole map and raycasts a clicked (X, Z) down onto the ground.
use crate::graphics_menu::Menu;
use crate::map_transition::CurrentMap;
use crate::physics::{GamePhysics, SkaterRuntime};
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use skate_core::math::Vector3;
use skate_core::physics::board_world::{BoardWorld, WorldTriangle};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SIZE: usize = 512;
const FRAME: f32 = 180.;
const PICKER: f32 = 560.;
const VIEW_METRES: f32 = 120.;

#[derive(Resource, Default)]
pub(crate) struct MapOverview {
    image: Handle<Image>,
    min_x: f32,
    max_x: f32,
    min_z: f32,
    max_z: f32,
    min_y: f32,
    max_y: f32,
    ready: bool,
}

struct PlotTri {
    p: [[f32; 3]; 3],
    ny: f32,
}

struct Raster {
    rgba: Vec<u8>,
    min_x: f32,
    max_x: f32,
    min_z: f32,
    max_z: f32,
    min_y: f32,
    max_y: f32,
}

pub(crate) struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapOverview>()
            .add_systems(PostStartup, setup)
            .add_systems(Update, (rebuild, update_hud, update_picker).chain());
    }
}

#[derive(Component)]
struct MinimapRoot;
#[derive(Component)]
struct MinimapImage;
#[derive(Component)]
struct MinimapReadout;
#[derive(Component)]
struct MinimapPin(u8);
#[derive(Component)]
struct PickerRoot;
#[derive(Component)]
struct PickerImage;
#[derive(Component)]
struct PickerReadout;
#[derive(Component)]
struct PickerPin(u8);
#[derive(Component)]
struct PickerButton(u8);

fn setup(mut commands: Commands) {
    let frame = Node {
        width: px(FRAME),
        height: px(FRAME),
        overflow: Overflow::clip(),
        ..default()
    };
    commands.spawn((
        MinimapRoot,
        GlobalZIndex(4),
        bevy::ui::FocusPolicy::Pass,
        Node {
            position_type: PositionType::Absolute,
            top: px(16.),
            right: px(16.),
            width: px(FRAME),
            flex_direction: FlexDirection::Column,
            row_gap: px(4.),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::srgb(0.04, 0.05, 0.06)),
    )).with_children(|root| {
        root.spawn(frame).with_children(|frame| {
            frame.spawn((
                MinimapImage,
                ImageNode::default(),
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
            ));
            spawn_pin(frame, MinimapPin(0), Color::srgb(0.95, 0.85, 0.2), true);
            spawn_pin(frame, MinimapPin(1), Color::srgb(0.35, 0.85, 0.95), false);
            spawn_pin(frame, MinimapPin(2), Color::srgb(0.95, 0.45, 0.2), false);
            frame.spawn((
                Text::new("N"),
                TextFont { font_size: 12., ..default() },
                TextColor(Color::srgb(0.9, 0.95, 0.9)),
                Node { position_type: PositionType::Absolute, top: px(2.), left: px(FRAME / 2. - 4.), ..default() },
            ));
        });
        root.spawn((
            MinimapReadout,
            Text::new(""),
            TextFont { font_size: 12., ..default() },
            TextColor(Color::srgb(0.85, 0.9, 0.85)),
            Node { margin: UiRect::axes(px(6.), px(4.)), ..default() },
        ));
    });

    commands.spawn((
        PickerRoot,
        GlobalZIndex(21),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.),
            height: Val::Percent(100.),
            display: Display::None,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(Color::srgba(0.015, 0.02, 0.025, 0.94)),
    )).with_children(|root| {
        root.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(10.),
            align_items: AlignItems::Center,
            width: Val::Percent(100.),
            ..default()
        }).with_children(|column| {
            column.spawn((
                Text::new("PLACE ON MAP"),
                TextFont { font_size: 28., ..default() },
                TextColor(Color::WHITE),
            ));
            column.spawn((
                Text::new("Left click teleports and resumes. Right click also saves that spot as the map's default spawn."),
                TextFont { font_size: 14., ..default() },
                TextColor(Color::srgb(0.7, 0.76, 0.74)),
                Node { max_width: px(PICKER), ..default() },
            ));
            column.spawn((
                Node {
                    width: px(PICKER),
                    height: px(PICKER),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.05, 0.06, 0.07)),
            )).with_children(|frame| {
                frame.spawn((
                    PickerImage,
                    ImageNode::default(),
                    RelativeCursorPosition::default(),
                    Node { width: px(PICKER), height: px(PICKER), ..default() },
                ));
                spawn_pin(frame, PickerPin(0), Color::srgb(0.95, 0.85, 0.2), true);
                spawn_pin(frame, PickerPin(1), Color::srgb(0.35, 0.85, 0.95), false);
            });
            column.spawn((
                PickerReadout,
                Text::new("Move the cursor over the map."),
                TextFont { font_size: 16., ..default() },
                TextColor(Color::srgb(0.85, 0.95, 0.7)),
            ));
            column.spawn(Node { flex_direction: FlexDirection::Row, column_gap: px(8.), ..default() }).with_children(|row| {
                picker_button(row, 1, "Save as map default spawn");
                picker_button(row, 0, "Back");
            });
        });
    });
}

fn spawn_pin(parent: &mut bevy::ecs::hierarchy::ChildSpawnerCommands, pin: impl Component, color: Color, nose: bool) {
    parent.spawn((
        pin,
        Node {
            position_type: PositionType::Absolute,
            width: px(10.),
            height: px(10.),
            ..default()
        },
        BackgroundColor(color),
        UiTransform::default(),
    )).with_children(|pin| {
        if nose {
            pin.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: px(4.),
                    height: px(14.),
                    left: px(3.),
                    top: px(-14.),
                    ..default()
                },
                BackgroundColor(color),
            ));
        }
    });
}

fn picker_button(row: &mut bevy::ecs::hierarchy::ChildSpawnerCommands, id: u8, label: &str) {
    row.spawn((
        Button,
        PickerButton(id),
        Node {
            padding: UiRect::axes(px(14.), px(10.)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.1, 0.14, 0.12)),
    )).with_child((
        Text::new(label),
        TextFont { font_size: 16., ..default() },
        TextColor(Color::WHITE),
    ));
}

fn px(value: f32) -> Val { Val::Px(value) }

fn rebuild(
    mut overview: ResMut<MapOverview>,
    mut images: ResMut<Assets<Image>>,
    physics: Res<GamePhysics>,
    current: Option<Res<CurrentMap>>,
    mut built: Local<Option<u64>>,
) {
    let generation = current.as_ref().map(|map| map.generation).unwrap_or(0);
    if overview.ready && *built == Some(generation) {
        return;
    }
    let triangles = physics.world_triangles();
    if triangles.is_empty() {
        return;
    }
    let raster = rasterize(triangles);
    if overview.image != Handle::default() {
        images.remove(&overview.image);
    }
    overview.image = images.add(overview_image(&raster.rgba));
    overview.min_x = raster.min_x;
    overview.max_x = raster.max_x;
    overview.min_z = raster.min_z;
    overview.max_z = raster.max_z;
    overview.min_y = raster.min_y;
    overview.max_y = raster.max_y;
    overview.ready = true;
    *built = Some(generation);
}

fn update_hud(
    menu: Option<Res<Menu>>,
    overview: Res<MapOverview>,
    skater: Option<Res<SkaterRuntime>>,
    current: Option<Res<CurrentMap>>,
    session: Option<Res<crate::session_marker::SessionMarker>>,
    customiser: Option<Res<crate::customiser::Customiser>>,
    models: Option<Res<crate::custom_models::CustomModels>>,
    travel: Option<Res<crate::teleport_menu::Travel>>,
    mods: Option<Res<crate::modding::ModMenu>>,
    mut root: Query<&mut Node, (With<MinimapRoot>, Without<MinimapImage>, Without<MinimapPin>)>,
    mut images: Query<(&mut Node, &mut ImageNode), (With<MinimapImage>, Without<MinimapRoot>, Without<MinimapPin>)>,
    mut pins: Query<(&MinimapPin, &mut Node, &mut UiTransform), (Without<MinimapImage>, Without<MinimapRoot>)>,
    mut readout: Query<&mut Text, With<MinimapReadout>>,
) {
    let hidden = menu.as_ref().is_none_or(|menu| menu.open || menu.place_on_map() || !menu.minimap_enabled())
        || customiser.as_ref().is_some_and(|ui| ui.open)
        || models.as_ref().is_some_and(|ui| ui.open)
        || travel.as_ref().is_some_and(|ui| ui.open)
        || mods.as_ref().is_some_and(|ui| ui.open)
        || !overview.ready
        || skater.is_none();
    let Ok(mut root) = root.single_mut() else { return };
    root.display = if hidden { Display::None } else { Display::Flex };
    if hidden { return; }
    let menu = menu.unwrap();
    root.top = px(if menu.frame_stats_visible() { 78. } else { 16. });
    let skater = skater.unwrap();
    let pose = skater.animated_skeleton.roots.animation_to_world;
    let (x, y, z) = (pose[3][0], pose[3][1], pose[3][2]);
    let yaw = pose[2][0].atan2(pose[2][2]);
    if let Ok((mut node, mut image)) = images.single_mut() {
        image.image = overview.image.clone();
        let layout = frame_layout(&overview, x, z, FRAME);
        node.width = px(layout.image_w);
        node.height = px(layout.image_h);
        node.left = px(layout.left);
        node.top = px(layout.top);
        place_pins(&overview, &mut pins, layout, x, z, yaw, current.as_deref().map(|map| map.spawn), session.as_ref().and_then(|marker| marker.placed_position()));
    }
    if let Ok(mut text) = readout.single_mut() {
        **text = format!("X {x:.0}   Y {y:.1}   Z {z:.0}");
    }
}

fn update_picker(
    mut menu: Option<ResMut<Menu>>,
    overview: Res<MapOverview>,
    physics: Res<GamePhysics>,
    config: Res<crate::config::Config>,
    mut current: Option<ResMut<CurrentMap>>,
    mut skater: Option<ResMut<SkaterRuntime>>,
    mut time: ResMut<Time<Virtual>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut root: Query<&mut Node, (With<PickerRoot>, Without<PickerPin>)>,
    mut image: Query<(&mut ImageNode, &RelativeCursorPosition), With<PickerImage>>,
    mut pins: Query<(&PickerPin, &mut Node, &mut UiTransform), (Without<PickerImage>, Without<PickerRoot>)>,
    mut readout: Query<&mut Text, With<PickerReadout>>,
    buttons: Query<(&PickerButton, &Interaction)>,
    mut hover: Local<Option<[f32; 3]>>,
) {
    let open = menu.as_ref().is_some_and(|menu| menu.place_on_map()) && overview.ready;
    let Ok(mut root) = root.single_mut() else { return };
    root.display = if open { Display::Flex } else { Display::None };
    if !open {
        *hover = None;
        return;
    }
    let fresh = menu.as_mut().is_some_and(|menu| menu.place_is_fresh());
    let Ok((mut picture, cursor)) = image.single_mut() else { return };
    picture.image = overview.image.clone();
    if cursor.cursor_over() {
        if let Some(point) = cursor.normalized {
            let (x, z) = uv_to_xz(point.x.clamp(0., 1.), point.y.clamp(0., 1.), &overview);
            let y = ground_y(physics.world(), x, z, overview.min_y, overview.max_y);
            *hover = Some([x, y.unwrap_or(overview.min_y.max(0.)), z]);
        }
    }
    if let (Ok(mut text), Some(spot)) = (readout.single_mut(), *hover) {
        let ground = ground_y(physics.world(), spot[0], spot[2], overview.min_y, overview.max_y);
        **text = match ground {
            Some(y) => format!("X {:.1}    Y {:.1}    Z {:.1}", spot[0], y, spot[2]),
            None => format!("X {:.1}    no ground    Z {:.1}", spot[0], spot[2]),
        };
    }
    if let Some(skater) = skater.as_ref() {
        let pose = skater.animated_skeleton.roots.animation_to_world;
        let layout = Layout { image_w: PICKER, image_h: PICKER, left: 0., top: 0. };
        place_picker_pins(&overview, &mut pins, layout, pose[3][0], pose[3][2], pose[2][0].atan2(pose[2][2]), current.as_ref().map(|map| map.spawn));
    }
    if fresh { return; }
    let back = mouse.just_pressed(MouseButton::Left) && buttons.iter().any(|(button, state)| button.0 == 0 && *state == Interaction::Pressed);
    let save_button = mouse.just_pressed(MouseButton::Left) && buttons.iter().any(|(button, state)| button.0 == 1 && *state == Interaction::Pressed);
    if back {
        if let Some(menu) = menu.as_mut() { menu.close_place(false); }
        return;
    }
    let on_map = cursor.cursor_over();
    let left = mouse.just_pressed(MouseButton::Left) && on_map;
    let right = mouse.just_pressed(MouseButton::Right) && on_map;
    let confirm = keys.just_pressed(KeyCode::Enter);
    if !(left || right || save_button || confirm) { return; }
    let Some(spot) = *hover else {
        if let Some(menu) = menu.as_mut() { menu.set_status("Move the cursor over the map first.".into()); }
        return;
    };
    let save = right || save_button;
    if save {
        match current.as_ref().and_then(|map| map.path.clone()) {
            Some(path) => {
                match save_spawn(&config.asset_root, &path, spot) {
                    Ok(message) => if let Some(menu) = menu.as_mut() { menu.set_status(message.into()); },
                    Err(error) => {
                        if let Some(menu) = menu.as_mut() { menu.set_status(error); }
                        return;
                    }
                }
                if let Some(current) = current.as_mut() { current.spawn = spot; }
            }
            None => {
                if let Some(menu) = menu.as_mut() {
                    menu.set_status("This world has no map file, so the spawn was not saved.".into());
                }
                if save_button { return; }
            }
        }
    }
    let Some(skater) = skater.as_mut() else { return };
    let y = ground_y(physics.world(), spot[0], spot[2], overview.min_y, overview.max_y).unwrap_or(spot[1]) + 0.35;
    let matrix = placement(skater, spot[0], y, spot[2]);
    match skater.travel_to(matrix) {
        Ok(()) => {
            if let Some(menu) = menu.as_mut() { menu.close_place(true); }
            time.unpause();
        }
        Err(error) => if let Some(menu) = menu.as_mut() {
            menu.set_status(format!("Could not travel: {error}"));
        },
    }
}

struct Layout {
    image_w: f32,
    image_h: f32,
    left: f32,
    top: f32,
}

fn frame_layout(overview: &MapOverview, x: f32, z: f32, frame: f32) -> Layout {
    let span_x = (overview.max_x - overview.min_x).max(1.);
    let span_z = (overview.max_z - overview.min_z).max(1.);
    let view = VIEW_METRES.min(span_x.max(span_z));
    let pixels = frame / view;
    let image_w = span_x * pixels;
    let image_h = span_z * pixels;
    let (u, v) = uv_of(overview, x, z);
    let mut left = frame * 0.5 - u * image_w;
    let mut top = frame * 0.5 - v * image_h;
    if image_w <= frame { left = (frame - image_w) * 0.5; } else { left = left.clamp(frame - image_w, 0.); }
    if image_h <= frame { top = (frame - image_h) * 0.5; } else { top = top.clamp(frame - image_h, 0.); }
    Layout { image_w, image_h, left, top }
}

fn place_pins(
    overview: &MapOverview,
    pins: &mut Query<(&MinimapPin, &mut Node, &mut UiTransform), (Without<MinimapImage>, Without<MinimapRoot>)>,
    layout: Layout,
    x: f32,
    z: f32,
    yaw: f32,
    spawn: Option<[f32; 3]>,
    session: Option<[f32; 3]>,
) {
    for (pin, mut node, mut transform) in pins.iter_mut() {
        let (spot, rotate) = match pin.0 {
            0 => (Some([x, 0., z]), true),
            1 => (spawn, false),
            _ => (session, false),
        };
        let Some(spot) = spot else {
            node.display = Display::None;
            continue;
        };
        node.display = Display::Flex;
        let (x, y) = pin_px(overview, &layout, spot[0], spot[2], FRAME);
        node.left = Val::Px(x - 5.);
        node.top = Val::Px(y - 5.);
        transform.rotation = if rotate { Rot2::radians(-yaw) } else { Rot2::IDENTITY };
    }
}

fn place_picker_pins(
    overview: &MapOverview,
    pins: &mut Query<(&PickerPin, &mut Node, &mut UiTransform), (Without<PickerImage>, Without<PickerRoot>)>,
    layout: Layout,
    x: f32,
    z: f32,
    yaw: f32,
    spawn: Option<[f32; 3]>,
) {
    for (pin, mut node, mut transform) in pins.iter_mut() {
        let spot = if pin.0 == 0 { Some([x, 0., z]) } else { spawn };
        let Some(spot) = spot else { node.display = Display::None; continue; };
        node.display = Display::Flex;
        let (px, py) = pin_px(overview, &layout, spot[0], spot[2], PICKER);
        node.left = Val::Px(px - 5.);
        node.top = Val::Px(py - 5.);
        transform.rotation = if pin.0 == 0 { Rot2::radians(-yaw) } else { Rot2::IDENTITY };
    }
}

fn pin_px(overview: &MapOverview, layout: &Layout, x: f32, z: f32, frame: f32) -> (f32, f32) {
    let (u, v) = uv_of(overview, x, z);
    let x = layout.left + u * layout.image_w;
    let y = layout.top + v * layout.image_h;
    (x.clamp(6., frame - 6.), y.clamp(6., frame - 6.))
}

fn uv_of(overview: &MapOverview, x: f32, z: f32) -> (f32, f32) {
    let span_x = (overview.max_x - overview.min_x).max(0.01);
    let span_z = (overview.max_z - overview.min_z).max(0.01);
    ((x - overview.min_x) / span_x, (overview.max_z - z) / span_z)
}

fn uv_to_xz(u: f32, v: f32, overview: &MapOverview) -> (f32, f32) {
    let x = overview.min_x + u * (overview.max_x - overview.min_x);
    let z = overview.max_z - v * (overview.max_z - overview.min_z);
    (x, z)
}

pub(crate) fn ground_y(world: &BoardWorld, x: f32, z: f32, min_y: f32, max_y: f32) -> Option<f32> {
    let start = Vector3::new(x, max_y + 50., z);
    let end = Vector3::new(x, min_y - 10., z);
    world.query_thin_line(start, end).ok().flatten().map(|hit| hit.geometry.position.y)
}

fn placement(skater: &SkaterRuntime, x: f32, y: f32, z: f32) -> [[f32; 4]; 4] {
    let pose = skater.animated_skeleton.roots.animation_to_world;
    let mut forward = Vec3::new(pose[2][0], 0., pose[2][2]);
    if forward.length_squared() < 1e-4 { forward = Vec3::Z; }
    forward = forward.normalize();
    let right = Vec3::Y.cross(forward);
    [
        [right.x, right.y, right.z, 0.],
        [0., 1., 0., 0.],
        [forward.x, forward.y, forward.z, 0.],
        [x, y, z, 0.],
    ]
}

fn rasterize(triangles: &[WorldTriangle]) -> Raster {
    let plotted: Vec<PlotTri> = triangles.iter().filter_map(|triangle| {
        let vertices = triangle.triangle.vertices;
        let p = [
            [vertices[0].x, vertices[0].y, vertices[0].z],
            [vertices[1].x, vertices[1].y, vertices[1].z],
            [vertices[2].x, vertices[2].y, vertices[2].z],
        ];
        if p.iter().any(|corner| corner.iter().any(|value| !value.is_finite())) { return None; }
        let normal = triangle.triangle.feature.normal;
        let ny = if normal.x * normal.x + normal.y * normal.y + normal.z * normal.z > 1e-8 { normal.y } else { 1. };
        Some(PlotTri { p, ny })
    }).collect();
    rasterize_plots(&plotted)
}

fn rasterize_plots(triangles: &[PlotTri]) -> Raster {
    let floor_only = !triangles.iter().any(|triangle| triangle.p.iter().any(|corner| corner[1] >= -0.1));
    let kept: Vec<&PlotTri> = triangles.iter().filter(|triangle| {
        floor_only || triangle.p.iter().any(|corner| corner[1] >= -0.1)
    }).collect();
    if kept.is_empty() {
        return empty_raster();
    }
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_z = f32::MAX;
    let mut max_z = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for triangle in &kept {
        for corner in triangle.p {
            min_x = min_x.min(corner[0]);
            max_x = max_x.max(corner[0]);
            min_z = min_z.min(corner[2]);
            max_z = max_z.max(corner[2]);
            min_y = min_y.min(corner[1]);
            max_y = max_y.max(corner[1]);
        }
    }
    let pad_x = (max_x - min_x).max(1.) * 0.02;
    let pad_z = (max_z - min_z).max(1.) * 0.02;
    min_x -= pad_x;
    max_x += pad_x;
    min_z -= pad_z;
    max_z += pad_z;
    let span_x = (max_x - min_x).max(0.01);
    let span_z = (max_z - min_z).max(0.01);
    let mut height = vec![f32::NEG_INFINITY; SIZE * SIZE];
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[18, 24, 28, 255]);
    }
    let project = |x: f32, z: f32| -> [f32; 2] {
        [
            (x - min_x) / span_x * (SIZE as f32 - 1.),
            (max_z - z) / span_z * (SIZE as f32 - 1.),
        ]
    };
    for triangle in &kept {
        let a = project(triangle.p[0][0], triangle.p[0][2]);
        let b = project(triangle.p[1][0], triangle.p[1][2]);
        let c = project(triangle.p[2][0], triangle.p[2][2]);
        let min_i = (a[0].min(b[0]).min(c[0]).floor().max(0.) as usize).min(SIZE - 1);
        let max_i = (a[0].max(b[0]).max(c[0]).ceil().max(0.) as usize).min(SIZE - 1);
        let min_j = (a[1].min(b[1]).min(c[1]).floor().max(0.) as usize).min(SIZE - 1);
        let max_j = (a[1].max(b[1]).max(c[1]).ceil().max(0.) as usize).min(SIZE - 1);
        if min_i > max_i || min_j > max_j { continue; }
        for j in min_j..=max_j {
            for i in min_i..=max_i {
                let Some((wa, wb, wc)) = barycentric(i as f32, j as f32, a, b, c) else { continue };
                let y = wa * triangle.p[0][1] + wb * triangle.p[1][1] + wc * triangle.p[2][1];
                let index = j * SIZE + i;
                if y <= height[index] { continue; }
                height[index] = y;
                let color = shade(y, triangle.ny, min_y, max_y);
                rgba[index * 4..index * 4 + 4].copy_from_slice(&color);
            }
        }
    }
    Raster { rgba, min_x, max_x, min_z, max_z, min_y, max_y }
}

fn empty_raster() -> Raster {
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    for pixel in rgba.chunks_exact_mut(4) { pixel.copy_from_slice(&[18, 24, 28, 255]); }
    Raster { rgba, min_x: -1., max_x: 1., min_z: -1., max_z: 1., min_y: 0., max_y: 1. }
}

fn barycentric(px: f32, py: f32, a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> Option<(f32, f32, f32)> {
    let (v0x, v0y) = (c[0] - a[0], c[1] - a[1]);
    let (v1x, v1y) = (b[0] - a[0], b[1] - a[1]);
    let (v2x, v2y) = (px - a[0], py - a[1]);
    let dot00 = v0x * v0x + v0y * v0y;
    let dot01 = v0x * v1x + v0y * v1y;
    let dot02 = v0x * v2x + v0y * v2y;
    let dot11 = v1x * v1x + v1y * v1y;
    let dot12 = v1x * v2x + v1y * v2y;
    let denom = dot00 * dot11 - dot01 * dot01;
    if denom.abs() < 1e-8 { return None; }
    let inv = 1. / denom;
    let along_c = (dot11 * dot02 - dot01 * dot12) * inv;
    let along_b = (dot00 * dot12 - dot01 * dot02) * inv;
    let along_a = 1. - along_c - along_b;
    if along_a < -0.01 || along_b < -0.01 || along_c < -0.01 { return None; }
    Some((along_a, along_b, along_c))
}

fn shade(y: f32, ny: f32, min_y: f32, max_y: f32) -> [u8; 4] {
    let span = (max_y - min_y).max(0.01);
    let height = ((y - min_y) / span).clamp(0., 1.);
    let light = 0.40 + 0.60 * ny.abs().clamp(0., 1.);
    let channel = |base: f32, lift: f32| ((base + lift * height) * light).clamp(0., 255.) as u8;
    [channel(46., 120.), channel(62., 96.), channel(54., 42.), 255]
}

fn overview_image(rgba: &[u8]) -> Image {
    Image::new(
        Extent3d { width: SIZE as u32, height: SIZE as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba.to_vec(),
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    )
}

fn spawns_path(asset_root: &Path) -> PathBuf {
    asset_root.parent().unwrap_or(asset_root).join("settings/map-spawns.json")
}

fn spawn_key(path: &Path) -> Option<String> {
    path.file_stem().and_then(|stem| stem.to_str()).map(|stem| stem.to_ascii_lowercase())
}

pub(crate) fn saved_spawn(asset_root: &Path, map_path: &Path) -> Option<[f32; 3]> {
    let key = spawn_key(map_path)?;
    let bytes = std::fs::read(spawns_path(asset_root)).ok()?;
    let table: BTreeMap<String, [f32; 3]> = serde_json::from_slice(&bytes).ok()?;
    table.get(&key).copied().filter(|spawn| spawn.iter().all(|value| value.is_finite()))
}

pub(crate) fn save_spawn(asset_root: &Path, map_path: &Path, spawn: [f32; 3]) -> Result<&'static str, String> {
    if spawn.iter().any(|value| !value.is_finite()) {
        return Err("Spawn must be a finite position".into());
    }
    let key = spawn_key(map_path).ok_or("This world has no map file")?;
    let path = spawns_path(asset_root);
    let mut table: BTreeMap<String, [f32; 3]> = std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    table.insert(key, spawn);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&table).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let sidecar = map_path.with_extension("import.json");
    if sidecar.is_file() {
        if let Ok(text) = std::fs::read_to_string(&sidecar) {
            if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text) {
                value["spawn"] = serde_json::json!(spawn);
                if let Ok(bytes) = serde_json::to_vec_pretty(&value) {
                    let _ = std::fs::write(&sidecar, bytes);
                }
            }
        }
    }
    match skate_data::skate_map::SkateMap::load(map_path) {
        Ok(mut map) => {
            map.spawn = spawn;
            match map.to_v9_bytes() {
                Ok(bytes) => {
                    replace_file(map_path, &bytes)?;
                    Ok("Saved as this map's default spawn.")
                }
                Err(_) => Ok("Saved. Retail map files stay as they are; the next load uses settings/map-spawns.json."),
            }
        }
        Err(error) => Err(format!("Spawn override saved, but the map file could not be read: {error}")),
    }
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("skate.saving");
    std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    match std::fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            let backup = path.with_extension("skate.bak");
            std::fs::rename(path, &backup).map_err(|error| error.to_string())?;
            if let Err(error) = std::fs::rename(&temporary, path) {
                let _ = std::fs::rename(&backup, path);
                return Err(error.to_string());
            }
            let _ = std::fs::remove_file(&backup);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_triangle_paints_the_middle_of_the_overview() {
        let triangle = PlotTri {
            p: [[-10., 2., -10.], [10., 2., -10.], [0., 4., 10.]],
            ny: 1.,
        };
        let raster = rasterize_plots(&[triangle]);
        let mid = SIZE / 2 * SIZE + SIZE / 2;
        assert_ne!(&raster.rgba[mid * 4..mid * 4 + 3], &[18, 24, 28]);
        assert!(raster.max_y > raster.min_y);
    }

    #[test]
    fn the_catch_floor_does_not_cover_real_ground() {
        let ground = PlotTri { p: [[-2., 1., -2.], [2., 1., -2.], [0., 1., 2.]], ny: 1. };
        let floor = PlotTri { p: [[-50., -0.25, -50.], [50., -0.25, -50.], [0., -0.25, 50.]], ny: 1. };
        let raster = rasterize_plots(&[floor, ground]);
        let mid = SIZE / 2 * SIZE + SIZE / 2;
        assert_ne!(&raster.rgba[mid * 4..mid * 4 + 3], &[18, 24, 28]);
        assert!(raster.min_y > -0.1);
    }

    #[test]
    fn saved_spawn_round_trips_by_map_name() {
        let root = std::env::temp_dir().join(format!("skate3-spawns-{}", std::process::id()));
        let assets = root.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let map = root.join("maps").join("Laru.skate");
        std::fs::create_dir_all(map.parent().unwrap()).unwrap();
        std::fs::write(&map, b"not a skate file").unwrap();
        let error = save_spawn(&assets, &map, [1.5, 3.25, -2.]).unwrap_err();
        assert!(error.contains("could not be read"), "{error}");
        assert_eq!(saved_spawn(&assets, &map), Some([1.5, 3.25, -2.]));
        let _ = std::fs::remove_dir_all(&root);
    }
}
