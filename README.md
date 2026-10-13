# skate3

A local workspace for playing the [Skate 3 Rust Engine](https://github.com/SK8-ENGINE/skate-3-rust-engine) with a keyboard and mouse, and for importing custom FBX maps.

The engine is an unofficial GPLv3 project. It is not affiliated with Electronic Arts. This repository does not grant any rights to EA's game code, data, assets or trademarks. Your converted game files and disc image stay on this machine and are listed in [.gitignore](.gitignore).

## Layout

| Folder | In git | What it is |
| --- | --- | --- |
| `skate-3-rust-engine/` | yes | Engine source, imported as a git subtree of [SK8-ENGINE/skate-3-rust-engine](https://github.com/SK8-ENGINE/skate-3-rust-engine) at commit `2e16697`. |
| `skate3rust-windows-x64/` | no | The working game. Setup has already converted the disc into `data/`. |
| `skate_iso/` | no | The Skate 3 Xbox 360 disc image used by setup. |
| `fbx/` | no | Custom maps and characters, including `laru_LOD_40cm_scale_1.fbx`. |

## Tracking changes

Everything we change is committed in this repository, so `git status` and `git diff` at this folder show the engine changes along with this README.

The engine was absorbed with `git subtree`, which keeps a record of the upstream commit. To pull later engine updates:

```bat
git subtree pull --prefix=skate-3-rust-engine engine-upstream main --squash
```

`engine-upstream` is the remote `https://github.com/SK8-ENGINE/skate-3-rust-engine.git`. A subtree pull that touches a file we have edited stops with a conflict in that file. Resolve it the usual way, then `git add` the file and `git commit`.

The engine is GPLv3. Its `LICENSE` and notices stay in `skate-3-rust-engine/`, and a public copy of this repository has to include the corresponding source of any modified engine build.

## Prerequisites

Checked on this machine:

- Rust 1.99.0 through rustup, toolchain `stable-x86_64-pc-windows-msvc`.
- LLVM 23.1.3 in `C:\Program Files\LLVM`.
- Visual Studio 2022 Community with the x64 C++ tools and Windows SDK 10.0.19041.0.
- Git.

`CMAKE` is set to Visual Studio's CMake, so the SDL3 build does not pick up Strawberry Perl's copy:

```text
C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe
```

Open a new terminal after changing `CMAKE`. The game folder sits next to `skate-3-rust-engine`, which is where `scripts/Resolve-DevAssets.ps1` looks, so `SKATE3_ASSETS` is not needed.

## Build and run

From `skate-3-rust-engine`:

```bat
BUILD.bat
PLAY.bat
```

The first build downloads the Rust dependencies and compiles them, so it needs a network connection and takes a while. `PLAY.bat` launches the game against the converted data in `skate3rust-windows-x64`.

Maloof Money Cup failed to convert during the original setup. The other nine disc maps are installed. Running the game's setup again retries that map.

## Keyboard and mouse

The game plays with a keyboard and mouse. A connected gamepad still works, and the two are combined: buttons add together, and each stick uses whichever input is pushed further.

| Input | Action |
| --- | --- |
| W A S D | Steer and lean |
| Mouse, or arrow keys, or I J K L | Flick-it tricks and manuals |
| Space | Push |
| Left Ctrl | Push, other foot |
| Q | Brake, or step off the board |
| F | Get on or off the board |
| Left mouse button, or Left Shift | Grab |
| Right mouse button, or E | Grab |
| Z / C | Shoulder buttons |
| R | Bail |
| 1 2 3 4 | D-pad |
| Escape | Menu |

Trick flicks need a short movement rather than a tap. Pull the mouse down and back up for an ollie. The arrow keys move the stick gradually so the same gesture works from the keyboard.

Escape opens the menu. Its CONTROLS page sets the mouse sensitivity, inverts the vertical axis, and turns mouse control on or off. The choices are saved in `settings/keyboard.json` next to the game's other settings, under the converted installation, and you can edit that file directly.

The mouse pointer is hidden and locked to the window while you skate, and it comes back while the menu is open.

## Importing an FBX map

`fbx/laru_LOD_40cm_scale_1.fbx` is one mesh of 749,105 vertices and 496,302 triangles, with 501 materials and an embedded PNG on each. It was exported from Blender 4.1.1 in centimetres with an object scale of 100, so its vertex values are metres. At scale 1.0 the map is about 3.04 km by 3.22 km, with 66.5 m between its lowest and highest points. The triangles are large for skating, so buildings and ground will feel coarse. A scale of 0.1 makes it about 304 m by 322 m.

The importer is the `skate-map-import` library in the engine. `import_file` reads a binary FBX and writes a portable SKATE09 `.skate` file (zlib-compressed texture and geometry blocks) plus `<name>.import.json`, so the same options can be reused. It parses the package back and runs the portable runtime checks before anything is written. The game flag `--import-map`, the in-game import page, and dropping an FBX on the window are not wired up yet; they should call `import_file` and pass the installation `maps` folder.

`ImportOptions` is the public control surface:

- `scale` multiplies the real-world size. The map is always recentred so its bounding-box centre is at the horizontal origin and its lowest point is at y = 0.
- `collision` `full` (the default) keeps every non-degenerate triangle. `grid:2` clusters vertices onto a 2 m grid.
- `floor` adds a flat catch plane just under the mesh, extended past the bounds.
- `spawn` is an optional `[x, y, z]` in final map metres. When it is omitted, the importer looks for flat upward ground under the centre, then nearby. `heading` is yaw in radians, the same value the `.skate` file stores.
- `crop` is an optional XZ rectangle `[x0, z0, x1, z1]` in metres after scaling and before recentring. Triangles whose centre falls outside are dropped.
- `max_texture` caps the longest texture edge in pixels. 0 keeps the embedded PNG size.
- `surface` is the physics surface. 1 is smooth and 2 is rough. Audio is concrete (id 3), matching the demo map writer. Mesh edges are not grindable; only authored rails are.

Collision triangles use that physics id, packed the way wheel probes read a surface tag, so a large map does not turn into water by accident. The default collision mesh is the full render mesh. The menu import includes the catch floor. Shift plus Left or Right changes the scale by 0.1.

Grind rails are not detected from the mesh. Put a `<map>.rails.json` next to the FBX to add them, with coordinates in metres after scaling and recentring:

```json
[{ "name": "ledge", "closed": false, "points": [[0, 1, 0], [4, 1, 0]] }]
```

## What stays out of git

[.gitignore](.gitignore) excludes the disc image, the converted game, archives (`zip`, `7z`, `rar`, `iso`) and large art files (`fbx`, `blend`, `glb`, `gltf`). The engine's own `.gitignore` still applies inside `skate-3-rust-engine/` and covers build output and retail data there.
