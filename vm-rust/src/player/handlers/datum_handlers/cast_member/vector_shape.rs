use std::collections::VecDeque;

use crate::{
    director::lingo::datum::{datum_bool, Datum, DatumType},
    player::{
        bitmap::bitmap::{Bitmap, BuiltInPalette, PaletteRef},
        cast_lib::CastMemberRef,
        cast_member::CastMemberType,
        datum_ref::DatumRef,
        sprite::ColorRef,
        DirPlayer, ScriptError,
    },
};

/// Lingo property get/set for `#vectorShape` cast members.
///
/// Per-property semantics and the FLSH binary offsets that back them are
/// documented in `cast_member.rs::parse_flsh_payload. Director's Lingo is
/// case-insensitive on property names, so both reads and writes route
/// through `match_ci!` to accept any casing the user types.
pub struct VectorShapeMemberHandlers;

impl VectorShapeMemberHandlers {
    pub fn get_prop(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        // The .image getter rasterizes the polygon into a fresh ephemeral
        // bitmap and needs `&mut player.bitmap_manager`, which conflicts
        // with holding `&cast_member` from the cast manager. Handle .image
        // first by snapshotting all the values it needs, dropping the
        // borrow, and only then touching bitmap_manager. Other props read
        // directly from the cast member.
        if prop.eq_ignore_ascii_case("image") {
            return Self::get_image(player, member_ref);
        }

        let cast_member = player
            .movie
            .cast_manager
            .find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
        let vs = match &cast_member.member_type {
            CastMemberType::VectorShape(vs) => vs,
            _ => return Err(ScriptError::new("Expected vectorShape member".to_string())),
        };

        // `vertexList` returns a list of prop-lists, which requires
        // `player.alloc_datum` calls — also a borrow conflict with
        // `cast_member`. Snapshot the data, drop the borrow, then build.
        if prop.eq_ignore_ascii_case("vertexList") {
            // Director decides handle emission per *shape*, not per vertex: a
            // bezier shape lists #handle1/#handle2 on EVERY vertex (even when
            // they're point(0,0)), while a plain polygon shape (all handles
            // zero — e.g. figure8 Slider Groove) lists none. So treat the
            // shape as bezier if ANY vertex has a non-zero handle.
            let shape_is_bezier = vs.vertices.iter().any(|v| {
                v.handle1_x != 0.0 || v.handle1_y != 0.0
                    || v.handle2_x != 0.0 || v.handle2_y != 0.0
            });
            let verts: Vec<(i32, i32, i32, i32, i32, i32, bool)> = vs
                .vertices
                .iter()
                .map(|v| {
                    (
                        v.x as i32, v.y as i32,
                        v.handle1_x as i32, v.handle1_y as i32,
                        v.handle2_x as i32, v.handle2_y as i32,
                        shape_is_bezier,
                    )
                })
                .collect();
            let new_curve_count = vs.new_curve_count;
            return Self::build_vertex_list(player, verts, new_curve_count);
        }

        // `member.vertex` (chunk expression) — Director returns the list of
        // vertex *locations* as points; `member.vertex[i]` then indexes it
        // and `member.vertex.count` reads its length (Director 11.5 Scripting
        // Dictionary, `vertex`). Handle1/handle2 are reached via the
        // getPropRef path (`member.vertex[i].handle1`), not here. Snapshot
        // the coords, drop the borrow, then allocate the point list.
        if prop.eq_ignore_ascii_case("vertex") {
            let pts: Vec<(f32, f32)> =
                vs.vertices.iter().map(|v| (v.x, v.y)).collect();
            let list: VecDeque<DatumRef> = pts
                .iter()
                .map(|(x, y)| player.alloc_datum(Datum::Point([*x as f64, *y as f64], 0)))
                .collect();
            return Ok(Datum::List(DatumType::List, list, false));
        }

        match_ci!(prop, {
            "width"           => Ok(Datum::Int(vs.width().ceil() as i32)),
            "height"          => Ok(Datum::Int(vs.height().ceil() as i32)),
            "rect"            => Ok(Datum::Rect(
                                    [0.0, 0.0, vs.width().ceil() as f64, vs.height().ceil() as f64],
                                    0,
                                )),
            "strokeColor"     => {
                                    let (r, g, b) = vs.stroke_color;
                                    Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                                },
            "strokeWidth"     => Ok(Datum::Float(vs.stroke_width as f64)),
            "closed"          => Ok(datum_bool(vs.closed)),
            "fillMode"        => {
                                    let sym = match vs.fill_mode {
                                        0 => "none",
                                        1 => "solid",
                                        2 => "gradient",
                                        _ => "none",
                                    };
                                    Ok(Datum::Symbol(sym.to_string()))
                                },
            "fillColor"       => {
                                    let (r, g, b) = vs.fill_color;
                                    Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                                },
            "backgroundColor" | "bgColor" => {
                                    let (r, g, b) = vs.bg_color;
                                    Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                                },
            "endColor"        => {
                                    let (r, g, b) = vs.end_color;
                                    Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                                },
            "gradientType"    => Ok(Datum::Symbol(vs.gradient_type.clone())),
            "fillScale"       => Ok(Datum::Float(vs.fill_scale as f64)),
            "fillDirection"   => Ok(Datum::Float(vs.fill_direction as f64)),
            "fillOffset"      => Ok(Datum::Point(
                                    [vs.fill_offset.0 as f64, vs.fill_offset.1 as f64],
                                    0,
                                )),
            "fillCycles"      => Ok(Datum::Int(vs.fill_cycles)),
            "scaleMode"       => Ok(Datum::Symbol(vs.scale_mode.clone())),
            "scale"           => Ok(Datum::Float(vs.scale as f64)),
            "antialias"       => Ok(datum_bool(vs.antialias)),
            "centerRegPoint"  => Ok(datum_bool(vs.center_reg_point)),
            "regPointVertex"  => Ok(Datum::Int(vs.reg_point_vertex)),
            "directToStage"   => Ok(datum_bool(vs.direct_to_stage)),
            "originMode"      => Ok(Datum::Symbol(vs.origin_mode.clone())),
            "originPoint"     => Ok(Datum::Point(
                                    [vs.reg_point.0 as f64, vs.reg_point.1 as f64],
                                    0,
                                )),
            _ => Err(ScriptError::new(format!(
                "VectorShape members don't support property {}", prop
            ))),
        })
    }

