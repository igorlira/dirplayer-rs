use image::RgbaImage;
use itertools::Itertools;

use crate::{
    director::{
        file::get_variable_multiplier,
        lingo::decompiler::handler::decompile_handler, static_datum::{format_static_datum, static_datum_to_runtime},
    },
    player::{
        DirPlayer, bitmap::bitmap::{Bitmap, BuiltInPalette}, cast_lib::CastLib, cast_member::{
            BitmapMember, ButtonMember, CastMemberType, FieldMember, FilmLoopMember, FlashMember, PaletteMember, ScriptMember, ShapeMember, Shockwave3dMember, SoundMember, TextMember, VectorShapeMember
        }, script::Script
    },
};

pub struct ExportFile {
    pub path: String,
    pub content: FileContent,
}

pub enum FileContent {
    Text(String),
    Binary(Vec<u8>),
}

// ── Section builder ────────────────────────────────────────────────────────────

struct Section {
    name: &'static str,
    lines: Vec<String>,
}

impl Section {
    fn new(name: &'static str) -> Self {
        Self { name, lines: Vec::new() }
    }

    fn push(&mut self, line: impl Into<String>) {
        self.lines.push(format!("  {}", line.into()));
    }

    fn build(&self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }
        let mut s = format!("{}:\n", self.name);
        for line in &self.lines {
            s.push_str(line);
            s.push('\n');
        }
        s
    }
}

// ── Entry point ────────────────────────────────────────────────────────────────

pub fn export_cast(player: &DirPlayer, cast_number: u32) -> Option<(String, Vec<ExportFile>)> {
    let cast = player.movie.cast_manager.get_cast(cast_number).ok()?;

    let mut files: Vec<ExportFile> = Vec::new();

    files.push(ExportFile {
        path: "_cast.yml".to_string(),
        content: FileContent::Text(format!("name: {}\n", yaml_string(&cast.name))),
    });

    let mut member_numbers: Vec<u32> = cast.members.keys().copied().collect();
    member_numbers.sort();

    for number in member_numbers {
        let member = cast.members.get(&number).unwrap();
        let safe_name = sanitize_filename(&member.name);
        let stem = if safe_name.is_empty() {
            format!("member_{number}")
        } else {
            safe_name.clone()
        };

        let name_differs = !member.name.is_empty() && member.name != safe_name;
        let mut header = format!("slot: {number}\n");
        if name_differs {
            header.push_str(&format!("name: {}\n", yaml_string(&member.name)));
        }
        if !member.comments.is_empty() {
            header.push_str(&format!("comments: {}\n", yaml_string(&member.comments)));
        }

        match &member.member_type {
            CastMemberType::Bitmap(bm) => {
                let yml = build_bitmap_yml(&header, bm, cast);
                files.push(text_file(format!("{stem}.yml"), yml));
                if let Some(bitmap) = player.bitmap_manager.get_bitmap(bm.image_ref) {
                    if let Some(png) = encode_png(bitmap) {
                        files.push(binary_file(format!("{stem}.png"), png));
                    }
                }
            }
            CastMemberType::Sound(sm) => {
                files.push(text_file(format!("{stem}.yml"), build_sound_yml(&header, sm)));
                files.push(binary_file(format!("{stem}.wav"), sm.sound.to_wav()));
            }
            CastMemberType::Script(sm) => {
                files.push(text_file(format!("{stem}.yml"), build_script_yml(&header, sm)));
                if let Some(script) = cast.scripts.get(&number) {
                    files.push(text_file(format!("{stem}.ls"), decompile_script(script, cast)));
                }
            }
            CastMemberType::Text(tm) => {
                files.push(text_file(format!("{stem}.yml"), build_text_yml(&header, tm)));
                if !tm.rtf_source.is_empty() {
                    files.push(text_file(format!("{stem}.rtf"), tm.rtf_source.clone()));
                } else {
                    files.push(text_file(format!("{stem}.txt"), tm.text.clone()));
                }
            }
            CastMemberType::Field(fm) => {
                files.push(text_file(format!("{stem}.yml"), build_field_yml(&header, fm)));
                files.push(text_file(format!("{stem}.txt"), fm.text.clone()));
            }
            CastMemberType::Button(bm) => {
                files.push(text_file(format!("{stem}.yml"), build_button_yml(&header, bm)));
                files.push(text_file(format!("{stem}.txt"), bm.field.text.clone()));
            }
            CastMemberType::Shape(sm) => {
                files.push(text_file(format!("{stem}.yml"), build_shape_yml(&header, sm)));
            }
            CastMemberType::VectorShape(vm) => {
                files.push(text_file(format!("{stem}.yml"), build_vector_shape_yml(&header, vm)));
            }
            CastMemberType::Palette(pm) => {
                files.push(text_file(format!("{stem}.yml"), format!("{header}type: palette\n")));
                files.push(binary_file(format!("{stem}.act"), encode_act(pm)));
            }
            CastMemberType::Flash(fm) => {
                files.push(text_file(format!("{stem}.yml"), build_flash_yml(&header, fm)));
                files.push(binary_file(format!("{stem}.swf"), fm.data.clone()));
            }
            CastMemberType::FilmLoop(fl) => {
                files.push(text_file(format!("{stem}.yml"), build_film_loop_yml(&header, fl)));
            }
            CastMemberType::Shockwave3d(s3d) => {
                files.push(text_file(format!("{stem}.yml"), build_shockwave3d_yml(&header, s3d)));
                files.push(binary_file(format!("{stem}.w3d"), s3d.w3d_data.clone()));
            }
            CastMemberType::Font(_) => {
                files.push(text_file(format!("{stem}.yml"), format!("{header}type: font\n")));
            }
            CastMemberType::HavokPhysics(_) | CastMemberType::Unknown => {}
        }
    }

    Some((cast.name.clone(), files))
}

