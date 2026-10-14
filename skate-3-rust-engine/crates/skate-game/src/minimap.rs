//! Top-down map overview, the corner HUD, and the pause-menu click-to-spawn view.
//!
//! World +X is to the left and world +Z (yaw 0, the way the skater faces) is
//! toward the top. The rendered world is the reference; this view is mirrored
//! in X so it matches that picture. The picture fits the whole map in the square at one
//! metres-per-pixel, letterboxed when the bounds are not square. Imported and
//! retail maps are painted from render triangles while the package is still in
//! memory. The test world, which has no package, keeps the height shading.
use crate::graphics_menu::Menu;
use crate::map_transition::CurrentMap;
use crate::physics::{GamePhysics, SkaterRuntime};
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use skate_core::math::Vector3;
use skate_core::physics::board_world::{BoardWorld, WorldTriangle};
use skate_data::skate_map::SkateMap;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SIZE: usize = 512;
const FRAME: f32 = 180.;
const PICKER: f32 = 560.;
const HEADROOM: f32 = 2.;
/// `normal.y` above this is ground rather than a wall. About 70° from flat.
const WALKABLE: f32 = 0.35;

/// Albedo overview produced on the map loader, applied when that world's generation is current.
#[derive(Resource, Default)]
pub(crate) struct OverviewPicture {
    pub generation: u64,
    pub picture: Option<MapPicture>,
    pub ready: bool,
}

pub(crate) struct MapPicture {
    rgba: Vec<u8>,
    min_x: f32,
    max_x: f32,
    min_z: f32,
    max_z: f32,
    min_y: f32,
    max_y: f32,
}

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
    /// Albedo at the centroid. Absent pictures use the height ramp.
    color: Option<[u8; 3]>,
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
            .init_resource::<OverviewPicture>()
            .add_systems(PostStartup, setup)
            .add_systems(Update, (rebuild, update_hud, update_picker).chain());
    }
}

#[derive(Component)]
struct MinimapRoot;
#[derive(Component)]
struct MinimapFrame;
#[derive(Component)]
struct MinimapNorth;
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
        root.spawn((MinimapFrame, frame)).with_children(|frame| {
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
                MinimapNorth,
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
                Text::new("Scroll or + / − zooms toward the cursor. Middle-drag or the arrows pan. Left click teleports and resumes. Right click also saves that spot."),
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
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(PICKER),
                        height: px(PICKER),
                        ..default()
                    },
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
    picture: Res<OverviewPicture>,
    mut images: ResMut<Assets<Image>>,
    physics: Res<GamePhysics>,
    current: Option<Res<CurrentMap>>,
    mut built: Local<Option<u64>>,
) {
    let generation = current.as_ref().map(|map| map.generation).unwrap_or(0);
    if overview.ready && *built == Some(generation) {
        return;
    }
    if picture.ready && picture.generation == generation {
        if let Some(shot) = &picture.picture {
            install_picture(&mut overview, &mut images, shot);
            *built = Some(generation);
            return;
        }
    }
    let triangles = physics.world_triangles();
    if triangles.is_empty() {
        return;
    }
    let raster = rasterize(triangles);
    install_picture(&mut overview, &mut images, &MapPicture {
        rgba: raster.rgba,
        min_x: raster.min_x,
        max_x: raster.max_x,
        min_z: raster.min_z,
        max_z: raster.max_z,
        min_y: raster.min_y,
        max_y: raster.max_y,
    });
    *built = Some(generation);
}