    /// Method-call surface for `#vectorShape` members (`addVertex`,
    /// `deleteVertex`, `moveVertex`, plus the indexed `vertex[i]` get/set
    /// reference path). Director's Lingo is case-insensitive on handler
    /// names, so they're matched via `match_ci!`. `erase` is handled
    /// generically by `CastMemberRefHandlers` (it deletes the member), so it
    /// is not routed here.
    pub fn call(
        player: &mut DirPlayer,
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let member_ref = match player.get_datum(datum) {
            Datum::CastMember(r) => r.to_owned(),
            _ => return Err(ScriptError::new(
                "Cannot call vectorShape handler on non-cast-member".to_string(),
            )),
        };
        match_ci!(handler_name, {
            // addVertex(indexToAddAt, point {, h1H, h1V, h2H, h2V})
            // Inserts a vertex at the given 1-based index (Director 11.5
            // Scripting Dictionary). Optional trailing ints are the two
            // Bezier control-handle offsets, relative to the vertex.
            "addVertex" => Self::add_vertex(player, &member_ref, args),
            // deleteVertex(index) — removes the vertex at the 1-based index.
            "deleteVertex" => Self::delete_vertex(player, &member_ref, args),
            // moveVertex(index, dx, dy) — offsets an existing vertex.
            "moveVertex" => Self::move_vertex(player, &member_ref, args),
            // `member.vertex[i]` by reference (compiled as
            // getPropRef(member, #vertex, i)) — returns a writable
            // VectorVertexRef so `.handle1`/`.handle2`/`.vertex` reads and
            // writes resolve back into the member (handled in
            // player_get_obj_prop / player_set_obj_prop). The plain
            // `getPropRef(member, #vertexList)` form (no index) has no
            // chunk semantics, so it falls through to a normal prop read.
            "getPropRef" => Self::get_vertex_ref(player, &member_ref, args),
            // `member.vertex[i] = value` (compiled as
            // setProp(member, #vertex, i, value)). Sets the vertex location
            // from a point value (or a full [#vertex/#handle1/#handle2]
            // property list, mirroring vertexList entries).
            "setProp" => Self::set_vertex_by_index(player, &member_ref, args),
            _ => Err(ScriptError::new(format!(
                "No handler {} for vectorShape member", handler_name
            ))),
        })
    }

    /// getPropRef(member, #vertex, i) → a writable `VectorVertexRef`.
    /// Only `#vertex` has indexed-reference semantics; any other prop name
    /// (or a missing index) returns an error so the caller's getPropRef
    /// fallback resolves it as a normal by-value property instead.
    fn get_vertex_ref(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.len() < 2 {
            return Err(ScriptError::new(
                "vectorShape getPropRef without an index".to_string(),
            ));
        }
        let prop = player.get_datum(&args[0]).string_value().unwrap_or_default();
        if !prop.eq_ignore_ascii_case("vertex") {
            return Err(ScriptError::new(format!(
                "vectorShape getPropRef unsupported for {}", prop
            )));
        }
        let index = player.get_datum(&args[1]).int_value()?;
        let count = Self::vertex_count(player, member_ref)?;
        let pos = (index - 1).max(0) as usize;
        if pos >= count {
            return Err(ScriptError::new(format!(
                "vertex index {} out of range (count {})", index, count
            )));
        }
        Ok(player.alloc_datum(Datum::VectorVertexRef(member_ref.clone(), pos)))
    }

    /// setProp(member, #vertex, i, value) — set the i-th vertex location
    /// (point value) or replace the whole vertex (property-list value).
    fn set_vertex_by_index(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.len() < 3 {
            return Err(ScriptError::new(format!(
                "No handler setProp for vectorShape member"
            )));
        }
        let prop = player.get_datum(&args[0]).string_value().unwrap_or_default();
        if !prop.eq_ignore_ascii_case("vertex") {
            return Err(ScriptError::new(format!(
                "vectorShape setProp unsupported for {}", prop
            )));
        }
        let index = player.get_datum(&args[1]).int_value()?;
        let (pt, _) = player.get_datum(&args[2]).to_point_inline()?;
        let pos = (index - 1).max(0) as usize;
        Self::with_vs_mut(player, member_ref, |vs| {
            if let Some(v) = vs.vertices.get_mut(pos) {
                v.x = pt[0] as f32;
                v.y = pt[1] as f32;
                Ok(())
            } else {
                Err(ScriptError::new(format!(
                    "vertex index {} out of range", index
                )))
            }
        })?;
        Ok(DatumRef::Void)
    }