// ── YAML builders ─────────────────────────────────────────────────────────────

fn build_bitmap_yml(header: &str, bm: &BitmapMember, cast: &CastLib) -> String {
    let mut image = Section::new("image");
    image.push(format!("depth: {}", bm.info.bit_depth));
    image.push(format!("palette: {}", palette_name(bm.info.palette_id, cast)));
    image.push(format!("reg_point: [{}, {}]", bm.info.reg_x, bm.info.reg_y));
    image.push(format!("alpha: {}", bm.info.use_alpha));
    image.push(format!("center_reg_point: {}", bm.info.center_reg_point));
    image.push(format!("trim_white_space: {}", bm.info.trim_white_space));
    format!("{header}type: bitmap\n\n{}", image.build())
}

fn build_sound_yml(header: &str, sm: &SoundMember) -> String {
    let mut audio = Section::new("audio");
    audio.push(format!("loop: {}", sm.info.loop_enabled));
    format!("{header}type: sound\n\n{}", audio.build())
}

fn build_script_yml(header: &str, sm: &ScriptMember) -> String {
    use crate::director::enums::ScriptType;
    let type_str = match sm.script_type {
        ScriptType::Movie => "movie",
        ScriptType::Score => "score",
        ScriptType::Member => "behavior",
        ScriptType::Parent => "parent",
        _ => "unknown",
    };
    let mut script = Section::new("script");
    script.push(format!("script_type: {type_str}"));
    format!("{header}type: script\n\n{}", script.build())
}

fn build_text_yml(header: &str, tm: &TextMember) -> String {
    let mut text = Section::new("text");
    text.push(format!("alignment: {}", tm.alignment));
    text.push(format!("box_type: {}", tm.box_type));
    text.push(format!("word_wrap: {}", tm.word_wrap));
    text.push(format!("width: {}", tm.width));
    text.push(format!("height: {}", tm.height));
    text.push(format!("char_spacing: {}", tm.char_spacing));
    if !tm.tab_stops.is_empty() {
        text.push("tab_stops:".to_string());
        for stop in &tm.tab_stops {
            text.push(format!("  - type: {}", stop.tab_type));
            text.push(format!("    position: {}", stop.position));
        }
    }

    let mut font = Section::new("font");
    font.push(format!("name: {}", yaml_string(&tm.font)));
    font.push(format!("style: [{}]", tm.font_style.join(", ")));
    font.push(format!("size: {}", tm.font_size));
    font.push(format!("anti_alias: {}", tm.anti_alias));
    font.push(format!("anti_alias_type: {}", tm.anti_alias_type));
    font.push(format!("fixed_line_space: {}", tm.fixed_line_space));
    font.push(format!("top_spacing: {}", tm.top_spacing));
    font.push(format!("bottom_spacing: {}", tm.bottom_spacing));

    format!("{header}type: text\n\n{}\n{}", text.build(), font.build())
}

