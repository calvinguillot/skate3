//! FBX map import from the command line, the map menu, and a file dropped on the window.
use crate::config::Config;
use bevy::prelude::*;
use bevy::window::FileDragAndDrop;
use serde::{Deserialize, Serialize};
use skate_map_import::{import_file, ImportOptions, ImportReport};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Mutex;
use std::thread;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SavedImport {
    scale: f32,
    collision: String,
    floor: bool,
    max_texture: u32,
    surface: u32,
}

impl Default for SavedImport {
    fn default() -> Self {
        Self { scale: 1.0, collision: "full".into(), floor: true, max_texture: 0, surface: 1 }
    }
}

struct Job {
    rx: Mutex<Receiver<Result<ImportReport, String>>>,
}

#[derive(Resource)]
pub(crate) struct MapImport {
    scale: f32,
    collision: String,
    floor: bool,
    max_texture: u32,
    surface: u32,
    path: PathBuf,
    job: Option<Job>,
}

impl Default for MapImport {
    fn default() -> Self {
        Self {
            scale: 1.0,
            collision: "full".into(),
            floor: true,
            max_texture: 0,
            surface: 1,
            path: PathBuf::new(),
            job: None,
        }
    }
}

impl MapImport {
    pub(crate) fn load(asset_root: &Path) -> Self {
        let path = settings_path(asset_root);
        let saved = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                warn!("Invalid map import settings {}: {error}", path.display());
                SavedImport::default()
            }),
            Err(_) => SavedImport::default(),
        };
        Self {
            scale: saved.scale.clamp(0.01, 10.0),
            collision: saved.collision,
            floor: saved.floor,
            max_texture: saved.max_texture,
            surface: saved.surface,
            path,
            job: None,
        }
    }

    pub(crate) fn save(&self) -> Result<(), String> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let saved = SavedImport {
            scale: self.scale,
            collision: self.collision.clone(),
            floor: self.floor,
            max_texture: self.max_texture,
            surface: self.surface,
        };
        std::fs::write(&self.path, serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }

    pub(crate) fn busy(&self) -> bool { self.job.is_some() }

    pub(crate) fn scale_label(&self) -> String {
        format!("Import scale         {:.2}    Shift changes by 0.1", self.scale)
    }

    pub(crate) fn nudge_scale(&mut self, direction: i32, coarse: bool) {
        let step = if coarse { 0.1 } else { 0.01 };
        self.scale = (self.scale + direction as f32 * step).clamp(0.01, 10.0);
    }

    pub(crate) fn begin_dialog(&mut self, asset_root: PathBuf) {
        if self.job.is_some() { return; }
        let options = self.options("Imported map");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let Some(path) = rfd::FileDialog::new().set_title("Import an FBX map").add_filter("FBX", &["fbx"]).pick_file() else {
                let _ = tx.send(Err("cancelled".into()));
                return;
            };
            let _ = tx.send(run_import(&path, &asset_root, options));
        });
        self.job = Some(Job { rx: Mutex::new(rx) });
    }

    pub(crate) fn begin_path(&mut self, path: PathBuf, asset_root: PathBuf) {
        if self.job.is_some() { return; }
        let options = self.options("Imported map");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(run_import(&path, &asset_root, options));
        });
        self.job = Some(Job { rx: Mutex::new(rx) });
    }

    fn options(&self, name: &str) -> ImportOptions {
        ImportOptions {
            scale: self.scale,
            name: name.into(),
            collision: self.collision.clone(),
            floor: self.floor,
            spawn: None,
            heading: 0.0,
            max_texture: self.max_texture,
            surface: self.surface,
            crop: None,
        }
    }
}

fn settings_path(asset_root: &Path) -> PathBuf {
    asset_root.parent().unwrap_or(asset_root).join("settings/map-import.json")
}

fn run_import(source: &Path, asset_root: &Path, mut options: ImportOptions) -> Result<ImportReport, String> {
    if options.name.is_empty() || options.name == "Imported map" {
        options.name = source.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported map".into());
    }
    let output = asset_root.parent().unwrap_or(asset_root).join("maps");
    let report = import_file(source, &output, &options)?;
    let map = skate_data::skate_map::SkateMap::load(&report.output)?;
    crate::skate_world::validate_runtime(&map)?;
    Ok(report)
}