    /// Number of vertices in the referenced vectorShape member.
    pub fn vertex_count(
        player: &DirPlayer,
        member_ref: &CastMemberRef,
    ) -> Result<usize, ScriptError> {
        let cast_member = player
            .movie
            .cast_manager
            .find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
        match &cast_member.member_type {
            CastMemberType::VectorShape(vs) => Ok(vs.vertices.len()),
            _ => Err(ScriptError::new("Expected vectorShape member".to_string())),
        }
    }

    /// Read a `VectorVertexRef` sub-property (`vertex` / `handle1` /
    /// `handle2`) as a point. Used by `player_get_obj_prop`.
    pub fn get_vertex_ref_prop(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        index: usize,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        let cast_member = player
            .movie
            .cast_manager
            .find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
        let vs = match &cast_member.member_type {
            CastMemberType::VectorShape(vs) => vs,
            _ => return Err(ScriptError::new("Expected vectorShape member".to_string())),
        };
        let v = vs.vertices.get(index).ok_or_else(|| {
            ScriptError::new(format!("vertex index {} out of range", index + 1))
        })?;
        let pt = match_ci!(prop, {
            "vertex"  => (v.x, v.y),
            "handle1" => (v.handle1_x, v.handle1_y),
            "handle2" => (v.handle2_x, v.handle2_y),
            _ => return Err(ScriptError::new(format!(
                "VectorVertexRef has no property {}", prop
            ))),
        });
        Ok(Datum::Point([pt.0 as f64, pt.1 as f64], 0))
    }

    /// Write a `VectorVertexRef` sub-property (`vertex` / `handle1` /
    /// `handle2`) from a point and recompute the bbox. Used by
    /// `player_set_obj_prop`.
    pub fn set_vertex_ref_prop(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        index: usize,
        prop: &str,
        value: &Datum,
    ) -> Result<(), ScriptError> {
        let (pt, _) = value.to_point_inline()?;
        let prop = prop.to_owned();
        Self::with_vs_mut(player, member_ref, |vs| {
            let v = vs.vertices.get_mut(index).ok_or_else(|| {
                ScriptError::new(format!("vertex index {} out of range", index + 1))
            })?;
            match_ci!(prop.as_str(), {
                "vertex"  => { v.x = pt[0] as f32; v.y = pt[1] as f32; },
                "handle1" => { v.handle1_x = pt[0] as f32; v.handle1_y = pt[1] as f32; },
                "handle2" => { v.handle2_x = pt[0] as f32; v.handle2_y = pt[1] as f32; },
                _ => return Err(ScriptError::new(format!(
                    "VectorVertexRef has no property {}", prop
                ))),
            });
            Ok(())
        })
    }

    /// Borrow the member's `VectorShapeMember`, run `f`, then recompute the
    /// bbox so the rasterizer / `width()`/`height()` stay correct.
    fn with_vs_mut<F>(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        f: F,
    ) -> Result<(), ScriptError>
    where
        F: FnOnce(&mut crate::player::cast_member::VectorShapeMember) -> Result<(), ScriptError>,
    {
        let cast_member = player
            .movie
            .cast_manager
            .find_member_by_ref_mut(member_ref)
            .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
        let vs = match &mut cast_member.member_type {
            CastMemberType::VectorShape(vs) => vs,
            _ => return Err(ScriptError::new("Expected vectorShape member".to_string())),
        };
        f(vs)?;
        vs.recompute_bbox();
        Ok(())
    }

    fn add_vertex(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.len() < 2 {
            return Err(ScriptError::new(
                "addVertex requires (index, point)".to_string(),
            ));
        }
        let index = player.get_datum(&args[0]).int_value()?;
        let (pt, _) = player.get_datum(&args[1]).to_point_inline()?;
        // Optional Bezier control-handle offsets (relative to the vertex).
        let read_i = |i: usize| -> i32 {
            args.get(i)
                .map(|a| player.get_datum(a).int_value().unwrap_or(0))
                .unwrap_or(0)
        };
        let (h1x, h1y, h2x, h2y) =
            (read_i(2) as f32, read_i(3) as f32, read_i(4) as f32, read_i(5) as f32);
        let vertex = crate::director::enums::VectorShapeVertex {
            x: pt[0] as f32,
            y: pt[1] as f32,
            handle1_x: h1x,
            handle1_y: h1y,
            handle2_x: h2x,
            handle2_y: h2y,
        };
        let mut new_count = 0;
        Self::with_vs_mut(player, member_ref, |vs| {
            // Director uses a 1-based insert index; clamp into [0, len] so
            // index 1 prepends and index > count appends (matches Director
            // tolerating an out-of-range index by appending).
            let pos = ((index - 1).max(0) as usize).min(vs.vertices.len());
            vs.vertices.insert(pos, vertex);
            new_count = vs.vertices.len();
            Ok(())
        })?;
        Ok(player.alloc_datum(Datum::Int(new_count as i32)))
    }