fn build_field_yml(header: &str, fm: &FieldMember) -> String {
    let mut field = Section::new("field");
    field.push(format!("box_type: {}", fm.box_type));
    field.push(format!("word_wrap: {}", fm.word_wrap));
    field.push(format!("width: {}", fm.width));
    field.push(format!("height: {}", fm.height));
    field.push(format!("text_height: {}", fm.text_height));
    field.push(format!("auto_tab: {}", fm.auto_tab));
    field.push(format!("editable: {}", fm.editable));
    field.push(format!("border: {}", fm.border));
    field.push(format!("margin: {}", fm.margin));
    field.push(format!("box_drop_shadow: {}", fm.box_drop_shadow));
    field.push(format!("drop_shadow: {}", fm.drop_shadow));
    field.push(format!("top_spacing: {}", fm.top_spacing));
    field.push(format!("anti_alias: {}", fm.anti_alias));

    let mut font = Section::new("font");
    font.push(format!("name: {}", yaml_string(&fm.font)));
    font.push(format!("style: {}", fm.font_style));
    font.push(format!("size: {}", fm.font_size));
    font.push(format!("alignment: {}", fm.alignment));
    font.push(format!("fixed_line_space: {}", fm.fixed_line_space));

    format!("{header}type: field\n\n{}\n{}", field.build(), font.build())
}

fn build_button_yml(header: &str, bm: &ButtonMember) -> String {
    let fm = &bm.field;
    let mut button = Section::new("button");
    button.push(format!("button_type: {}", bm.button_type.symbol_string()));
    button.push(format!("width: {}", fm.width));
    button.push(format!("height: {}", fm.height));
    button.push(format!("alignment: {}", fm.alignment));
    button.push(format!("word_wrap: {}", fm.word_wrap));
    button.push(format!("auto_tab: {}", fm.auto_tab));
    button.push(format!("border: {}", fm.border));
    button.push(format!("margin: {}", fm.margin));
    button.push(format!("box_drop_shadow: {}", fm.box_drop_shadow));
    button.push(format!("drop_shadow: {}", fm.drop_shadow));
    button.push(format!("top_spacing: {}", fm.top_spacing));
    button.push(format!("anti_alias: {}", fm.anti_alias));

    let mut font = Section::new("font");
    font.push(format!("name: {}", yaml_string(&fm.font)));
    font.push(format!("style: {}", fm.font_style));
    font.push(format!("size: {}", fm.font_size));
    font.push(format!("fixed_line_space: {}", fm.fixed_line_space));

    format!("{header}type: button\n\n{}\n{}", button.build(), font.build())
}

fn build_shape_yml(header: &str, sm: &ShapeMember) -> String {
    let info = &sm.shape_info;
    let mut shape = Section::new("shape");
    shape.push(format!("shape_type: {}", shape_type_name(&info.shape_type)));
    shape.push(format!(
        "rect: [{}, {}, {}, {}]",
        info.rect_left, info.rect_top, info.rect_right, info.rect_bottom
    ));
    shape.push(format!("pattern: {}", info.pattern));
    shape.push(format!("fore_color: {}", info.fore_color));
    shape.push(format!("back_color: {}", info.back_color));
    shape.push(format!("fill_type: {}", info.fill_type));
    shape.push(format!("line_thickness: {}", info.line_thickness));
    shape.push(format!("line_direction: {}", info.line_direction));
    format!("{header}type: shape\n\n{}", shape.build())
}

fn build_vector_shape_yml(header: &str, vm: &VectorShapeMember) -> String {
    let mut stroke = Section::new("stroke");
    stroke.push(format!("color: {}", rgb_hex(vm.stroke_color)));
    stroke.push(format!("width: {}", vm.stroke_width));

    let mut fill = Section::new("fill");
    fill.push(format!("mode: {}", fill_mode_name(vm.fill_mode)));
    fill.push(format!("color: {}", rgb_hex(vm.fill_color)));
    fill.push(format!("end_color: {}", rgb_hex(vm.end_color)));
    fill.push(format!("gradient_type: {}", vm.gradient_type));
    fill.push(format!("scale: {}", vm.fill_scale));
    fill.push(format!("direction: {}", vm.fill_direction));
    fill.push(format!("offset: [{}, {}]", vm.fill_offset.0, vm.fill_offset.1));
    fill.push(format!("cycles: {}", vm.fill_cycles));

    let mut shape = Section::new("shape");
    shape.push(format!("closed: {}", vm.closed));
    shape.push(format!("bg_color: {}", rgb_hex(vm.bg_color)));
    shape.push(format!("antialias: {}", vm.antialias));
    shape.push(format!("reg_point: [{}, {}]", vm.reg_point.0, vm.reg_point.1));
    shape.push(format!("center_reg_point: {}", vm.center_reg_point));
    shape.push(format!("reg_point_vertex: {}", vm.reg_point_vertex));
    shape.push(format!("direct_to_stage: {}", vm.direct_to_stage));
    shape.push(format!("origin_mode: {}", vm.origin_mode));
    shape.push(format!("scale_mode: {}", vm.scale_mode));
    shape.push(format!("scale: {}", vm.scale));

    let mut vertices_yaml = String::new();
    if !vm.vertices.is_empty() {
        vertices_yaml.push_str("vertices:\n");
        for v in &vm.vertices {
            vertices_yaml.push_str(&format!(
                "  - x: {}\n    y: {}\n    handle1_x: {}\n    handle1_y: {}\n    handle2_x: {}\n    handle2_y: {}\n",
                v.x, v.y, v.handle1_x, v.handle1_y, v.handle2_x, v.handle2_y
            ));
        }
    }

    format!(
        "{header}type: vector_shape\n\n{}\n{}\n{}\n{}",
        stroke.build(), fill.build(), shape.build(), vertices_yaml
    )
}

