# Cast Pack Format

A filesystem-based format for authoring and exchanging Director cast libraries.
Each cast is a directory. Members are pairs of files — a YAML metadata file and
an optional media file — that can be committed to a repository and built into a
Director-compatible cast via a terminal command.

---

## Goals

- Human-readable and diff-friendly (YAML + standard media formats)
- Reconstructible — enough information to fully rebuild a cast library
- Authorable — can be written by hand, not just exported from a running movie
- Repository-friendly — no binary blobs except actual media assets

---

## Directory structure

```
my-cast/
  _cast.yml             # cast-level metadata (optional)
  Background.yml
  Background.png
  Intro Music.yml
  Intro Music.wav
  Main Script.yml
  Main Script.ls
  Title Text.yml
  Title Text.rtf
  Rollover Button.yml
  Rollover Button.txt
  Palette.yml
  Palette.act
  Logo.yml              # flash member
  Logo.swf
  Scene.yml             # shockwave 3d member
  Scene.w3d
```

The directory name is the cast library name. If the canonical name differs from
what the directory name encodes (e.g., due to sanitization), set `name:` in
`_cast.yml`.

---

## `_cast.yml` — cast library metadata

```yaml
name: "My Cast"         # optional; overrides directory name as canonical cast name
default_palette: systemMac
```

`_cast.yml` is reserved and must not be used as a member name. Any file whose
stem begins with `_` is treated as cast-level metadata, not a cast member.

---

## Member files

Each cast member is represented by:

- **`<Name>.yml`** — always present; contains the member type and all metadata
- **`<Name>.<ext>`** — optional media file; extension depends on member type

The filename stem (without extension) is the canonical member name. Both files
for the same member share the same stem.

### Filename sanitization

