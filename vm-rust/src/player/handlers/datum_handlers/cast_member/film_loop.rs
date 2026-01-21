use crate::{
    director::lingo::datum::Datum,
    player::{cast_lib::CastMemberRef, DirPlayer, ScriptError},
};

pub struct FilmLoopMemberHandlers {}

impl FilmLoopMemberHandlers {
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
        let film_loop = member.member_type.as_film_loop().unwrap();

        // The filmloop's rect is stored in info as:
        // - reg_point = (left, top) coordinates
        // - width = right coordinate
        // - height = bottom coordinate
        // So rect = (reg_point.0, reg_point.1, width, height)
        // And actual dimensions = (width - reg_point.0, height - reg_point.1)
        let rect_left = film_loop.info.reg_point.0 as i32;
        let rect_top = film_loop.info.reg_point.1 as i32;
        let rect_right = film_loop.info.width as i32;
        let rect_bottom = film_loop.info.height as i32;
        let rect_width = rect_right - rect_left;
        let rect_height = rect_bottom - rect_top;

        match prop.as_str() {
            "rect" => Ok(Datum::Rect([
                player.alloc_datum(Datum::Int(rect_left)),
                player.alloc_datum(Datum::Int(rect_top)),
                player.alloc_datum(Datum::Int(rect_right)),
                player.alloc_datum(Datum::Int(rect_bottom)),
            ])),
            "width" => Ok(Datum::Int(rect_width)),
            "height" => Ok(Datum::Int(rect_height)),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for film loop",
                prop
            ))),
        }
    }
}
