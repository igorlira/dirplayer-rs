use log::warn;

use crate::director::lingo::datum::Datum;
use crate::director::lingo::datum;
use crate::player::bitmap::manager::INVALID_BITMAP_REF;
use super::{
    allocator::{DatumAllocator, DatumAllocatorTrait},
    handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers,
    DatumRef, ScriptError,
};

#[inline]
fn seq_equals(left_seq: &[DatumRef], right_seq: &[DatumRef], allocator: &DatumAllocator) -> Result<bool, ScriptError> {
    if left_seq.len() != right_seq.len() {
        return Ok(false);
    }
    for (left_item, right_item) in left_seq.iter().zip(right_seq.iter()) {
        let left_item = allocator.get_datum(left_item);
        let right_item = allocator.get_datum(right_item);
        if !datum_equals(left_item, right_item, allocator)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn datum_equals(
    left: &Datum,
    right: &Datum,
    allocator: &DatumAllocator,
) -> Result<bool, ScriptError> {
    use Datum::*;
    match (left, right) {
        (Int(i), other) | (other, Int(i)) => Ok(match other {
            Int(other_i) => *i == *other_i,
            Float(f) => (*i as f64) == *f, // TODO: is this correct? Flutter compares ints instead
            String(s) => s.parse::<i32>().ok() == Some(*i), // Handle string-to-int comparison (e.g., "2" should match key 2)
            sc @ StringChunk(..) => sc.string_value()?.parse::<i32>().ok() == Some(*i),
            Void => *i == 0,
            _ => false,
        }),

        (Float(f), other) | (other, Float(f)) => Ok(match other {
            Float(other_f) => *f == *other_f,
            Void => *f == 0.0,
            _ => false,
        }),

        // String equality: case-insensitive (like Director `=` operator)
        (s @ (String(_) | StringChunk(..)), other) | (other, s @ (String(_) | StringChunk(..))) => Ok({
            let sstr = s.string_value_cow().expect("cannot fail");
            match other {
                String(_) | StringChunk(..) => sstr.eq_ignore_ascii_case(&other.string_value_cow().expect("cannot fail")), // Case-insensitive comparison for String and StringChunk
                _ => false,
            }
        }),

        (Void | Null, x) | (x, Void | Null) => Ok(match x {
            Void | Null => true,
            VarRef(datum::VarRef::Script(var_ref)) => !var_ref.is_valid(),
            CastMember(member_ref) => !member_ref.is_valid(), // TODO return true if member is empty?
            BitmapRef(b) => *b == INVALID_BITMAP_REF,
            _ => false,
        }),

        (VarRef(a), o) | (o, VarRef(a)) => Ok(match o {
            VarRef(b) => match (a, b) {
                (datum::VarRef::Script(va), datum::VarRef::Script(vb)) => {
                    !va.is_valid() && !vb.is_valid() || // Both invalid = equal
                        CastMemberRefHandlers::get_cast_slot_number(
                            va.cast_lib as u32,
                            va.cast_member as u32,
                        ) == CastMemberRefHandlers::get_cast_slot_number(
                            vb.cast_lib as u32,
                            vb.cast_member as u32,
                        )
                },
                (datum::VarRef::ScriptInstance(va), datum::VarRef::ScriptInstance(vb)) => {
                    **va == **vb
                },
                _ => false
            },
            _ => false
        }),

        (List(_, l, _), other) | (other, List(_, l, _)) => Ok({
            let l_slice: Vec<_> = l.iter().cloned().collect();
            match other {
                List(_, r, _) => { let r_slice: Vec<_> = r.iter().cloned().collect(); seq_equals(&l_slice, &r_slice, allocator)? },
                Point(vals, flags) => {
                    // Director treats 2-element lists and points interchangeably
                    if l_slice.len() != 2 { false }
                    else {
                        let lx = allocator.get_datum(&l_slice[0]);
                        let ly = allocator.get_datum(&l_slice[1]);
                        let px = Datum::inline_component_to_datum(vals[0], Datum::inline_is_float(*flags, 0));
                        let py = Datum::inline_component_to_datum(vals[1], Datum::inline_is_float(*flags, 1));
                        datum_equals(lx, &px, allocator)? && datum_equals(ly, &py, allocator)?
                    }
                }
                Rect(vals, flags) => {
                    // Director treats 4-element lists and rects interchangeably
                    if l_slice.len() != 4 { false }
                    else {
                        let mut eq = true;
                        for i in 0..4 {
                            let li = allocator.get_datum(&l_slice[i]);
                            let ri = Datum::inline_component_to_datum(vals[i], Datum::inline_is_float(*flags, i));
                            if !datum_equals(li, &ri, allocator)? { eq = false; break; }
                        }
                        eq
                    }
                }
                _ => false
            }
        }),

        (PropList(pairs_a, _), PropList(pairs_b, _)) => {
            // Fast path: same datum in memory (same DatumRef)
            if std::ptr::eq(left, right) {
                return Ok(true);
            }
            // Structural comparison: same keys and values in same order
            if pairs_a.len() != pairs_b.len() {
                return Ok(false);
            }
            for (pair_a, pair_b) in pairs_a.iter().zip(pairs_b.iter()) {
                let key_a = allocator.get_datum(&pair_a.0);
                let key_b = allocator.get_datum(&pair_b.0);
                if !datum_equals(key_a, key_b, allocator)? {
                    return Ok(false);
                }
                let val_a = allocator.get_datum(&pair_a.1);
                let val_b = allocator.get_datum(&pair_b.1);
                if !datum_equals(val_a, val_b, allocator)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        (Symbol(s), o) | (o, Symbol(s)) => Ok(match o {
            Symbol(other) => s.eq_ignore_ascii_case(other),
            _ => false
        }),

        (CastLib(a), o) | (o, CastLib(a)) => Ok(match o {
            CastLib(b) => a == b,
            _ => false
        }),

        (Stage, o) | (o, Stage) => Ok(matches!(o, Stage)),

        (ScriptRef(a), o) | (o, ScriptRef(a)) => Ok(match o {
            ScriptRef(b) => a == b,
            _ => false
        }),

        (ScriptInstanceRef(a), o) | (o, ScriptInstanceRef(a)) => Ok(match o {
            ScriptInstanceRef(b) => **a == **b,
            _ => false
        }),

        (CastMember(a), o) | (o, CastMember(a)) => Ok(match o {
            CastMember(b) => CastMemberRefHandlers::get_cast_slot_number(
                a.cast_lib as u32,
                a.cast_member as u32,
            ) == CastMemberRefHandlers::get_cast_slot_number(
                b.cast_lib as u32,
                b.cast_member as u32,
            ),
            _ => false
        }),

        (SpriteRef(a), o) | (o, SpriteRef(a)) => Ok(match o {
            SpriteRef(b) => a == b,
            _ => false
        }),

        (Rect(a_vals, a_flags), o) | (o, Rect(a_vals, a_flags)) => Ok(match o {
            Rect(b_vals, b_flags) => {
                let mut eq = true;
                for i in 0..4 {
                    let ai = Datum::inline_component_to_datum(a_vals[i], Datum::inline_is_float(*a_flags, i));
                    let bi = Datum::inline_component_to_datum(b_vals[i], Datum::inline_is_float(*b_flags, i));
                    if !datum_equals(&ai, &bi, allocator)? { eq = false; break; }
                }
                eq
            }
            _ => false
        }),

        (Point(a_vals, a_flags), o) | (o, Point(a_vals, a_flags)) => Ok(match o {
            Point(b_vals, b_flags) => {
                let ax = Datum::inline_component_to_datum(a_vals[0], Datum::inline_is_float(*a_flags, 0));
                let ay = Datum::inline_component_to_datum(a_vals[1], Datum::inline_is_float(*a_flags, 1));
                let bx = Datum::inline_component_to_datum(b_vals[0], Datum::inline_is_float(*b_flags, 0));
                let by = Datum::inline_component_to_datum(b_vals[1], Datum::inline_is_float(*b_flags, 1));
                datum_equals(&ax, &bx, allocator)? && datum_equals(&ay, &by, allocator)?
            }
            List(_, list, _) if list.len() == 2 => {
                // Director treats 2-element lists and points interchangeably
                let ax = Datum::inline_component_to_datum(a_vals[0], Datum::inline_is_float(*a_flags, 0));
                let ay = Datum::inline_component_to_datum(a_vals[1], Datum::inline_is_float(*a_flags, 1));
                let list_slice: Vec<_> = list.iter().cloned().collect();
                let lx = allocator.get_datum(&list_slice[0]);
                let ly = allocator.get_datum(&list_slice[1]);
                datum_equals(&ax, lx, allocator)? && datum_equals(&ay, ly, allocator)?
            }
            _ => false
        }),

        (SoundChannel(a), o) | (o, SoundChannel(a)) => Ok(match o {
            SoundChannel(b) => a == b,
            _ => false
        }),

        (CursorRef(a), o) | (o, CursorRef(a)) => Ok(match o {
            // TODO: is equality based on value?
            _ => false
        }),

        (TimeoutRef(a), o) | (o, TimeoutRef(a)) => Ok(match o {
            TimeoutRef(b) => a == b,
            _ => false
        }),

        (TimeoutFactory, o) | (o, TimeoutFactory) => Ok(matches!(o, TimeoutFactory)),

        (TimeoutInstance { .. }, o) | (o, TimeoutInstance { .. }) => Ok(match o {
            // TODO: is equality based on value?
            _ => false
        }),

        (ColorRef(a), o) | (o, ColorRef(a)) => Ok(match o {
            ColorRef(b) => a == b,
            _ => false
        }),

        (BitmapRef(a), o) | (o, BitmapRef(a)) => Ok(match o {
            BitmapRef(b) => a == b,
            _ => false
        }),

        (PaletteRef(a), o) | (o, PaletteRef(a)) => Ok(match o {
            PaletteRef(b) => a == b,
            _ => false
        }),

        (SoundRef(a), o) | (o, SoundRef(a)) => Ok(match o {
            SoundRef(b) => a == b,
            _ => false
        }),

        (Xtra(a), o) | (o, Xtra(a)) => Ok(match o {
            Xtra(b) => a == b,
            _ => false
        }),

        (XtraInstance(a, ai), o) | (o, XtraInstance(a, ai)) => Ok(match o {
            XtraInstance(b, bi) => a == b && ai == bi,
            _ => false
        }),

        (Matte(a), o) | (o, Matte(a)) => Ok(match o {
            Matte(b) => a == b,
            _ => false
        }),

        (PlayerRef, o) | (o, PlayerRef) => Ok(matches!(o, PlayerRef)),

        (MovieRef, o) | (o, MovieRef) => Ok(matches!(o, MovieRef)),

        (MouseRef, o) | (o, MouseRef) => Ok(matches!(o, MouseRef)),

        (XmlRef(a), o) | (o, XmlRef(a)) => Ok(match o {
            XmlRef(b) => a == b,
            _ => false
        }),

        (DateRef(a), o) | (o, DateRef(a)) => Ok(match o {
            DateRef(b) => a == b,
            _ => false
        }),

        (FlashObjectRef(a), o) | (o, FlashObjectRef(a)) => Ok(match o {
            // Two AS object refs are equal iff they point to the same path.
            // Without this case, the catch-all at the bottom returns false even
            // for identical refs, which makes any prop list containing AS object
            // values (e.g. Coke Studios' friend list with #lastAccess Date refs)
            // fail deep equality against its own .duplicate() — causing every
            // friendslist exitFrame to redraw the whole list during scrolling.
            FlashObjectRef(b) => a.path == b.path
                && a.cast_lib == b.cast_lib
                && a.cast_member == b.cast_member,
            _ => false
        }),

        (MathRef(a), o) | (o, MathRef(a)) => Ok(match o {
            MathRef(b) => a == b,
            _ => false
        }),

        (Vector(v), o) | (o, Vector(v)) => Ok(match o {
            Vector(other_v) => v == other_v,
            _ => false
        }),

        // Two PhysX rigid-body / joint / terrain refs are the same Director
        // object when they point to the same (cast_lib, cast_member, id).
        // Without this, `collisionreport.objectA = Vehicle.Physics` in the
        // registered #collisionCallback always returns false (the catch-all
        // below returns false), so OnGround never flips and bodies don't
        // come to rest on the ground.
        (PhysXObjectRef(a), o) | (o, PhysXObjectRef(a)) => Ok(match o {
            PhysXObjectRef(b) => a.cast_lib == b.cast_lib
                && a.cast_member == b.cast_member
                && a.id == b.id,
            _ => false
        }),

        // Same treatment for Havok object refs — LEGO Supersonic's
        // collision callbacks rely on this equality.
        (HavokObjectRef(a), o) | (o, HavokObjectRef(a)) => Ok(match o {
            HavokObjectRef(b) => a.cast_lib == b.cast_lib
                && a.cast_member == b.cast_member
                && a.name.eq_ignore_ascii_case(&b.name),
            _ => false
        }),

        // Two W3D scene-object refs (model/group/light/camera/collision/...)
        // are the same Director object when they point to the same member,
        // object type and (case-insensitive) name. Without this the catch-all
        // returns false and `collisionData.modelA = s.model("ft")` — the heart
        // of every native #collision callback — never matches.
        (Shockwave3dObjectRef(a), o) | (o, Shockwave3dObjectRef(a)) => Ok(match o {
            Shockwave3dObjectRef(b) => a.cast_lib == b.cast_lib
                && a.cast_member == b.cast_member
                && a.object_type.eq_ignore_ascii_case(&b.object_type)
                && a.name.eq_ignore_ascii_case(&b.name),
            _ => false
        }),

        (Media(a), o) | (o, Media(a)) => Ok(match o {
            // TODO: is equality based on value?
            _ => false
        }),

        (JavaScript(a), o) | (o, JavaScript(a)) => Ok(match o {
            JavaScript(b) => a == b,
            _ => false
        }),

        _ => {
            warn!(
                "datum_equals not supported for types: {} and {}",
                left.type_str(),
                right.type_str()
            );
            Ok(false)
        }
    }
}

/// List-membership equality (`getPos`, `getOne`, `findPos`).
///
/// Director's `=` operator is strict — `#foo = "foo"` returns FALSE — but the
/// list-membership lookup family matches by text content across the
/// Symbol/String boundary. Verified with `put getPos([#foo, #bar], "foo")`
/// returning 1 in Director 11.5. Scripts in the wild (e.g. Trick or Treat
/// Beat's `getPos(gSingleTileObjNames, member.name)`) rely on this looser
/// rule even though general `=` would not match.
pub fn datum_equals_member(
    left: &Datum,
    right: &Datum,
    allocator: &DatumAllocator,
) -> Result<bool, ScriptError> {
    use Datum::*;
    let symbol_string_match = match (left, right) {
        (Symbol(sym), other @ (String(_) | StringChunk(..)))
        | (other @ (String(_) | StringChunk(..)), Symbol(sym)) => {
            Some(sym.eq_ignore_ascii_case(&other.string_value_cow()?))
        }
        _ => None,
    };
    if let Some(matched) = symbol_string_match {
        return Ok(matched);
    }
    datum_equals(left, right, allocator)
}

#[allow(dead_code)]
pub fn datum_greater_than(left: &Datum, right: &Datum, allocator: &DatumAllocator) -> Result<bool, ScriptError> {
    // See `datum_less_than`: a string chunk compares by its resolved text, and
    // a sprite reference compares by its sprite (channel) number.
    if let Datum::StringChunk(_, _, s) = left {
        return datum_greater_than(&Datum::String(s.clone()), right, allocator);
    }
    if let Datum::StringChunk(_, _, s) = right {
        return datum_greater_than(left, &Datum::String(s.clone()), allocator);
    }
    if let Datum::SpriteRef(n) = left {
        return datum_greater_than(&Datum::Int(*n as i32), right, allocator);
    }
    if let Datum::SpriteRef(n) = right {
        return datum_greater_than(left, &Datum::Int(*n as i32), allocator);
    }
    // A symbol compares as its string name (see `datum_less_than`).
    if let Datum::Symbol(s) = left {
        return datum_greater_than(&Datum::String(s.clone()), right, allocator);
    }
    if let Datum::Symbol(s) = right {
        return datum_greater_than(left, &Datum::String(s.clone()), allocator);
    }
    match (left, right) {
        // Int comparisons
        (Datum::Int(left), Datum::Int(right)) => Ok(*left > *right),
        (Datum::Int(left), Datum::Float(right)) => Ok((*left as f64) > *right),
        (Datum::Int(left), Datum::Void) => Ok(*left > 0),
        (Datum::Int(left), Datum::String(right)) => {
            if let Ok(right_number) = right.parse::<i32>() {
                Ok(*left > right_number)
            } else {
                Ok(right.is_empty())
            }
        }
        
        // Float comparisons
        (Datum::Float(left), Datum::Int(right)) => Ok(*left > (*right as f64)),
        (Datum::Float(left), Datum::Float(right)) => Ok(*left > *right),
        (Datum::Float(left), Datum::Void) => Ok(*left > 0.0),
        
        // Void comparisons - Void is never > any number
        (Datum::Void, Datum::Int(_)) => Ok(false),
        (Datum::Void, Datum::Float(_)) => Ok(false),
        
        // String vs number: Director coerces strings to numbers (empty string = 0)
        (Datum::String(left), Datum::Int(right)) => {
            let left_number = left.parse::<i32>().unwrap_or(0);
            Ok(left_number > *right)
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_number = left.parse::<f64>().unwrap_or(0.0);
            Ok(left_number > *right)
        }

        // Point comparisons
        (Datum::Point(left_vals, _), Datum::Point(right_vals, _)) => {
            let left_x = left_vals[0] as i32;
            let left_y = left_vals[1] as i32;
            let right_x = right_vals[0] as i32;
            let right_y = right_vals[1] as i32;
            Ok(left_x > right_x && left_y > right_y)
        }

        // Point vs scalar: Director compares the point against the scalar
        // component-wise; the result is true when ANY component satisfies the
        // comparison. Summer Resort's room-scroll clamp relies on this — it
        // tests an axis-aligned delta `point(0,16) > 0` / `point(0,-16) < 0`
        // (one component is 0, the other ±16) to decide the scroll direction
        // and clamp the player to the room edge. Without this the clamp never
        // fired, the player over-scrolled past the screen bottom, and the next
        // move bounced straight back into the previous room.
        (Datum::Point(vals, _), Datum::Int(n)) => {
            Ok((vals[0] as i32) > *n || (vals[1] as i32) > *n)
        }
        (Datum::Int(n), Datum::Point(vals, _)) => {
            Ok(*n > (vals[0] as i32) || *n > (vals[1] as i32))
        }

        // Linear list comparison — element-wise, mirroring `datum_less_than`
        // (and the 11.5 dictionary's rect/point-as-list rule). True only if
        // every corresponding element of the left is > the right's.
        (Datum::List(_, left_items, _), Datum::List(_, right_items, _)) => {
            if left_items.is_empty() || right_items.is_empty() {
                return Ok(false);
            }
            for (l, r) in left_items.iter().zip(right_items.iter()) {
                let ld = allocator.get_datum(l);
                let rd = allocator.get_datum(r);
                if !datum_greater_than(ld, rd, allocator)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        // Script instances compare by allocation id (see `datum_less_than`).
        (Datum::ScriptInstanceRef(l), Datum::ScriptInstanceRef(r)) => Ok(l.id() > r.id()),

        // Two strings compare lexicographically, case-insensitively — the mirror of
        // `datum_less_than`. (Symbols are pre-converted to strings above.) Director
        // uses this for text/version checks like `GrooveVersion() >= "1.7"`.
        (Datum::String(left), Datum::String(right)) =>
            Ok(left.to_ascii_lowercase() > right.to_ascii_lowercase()),

        // Catch-all
        _ => {
            warn!(
                "datum_greater_than not supported for types: {} and {}",
                left.type_str(),
                right.type_str()
            );
            Ok(false)
        }
    }
}

pub fn datum_less_than(left: &Datum, right: &Datum, allocator: &DatumAllocator) -> Result<bool, ScriptError> {
    // A string chunk (`char/word/item/line N of x`) evaluates to a plain
    // string — compare by its resolved text. Without this, `char 1 of x < "m"`
    // (and any chunk poll in a per-frame loop) falls through to the catch-all,
    // returning a wrong result AND warning every frame.
    if let Datum::StringChunk(_, _, s) = left {
        return datum_less_than(&Datum::String(s.clone()), right, allocator);
    }
    if let Datum::StringChunk(_, _, s) = right {
        return datum_less_than(left, &Datum::String(s.clone()), allocator);
    }
    // A sprite reference compares by its sprite (channel) number, so a movie
    // that sorts/compares `sprite(a) < sprite(b)` works instead of hitting the
    // catch-all (wrong result + per-frame warn).
    if let Datum::SpriteRef(n) = left {
        return datum_less_than(&Datum::Int(*n as i32), right, allocator);
    }
    if let Datum::SpriteRef(n) = right {
        return datum_less_than(left, &Datum::Int(*n as i32), allocator);
    }
    // A symbol compares as its string name (Director treats `#foo` like "foo" in
    // ordered comparisons), so route symbols through the string logic below. This
    // covers int-vs-symbol, symbol-vs-symbol, etc. without dedicated arms.
    if let Datum::Symbol(s) = left {
        return datum_less_than(&Datum::String(s.clone()), right, allocator);
    }
    if let Datum::Symbol(s) = right {
        return datum_less_than(left, &Datum::String(s.clone()), allocator);
    }
    match (left, right) {
        // Int comparisons
        (Datum::Int(left), Datum::Int(right)) => Ok(*left < *right),
        (Datum::Int(left), Datum::Float(right)) => Ok((*left as f64) < *right),
        (Datum::Int(left), Datum::Void) => Ok(*left < 0),
        (Datum::Int(left), Datum::String(right)) => {
            if let Ok(right_number) = right.parse::<i32>() {
                Ok(*left < right_number)
            } else {
                Ok(!right.is_empty())
            }
        }
        
        // Float comparisons
        (Datum::Float(left), Datum::Int(right)) => Ok(*left < (*right as f64)),
        (Datum::Float(left), Datum::Float(right)) => Ok(*left < *right),
        (Datum::Float(left), Datum::Void) => Ok(*left < 0.0),
        
        // Void comparisons - Void is always < any number
        (Datum::Void, Datum::Int(_)) => Ok(true),
        (Datum::Void, Datum::Float(_)) => Ok(true),
        
        // Point comparisons
        (Datum::Point(left_vals, _), Datum::Point(right_vals, _)) => {
            let left_x = left_vals[0] as i32;
            let left_y = left_vals[1] as i32;
            let right_x = right_vals[0] as i32;
            let right_y = right_vals[1] as i32;
            Ok(left_x < right_x && left_y < right_y)
        }

        // Point vs scalar — see the note in `datum_greater_than`. Any component
        // satisfying the comparison makes it true (axis-aligned scroll deltas).
        (Datum::Point(vals, _), Datum::Int(n)) => {
            Ok((vals[0] as i32) < *n || (vals[1] as i32) < *n)
        }
        (Datum::Int(n), Datum::Point(vals, _)) => {
            Ok(*n < (vals[0] as i32) || *n < (vals[1] as i32))
        }

        // String vs number: Director coerces strings to numbers (empty string = 0)
        (Datum::String(left), Datum::Int(right)) => {
            let left_number = left.parse::<i32>().unwrap_or(0);
            Ok(left_number < *right)
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_number = left.parse::<f64>().unwrap_or(0.0);
            Ok(left_number < *right)
        }

        // String / Symbol comparisons — Director compares case-insensitively
        // lexicographically. Without this, any `.add()` to a sorted list of
        // strings (e.g. CS FurnitureItem draw-order tags "a","b","c","d") would
        // always return 0 from find_index_to_add and silently prepend every
        // item, breaking sprite-pool allocation order.
        (Datum::String(left), Datum::String(right)) =>
            Ok(left.to_ascii_lowercase() < right.to_ascii_lowercase()),
        (Datum::Symbol(left), Datum::Symbol(right)) =>
            Ok(left.to_ascii_lowercase() < right.to_ascii_lowercase()),
        (Datum::String(left), Datum::Symbol(right)) =>
            Ok(left.to_ascii_lowercase() < right.to_ascii_lowercase()),
        (Datum::Symbol(left), Datum::String(right)) =>
            Ok(left.to_ascii_lowercase() < right.to_ascii_lowercase()),

        // String comparisons
        (Datum::String(..), Datum::String(..)) => Ok(false),

        // PropList comparisons - Director compares property lists by their first value.
        // This is essential for sorted lists used as priority queues (e.g. A* pathfinding).
        (Datum::PropList(left_pairs, ..), Datum::PropList(right_pairs, ..)) => {
            if let (Some((_, left_val)), Some((_, right_val))) = (left_pairs.front(), right_pairs.front()) {
                let left_datum = allocator.get_datum(left_val);
                let right_datum = allocator.get_datum(right_val);
                datum_less_than(left_datum, right_datum, allocator)
            } else {
                Ok(false)
            }
        }

        // Linear list comparison. Per the 11.5 dictionary `<` entry, rects/points
        // (and by extension lists) compare "with each element of the first list
        // compared to the corresponding element of the second list" — the same
        // all-components rule the Point arm above uses. True only if every
        // corresponding element of the left is < the right's.
        (Datum::List(_, left_items, _), Datum::List(_, right_items, _)) => {
            if left_items.is_empty() || right_items.is_empty() {
                return Ok(false);
            }
            for (l, r) in left_items.iter().zip(right_items.iter()) {
                let ld = allocator.get_datum(l);
                let rd = allocator.get_datum(r);
                if !datum_less_than(ld, rd, allocator)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        // Script instances have no meaningful ordering in Director, but a movie
        // that sorts or compares them needs a stable result — compare by their
        // allocation id.
        (Datum::ScriptInstanceRef(l), Datum::ScriptInstanceRef(r)) => Ok(l.id() < r.id()),

        // Catch-all
        _ => {
            warn!(
                "datum_less_than not supported for types: {} and {}",
                left.type_str(),
                right.type_str()
            );
            Ok(false)
        }
    }
}

pub fn datum_is_zero(datum: &Datum, datums: &DatumAllocator) -> Result<bool, ScriptError> {
    Ok(match datum {
        Datum::Int(value) => *value == 0,
        Datum::Float(value) => *value == 0.0,
        Datum::Void => true,
        Datum::ScriptInstanceRef(_) => false,
        Datum::Null => true,
        Datum::Point(vals, _) => {
            vals[0] as i32 == 0 && vals[1] as i32 == 0
        }
        Datum::Rect(vals, _) => {
            vals[0] as i32 == 0 && vals[1] as i32 == 0 && vals[2] as i32 == 0 && vals[3] as i32 == 0
        }
        _ => {
            warn!("datum_is_zero not supported for type: {}", datum.type_str());
            datum.int_value()? == 0
        }
    })
}

pub fn sort_datums(
    datums: &Vec<DatumRef>,
    allocator: &DatumAllocator,
) -> Result<Vec<DatumRef>, ScriptError> {
    let mut sorted_list = datums.clone();
    sorted_list.sort_by(|a, b| {
        let left = allocator.get_datum(a);
        let right = allocator.get_datum(b);

        if datum_equals(left, right, allocator).unwrap() {
            return std::cmp::Ordering::Equal;
        } else if datum_less_than(left, right, allocator).unwrap() {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    });
    Ok(sorted_list)
}

fn datum_to_f64(datum: &Datum) -> Result<f64, ScriptError> {
    match datum {
        Datum::Int(i) => Ok(*i as f64),
        Datum::Float(f) => Ok(*f),
        _ => datum.float_value()
    }
}
