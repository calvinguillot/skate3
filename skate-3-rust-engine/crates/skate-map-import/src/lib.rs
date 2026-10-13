//! Import a binary FBX file as a portable `.skate` map.
//!
//! [`import_file`] reads the FBX, writes `<output_dir>/<name>.skate`, and saves
//! the options beside it as `<name>.import.json`. The package is parsed back
//! and checked with [`skate_data::skate_map::SkateMap::validate_runtime`].
//! A later game command, menu, or file drop should call [`import_file`] and
//! pass the installation's `maps` folder as the output directory.
mod convert;
mod fbx;
mod scene;

pub use convert::{convert_bytes, import_file, load_rails, ImportOptions, ImportReport};