fn install_picture(overview: &mut MapOverview, images: &mut Assets<Image>, picture: &MapPicture) {
    if overview.image != Handle::default() {
        images.remove(&overview.image);
    }
    overview.image = images.add(overview_image(&picture.rgba));
    overview.min_x = picture.min_x;
    overview.max_x = picture.max_x;
    overview.min_z = picture.min_z;
    overview.max_z = picture.max_z;
    overview.min_y = picture.min_y;
    overview.max_y = picture.max_y;
    overview.ready = true;
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
    mut root: Query<&mut Node, (With<MinimapRoot>, Without<MinimapImage>, Without<MinimapPin>, Without<MinimapFrame>, Without<MinimapNorth>)>,
    mut frames: Query<&mut Node, (With<MinimapFrame>, Without<MinimapRoot>, Without<MinimapImage>, Without<MinimapNorth>, Without<MinimapPin>)>,
    mut north: Query<&mut Node, (With<MinimapNorth>, Without<MinimapRoot>, Without<MinimapFrame>, Without<MinimapImage>, Without<MinimapPin>)>,
    mut images: Query<(&mut Node, &mut ImageNode), (With<MinimapImage>, Without<MinimapRoot>, Without<MinimapPin>, Without<MinimapFrame>, Without<MinimapNorth>)>,
    mut pins: Query<(&MinimapPin, &mut Node, &mut UiTransform), (Without<MinimapImage>, Without<MinimapRoot>, Without<MinimapFrame>, Without<MinimapNorth>)>,
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
    let frame = menu.minimap_size();
    root.top = px(if menu.frame_stats_visible() { 78. } else { 16. });
    root.width = px(frame);
    if let Ok(mut node) = frames.single_mut() {
        node.width = px(frame);
        node.height = px(frame);
    }
    if let Ok(mut node) = north.single_mut() {
        node.left = px(frame * 0.5 - 4.);
    }
    let skater = skater.unwrap();
    let pose = skater.animated_skeleton.roots.animation_to_world;
    let (x, y, z) = (pose[3][0], pose[3][1], pose[3][2]);
    let yaw = pose[2][0].atan2(pose[2][2]);
    if let Ok((mut node, mut image)) = images.single_mut() {
        image.image = overview.image.clone();
        let layout = fit_layout(&overview, frame);
        node.width = px(layout.image_w);
        node.height = px(layout.image_h);
        node.left = px(layout.left);
        node.top = px(layout.top);
        place_pins(&overview, &mut pins, layout, x, z, yaw, frame, current.as_deref().map(|map| map.spawn), session.as_ref().and_then(|marker| marker.placed_position()));
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
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    mut root: Query<&mut Node, (With<PickerRoot>, Without<PickerPin>, Without<PickerImage>)>,
    mut image: Query<(&mut Node, &mut ImageNode, &RelativeCursorPosition), With<PickerImage>>,
    mut pins: Query<(&PickerPin, &mut Node, &mut UiTransform), (Without<PickerImage>, Without<PickerRoot>)>,
    mut readout: Query<&mut Text, With<PickerReadout>>,
    buttons: Query<(&PickerButton, &Interaction)>,
    mut state: Local<PickerState>,
) {
    let mut steps = wheel.read().map(|event| {
        event.y * if event.unit == bevy::input::mouse::MouseScrollUnit::Line { 1. } else { 1. / 40. }
    }).sum::<f32>();
    let open = menu.as_ref().is_some_and(|menu| menu.place_on_map()) && overview.ready;
    let Ok(mut root) = root.single_mut() else { return };
    root.display = if open { Display::Flex } else { Display::None };
    if !open {
        *state = PickerState::default();
        return;
    }
    let fresh = menu.as_mut().is_some_and(|menu| menu.place_is_fresh());
    let Ok((mut node, mut picture, cursor)) = image.single_mut() else { return };
    if fresh {
        state.zoom = 1.;
        state.center_x = (overview.min_x + overview.max_x) * 0.5;
        state.center_z = (overview.min_z + overview.max_z) * 0.5;
        state.drag = None;
    }
    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) { steps += 1.; }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) { steps -= 1.; }
    // Bevy's cursor is centered: (-0.5, -0.5) is the node's top-left.
    let cursor_uv = cursor.normalized.filter(|_| cursor.cursor_over()).map(|point| cursor_to_uv(point.x, point.y));
    if steps.abs() > f32::EPSILON {
        zoom_about(&mut state, &overview, steps, cursor_uv);
    }
    pan_from_keys(&mut state, &overview, &keys);
    pan_from_drag(&mut state, &overview, cursor_uv, mouse.pressed(MouseButton::Middle));
    let window = map_window(&overview, state.zoom, state.center_x, state.center_z);
    state.center_x = (window.min_x + window.max_x) * 0.5;
    state.center_z = (window.min_z + window.max_z) * 0.5;
    picture.image = overview.image.clone();
    picture.rect = picker_crop(&overview, &window);
    picture.image_mode = NodeImageMode::Stretch;
    let layout = fit_window(&window, PICKER);
    node.width = px(layout.image_w);
    node.height = px(layout.image_h);
    node.left = px(layout.left);
    node.top = px(layout.top);
    if let Some((u, v)) = cursor_uv {
        let (x, z) = window_to_xz(u, v, &window);
        let y = walkable_surface(physics.world(), x, z, overview.min_y, overview.max_y);
        state.hover = Some([x, y.unwrap_or(overview.min_y.max(0.)), z]);
    }
    if let (Ok(mut text), Some(spot)) = (readout.single_mut(), state.hover) {
        let ground = walkable_surface(physics.world(), spot[0], spot[2], overview.min_y, overview.max_y);
        let magnification = if state.zoom > 1.05 { format!("    {:.1}×", state.zoom) } else { String::new() };
        **text = match ground {
            Some(y) => format!("X {:.1}    Y {:.1}    Z {:.1}{magnification}", spot[0], y, spot[2]),
            None => format!("X {:.1}    no ground    Z {:.1}{magnification}", spot[0], spot[2]),
        };
    }
    if let Some(skater) = skater.as_ref() {
        let pose = skater.animated_skeleton.roots.animation_to_world;
        place_picker_pins(&window, &mut pins, layout, pose[3][0], pose[3][2], pose[2][0].atan2(pose[2][2]), current.as_ref().map(|map| map.spawn));
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
    let Some(spot) = state.hover else {
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
    let contact = walkable_surface(physics.world(), spot[0], spot[2], overview.min_y, overview.max_y).unwrap_or(spot[1]);
    let y = contact + board_lift(&physics);
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

/// Fit the whole map in `frame` pixels. One scale for both axes, so a wide map is letterboxed.
fn fit_layout(overview: &MapOverview, frame: f32) -> Layout {
    fit_spans(overview.max_x - overview.min_x, overview.max_z - overview.min_z, frame)
}

fn fit_window(window: &MapWindow, frame: f32) -> Layout {
    fit_spans(window.max_x - window.min_x, window.max_z - window.min_z, frame)
}

fn fit_spans(span_x: f32, span_z: f32, frame: f32) -> Layout {
    let span_x = span_x.max(1.);
    let span_z = span_z.max(1.);
    let scale = frame / span_x.max(span_z);
    let image_w = span_x * scale;
    let image_h = span_z * scale;
    Layout {
        image_w,
        image_h,
        left: (frame - image_w) * 0.5,
        top: (frame - image_h) * 0.5,
    }
}

const PICKER_ZOOM_MIN: f32 = 1.;
const PICKER_ZOOM_MAX: f32 = 64.;
const PICKER_ZOOM_STEP: f32 = 1.25;

struct PickerState {
    hover: Option<[f32; 3]>,
    zoom: f32,
    center_x: f32,
    center_z: f32,
    drag: Option<(f32, f32)>,
    pan_at: Option<std::time::Instant>,
}

impl Default for PickerState {
    fn default() -> Self {
        Self { hover: None, zoom: 1., center_x: 0., center_z: 0., drag: None, pan_at: None }
    }
}

struct MapWindow {
    min_x: f32,
    max_x: f32,
    min_z: f32,
    max_z: f32,
}

/// Visible world rectangle. Zoom 1 is the whole map. The window keeps the map's aspect, so the letterbox does not move.
fn map_window(overview: &MapOverview, zoom: f32, center_x: f32, center_z: f32) -> MapWindow {
    let span_x = (overview.max_x - overview.min_x).max(0.01);
    let span_z = (overview.max_z - overview.min_z).max(0.01);
    let zoom = zoom.clamp(PICKER_ZOOM_MIN, PICKER_ZOOM_MAX);
    let (min_x, max_x) = clamp_span(overview.min_x, overview.max_x, center_x, span_x / zoom);
    let (min_z, max_z) = clamp_span(overview.min_z, overview.max_z, center_z, span_z / zoom);
    MapWindow { min_x, max_x, min_z, max_z }
}

fn clamp_span(min_bound: f32, max_bound: f32, center: f32, size: f32) -> (f32, f32) {
    let span = max_bound - min_bound;
    if size >= span - 0.0001 {
        return (min_bound, max_bound);
    }
    let mut start = center - size * 0.5;
    let mut end = center + size * 0.5;
    if start < min_bound {
        end += min_bound - start;
        start = min_bound;
    }
    if end > max_bound {
        start -= end - max_bound;
        end = max_bound;
    }
    (start, end)
}

fn zoom_about(state: &mut PickerState, overview: &MapOverview, steps: f32, cursor_uv: Option<(f32, f32)>) {
    let next = (state.zoom * PICKER_ZOOM_STEP.powf(steps)).clamp(PICKER_ZOOM_MIN, PICKER_ZOOM_MAX);
    if let Some((u, v)) = cursor_uv {
        let window = map_window(overview, state.zoom, state.center_x, state.center_z);
        let (x, z) = window_to_xz(u, v, &window);
        let (center_x, center_z) = center_on_cursor(x, z, u, v, overview, next);
        state.center_x = center_x;
        state.center_z = center_z;
    }
    state.zoom = next;
}

fn center_on_cursor(x: f32, z: f32, u: f32, v: f32, overview: &MapOverview, zoom: f32) -> (f32, f32) {
    let span_x = (overview.max_x - overview.min_x).max(0.01);
    let span_z = (overview.max_z - overview.min_z).max(0.01);
    let width = span_x / zoom.clamp(PICKER_ZOOM_MIN, PICKER_ZOOM_MAX);
    let height = span_z / zoom.clamp(PICKER_ZOOM_MIN, PICKER_ZOOM_MAX);
    (x + width * (u - 0.5), z - height * (0.5 - v))
}

fn pan_from_keys(state: &mut PickerState, overview: &MapOverview, keys: &ButtonInput<KeyCode>) {
    let dx = (keys.pressed(KeyCode::ArrowRight) as i32 - keys.pressed(KeyCode::ArrowLeft) as i32) as f32;
    let dz = (keys.pressed(KeyCode::ArrowUp) as i32 - keys.pressed(KeyCode::ArrowDown) as i32) as f32;
    if dx == 0. && dz == 0. {
        state.pan_at = None;
        return;
    }
    let now = std::time::Instant::now();
    let dt = state.pan_at.map(|then| now.saturating_duration_since(then).as_secs_f32()).unwrap_or(0.).min(0.05);
    state.pan_at = Some(now);
    let span_x = (overview.max_x - overview.min_x).max(0.01);
    let span_z = (overview.max_z - overview.min_z).max(0.01);
    state.center_x -= dx * dt * 0.85 * span_x / state.zoom;
    state.center_z += dz * dt * 0.85 * span_z / state.zoom;
}

fn pan_from_drag(state: &mut PickerState, overview: &MapOverview, cursor_uv: Option<(f32, f32)>, dragging: bool) {
    if !dragging {
        state.drag = None;
        return;
    }
    let Some((u, v)) = cursor_uv else { return };
    if let Some((u0, v0)) = state.drag {
        let span_x = (overview.max_x - overview.min_x).max(0.01);
        let span_z = (overview.max_z - overview.min_z).max(0.01);
        state.center_x += (u - u0) * span_x / state.zoom;
        state.center_z -= (v - v0) * span_z / state.zoom;
    }
    state.drag = Some((u, v));
}

fn window_to_xz(u: f32, v: f32, window: &MapWindow) -> (f32, f32) {
    (
        window.max_x - u * (window.max_x - window.min_x),
        window.max_z - v * (window.max_z - window.min_z),
    )
}

/// Crop of the 512² overview. `None` is the whole texture, which is zoom 1.
fn picker_crop(overview: &MapOverview, window: &MapWindow) -> Option<Rect> {
    let span_x = (overview.max_x - overview.min_x).max(0.01);
    let span_z = (overview.max_z - overview.min_z).max(0.01);
    let u0 = (overview.max_x - window.max_x) / span_x;
    let u1 = (overview.max_x - window.min_x) / span_x;
    let v0 = (overview.max_z - window.max_z) / span_z;
    let v1 = (overview.max_z - window.min_z) / span_z;
    if u0 <= 0.001 && v0 <= 0.001 && u1 >= 0.999 && v1 >= 0.999 {
        return None;
    }
    Some(Rect::from_corners(
        Vec2::new(u0.clamp(0., 1.) * SIZE as f32, v0.clamp(0., 1.) * SIZE as f32),
        Vec2::new(u1.clamp(0., 1.) * SIZE as f32, v1.clamp(0., 1.) * SIZE as f32),
    ))
}

fn place_pins(
    overview: &MapOverview,
    pins: &mut Query<(&MinimapPin, &mut Node, &mut UiTransform), (Without<MinimapImage>, Without<MinimapRoot>, Without<MinimapFrame>, Without<MinimapNorth>)>,
    layout: Layout,
    x: f32,
    z: f32,
    yaw: f32,
    frame: f32,
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
        let (x, y) = pin_px(overview, &layout, spot[0], spot[2], frame);
        node.left = Val::Px(x - 5.);
        node.top = Val::Px(y - 5.);
        transform.rotation = if rotate { Rot2::radians(marker_angle(yaw)) } else { Rot2::IDENTITY };
    }
}

fn place_picker_pins(
    window: &MapWindow,
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
        let Some((px, py)) = pin_in_window(window, &layout, spot[0], spot[2]) else {
            node.display = Display::None;
            continue;
        };
        node.display = Display::Flex;
        node.left = Val::Px(px - 5.);
        node.top = Val::Px(py - 5.);
        transform.rotation = if pin.0 == 0 { Rot2::radians(marker_angle(yaw)) } else { Rot2::IDENTITY };
    }
}

