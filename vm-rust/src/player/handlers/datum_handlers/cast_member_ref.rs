use std::collections::VecDeque;

use log::{warn, debug};

use super::cast_member::{
    bitmap::BitmapMemberHandlers, button::ButtonMemberHandlers, field::FieldMemberHandlers,
    film_loop::FilmLoopMemberHandlers, font::FontMemberHandlers,
    havok::HavokPhysicsMemberHandlers,
    shockwave3d::Shockwave3dMemberHandlers,
    sound::SoundMemberHandlers, text::TextMemberHandlers, palette::PaletteMemberHandlers,
    vector_shape::VectorShapeMemberHandlers,
};

use crate::{
    director::{
        enums::{ScriptType, ShapeType},
        lingo::datum::{datum_bool, Datum, DatumType},
    },
    js_api::JsApi,
    player::{
        cast_lib::CastMemberRef,
        cast_member::{BitmapMember, CastMember, CastMemberType, CastMemberTypeId, TextMember},
        handlers::types::TypeUtils,
        reserve_player_mut, reserve_player_ref, DatumRef, DirPlayer, ScriptError,
        sprite::ColorRef,
    },
};

pub struct CastMemberRefHandlers {}

fn is_3d_member(datum: &DatumRef) -> Result<bool, ScriptError> {
    reserve_player_mut(|player| {
        let r = match player.get_datum(datum) {
            Datum::CastMember(r) => r.to_owned(),
            _ => return Ok(false),
        };
        Ok(player.movie.cast_manager.find_member_by_ref(&r)
            .map_or(false, |m| m.member_type.as_shockwave3d().is_some()))
    })
}

fn is_havok_member(datum: &DatumRef) -> Result<bool, ScriptError> {
    reserve_player_mut(|player| {
        let r = match player.get_datum(datum) {
            Datum::CastMember(r) => r.to_owned(),
            _ => return Ok(false),
        };
        Ok(player.movie.cast_manager.find_member_by_ref(&r)
            .map_or(false, |m| matches!(m.member_type, CastMemberType::HavokPhysics(_))))
    })
}

pub fn borrow_member_mut<T1, F1, T2, F2>(member_ref: &CastMemberRef, player_f: F2, f: F1) -> T1
where
    F1: FnOnce(&mut CastMember, T2) -> T1,
    F2: FnOnce(&mut DirPlayer) -> T2,
{
    reserve_player_mut(|player| {
        let arg = player_f(player);
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(&member_ref)
            .expect("cast member ref should be valid in borrow_member_mut");
        f(member, arg)
    })
}

fn get_text_member_line_height(text_data: &TextMember) -> u16 {
    return text_data.font_size + 3; // TODO: Implement text line height
}

impl CastMemberRefHandlers {
    pub fn get_cast_slot_number(cast_lib: u32, cast_member: u32) -> u32 {
        (cast_lib << 16) | (cast_member & 0xFFFF)
    }

    pub fn member_ref_from_slot_number(slot_number: u32) -> CastMemberRef {
        CastMemberRef {
            cast_lib: (slot_number >> 16) as i32,
            cast_member: (slot_number & 0xFFFF) as i32,
        }
    }

    /// Check if a cast member handler needs async dispatch.
    /// Currently only Havok "step" needs it (for step callbacks and collision interest callbacks).
    pub fn has_async_handler(datum: &DatumRef, handler_name: &str) -> bool {
        if handler_name != "step" { return false; }
        // Check if this is a Havok member
        reserve_player_ref(|player| {
            let r = match player.get_datum(datum) {
                Datum::CastMember(r) => r.to_owned(),
                _ => return false,
            };
            player.movie.cast_manager.find_member_by_ref(&r)
                .map_or(false, |m| matches!(m.member_type, CastMemberType::HavokPhysics(_)))
        })
    }