pub(crate) struct MapImportPlugin;

impl Plugin for MapImportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapImport>()
            .add_systems(Startup, load)
            .add_systems(Update, (poll, dropped));
    }
}

fn load(config: Res<Config>, mut imports: ResMut<MapImport>) {
    let job = imports.job.take();
    *imports = MapImport::load(&config.asset_root);
    imports.job = job;
}

fn poll(mut imports: ResMut<MapImport>, mut menu: Option<ResMut<crate::graphics_menu::Menu>>, config: Res<Config>) {
    let Some(job) = imports.job.as_ref() else { return };
    let result = match job.rx.lock().expect("import result").try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("Import stopped before it finished".into()),
    };
    imports.job = None;
    let Some(menu) = menu.as_mut() else { return };
    match result {
        Ok(report) => {
            let summary = report.to_string();
            menu.show_imported_maps(&config.asset_root);
            menu.set_status(format!("{summary}\nSelect the map in the list to skate it."));
        }
        Err(error) if error == "cancelled" => menu.set_status("Import cancelled".into()),
        Err(error) => menu.set_status(format!("Import failed: {error}\nThe current map is still loaded.")),
    }
}

fn dropped(mut events: MessageReader<FileDragAndDrop>, mut imports: ResMut<MapImport>, config: Res<Config>, mut menu: Option<ResMut<crate::graphics_menu::Menu>>) {
    for event in events.read() {
        let FileDragAndDrop::DroppedFile { path_buf: path, .. } = event else { continue };
        if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("fbx")) {
            imports.begin_path(path.clone(), config.asset_root.clone());
            if let Some(menu) = menu.as_mut() {
                menu.set_status(format!("Importing {}", path.display()));
            }
        }
    }
}

/// `--import-map` converts a file and exits before the game starts.
pub(crate) fn cli() -> Option<i32> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if !args.iter().any(|arg| arg.to_str() == Some("--import-map")) {
        return None;
    }
    match cli_run(&args) {
        Ok(report) => {
            println!("{report}");
            Some(0)
        }
        Err(error) => {
            eprintln!("{error}");
            Some(1)
        }
    }
}

fn cli_run(args: &[std::ffi::OsString]) -> Result<ImportReport, String> {
    let mut source = None;
    let mut asset_override = None;
    let mut options = ImportOptions { floor: false, ..ImportOptions::default() };
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].to_string_lossy().into_owned();
        let mut take = || {
            index += 1;
            args.get(index).map(|value| value.to_string_lossy().into_owned()).ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--import-map" => source = Some(PathBuf::from(take()?)),
            "--scale" => options.scale = take()?.parse().map_err(|_| "Scale must be a number")?,
            "--collision" => options.collision = take()?,
            "--floor" => options.floor = true,
            "--heading" => options.heading = take()?.parse().map_err(|_| "Heading must be degrees")?,
            "--max-texture" => options.max_texture = take()?.parse().map_err(|_| "Texture size must be a number of pixels")?,
            "--surface" => options.surface = take()?.parse().map_err(|_| "Surface must be a number from 0 to 13")?,
            "--name" => options.name = take()?,
            "--spawn" => options.spawn = Some(parse_floats(&take()?, 3)?.try_into().unwrap()),
            "--crop" => options.crop = Some(parse_floats(&take()?, 4)?.try_into().unwrap()),
            "--assets" => asset_override = Some(PathBuf::from(take()?)),
            _ => return Err(format!("Unknown import argument {arg}")),
        }
        index += 1;
    }
    let source = source.ok_or("--import-map needs an FBX file")?;
    let asset_root = match asset_override {
        Some(path) => path.canonicalize().map_err(|error| format!("{}: {error}", path.display()))?,
        None => crate::setup::asset_root()?,
    };
    run_import(&source, &asset_root, options)
}

fn parse_floats(value: &str, count: usize) -> Result<Vec<f32>, String> {
    let numbers: Vec<f32> = value.split(',').map(|part| part.trim().parse()).collect::<Result<_, _>>()
        .map_err(|_| format!("{value:?} must be {count} numbers separated by commas"))?;
    if numbers.len() != count || numbers.iter().any(|number| !number.is_finite()) {
        return Err(format!("{value:?} must be {count} finite numbers separated by commas"));
    }
    Ok(numbers)
}
