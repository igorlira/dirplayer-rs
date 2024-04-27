use crate::{
    director::lingo::datum::{Datum, StringChunkType},
    player::{
        cast_lib::CastMemberRef,
        handlers::datum_handlers::{
            cast_member_ref::borrow_member_mut, string_chunk::StringChunkUtils,
        },
        DatumRef, DirPlayer, ScriptError,
    },
};

pub struct FieldMemberHandlers {}

impl FieldMemberHandlers {
    pub fn call(
        player: &mut DirPlayer,
        datum: DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let member_ref = player.get_datum(datum).to_member_ref()?;
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(&member_ref)
            .unwrap();
        let field = member.member_type.as_field().unwrap();
        match handler_name.as_str() {
            "count" => {
                let count_of = player.get_datum(args[0]).string_value(&player.datums)?;
                if args.len() != 1 {
                    return Err(ScriptError::new("count requires 1 argument".to_string()));
                }
                let delimiter = &player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &field.text,
                    StringChunkType::from(&count_of),
                    delimiter,
                )?;
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            }
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for field member type"
            ))),
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref)
            .unwrap();
        let field = member.member_type.as_field().unwrap();
        match prop.as_str() {
            "text" => Ok(Datum::String(field.text.to_owned())),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for field",
                prop
            ))),
        }
    }

    pub fn set_prop(
        member_ref: &CastMemberRef,
        prop: &String,
        value: Datum,
    ) -> Result<(), ScriptError> {
        match prop.as_str() {
            "text" => borrow_member_mut(
                member_ref,
                |player| value.string_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().text = value?;
                    Ok(())
                },
            ),
            "rect" => borrow_member_mut(
                member_ref,
                |_| value.to_int_rect(),
                |cast_member, value| {
                    let value = value?;
                    let field_data = cast_member.member_type.as_field_mut().unwrap();
                    field_data.width = value.2 as u16;
                    Ok(())
                },
            ),
            "alignment" => borrow_member_mut(
                member_ref,
                |player| value.string_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().alignment = value?;
                    Ok(())
                },
            ),
            "wordWrap" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().word_wrap = value?;
                    Ok(())
                },
            ),
            "width" => borrow_member_mut(
                member_ref,
                |player| value.int_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().width = value? as u16;
                    Ok(())
                },
            ),
            "font" => borrow_member_mut(
                member_ref,
                |player| value.string_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().font = value?;
                    Ok(())
                },
            ),
            "fontSize" => borrow_member_mut(
                member_ref,
                |player| value.int_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().font_size = value? as u16;
                    Ok(())
                },
            ),
            "fontStyle" => borrow_member_mut(
                member_ref,
                |player| value.string_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().font_style = value?;
                    Ok(())
                },
            ),
            "fixedLineSpace" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(&player.datums),
                |cast_member, value| {
                    cast_member
                        .member_type
                        .as_field_mut()
                        .unwrap()
                        .fixed_line_space = value? as u16;
                    Ok(())
                },
            ),
            "topSpacing" => borrow_member_mut(
                member_ref,
                |player| value.int_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().top_spacing = value? as i16;
                    Ok(())
                },
            ),
            "boxType" => borrow_member_mut(
                member_ref,
                |player| value.string_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().box_type = value?;
                    Ok(())
                },
            ),
            "antialias" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().anti_alias = value?;
                    Ok(())
                },
            ),
            "autoTab" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().auto_tab = value?;
                    Ok(())
                },
            ),
            "editable" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().editable = value?;
                    Ok(())
                },
            ),
            "border" => borrow_member_mut(
                member_ref,
                |player| value.int_value(&player.datums),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().border = value? as u16;
                    Ok(())
                },
            ),
            _ => Err(ScriptError::new(format!(
                "Cannot set castMember prop {} for field",
                prop
            ))),
        }
    }
}
