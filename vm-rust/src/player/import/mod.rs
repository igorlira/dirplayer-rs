use std::collections::HashMap;

use image::GenericImageView;
use serde_yaml::Value;

use crate::{
    director::{
        chunks::sound::SoundChunk,
        enums::{
            BitmapInfo, ScriptType, ShapeInfo, ShapeType, Shockwave3dInfo, SoundInfo,
            VectorShapeVertex,
        },
        lingo::script::ScriptContext,
    },
    js_api::JsApi,
    player::{
        bitmap::{
            bitmap::{Bitmap, BuiltInPalette, PaletteRef},
            manager::BitmapManager,
        },
        cast_member::{
            BitmapMember, ButtonMember, ButtonType, CastMember, CastMemberType, FieldMember,
            FlashMember, PaletteMember, ScriptMember, ShapeMember, Shockwave3dMember,
            Shockwave3dRuntimeState, SoundMember, TextMember, VectorShapeMember,
        },
        lingo_compiler::{compile_lingo, inject_into_lctx},
        sprite::ColorRef,
        symbols::{builtin::BuiltInSymbol, symbol::Symbol},
        DirPlayer,
    },
};

pub struct ImportFile {
    pub path: String,
    pub content: Vec<u8>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn import_cast_pack(
    player: &mut DirPlayer,
    cast_number: u32,
    files: Vec<ImportFile>,
) -> Result<usize, String> {
    let grouped = group_files(files);

    if let Some(cast_files) = grouped.get("_cast") {
        if let Some(yml) = cast_files.get("yml") {
            if let Ok(text) = std::str::from_utf8(yml) {
                if let Ok(doc) = serde_yaml::from_str::<Value>(text) {
                    if let Some(name) = doc.get("name").and_then(|v| v.as_str()) {
                        player.movie.cast_manager.get_cast_mut(cast_number).name =
                            name.to_string();
                    }
                }
            }
        }
    }

    let mut members: Vec<(u32, CastMember)> = Vec::new();
    let mut pending_scripts: Vec<(u32, String)> = Vec::new(); // (slot, ls_source)

    for (stem, ext_map) in &grouped {
        if stem.starts_with('_') {
            continue;
        }
        let yml_bytes = match ext_map.get("yml") {
            Some(b) => b,
            None => continue,
        };
        let text = match std::str::from_utf8(yml_bytes) {
            Ok(t) => t,
            Err(e) => return Err(format!("{stem}.yml: {e}")),
        };
        let doc: Value = serde_yaml::from_str(text)
            .map_err(|e| format!("{stem}.yml: {e}"))?;

        let slot = doc
            .get("slot")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("{stem}.yml: missing slot"))? as u32;
        let name = doc
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(stem)
            .to_string();
        let comments = doc
            .get("comments")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let type_str = doc.get("type").and_then(|v| v.as_str()).unwrap_or("");

        let member_type = match type_str {
            "bitmap" => build_bitmap(&doc, ext_map, &mut player.bitmap_manager)?,
            "palette" => build_palette(ext_map)?,
            "shape" => build_shape(&doc)?,
            "script" => {
                // Collect .ls source for later compilation
                if let Some(ls_bytes) = ext_map.get("ls") {
                    if let Ok(src) = std::str::from_utf8(ls_bytes) {
                        pending_scripts.push((slot, src.to_string()));
                    }
                }
                build_script(&doc, slot)?
            }
            "field" => build_field(&doc, ext_map)?,
            "button" => build_button(&doc, ext_map)?,
            "sound" => build_sound(&doc, ext_map)?,
            "text" => build_text(&doc, ext_map)?,
            "vector_shape" => build_vector_shape(&doc)?,
            "flash" => build_flash(ext_map)?,
            "shockwave3d" => build_shockwave3d(&doc, ext_map)?,
            _ => continue,
        };

        let mut member = CastMember {
            number: slot,
            name,
            comments,
            member_type,
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
            reg_point: (0, 0),
        };

        // A non-script member may carry its own behaviour, exported as a `.ls`
        // beside it with a `member_script:` section naming it. Compile it under
        // the member's own slot and point the member at it, so the behaviour
        // survives the round trip.
        if doc.get("member_script").is_some() {
            if let Some(ls_bytes) = ext_map.get("ls") {
                if let Ok(src) = std::str::from_utf8(ls_bytes) {
                    pending_scripts.push((slot, src.to_string()));
                    member.set_script_id(slot);
                }
            }
        }

        members.push((slot, member));
    }