Director member names may contain characters that are illegal on some
filesystems (`/`, `:`, `*`, `?`, `\`, `"`, `<`, `>`, `|` on Windows). When
exporting, replace each illegal character with `_`. If the sanitized name
differs from the original, add a `name:` field to the YAML:

```yaml
type: bitmap
name: "Frame: Title"    # canonical name; filename is "Frame_ Title.yml"
```

When importing, `name:` in the YAML always takes precedence over the filename
stem as the member name.

### Slot assignment

Members are assigned to numbered slots in a cast library. Slots are specified
via the `slot:` field in the YAML:

```yaml
slot: 5   # place this member in slot 5
```

Rules:

1. If `slot:` is present, the member occupies that exact slot.
2. If `slot:` is absent, the member is assigned the next available slot after
   all explicitly-slotted members, in ascending alphabetical order of filename.
3. Gaps between explicit slot numbers are preserved as empty slots.
4. Slot numbers start at 1. Duplicate slot numbers within one cast are an error.

### Cross-references

Members that reference other members (e.g. a bitmap referencing a palette) do
so **by name**, not by slot number. The build tool resolves names to slot
numbers at build time.

Cross-cast references use the form `"CastName:MemberName"`:

```yaml
image:
  palette: "Shared Assets:System Palette"
```

System palette names are reserved strings (see [Palette reference](#palette)).

### Color notation

Colors appear throughout member metadata. All of the following are valid:

```yaml
color: "#ff0000"          # hex RGB string (preferred)
color: [255, 0, 0]        # RGB array
color: [255, 0, 0, 128]   # RGBA array (alpha 0–255)
```

Palette-indexed colors (for 8-bit bitmap members) use a plain integer:

```yaml
shape:
  fore_color: 0    # palette index 0
  back_color: 255  # palette index 255
```

---

## YAML structure

Each member YAML has a flat **header** with identity fields, followed by one or
more **sections** that group related properties under a named key.

```yaml
# Header — identity fields common to all members
type: <member-type>
slot: <integer>          # optional; auto-assigned if absent
name: <string>           # optional; only present when filename stem is sanitized
comments: <string>       # optional; only present when non-empty

# Sections — grouped by concept, vary by type
<section-name>:
  key: value
  ...
```

Sections are only written when they contain at least one field. Optional fields
within a section are omitted when they equal their default value.

---

## Type reference

### `bitmap`

A raster image member.

**Media file:** `.png`

- **32-bit bitmaps** are exported as standard RGBA PNG.
- **Indexed bitmaps** (depth 1/2/4/8) are exported as RGBA PNG where every
  pixel is encoded as `R=G=B=(255-index), A=255` — the palette index inverted
  and stored in all three color channels. The inversion makes the image look
  visually correct for the Mac system palette (index 0 = white, index 255 =
  black) so it can be edited in any image editor including MS Paint. On import,
  the reimporter recovers the index as `255 - R` and must verify that `R == G
  == B` for every pixel; unequal channels mean the image was palette-baked and
  the import must fail with an error. To recolor an indexed bitmap, change the
  `palette:` field in the YAML and leave the PNG unchanged.

```yaml
type: bitmap
slot: 1

image:
  depth: 8                  # bit depth: 1, 2, 4, 8, 16, 32
  palette: systemMac       # system palette name or member name
  reg_point: [0, 0]         # [x, y] registration point
  alpha: false
  center_reg_point: false
  trim_white_space: false
```

**`image:` fields:**

| Field             | Type    | Default      | Description                                 |
|-------------------|---------|--------------|---------------------------------------------|
| `depth`           | integer | `8`          | Bit depth of the original bitmap            |
| `palette`         | string  | `systemMac` | Palette: system name or member name         |
| `reg_point`       | [x, y]  | `[0, 0]`     | Registration point relative to image origin |
| `alpha`           | boolean | `false`      | Whether the image has an alpha channel      |
| `center_reg_point`| boolean | `false`      | If true, reg point is image center          |
| `trim_white_space`| boolean | `false`      | Director's trim-white-space flag            |

---

### `sound`

An audio member.

**Media file:** `.wav` or `.aiff` — the original encoding is preserved on
export. Use either format on import; the importer detects by extension.

```yaml
type: sound
slot: 2

audio:
  loop: false
```

**`audio:` fields:**

| Field  | Type    | Default | Description                    |
|--------|---------|---------|--------------------------------|
| `loop` | boolean | `false` | Whether the sound loops        |

---

### `script`

A Lingo script member.

**Media file:** `.ls` — plain text Lingo source code.

```yaml
type: script
slot: 3

script:
  script_type: movie    # movie | score | parent | behavior | unknown
```

**`script:` fields:**

| Field         | Type   | Default  | Description                                        |
|---------------|--------|----------|----------------------------------------------------|
| `script_type` | string | `movie`  | `movie`, `score`, `parent`, `behavior`, or `unknown` |

---

### `text`

A rich text member (Director's Text cast member, supporting styled spans).

**Media file:** `.rtf` when the member contains styled text; `.txt` when plain
text only. The importer detects by extension.

```yaml
type: text
slot: 4

text:
  alignment: left       # left | right | center
  box_type: adjust      # adjust | scroll | fixed
  word_wrap: true
  width: 100
  height: 100
  char_spacing: 0
  tab_stops:
    - type: left        # left | center | right
      position: 100     # pixels from left edge

font:
  name: Arial
  style: [plain]        # list: plain | bold | italic | underline | shadow | condense | extend
  size: 12
  anti_alias: false
  anti_alias_type: AutoAlias  # AutoAlias | GrayScaleAllAlias | SubpixelAllAlias
                              # | GrayscaleLargerThanAlias | NoneAlias
  fixed_line_space: 0
  top_spacing: 0
  bottom_spacing: 0
```

**`text:` fields:**

| Field          | Type       | Default   | Description                          |
|----------------|------------|-----------|--------------------------------------|
| `alignment`    | string     | `left`    | Text alignment                       |
| `box_type`     | string     | `adjust`  | How the box resizes to content       |
| `word_wrap`    | boolean    | `true`    | Enables word wrap                    |
| `width`        | integer    | `100`     | Member width in pixels               |
| `height`       | integer    | `100`     | Member height in pixels              |
| `char_spacing` | integer    | `0`       | Extra character spacing in pixels    |
| `tab_stops`    | [tab_stop] | `[]`      | Custom tab stop list                 |

**`font:` fields:**

| Field             | Type     | Default      | Description                        |
|-------------------|----------|--------------|------------------------------------|
| `name`            | string   | `Arial`      | Font name                          |
| `style`           | [string] | `[plain]`    | Style flags (list)                 |
| `size`            | integer  | `12`         | Font size in points                |
| `anti_alias`      | boolean  | `false`      | Enables anti-aliasing              |
| `anti_alias_type` | string   | `AutoAlias`  | Anti-alias method                  |
| `fixed_line_space`| integer  | `0`          | Fixed line spacing (0 = automatic) |
| `top_spacing`     | integer  | `0`          | Extra spacing above first line     |
| `bottom_spacing`  | integer  | `0`          | Extra spacing below last line      |

---

### `field`

A classic Director field member (unformatted, single-style text).

**Media file:** `.txt` — plain text content.

```yaml
type: field
slot: 5

field:
  box_type: adjust      # adjust | scroll | fixed | limit
  word_wrap: true
  width: 100
  height: 100
  text_height: 100
  auto_tab: false
  editable: false
  border: 0
  margin: 0
  box_drop_shadow: 0
  drop_shadow: 0
  top_spacing: 0
  anti_alias: false

font:
  name: Arial
  style: plain          # single value: plain | bold | italic | underline
                        #   shadow | condense | extend
  size: 12
  alignment: left       # left | right | center
  fixed_line_space: 0
```

**`field:` fields:**

| Field             | Type    | Default   | Description                                    |
|-------------------|---------|-----------|------------------------------------------------|
| `box_type`        | string  | `adjust`  | `adjust`, `scroll`, `fixed`, or `limit`        |
| `word_wrap`       | boolean | `true`    | Enables word wrap                              |
| `width`           | integer | `100`     | Member width in pixels                         |
| `height`          | integer | `100`     | Member height in pixels                        |
| `text_height`     | integer | `100`     | Text area height used for layout calculations  |
| `auto_tab`        | boolean | `false`   | Tab order follows sprite number order          |
| `editable`        | boolean | `false`   | Whether the field is editable at runtime       |
| `border`          | integer | `0`       | Border thickness in pixels                     |
| `margin`          | integer | `0`       | Inner margin in pixels                         |
| `box_drop_shadow` | integer | `0`       | Drop shadow size for the field box             |
| `drop_shadow`     | integer | `0`       | Drop shadow size for the text                  |
| `top_spacing`     | integer | `0`       | Extra spacing above first line                 |
| `anti_alias`      | boolean | `false`   | Enables anti-aliasing                          |

**`font:` fields:**

| Field             | Type    | Default  | Description                        |
|-------------------|---------|----------|------------------------------------|
| `name`            | string  | `Arial`  | Font name                          |
| `style`           | string  | `plain`  | Style flags (single string)        |
| `size`            | integer | `12`     | Font size in points                |
| `alignment`       | string  | `left`   | Text alignment                     |
| `fixed_line_space`| integer | `0`      | Fixed line spacing (0 = automatic) |

---

### `button`

A button member. Shares font properties with `field`.

**Media file:** `.txt` — the button label text.

```yaml
type: button
slot: 6

button:
  button_type: pushButton   # pushButton | checkBox | radioButton
  width: 100
  height: 22
  alignment: center
  word_wrap: false
  auto_tab: false
  border: 0
  margin: 0
  box_drop_shadow: 0
  drop_shadow: 0
  top_spacing: 0
  anti_alias: false

font:
  name: Arial
  style: plain
  size: 12
  fixed_line_space: 0
```

**`button:` fields:**

| Field             | Type    | Default        | Description                              |
|-------------------|---------|----------------|------------------------------------------|
| `button_type`     | string  | `pushButton`   | `pushButton`, `checkBox`, or `radioButton` |
| `width`           | integer | `100`          | Member width in pixels                   |
| `height`          | integer | `22`           | Member height in pixels                  |
| `alignment`       | string  | `center`       | Text alignment                           |
| `word_wrap`       | boolean | `false`        | Enables word wrap                        |
| `auto_tab`        | boolean | `false`        | Tab order follows sprite number order    |
| `border`          | integer | `0`            | Border thickness in pixels               |
| `margin`          | integer | `0`            | Inner margin in pixels                   |
| `box_drop_shadow` | integer | `0`            | Drop shadow size for the field box       |
| `drop_shadow`     | integer | `0`            | Drop shadow size for the text            |
| `top_spacing`     | integer | `0`            | Extra spacing above first line           |
| `anti_alias`      | boolean | `false`        | Enables anti-aliasing                    |

**`font:` fields:** same as `field`.

---

### `shape`

A primitive vector shape (rect, oval, or line). Fully described in YAML; no
media file.

```yaml
type: shape
slot: 7

shape:
  shape_type: rect      # rect | oval | ovalRect | line
  rect: [0, 0, 100, 100]
  pattern: 0
  fore_color: 0         # palette index
  back_color: 255       # palette index
  fill_type: 0
  line_thickness: 1
  line_direction: 0
```

**`shape:` fields:**

| Field            | Type      | Default | Description                               |
|------------------|-----------|---------|-------------------------------------------|
| `shape_type`     | string    | `rect`  | `rect`, `oval`, `ovalRect`, or `line`     |
| `rect`           | [l,t,r,b] | required| Bounding rectangle                        |
| `pattern`        | integer   | `0`     | Fill pattern index                        |
| `fore_color`     | integer   | `0`     | Foreground palette index                  |
| `back_color`     | integer   | `255`   | Background palette index                  |
| `fill_type`      | integer   | `0`     | Fill type flag                            |
| `line_thickness` | integer   | `1`     | Line/border thickness in pixels           |
| `line_direction` | integer   | `0`     | Line direction (for `line` shape type)    |

---

### `vector_shape`

A Bézier vector shape. Fully described in YAML; no media file.

```yaml
type: vector_shape
slot: 8

stroke:
  color: "#000000"
  width: 1.0

fill:
  mode: solid           # none | solid | gradient
  color: "#ffffff"      # fill color (gradient start)
  end_color: "#000000"  # gradient end color
  gradient_type: linear # linear | radial
  scale: 100.0
  direction: 0.0
  offset: [0, 0]
  cycles: 1

shape:
  closed: true
  bg_color: "#ffffff"
  antialias: true
  reg_point: [0, 0]
  center_reg_point: false
  reg_point_vertex: 0
  direct_to_stage: false
  origin_mode: center
  scale_mode: autoSize
  scale: 100.0

vertices:
  - x: 0.0
    y: 0.0
    handle1_x: 0.0     # outgoing control point (relative to vertex)
    handle1_y: 0.0
    handle2_x: 0.0     # incoming control point (relative to vertex)
    handle2_y: 0.0
```

**`stroke:` fields:**

| Field   | Type  | Default    | Description             |
|---------|-------|------------|-------------------------|
| `color` | color | `"#000000"`| Stroke color            |
| `width` | float | `1.0`      | Stroke width in pixels  |

**`fill:` fields:**

| Field          | Type   | Default    | Description                     |
|----------------|--------|------------|---------------------------------|
| `mode`         | string | `solid`    | `none`, `solid`, or `gradient`  |
| `color`        | color  | `"#ffffff"`| Fill color (gradient start)     |
| `end_color`    | color  | `"#000000"`| Gradient end color              |
| `gradient_type`| string | `linear`   | `linear` or `radial`            |
| `scale`        | float  | `100.0`    | Gradient scale percentage       |
| `direction`    | float  | `0.0`      | Gradient direction in degrees   |
| `offset`       | [x, y] | `[0, 0]`  | Gradient origin offset          |
| `cycles`       | integer| `1`        | Number of gradient cycles       |

**`shape:` fields:**

| Field              | Type   | Default    | Description                               |
|--------------------|--------|------------|-------------------------------------------|
| `closed`           | boolean| `true`     | Whether the path is closed                |
| `bg_color`         | color  | `"#ffffff"`| Background color                          |
| `antialias`        | boolean| `true`     | Enables anti-aliasing                     |
| `reg_point`        | [x, y] | `[0, 0]`  | Registration point                        |
| `center_reg_point` | boolean| `false`    | If true, reg point is shape center        |
| `reg_point_vertex` | integer| `0`        | Vertex index to use as reg point          |
| `direct_to_stage`  | boolean| `false`    | Render directly to stage                  |
| `origin_mode`      | string | `center`   | Origin point mode                         |
| `scale_mode`       | string | `autoSize` | Scale mode                                |
| `scale`            | float  | `100.0`    | Shape scale percentage                    |

**`vertices:`** is a top-level sequence (not under a named section) containing
Bézier vertex objects with `x`, `y`, `handle1_x`, `handle1_y`, `handle2_x`,
`handle2_y`.

---

### `film_loop`

An animated sequence of sprites referencing other cast members.

**Media file:** none. The embedded score is encoded as `score:` inside the YAML.
Score format is TBD; initial implementations may store a base64-encoded binary
blob under `score_data:` as a fallback.

```yaml
type: film_loop
slot: 9

film_loop:
  reg_point: [0, 0]
  width: 100
  height: 100
  center: false
  crop: false
  sound: true
  loop: true
  score_data: <base64>    # raw score chunk, pending a structured score sub-format
```

**`film_loop:` fields:**

| Field        | Type    | Default  | Description                            |
|--------------|---------|----------|----------------------------------------|
| `reg_point`  | [x, y]  | `[0, 0]` | Registration point                     |
| `width`      | integer | `0`      | Bounding width                         |
| `height`     | integer | `0`      | Bounding height                        |
| `center`     | boolean | `false`  | Center the loop on its reg point       |
| `crop`       | boolean | `false`  | Crop to bounding rect                  |
| `sound`      | boolean | `true`   | Include sound channels from the score  |
| `loop`       | boolean | `true`   | Loop the animation                     |

---

### `palette`

A 256-entry color palette.

**Media file:** `.act` (Adobe Color Table — 768 bytes of packed RGB triples,
accepted by Photoshop and most palette editors). Alternatively, inline the
colors in YAML when no `.act` file is present:

```yaml
type: palette
slot: 10
colors:           # exactly 256 entries; used only if no .act file is present
  - [0, 0, 0]
  - [255, 255, 255]
  # ...
```

If both `colors:` and a `.act` file are present, the `.act` file takes
precedence.

**System palette names** (used in `image: palette:` cross-references):

| Name             | Description                  |
|------------------|------------------------------|
| `systemMac`      | Macintosh system palette     |
| `systemWin`      | Windows system palette       |
| `systemWinDir4`  | Windows Director 4 palette   |
| `rainbow`        | Director rainbow palette     |
| `grayscale`      | 256-step grayscale           |
| `pastels`        | Director pastels palette     |
| `vivid`          | Director vivid palette       |
| `ntsc`           | NTSC-safe palette            |
| `metallic`       | Director metallic palette    |
| `vga`            | Standard VGA palette         |
| `web216`         | Web-safe 216-color palette   |

---

### `flash`

A Flash (SWF) member.

**Media file:** `.swf`

```yaml
type: flash
slot: 11

flash:
  reg_point: [0, 0]
```

**`flash:` fields:**

| Field       | Type   | Default  | Description          |
|-------------|--------|----------|----------------------|
| `reg_point` | [x, y] | `[0, 0]` | Registration point   |

---

### `font`

A bitmap font member (Bitstream PFR format).

**Media file:** `.pfr`

```yaml
type: font
slot: 12
```

No additional sections.

---

### `shockwave3d`

A Shockwave 3D scene member.

**Media file:** `.w3d`

```yaml
type: shockwave3d
slot: 13

scene:
  loop: false
  duration: 0             # total duration in milliseconds; 0 = unbounded
  animation_enabled: true
  preload: false
  direct_to_stage: false
  reg_point: [0, 0]
  default_rect: [0, 0, 320, 240]

camera:                   # omit entire section to use scene defaults
  position: [0.0, 0.0, 100.0]
  rotation: [0.0, 0.0, 0.0]    # Euler angles in degrees

lighting:                 # omit entire section to use scene defaults
  bg_color: "#000000"
  ambient_color: "#000000"
```

**`scene:` fields:**

| Field               | Type      | Default         | Description                    |
|---------------------|-----------|-----------------|--------------------------------|
| `loop`              | boolean   | `false`         | Loop the animation             |
| `duration`          | integer   | `0`             | Duration in ms (0 = unbounded) |
| `animation_enabled` | boolean   | `true`          | Enable animation playback      |
| `preload`           | boolean   | `false`         | Preload all assets             |
| `direct_to_stage`   | boolean   | `false`         | Bypass compositing             |
| `reg_point`         | [x, y]   | `[0, 0]`        | Registration point             |
| `default_rect`      | [l,t,r,b] | `[0,0,320,240]` | Default display rectangle      |

**`camera:` fields** (entire section omitted when no overrides are set):

| Field      | Type    | Description                           |
|------------|---------|---------------------------------------|
| `position` | [x,y,z] | Initial camera position               |
| `rotation` | [x,y,z] | Initial camera rotation (Euler, deg)  |

**`lighting:` fields** (entire section omitted when no overrides are set):

| Field           | Type  | Description                  |
|-----------------|-------|------------------------------|
| `bg_color`      | color | Background color override    |
| `ambient_color` | color | Ambient light color override |

---

## Media file format summary

| Member type    | Extension        | Format                           |
|----------------|------------------|----------------------------------|
| `bitmap`       | `.png`           | RGBA PNG; indexed bitmaps use R=G=B=index encoding |
| `sound`        | `.wav` / `.aiff` | Original encoding preserved      |
| `script`       | `.ls`            | Plain text (UTF-8)               |
| `text`         | `.rtf` / `.txt`  | RTF for styled; plain for plain  |
| `field`        | `.txt`           | Plain text (UTF-8)               |
| `button`       | `.txt`           | Plain text label (UTF-8)         |
| `palette`      | `.act`           | Adobe Color Table (768 bytes)    |
| `flash`        | `.swf`           | SWF binary                       |
| `font`         | `.pfr`           | Bitstream PFR binary             |
| `shockwave3d`  | `.w3d`           | W3D binary                       |
| `shape`        | *(none)*         | Fully in YAML                    |
| `vector_shape` | *(none)*         | Fully in YAML                    |
| `film_loop`    | *(none)*         | Score embedded in YAML           |
