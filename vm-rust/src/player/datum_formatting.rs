use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{allocator::ScriptInstanceAllocatorTrait, sprite::ColorRef, bitmap::bitmap::PaletteRef},
};

use super::{DatumRef, DirPlayer};

pub fn format_concrete_datum(datum: &Datum, player: &DirPlayer) -> String {
    match datum {
        Datum::String(s) => format!("\"{s}\""),
        Datum::Int(i) => i.to_string(),
        Datum::Float(f) => format_float_with_precision(*f, player),
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
        Datum::Rect(refs) => {
            let x1 = player.get_datum(&refs[0]);
            let y1 = player.get_datum(&refs[1]);
            let x2 = player.get_datum(&refs[2]);
            let y2 = player.get_datum(&refs[3]);
            format!(
                "rect({}, {}, {}, {})",
                format_numeric_value(x1, player),
                format_numeric_value(y1, player),
                format_numeric_value(x2, player),
                format_numeric_value(y2, player)
            )
        }
        Datum::Point(refs) => {
            let x = player.get_datum(&refs[0]);
            let y = player.get_datum(&refs[1]);
            format!(
                "point({}, {})",
                format_numeric_value(x, player),
                format_numeric_value(y, player)
            )
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
        Datum::TimeoutFactory => {
            format!("<timeoutFactory>")
        }
        Datum::TimeoutInstance { name, .. } => {
            format!("timeoutInstance(\"{0}\")", name)
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
        Datum::PaletteRef(palette_ref) => match palette_ref {
            PaletteRef::BuiltIn(builtin) => {
                format!("{:?}", builtin).to_lowercase()
            }
            PaletteRef::Member(member_ref) => {
                format!(
                    "(member {} of castLib {})",
                    member_ref.cast_member,
                    member_ref.cast_lib
                )
            }
        },
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
        Datum::Vector(v) => {
            format!(
                "vector({}, {}, {})", 
                format_float_with_precision(v[0], player),
                format_float_with_precision(v[1], player),
                format_float_with_precision(v[2], player),
            )
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
        Datum::String(s) => s.clone(),
               
        Datum::Symbol(s) => s.clone(),
        
        // Void/Null become empty string in concatenation
        Datum::Void | Datum::Null => String::new(),
        
        Datum::ColorRef(cr) => match cr {
            ColorRef::PaletteIndex(i) => format!("color({})", i),
            ColorRef::Rgb(r, g, b) => format!("rgb({}, {}, {})", r, g, b),
        },
        
        Datum::List(_, list, _) => {
            let elements: Vec<String> = list
                .iter()
                .map(|r| datum_to_string_for_concat(player.get_datum(r), player))
                .collect();
            format!("[{}]", elements.join(", "))
        },
        
        Datum::PropList(entries, _) => {
            if entries.is_empty() {
                return "[:]".to_string();
            }
            let elements: Vec<String> = entries
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        datum_to_string_for_concat(player.get_datum(k), player),
                        datum_to_string_for_concat(player.get_datum(v), player)
                    )
                })
                .collect();
            format!("[{}]", elements.join(", "))
        },
        
        Datum::StringChunk(..) => {
            datum.string_value().unwrap_or(String::new())
        },
        
        // For other complex types, use the standard formatter
        _ => format_concrete_datum(datum, player),
    }
}

pub fn format_datum(datum_ref: &DatumRef, player: &DirPlayer) -> String {
    let datum = player.get_datum(datum_ref);
    format_concrete_datum(datum, player)
}

// Helper function to format a numeric value according to floatPrecision
pub fn format_float_with_precision(val: f64, player: &DirPlayer) -> String {
    // Normalize negative zero to positive zero
    let val = if val == 0.0 { 0.0 } else { val };

    let fp = player.float_precision as i32;
    
    // Calculate how many characters the decimal notation would take
    let integer_digits = if val.abs() < 1.0 {
        1 // Just "0"
    } else {
        (val.abs().log10().floor() as i32 + 1).max(1)
    };
    
    let decimal_places = if fp > 0 { fp } else { 0 };
    let total_chars = integer_digits + 1 + decimal_places; // digits + '.' + decimals
    
    // Director switches to scientific notation when formatted string >= 18 chars
    if total_chars >= 18 {
        return format!("{:.14e}", val);
    }
    
    // Normal formatting based on floatPrecision
    if fp > 0 {
        let p = fp.min(15) as usize;
        format!("{:.*}", p, val)
    } else if fp == 0 {
        format!("{}", val.round() as i32)
    } else {
        let p = (-fp).min(15);
        let pow = 10f64.powi(p);
        let rounded = (val * pow).round() / pow;
        let s = format!("{:.*}", p as usize, rounded);
        s.trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

pub fn format_numeric_value(datum: &Datum, player: &DirPlayer) -> String {
    match datum {
        Datum::Int(i) => i.to_string(),
        Datum::Float(f) => format_float_with_precision(*f, player),
        _ => format_concrete_datum(datum, player),
    }
}