fn pin_in_window(window: &MapWindow, layout: &Layout, x: f32, z: f32) -> Option<(f32, f32)> {
    let span_x = (window.max_x - window.min_x).max(0.01);
    let span_z = (window.max_z - window.min_z).max(0.01);
    let u = (window.max_x - x) / span_x;
    let v = (window.max_z - z) / span_z;
    if !(0. ..=1.).contains(&u) || !(0. ..=1.).contains(&v) {
        return None;
    }
    Some((layout.left + u * layout.image_w, layout.top + v * layout.image_h))
}

fn pin_px(overview: &MapOverview, layout: &Layout, x: f32, z: f32, frame: f32) -> (f32, f32) {
    let (u, v) = uv_of(overview, x, z);
    let x = layout.left + u * layout.image_w;
    let y = layout.top + v * layout.image_h;
    (x.clamp(6., frame - 6.), y.clamp(6., frame - 6.))
}

/// Nose graphic points up the screen (local -Y). Positive yaw faces +X, which is screen left.
/// Bevy's `Rot2` is counterclockwise and UI +Y points down, so the mirrored angle is `-yaw`.
fn marker_angle(yaw: f32) -> f32 { -yaw }

/// `RelativeCursorPosition.normalized` is centered on the node.
fn cursor_to_uv(x: f32, y: f32) -> (f32, f32) {
    ((x + 0.5).clamp(0., 1.), (y + 0.5).clamp(0., 1.))
}

