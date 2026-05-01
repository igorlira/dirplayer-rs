use crate::{
    director::lingo::datum::Datum,
    player::{
        cast_lib::CastMemberRef, reserve_player_mut, DatumRef, ScriptError,
        ScriptErrorCode,
    },
};

pub struct CastLibDatumHandlers {}

impl CastLibDatumHandlers {
    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "getPropRef" | "getProp" => Self::get_prop_ref(datum, args),
            "count" => Self::count(datum, args),
            "findEmpty" => Self::find_empty(datum, args),
            _ => Err(ScriptError::new_code(
                ScriptErrorCode::HandlerNotFound,
                format!("No handler {handler_name} for castLib datum"),
            )),
        }
    }

    fn count(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let cast_lib_num = match player.get_datum(datum) {
                Datum::CastLib(num) => *num,
                _ => return Err(ScriptError::new("count: datum is not a castLib".to_string())),
            };

            // count(#member) returns the number of cast members
            if !args.is_empty() {
                let prop = player.get_datum(&args[0]).string_value().unwrap_or_default();
                if prop.eq_ignore_ascii_case("member") {
                    let cast = player.movie.cast_manager.get_cast(cast_lib_num)?;
                    return Ok(player.alloc_datum(Datum::Int(cast.members.len() as i32)));
                }
            }

            // Default: return member count
            let cast = player.movie.cast_manager.get_cast(cast_lib_num)?;
            Ok(player.alloc_datum(Datum::Int(cast.members.len() as i32)))
        })
    }

    fn get_prop_ref(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let cast_lib_num = match player.get_datum(datum) {
                Datum::CastLib(num) => *num,
                _ => {
                    return Err(ScriptError::new(
                        "getPropRef: datum is not a castLib".to_string(),
                    ))
                }
            };

            if args.is_empty() {
                return Err(ScriptError::new(
                    "getPropRef requires at least one argument".to_string(),
                ));
            } else if args.len() > 2 {
                return Err(ScriptError::new(
                    "getPropRef for castLib only supports one property".to_string(),
                ));
            }

            let prop_name = player.get_datum(&args[0]).symbol_value()?;

            match prop_name.to_lowercase().as_str() {
                "member" => {
                    if args.len() < 2 {
                        return Err(ScriptError::new(
                            "getPropRef(#member, ...) requires a member name or number".to_string(),
                        ));
                    }

                    let member_name_or_num = player.get_datum(&args[1]).clone();
                    let cast = player.movie.cast_manager.get_cast(cast_lib_num)?;

                    let member_ref = match &member_name_or_num {
                        Datum::String(name) => {
                            cast.find_member_by_name(name).map(|member| CastMemberRef {
                                cast_lib: cast_lib_num as i32,
                                cast_member: member.number as i32,
                            })
                        }
                        Datum::Int(num) => {
                            cast.find_member_by_number(*num as u32).map(|member| CastMemberRef {
                                cast_lib: cast_lib_num as i32,
                                cast_member: member.number as i32,
                            })
                        }
                        _ => {
                            return Err(ScriptError::new(format!(
                                "getPropRef(#member, ...) expects a string or int, got {}",
                                member_name_or_num.type_str()
                            )))
                        }
                    };

                    match member_ref {
                        Some(mr) => Ok(player.alloc_datum(Datum::CastMember(mr))),
                        None => {
                            // Return an invalid member ref (member 0) for non-existent members
                            Ok(player.alloc_datum(Datum::CastMember(CastMemberRef {
                                cast_lib: cast_lib_num as i32,
                                cast_member: 0,
                            })))
                        }
                    }
                }
                _ => Err(ScriptError::new(format!(
                    "getPropRef: unknown property #{} for castLib",
                    prop_name
                ))),
            }
        })
    }

    fn find_empty(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let cast_lib_num = match player.get_datum(datum) {
                Datum::CastLib(num) => *num,
                _ => {
                    return Err(ScriptError::new(
                        "findEmpty: datum is not a castLib".to_string(),
                    ))
                }
            };

            let (c_start, c_end) = match &player.movie.file {
                Some(file) => (file.config.min_member as u32, file.config.max_member as u32),
                None => return Err(ScriptError::new("findEmpty: no movie file loaded".to_string())),
            };

            let start = if !args.is_empty() {
                let member_ref = player.get_datum(&args[0]).to_member_ref()?;
                let member_num = member_ref.cast_member as u32;
                if member_num > c_end {
                    return Ok(player.alloc_datum(Datum::Int(member_num as i32)));
                }
                if member_num > c_start { member_num } else { c_start }
            } else {
                c_start
            };

            let cast = player.movie.cast_manager.get_cast(cast_lib_num)?;
            for slot in start..=c_end {
                if !cast.members.contains_key(&slot) {
                    return Ok(player.alloc_datum(Datum::Int(slot as i32)));
                }
            }
            Ok(player.alloc_datum(Datum::Int(c_end as i32 + 1)))
        })
    }
}
