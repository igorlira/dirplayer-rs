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
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let prev_word_boundary = |from: i32| -> i32 {
        let mut p = from.clamp(0, len) as usize;
        while p > 0 && !is_word(bytes_for_nav[p - 1]) { p -= 1; }
        while p > 0 && is_word(bytes_for_nav[p - 1]) { p -= 1; }
        p as i32
    };
    let next_word_boundary = |from: i32| -> i32 {
        let n = bytes_for_nav.len();
        let mut p = from.clamp(0, len) as usize;
        while p < n && is_word(bytes_for_nav[p]) { p += 1; }
        while p < n && !is_word(bytes_for_nav[p]) { p += 1; }
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

    let active = *sel_end;
    let has_range = hi > lo;

    match key {
        "ArrowLeft" => {
            let new_pos = if has_range && !shift {
                lo
            } else if ctrl_or_meta {
                prev_word_boundary(active)
            } else {
                (active - 1).max(0)
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
                (active + 1).min(len)
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
            let new_pos = if cur_line_start == 0 {
                0
            } else {
                let prev_line_end = cur_line_start - 1;
                let prev_line_start = line_start_of(prev_line_end);
                prev_line_start + col.min(prev_line_end - prev_line_start)
            };
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
            let new_pos = if cur_line_end >= len {
                len
            } else {
                let next_line_start = cur_line_end + 1;
                let next_line_end = line_end_of(next_line_start);
                next_line_start + col.min(next_line_end - next_line_start)
            };
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
                text.replace_range((lo - 1) as usize..lo as usize, "");
                collapse(sel_start, sel_end, sel_anchor, lo - 1, text.len() as i32);
            }
        }
        "Delete" => {
            if has_range {
                text.replace_range(lo as usize..hi as usize, "");
                collapse(sel_start, sel_end, sel_anchor, lo, text.len() as i32);
            } else if lo < len {
                text.replace_range(lo as usize..(lo + 1) as usize, "");
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
            // Single ASCII char insertion. Any modifier other than Shift
            // suppresses insertion (Cmd+S, Ctrl+B, etc.).
            if key.len() == 1 && !ctrl_or_meta {
                text.replace_range(lo as usize..hi as usize, key);
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
                let font_opt = crate::rendering::get_or_load_font_with_id(
                    &mut player.font_manager,
                    &player.movie.cast_manager,
                    font,
                    Some(*font_size),
                    None,
                    *font_id,
                );
                let Some(f) = font_opt else { return false };
                let local_x = movie_x - sprite_loc_h;
                let local_y = movie_y - sprite_loc_v - *top_spacing as i32;
                let line_h = effective_line_height(&f, *fixed_line_space);
                // Always pass sprite_width so xy_to_caret_index can compute the
                // per-line alignment offset for centered/right text. Setting
                // max_width to 0 when word_wrap is off makes the helper assume
                // left alignment, which sends every click to the left edge of
                // the sprite even when the visible glyphs are centered.
                crate::player::bitmap::bitmap::Bitmap::xy_to_caret_index(
                    text, &f, sprite_width, alignment, line_h, local_x, local_y,
                )
            }
            MemberSnapshot::Text { text, font, font_size, fixed_line_space, top_spacing } => {
                let font_opt = crate::rendering::get_or_load_font(
                    &mut player.font_manager,
                    &player.movie.cast_manager,
                    font,
                    Some(*font_size),
                    None,
                );
                let Some(f) = font_opt else { return false };
                // For TextMember bitmap path we treat width as unbounded, matching
                // the renderer (see rendering.rs draw_text-with-caret block).
                let local_x = movie_x - sprite_loc_h;
                let local_y = movie_y - sprite_loc_v - *top_spacing as i32;
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
                let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
                let mut start = clamped as usize;
                let mut end = clamped as usize;
                let n = bytes.len();
                if start < n && is_word(bytes[start]) {
                    while start > 0 && is_word(bytes[start - 1]) { start -= 1; }
                    while end < n && is_word(bytes[end]) { end += 1; }
                } else if start > 0 && is_word(bytes[start - 1]) {
                    while start > 0 && is_word(bytes[start - 1]) { start -= 1; }
                    end = clamped as usize;
                }
                *sel_start_ref = start as i32;
                *sel_end_ref = end as i32;
                *sel_anchor_ref = start as i32;
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
            let ctrl_or_meta = player.keyboard_manager.is_control_down()
                || player.keyboard_manager.is_command_down();

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