    // Compile all pending scripts and inject into the cast's lctx
    if !pending_scripts.is_empty() {
        let cast = player.movie.cast_manager.get_cast_mut(cast_number);
        cast.capital_x = true; // use variable multiplier = 1
        if cast.lctx.is_none() {
            cast.lctx = Some(ScriptContext {
                names: Vec::new(),
                scripts: HashMap::new(),
            });
        }
        let lctx = cast.lctx.as_mut().unwrap();
        for (slot, src) in pending_scripts {
            match compile_lingo(&src, slot as u16) {
                Ok(result) => inject_into_lctx(lctx, result, slot as u32),
                Err(e) => {
                    crate::utils::log_i(&format!(
                        "Script compile error at slot {slot}: {e}"
                    ));
                }
            }
        }
    }

    let count = members.len();
    let cast = player.movie.cast_manager.get_cast_mut(cast_number);
    for (slot, member) in members {
        cast.insert_member(slot, member);
    }

    JsApi::dispatch_cast_member_list_changed(cast_number);

    Ok(count)
}

// ── File grouping ─────────────────────────────────────────────────────────────

fn group_files(files: Vec<ImportFile>) -> HashMap<String, HashMap<String, Vec<u8>>> {
    let mut grouped: HashMap<String, HashMap<String, Vec<u8>>> = HashMap::new();
    for file in files {
        let path = std::path::Path::new(&file.path);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !stem.is_empty() {
            grouped.entry(stem).or_default().insert(ext, file.content);
        }
    }
    grouped
}

// ── Member builders ───────────────────────────────────────────────────────────