fn uv_of(overview: &MapOverview, x: f32, z: f32) -> (f32, f32) {
    let span_x = (overview.max_x - overview.min_x).max(0.01);
    let span_z = (overview.max_z - overview.min_z).max(0.01);
    ((overview.max_x - x) / span_x, (overview.max_z - z) / span_z)
}

fn uv_to_xz(u: f32, v: f32, overview: &MapOverview) -> (f32, f32) {
    let x = overview.max_x - u * (overview.max_x - overview.min_x);
    let z = overview.max_z - v * (overview.max_z - overview.min_z);
    (x, z)
}

/// Lowest upward surface at `(x, z)` that has `HEADROOM` metres clear above it.
/// The catch floor under `y = -0.1` is ignored while any other hit exists, so a roof
/// does not hide the street and the safety plane does not hide the street either.
pub(crate) fn walkable_surface(world: &BoardWorld, x: f32, z: f32, min_y: f32, max_y: f32) -> Option<f32> {
    let mut top = max_y + 50.;
    let bottom = min_y - 10.;
    let mut heights = Vec::new();
    for _ in 0..48 {
        if top <= bottom { break; }
        let Ok(Some(hit)) = world.query_thin_line(Vector3::new(x, top, z), Vector3::new(x, bottom, z)) else { break };
        let y = hit.geometry.position.y;
        if hit.geometry.normal.y >= WALKABLE && y.is_finite() {
            heights.push(y);
        }
        let next = y - 0.05;
        if next >= top - 0.001 { break; }
        top = next;
    }
    lowest_open(&heights)
}

