use super::{
    cast_member::CastMemberType,
    events::{player_invoke_event_to_instances, player_dispatch_movie_callback, player_invoke_frame_and_movie_scripts},
    player_is_playing, reserve_player_mut, DatumRef, DirPlayer, ScriptError,
};

/// Per-line stride to use for caret math, matching the renderer's safety
/// guard: prefer `fixed_line_space` when it looks sane, otherwise fall back
/// to the font's natural line height. Values much larger than that come
/// from XMED-misparsed STXT runs that store the field's box height instead
/// of its real per-line height — honoring them sends the caret to the
/// wrong line on click and breaks selection past the first line.
pub(crate) fn effective_line_height(font: &crate::player::font::BitmapFont, fixed_line_space: u16) -> i32 {
    let natural_lh = if font.font_size > 0 {
        font.font_size as i32
    } else {
        font.char_height as i32
    };
    if fixed_line_space > 0 && (fixed_line_space as i32) <= natural_lh * 5 / 2 {
        fixed_line_space as i32
    } else {
        natural_lh
    }
}

fn get_next_focus_sprite_id(player: &DirPlayer, after: i16) -> i16 {
    for sprite_id in after + 1..=player.movie.score.get_channel_count() as i16 {
        let sprite = player.movie.score.get_sprite(sprite_id);
        let member_ref = sprite.and_then(|x| x.member.clone());
        let member = member_ref.and_then(|x| player.movie.cast_manager.find_member_by_ref(&x));
        let editable = match member.map(|m| &m.member_type) {
            Some(CastMemberType::Field(f)) => f.editable,
            Some(CastMemberType::Text(t)) => t.info.as_ref().map_or(false, |i| i.editable),
            _ => false,
        };
        if editable {
            return sprite_id;
        }
    }
    -1
}

/// Whether the focused sprite's member is an editable Field or Text.
fn focused_member_is_editable(player: &DirPlayer) -> bool {
    if player.keyboard_focus_sprite < 0 {
        return false;
    }
    let sprite = player.movie.score.get_sprite(player.keyboard_focus_sprite as i16);
    let member = sprite
        .and_then(|s| s.member.clone())
        .and_then(|r| player.movie.cast_manager.find_member_by_ref(&r));
    match member.map(|m| &m.member_type) {
        Some(CastMemberType::Field(f)) => f.editable,
        Some(CastMemberType::Text(t)) => t.info.as_ref().map_or(false, |i| i.editable),
        _ => false,
    }
}

