use crate::{
    director::lingo::datum::{Datum, datum_bool},
    player::{
        DirPlayer, ScriptError,
        cast_lib::CastMemberRef,
        cast_member::ButtonType,
        handlers::datum_handlers::cast_member_ref::borrow_member_mut, symbols::{builtin::BuiltInSymbol, symbol::Symbol},
    },
};

pub struct ButtonMemberHandlers {}

impl ButtonMemberHandlers {
    pub fn call(
        player: &mut DirPlayer,
        datum: &crate::player::DatumRef,
        handler_name: Symbol,
        args: &Vec<crate::player::DatumRef>,
    ) -> Result<crate::player::DatumRef, ScriptError> {
        match handler_name.into_builtin_or_error()? {
            BuiltInSymbol::Count => {
                let member_ref = player.get_datum(datum).to_member_ref()?;
                let member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .unwrap();
                let button = member.member_type.as_button().unwrap();
                let count_of = player.get_datum(&args[0]).symbol_value()?;
                use crate::player::handlers::datum_handlers::string_chunk::StringChunkUtils;
                
                let delimiter = player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &button.field.text,
                    count_of.into(),
                    delimiter,
                )?;
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            }
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for button member"
            ))),
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: Symbol,
    ) -> Result<Datum, ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref)
            .unwrap();
        let button = member.member_type.as_button().unwrap();

        match prop.into_builtin_or_error()? {
            BuiltInSymbol::Text => Ok(Datum::String(button.field.text.to_owned())),
            BuiltInSymbol::Font => Ok(Datum::String(button.field.font.to_owned())),
            BuiltInSymbol::FontSize => Ok(Datum::Int(button.field.font_size as i32)),
            BuiltInSymbol::FontStyle => Ok(Datum::String(button.field.font_style.to_string())),
            BuiltInSymbol::Alignment => Ok(Datum::String(button.field.alignment.to_string())),
            BuiltInSymbol::Width => Ok(Datum::Int(button.field.width as i32)),
            BuiltInSymbol::Height => Ok(Datum::Int(button.field.height as i32)),
            BuiltInSymbol::Hilite => Ok(datum_bool(button.hilite)),
            BuiltInSymbol::ButtonType => Ok(Datum::Symbol(Symbol::builtin(button.button_type.symbol()))),
            BuiltInSymbol::ForeColor => {
                match &button.field.fore_color {
                    Some(crate::player::sprite::ColorRef::Rgb(r, _, _)) => Ok(Datum::Int(*r as i32)),
                    Some(crate::player::sprite::ColorRef::PaletteIndex(idx)) => Ok(Datum::Int(*idx as i32)),
                    None => Ok(Datum::Int(255)),
                }
            }
            BuiltInSymbol::WordWrap => Ok(datum_bool(button.field.word_wrap)),
            BuiltInSymbol::Border => Ok(Datum::Int(button.field.border as i32)),
            BuiltInSymbol::Editable => Ok(datum_bool(button.field.editable)),
            _ => Err(ScriptError::new(format!(
                "Button member doesn't support property {}",
                prop
            ))),
        }
    }

    pub fn set_prop(
        member_ref: &CastMemberRef,
        prop: Symbol,
        value: Datum,
    ) -> Result<(), ScriptError> {
        match prop.into_builtin_or_error()? {
            BuiltInSymbol::Text => borrow_member_mut(
                member_ref,
                |_player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_button_mut().unwrap().field.set_text_preserving_caret(value?.trim_end_matches('\0').to_string());
                    Ok(())
                },
            ),
            BuiltInSymbol::Hilite => borrow_member_mut(
                member_ref,
                |_player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_button_mut().unwrap().hilite = value?;
                    Ok(())
                },
            ),
            BuiltInSymbol::ButtonType => borrow_member_mut(
                member_ref,
                |_player| value.string_value(),
                |cast_member, value| {
                    let type_str = value?;
                    let button = cast_member.member_type.as_button_mut().unwrap();
                    match type_str.to_lowercase().as_str() {
                        "pushbutton" | "#pushbutton" => button.button_type = ButtonType::PushButton,
                        "checkbox" | "#checkbox" => button.button_type = ButtonType::CheckBox,
                        "radiobutton" | "#radiobutton" => button.button_type = ButtonType::RadioButton,
                        _ => return Err(ScriptError::new(format!("Unknown button type: {}", type_str))),
                    }
                    Ok(())
                },
            ),
            BuiltInSymbol::Font => borrow_member_mut(
                member_ref,
                |_player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_button_mut().unwrap().field.font = value?;
                    Ok(())
                },
            ),
            BuiltInSymbol::FontSize => borrow_member_mut(
                member_ref,
                |_player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_button_mut().unwrap().field.font_size = value? as u16;
                    Ok(())
                },
            ),
            BuiltInSymbol::Alignment => borrow_member_mut(
                member_ref,
                |_player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_button_mut().unwrap().field.alignment = Symbol::from_str(&value?).into_builtin_or_error()?;
                    Ok(())
                },
            ),
            BuiltInSymbol::Width => borrow_member_mut(
                member_ref,
                |_player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_button_mut().unwrap().field.width = value? as u16;
                    Ok(())
                },
            ),
            BuiltInSymbol::Height => borrow_member_mut(
                member_ref,
                |_player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_button_mut().unwrap().field.height = value? as u16;
                    Ok(())
                },
            ),
            _ => Err(ScriptError::new(format!(
                "Cannot set button member prop {}",
                prop
            ))),
        }
    }
}
