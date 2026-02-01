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
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "getPropRef" => Self::get_prop_ref(datum, args),
            _ => Err(ScriptError::new_code(
                ScriptErrorCode::HandlerNotFound,
                format!("No handler {handler_name} for castLib datum"),
            )),
        }
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
}
