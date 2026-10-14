"""View raw RGBA8 files, and convert PNG / JPEG / other images to and from that format.

Sky packages under assets/private/native-skies store uncompressed RGBA8 bytes.
Width and height come from the matching JSON (`width`/`height`, or
`sun_width`/`sun_height` for `Name.sun.rgba`). Other raw RGBA8 files can be
opened the same way; if no sidecar matches, pick a size from the file length.

PNG, JPEG, WebP, BMP, and TIFF open as images. Save RGBA writes those pixels
back as raw RGBA8 (JPEG becomes opaque alpha). A small JSON sidecar is written
when the raw file does not already have one, so it opens at the right size.

    python rgba_viewer.py
    python rgba_viewer.py path\\to\\texture.rgba
    python rgba_viewer.py path\\to\\photo.png
"""

from __future__ import annotations

import json
import math
import re
import sys
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

try:
    from PIL import Image, ImageOps, ImageTk
except ImportError:
    root = tk.Tk()
    root.withdraw()
    messagebox.showerror(
        "RGBA Viewer",
        "Pillow is required.\n\npython -m pip install pillow",
    )
    raise SystemExit(1)


SCRIPT_DIR = Path(__file__).resolve().parent
SAVE_TYPES = [
    ("Raw RGBA", "*.rgba"),
    ("PNG", "*.png"),
    ("JPEG", "*.jpg"),
    ("WebP", "*.webp"),
    ("TIFF", "*.tif"),
    ("BMP", "*.bmp"),
]
OPEN_TYPES = [
    ("Images", "*.rgba *.png *.jpg *.jpeg *.webp *.bmp *.tif *.tiff"),
    ("RGBA", "*.rgba"),
    ("PNG", "*.png"),
    ("JPEG", "*.jpg *.jpeg"),
    ("WebP", "*.webp"),
    ("All files", "*.*"),
]
VIEW_SUFFIXES = {".rgba", ".png", ".jpg", ".jpeg", ".webp", ".bmp", ".tif", ".tiff"}
ALPHA_FORMATS = {".png", ".webp", ".tif", ".tiff"}
KIND_NAMES = {
    ".jpg": "JPEG",
    ".jpeg": "JPEG",
    ".tif": "TIFF",
    ".tiff": "TIFF",
}
ZOOM_MIN = 0.02
ZOOM_MAX = 32.0


def default_sky_dir(start: Path | None = None) -> Path | None:
    """Find an installed native-skies folder next to this script."""
    root = start or SCRIPT_DIR
    direct = [
        root / "assets" / "private" / "native-skies",
        root / "skate-3-rust-engine" / "assets" / "private" / "native-skies",
    ]
    for path in direct:
        if path.is_dir():
            return path
    installs = root / "skate3rust-windows-x64" / "data" / "installations"
    if not installs.is_dir():
        return None
    found = [
        child / "assets" / "private" / "native-skies"
        for child in installs.iterdir()
        if (child / "assets" / "private" / "native-skies").is_dir()
    ]
    if not found:
        return None
    return max(found, key=lambda path: path.stat().st_mtime)


def _as_int(value: object) -> int | None:
    if isinstance(value, bool) or value is None:
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, float) and value.is_integer():
        return int(value)
    return None


def _matching_pair(data: dict, pixels: int, keys: tuple[str, str]) -> tuple[int, int] | None:
    width = _as_int(data.get(keys[0]))
    height = _as_int(data.get(keys[1]))
    if width and height and width > 0 and height > 0 and width * height == pixels:
        return width, height
    return None


def sidecar_size(path: Path, pixels: int) -> tuple[int, int, str] | None:
    """Read dimensions from the JSON that belongs to this RGBA file."""
    lookups: list[tuple[Path, tuple[str, str]]] = []
    if path.name.endswith(".sun.rgba"):
        base = path.name[: -len(".sun.rgba")]
        lookups.append((path.with_name(base + ".json"), ("sun_width", "sun_height")))
    lookups.append((path.with_suffix(".json"), ("width", "height")))
    for json_path, keys in lookups:
        if not json_path.is_file():
            continue
        try:
            data = json.loads(json_path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError):
            continue
        if not isinstance(data, dict):
            continue
        pair = _matching_pair(data, pixels, keys)
        if pair:
            return pair[0], pair[1], f"from {json_path.name}"
    return None