fn build_film_loop_yml(header: &str, fl: &FilmLoopMember) -> String {
    let info = &fl.info;
    let mut section = Section::new("film_loop");
    section.push(format!("reg_point: [{}, {}]", info.reg_point.0, info.reg_point.1));
    section.push(format!("width: {}", info.width));
    section.push(format!("height: {}", info.height));
    section.push(format!("center: {}", info.center != 0));
    section.push(format!("crop: {}", info.crop != 0));
    section.push(format!("sound: {}", info.sound != 0));
    section.push(format!("loop: {}", info.loops != 0));
    format!("{header}type: film_loop\n\n{}", section.build())
}

fn build_flash_yml(header: &str, fm: &FlashMember) -> String {
    let mut flash = Section::new("flash");
    flash.push(format!("reg_point: [{}, {}]", fm.reg_point.0, fm.reg_point.1));
    format!("{header}type: flash\n\n{}", flash.build())
}

fn build_shockwave3d_yml(header: &str, s: &Shockwave3dMember) -> String {
    let info = &s.info;
    let mut scene = Section::new("scene");
    scene.push(format!("loop: {}", info.loops));
    scene.push(format!("duration: {}", info.duration));
    scene.push(format!("animation_enabled: {}", info.animation_enabled));
    scene.push(format!("preload: {}", info.preload));
    scene.push(format!("direct_to_stage: {}", info.direct_to_stage));
    scene.push(format!("reg_point: [{}, {}]", info.reg_point.0, info.reg_point.1));
    let r = info.default_rect;
    scene.push(format!("default_rect: [{}, {}, {}, {}]", r.0, r.1, r.2, r.3));

    let camera_yaml = if info.camera_position.is_some() || info.camera_rotation.is_some() {
        let mut camera = Section::new("camera");
        if let Some((x, y, z)) = info.camera_position {
            camera.push(format!("position: [{x}, {y}, {z}]"));
        }
        if let Some((x, y, z)) = info.camera_rotation {
            camera.push(format!("rotation: [{x}, {y}, {z}]"));
        }
        format!("\n{}", camera.build())
    } else {
        String::new()
    };

    let lighting_yaml = if info.bg_color.is_some() || info.ambient_color.is_some() {
        let mut lighting = Section::new("lighting");
        if let Some((r, g, b)) = info.bg_color {
            lighting.push(format!("bg_color: {}", rgb_hex((r, g, b))));
        }
        if let Some((r, g, b)) = info.ambient_color {
            lighting.push(format!("ambient_color: {}", rgb_hex((r, g, b))));
        }
        format!("\n{}", lighting.build())
    } else {
        String::new()
    };

    format!(
        "{header}type: shockwave3d\n\n{}{}{}",
        scene.build(), camera_yaml, lighting_yaml
    )
}

// ── Script decompilation ───────────────────────────────────────────────────────

