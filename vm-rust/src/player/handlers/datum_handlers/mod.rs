pub mod bitmap;
pub mod cast_member;
pub mod cast_member_ref;
pub mod color;
pub mod date;
pub mod int;
pub mod list_handlers;
pub mod math;
pub mod player;
pub mod point;
pub mod prop_list;
pub mod rect;
pub mod script;
pub mod script_instance;
pub mod sound_channel;
pub mod sprite;
pub mod string;
pub mod string_chunk;
pub mod symbol;
pub mod timeout;
pub mod vector;
pub mod void;
pub mod xml;

use player::PlayerDatumHandlers;

use self::date::DateDatumHandlers;
use self::math::MathDatumHandlers;
use self::vector::VectorDatumHandlers;
use self::xml::XmlDatumHandlers;
use self::{
    bitmap::BitmapDatumHandlers, list_handlers::ListDatumHandlers, point::PointDatumHandlers,
    prop_list::PropListDatumHandlers, rect::RectDatumHandlers, script::ScriptDatumHandlers,
    sound_channel::SoundChannelDatumHandlers, sprite::SpriteDatumHandlers,
    string::StringDatumHandlers, string_chunk::StringChunkHandlers, timeout::TimeoutDatumHandlers,
};
use crate::{
    director::lingo::datum::DatumType,
    player::{
        format_datum, reserve_player_mut, reserve_player_ref,
        xtra::manager::{
            call_xtra_instance_async_handler, call_xtra_instance_handler,
            has_xtra_instance_async_handler,
        },
        DatumRef, ScriptError, ScriptErrorCode,
    },
};

pub async fn player_call_datum_handler(
    obj_ref: &DatumRef,
    handler_name: &String,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    let datum_type = reserve_player_ref(|player| player.get_datum(obj_ref).type_enum());

    // let profile_token = start_profiling(format!("{}::{}", datum_type.type_str(), handler_name));
    let result = match datum_type {
        DatumType::List => ListDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::XmlChildNodes => ListDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::PropList => PropListDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::String => StringDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::StringChunk => StringChunkHandlers::call(obj_ref, handler_name, args),
        DatumType::ScriptRef => {
            if ScriptDatumHandlers::has_async_handler(obj_ref, handler_name) {
                ScriptDatumHandlers::call_async(obj_ref, handler_name, args).await
            } else {
                ScriptDatumHandlers::call(obj_ref, handler_name, args)
            }
        }
        DatumType::ScriptInstanceRef => {
            if script_instance::ScriptInstanceDatumHandlers::has_async_handler(
                obj_ref,
                handler_name,
            )? {
                script_instance::ScriptInstanceDatumHandlers::call_async(
                    obj_ref,
                    handler_name,
                    args,
                )
                .await
            } else {
                script_instance::ScriptInstanceDatumHandlers::call(obj_ref, handler_name, args)
            }
        }
        DatumType::TimeoutRef => {
            if TimeoutDatumHandlers::has_async_handler(handler_name) {
                TimeoutDatumHandlers::call_async(obj_ref, handler_name, args).await
            } else {
                TimeoutDatumHandlers::call(obj_ref, handler_name, args)
            }
        }
        DatumType::CastMemberRef => {
            cast_member_ref::CastMemberRefHandlers::call(obj_ref, handler_name, args)
        }
        DatumType::IntRect => RectDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::IntPoint => PointDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::BitmapRef => BitmapDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::SpriteRef => {
            if SpriteDatumHandlers::has_async_handler(obj_ref, handler_name)? {
                SpriteDatumHandlers::call_async(obj_ref.clone(), handler_name, args).await
            } else {
                SpriteDatumHandlers::call(obj_ref, handler_name, args)
            }
        }
        DatumType::XtraInstance => {
            let (xtra_name, instance_id) = reserve_player_ref(|player| {
                let (xtra_name, instance_id) =
                    player.get_datum(obj_ref).to_xtra_instance().unwrap();
                (xtra_name.clone(), instance_id.clone())
            });
            if has_xtra_instance_async_handler(&xtra_name, handler_name, instance_id) {
                call_xtra_instance_async_handler(&xtra_name, instance_id, handler_name, args).await
            } else {
                call_xtra_instance_handler(&xtra_name, instance_id, handler_name, args)
            }
        }
        DatumType::ColorRef => color::ColorDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::PlayerRef => PlayerDatumHandlers::call(handler_name, args),
        DatumType::XmlRef => XmlDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::DateRef => DateDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::MathRef => MathDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::Vector => VectorDatumHandlers::call(obj_ref, handler_name, args),
        DatumType::SoundChannel => reserve_player_mut(|player| {
            SoundChannelDatumHandlers::call(player, obj_ref, handler_name, args)
        }),
        _ => reserve_player_ref(|player| {
            let formatted_datum = format_datum(obj_ref, &player);
            Err(ScriptError::new_code(
                ScriptErrorCode::HandlerNotFound,
                format!("No handler {handler_name} for datum {}", formatted_datum),
            ))
        }),
    };
    // end_profiling(profile_token);
    result
}