/// Contact height for a package spawn. A point already on or above the surface stays
/// there (retail drop-ins). A point below the surface is lifted onto it.
pub(crate) fn contact_height(world: &BoardWorld, x: f32, z: f32, authored: f32) -> f32 {
    let (min_y, max_y) = world.triangles().iter().flat_map(|triangle| triangle.triangle.vertices).fold(
        (f32::MAX, f32::MIN),
        |(min_y, max_y), point| (min_y.min(point.y), max_y.max(point.y)),
    );
    if !min_y.is_finite() || !max_y.is_finite() {
        return authored;
    }
    match walkable_surface(world, x, z, min_y, max_y) {
        Some(surface) if authored + 0.05 < surface => surface,
        _ => authored,
    }
}

fn board_lift(physics: &GamePhysics) -> f32 {
    physics.settings.wheel_radius - physics.settings.authored[0].translation.y
}

fn lowest_open(heights: &[f32]) -> Option<f32> {
    let raised: Vec<f32> = heights.iter().copied().filter(|y| *y >= -0.1).collect();
    let set = if raised.is_empty() { heights } else { raised.as_slice() };
    set.iter().copied().filter(|y| {
        !set.iter().any(|other| *other > *y + 0.05 && *other < *y + HEADROOM)
    }).reduce(f32::min)
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
        Some(PlotTri { p, ny, color: None })
    }).collect();
    rasterize_plots(&plotted)
}