fn decompile_script(script: &Script, cast: &CastLib) -> String {
    let lctx = match cast.lctx.as_ref() {
        Some(l) => l,
        None => return String::new(),
    };
    let multiplier = get_variable_multiplier(cast.capital_x, cast.dir_version);

    let properties = script.chunk.property_name_ids.iter()
        .map(|prop_name_id| {
            let default_value = script.chunk.property_defaults.get(prop_name_id).map(|v| {
                format_static_datum(v)
            });
            let name = if let Some(name) = lctx.names.get(*prop_name_id as usize) {
                yaml_string(name)
            } else {
                format!("prop{}", prop_name_id)
            };
            if let Some(value) = default_value {
                format!("property {} = {}", name, value)
            } else {
                format!("property {}", name)
            }
        })
        .collect_vec();

    let handlers = script
        .handler_names
        .iter()
        .filter_map(|name| script.get_own_handler(&name.to_lowercase()).map(|h| (name, h)))
        .map(|(name, handler)| {
            let decompiled = decompile_handler(
                handler,
                &script.chunk,
                lctx,
                cast.dir_version,
                multiplier,
            );

            let args = handler.argument_name_ids.iter().map(|id| {
                if let Some(name) = lctx.names.get(*id as usize) {
                    yaml_string(name)
                } else {
                    format!("arg{}", id)
                }
            }).collect::<Vec<_>>().join(", ");

            let lines = decompiled
                .lines
                .iter()
                .map(|line| format!("{}{}", "  ".repeat(line.indent as usize + 1), line.text))
                .collect::<Vec<_>>()
                .join("\n");

            format!("on {} {}\n{}\nend", name, args, lines)
        })
        .collect_vec();

    let mut result = String::new();
    if !properties.is_empty() {
        result.push_str(&properties.join("\n"));
        result.push_str("\n\n");
    }
    if !handlers.is_empty() {
        result.push_str(&handlers.join("\n\n"));
    }
    result
}

// ── Media encoders ─────────────────────────────────────────────────────────────

fn encode_png(bitmap: &Bitmap) -> Option<Vec<u8>> {
    if bitmap.data.is_empty() || bitmap.width == 0 || bitmap.height == 0 {
        return None;
    }
    // For indexed bitmaps, encode each index as R=G=B=index so the PNG is
    // editor-friendly (any tool can open/save it) while still preserving the
    // raw index values. The reimporter validates R==G==B to detect accidental
    // palette-baked edits.
    let rgba_data = if bitmap.bit_depth == 32 {
        bitmap.data.clone()
    } else {
        let mut rgba = Vec::with_capacity(bitmap.width as usize * bitmap.height as usize * 4);
        for &idx in &bitmap.data {
            let v = 255 - idx;
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
        rgba
    };
    let img = RgbaImage::from_raw(bitmap.width as u32, bitmap.height as u32, rgba_data)?;
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).ok()?;
    Some(buf)
}

fn encode_act(pm: &PaletteMember) -> Vec<u8> {
    let mut data = Vec::with_capacity(768);
    for &(r, g, b) in pm.colors.iter().take(256) {
        data.extend_from_slice(&[r, g, b]);
    }
    data.resize(768, 0);
    data
}

// ── YAML helpers ──────────────────────────────────────────────────────────────

fn yaml_string(s: &str) -> String {
    let needs_quoting = s.is_empty()
        || s.contains(|c: char| {
            matches!(c, ':' | '#' | '[' | ']' | '{' | '}' | ',' | '&' | '*' | '?' | '|'
                | '-' | '<' | '>' | '=' | '!' | '%' | '@' | '`' | '\'' | '"' | '\n' | '\r')
        })
        || s.starts_with(|c: char| c.is_ascii_digit())
        || matches!(s, "true" | "false" | "null" | "yes" | "no");
    if needs_quoting {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn rgb_hex(c: (u8, u8, u8)) -> String {
    format!("\"#{:02x}{:02x}{:02x}\"", c.0, c.1, c.2)
}

fn palette_name(palette_id: i16, cast: &CastLib) -> String {
    if palette_id < 0 {
        if let Some(built_in) = <BuiltInPalette as num::FromPrimitive>::from_i16(palette_id) {
            return built_in.symbol_string();
        }
    }
    if palette_id > 0 {
        if let Some(member) = cast.find_member_by_number(palette_id as u32) {
            if !member.name.is_empty() {
                return yaml_string(&member.name);
            }
        }
        return format!("{palette_id}");
    }
    "system-mac".to_string()
}

fn shape_type_name(t: &crate::director::enums::ShapeType) -> &'static str {
    use crate::director::enums::ShapeType;
    match t {
        ShapeType::Rect => "rect",
        ShapeType::Oval => "oval",
        ShapeType::OvalRect => "ovalRect",
        ShapeType::Line => "line",
        ShapeType::Unknown => "rect",
    }
}

fn fill_mode_name(mode: u32) -> &'static str {
    match mode {
        1 => "solid",
        2 => "gradient",
        _ => "none",
    }
}

// ── File helpers ───────────────────────────────────────────────────────────────

fn text_file(path: String, content: String) -> ExportFile {
    ExportFile { path, content: FileContent::Text(content) }
}

fn binary_file(path: String, data: Vec<u8>) -> ExportFile {
    ExportFile { path, content: FileContent::Binary(data) }
}

pub fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    sanitized.trim_matches(|c: char| c == '.' || c == ' ').to_string()
}