    fn delete_vertex(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::new("deleteVertex requires (index)".to_string()));
        }
        let index = player.get_datum(&args[0]).int_value()?;
        let mut removed = false;
        Self::with_vs_mut(player, member_ref, |vs| {
            let pos = (index - 1).max(0) as usize;
            if pos < vs.vertices.len() {
                vs.vertices.remove(pos);
                removed = true;
            }
            Ok(())
        })?;
        Ok(player.alloc_datum(datum_bool(removed)))
    }

    fn move_vertex(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.len() < 3 {
            return Err(ScriptError::new(
                "moveVertex requires (index, dx, dy)".to_string(),
            ));
        }
        let index = player.get_datum(&args[0]).int_value()?;
        let dx = player.get_datum(&args[1]).int_value()? as f32;
        let dy = player.get_datum(&args[2]).int_value()? as f32;
        let mut moved = false;
        Self::with_vs_mut(player, member_ref, |vs| {
            let pos = (index - 1).max(0) as usize;
            if let Some(v) = vs.vertices.get_mut(pos) {
                v.x += dx;
                v.y += dy;
                moved = true;
            }
            Ok(())
        })?;
        Ok(player.alloc_datum(datum_bool(moved)))
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        // `regPoint` writes both vs.reg_point AND the outer
        // cast_member.reg_point (so the generic `the regPoint of member`
        // getter sees it). Apply outer first to avoid the vs-borrow
        // overlap, then vs.
        if prop.eq_ignore_ascii_case("regPoint") || prop.eq_ignore_ascii_case("originPoint") {
            let (vals, _) = value.to_point_inline()?;
            let cast_member = player
                .movie
                .cast_manager
                .find_member_by_ref_mut(member_ref)
                .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
            cast_member.reg_point = (vals[0] as i32, vals[1] as i32);
            if let CastMemberType::VectorShape(vs) = &mut cast_member.member_type {
                vs.reg_point = (vals[0] as i16, vals[1] as i16);
            }
            return Ok(());
        }

        // `member.vertexList = <list>` — replace the whole vertex list.
        // Director's vertexList is `[[#vertex: point, #handle1: point,
        // #handle2: point], ...]` (Director 11.5 Scripting Dictionary);
        // #handle1/#handle2 are optional control-point offsets relative to
        // the vertex. Parse the list (resolving sub-datums from `player`)
        // BEFORE taking the mutable member borrow, then assign + recompute
        // the bbox so the rasterizer / width()/height() stay correct.
        // parent_talkBox restores a backed-up shape via
        // `vsMem.vertexList = bupMem.vertexList`.
        if prop.eq_ignore_ascii_case("vertexList") {
            let items = match &value {
                Datum::List(_, items, _) => items.clone(),
                _ => return Err(ScriptError::new(
                    "vertexList expects a list".to_string(),
                )),
            };
            let mut verts: Vec<crate::director::enums::VectorShapeVertex> =
                Vec::with_capacity(items.len());
            let mut new_curve_count = 0usize;
            for item_ref in &items {
                let entry = player.get_datum(item_ref);
                // A `[#newCurve]` marker is a linear list holding the symbol
                // (not a prop-list). Count it and move on.
                if let Datum::List(_, elems, _) = &entry {
                    let is_new_curve = elems.iter().any(|e| {
                        player.get_datum(e).string_value()
                            .map_or(false, |s| s.eq_ignore_ascii_case("newCurve"))
                    });
                    if is_new_curve {
                        new_curve_count += 1;
                        continue;
                    }
                }
                let pairs = match entry {
                    Datum::PropList(pairs, _) => pairs.clone(),
                    _ => return Err(ScriptError::new(
                        "vertexList entry must be a property list".to_string(),
                    )),
                };
                // Tolerate a #newCurve key inside a prop-list form too (older
                // round-trips) — count it and skip the point parsing.
                let is_new_curve = pairs.iter().any(|(k_ref, _)| {
                    player.get_datum(k_ref).string_value()
                        .map_or(false, |k| k.eq_ignore_ascii_case("newCurve"))
                });
                if is_new_curve {
                    new_curve_count += 1;
                    continue;
                }
                let mut v = crate::director::enums::VectorShapeVertex {
                    x: 0.0, y: 0.0,
                    handle1_x: 0.0, handle1_y: 0.0,
                    handle2_x: 0.0, handle2_y: 0.0,
                };
                for (k_ref, val_ref) in &pairs {
                    // Keys are symbols (#vertex); string_value handles both
                    // Symbol and String forms.
                    let key = player.get_datum(k_ref).string_value().unwrap_or_default();
                    // Skip keys whose value isn't a point (defensive).
                    let pt = match player.get_datum(val_ref).to_point_inline() {
                        Ok((pt, _)) => pt,
                        Err(_) => continue,
                    };
                    match_ci!(key.as_str(), {
                        "vertex"  => { v.x = pt[0] as f32; v.y = pt[1] as f32; },
                        "handle1" => { v.handle1_x = pt[0] as f32; v.handle1_y = pt[1] as f32; },
                        "handle2" => { v.handle2_x = pt[0] as f32; v.handle2_y = pt[1] as f32; },
                        _ => {},
                    });
                }
                verts.push(v);
            }
            let cast_member = player
                .movie
                .cast_manager
                .find_member_by_ref_mut(member_ref)
                .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
            match &mut cast_member.member_type {
                CastMemberType::VectorShape(vs) => {
                    vs.vertices = verts;
                    vs.new_curve_count = new_curve_count;
                    vs.recompute_bbox();
                    return Ok(());
                }
                _ => return Err(ScriptError::new(
                    "Expected vectorShape member".to_string(),
                )),
            }
        }

        // VectorShapeMember stores colors as flat (u8, u8, u8) RGB tuples
        // because the rasterizer (`get_image`) writes raw RGB pixels.
        // Lingo can hand us either an Rgb color or a PaletteIndex (CS
        // catalog data files use both — most colors are `rgb(r,g,b)` but
        // some come through as `paletteIndex(N)` via `value()` parsing
        // of packed text records). The earlier setter only handled Rgb
        // and silently fell through on PaletteIndex, leaving
        // `vs.fill_color` unchanged — so `.image` kept returning the old
        // fill and the WFprev floor preview didn't update on color
        // picker changes. Resolve any PaletteIndex up-front (BEFORE the
        // mutable cast-member borrow) so both forms reach the setter.
        let resolved_color_rgb: Option<(u8, u8, u8)> = if prop.eq_ignore_ascii_case("fillColor")
            || prop.eq_ignore_ascii_case("endColor")
            || prop.eq_ignore_ascii_case("bgColor")
            || prop.eq_ignore_ascii_case("backgroundColor")
            || prop.eq_ignore_ascii_case("strokeColor")
        {
            let cref = value.to_color_ref()?.to_owned();
            let rgb = match &cref {
                ColorRef::Rgb(r, g, b) => (*r, *g, *b),
                ColorRef::PaletteIndex(_) => {
                    let palettes = player.movie.cast_manager.palettes();
                    let bitmap_palette =
                        crate::player::bitmap::bitmap::PaletteRef::BuiltIn(
                            crate::player::bitmap::bitmap::get_system_default_palette(),
                        );
                    crate::player::bitmap::bitmap::resolve_color_ref(
                        &palettes, &cref, &bitmap_palette, 8,
                    )
                }
            };
            Some(rgb)
        } else {
            None
        };

        let cast_member = player
            .movie
            .cast_manager
            .find_member_by_ref_mut(member_ref)
            .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
        let vs = match &mut cast_member.member_type {
            CastMemberType::VectorShape(vs) => vs,
            _ => return Err(ScriptError::new("Expected VectorShape member".to_string())),
        };

        match_ci!(prop, {
            // ---- Colors ------------------------------------------------
            "fillColor" => {
                if let Some(rgb) = resolved_color_rgb{ vs.fill_color = rgb; }
                Ok(())
            },
            "endColor" => {
                if let Some(rgb) = resolved_color_rgb{ vs.end_color = rgb; }
                Ok(())
            },
            "bgColor" | "backgroundColor" => {
                if let Some(rgb) = resolved_color_rgb{ vs.bg_color = rgb; }
                Ok(())
            },
            "strokeColor" => {
                if let Some(rgb) = resolved_color_rgb{ vs.stroke_color = rgb; }
                Ok(())
            },
            // ---- Stroke / shape ---------------------------------------
            "strokeWidth" => {
                vs.stroke_width = value.to_float()? as f32;
                Ok(())
            },
            "closed" => {
                vs.closed = value.int_value()? != 0;
                Ok(())
            },
            // ---- Fill mode + gradient ---------------------------------
            "fillMode" => {
                vs.fill_mode = parse_fill_mode(&value)?;
                Ok(())
            },
            "gradientType" => {
                vs.gradient_type = symbol_or_string_lc(&value)?;
                Ok(())
            },
            "fillScale" => {
                vs.fill_scale = value.to_float()? as f32;
                Ok(())
            },
            "fillDirection" => {
                vs.fill_direction = value.to_float()? as f32;
                Ok(())
            },
            "fillOffset" => {
                let (vals, _) = value.to_point_inline()?;
                vs.fill_offset = (vals[0] as i32, vals[1] as i32);
                Ok(())
            },
            "fillCycles" => {
                vs.fill_cycles = value.int_value()?;
                Ok(())
            },
            // ---- Display / scale / origin -----------------------------
            "scaleMode" => {
                vs.scale_mode = symbol_or_string_lc(&value)?;
                Ok(())
            },
            "scale" => {
                vs.scale = value.to_float()? as f32;
                Ok(())
            },
            "antialias" => {
                vs.antialias = value.int_value()? != 0;
                Ok(())
            },
            "centerRegPoint" => {
                let v = value.int_value()? != 0;
                vs.center_reg_point = v;
                // Director treats centerRegPoint and originMode=#point as
                // mutually exclusive: enabling centerRegPoint snaps origin
                // back to #center. (The reverse — originMode=#point clearing
                // centerRegPoint — is handled in the originMode arm.)
                if v {
                    vs.origin_mode = "center".to_string();
                }
                Ok(())
            },
            "regPointVertex" => {
                vs.reg_point_vertex = value.int_value()?;
                Ok(())
            },
            "directToStage" => {
                vs.direct_to_stage = value.int_value()? != 0;
                Ok(())
            },
            "originMode" => {
                let s = symbol_or_string_lc(&value)?;
                if s.eq_ignore_ascii_case("point") {
                    vs.center_reg_point = false;
                }
                vs.origin_mode = s;
                Ok(())
            },
            _ => Err(ScriptError::new(format!(
                "Cannot set VectorShape prop {}", prop
            ))),
        })
    }

    /// Allocate the `[[#vertex: point(x,y), ...], ...]` Lingo list for
    /// `the vertexList of member`. Split out so we can drop the
    /// cast-member borrow before doing the allocations.
    fn build_vertex_list(
        player: &mut DirPlayer,
        verts: Vec<(i32, i32, i32, i32, i32, i32, bool)>,
        new_curve_count: usize,
    ) -> Result<Datum, ScriptError> {
        let mut list: VecDeque<DatumRef> = verts
            .iter()
            .map(|(vx, vy, h1x, h1y, h2x, h2y, is_bezier)| {
                let vertex_key = player.alloc_datum(Datum::Symbol("vertex".to_string()));
                let vertex_val =
                    player.alloc_datum(Datum::Point([*vx as f64, *vy as f64], 0));
                let mut entries = vec![(vertex_key, vertex_val)];
                if *is_bezier {
                    let h1_key = player.alloc_datum(Datum::Symbol("handle1".to_string()));
                    let h1_val =
                        player.alloc_datum(Datum::Point([*h1x as f64, *h1y as f64], 0));
                    let h2_key = player.alloc_datum(Datum::Symbol("handle2".to_string()));
                    let h2_val =
                        player.alloc_datum(Datum::Point([*h2x as f64, *h2y as f64], 0));
                    entries.push((h1_key, h1_val));
                    entries.push((h2_key, h2_val));
                }
                let prop_list = Datum::PropList(VecDeque::from(entries), false);
                player.alloc_datum(prop_list)
            })
            .collect();
        // Trailing `[#newCurve]` markers (sub-path breaks). Director prints
        // these as a linear list holding just the symbol — `[#newCurve]` — not
        // a prop-list, so emit Datum::List([#newCurve]) to round-trip exactly.
        for _ in 0..new_curve_count {
            let sym = player.alloc_datum(Datum::Symbol("newCurve".to_string()));
            let marker = player.alloc_datum(Datum::List(
                DatumType::List,
                VecDeque::from(vec![sym]),
                false,
            ));
            list.push_back(marker);
        }
        Ok(Datum::List(DatumType::List, list, false))
    }

    /// `the image of member` — rasterizes the polygon into a fresh
    /// ephemeral bitmap (refcounted via DatumRef so it's freed when the
    /// last script reference goes away). Director's `member.image` for
    /// VectorShape produces a solid `fillColor` polygon; gradient /
    /// fillScale / fillOffset etc. don't affect the rasterized output.
    /// Verified against CS catalog `floor_shape_preview`.
    fn get_image(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
    ) -> Result<Datum, ScriptError> {
        // Snapshot everything we need from the cast member before we
        // need `&mut player.bitmap_manager`.
        let (w, h, fill, end, bg, stroke, stroke_width, fill_mode, closed, poly,
             gradient_type, fill_scale, fill_offset) = {
            let cast_member = player
                .movie
                .cast_manager
                .find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
            let vs = match &cast_member.member_type {
                CastMemberType::VectorShape(vs) => vs,
                _ => return Err(ScriptError::new("Expected vectorShape member".to_string())),
            };
            // For .image rasterization we want the vertex bbox dims (the
            // pixel extent the polygon actually occupies), NOT the
            // authored member.width/height which include extra padding.
            //
            // Guard against an absurd bbox: a corrupt/uninitialized vertex
            // can make bbox_width/height enormous, and `f32 as u16`
            // saturates to 65535 — a 65535² bitmap then overruns bitvec's
            // mask capacity and panics. No legitimate vector shape in a
            // Director movie is anywhere near this; clamp and log so the
            // player keeps running and we can see the offending member/bbox.
            const MAX_VS_IMAGE_DIM: f32 = 4096.0;
            let raw_w = vs.bbox_width().ceil();
            let raw_h = vs.bbox_height().ceil();
            if raw_w > MAX_VS_IMAGE_DIM || raw_h > MAX_VS_IMAGE_DIM
                || !raw_w.is_finite() || !raw_h.is_finite()
            {
                log::warn!(
                    "[vectorShape .image] member ({}, {}) has absurd bbox \
                     {}x{} (left={} top={} right={} bottom={}, {} verts) — \
                     clamping to {}px to avoid an oversized-bitmap panic",
                    member_ref.cast_lib, member_ref.cast_member,
                    raw_w, raw_h,
                    vs.bbox_left, vs.bbox_top, vs.bbox_right, vs.bbox_bottom,
                    vs.vertices.len(), MAX_VS_IMAGE_DIM,
                );
            }
            let w = raw_w.clamp(0.0, MAX_VS_IMAGE_DIM) as u16;
            let h = raw_h.clamp(0.0, MAX_VS_IMAGE_DIM) as u16;
            let fill = vs.fill_color;
            let end = vs.end_color;
            let bg = vs.bg_color;
            let stroke = vs.stroke_color;
            let stroke_width = vs.stroke_width;
            let fill_mode = vs.fill_mode;
            let closed = vs.closed;
            let bbox_left = vs.bbox_left;
            let bbox_top = vs.bbox_top;
            let gradient_type = vs.gradient_type.clone();
            let fill_scale = vs.fill_scale;
            let fill_offset = vs.fill_offset;
            // Tessellate the closed cubic-Bezier outline into a dense polygon
            // in bitmap-local coords. Each vertex carries two control-handle
            // offsets: handle1 = outgoing (toward the next vertex), handle2 =
            // incoming (from the previous vertex). For edge V[i]→V[i+1] the
            // cubic is V[i], V[i]+h1, V[i+1]+h2, V[i+1]. Without this the
            // rounded bubble/dialog shapes render as straight-edged polygons
            // (the "drawn by a child" look). Verified against ui_pratbubbla.
            let local: Vec<(f32, f32, f32, f32, f32, f32)> = vs
                .vertices
                .iter()
                .map(|v| (
                    v.x - bbox_left, v.y - bbox_top,
                    v.handle1_x, v.handle1_y,
                    v.handle2_x, v.handle2_y,
                ))
                .collect();
            let has_curves = local.iter().any(|v| {
                v.2 != 0.0 || v.3 != 0.0 || v.4 != 0.0 || v.5 != 0.0
            });
            let poly: Vec<(f32, f32)> = if has_curves && local.len() >= 2 {
                const SEGS: usize = 12; // samples per Bezier edge
                let n = local.len();
                let mut out: Vec<(f32, f32)> = Vec::with_capacity(n * SEGS);
                for i in 0..n {
                    let a = local[i];
                    let b = local[(i + 1) % n];
                    let p0 = (a.0, a.1);
                    let p1 = (a.0 + a.2, a.1 + a.3);          // V[i] + handle1 (out)
                    let p2 = (b.0 + b.4, b.1 + b.5);          // V[i+1] + handle2 (in)
                    let p3 = (b.0, b.1);
                    // Sample t in [0,1); the next edge contributes its own p0.
                    for s in 0..SEGS {
                        let t = s as f32 / SEGS as f32;
                        let mt = 1.0 - t;
                        let x = mt*mt*mt*p0.0 + 3.0*mt*mt*t*p1.0 + 3.0*mt*t*t*p2.0 + t*t*t*p3.0;
                        let y = mt*mt*mt*p0.1 + 3.0*mt*mt*t*p1.1 + 3.0*mt*t*t*p2.1 + t*t*t*p3.1;
                        out.push((x, y));
                    }
                }
                out
            } else {
                local.iter().map(|v| (v.0, v.1)).collect()
            };
            (w, h, fill, end, bg, stroke, stroke_width, fill_mode, closed, poly,
             gradient_type, fill_scale, fill_offset)
        };

        let mut bitmap = Bitmap::new(
            w.max(1),
            h.max(1),
            32,
            32,
            0,
            PaletteRef::BuiltIn(BuiltInPalette::GrayScale),
        );
        // The vector-shape image carries an alpha channel: inside the shape is
        // opaque (the fill), outside is transparent. spectral-wizard's
        // parent_talkBox builds the speech bubble via `_img.extractAlpha()` +
        // `useAlpha`, so the area outside the polygon MUST be alpha 0 or the
        // bubble's bounding box renders as a solid black rectangle.
        bitmap.use_alpha = true;
        let bw = bitmap.width as usize;
        let bh = bitmap.height as usize;

        // Pre-fill with TRANSPARENT BLACK. The RGB stays black (not vs.bg_color):
        // Director's vector-shape .image rasterizes onto a black backdrop
        // regardless of `the backgroundColor of member` — verified empirically
        // (`member("floor_shape_preview").backgroundColor` is rgb(255,255,255)
        // but `image.getPixel(2,2)` returns rgb(0,0,0)), and CS catalog scripts
        // key on that black via `[#ink: 5, #color: rgb(0,0,0), ...]`. The alpha
        // is 0 so alpha-aware consumers (extractAlpha / useAlpha) treat the
        // outside as transparent; color-key (ink 5) consumers still see black.
        let _ = bg;
        for y in 0..bh {
            for x in 0..bw {
                let i = (y * bw + x) * 4;
                bitmap.data[i] = 0;
                bitmap.data[i + 1] = 0;
                bitmap.data[i + 2] = 0;
                bitmap.data[i + 3] = 0x00;
            }
        }

        // Even-odd ray-cast point-in-polygon (closed polygon — last
        // vertex connects back to first).
        let point_in_poly = |px: f32, py: f32| -> bool {
            let n = poly.len();
            if n < 3 {
                return false;
            }
            let mut inside = false;
            let mut j = n - 1;
            for i in 0..n {
                let (xi, yi) = poly[i];
                let (xj, yj) = poly[j];
                let cond = (yi > py) != (yj > py)
                    && px < (xj - xi) * (py - yi) / (yj - yi + 1e-9) + xi;
                if cond {
                    inside = !inside;
                }
                j = i;
            }
            inside
        };

        // Only fill if `closed` is true. Director's Lingo `member.image`
        // for an open path produces an unfilled bitmap (just the bg) —
        // the SVG-style "implicit close for fill" is not applied. Mirrors
        // the same gate in drawing.rs::draw_vector_shape.
        //
        // Gradients:
        //  - `linear` (vertical) — t = y / (bh-1), lerp fill→end. fillDirection
        //    and fillCycles are not yet honoured (no concrete test case).
        //  - `radial` — origin at (bw/2 + fillOffset.x, bh/2 + fillOffset.y),
        //    radius ≈ half-bbox-diagonal × fillScale/100. The CS catalog
        //    `floor_shape_preview` is radial with offset (-80,+80) and
        //    fillScale 210; this approximation reproduces Director's
        //    `getPixel(53,23)` to within a few percent (t≈0.93 vs the
        //    authoritative 0.91). fillDirection (288°) is ignored — for
        //    a circular radial gradient it's a no-op; only relevant once
        //    we add elliptical/rotated gradients.
        let is_gradient = fill_mode == 2;
        let is_radial = is_gradient && gradient_type.eq_ignore_ascii_case("radial");
        let bh_minus_1 = (bh as f32 - 1.0).max(1.0);
        let radial_origin = (
            bw as f32 / 2.0 + fill_offset.0 as f32,
            bh as f32 / 2.0 + fill_offset.1 as f32,
        );
        let radial_radius = {
            let half_diag = ((bw as f32).powi(2) + (bh as f32).powi(2)).sqrt() / 2.0;
            (half_diag * fill_scale / 100.0).max(1.0)
        };
        let lerp_u8 = |a: u8, b: u8, t: f32| -> u8 {
            ((a as f32) * (1.0 - t) + (b as f32) * t).round().clamp(0.0, 255.0) as u8
        };
        let sample = |x: usize, y: usize| -> (u8, u8, u8) {
            if !is_gradient {
                return fill;
            }
            let t = if is_radial {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let dx = px - radial_origin.0;
                let dy = py - radial_origin.1;
                ((dx * dx + dy * dy).sqrt() / radial_radius).clamp(0.0, 1.0)
            } else {
                ((y as f32) / bh_minus_1).clamp(0.0, 1.0)
            };
            (
                lerp_u8(fill.0, end.0, t),
                lerp_u8(fill.1, end.1, t),
                lerp_u8(fill.2, end.2, t),
            )
        };
        if fill_mode > 0 && closed && poly.len() >= 3 {
            for y in 0..bh {
                let py = y as f32 + 0.5;
                for x in 0..bw {
                    let px = x as f32 + 0.5;
                    if point_in_poly(px, py) {
                        let c = sample(x, y);
                        let i = (y * bw + x) * 4;
                        bitmap.data[i] = c.0;
                        bitmap.data[i + 1] = c.1;
                        bitmap.data[i + 2] = c.2;
                        bitmap.data[i + 3] = 0xFF;
                    }
                }
            }
        } else if fill_mode == 1 && closed {
            // Degenerate (no-vertex) solid fill — flood whole bitmap.
            for y in 0..bh {
                for x in 0..bw {
                    let c = sample(x, y);
                    let i = (y * bw + x) * 4;
                    bitmap.data[i] = c.0;
                    bitmap.data[i + 1] = c.1;
                    bitmap.data[i + 2] = c.2;
                    bitmap.data[i + 3] = 0xFF;
                }
            }
        }

        // Anti-aliased stroke along the (tessellated) outline. Without this,
        // `member.image` produced only the fill — the talkbox / dialog shapes
        // lost their black border (the old opaque-black bbox used to hide its
        // absence). `blend_pixel_aa` accumulates alpha, so stroke pixels get
        // opaque even where they sit outside the fill (alpha 0). Mirrors the
        // stroke pass in drawing.rs::draw_vector_shape.
        if stroke_width > 0.0 && poly.len() >= 2 {
            let palettes = player.movie.cast_manager.palettes();
            let half_w = stroke_width / 2.0;
            let n = poly.len();
            let seg_count = if closed { n } else { n - 1 };
            for i in 0..seg_count {
                let (x1, y1) = poly[i];
                let (x2, y2) = poly[(i + 1) % n];
                bitmap.draw_line_aa(x1, y1, x2, y2, half_w, stroke, &palettes, 1.0);
            }
            // Round caps/joins at each point so corners stay smooth.
            for &(px, py) in &poly {
                bitmap.draw_circle_aa(px, py, half_w, stroke, &palettes, 1.0);
            }
        }

        let bitmap_id = player.bitmap_manager.add_ephemeral_bitmap(bitmap);
        Ok(Datum::BitmapRef(bitmap_id))
    }
}

/// `set the fillMode` accepts either a #symbol (`#none`/`#solid`/`#gradient`)
/// or a 0/1/2 integer. Map to the FLSH-stored u32 enum (offset 0x84).
fn parse_fill_mode(value: &Datum) -> Result<u32, ScriptError> {
    if let Datum::Symbol(s) = value {
        Ok(match_ci!(s.as_str(), {
            "none"     => 0u32,
            "solid"    => 1u32,
            "gradient" => 2u32,
            _ => return Err(ScriptError::new(format!("invalid fillMode {}", s))),
        }))
    } else {
        Ok(value.int_value()? as u32)
    }
}

/// Pull a string out of a Datum::Symbol or generic value, lowercase-normalized
/// — used by `gradientType`/`scaleMode`/`originMode` setters where Lingo
/// callers pass `#linear`, `"linear"`, `#Linear`, etc. interchangeably.
fn symbol_or_string_lc(value: &Datum) -> Result<String, ScriptError> {
    Ok(if let Datum::Symbol(s) = value {
        s.to_ascii_lowercase()
    } else {
        value.string_value()?.to_ascii_lowercase()
    })
}