/// Render-mesh overview. Stored textures are bottom-up and the UVs match that, so the sample uses them directly.
pub(crate) fn picture_of(map: &SkateMap) -> MapPicture {
    let plotted: Vec<PlotTri> = map.geometry.indices.chunks_exact(3).filter_map(|tri| {
        let verts = [
            map.geometry.vertices.get(tri[0] as usize)?,
            map.geometry.vertices.get(tri[1] as usize)?,
            map.geometry.vertices.get(tri[2] as usize)?,
        ];
        let p = [verts[0].position, verts[1].position, verts[2].position];
        if p.iter().any(|corner| corner.iter().any(|value| !value.is_finite())) { return None; }
        let edge_u = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let edge_v = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
        let cross = [
            edge_u[1] * edge_v[2] - edge_u[2] * edge_v[1],
            edge_u[2] * edge_v[0] - edge_u[0] * edge_v[2],
            edge_u[0] * edge_v[1] - edge_u[1] * edge_v[0],
        ];
        let length = cross.iter().map(|value| value * value).sum::<f32>().sqrt();
        let ny = if length > 1e-8 { cross[1] / length } else { 1. };
        let uv = [
            (verts[0].uv[0] + verts[1].uv[0] + verts[2].uv[0]) / 3.,
            (verts[0].uv[1] + verts[1].uv[1] + verts[2].uv[1]) / 3.,
        ];
        Some(PlotTri { p, ny, color: Some(sample_albedo(map, verts[0].material, uv)) })
    }).collect();
    let raster = rasterize_plots(&plotted);
    MapPicture {
        rgba: raster.rgba,
        min_x: raster.min_x,
        max_x: raster.max_x,
        min_z: raster.min_z,
        max_z: raster.max_z,
        min_y: raster.min_y,
        max_y: raster.max_y,
    }
}