def candidate_sizes(pixels: int) -> list[tuple[int, int]]:
    """Factor pairs that can be a width and height, most texture-like first."""
    if pixels <= 0:
        return []
    root = int(math.isqrt(pixels))
    found: list[tuple[int, int]] = []
    for height in range(1, root + 1):
        if pixels % height:
            continue
        width = pixels // height
        found.append((width, height))
        if width != height:
            found.append((height, width))

    def rank(size: tuple[int, int]) -> tuple:
        width, height = size
        power_of_two = int(width & (width - 1) == 0 and height & (height - 1) == 0)
        sane = int(width >= 8 and height >= 8)
        landscape = int(width >= height)
        aspect = width / height
        extreme = int(not 0.0625 <= aspect <= 16)
        return (-sane, -power_of_two, extreme, -landscape, abs(math.log2(aspect)), -width)

    found.sort(key=rank)
    unique: list[tuple[int, int]] = []
    seen: set[tuple[int, int]] = set()
    for size in found:
        if size in seen:
            continue
        seen.add(size)
        unique.append(size)
        if len(unique) == 24:
            break
    return unique


def load_rgba(
    path: Path, size: tuple[int, int] | None = None
) -> tuple[Image.Image, str]:
    """Decode a raw RGBA8 file. `size` overrides the sidecar when given."""
    data = path.read_bytes()
    if len(data) < 4 or len(data) % 4:
        raise ValueError(f"{path.name} is {len(data)} bytes, not a multiple of 4 (RGBA8).")
    pixels = len(data) // 4
    source = "chosen size"
    if size is None:
        sidecar = sidecar_size(path, pixels)
        if sidecar:
            size = sidecar[0], sidecar[1]
            source = sidecar[2]
        else:
            guesses = candidate_sizes(pixels)
            if not guesses:
                raise ValueError(f"Cannot infer a size for {pixels} pixels.")
            size = guesses[0]
            source = "guessed from file length"
    width, height = size
    if width <= 0 or height <= 0 or width * height != pixels:
        raise ValueError(
            f"{width}×{height} does not match {len(data)} bytes "
            f"({pixels} pixels)."
        )
    image = Image.frombytes("RGBA", (width, height), data)
    return image, source


def load_image(
    path: Path, size: tuple[int, int] | None = None
) -> tuple[Image.Image, str]:
    """Load a raw RGBA8 file or a normal image (PNG, JPEG, and similar)."""
    if path.suffix.lower() == ".rgba":
        return load_rgba(path, size)
    with Image.open(path) as opened:
        oriented = ImageOps.exif_transpose(opened) or opened
        image = oriented.convert("RGBA")
        image.load()
        image = image.copy()
    kind = KIND_NAMES.get(path.suffix.lower(), path.suffix.lstrip(".").upper())
    return image, kind


def sync_sidecar(dest: Path, width: int, height: int) -> str:
    """Remember width and height for a raw file.

    An existing sky JSON keeps every other field. Only the integer dimension
    keys change, and only when the new size differs. A new raw file gets a
    small JSON file so the viewer can open it again.
    """
    sun = dest.name.lower().endswith(".sun.rgba")
    if sun:
        base = dest.name[: -len(".sun.rgba")]
        sky_json = dest.with_name(base + ".json")
        if sky_json.is_file():
            return _update_dimension_keys(sky_json, ("sun_width", "sun_height"), width, height)
    own = dest.with_suffix(".json")
    if own.is_file():
        return _update_dimension_keys(own, ("width", "height"), width, height)
    own.write_text(
        json.dumps({"width": width, "height": height}, indent=2) + "\n",
        encoding="utf-8",
    )
    return f"wrote {own.name}"


