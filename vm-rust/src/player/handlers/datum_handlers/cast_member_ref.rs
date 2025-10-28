use log::warn;

use crate::{
    director::lingo::datum::Datum,
    js_api::JsApi,
    player::{
        cast_lib::CastMemberRef,
        cast_member::{BitmapMember, CastMember, CastMemberType, CastMemberTypeId, TextMember},
        handlers::types::TypeUtils,
        reserve_player_mut, reserve_player_ref, DatumRef, DirPlayer, ScriptError,
    },
};

use super::cast_member::{
    bitmap::BitmapMemberHandlers, field::FieldMemberHandlers, film_loop::FilmLoopMemberHandlers,
    font::FontMemberHandlers, sound::SoundMemberHandlers, text::TextMemberHandlers,
};

pub struct CastMemberRefHandlers {}

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
            .unwrap();
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

    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "duplicate" => Self::duplicate(datum, args),
            "erase" => Self::erase(datum, args),
            "charPosToLoc" => {
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                        _ => {
                            return Err(ScriptError::new(
                                "Cannot call charPosToLoc on non-cast-member".to_string(),
                            ))
                        }
                    };
                    let cast_member = player
                        .movie
                        .cast_manager
                        .find_member_by_ref(&cast_member_ref)
                        .unwrap();
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let char_pos = player.get_datum(&args[0]).int_value()? as u16;
                    let char_width: u16 = 7; // TODO: Implement char width
                    let line_height = get_text_member_line_height(&text_data);
                    let result = if text_data.text.is_empty() || char_pos <= 0 {
                        Datum::IntPoint((0, 0))
                    } else if char_pos > text_data.text.len() as u16 {
                        Datum::IntPoint((
                            (char_width * (text_data.text.len() as u16)) as i32,
                            line_height as i32,
                        ))
                    } else {
                        Datum::IntPoint(((char_width * (char_pos - 1)) as i32, line_height as i32))
                    };
                    // TODO this is a stub!
                    Ok(player.alloc_datum(result))
                })
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
            _ => Self::call_member_type(datum, handler_name, args),
        }
    }

    fn call_member_type(
        datum: &DatumRef,
        handler_name: &String,
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
            let cast_member = player
                .movie
                .cast_manager
                .find_member_by_ref(&member_ref)
                .unwrap();
            match &cast_member.member_type {
                CastMemberType::Field(_) => {
                    FieldMemberHandlers::call(player, datum, handler_name, args)
                }
                CastMemberType::Text(_) => {
                    TextMemberHandlers::call(player, datum, handler_name, args)
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
            player
                .movie
                .cast_manager
                .remove_member_with_ref(&cast_member_ref)?;
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
            let dest_slot_number = args.get(0).map(|x| player.get_datum(x).int_value());

            if dest_slot_number.is_none() {
                return Err(ScriptError::new(
                    "Cannot duplicate cast member without destination slot number".to_string(),
                ));
            }
            let dest_slot_number = dest_slot_number.unwrap()?;
            let dest_ref = Self::member_ref_from_slot_number(dest_slot_number as u32);

            let mut new_member = {
                let src_member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&cast_member_ref);
                if src_member.is_none() {
                    return Err(ScriptError::new(
                        "Cannot duplicate non-existent cast member reference".to_string(),
                    ));
                }
                src_member.unwrap().clone()
            };
            new_member.number = dest_ref.cast_member as u32;

            let dest_cast = player
                .movie
                .cast_manager
                .get_cast_mut(dest_ref.cast_lib as u32);
            dest_cast.insert_member(dest_ref.cast_member as u32, new_member);

            Ok(player.alloc_datum(Datum::Int(dest_slot_number)))
        })
    }

    fn get_invalid_member_prop(
        _: &DirPlayer,
        member_ref: &CastMemberRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        match prop.as_str() {
            "name" => Ok(Datum::String("".to_string())),
            "number" => Ok(Datum::Int(-1)),
            "type" => Ok(Datum::String("empty".to_string())),
            "castLibNum" => Ok(Datum::Int(-1)),
            "width" => Ok(Datum::Void),
            "height" => Ok(Datum::Void),
            "rect" => Ok(Datum::Void),
            "duration" => Ok(Datum::Void),
            "memberNum" => Ok(Datum::Int(-1)),
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
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        match &member_type {
            CastMemberTypeId::Bitmap => {
                BitmapMemberHandlers::get_prop(player, cast_member_ref, prop)
            }
            CastMemberTypeId::Field => FieldMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Text => TextMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::FilmLoop => {
                FilmLoopMemberHandlers::get_prop(player, cast_member_ref, prop)
            }
            CastMemberTypeId::Sound => SoundMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Font => FontMemberHandlers::get_prop(player, cast_member_ref, prop),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember prop {} for member of type {:?}",
                prop, member_type
            ))),
        }
    }

    fn set_member_type_prop(
        member_ref: &CastMemberRef,
        prop: &String,
        value: Datum,
    ) -> Result<(), ScriptError> {
        let member_type = reserve_player_ref(|player| {
            let cast_member = player.movie.cast_manager.find_member_by_ref(member_ref);
            match cast_member {
                Some(cast_member) => Ok(cast_member.member_type.member_type_id()),
                None => Err(ScriptError::new(format!(
                    "Setting prop of invalid castMember reference"
                ))),
            }
        })?;

        match member_type {
            CastMemberTypeId::Field => FieldMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Text => TextMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Font => reserve_player_mut(|player| {
                FontMemberHandlers::set_prop(player, member_ref, prop, value)
            }),
            CastMemberTypeId::Bitmap => BitmapMemberHandlers::set_prop(member_ref, prop, value),
            _ => {
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
                            .unwrap();

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
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        let is_invalid = cast_member_ref.cast_lib < 0 || cast_member_ref.cast_member < 0;
        if is_invalid {
            return Self::get_invalid_member_prop(player, cast_member_ref, prop);
        }
        let cast_member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref);
        let (name, slot_number, member_type, color, bg_color, member_num) = match cast_member {
            Some(cast_member) => {
                let name = cast_member.name.to_owned();
                let slot_number = Self::get_cast_slot_number(
                    cast_member_ref.cast_lib as u32,
                    cast_member_ref.cast_member as u32,
                ) as i32;
                let member_type = cast_member.member_type.member_type_id();
                let member_num = cast_member.number;
                let color = cast_member.color.to_owned();
                let bg_color = cast_member.bg_color.to_owned();
                (name, slot_number, member_type, color, bg_color, member_num)
            }
            None => {
                warn!(
                    "Getting prop {} of non-existent castMember reference {}, {}",
                    prop, cast_member_ref.cast_lib, cast_member_ref.cast_member
                );
                return Self::get_invalid_member_prop(player, cast_member_ref, prop);
            }
        };

        match prop.as_str() {
            "name" => Ok(Datum::String(name)),
            "memberNum" => Ok(Datum::Int(member_num as i32)),
            "number" => Ok(Datum::Int(slot_number)),
            "type" => Ok(Datum::Symbol(member_type.symbol_string()?.to_string())),
            "castLibNum" => Ok(Datum::Int(cast_member_ref.cast_lib as i32)),
            "color" => Ok(Datum::ColorRef(color)),
            "bgColor" => Ok(Datum::ColorRef(bg_color)),
            _ => Self::get_member_type_prop(player, cast_member_ref, &member_type, prop),
        }
    }

    pub fn set_prop(
        cast_member_ref: &CastMemberRef,
        prop: &String,
        value: Datum,
    ) -> Result<(), ScriptError> {
        let is_invalid = cast_member_ref.cast_lib < 0 || cast_member_ref.cast_member < 0;
        if is_invalid {
            return Err(ScriptError::new(format!(
                "Setting prop of invalid castMember reference"
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
            match prop.as_str() {
                "name" => borrow_member_mut(
                    cast_member_ref,
                    |player| value.string_value(),
                    |cast_member, value| {
                        cast_member.name = value?;
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
            Err(ScriptError::new(format!(
                "Setting prop of invalid castMember reference"
            )))
        };
        if result.is_ok() {
            JsApi::dispatch_cast_member_changed(cast_member_ref.to_owned());
        }
        result
    }
}