/// Apply a single edit/navigation key to a text + selection state. Mutates in
/// place. Used for both Field and Text members since the editing rules are
/// identical at this level.
///
/// Convention: `sel_start <= sel_end`, `sel_anchor` is the fixed end during
/// shift+arrow / drag selections. ASCII byte indices.
pub(crate) fn apply_text_edit(
    text: &mut String,
    sel_start: &mut i32,
    sel_end: &mut i32,
    sel_anchor: &mut i32,
    key: &str,
    ctrl_or_meta: bool,
    shift: bool,
) {
    let len = text.len() as i32;
    let mut lo = (*sel_start).clamp(0, len);
    let mut hi = (*sel_end).clamp(0, len);
    if lo > hi {
        std::mem::swap(&mut lo, &mut hi);
    }
    let anchor = (*sel_anchor).clamp(0, len);

    fn collapse(start: &mut i32, end: &mut i32, anc: &mut i32, pos: i32, len: i32) {
        let p = pos.clamp(0, len);
        *start = p;
        *end = p;
        *anc = p;
    }
    fn extend(start: &mut i32, end: &mut i32, anchor: i32, pos: i32, len: i32) {
        let p = pos.clamp(0, len);
        *start = anchor.min(p);
        *end = anchor.max(p);
    }

    let bytes_for_nav = text.as_bytes().to_vec();
    // Word-character predicate at byte level for the line-break helpers
    // (which only consult ASCII \r / \n -- safe with bytes).
    let _ = bytes_for_nav.len(); // suppress unused warning if only line helpers consult bytes
    // Char-aware "is word char": alphanumeric or underscore. Unlike the
    // earlier byte-based check, this returns true for umlauts (ä, é, ñ,
    // å, etc.) so Ctrl+Arrow / double-click word-select treats "café" as
    // one word instead of stopping at the multi-byte boundary.
    let is_word_ch = |c: char| c.is_alphanumeric() || c == '_';

    // Pre-snapshot (byte_pos, char) for the whole text. Allocates once per
    // apply_text_edit call; text in real fields is short (chat input, name
    // fields, descriptions) so memory is fine.
    let nav_chars: Vec<(usize, char)> = text.char_indices().collect();
    // Find the char-index in nav_chars whose byte_pos == `byte_pos`, or
    // the index where it would be inserted (== nav_chars.len() for EOS).
    let char_idx_of = |byte_pos: i32| -> usize {
        let bp = byte_pos.clamp(0, len) as usize;
        nav_chars.iter().position(|(b, _)| *b >= bp).unwrap_or(nav_chars.len())
    };
    let prev_word_boundary = |from: i32| -> i32 {
        let mut i = char_idx_of(from);
        while i > 0 && !is_word_ch(nav_chars[i - 1].1) { i -= 1; }
        while i > 0 && is_word_ch(nav_chars[i - 1].1) { i -= 1; }
        nav_chars.get(i).map(|(b, _)| *b as i32).unwrap_or(len)
    };
    let next_word_boundary = |from: i32| -> i32 {
        let n = nav_chars.len();
        let mut i = char_idx_of(from);
        while i < n && is_word_ch(nav_chars[i].1) { i += 1; }
        while i < n && !is_word_ch(nav_chars[i].1) { i += 1; }
        nav_chars.get(i).map(|(b, _)| *b as i32).unwrap_or(len)
    };
    // Char-boundary helpers. The text buffer is UTF-8 (Rust String), so a
    // single visible character like 'ä' or '€' is 2-3 bytes. Stepping the
    // caret by 1 byte would land inside a multi-byte sequence and make
    // `text.replace_range` panic on the next edit. These walk to the
    // nearest legal char boundary using `str::is_char_boundary`, which is
    // O(1) per step.
    let prev_char_boundary = |from: i32| -> i32 {
        if from <= 0 { return 0; }
        let mut p = (from as usize).min(text.len());
        if p == 0 { return 0; }
        p -= 1;
        while p > 0 && !text.is_char_boundary(p) { p -= 1; }
        p as i32
    };
    let next_char_boundary = |from: i32| -> i32 {
        let n = text.len();
        let mut p = (from as usize).min(n);
        if p >= n { return n as i32; }
        p += 1;
        while p < n && !text.is_char_boundary(p) { p += 1; }
        p as i32
    };
    let line_start_of = |from: i32| -> i32 {
        let mut p = from.clamp(0, len) as usize;
        while p > 0 && bytes_for_nav[p - 1] != b'\r' && bytes_for_nav[p - 1] != b'\n' { p -= 1; }
        p as i32
    };
    let line_end_of = |from: i32| -> i32 {
        let n = bytes_for_nav.len();
        let mut p = from.clamp(0, len) as usize;
        while p < n && bytes_for_nav[p] != b'\r' && bytes_for_nav[p] != b'\n' { p += 1; }
        p as i32
    };

    // The cursor (active/moving end) is the end opposite the anchor.
    // When the anchor is at sel_end and there is a range, the cursor is at
    // sel_start; otherwise it is at sel_end. This matters for Shift+Left: after
    // extending leftward past the anchor, sel_end == anchor and continuing to
    // press Shift+Left must move from sel_start, not get stuck at sel_end.
    let active = if *sel_end == anchor && *sel_start != *sel_end {
        (*sel_start).clamp(0, len)
    } else {
        (*sel_end).clamp(0, len)
    };
    let has_range = hi > lo;

    match key {
        "ArrowLeft" => {
            let new_pos = if has_range && !shift {
                lo
            } else if ctrl_or_meta {
                prev_word_boundary(active)
            } else {
                prev_char_boundary(active)
            };
            if shift {
                extend(sel_start, sel_end, anchor, new_pos, len);
            } else {
                collapse(sel_start, sel_end, sel_anchor, new_pos, len);
            }
        }
        "ArrowRight" => {
            let new_pos = if has_range && !shift {
                hi
            } else if ctrl_or_meta {
                next_word_boundary(active)
            } else {
                next_char_boundary(active)
            };
            if shift {
                extend(sel_start, sel_end, anchor, new_pos, len);
            } else {
                collapse(sel_start, sel_end, sel_anchor, new_pos, len);
            }
        }
        "ArrowUp" => {
            let cur_line_start = line_start_of(active);
            let col = active - cur_line_start;
            let mut new_pos = if cur_line_start == 0 {
                0
            } else {
                let prev_line_end = cur_line_start - 1;
                let prev_line_start = line_start_of(prev_line_end);
                prev_line_start + col.min(prev_line_end - prev_line_start)
            };
            // `col` is a byte offset; preserving it across lines with
            // different multi-byte char counts can land mid-codepoint
            // (panic on next edit). Snap to the nearest legal char
            // boundary, walking backwards so visual column stays close
            // to where the user expects.
            while (new_pos as usize) < text.len()
                && !text.is_char_boundary(new_pos as usize)
            {
                new_pos -= 1;
            }
            if shift {
                extend(sel_start, sel_end, anchor, new_pos, len);
            } else {
                collapse(sel_start, sel_end, sel_anchor, new_pos, len);
            }
        }
        "ArrowDown" => {
            let cur_line_start = line_start_of(active);
            let col = active - cur_line_start;
            let cur_line_end = line_end_of(active);
            let mut new_pos = if cur_line_end >= len {
                len
            } else {
                let next_line_start = cur_line_end + 1;
                let next_line_end = line_end_of(next_line_start);
                next_line_start + col.min(next_line_end - next_line_start)
            };
            while (new_pos as usize) < text.len()
                && !text.is_char_boundary(new_pos as usize)
            {
                new_pos -= 1;
            }
            if shift {
                extend(sel_start, sel_end, anchor, new_pos, len);
            } else {
                collapse(sel_start, sel_end, sel_anchor, new_pos, len);
            }
        }
        "Home" => {
            let new_pos = if ctrl_or_meta { 0 } else { line_start_of(active) };
            if shift {
                extend(sel_start, sel_end, anchor, new_pos, len);
            } else {
                collapse(sel_start, sel_end, sel_anchor, new_pos, len);
            }
        }
        "End" => {
            let new_pos = if ctrl_or_meta { len } else { line_end_of(active) };
            if shift {
                extend(sel_start, sel_end, anchor, new_pos, len);
            } else {
                collapse(sel_start, sel_end, sel_anchor, new_pos, len);
            }
        }
        "Backspace" => {
            if has_range {
                text.replace_range(lo as usize..hi as usize, "");
                collapse(sel_start, sel_end, sel_anchor, lo, text.len() as i32);
            } else if lo > 0 {
                let prev = prev_char_boundary(lo);
                text.replace_range(prev as usize..lo as usize, "");
                collapse(sel_start, sel_end, sel_anchor, prev, text.len() as i32);
            }
        }
        "Delete" => {
            if has_range {
                text.replace_range(lo as usize..hi as usize, "");
                collapse(sel_start, sel_end, sel_anchor, lo, text.len() as i32);
            } else if lo < len {
                let next = next_char_boundary(lo);
                text.replace_range(lo as usize..next as usize, "");
                collapse(sel_start, sel_end, sel_anchor, lo, text.len() as i32);
            }
        }
        // Cmd/Ctrl+A → select all (the only modifier-letter combo we own here;
        // others fall through and are NOT inserted).
        "a" | "A" if ctrl_or_meta => {
            *sel_start = 0;
            *sel_end = len;
            *sel_anchor = 0;
        }
        // Tab is handled by the caller (advances focus across editable
        // members). Enter inserts a Director-style \r line terminator;
        // scripts that want Enter to mean "submit" trap keyDown and
        // suppress default insertion.
        "Tab" => {}
        "Enter" => {
            text.replace_range(lo as usize..hi as usize, "\r");
            let new_caret = lo + 1;
            collapse(sel_start, sel_end, sel_anchor, new_caret, text.len() as i32);
        }
        _ => {
            // Single-character insertion. Browser keydown delivers printable
            // chars as 1-codepoint strings: "a", "Ä", "ä", "ñ", "€", "ß".
            // Named keys (Tab, Backspace, ArrowLeft, F5, ...) arrive as
            // multi-char strings and are filtered out here. The previous
            // `key.len() == 1` byte-length check rejected umlauts because
            // "ä" is 2 bytes in UTF-8. Any modifier other than Shift still
            // suppresses insertion (Cmd+S, Ctrl+B, etc.).
            let one_printable_char = {
                let mut it = key.chars();
                match (it.next(), it.next()) {
                    (Some(c), None) => !c.is_control(),
                    _ => false,
                }
            };
            if one_printable_char && !ctrl_or_meta {
                text.replace_range(lo as usize..hi as usize, key);
                // `key.len()` is the UTF-8 byte length -- correct here
                // because sel_start/sel_end are byte indices into `text`.
                let new_caret = lo + key.len() as i32;
                collapse(sel_start, sel_end, sel_anchor, new_caret, text.len() as i32);
            }
        }
    }
}

