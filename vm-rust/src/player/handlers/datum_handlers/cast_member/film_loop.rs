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
        let film_loop_info = &film_loop.info;
        match prop.as_str() {
            "rect" => Ok(Datum::IntRect((
                0,
                0,
                film_loop_info.width as i32,
                film_loop_info.height as i32,
            ))),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for film loop",
                prop
            ))),
        }
    }
}