def _update_dimension_keys(
    path: Path, keys: tuple[str, str], width: int, height: int
) -> str:
    text = path.read_text(encoding="utf-8")
    try:
        data = json.loads(text)
    except json.JSONDecodeError:
        return f"{path.name} is not JSON, left unchanged"
    if not isinstance(data, dict) or any(key not in data for key in keys):
        return f"{path.name} has no {keys[0]}/{keys[1]}, left unchanged"
    if _as_int(data[keys[0]]) == width and _as_int(data[keys[1]]) == height:
        return ""
    updated = text
    for key, value in ((keys[0], width), (keys[1], height)):
        pattern = rf'("{re.escape(key)}"\s*:\s*)-?\d+(?:\.\d+)?'
        updated, count = re.subn(pattern, rf"\g<1>{value}", updated, count=1)
        if count != 1:
            return f"could not update {path.name}"
    path.write_text(updated, encoding="utf-8")
    return f"updated {path.name} to {width}×{height}"


def flatten(image: Image.Image, background: str) -> Image.Image:
    """Composite RGBA onto a background for formats that have no alpha."""
    if background == "White":
        base = Image.new("RGBA", image.size, (255, 255, 255, 255))
    elif background == "Checker":
        base = checkerboard(*image.size).convert("RGBA")
    else:
        base = Image.new("RGBA", image.size, (0, 0, 0, 255))
    base.alpha_composite(image)
    return base.convert("RGB")