fn build_bitmap(
    doc: &Value,
    ext_map: &HashMap<String, Vec<u8>>,
    bm: &mut BitmapManager,
) -> Result<CastMemberType, String> {
    let img_sec = doc.get("image");
    let bit_depth = img_sec
        .and_then(|s| s.get("depth"))
        .and_then(|v| v.as_u64())
        .unwrap_or(8) as u8;
    let palette_str = img_sec
        .and_then(|s| s.get("palette"))
        .and_then(|v| v.as_str())
        .unwrap_or("systemMac");
    let reg = parse_i16_pair(img_sec.and_then(|s| s.get("reg_point")));
    let use_alpha = img_sec
        .and_then(|s| s.get("alpha"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let center_reg_point = img_sec
        .and_then(|s| s.get("center_reg_point"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let trim_white_space = img_sec
        .and_then(|s| s.get("trim_white_space"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let palette_ref = parse_palette_ref(palette_str);
    let palette_id = palette_ref_to_id(&palette_ref);

    let (bitmap_width, bitmap_height, actual_bit_depth) =
        if let Some(png_bytes) = ext_map.get("png") {
            decode_png_dimensions(png_bytes)?
        } else {
            (0, 0, bit_depth)
        };

    let bitmap = if let Some(png_bytes) = ext_map.get("png") {
        decode_png_to_bitmap(png_bytes, palette_ref.clone())?
    } else {
        Bitmap::new(0, 0, bit_depth, bit_depth, 0, palette_ref)
    };

    let image_ref = bm.add_bitmap(bitmap);

    Ok(CastMemberType::Bitmap(BitmapMember {
        image_ref,
        reg_point: reg,
        script_id: 0,
        member_script_ref: None,
        info: BitmapInfo {
            width: bitmap_width,
            height: bitmap_height,
            reg_x: reg.0,
            reg_y: reg.1,
            bit_depth: actual_bit_depth,
            palette_id,
            clut_cast_lib: 0,
            pitch: 0,
            use_alpha,
            trim_white_space,
            center_reg_point,
        },
    }))
}

fn build_palette(ext_map: &HashMap<String, Vec<u8>>) -> Result<CastMemberType, String> {
    let mut pm = PaletteMember::new();
    if let Some(act) = ext_map.get("act") {
        if act.len() >= 768 {
            for i in 0..256usize {
                let base = i * 3;
                pm.colors[i] = (act[base], act[base + 1], act[base + 2]);
            }
        }
    }
    Ok(CastMemberType::Palette(pm))
}

fn build_shape(doc: &Value) -> Result<CastMemberType, String> {
    let sec = doc.get("shape");
    let shape_type = match sec
        .and_then(|s| s.get("shape_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("rect")
    {
        "oval" => ShapeType::Oval,
        "ovalRect" => ShapeType::OvalRect,
        "line" => ShapeType::Line,
        _ => ShapeType::Rect,
    };
    let rect = parse_i16_quad(sec.and_then(|s| s.get("rect")));
    let mut info = ShapeInfo::default_rect();
    info.shape_type = shape_type;
    info.rect_left = rect.0;
    info.rect_top = rect.1;
    info.rect_right = rect.2;
    info.rect_bottom = rect.3;
    info.pattern = sec
        .and_then(|s| s.get("pattern"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    info.fore_color = sec
        .and_then(|s| s.get("fore_color"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u8;
    info.back_color = sec
        .and_then(|s| s.get("back_color"))
        .and_then(|v| v.as_u64())
        .unwrap_or(255) as u8;
    info.fill_type = sec
        .and_then(|s| s.get("fill_type"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u8;
    info.line_thickness = sec
        .and_then(|s| s.get("line_thickness"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u8;
    info.line_direction = sec
        .and_then(|s| s.get("line_direction"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u8;
    Ok(CastMemberType::Shape(ShapeMember {
        shape_info: info,
        script_id: 0,
        member_script_ref: None,
    }))
}

fn build_script(doc: &Value, slot: u32) -> Result<CastMemberType, String> {
    let sec = doc.get("script");
    let script_type = match sec
        .and_then(|s| s.get("script_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("behavior")
    {
        "movie" => ScriptType::Movie,
        "score" => ScriptType::Score,
        "parent" => ScriptType::Parent,
        _ => ScriptType::Member,
    };
    let name = doc
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(CastMemberType::Script(ScriptMember {
        script_id: slot, // use slot as script_id so lctx lookup works
        script_type,
        name,
    }))
}

fn build_field(
    doc: &Value,
    ext_map: &HashMap<String, Vec<u8>>,
) -> Result<CastMemberType, String> {
    let text = read_text(ext_map);
    let field_sec = doc.get("field");
    let font_sec = doc.get("font");
    let mut fm = FieldMember::default();
    fm.text = text;
    fm.box_type = parse_builtin_symbol(
        field_sec
            .and_then(|s| s.get("box_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("adjust"),
        BuiltInSymbol::Adjust,
    );
    fm.word_wrap = field_sec
        .and_then(|s| s.get("word_wrap"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    fm.width = field_sec
        .and_then(|s| s.get("width"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.height = field_sec
        .and_then(|s| s.get("height"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.text_height = field_sec
        .and_then(|s| s.get("text_height"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.auto_tab = field_sec
        .and_then(|s| s.get("auto_tab"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    fm.editable = field_sec
        .and_then(|s| s.get("editable"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    fm.border = field_sec
        .and_then(|s| s.get("border"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.margin = field_sec
        .and_then(|s| s.get("margin"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.box_drop_shadow = field_sec
        .and_then(|s| s.get("box_drop_shadow"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.drop_shadow = field_sec
        .and_then(|s| s.get("drop_shadow"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.top_spacing = field_sec
        .and_then(|s| s.get("top_spacing"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i16;
    fm.anti_alias = field_sec
        .and_then(|s| s.get("anti_alias"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    fm.font = font_sec
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    fm.font_style = font_sec
        .and_then(|s| s.get("style"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    fm.font_size = font_sec
        .and_then(|s| s.get("size"))
        .and_then(|v| v.as_u64())
        .unwrap_or(12) as u16;
    fm.alignment = parse_builtin_symbol(
        font_sec
            .and_then(|s| s.get("alignment"))
            .and_then(|v| v.as_str())
            .unwrap_or("left"),
        BuiltInSymbol::Left,
    );
    fm.fixed_line_space = font_sec
        .and_then(|s| s.get("fixed_line_space"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    Ok(CastMemberType::Field(fm))
}

fn build_button(
    doc: &Value,
    ext_map: &HashMap<String, Vec<u8>>,
) -> Result<CastMemberType, String> {
    let text = read_text(ext_map);
    let btn_sec = doc.get("button");
    let font_sec = doc.get("font");
    let button_type = match btn_sec
        .and_then(|s| s.get("button_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("pushButton")
    {
        "checkBox" => ButtonType::CheckBox,
        "radioButton" => ButtonType::RadioButton,
        _ => ButtonType::PushButton,
    };
    let mut fm = FieldMember::default();
    fm.text = text;
    fm.width = btn_sec
        .and_then(|s| s.get("width"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.height = btn_sec
        .and_then(|s| s.get("height"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.alignment = parse_builtin_symbol(
        btn_sec
            .and_then(|s| s.get("alignment"))
            .and_then(|v| v.as_str())
            .unwrap_or("left"),
        BuiltInSymbol::Left,
    );
    fm.word_wrap = btn_sec
        .and_then(|s| s.get("word_wrap"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    fm.auto_tab = btn_sec
        .and_then(|s| s.get("auto_tab"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    fm.border = btn_sec
        .and_then(|s| s.get("border"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.margin = btn_sec
        .and_then(|s| s.get("margin"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.box_drop_shadow = btn_sec
        .and_then(|s| s.get("box_drop_shadow"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.drop_shadow = btn_sec
        .and_then(|s| s.get("drop_shadow"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    fm.top_spacing = btn_sec
        .and_then(|s| s.get("top_spacing"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i16;
    fm.anti_alias = btn_sec
        .and_then(|s| s.get("anti_alias"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    fm.font = font_sec
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    fm.font_style = font_sec
        .and_then(|s| s.get("style"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    fm.font_size = font_sec
        .and_then(|s| s.get("size"))
        .and_then(|v| v.as_u64())
        .unwrap_or(12) as u16;
    fm.fixed_line_space = font_sec
        .and_then(|s| s.get("fixed_line_space"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    Ok(CastMemberType::Button(ButtonMember {
        field: fm,
        button_type,
        hilite: false,
        script_id: 0,
        member_script_ref: None,
    }))
}

fn build_sound(
    doc: &Value,
    ext_map: &HashMap<String, Vec<u8>>,
) -> Result<CastMemberType, String> {
    let audio_sec = doc.get("audio");
    let loop_enabled = audio_sec
        .and_then(|s| s.get("loop"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (sound, info) = if let Some(wav_bytes) = ext_map.get("wav") {
        let chunk = SoundChunk::from_wav(wav_bytes)?;
        let info = SoundInfo {
            sample_rate: chunk.sample_rate(),
            sample_size: chunk.bits_per_sample(),
            channels: chunk.channels(),
            sample_count: chunk.sample_count(),
            duration: 0,
            loop_enabled,
        };
        (chunk, info)
    } else {
        let info = SoundInfo {
            sample_rate: 44100,
            sample_size: 16,
            channels: 1,
            sample_count: 0,
            duration: 0,
            loop_enabled,
        };
        (SoundChunk::new(vec![]), info)
    };

    Ok(CastMemberType::Sound(SoundMember {
        info,
        sound,
        cue_point_times: Vec::new(),
        cue_point_names: Vec::new(),
    }))
}

fn build_text(
    doc: &Value,
    ext_map: &HashMap<String, Vec<u8>>,
) -> Result<CastMemberType, String> {
    let raw_text = read_text(ext_map);
    let rtf_source = if ext_map.contains_key("rtf") { raw_text.clone() } else { String::new() };
    let plain_text = if ext_map.contains_key("rtf") { String::new() } else { raw_text };

    let text_sec = doc.get("text");
    let font_sec = doc.get("font");
    let font_style = font_sec
        .and_then(|s| s.get("style"))
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str())
                .map(|s| parse_builtin_symbol(s, BuiltInSymbol::Plain))
                .collect()
        })
        .unwrap_or_default();
    let mut tm = TextMember {
        text: plain_text,
        rtf_source,
        html_source: String::new(),
        alignment: BuiltInSymbol::Left,
        box_type: BuiltInSymbol::Adjust,
        word_wrap: false,
        anti_alias: false,
        font: String::new(),
        font_style,
        font_size: 12,
        fixed_line_space: 0,
        top_spacing: 0,
        bottom_spacing: 0,
        width: 0,
        height: 0,
        rect_set_at_runtime: false,
        text_set_at_runtime: false,
        char_spacing: 0,
        tab_stops: Vec::new(),
        html_styled_spans: Vec::new(),
        par_infos: Vec::new(),
        par_runs: Vec::new(),
        hyperlinks: Vec::new(),
        info: None,
        w3d: None,
        anti_alias_type: BuiltInSymbol::AutoAlias,
        sel_start: 0,
        sel_end: 0,
        sel_anchor: 0,
        script_id: 0,
        member_script_ref: None,
    };
    tm.alignment = parse_builtin_symbol(
        text_sec
            .and_then(|s| s.get("alignment"))
            .and_then(|v| v.as_str())
            .unwrap_or("left"),
        BuiltInSymbol::Left,
    );
    tm.box_type = parse_builtin_symbol(
        text_sec
            .and_then(|s| s.get("box_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("adjust"),
        BuiltInSymbol::Adjust,
    );
    tm.word_wrap = text_sec
        .and_then(|s| s.get("word_wrap"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    tm.width = text_sec
        .and_then(|s| s.get("width"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    tm.height = text_sec
        .and_then(|s| s.get("height"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    tm.char_spacing = text_sec
        .and_then(|s| s.get("char_spacing"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    tm.font = font_sec
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    tm.font_size = font_sec
        .and_then(|s| s.get("size"))
        .and_then(|v| v.as_u64())
        .unwrap_or(12) as u16;
    tm.anti_alias = font_sec
        .and_then(|s| s.get("anti_alias"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    tm.fixed_line_space = font_sec
        .and_then(|s| s.get("fixed_line_space"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    tm.top_spacing = font_sec
        .and_then(|s| s.get("top_spacing"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i16;
    tm.bottom_spacing = font_sec
        .and_then(|s| s.get("bottom_spacing"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i16;
    Ok(CastMemberType::Text(tm))
}

fn build_vector_shape(doc: &Value) -> Result<CastMemberType, String> {
    let stroke_sec = doc.get("stroke");
    let fill_sec = doc.get("fill");
    let shape_sec = doc.get("shape");

    let stroke_color = parse_rgb_hex(
        stroke_sec
            .and_then(|s| s.get("color"))
            .and_then(|v| v.as_str())
            .unwrap_or("#000000"),
    );
    let stroke_width = stroke_sec
        .and_then(|s| s.get("width"))
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0) as f32;
    let fill_mode = match fill_sec
        .and_then(|s| s.get("mode"))
        .and_then(|v| v.as_str())
        .unwrap_or("none")
    {
        "solid" => 1u32,
        "gradient" => 2,
        _ => 0,
    };
    let fill_color = parse_rgb_hex(
        fill_sec
            .and_then(|s| s.get("color"))
            .and_then(|v| v.as_str())
            .unwrap_or("#000000"),
    );
    let end_color = parse_rgb_hex(
        fill_sec
            .and_then(|s| s.get("end_color"))
            .and_then(|v| v.as_str())
            .unwrap_or("#000000"),
    );
    let gradient_type = parse_builtin_symbol(
        fill_sec
            .and_then(|s| s.get("gradient_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("linear"),
        BuiltInSymbol::Linear,
    );
    let fill_scale = fill_sec
        .and_then(|s| s.get("scale"))
        .and_then(|v| v.as_f64())
        .unwrap_or(100.0) as f32;
    let fill_direction = fill_sec
        .and_then(|s| s.get("direction"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let fill_offset = parse_i32_pair(fill_sec.and_then(|s| s.get("offset")));
    let fill_cycles = fill_sec
        .and_then(|s| s.get("cycles"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1) as i32;

    let closed = shape_sec
        .and_then(|s| s.get("closed"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let bg_color = parse_rgb_hex(
        shape_sec
            .and_then(|s| s.get("bg_color"))
            .and_then(|v| v.as_str())
            .unwrap_or("#ffffff"),
    );
    let antialias = shape_sec
        .and_then(|s| s.get("antialias"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let reg_point_raw = parse_i32_pair(shape_sec.and_then(|s| s.get("reg_point")));
    let reg_point = (reg_point_raw.0 as i16, reg_point_raw.1 as i16);
    let center_reg_point = shape_sec
        .and_then(|s| s.get("center_reg_point"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reg_point_vertex = shape_sec
        .and_then(|s| s.get("reg_point_vertex"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    let direct_to_stage = shape_sec
        .and_then(|s| s.get("direct_to_stage"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let origin_mode = parse_builtin_symbol(
        shape_sec
            .and_then(|s| s.get("origin_mode"))
            .and_then(|v| v.as_str())
            .unwrap_or("center"),
        BuiltInSymbol::Center,
    );
    let scale_mode = parse_builtin_symbol(
        shape_sec
            .and_then(|s| s.get("scale_mode"))
            .and_then(|v| v.as_str())
            .unwrap_or("autoSize"),
        BuiltInSymbol::AutoSize,
    );
    let scale = shape_sec
        .and_then(|s| s.get("scale"))
        .and_then(|v| v.as_f64())
        .unwrap_or(100.0) as f32;

    let mut vertices: Vec<VectorShapeVertex> = Vec::new();
    if let Some(Value::Sequence(verts)) = doc.get("vertices") {
        for v in verts {
            vertices.push(VectorShapeVertex {
                x: v.get("x").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                y: v.get("y").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                handle1_x: v
                    .get("handle1_x")
                    .and_then(|x| x.as_f64())
                    .unwrap_or(0.0) as f32,
                handle1_y: v
                    .get("handle1_y")
                    .and_then(|x| x.as_f64())
                    .unwrap_or(0.0) as f32,
                handle2_x: v
                    .get("handle2_x")
                    .and_then(|x| x.as_f64())
                    .unwrap_or(0.0) as f32,
                handle2_y: v
                    .get("handle2_y")
                    .and_then(|x| x.as_f64())
                    .unwrap_or(0.0) as f32,
            });
        }
    }

    Ok(CastMemberType::VectorShape(VectorShapeMember {
        stroke_color,
        fill_color,
        bg_color,
        end_color,
        stroke_width,
        fill_mode,
        closed,
        vertices,
        bbox_left: 0.0,
        bbox_top: 0.0,
        bbox_right: 0.0,
        bbox_bottom: 0.0,
        member_width: 0,
        member_height: 0,
        reg_point,
        gradient_type,
        fill_scale,
        fill_direction,
        fill_offset,
        fill_cycles,
        scale_mode,
        scale,
        antialias,
        center_reg_point,
        reg_point_vertex,
        direct_to_stage,
        origin_mode,
        new_curve_count: 0,
    }))
}

fn build_flash(ext_map: &HashMap<String, Vec<u8>>) -> Result<CastMemberType, String> {
    let data = ext_map.get("swf").cloned().unwrap_or_default();
    Ok(CastMemberType::Flash(FlashMember {
        data,
        reg_point: (0, 0),
        flash_info: None,
    }))
}

fn build_shockwave3d(
    doc: &Value,
    ext_map: &HashMap<String, Vec<u8>>,
) -> Result<CastMemberType, String> {
    let scene_sec = doc.get("scene");
    let camera_sec = doc.get("camera");
    let lighting_sec = doc.get("lighting");

    let reg = parse_i32_pair(scene_sec.and_then(|s| s.get("reg_point")));
    let default_rect = parse_i32_quad(scene_sec.and_then(|s| s.get("default_rect")));

    let camera_position = camera_sec.and_then(|s| {
        let p = parse_f32_triple(s.get("position"))?;
        Some(p)
    });
    let camera_rotation = camera_sec.and_then(|s| {
        let r = parse_f32_triple(s.get("rotation"))?;
        Some(r)
    });
    let bg_color = lighting_sec
        .and_then(|s| s.get("bg_color"))
        .and_then(|v| v.as_str())
        .map(parse_rgb_hex);
    let ambient_color = lighting_sec
        .and_then(|s| s.get("ambient_color"))
        .and_then(|v| v.as_str())
        .map(parse_rgb_hex);

    let info = Shockwave3dInfo {
        loops: scene_sec
            .and_then(|s| s.get("loop"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        duration: scene_sec
            .and_then(|s| s.get("duration"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        animation_enabled: scene_sec
            .and_then(|s| s.get("animation_enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        preload: scene_sec
            .and_then(|s| s.get("preload"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        direct_to_stage: scene_sec
            .and_then(|s| s.get("direct_to_stage"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        reg_point: (reg.0, reg.1),
        default_rect,
        camera_position,
        camera_rotation,
        bg_color,
        ambient_color,
    };

    let w3d_data = ext_map.get("w3d").cloned().unwrap_or_default();
    let runtime_state = Shockwave3dRuntimeState::default();

    Ok(CastMemberType::Shockwave3d(Shockwave3dMember {
        info,
        w3d_data,
        source_scene: None,
        parsed_scene: None,
        runtime_state,
        converted_from_text: false,
        text3d_state: None,
        text3d_source: None,
    }))
}

// ── PNG decode ────────────────────────────────────────────────────────────────

fn decode_png_dimensions(data: &[u8]) -> Result<(u16, u16, u8), String> {
    let img = image::load_from_memory(data).map_err(|e| e.to_string())?;
    let (w, h) = img.dimensions();
    let rgba = img.to_rgba8();
    let is_indexed = rgba
        .pixels()
        .all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255);
    let bit_depth = if is_indexed { 8u8 } else { 32u8 };
    Ok((w as u16, h as u16, bit_depth))
}

fn decode_png_to_bitmap(data: &[u8], palette_ref: PaletteRef) -> Result<Bitmap, String> {
    let img = image::load_from_memory(data).map_err(|e| e.to_string())?;
    let (w, h) = img.dimensions();
    let rgba = img.to_rgba8();

    let is_indexed = rgba
        .pixels()
        .all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255);

    if is_indexed {
        // Validate and recover: index = 255 - R
        let pixel_data: Vec<u8> = rgba.pixels().map(|p| {
            if p[0] != p[1] || p[1] != p[2] {
                // Shouldn't happen after the all() check, but be safe
                0u8
            } else {
                255 - p[0]
            }
        }).collect();
        let mut bitmap = Bitmap::new(w as u16, h as u16, 8, 8, 0, palette_ref);
        bitmap.data = pixel_data;
        Ok(bitmap)
    } else {
        // 32-bit RGBA
        let pixel_data = rgba.into_raw();
        let mut bitmap = Bitmap::new(w as u16, h as u16, 32, 32, 8, palette_ref);
        bitmap.data = pixel_data;
        Ok(bitmap)
    }
}

// ── Parse helpers ─────────────────────────────────────────────────────────────

fn parse_palette_ref(name: &str) -> PaletteRef {
    if let Some(built_in) = BuiltInPalette::from_symbol(Symbol::from_str(name)) {
        PaletteRef::BuiltIn(built_in)
    } else {
        PaletteRef::BuiltIn(BuiltInPalette::SystemMac)
    }
}

/// Resolves a YAML-authored style keyword (e.g. "adjust", "left") to its
/// built-in symbol, falling back to `default` for unrecognized values.
fn parse_builtin_symbol(s: &str, default: BuiltInSymbol) -> BuiltInSymbol {
    Symbol::from_str(s).into_builtin().unwrap_or(default)
}

fn palette_ref_to_id(pr: &PaletteRef) -> i16 {
    match pr {
        PaletteRef::BuiltIn(b) => *b as i16,
        PaletteRef::Member(r) => r.cast_member as i16,
        PaletteRef::Default => BuiltInPalette::SystemMac as i16,
    }
}

fn parse_i16_pair(v: Option<&Value>) -> (i16, i16) {
    if let Some(Value::Sequence(seq)) = v {
        let x = seq.first().and_then(|v| v.as_i64()).unwrap_or(0) as i16;
        let y = seq.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i16;
        (x, y)
    } else {
        (0, 0)
    }
}

fn parse_i32_pair(v: Option<&Value>) -> (i32, i32) {
    if let Some(Value::Sequence(seq)) = v {
        let x = seq.first().and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let y = seq.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        (x, y)
    } else {
        (0, 0)
    }
}

fn parse_i16_quad(v: Option<&Value>) -> (i16, i16, i16, i16) {
    if let Some(Value::Sequence(seq)) = v {
        let a = seq.first().and_then(|v| v.as_i64()).unwrap_or(0) as i16;
        let b = seq.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i16;
        let c = seq.get(2).and_then(|v| v.as_i64()).unwrap_or(0) as i16;
        let d = seq.get(3).and_then(|v| v.as_i64()).unwrap_or(0) as i16;
        (a, b, c, d)
    } else {
        (0, 0, 0, 0)
    }
}

fn parse_i32_quad(v: Option<&Value>) -> (i32, i32, i32, i32) {
    if let Some(Value::Sequence(seq)) = v {
        let a = seq.first().and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let b = seq.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let c = seq.get(2).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let d = seq.get(3).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        (a, b, c, d)
    } else {
        (0, 0, 0, 0)
    }
}

fn parse_f32_triple(v: Option<&Value>) -> Option<(f32, f32, f32)> {
    if let Some(Value::Sequence(seq)) = v {
        let x = seq.first().and_then(|v| v.as_f64())? as f32;
        let y = seq.get(1).and_then(|v| v.as_f64())? as f32;
        let z = seq.get(2).and_then(|v| v.as_f64())? as f32;
        Some((x, y, z))
    } else {
        None
    }
}

fn parse_rgb_hex(s: &str) -> (u8, u8, u8) {
    let s = s.trim_matches('"').trim_start_matches('#');
    if s.len() < 6 {
        return (0, 0, 0);
    }
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0);
    (r, g, b)
}

fn read_text(ext_map: &HashMap<String, Vec<u8>>) -> String {
    for ext in &["txt", "rtf", "ls"] {
        if let Some(bytes) = ext_map.get(*ext) {
            return String::from_utf8_lossy(bytes).into_owned();
        }
    }
    String::new()
}