fn sample_albedo(map: &SkateMap, material: u32, uv: [f32; 2]) -> [u8; 3] {
    let fallback = map.materials.get(material.saturating_sub(1) as usize).map(|material| {
        material.color.map(|channel| (channel.clamp(0., 1.) * 255.) as u8)
    }).unwrap_or([180, 180, 180]);
    let Some(material) = map.materials.get(material.saturating_sub(1) as usize) else { return fallback };
    let id = material.textures[0] as usize;
    if id == 0 || id > map.textures.len() { return fallback; }
    let texture = &map.textures[id - 1];
    if texture.width == 0 || texture.height == 0 { return fallback; }
    let pixels = texture.width as usize * texture.height as usize;
    if texture.rgba.len() < pixels * 4 { return fallback; }
    let u = uv[0] - uv[0].floor();
    let v = uv[1] - uv[1].floor();
    let x = ((u * texture.width as f32) as usize).min(texture.width as usize - 1);
    // Row 0 of the stored image is the bottom, matching v = 0.
    let y = ((v * texture.height as f32) as usize).min(texture.height as usize - 1);
    let index = (y * texture.width as usize + x) * 4;
    [texture.rgba[index], texture.rgba[index + 1], texture.rgba[index + 2]]
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
            (max_x - x) / span_x * (SIZE as f32 - 1.),
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
                let color = paint(triangle, y, min_y, max_y);
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

fn paint(triangle: &PlotTri, y: f32, min_y: f32, max_y: f32) -> [u8; 4] {
    let Some(rgb) = triangle.color else { return shade(y, triangle.ny, min_y, max_y); };
    let light = 0.62 + 0.38 * triangle.ny.abs().clamp(0., 1.);
    let scale = |channel: u8| (channel as f32 * light).clamp(0., 255.) as u8;
    [scale(rgb[0]), scale(rgb[1]), scale(rgb[2]), 255]
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
            color: None,
        };
        let raster = rasterize_plots(&[triangle]);
        let mid = SIZE / 2 * SIZE + SIZE / 2;
        assert_ne!(&raster.rgba[mid * 4..mid * 4 + 3], &[18, 24, 28]);
        assert!(raster.max_y > raster.min_y);
    }

    #[test]
    fn the_catch_floor_does_not_cover_real_ground() {
        let ground = PlotTri { p: [[-2., 1., -2.], [2., 1., -2.], [0., 1., 2.]], ny: 1., color: None };
        let floor = PlotTri { p: [[-50., -0.25, -50.], [50., -0.25, -50.], [0., -0.25, 50.]], ny: 1., color: None };
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

    #[test]
    fn the_hud_node_queries_can_be_scheduled_together() {
        let mut app = App::new();
        app.init_resource::<MapOverview>().add_systems(Update, update_hud);
        app.update();
    }

    #[test]
    fn the_whole_map_fits_and_a_click_lands_on_the_same_spot() {
        let overview = MapOverview {
            image: Handle::default(),
            min_x: -20., max_x: 20., min_z: -5., max_z: 5., min_y: 0., max_y: 1., ready: true,
        };
        let layout = fit_layout(&overview, 100.);
        assert!((layout.image_w - 100.).abs() < 0.01, "{}", layout.image_w);
        assert!((layout.image_h - 25.).abs() < 0.01, "{}", layout.image_h);
        assert!((layout.top - 37.5).abs() < 0.01, "{}", layout.top);
        let (u, v) = cursor_to_uv(-0.5, -0.5);
        let (x, z) = uv_to_xz(u, v, &overview);
        assert!((x - overview.max_x).abs() < 0.01 && (z - overview.max_z).abs() < 0.01, "{x} {z}");
        let (u, v) = cursor_to_uv(0., 0.);
        let (x, z) = uv_to_xz(u, v, &overview);
        assert!(x.abs() < 0.01 && z.abs() < 0.01, "{x} {z}");
        let (px, py) = pin_px(&overview, &layout, 0., 0., 100.);
        assert!((px - 50.).abs() < 0.1 && (py - 50.).abs() < 0.1, "{px} {py}");
    }

    #[test]
    fn zooming_keeps_the_point_under_the_cursor() {
        let overview = MapOverview {
            image: Handle::default(),
            min_x: -200., max_x: 200., min_z: -50., max_z: 50., min_y: 0., max_y: 1., ready: true,
        };
        let whole = map_window(&overview, 1., 0., 0.);
        assert!((whole.min_x - overview.min_x).abs() < 0.01 && (whole.max_z - overview.max_z).abs() < 0.01);
        assert!(picker_crop(&overview, &whole).is_none());
        let layout = fit_window(&whole, 100.);
        let full = fit_layout(&overview, 100.);
        assert!((layout.image_w - full.image_w).abs() < 0.01 && (layout.image_h - full.image_h).abs() < 0.01);
        let (u, v) = (0.8, 0.25);
        let (x, z) = window_to_xz(u, v, &whole);
        let mut state = PickerState { center_x: 0., center_z: 0., ..PickerState::default() };
        zoom_about(&mut state, &overview, 4_f32.log(PICKER_ZOOM_STEP), Some((u, v)));
        let window = map_window(&overview, state.zoom, state.center_x, state.center_z);
        state.center_x = (window.min_x + window.max_x) * 0.5;
        state.center_z = (window.min_z + window.max_z) * 0.5;
        let window = map_window(&overview, state.zoom, state.center_x, state.center_z);
        let (x2, z2) = window_to_xz(u, v, &window);
        assert!((x2 - x).abs() < 0.05 && (z2 - z).abs() < 0.05, "{x2} {z2} vs {x} {z}");
        assert!((state.zoom - 4.).abs() < 0.02, "{}", state.zoom);
        let zoomed = fit_window(&window, 100.);
        let (px, py) = pin_in_window(&window, &zoomed, x, z).unwrap();
        assert!((px - (zoomed.left + u * zoomed.image_w)).abs() < 0.1, "{px}");
        assert!((py - (zoomed.top + v * zoomed.image_h)).abs() < 0.1, "{py}");
        let crop = picker_crop(&overview, &window).unwrap();
        assert!((crop.min.x / SIZE as f32 - (overview.max_x - window.max_x) / 400.).abs() < 0.01);
        assert!(pin_in_window(&window, &zoomed, overview.min_x, overview.min_z).is_none());
    }

    #[test]
    fn a_textured_triangle_keeps_its_color() {
        let triangle = PlotTri {
            p: [[-10., 2., -10.], [10., 2., -10.], [0., 4., 10.]],
            ny: 1.,
            color: Some([200, 20, 20]),
        };
        let raster = rasterize_plots(&[triangle]);
        let mid = SIZE / 2 * SIZE + SIZE / 2;
        let pixel = &raster.rgba[mid * 4..mid * 4 + 3];
        assert!(pixel[0] > pixel[1] && pixel[0] > pixel[2], "{pixel:?}");
    }

    #[test]
    fn yaw_zero_points_up_and_positive_x_points_left() {
        let nose = |yaw: f32| {
            let (s, c) = marker_angle(yaw).sin_cos();
            [s, -c]
        };
        assert!(nose(0.)[1] < -0.9 && nose(0.)[0].abs() < 0.01);
        assert!(nose(std::f32::consts::FRAC_PI_2)[0] < -0.9 && nose(std::f32::consts::FRAC_PI_2)[1].abs() < 0.01);
    }

    #[test]
    fn positive_x_paints_the_left_side() {
        let east = PlotTri { p: [[5., 1., -2.], [15., 1., -2.], [10., 1., 2.]], ny: 1., color: Some([220, 20, 20]) };
        let west = PlotTri { p: [[-15., 1., -2.], [-5., 1., -2.], [-10., 1., 2.]], ny: 1., color: Some([20, 20, 220]) };
        let raster = rasterize_plots(&[east, west]);
        let left = (SIZE / 4) * 4 + (SIZE / 2) * SIZE * 4;
        let right = (SIZE * 3 / 4) * 4 + (SIZE / 2) * SIZE * 4;
        assert!(raster.rgba[left] > raster.rgba[left + 2], "left {:?}", &raster.rgba[left..left + 3]);
        assert!(raster.rgba[right + 2] > raster.rgba[right], "right {:?}", &raster.rgba[right..right + 3]);
    }

    #[test]
    fn walkable_ground_prefers_the_open_floor_over_a_roof_or_catch_plane() {
        let mut triangles = quad(0.);
        triangles.extend(quad(40.));
        triangles.extend(quad(-0.25));
        let world = BoardWorld::new(triangles);
        let floor = walkable_surface(&world, 0., 0., -1., 50.).unwrap();
        assert!(floor.abs() < 0.2, "{floor}");
        assert!((contact_height(&world, 0., 0., 0.5) - 0.5).abs() < 0.01);
        let high = BoardWorld::new(quad(40.));
        assert!((contact_height(&high, 0., 0., 1.) - 40.).abs() < 0.2);
        let raised = BoardWorld::new({ let mut t = quad(5.); t.extend(quad(-0.25)); t });
        assert!((walkable_surface(&raised, 0., 0., -1., 10.).unwrap() - 5.).abs() < 0.2);
    }

    fn quad(y: f32) -> Vec<WorldTriangle> {
        use skate_core::physics::collision::TriangleFeature;
        use skate_core::physics::contact::RetailContactMaterial;
        let material = RetailContactMaterial { static_friction: 0., dynamic_friction: 0., restitution: 0. };
        let v = |x, z| Vector3::new(x, y, z);
        let corners = [v(-20., -20.), v(20., -20.), v(20., 20.), v(-20., 20.)];
        [[0usize, 2, 1], [0, 3, 2]].into_iter().map(|ids| {
            WorldTriangle::from_vertices(ids.map(|index| corners[index]), material, 1, TriangleFeature::ONE_SIDED, [1.; 3], 0.).unwrap()
        }).collect()
    }
}
