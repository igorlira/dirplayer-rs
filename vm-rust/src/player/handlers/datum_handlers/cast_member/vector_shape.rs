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
            // Director omits #handle1 / #handle2 keys for plain polygon
            // vertices (both handles == 0,0) and includes them only for
            // Bezier vertices. Verified against figure8 Slider Groove —
            // its 4 plain vertices come back as `[#vertex: point(...)]`
            // without handle keys.
            let verts: Vec<(i32, i32, i32, i32, i32, i32, bool)> = vs
                .vertices
                .iter()
                .map(|v| {
                    let h1x = v.handle1_x as i32;
                    let h1y = v.handle1_y as i32;
                    let h2x = v.handle2_x as i32;
                    let h2y = v.handle2_y as i32;
                    let is_bezier = !(h1x == 0 && h1y == 0 && h2x == 0 && h2y == 0);
                    (v.x as i32, v.y as i32, h1x, h1y, h2x, h2y, is_bezier)
                })
                .collect();
            return Self::build_vertex_list(player, verts);
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
    ) -> Result<Datum, ScriptError> {
        let list: VecDeque<DatumRef> = verts
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
        let (w, h, fill, end, bg, fill_mode, closed, poly,
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
            let w = vs.bbox_width().ceil() as u16;
            let h = vs.bbox_height().ceil() as u16;
            let fill = vs.fill_color;
            let end = vs.end_color;
            let bg = vs.bg_color;
            let fill_mode = vs.fill_mode;
            let closed = vs.closed;
            let bbox_left = vs.bbox_left;
            let bbox_top = vs.bbox_top;
            let gradient_type = vs.gradient_type.clone();
            let fill_scale = vs.fill_scale;
            let fill_offset = vs.fill_offset;
            // Translate vertices into bitmap-local coords.
            let poly: Vec<(f32, f32)> = vs
                .vertices
                .iter()
                .map(|v| (v.x - bbox_left, v.y - bbox_top))
                .collect();
            (w, h, fill, end, bg, fill_mode, closed, poly,
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
        let bw = bitmap.width as usize;
        let bh = bitmap.height as usize;

        // Pre-fill with BLACK, not vs.bg_color. Director's vector-shape
        // .image always rasterizes onto a black backdrop regardless of
        // `the backgroundColor of member` — verified empirically:
        // `member("floor_shape_preview").backgroundColor` returns
        // rgb(255,255,255) but `image.getPixel(2, 2)` returns rgb(0,0,0).
        // CS catalog scripts rely on this: they composite floor preview
        // shapes onto bitmaps with `[#ink: 5, #color: rgb(0,0,0), ...]`,
        // using black as the transparency key. Filling with vs.bg_color
        // (white) made the entire 106×46 rect contribute to the multiply
        // blend and washed the destination to gray.
        let _ = bg;
        for y in 0..bh {
            for x in 0..bw {
                let i = (y * bw + x) * 4;
                bitmap.data[i] = 0;
                bitmap.data[i + 1] = 0;
                bitmap.data[i + 2] = 0;
                bitmap.data[i + 3] = 0xFF;
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
