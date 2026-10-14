//! Installed map discovery and the default pointer, shared by startup and the menu.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub label: String,
    pub path: Option<PathBuf>,
}

pub(crate) fn discover(assets: &Path) -> Vec<Entry> {
    let mut maps = Vec::new();
    let root = assets.parent().unwrap_or(assets).join("maps");
    for directory in [&root, &root.join("private")] {
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("skate")) {
                    maps.push(Entry {
                        label: path.file_stem().unwrap().to_string_lossy().replace('_', " "),
                        path: Some(path.canonicalize().unwrap_or(path)),
                    });
                }
            }
        }
    }
    maps.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    maps.insert(0, Entry { label: "Test world".into(), path: None });
    maps
}

pub(crate) fn default_map(assets: &Path) -> Result<Option<PathBuf>, String> {
    // Only a completed release installation creates this pointer. Existing
    // development checkouts continue to boot the test world.
    let pointer = assets.parent().unwrap_or(assets).join("settings/default-map.json");
    let bytes = match std::fs::read(&pointer) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", pointer.display())),
    };
    let relative: Option<String> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let Some(relative) = relative else { return Ok(None); };
    let relative = Path::new(&relative);
    if relative.is_absolute() || relative.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return Err("Invalid installed default map path".into());
    }
    Ok(Some(assets.parent().unwrap_or(assets).join(relative)))
}

/// Retail Start Park, `maps/StartPark.skate` (or the same file under `maps/private`).
pub(crate) fn start_park(assets: &Path) -> Option<PathBuf> {
    let root = assets.parent().unwrap_or(assets);
    for relative in ["maps/StartPark.skate", "maps/private/StartPark.skate"] {
        let path = root.join(relative);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Which map this process opens. A refused saved pointer falls back without
/// rewriting `settings/default-map.json`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LaunchTarget {
    Map(PathBuf),
    TestWorld,
}

pub(crate) fn select_launch(
    force_start_park: bool,
    saved: Option<&Path>,
    saved_playable: bool,
    start_park: Option<&Path>,
) -> Result<LaunchTarget, String> {
    if force_start_park {
        return start_park
            .map(|path| LaunchTarget::Map(path.to_path_buf()))
            .ok_or_else(|| "Start Park is not installed (maps/StartPark.skate).".into());
    }
    if let Some(path) = saved.filter(|_| saved_playable) {
        return Ok(LaunchTarget::Map(path.to_path_buf()));
    }
    if saved.is_some() {
        if let Some(path) = start_park {
            return Ok(LaunchTarget::Map(path.to_path_buf()));
        }
    }
    Ok(LaunchTarget::TestWorld)
}

/// Save only a successfully activated map. Null explicitly selects the test world.
pub(crate) fn save_default(assets: &Path, map: Option<&Path>) -> Result<(), String> {
    let root = assets.parent().unwrap_or(assets);
    let relative = map.map(|path| path.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .map_err(|_| "Map is outside this installation; default map unchanged".to_string())).transpose()?;
    let directory = root.join("settings");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let temporary = directory.join("default-map.json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(&relative).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::rename(temporary, directory.join("default-map.json")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_pointer_round_trips_map_and_explicit_test_world() {
        let root = std::env::temp_dir().join(format!("skate-map-pointer-{}", std::process::id()));
        let assets = root.join("assets");
        let map = root.join("maps/example.skate");
        save_default(&assets, Some(&map)).unwrap();
        assert_eq!(default_map(&assets).unwrap(), Some(map));
        save_default(&assets, None).unwrap();
        assert_eq!(default_map(&assets).unwrap(), None);
        std::fs::write(root.join("settings/default-map.json"), br#""../outside.skate""#).unwrap();
        assert!(default_map(&assets).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn safe_launch_selects_start_park_and_skips_an_unplayable_saved_map() {
        let saved = std::path::PathBuf::from("maps/helsinki_kallio_blend.skate");
        let park = std::path::PathBuf::from("maps/StartPark.skate");
        assert_eq!(
            select_launch(true, Some(&saved), true, Some(&park)).unwrap(),
            LaunchTarget::Map(park.clone())
        );
        assert_eq!(
            select_launch(false, Some(&saved), true, Some(&park)).unwrap(),
            LaunchTarget::Map(saved.clone())
        );
        assert_eq!(
            select_launch(false, Some(&saved), false, Some(&park)).unwrap(),
            LaunchTarget::Map(park.clone())
        );
        assert_eq!(
            select_launch(false, Some(&saved), false, None).unwrap(),
            LaunchTarget::TestWorld
        );
        assert_eq!(select_launch(false, None, false, Some(&park)).unwrap(), LaunchTarget::TestWorld);
        assert!(select_launch(true, Some(&saved), true, None).unwrap_err().contains("StartPark.skate"));

        let root = std::env::temp_dir().join(format!("skate-start-park-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let assets = root.join("assets");
        std::fs::create_dir_all(root.join("maps")).unwrap();
        std::fs::write(root.join("maps/StartPark.skate"), b"SKATE").unwrap();
        assert_eq!(start_park(&assets).unwrap(), root.join("maps/StartPark.skate"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
