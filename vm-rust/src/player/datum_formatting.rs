use crate::{director::lingo::datum::Datum, player::sprite::ColorRef};

use super::{get_datum, DatumRef, DirPlayer};

pub fn format_concrete_datum(datum: &Datum, player: &DirPlayer) -> String {
  match datum {
    Datum::String(s) => format!("\"{s}\""),
    Datum::Int(i) => i.to_string(),
    Datum::Float(f) => {
      match player.float_precision {
        1 => return format!("{:.1}", f),
        2 => return format!("{:.2}", f),
        3 => return format!("{:.3}", f),
        4 => return format!("{:.4}", f),
        5 => return format!("{:.5}", f),
        6 => return format!("{:.6}", f),
        _ => f.to_string(),
      }
    },
    Datum::List(_, items, _) => {
      let formatted_items: Vec<String> = items.iter().map(|x| format_datum(*x, player)).collect();
      format!("[{}]", formatted_items.join(", "))
    }
    Datum::VarRef(_) => "VarRef".to_string(),
    Datum::Void => "Void".to_string(),
    Datum::Symbol(s) => format!("#{s}"),
    Datum::CastLib(n) => format!("castLib({n})"),
    Datum::Stage => "the stage".to_string(),
    Datum::PropList(entries, ..) => {
      if entries.is_empty() {
        return "[:]".to_string();
      }
      let formatted_entries: Vec<String> = entries.iter().map(|(k, v)| format!("{}: {}", format_datum(*k, player), format_datum(*v, player))).collect();
      format!("[{}]", formatted_entries.join(", "))
    }
    Datum::StringChunk(..) => format!("\"{}\"", datum.string_value(&player.datums).unwrap_or("!!!ERR!!!".to_string())),
    Datum::ScriptRef(member_ref) => {
      let script = player.movie.cast_manager.get_script_by_ref(&member_ref).unwrap();
      format!("(script {})", script.name)
    }
    Datum::ScriptInstanceRef(instance_id) => {
      let instance = player.script_instances.get(instance_id).unwrap();
      let script = player.movie.cast_manager.get_script_by_ref(&instance.script).unwrap();

      format!("<offspring {} {} _>", script.name, instance_id)
    }
    Datum::CastMember(member_ref) => {
      format!("(member {} of castLib {})", member_ref.cast_member, member_ref.cast_lib)
    }
    Datum::SpriteRef(sprite_ref) => {
      format!("(sprite {})", sprite_ref)
    }
    Datum::IntRect((x1, y1, x2, y2)) => {
      format!("rect({}, {}, {}, {})", x1, y1, x2, y2)
    }
    Datum::IntPoint((x, y)) => {
      format!("point({}, {})", x, y)
    }
    Datum::CursorRef(_) => {
      format!("<cursor>")
    }
    Datum::TimeoutRef(name) => {
      format!("timeout(\"{name}\")")
    }
    Datum::ColorRef(color_ref) => {
      match color_ref {
        ColorRef::PaletteIndex(i) => {
          format!("color({})", i)
        }
        ColorRef::Rgb(r, g, b) => {
          format!("rgb({}, {}, {})", r, g, b)
        }
      }
    }
    Datum::BitmapRef(bitmap) => {
      let bitmap = player.bitmap_manager.get_bitmap(*bitmap).unwrap();
      format!("<bitmap {}x{}x{}>", bitmap.width, bitmap.height, bitmap.bit_depth)
    }
    Datum::PaletteRef(_) => {
      format!("<palette>")
    }
    Datum::Xtra(name) => {
      format!("<Xtra \"{}\" _ _______>", name)
    }
    Datum::XtraInstance(name, instance_id) => {
      // "<Xtra child \"Multiuser\" _ _______>";
      format!("<Xtra child \"{}\" #{}>", name, instance_id)
    }
    Datum::Matte(..) => {
      format!("<mask:0000000>")
    }
    Datum::Null => {
      format!("<Null>")
    }
    Datum::PlayerRef => {
      format!("<_player>")
    }
    Datum::MovieRef => {
      format!("<_movie>")
    }
  }
}

pub fn format_datum(datum_ref: DatumRef, player: &DirPlayer) -> String {
  let datum = get_datum(datum_ref, &player.datums);
  format_concrete_datum(datum, player)
}
