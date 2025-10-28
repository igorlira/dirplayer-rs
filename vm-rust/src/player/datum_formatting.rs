use crate::{
    director::lingo::datum::Datum,
    player::{allocator::ScriptInstanceAllocatorTrait, sprite::ColorRef},
};

use super::{DatumRef, DirPlayer};

pub fn format_concrete_datum(datum: &Datum, player: &DirPlayer) -> String {
    match datum {
        Datum::String(s) => format!("\"{s}\""),
        Datum::Int(i) => i.to_string(),
        Datum::Float(f) => match player.float_precision {
            1 => format!("{:.1}", f),
            2 => format!("{:.2}", f),
            3 => format!("{:.3}", f),
            4 => format!("{:.4}", f),
            5 => format!("{:.5}", f),
            6 => format!("{:.6}", f),
            _ => f.to_string(),
        },
        Datum::List(_, items, _) => {
            let formatted_items: Vec<String> =
                items.iter().map(|x| format_datum(x, player)).collect();
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
            let formatted_entries: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("{}: {}", format_datum(k, player), format_datum(v, player)))
                .collect();
            format!("[{}]", formatted_entries.join(", "))
        }
        Datum::StringChunk(..) => format!(
            "\"{}\"",
            datum.string_value().unwrap_or("!!!ERR!!!".to_string())
        ),
        Datum::ScriptRef(member_ref) => {
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(&member_ref)
                .unwrap();
            format!("(script {})", script.name)
        }
        Datum::ScriptInstanceRef(instance_ref) => {
            let instance = player.allocator.get_script_instance(instance_ref);
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(&instance.script)
                .unwrap();

            format!("<offspring {} {} _>", script.name, instance_ref)
        }
        Datum::CastMember(member_ref) => {
            format!(
                "(member {} of castLib {})",
                member_ref.cast_member, member_ref.cast_lib
            )
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
        Datum::SoundChannel(_) => {
            format!("<soundChannel>")
        }
        Datum::CursorRef(_) => {
            format!("<cursor>")
        }
        Datum::TimeoutRef(name) => {
            format!("timeout(\"{name}\")")
        }
        Datum::ColorRef(color_ref) => match color_ref {
            ColorRef::PaletteIndex(i) => {
                format!("color({})", i)
            }
            ColorRef::Rgb(r, g, b) => {
                format!("rgb({}, {}, {})", r, g, b)
            }
        },
        Datum::BitmapRef(bitmap) => {
            let bitmap = player.bitmap_manager.get_bitmap(*bitmap).unwrap();
            format!(
                "<bitmap {}x{}x{}>",
                bitmap.width, bitmap.height, bitmap.bit_depth
            )
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
        Datum::XmlRef(id) => {
            format!("<xml:{}>", id)
        }
        Datum::MathRef(_) => {
            format!("<math>")
        }
        Datum::Vector(_) => {
            format!("<vector>")
        }
        Datum::SoundRef(_) => {
            format!("<_sound>")
        }
        Datum::DateRef(_) => {
            format!("<date>")
        }
    }
}

pub fn datum_to_string_for_concat(datum: &Datum, player: &DirPlayer) -> String {
    match datum {
        Datum::Int(n) => n.to_string(),
        Datum::Float(f) => format!("{:.3}", f), // or your precision logic
        Datum::String(s) => s.clone(),
        Datum::Void => "VOID".to_string(),
        Datum::Vector(v) => format!("[{},{},{}]", v[0], v[1], v[2]),
        Datum::IntRect(r) => format!("({}, {}, {}, {})", r.0, r.1, r.2, r.3),
        Datum::IntPoint(p) => format!("({}, {})", p.0, p.1),
        Datum::ColorRef(cr) => match cr {
            ColorRef::PaletteIndex(i) => format!("color({})", i),
            ColorRef::Rgb(r, g, b) => format!("rgb({}, {}, {})", r, g, b),
        },
        Datum::List(_, list, _) => {
            let elements: Vec<String> = list
                .iter()
                .map(|r| datum_to_string_for_concat(player.get_datum(r), player))
                .collect();
            elements.join(", ")
        }
        _ => "<unknown datum>".to_string(),
    }
}

pub fn format_datum(datum_ref: &DatumRef, player: &DirPlayer) -> String {
    let datum = player.get_datum(datum_ref);
    format_concrete_datum(datum, player)
}