/// Mode for `set_caret_at_screen` — controls how the new position interacts
/// with the existing selection.
pub(crate) enum CaretAtMode {
    /// Single click: collapse selection at the new position.
    SetAndAnchor,
    /// Shift+click: extend selection from sel_anchor to the new position.
    ExtendToAnchor,
    /// Drag: move sel_end to the new position; sel_anchor stays put.
    DragExtend,
    /// Double-click: select word containing the new position.
    SelectWord,
    /// Triple-click: select line containing the new position.
    SelectLine,
}

/// Convert a movie-space (x, y) point into a caret index inside the focused
/// editable member at `sprite_id`, then apply the requested selection update.
///
/// Returns `true` if the sprite was an editable Field/Text member and the
/// caret state was updated.
pub(crate) fn set_caret_at_screen(
    sprite_id: i16,
    movie_x: i32,
    movie_y: i32,
    mode: CaretAtMode,
) -> bool {
    let result = super::reserve_player_mut(|player| {
        let Some(sprite) = player.movie.score.get_sprite(sprite_id) else { return false };
        let Some(member_ref) = sprite.member.clone() else { return false };
        let sprite_loc_h = sprite.loc_h;
        let sprite_loc_v = sprite.loc_v;
        let sprite_width = sprite.width;
        // Match the renderer's stage-scaling: the GPU/PFR text path
        // rasterises glyphs at `font_size * stage_scale` (see
        // rendering_gpu/webgl2/mod.rs render-text entry), so the rasterised
        // atlas's `char_widths` reflect the scaled-up glyph widths. Without
        // applying the same scale here, `xy_to_caret_index` walks a
        // differently-sized font's char_widths than the renderer drew --
        // the click maps to half the visual position on a 2x-scaled stage.
        let (scale_x, scale_y) = crate::player::stage::stage_scale(player);
        let stage_scale = scale_x.min(scale_y);

        // Resolve which editable member kind we have, and its text + style
        // properties needed to drive xy_to_caret_index. We work on a snapshot
        // so we can borrow font_manager / cast_manager separately below.
        enum MemberSnapshot {
            Field {
                text: String,
                font: String,
                font_size: u16,
                font_id: Option<u16>,
                alignment: String,
                fixed_line_space: u16,
                top_spacing: i16,
                word_wrap: bool,
            },
            Text {
                text: String,
                font: String,
                font_size: u16,
                fixed_line_space: u16,
                top_spacing: i16,
            },
        }
        let snapshot = {
            let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
            let Some(member) = member else { return false };
            match &member.member_type {
                CastMemberType::Field(f) if f.editable => MemberSnapshot::Field {
                    text: f.text.clone(),
                    font: f.font.clone(),
                    font_size: f.font_size,
                    font_id: f.font_id,
                    alignment: f.alignment.clone(),
                    fixed_line_space: f.fixed_line_space,
                    top_spacing: f.top_spacing,
                    word_wrap: f.word_wrap,
                },
                CastMemberType::Text(t)
                    if t.info.as_ref().map_or(false, |i| i.editable) =>
                {
                    MemberSnapshot::Text {
                        text: t.text.clone(),
                        font: t.font.clone(),
                        font_size: t.font_size,
                        fixed_line_space: t.fixed_line_space,
                        top_spacing: t.top_spacing,
                    }
                }
                _ => return false,
            }
        };

        // Compute caret index from movie coords, using the appropriate font.
        let caret_idx = match &snapshot {
            MemberSnapshot::Field { text, font, font_size, font_id, alignment,
                                    fixed_line_space, top_spacing, word_wrap: _ } => {
                // Mirror the renderer: rasterise the font at the SAME scaled
                // size the PFR atlas uses (`font_size * stage_scale`), AND
                // use the SAME font-lookup path as the renderer so we get
                // the exact same cached BitmapFont (same char_widths, same
                // first_char_num). The previous `get_or_load_font_with_id`
                // path had a different cache-key scheme (case-sensitive)
                // and could fall through to a fuzzy `starts_with` match
                // that returned a DIFFERENT-size font than the renderer
                // used -- when char_widths differed, multibyte chars
                // diverged from ASCII chars because per-char proportional
                // widths don't scale uniformly.
                let scaled_font_size = ((*font_size as f64) * stage_scale)
                    .round().max(1.0) as u16;
                let font_opt = player.font_manager.get_font_with_cast_and_bitmap(
                    font,
                    &player.movie.cast_manager,
                    &mut player.bitmap_manager,
                    Some(scaled_font_size),
                    None,
                ).or_else(|| {
                    // Same font_id fallback the renderer uses when the
                    // name-based lookup misses (rich-text spans reference
                    // fonts by ID).
                    font_id.and_then(|id| {
                        player.font_manager.font_by_id.get(&id).copied()
                            .and_then(|fr| player.font_manager.fonts.get(&fr).cloned())
                    })
                });
                let Some(f) = font_opt else { return false };
                // Scale the click coords into the renderer's coordinate
                // space too. movie_x is in unscaled movie space; the
                // renderer's char_widths sum is in scaled space.
                let local_x = ((movie_x - sprite_loc_h) as f64 * stage_scale).round() as i32;
                let local_y = ((movie_y - sprite_loc_v - *top_spacing as i32) as f64 * stage_scale).round() as i32;
                let scaled_width = ((sprite_width as f64) * stage_scale).round() as i32;
                let line_h = effective_line_height(&f, *fixed_line_space);
                crate::player::bitmap::bitmap::Bitmap::xy_to_caret_index(
                    text, &f, scaled_width, alignment, line_h, local_x, local_y,
                )
            }
            MemberSnapshot::Text { text, font, font_size, fixed_line_space, top_spacing } => {
                // Same stage-scaled font lookup + coord scaling as the
                // Field branch above. See its comment for the rationale.
                let scaled_font_size = ((*font_size as f64) * stage_scale)
                    .round().max(1.0) as u16;
                let font_opt = player.font_manager.get_font_with_cast_and_bitmap(
                    font,
                    &player.movie.cast_manager,
                    &mut player.bitmap_manager,
                    Some(scaled_font_size),
                    None,
                );
                let Some(f) = font_opt else { return false };
                // For TextMember bitmap path we treat width as unbounded, matching
                // the renderer (see rendering.rs draw_text-with-caret block).
                let local_x = ((movie_x - sprite_loc_h) as f64 * stage_scale).round() as i32;
                let local_y = ((movie_y - sprite_loc_v - *top_spacing as i32) as f64 * stage_scale).round() as i32;
                let line_h = effective_line_height(&f, *fixed_line_space);
                crate::player::bitmap::bitmap::Bitmap::xy_to_caret_index(
                    text, &f, 0, "left", line_h, local_x, local_y,
                )
            }
        };

        // Apply the selection update to the live member (re-fetch mutably).
        let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) else {
            return false;
        };

        let (text_ref, sel_start_ref, sel_end_ref, sel_anchor_ref):
            (&str, &mut i32, &mut i32, &mut i32) = match &mut member.member_type {
            CastMemberType::Field(f) => (&f.text, &mut f.sel_start, &mut f.sel_end, &mut f.sel_anchor),
            CastMemberType::Text(t) => (&t.text, &mut t.sel_start, &mut t.sel_end, &mut t.sel_anchor),
            _ => return false,
        };
        let text_len = text_ref.len() as i32;
        let bytes = text_ref.as_bytes();
        let clamped = caret_idx.clamp(0, text_len);

        match mode {
            CaretAtMode::SetAndAnchor => {
                *sel_start_ref = clamped;
                *sel_end_ref = clamped;
                *sel_anchor_ref = clamped;
            }
            CaretAtMode::ExtendToAnchor => {
                let anchor = (*sel_anchor_ref).clamp(0, text_len);
                *sel_start_ref = anchor.min(clamped);
                *sel_end_ref = anchor.max(clamped);
            }
            CaretAtMode::DragExtend => {
                let anchor = (*sel_anchor_ref).clamp(0, text_len);
                *sel_start_ref = anchor.min(clamped);
                *sel_end_ref = anchor.max(clamped);
            }
            CaretAtMode::SelectWord => {
                // Char-aware word selection: double-clicking inside "café"
                // selects the whole word instead of stopping at the lead
                // byte of 'é'. We use `text_snapshot` (Rust String) and
                // walk char_indices to find the surrounding word boundaries.
                let click_b = clamped as usize;
                let is_word_ch = |c: char| c.is_alphanumeric() || c == '_';
                // Materialise (byte_pos, char) for the line. Strings in
                // editable fields are short -- one-shot collection is fine.
                let chars: Vec<(usize, char)> =
                    text_ref.char_indices().collect();
                // Locate the char-index whose byte_pos straddles click_b.
                // `i` is the first char-index whose byte_pos >= click_b;
                // if there's a char strictly before that, prefer it for
                // the "click_b is on a char boundary" case.
                let i = chars.iter().position(|(b, _)| *b >= click_b)
                    .unwrap_or(chars.len());
                let (mut s, mut e) = (i, i);
                // If the click lands on a word char, expand both sides;
                // if it's right after one (caret between word and non-
                // word), grab the trailing word -- matches the byte
                // behaviour for ASCII text.
                let on_word = chars.get(i).map_or(false, |(_, c)| is_word_ch(*c));
                let after_word = i > 0 && is_word_ch(chars[i - 1].1);
                if on_word {
                    while s > 0 && is_word_ch(chars[s - 1].1) { s -= 1; }
                    while e < chars.len() && is_word_ch(chars[e].1) { e += 1; }
                } else if after_word {
                    while s > 0 && is_word_ch(chars[s - 1].1) { s -= 1; }
                    e = i;
                }
                let start_b = chars.get(s).map(|(b, _)| *b).unwrap_or(text_ref.len());
                let end_b = chars.get(e).map(|(b, _)| *b).unwrap_or(text_ref.len());
                *sel_start_ref = start_b as i32;
                *sel_end_ref = end_b as i32;
                *sel_anchor_ref = start_b as i32;
            }
            CaretAtMode::SelectLine => {
                let mut start = clamped as usize;
                let mut end = clamped as usize;
                let n = bytes.len();
                while start > 0 && bytes[start - 1] != b'\r' && bytes[start - 1] != b'\n' { start -= 1; }
                while end < n && bytes[end] != b'\r' && bytes[end] != b'\n' { end += 1; }
                *sel_start_ref = start as i32;
                *sel_end_ref = end as i32;
                *sel_anchor_ref = start as i32;
            }
        }

        let new_start = *sel_start_ref;
        let new_end = *sel_end_ref;
        player.text_selection_start = new_start.max(0) as u16;
        player.text_selection_end = new_end.max(0) as u16;
        true
    });
    result
}