    /// Async handler for Havok step — runs physics with per-substep callback invocation.
    /// From the original engine (x86 sub_100175C0):
    ///   for each substep:
    ///     1. Integrate forces → velocity (Euler)
    ///     2. Apply actions (springs, dashpots, drag)
    ///     3. Step collision (Rapier)
    ///     4. Read back positions
    ///     5. Invoke step callbacks ← this is where Lingo handlers run
    ///   After all substeps:
    ///     6. Invoke collision interest callbacks
    ///     7. Sync to W3D models, clear forces
    pub fn call_async<'a>(
        datum: &'a DatumRef,
        _handler_name: &'a str,
        args: &'a Vec<DatumRef>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<DatumRef, ScriptError>> + 'a>> {
        Box::pin(async move {
            // Run the full physics step via the monolithic sync path.
            // This does: Euler integrate (full_dt) + Rapier substeps + readback + W3D sync + clear forces.
            let (step_result, step_cbs, collision_cbs) = HavokPhysicsMemberHandlers::step_with_callbacks(datum, args)?;

            // After the step, invoke step callbacks (async, post-step).
            for (cb_handler, cb_instance, dt_value) in &step_cbs {
                let dt_ref = reserve_player_mut(|player| {
                    player.alloc_datum(Datum::Float(*dt_value))
                });
                let _ = super::player_call_datum_handler(cb_instance, cb_handler, &vec![dt_ref]).await;
            }

            // Invoke collision interest callbacks (async, post-step).
            for (cb_handler, cb_instance, collision_info_ref) in &collision_cbs {
                let _ = super::player_call_datum_handler(cb_instance, cb_handler, &vec![collision_info_ref.clone()]).await;
            }

            Ok(step_result)
        })
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "duplicate" => Self::duplicate(datum, args),
            "erase" => Self::erase(datum, args),
            "charPosToLoc" => {
                let member_arg = datum.clone();
                let mut delegated_args: Vec<DatumRef> = Vec::with_capacity(args.len() + 1);
                delegated_args.push(member_arg);
                delegated_args.extend(args.iter().cloned());
                crate::player::handlers::manager::BuiltInHandlerManager::call_handler(
                    "charpostoloc",
                    &delegated_args,
                )
            }
            "getProp" => {
                let result_ref = reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                        _ => {
                            return Err(ScriptError::new(
                                "Cannot call getProp on non-cast-member".to_string(),
                            ))
                        }
                    };
                    let prop = player.get_datum(&args[0]).string_value()?;
                    let result = Self::get_prop(player, &cast_member_ref, &prop)?;
                    Ok(player.alloc_datum(result))
                })?;
                if args.len() > 1 {
                    reserve_player_mut(|player| {
                        TypeUtils::get_sub_prop(&result_ref, &args[1], player)
                    })
                } else {
                    Ok(result_ref)
                }
            }
            "getPropRef" => {
                if is_havok_member(datum)? {
                    HavokPhysicsMemberHandlers::call(datum, handler_name, args)
                } else if is_3d_member(datum)? {
                    Shockwave3dMemberHandlers::call(datum, handler_name, args)
                } else {
                    Self::call_member_type(datum, handler_name, args)
                        .or_else(|_| reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Void))))
                }
            }
            "count" => {
                if is_havok_member(datum)? {
                    HavokPhysicsMemberHandlers::call(datum, handler_name, args)
                } else if is_3d_member(datum)? {
                    Shockwave3dMemberHandlers::call(datum, handler_name, args)
                } else {
                    reserve_player_mut(|player| {
                        let cast_member_ref = match player.get_datum(datum) {
                            Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                            _ => {
                                return Err(ScriptError::new(
                                    "Cannot call count on non-cast-member".to_string(),
                                ))
                            }
                        };
                        if args.is_empty() {
                            return Err(ScriptError::new("count requires 1 argument".to_string()));
                        }
                        // Try to get the member's text
                        // First try "text" property, then fallback to "previewText" for Font members
                        let text = match Self::get_prop(player, &cast_member_ref, &"text".to_string()) {
                            Ok(datum) => datum.string_value()?,
                            Err(_) => {
                                // Try previewText for Font members
                                match Self::get_prop(player, &cast_member_ref, &"previewText".to_string()) {
                                    Ok(datum) => datum.string_value()?,
                                    Err(_) => {
                                        return Err(ScriptError::new(format!(
                                            "Member type does not support count operation"),
                                        ));
                                    }
                                }
                            }
                        };

                        let count_of = player.get_datum(&args[0]).string_value_cow()?;

                        let delimiter = player.movie.item_delimiter;
                        let chunk_type = std::panic::catch_unwind(|| crate::director::lingo::datum::StringChunkType::from(&*count_of))
                            .map_err(|_| ScriptError::new(format!("Invalid string chunk type: {}", count_of)))?;
                        let count = crate::player::handlers::datum_handlers::string_chunk::StringChunkUtils::resolve_chunk_count(
                            &text,
                            chunk_type,
                            delimiter,
                        )?;
                        Ok(player.alloc_datum(Datum::Int(count as i32)))
                    })
                }
            }
            // Havok Physics member handlers
            "initialize" | "Initialize" | "shutdown" | "shutDown" | "Shutdown"
            | "step" | "reset" | "rigidBody" | "rigidbody" | "spring"
            | "linearDashpot" | "lineardashpot" | "angularDashpot" | "angulardashpot"
            | "makeMovableRigidBody" | "makemovablerigidbody"
            | "makeFixedRigidBody" | "makefixedrigidbody"
            | "makeSpring" | "makespring"
            | "makeLinearDashpot" | "makelineardashpot"
            | "makeAngularDashpot" | "makeangulardashpot"
            | "deleteRigidBody" | "deleterigidbody"
            | "deleteSpring" | "deletespring"
            | "deleteLinearDashpot" | "deletelineardashpot"
            | "deleteAngularDashpot" | "deleteangulardashpot"
            | "registerInterest" | "registerinterest"
            | "removeInterest" | "removeinterest"
            | "registerStepCallback" | "registerstepcallback"
            | "removeStepCallback" | "removestepcallback"
            | "enableCollision" | "enablecollision"
            | "disableCollision" | "disablecollision"
            | "enableAllCollisions" | "enableallcollisions"
            | "disableAllCollisions" | "disableallcollisions" => {
                // Check if this is a Havok member before delegating
                let is_havok = reserve_player_ref(|player| {
                    let r = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return false,
                    };
                    player.movie.cast_manager.find_member_by_ref(&r)
                        .map_or(false, |m| matches!(m.member_type, CastMemberType::HavokPhysics(_)))
                });
                if is_havok {
                    HavokPhysicsMemberHandlers::call(datum, handler_name, args)
                } else {
                    Self::call_member_type(datum, handler_name, args)
                }
            }
            // Shockwave 3D member handlers — delegated to Shockwave3dMemberHandlers::call()
            "model" | "modelResource" | "shader" | "texture" | "light" | "camera" | "group" | "motion"
            | "resetWorld" | "revertToWorldDefaults"
            | "newTexture" | "newShader" | "newModel" | "newModelResource" | "newLight" | "newCamera" | "newGroup" | "newMotion" | "newMesh"
            | "deleteTexture" | "deleteShader" | "deleteModel" | "deleteModelResource" | "deleteLight" | "deleteCamera" | "deleteGroup" | "deleteMotion"
            | "cloneModelFromCastmember" | "cloneMotionFromCastmember" | "cloneDeep"
            | "loadFile" | "extrude3d" | "getPref" | "setPref"
            | "registerForEvent" | "registerScript"
            | "image"
            | "modelsUnderRay" | "modelsUnderLoc" | "modelUnderLoc" => {
                Shockwave3dMemberHandlers::call(datum, handler_name, args)
            }
            _ => Self::call_member_type(datum, handler_name, args),
        }
    }

    fn call_member_type(
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let member_ref = match player.get_datum(datum) {
                Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot call_member_type on non-cast-member".to_string(),
                    ))
                }
            };
            let cast_member = match player
                .movie
                .cast_manager
                .find_member_by_ref(&member_ref)
            {
                Some(m) => m,
                None => {
                    // preload/unload on non-existent members are no-ops in Director
                    if handler_name == "preload" || handler_name == "unload" {
                        return Ok(DatumRef::Void);
                    }
                    return Err(ScriptError::new(format!(
                        "Cannot call {} on non-existent member ({}, {})",
                        handler_name, member_ref.cast_lib, member_ref.cast_member
                    )));
                }
            };
            // preload/unload/stop/play/pause/rewind are no-ops for all member types in a web player
            if matches!(handler_name, "preload" | "unload" | "stop" | "play" | "pause" | "rewind") {
                return Ok(DatumRef::Void);
            }
            match &cast_member.member_type {
                CastMemberType::Field(_) => {
                    FieldMemberHandlers::call(player, datum, handler_name, args)
                }
                CastMemberType::Text(_) => {
                    TextMemberHandlers::call(player, datum, handler_name, args)
                }
                CastMemberType::Button(_) => {
                    ButtonMemberHandlers::call(player, datum, handler_name, args)
                }
                CastMemberType::HavokPhysics(_) => {
                    Err(ScriptError::new(format!("Havok handler {} should be dispatched from call()", handler_name)))
                }
                _ => Err(ScriptError::new(format!(
                    "No handler {handler_name} for member type"
                ))),
            }
        })
    }

    fn erase(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let cast_member_ref = match player.get_datum(datum) {
                Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                _ => return Err(ScriptError::new("Cannot erase non-cast-member".to_string())),
            };
            // Silently ignore invalid cast lib or non-existent members, matching Director behavior
            let _ = player
                .movie
                .cast_manager
                .remove_member_with_ref(&cast_member_ref);
            Ok(DatumRef::Void)
        })
    }

    fn duplicate(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let cast_member_ref = match player.get_datum(datum) {
                Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot duplicate non-cast-member".to_string(),
                    ))
                }
            };
            let dest_arg = args.get(0).map(|x| player.get_datum(x).clone());
            let dest_ref = match &dest_arg {
                Some(Datum::CastMember(r)) => r.clone(),
                Some(d) => {
                    let slot = d.int_value().map_err(|_| ScriptError::new(
                        "Cannot duplicate: expected member ref or slot number".to_string(),
                    ))?;
                    Self::member_ref_from_slot_number(slot as u32)
                }
                None => {
                    return Err(ScriptError::new(
                        "Cannot duplicate cast member without destination".to_string(),
                    ));
                }
            };

            let mut new_member = {
                let src_member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&cast_member_ref);
                match src_member {
                    Some(m) => m.clone(),
                    None => {
                        return Err(ScriptError::new(format!(
                            "Cannot duplicate cast member: source member not found (castLib {}, member {})",
                            cast_member_ref.cast_lib, cast_member_ref.cast_member
                        )));
                    }
                }
            };
            new_member.number = dest_ref.cast_member as u32;

            let dest_cast = player
                .movie
                .cast_manager
                .get_cast_mut(dest_ref.cast_lib as u32);
            dest_cast.insert_member(dest_ref.cast_member as u32, new_member);
            player.movie.cast_manager.invalidate_member_name_cache();
            player
                .movie
                .cast_manager
                .queue_texture_invalidation(dest_ref.clone());

            Ok(player.alloc_datum(Datum::CastMember(dest_ref)))
        })
    }

    fn get_invalid_member_prop(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        match prop {
            "name" => Ok(Datum::String("".to_string())),
            "number" => Ok(Datum::Int(-1)),
            "type" => Ok(Datum::String("empty".to_string())),
            "castLibNum" => Ok(Datum::Int(-1)),
            "memberNum" => Ok(Datum::Int(-1)),
            "text" | "comments" => Ok(Datum::String("".to_string())),
            "loaded" | "mediaReady" => Ok(Datum::Int(1)),
            "width" | "height" | "rect" | "duration" => Ok(Datum::Void),
            "image" => Ok(Datum::Void),
            "regPoint" => Ok(Datum::Point([0.0, 0.0], 0)),
            _ => Err(ScriptError::new(format!(
                "Cannot get prop {} of invalid cast member ({}, {})",
                prop, member_ref.cast_lib, member_ref.cast_member
            ))),
        }
    }

    fn get_member_type_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        member_type: &CastMemberTypeId,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        debug!("Getting prop '{}' for member type {:?}", prop, member_type);
        if prop.eq_ignore_ascii_case("regPoint") {
            // Text members with centerRegPoint use the CURRENT (measured) size for
            // the reg point, not the stored TextInfo.reg_x/reg_y which are authored
            // defaults that don't track wrapped-text expansion.
            if *member_type == CastMemberTypeId::Text {
                use crate::player::font::{measure_text, measure_text_wrapped};
                let member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                if let CastMemberType::Text(tm) = &member.member_type {
                    let is_center = tm.info.as_ref().map_or(false, |i| i.center_reg_point);
                    if is_center {
                        let authored_w = if tm.width > 0 {
                            tm.width
                        } else if let Some(ref info) = tm.info {
                            info.width as u16
                        } else { 0 };
                        // Measure current height (same logic as member.height getter).
                        let tm = tm.clone();
                        let cache_key = crate::player::font::FontManager::cache_key(&tm.font);
                        let font = player.font_manager.font_cache.get(&cache_key).cloned()
                            .or_else(|| player.font_manager.get_system_font());
                        let measured_h = font.as_ref().map(|f| {
                            if tm.word_wrap && authored_w > 0 {
                                measure_text_wrapped(
                                    &tm.text, f, authored_w, true,
                                    tm.fixed_line_space, tm.top_spacing, tm.bottom_spacing,
                                    tm.char_spacing,
                                ).1
                            } else {
                                measure_text(
                                    &tm.text, f, None,
                                    tm.fixed_line_space, tm.top_spacing, tm.bottom_spacing,
                                ).1
                            }
                        }).unwrap_or(tm.height);
                        let w = if authored_w > 0 { authored_w } else { tm.width };
                        let h = if measured_h > 0 { measured_h } else { tm.height };
                        return Ok(Datum::Point([(w as i32 / 2) as f64, (h as i32 / 2) as f64], 0));
                    }
                }
            }
            let member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
            let rp = member.reg_point;
            return Ok(Datum::Point([rp.0 as f64, rp.1 as f64], 0));
        }
        match &member_type {
            CastMemberTypeId::Bitmap => {
                BitmapMemberHandlers::get_prop(player, cast_member_ref, prop)
            }
            CastMemberTypeId::Field => FieldMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Text => {
                TextMemberHandlers::get_prop(player, cast_member_ref, prop)
                    .or_else(|_| {
                        // Forward to 3D handler if text member has embedded 3D world
                        if player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                            .and_then(|m| m.member_type.as_shockwave3d()).is_some()
                        {
                            Shockwave3dMemberHandlers::get_prop(player, cast_member_ref, prop)
                        } else {
                            Err(ScriptError::new(format!(
                                "Cannot get castMember property {} for text", prop
                            )))
                        }
                    })
            },
            CastMemberTypeId::Button => ButtonMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::FilmLoop => {
                FilmLoopMemberHandlers::get_prop(player, cast_member_ref, prop)
            }
            CastMemberTypeId::Sound => SoundMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Font => FontMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Palette => PaletteMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Shockwave3d => Shockwave3dMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::HavokPhysics => HavokPhysicsMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Script => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                let script_data = match &cast_member.member_type {
                    CastMemberType::Script(s) => s,
                    _ => return Err(ScriptError::new("Cast member is not a script".to_string())),
                };
                match prop {
                    "text" => Ok(Datum::String("".to_string())),
                    "script" => Ok(Datum::ScriptRef(cast_member_ref.clone())),
                    "scriptText" => Ok(Datum::String("".to_string())),
                    "scriptType" => {
                        let symbol = match script_data.script_type {
                            ScriptType::Movie => "movie",
                            ScriptType::Parent => "parent",
                            ScriptType::Score => "score",
                            ScriptType::Member => "member",
                            _ => "unknown",
                        };
                        Ok(Datum::Symbol(symbol.to_string()))
                    }
                    "ilk" => Ok(Datum::Symbol("script".to_string())),
                    _ => Err(ScriptError::new(format!("Script members don't support property {}", prop))),
                }
            }
            CastMemberTypeId::Shape => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;

                if let CastMemberType::Shape(shape_member) = &cast_member.member_type {
                    let info = &shape_member.shape_info;
                    match prop {
                        "rect" => {
                            let width = info.width() as f64;
                            let height = info.height() as f64;
                            Ok(Datum::Rect([0.0, 0.0, width, height], 0))
                        }
                        "width" => Ok(Datum::Int(info.width() as i32)),
                        "height" => Ok(Datum::Int(info.height() as i32)),
                        "shapeType" => {
                            let symbol = match info.shape_type {
                                ShapeType::Rect => "rect",
                                ShapeType::OvalRect => "roundRect",
                                ShapeType::Oval => "oval",
                                ShapeType::Line => "line",
                                ShapeType::Unknown => "rect",
                            };
                            Ok(Datum::Symbol(symbol.to_string()))
                        }
                        "filled" => Ok(datum_bool(info.fill_type != 0)),
                        // Director stores line thickness 1-based: 1 = no
                        // border ("hairline" / invisible), 2 = 1px, 3 = 2px,
                        // …, 6 = 5px (max). The Lingo `lineSize of member`
                        // getter returns the 0-based form (file value − 1),
                        // so file=1 → Lingo=0 (matches Director on a
                        // sprite-4 shape with no border).
                        "lineSize" => {
                            let raw = info.line_thickness as i32;
                            Ok(Datum::Int((raw - 1).max(0)))
                        }
                        "pattern" => Ok(Datum::Int(info.pattern as i32)),
                        "foreColor" => Ok(Datum::Int(info.fore_color as i32)),
                        "backColor" => Ok(Datum::Int(info.back_color as i32)),
                        _ => Err(ScriptError::new(format!(
                            "Shape members don't support property {}", prop
                        ))),
                    }
                } else {
                    Err(ScriptError::new("Expected shape member".to_string()))
                }
            }
            CastMemberTypeId::VectorShape => {
                return VectorShapeMemberHandlers::get_prop(player, cast_member_ref, prop);
            }
            CastMemberTypeId::Flash => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                if let CastMemberType::Flash(flash) = &cast_member.member_type {
                    let (l, t, r, b) = flash.effective_rect();
                    match prop {
                        "width" => Ok(Datum::Int((r - l) as i32)),
                        "height" => Ok(Datum::Int((b - t) as i32)),
                        "rect" => Ok(Datum::Rect([l as f64, t as f64, r as f64, b as f64], 0)),
                        "regPoint" => {
                            let rp = flash.reg_point;
                            Ok(Datum::Point([rp.0 as f64, rp.1 as f64], 0))
                        }
                        _ => Ok(Datum::Void),
                    }
                } else {
                    Ok(Datum::Void)
                }
            }
            _ => {
                // SWA/streaming media properties — return sensible defaults
                match prop {
                    "soundChannel" => Ok(Datum::Int(0)),
                    "preloadTime" => Ok(Datum::Int(0)),
                    "volume" => Ok(Datum::Int(0)),
                    "url" => Ok(Datum::String(String::new())),
                    "state" => Ok(Datum::Int(0)), // 0=stopped
                    "currentTime" => Ok(Datum::Int(0)),
                    "duration" => Ok(Datum::Int(0)),
                    "percentPlayed" => Ok(Datum::Int(0)),
                    "percentStreamed" => Ok(Datum::Int(0)),
                    "loop" => Ok(Datum::Int(0)),
                    "pausedAtStart" => Ok(Datum::Int(0)),
                    _ => Err(ScriptError::new(format!(
                        "Cannot get castMember prop {} for member of type {:?}",
                        prop, member_type
                    ))),
                }
            }
        }
    }

    fn set_member_type_prop(
        member_ref: &CastMemberRef,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        let member_type = reserve_player_ref(|player| {
            let cast_member = player.movie.cast_manager.find_member_by_ref(member_ref);
            match cast_member {
                Some(cast_member) => Ok(Some(cast_member.member_type.member_type_id())),
                None => {
                    // Silently ignore setting props on erased members
                    web_sys::console::warn_1(&format!(
                        "Ignoring set prop {} on erased member {} of castLib {}",
                        prop, member_ref.cast_member, member_ref.cast_lib
                    ).into());
                    Ok(None)
                }
            }
        })?;

        let member_type = match member_type {
            Some(t) => t,
            None => return Ok(()), // Member was erased, silently ignore
        };

        if prop.eq_ignore_ascii_case("regPoint") {
            return reserve_player_mut(|player| {
                let (vals, _flags) = value.to_point_inline()?;
                let x = vals[0] as i32;
                let y = vals[1] as i32;
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                member.reg_point = (x, y);
                if let CastMemberType::Bitmap(ref mut bm) = member.member_type {
                    bm.reg_point = (x as i16, y as i16);
                }
                Ok(())
            });
        }

        // Handle Script-specific props before the main match so unrecognized
        // props fall through to the wildcard arm (e.g. implicit bitmap conversion).
        if member_type == CastMemberTypeId::Script {
            match prop {
                "scriptText" => return Ok(()), // No-op: no lingo compiler
                "scriptType" => {
                    let type_str = value.string_value()?;
                    let script_type = match type_str.to_lowercase().as_str() {
                        "movie" => ScriptType::Movie,
                        "parent" => ScriptType::Parent,
                        "score" => ScriptType::Score,
                        "member" => ScriptType::Member,
                        _ => return Err(ScriptError::new(format!("Unknown scriptType: {}", type_str))),
                    };
                    return borrow_member_mut(
                        member_ref,
                        |_| {},
                        |cast_member, _| {
                            if let CastMemberType::Script(ref mut s) = cast_member.member_type {
                                s.script_type = script_type;
                            }
                            Ok(())
                        },
                    );
                }
                _ => {} // Fall through to main match
            }
        }

        match member_type {
            CastMemberTypeId::Field => FieldMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Text => {
                let text_result = TextMemberHandlers::set_prop(member_ref, prop, value.clone());
                if text_result.is_err() {
                    // Forward to 3D handler if text member has embedded 3D world
                    let has_w3d = reserve_player_ref(|player| {
                        player.movie.cast_manager.find_member_by_ref(member_ref)
                            .and_then(|m| m.member_type.as_shockwave3d()).is_some()
                    });
                    if has_w3d {
                        reserve_player_mut(|player| {
                            Shockwave3dMemberHandlers::set_prop(player, member_ref, prop, &value)
                        })
                    } else {
                        text_result
                    }
                } else {
                    text_result
                }
            },
            CastMemberTypeId::Button => ButtonMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Font => reserve_player_mut(|player| {
                FontMemberHandlers::set_prop(player, member_ref, prop, value)
            }),
            CastMemberTypeId::Bitmap => BitmapMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Sound => SoundMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Palette => reserve_player_mut(|player| {
                PaletteMemberHandlers::set_prop(player, member_ref, prop, value)
            }),
            CastMemberTypeId::VectorShape => reserve_player_mut(|player| {
                VectorShapeMemberHandlers::set_prop(player, member_ref, prop, value)
            }),
            CastMemberTypeId::Flash => {
                // Flash members accept various properties silently
                // (directToStage, quality, scaleMode, etc.)
                Ok(())
            }
            CastMemberTypeId::Shockwave3d => reserve_player_mut(|player| {
                Shockwave3dMemberHandlers::set_prop(player, member_ref, prop, &value)
            }),
            CastMemberTypeId::HavokPhysics => {
                HavokPhysicsMemberHandlers::set_prop(member_ref, prop, value)
            }
            _ => {
                // SWA/streaming media properties — accept silently as no-ops
                if matches!(prop, "soundChannel" | "preloadTime" | "volume" | "url"
                    | "state" | "currentTime" | "duration" | "percentPlayed"
                    | "percentStreamed" | "loop" | "pausedAtStart") {
                    return Ok(());
                }
                // Check if this is a bitmap-specific property being set on a non-bitmap
                if prop == "image"
                    || prop == "regPoint"
                    || prop == "paletteRef"
                    || prop == "palette"
                {
                    // Director allows setting bitmap properties on non-bitmap members
                    // by implicitly converting them to bitmap members
                    reserve_player_mut(|player| {
                        let cast_member = player
                            .movie
                            .cast_manager
                            .find_mut_member_by_ref(member_ref)
                            .expect("cast member ref should be valid in set_member_type_prop");

                        // If not already a bitmap, convert it
                        if cast_member.member_type.as_bitmap().is_none() {
                            // Create a new empty/default bitmap member
                            let new_bitmap = BitmapMember::default();

                            // Replace the member type
                            cast_member.member_type = CastMemberType::Bitmap(new_bitmap);
                        }

                        Ok(())
                    })?;
                    // Now try setting the property again
                    BitmapMemberHandlers::set_prop(member_ref, prop, value)
                } else {
                    Err(ScriptError::new(format!(
                        "Cannot set castMember prop {} for member of type {:?}",
                        prop, member_type
                    )))
                }
            }
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        let is_invalid = cast_member_ref.cast_lib < 0 || cast_member_ref.cast_member < 0;
        if is_invalid {
            return Self::get_invalid_member_prop(player, cast_member_ref, prop);
        }
        let cast_member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref);
        let (name, comments, slot_number, member_type, color, bg_color, member_num) = match cast_member {
            Some(cast_member) => {
                let name = cast_member.name.to_owned();
                let comments = cast_member.comments.to_owned();
                let slot_number = Self::get_cast_slot_number(
                    cast_member_ref.cast_lib as u32,
                    cast_member_ref.cast_member as u32,
                ) as i32;
                let member_type = cast_member.member_type.member_type_id();
                let member_num = cast_member.number;
                let color = cast_member.color.to_owned();
                let bg_color = cast_member.bg_color.to_owned();
                (name, comments, slot_number, member_type, color, bg_color, member_num)
            }
            None => {
                warn!(
                    "Getting prop {} of non-existent castMember reference {}, {}",
                    prop, cast_member_ref.cast_lib, cast_member_ref.cast_member
                );
                return Self::get_invalid_member_prop(player, cast_member_ref, prop);
            }
        };

        match prop {
            "name" => Ok(Datum::String(name)),
            "memberNum" => Ok(Datum::Int(member_num as i32)),
            "number" => {
                if player.movie.dir_version >= 600 {
                    Ok(Datum::Int(slot_number))
                } else {
                    Ok(Datum::Int(member_num as i32))
                }
            }
            "type" => Ok(Datum::Symbol(member_type.symbol_string()?.to_string())),
            "castLibNum" => Ok(Datum::Int(cast_member_ref.cast_lib as i32)),
            "color" => Ok(Datum::ColorRef(color)),
            "bgColor" => Ok(Datum::ColorRef(bg_color)),
            "loaded" => Ok(Datum::Int(1)),
            "mediaReady" => Ok(Datum::Int(1)),
            "comments" => Ok(Datum::String(comments)),
            "ilk" => Ok(Datum::Symbol("member".to_string())),
            // In Director, member.member returns the member reference itself
            "member" => Ok(Datum::CastMember(cast_member_ref.clone())),
            _ => Self::get_member_type_prop(player, cast_member_ref, &member_type, prop),
        }
    }

    pub fn set_prop(
        cast_member_ref: &CastMemberRef,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        let is_invalid = cast_member_ref.cast_lib < 0 || cast_member_ref.cast_member < 0;
        if is_invalid {
            return Err(ScriptError::new(format!(
                "Setting prop {} of invalid castMember reference (member {} of castLib {})",
                prop, cast_member_ref.cast_member, cast_member_ref.cast_lib
            )));
        }
        let exists = reserve_player_ref(|player| {
            player
                .movie
                .cast_manager
                .find_member_by_ref(cast_member_ref)
                .is_some()
        });
        let result = if exists {
            match prop {
                "name" => borrow_member_mut(
                    cast_member_ref,
                    |_player| value.string_value(),
                    |cast_member, value| {
                        cast_member.name = value?;
                        Ok(())
                    },
                ),
                "comments" => borrow_member_mut(
                    cast_member_ref,
                    |_player| value.string_value(),
                    |cast_member, value| {
                        cast_member.comments = value?;
                        Ok(())
                    },
                ),
                "color" => borrow_member_mut(
                    cast_member_ref,
                    |_| {},
                    |cast_member, _| {
                        cast_member.color = value.to_color_ref()?.to_owned();
                        Ok(())
                    },
                ),
                "bgColor" => borrow_member_mut(
                    cast_member_ref,
                    |_| {},
                    |cast_member, _| {
                        cast_member.bg_color = value.to_color_ref()?.to_owned();
                        Ok(())
                    },
                ),
                _ => Self::set_member_type_prop(cast_member_ref, prop, value),
            }
        } else {
            // Silently ignore setting props on non-existent members
            // This can happen when a script erases a member but still holds a reference
            // Director silently ignores this case
            web_sys::console::warn_1(&format!(
                "Ignoring set prop {} on erased member {} of castLib {}",
                prop, cast_member_ref.cast_member, cast_member_ref.cast_lib
            ).into());
            Ok(())
        };
        if result.is_ok() {
            if prop == "name" {
                reserve_player_mut(|player| {
                    player.movie.cast_manager.invalidate_member_name_cache();
                });
                JsApi::on_cast_member_name_changed(Self::get_cast_slot_number(
                    cast_member_ref.cast_lib as u32,
                    cast_member_ref.cast_member as u32,
                ));
            }
            JsApi::dispatch_cast_member_changed(cast_member_ref.to_owned());
        }
        result
    }
}