def checkerboard(width: int, height: int, cell: int = 16) -> Image.Image:
    cols = max(1, (width + cell - 1) // cell)
    rows = max(1, (height + cell - 1) // cell)
    small = Image.new("RGB", (cols, rows))
    pixels = small.load()
    for y in range(rows):
        for x in range(cols):
            pixels[x, y] = (58, 58, 58) if (x + y) & 1 else (32, 32, 32)
    return small.resize((width, height), Image.Resampling.NEAREST)


def format_size(num_bytes: int) -> str:
    if num_bytes < 1024:
        return f"{num_bytes} B"
    if num_bytes < 1024 * 1024:
        return f"{num_bytes / 1024:.1f} KB"
    return f"{num_bytes / (1024 * 1024):.1f} MB"


class Viewer(tk.Tk):
    def __init__(self) -> None:
        super().__init__()
        self.title("RGBA Viewer")
        self.geometry("1100x720")
        self.minsize(720, 480)

        self.directory: Path | None = None
        self.files: list[Path] = []
        self.path: Path | None = None
        self.image: Image.Image | None = None
        self.source = ""
        self.zoom = 1.0
        self.pan_x = 0.0
        self.pan_y = 0.0
        self._photo: ImageTk.PhotoImage | None = None
        self._drag: tuple[int, int, float, float] | None = None
        self._suppress_list = False

        self.background = tk.StringVar(value="Checker")
        self.size_text = tk.StringVar()
        self.status = tk.StringVar(value="Open an .rgba, .png, or .jpg file.")

        self._build()
        self._bind_keys()

    def _build(self) -> None:
        bar = ttk.Frame(self, padding=(8, 6))
        bar.pack(side=tk.TOP, fill=tk.X)
        ttk.Button(bar, text="Open…", command=self.open_dialog).pack(side=tk.LEFT)
        ttk.Button(bar, text="Open folder…", command=self.open_folder_dialog).pack(
            side=tk.LEFT, padx=(6, 0)
        )
        ttk.Button(bar, text="Save as…", command=self.save_dialog).pack(
            side=tk.LEFT, padx=(6, 0)
        )
        ttk.Button(bar, text="Save RGBA…", command=self.save_rgba_dialog).pack(
            side=tk.LEFT, padx=(6, 12)
        )
        ttk.Button(bar, text="Fit", command=self.fit).pack(side=tk.LEFT)
        ttk.Button(bar, text="1:1", command=self.actual_size).pack(side=tk.LEFT, padx=(6, 12))
        ttk.Label(bar, text="Size").pack(side=tk.LEFT)
        self.size_box = ttk.Combobox(bar, textvariable=self.size_text, width=28)
        self.size_box.pack(side=tk.LEFT, padx=(6, 12))
        self.size_box.bind("<<ComboboxSelected>>", lambda _e: self.apply_size())
        self.size_box.bind("<Return>", lambda _e: self.apply_size())
        ttk.Label(bar, text="Background").pack(side=tk.LEFT)
        bg = ttk.Combobox(
            bar,
            textvariable=self.background,
            values=("Checker", "Black", "White"),
            width=10,
            state="readonly",
        )
        bg.pack(side=tk.LEFT, padx=(6, 0))
        bg.bind("<<ComboboxSelected>>", lambda _e: self.redraw())

        pane = ttk.Panedwindow(self, orient=tk.HORIZONTAL)
        pane.pack(side=tk.TOP, fill=tk.BOTH, expand=True)

        side = ttk.Frame(pane, padding=(8, 0, 4, 8))
        pane.add(side, weight=0)
        ttk.Label(side, text="Images").pack(anchor=tk.W)
        list_frame = ttk.Frame(side)
        list_frame.pack(fill=tk.BOTH, expand=True, pady=(4, 0))
        scroll = ttk.Scrollbar(list_frame, orient=tk.VERTICAL)
        self.file_list = tk.Listbox(
            list_frame,
            width=28,
            exportselection=False,
            activestyle="dotbox",
            yscrollcommand=scroll.set,
        )
        scroll.config(command=self.file_list.yview)
        scroll.pack(side=tk.RIGHT, fill=tk.Y)
        self.file_list.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        self.file_list.bind("<<ListboxSelect>>", self._on_list_select)

        self.canvas = tk.Canvas(pane, bg="#1c1c1c", highlightthickness=0, cursor="fleur")
        pane.add(self.canvas, weight=1)
        self.canvas.bind("<ButtonPress-1>", self._on_press)
        self.canvas.bind("<B1-Motion>", self._on_drag)
        self.canvas.bind("<ButtonRelease-1>", lambda _e: setattr(self, "_drag", None))
        self.canvas.bind("<MouseWheel>", self._on_wheel)
        self.canvas.bind("<Motion>", self._on_motion)
        self.canvas.bind("<Configure>", lambda _e: self.redraw())

        ttk.Label(self, textvariable=self.status, padding=(8, 4), anchor=tk.W).pack(
            side=tk.BOTTOM, fill=tk.X
        )

    def _bind_keys(self) -> None:
        self.bind("<Control-o>", lambda _e: self.open_dialog())
        self.bind("<Control-s>", lambda _e: self.save_dialog())
        self.bind("<Left>", lambda e: self._nudge_file(e, -1))
        self.bind("<Right>", lambda e: self._nudge_file(e, 1))
        self.bind("<plus>", lambda _e: self._zoom_step(1.25))
        self.bind("<minus>", lambda _e: self._zoom_step(1 / 1.25))
        self.bind("<equal>", lambda _e: self._zoom_step(1.25))
        self.bind("f", lambda e: self._if_not_typing(e, self.fit))
        self.bind("<Escape>", lambda _e: self.destroy())

    def open_dialog(self) -> None:
        initial = self.directory or default_sky_dir() or SCRIPT_DIR
        chosen = filedialog.askopenfilename(
            parent=self,
            title="Open image",
            initialdir=str(initial),
            filetypes=OPEN_TYPES,
        )
        if chosen:
            self.open_path(Path(chosen))

    def open_folder_dialog(self) -> None:
        initial = self.directory or default_sky_dir() or SCRIPT_DIR
        chosen = filedialog.askdirectory(parent=self, title="Open folder", initialdir=str(initial))
        if chosen:
            self.open_path(Path(chosen))

    def open_path(self, path: Path) -> None:
        path = path.expanduser().resolve()
        if path.is_dir():
            self._set_directory(path)
            panoramas = [
                item
                for item in self.files
                if item.suffix.lower() == ".rgba" and not item.name.lower().endswith(".sun.rgba")
            ]
            self.load_file(panoramas[0] if panoramas else (self.files[0] if self.files else None))
            return
        if not path.is_file():
            messagebox.showerror("RGBA Viewer", f"Not a file:\n{path}", parent=self)
            return
        self._set_directory(path.parent)
        self.load_file(path)

    def _set_directory(self, directory: Path) -> None:
        self.directory = directory
        self.files = sorted(
            (
                item
                for item in directory.iterdir()
                if item.is_file() and item.suffix.lower() in VIEW_SUFFIXES
            ),
            key=lambda item: item.name.lower(),
        )
        self._suppress_list = True
        try:
            self.file_list.delete(0, tk.END)
            for item in self.files:
                self.file_list.insert(tk.END, item.name)
        finally:
            self._suppress_list = False

    def load_file(self, path: Path | None, size: tuple[int, int] | None = None) -> None:
        if path is None:
            self.image = None
            self.path = None
            self.status.set("This folder has no images.")
            self.redraw()
            return
        raw = path.suffix.lower() == ".rgba"
        try:
            image, source = load_image(path, size if raw else None)
        except (OSError, ValueError) as error:
            messagebox.showerror("RGBA Viewer", str(error), parent=self)
            return
        self.path = path
        self.image = image
        self.source = source
        self._fill_sizes(
            image.width * image.height,
            (image.width, image.height),
            source,
            editable=raw,
        )
        self._select_in_list(path)
        self.title(f"RGBA Viewer — {path.name}")
        self.after_idle(self._fit_when_ready)
        self._set_status()

    def _fill_sizes(
        self,
        pixels: int,
        current: tuple[int, int],
        source: str,
        editable: bool,
    ) -> None:
        self.size_box.configure(state="normal")
        options: list[str] = []
        label = f"{current[0]} × {current[1]}"
        if source not in {"chosen size", "guessed from file length"}:
            label = f"{label}  ({source})"
        options.append(label)
        if editable:
            for width, height in candidate_sizes(pixels):
                text = f"{width} × {height}"
                if (width, height) != current:
                    options.append(text)
        self.size_box["values"] = options
        self.size_text.set(options[0])
        if not editable:
            self.size_box.configure(state="disabled")

    def apply_size(self) -> None:
        if self.path is None or self.image is None or self.path.suffix.lower() != ".rgba":
            return
        pixels = self.image.width * self.image.height
        try:
            size = parse_size(self.size_text.get(), pixels)
        except ValueError as error:
            messagebox.showerror("RGBA Viewer", str(error), parent=self)
            return
        if size == (self.image.width, self.image.height):
            return
        self.load_file(self.path, size)

    def _select_in_list(self, path: Path) -> None:
        if path not in self.files:
            return
        index = self.files.index(path)
        self._suppress_list = True
        try:
            self.file_list.selection_clear(0, tk.END)
            self.file_list.selection_set(index)
            self.file_list.activate(index)
            self.file_list.see(index)
        finally:
            self._suppress_list = False

    def _on_list_select(self, _event: object = None) -> None:
        if self._suppress_list:
            return
        selected = self.file_list.curselection()
        if not selected:
            return
        path = self.files[selected[0]]
        if path != self.path:
            self.load_file(path)

    def _nudge_file(self, event: tk.Event, step: int) -> None:
        if self._typing(event) or not self.files or self.path is None:
            return
        index = self.files.index(self.path) if self.path in self.files else 0
        index = (index + step) % len(self.files)
        self.load_file(self.files[index])

    def save_dialog(self) -> None:
        if self.image is None or self.path is None:
            messagebox.showinfo("RGBA Viewer", "Open an image first.", parent=self)
            return
        raw = self.path.suffix.lower() == ".rgba"
        chosen = filedialog.asksaveasfilename(
            parent=self,
            title="Save image",
            initialdir=str(self.path.parent),
            initialfile=self.path.stem,
            defaultextension=".png" if raw else ".rgba",
            filetypes=SAVE_TYPES,
        )
        if chosen:
            self._save_to(Path(chosen))

    def save_rgba_dialog(self) -> None:
        if self.image is None or self.path is None:
            messagebox.showinfo("RGBA Viewer", "Open an image first.", parent=self)
            return
        chosen = filedialog.asksaveasfilename(
            parent=self,
            title="Save raw RGBA",
            initialdir=str(self.path.parent),
            initialfile=self.path.stem,
            defaultextension=".rgba",
            filetypes=[("Raw RGBA", "*.rgba")],
        )
        if not chosen:
            return
        dest = Path(chosen)
        if dest.suffix.lower() != ".rgba":
            dest = dest.with_suffix(".rgba")
        self._save_to(dest)

    def _save_to(self, dest: Path) -> None:
        if self.image is None:
            return
        suffix = dest.suffix.lower()
        try:
            if suffix == ".rgba":
                note = self._write_rgba(dest)
            elif suffix in ALPHA_FORMATS:
                self.image.save(dest)
                note = "alpha kept"
            elif suffix in {".jpg", ".jpeg", ".bmp"}:
                flat = flatten(self.image, self.background.get())
                quality = {"quality": 95} if suffix in {".jpg", ".jpeg"} else {}
                flat.save(dest, **quality)
                note = f"alpha flattened on {self.background.get().lower()}"
            else:
                raise ValueError(f"Unsupported format {suffix or '(none)'}. Use .rgba, .png, or .jpg.")
        except (OSError, ValueError) as error:
            messagebox.showerror("RGBA Viewer", f"Could not save:\n{error}", parent=self)
            return
        if self.directory and dest.parent == self.directory:
            current = self.path
            self._set_directory(self.directory)
            if current is not None:
                self._select_in_list(current)
        self.status.set(f"Saved {dest.name} ({note})")

    def _write_rgba(self, dest: Path) -> str:
        rgba = self.image.convert("RGBA") if self.image is not None else None
        if rgba is None:
            raise ValueError("No image is open.")
        dest.write_bytes(rgba.tobytes())
        note = f"raw RGBA8 {rgba.width}×{rgba.height}"
        extra = sync_sidecar(dest, rgba.width, rgba.height)
        if extra:
            note = f"{note}; {extra}"
        if dest.name.lower().endswith(".sun.rgba") and rgba.height != 16:
            note += "; game sun gradients are 16 px tall"
        return note

    def fit(self) -> None:
        if self.image is None:
            return
        width = max(1, self.canvas.winfo_width())
        height = max(1, self.canvas.winfo_height())
        self.zoom = min(width / self.image.width, height / self.image.height)
        self.zoom = min(ZOOM_MAX, max(ZOOM_MIN, self.zoom))
        self.pan_x = (width - self.image.width * self.zoom) / 2
        self.pan_y = (height - self.image.height * self.zoom) / 2
        self.redraw()

    def _fit_when_ready(self) -> None:
        if self.canvas.winfo_width() < 2:
            self.after(30, self._fit_when_ready)
            return
        self.fit()

    def actual_size(self) -> None:
        if self.image is None:
            return
        width = max(1, self.canvas.winfo_width())
        height = max(1, self.canvas.winfo_height())
        self.zoom = 1.0
        self.pan_x = (width - self.image.width) / 2
        self.pan_y = (height - self.image.height) / 2
        self.redraw()

    def _zoom_step(self, factor: float) -> None:
        if self.image is None:
            return
        width = max(1, self.canvas.winfo_width())
        height = max(1, self.canvas.winfo_height())
        self._zoom_at(width / 2, height / 2, factor)

    def _on_wheel(self, event: tk.Event) -> None:
        if self.image is None:
            return
        self._zoom_at(event.x, event.y, 1.1 if event.delta > 0 else 1 / 1.1)

    def _zoom_at(self, x: float, y: float, factor: float) -> None:
        if self.image is None:
            return
        image_x = (x - self.pan_x) / self.zoom
        image_y = (y - self.pan_y) / self.zoom
        self.zoom = min(ZOOM_MAX, max(ZOOM_MIN, self.zoom * factor))
        self.pan_x = x - image_x * self.zoom
        self.pan_y = y - image_y * self.zoom
        self.redraw()

    def _on_press(self, event: tk.Event) -> None:
        self._drag = (event.x, event.y, self.pan_x, self.pan_y)

    def _on_drag(self, event: tk.Event) -> None:
        if self._drag is None:
            return
        origin_x, origin_y, pan_x, pan_y = self._drag
        self.pan_x = pan_x + (event.x - origin_x)
        self.pan_y = pan_y + (event.y - origin_y)
        self.redraw()

    def _on_motion(self, event: tk.Event) -> None:
        self._hover = (event.x, event.y)
        if self._drag is None:
            self._set_status()

    def redraw(self) -> None:
        self.canvas.delete("all")
        if self.image is None:
            return
        view_w = max(1, self.canvas.winfo_width())
        view_h = max(1, self.canvas.winfo_height())
        left = max(0, math.floor((0 - self.pan_x) / self.zoom))
        top = max(0, math.floor((0 - self.pan_y) / self.zoom))
        right = min(self.image.width, math.ceil((view_w - self.pan_x) / self.zoom))
        bottom = min(self.image.height, math.ceil((view_h - self.pan_y) / self.zoom))
        if right <= left or bottom <= top:
            self._set_status()
            return
        crop = self.image.crop((left, top, right, bottom))
        disp_w = max(1, round((right - left) * self.zoom))
        disp_h = max(1, round((bottom - top) * self.zoom))
        if disp_w > 8192 or disp_h > 8192:
            return
        resized = crop.resize(
            (disp_w, disp_h),
            Image.Resampling.NEAREST if self.zoom >= 3 else Image.Resampling.BILINEAR,
        )
        preview = self._composite(resized)
        self._photo = ImageTk.PhotoImage(preview)
        self.canvas.create_image(
            self.pan_x + left * self.zoom,
            self.pan_y + top * self.zoom,
            anchor=tk.NW,
            image=self._photo,
        )
        self._set_status()

    def _composite(self, image: Image.Image) -> Image.Image:
        mode = self.background.get()
        if mode == "White":
            base = Image.new("RGBA", image.size, (255, 255, 255, 255))
        elif mode == "Black":
            base = Image.new("RGBA", image.size, (0, 0, 0, 255))
        else:
            base = checkerboard(*image.size).convert("RGBA")
        base.alpha_composite(image)
        return base

    def _set_status(self) -> None:
        if self.path is None or self.image is None:
            return
        pixel = ""
        hover = getattr(self, "_hover", None)
        if hover:
            ix = int((hover[0] - self.pan_x) / self.zoom)
            iy = int((hover[1] - self.pan_y) / self.zoom)
            if 0 <= ix < self.image.width and 0 <= iy < self.image.height:
                r, g, b, a = self.image.getpixel((ix, iy))
                pixel = f"    ({ix}, {iy})  rgba({r}, {g}, {b}, {a})"
        self.status.set(
            f"{self.path.name}    {self.image.width}×{self.image.height}    "
            f"{format_size(self.path.stat().st_size)}    {self.source}    "
            f"{self.zoom * 100:.0f}%{pixel}"
        )

    def _typing(self, event: tk.Event) -> bool:
        return isinstance(event.widget, (tk.Entry, ttk.Entry, ttk.Combobox, tk.Text))

    def _if_not_typing(self, event: tk.Event, action) -> None:
        if not self._typing(event):
            action()


def parse_size(text: str, pixels: int) -> tuple[int, int]:
    numbers = [int(part) for part in re.findall(r"\d+", text)]
    if len(numbers) >= 2:
        width, height = numbers[0], numbers[1]
    elif len(numbers) == 1 and numbers[0] > 0 and pixels % numbers[0] == 0:
        width = numbers[0]
        height = pixels // width
    else:
        raise ValueError("Type a size like 2048 × 256, or a width that divides the file.")
    if width <= 0 or height <= 0 or width * height != pixels:
        raise ValueError(
            f"{width}×{height} is {width * height * 4} bytes, but this file is {pixels * 4} bytes."
        )
    return width, height


def main() -> None:
    app = Viewer()
    if len(sys.argv) > 1:
        app.open_path(Path(sys.argv[1]))
    else:
        sky = default_sky_dir()
        if sky is not None:
            app.open_path(sky)
    app.mainloop()


if __name__ == "__main__":
    main()