/// Insert raw text at the current selection (or replace it). Used by paste and
/// IME commit. Same selection-update semantics as char insertion.
pub(crate) fn apply_text_insertion(
    text: &mut String,
    sel_start: &mut i32,
    sel_end: &mut i32,
    sel_anchor: &mut i32,
    insertion: &str,
) {
    let len = text.len() as i32;
    let mut lo = (*sel_start).clamp(0, len);
    let mut hi = (*sel_end).clamp(0, len);
    if lo > hi {
        std::mem::swap(&mut lo, &mut hi);
    }
    text.replace_range(lo as usize..hi as usize, insertion);
    let new_caret = lo + insertion.len() as i32;
    let new_len = text.len() as i32;
    *sel_start = new_caret.clamp(0, new_len);
    *sel_end = *sel_start;
    *sel_anchor = *sel_start;
}

pub async fn player_key_down(key: String, code: u16) -> Result<DatumRef, ScriptError> {
    if !player_is_playing().await {
        return Ok(DatumRef::Void);
    }

    // Note: keyboard_manager.key_down() is NOT called here because it's already
    // handled immediately in the WASM entry point (lib.rs key_down()). Calling it
    // here would re-add keys from stale queued commands after the user released them.
    let (instance_ids, is_editable_member, sprite_id) = reserve_player_mut(|player| {
        if player.keyboard_focus_sprite != -1 {
            let sprite_id = player.keyboard_focus_sprite as i16;
            player.sync_script_instance_list(sprite_id);
            let sprite = player.movie.score.get_sprite(sprite_id);
            if let Some(sprite) = sprite {
                let instance_list = sprite.script_instance_list.clone();
                let editable = focused_member_is_editable(player);
                (Some(instance_list), editable, sprite_id)
            } else {
                (None, false, -1)
            }
        } else {
            (None, false, -1)
        }
    });

    // Director event propagation order:
    // 1. Behavior scripts on focused sprite (synchronous)
    // 2. If not handled → frame script → movie scripts
    // 3. Movie callback
    // 4. If no script handled it → default text insertion / navigation
    let mut handled = false;
    if let Some(ref instances) = instance_ids {
        if !instances.is_empty() {
            handled = player_invoke_event_to_instances(
                &"keyDown".to_string(), &vec![], instances,
            ).await?;
        }
    }
    if !handled {
        player_invoke_frame_and_movie_scripts(&"keyDown".to_string(), &vec![]).await?;
    }
    player_dispatch_movie_callback("keyDown").await?;

    // Default text insertion / navigation: only if no script handled the event
    // and the focused sprite still points at the same editable member.
    if is_editable_member && !handled {
        reserve_player_mut(|player| {
            if player.keyboard_focus_sprite != sprite_id {
                return;
            }
            let shift = player.keyboard_manager.is_shift_down();
            let ctrl = player.keyboard_manager.is_control_down();
            let cmd = player.keyboard_manager.is_command_down();
            let alt = player.keyboard_manager.is_alt_down();
            let alt_graph = player.keyboard_manager.is_alt_graph_down();
            // AltGr detection: German / Nordic / many EU layouts make the
            // right Alt key produce printable characters like @, €, |, [, ].
            // The browser tells us this two ways depending on platform:
            //   (a) it fires `event.key = "AltGraph"` for the key itself
            //   (b) Windows synthesizes Ctrl+Alt and the event also has
            //       ctrlKey=true alongside altKey=true.
            // Both signal "the user is typing a layout-modified character,
            // not invoking a Ctrl shortcut". Treat either as AltGr and let
            // insertion proceed. Cmd (Mac Meta) is never AltGr -- those
            // shortcuts (Cmd+S, Cmd+C) keep getting suppressed.
            let is_alt_gr = alt_graph || (ctrl && alt);
            let ctrl_or_meta = cmd || (ctrl && !is_alt_gr);

            // Tab handled separately (advances focus across editable members);
            // hand off so apply_text_edit doesn't see it.
            if key == "Tab" {
                let next = get_next_focus_sprite_id(player, sprite_id);
                player.keyboard_focus_sprite = next;
                return;
            }

            let sprite = player.movie.score.get_sprite(sprite_id);
            let member_ref = sprite.and_then(|s| s.member.clone());
            let member = member_ref.and_then(|r| player.movie.cast_manager.find_mut_member_by_ref(&r));
            let Some(member) = member else { return };

            match &mut member.member_type {
                CastMemberType::Field(field) if field.editable => {
                    apply_text_edit(
                        &mut field.text,
                        &mut field.sel_start,
                        &mut field.sel_end,
                        &mut field.sel_anchor,
                        &key,
                        ctrl_or_meta,
                        shift,
                    );
                    player.text_selection_start = field.sel_start.max(0) as u16;
                    player.text_selection_end = field.sel_end.max(0) as u16;
                }
                CastMemberType::Text(text_member)
                    if text_member.info.as_ref().map_or(false, |i| i.editable) =>
                {
                    apply_text_edit(
                        &mut text_member.text,
                        &mut text_member.sel_start,
                        &mut text_member.sel_end,
                        &mut text_member.sel_anchor,
                        &key,
                        ctrl_or_meta,
                        shift,
                    );
                    player.text_selection_start = text_member.sel_start.max(0) as u16;
                    player.text_selection_end = text_member.sel_end.max(0) as u16;
                }
                _ => {}
            }
        });
    }

    let _ = code;
    Ok(DatumRef::Void)
}

pub async fn player_key_up(key: String, code: u16) -> Result<DatumRef, ScriptError> {
    if !player_is_playing().await {
        return Ok(DatumRef::Void);
    }
    // Note: keyboard_manager.key_up() is NOT called here because it's already
    // handled immediately in the WASM entry point (lib.rs key_up()).
    let instance_ids = reserve_player_mut(|player| {
        if player.keyboard_focus_sprite != -1 {
            let sprite = player.keyboard_focus_sprite as usize;
            let sprite = player.movie.score.get_sprite(sprite as i16);
            sprite.map(|x| x.script_instance_list.clone())
        } else {
            None
        }
    });
    let mut handled = false;
    if let Some(ref instances) = instance_ids {
        if !instances.is_empty() {
            handled = player_invoke_event_to_instances(
                &"keyUp".to_string(), &vec![], instances,
            ).await?;
        }
    }
    if !handled {
        player_invoke_frame_and_movie_scripts(&"keyUp".to_string(), &vec![]).await?;
    }
    player_dispatch_movie_callback("keyUp").await?;
    Ok(DatumRef::Void)
}
