use std::collections::VecDeque;
use itertools::Itertools;

use crate::{
    director::lingo::datum::{Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType, datum_bool},
    player::{
        ColorRef, DatumRef, DirPlayer, ScriptError, bitmap::{bitmap::{Bitmap, BuiltInPalette, PaletteRef}, drawing::CopyPixelsParams}, cast_lib::CastMemberRef, cast_member::Media, font::{get_text_index_at_pos, measure_text, measure_text_wrapped, DrawTextParams}, handlers::datum_handlers::{
            cast_member_ref::borrow_member_mut, string::{string_get_lines, string_get_words}, string_chunk::StringChunkUtils
        }
    },
};

pub struct FieldMemberHandlers {}

impl FieldMemberHandlers {
    pub fn call(
        player: &mut DirPlayer,
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "count" => {
                let member_ref = player.get_datum(datum).to_member_ref()?;
                let member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .unwrap();
                let field = member.member_type.as_field().unwrap();
                let count_of = player.get_datum(&args[0]).string_value()?;
                if args.len() != 1 {
                    return Err(ScriptError::new("count requires 1 argument".to_string()));
                }
                let delimiter = player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &field.text,
                    StringChunkType::from(&count_of),
                    delimiter,
                )?;
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            }
            "getPropRef" => {
                let member_ref = player.get_datum(datum).to_member_ref()?;
                let member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .unwrap();
                let field = member.member_type.as_field().unwrap();
                let prop_name = player.get_datum(&args[0]).string_value()?;
                let start = player.get_datum(&args[1]).int_value()?;
                let end = if args.len() > 2 {
                    player.get_datum(&args[2]).int_value()?
                } else {
                    start
                };
                let chunk_type = StringChunkType::from(&prop_name);
                let chunk_expr = StringChunkExpr {
                    chunk_type,
                    start,
                    end,
                    item_delimiter: player.movie.item_delimiter,
                };
                let resolved_str =
                    StringChunkUtils::resolve_chunk_expr_string(&field.text, &chunk_expr)?;
                Ok(player.alloc_datum(Datum::StringChunk(
                    StringChunkSource::Member(member_ref),
                    chunk_expr,
                    resolved_str,
                )))
            }
            "setContents" => {
                if args.len() != 1 {
                    return Err(ScriptError::new(
                        "setContents requires 1 argument".to_string(),
                    ));
                }
                let new_contents = player.get_datum(&args[0]).string_value()?;
                let member_ref = player.get_datum(datum).to_member_ref()?;
                let member = player
                    .movie
                    .cast_manager
                    .find_mut_member_by_ref(&member_ref)
                    .unwrap()
                    .member_type
                    .as_field_mut()
                    .unwrap();
                member.set_text_preserving_caret(new_contents.trim_end_matches('\0').to_string());
                Ok(DatumRef::Void)
            }
            "locToCharPos" => {
                // `member(N).locToCharPos(point(x, y))` — returns the 1-based
                // char index under the local (member-relative) coordinate.
                // Mirrors the text-member implementation in cast_member/text.rs.
                //
                // Must honor scroll_top + word_wrap: Narrative_Buttons#upCount
                // / downCount rely on the index returned by
                // `locToCharPos(point(1, 1))` *changing* as scrollTop is bumped
                // each iteration. If the index stays constant the inner
                // `repeat while (x = z) and (scrollTop > 0)` loop never
                // terminates and the movie hard-stucks. Without word-wrap,
                // a paged narrative member only has \r\n line breaks so the
                // visible page boundary is determined purely by wrapping.
                let (pt_vals, _flags) = player.get_datum(&args[0]).to_point_inline()?;
                let x = pt_vals[0] as i32;
                let y = pt_vals[1] as i32;
                let member_ref = player.get_datum(datum).to_member_ref()?;
                let member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .unwrap();
                let field = member.member_type.as_field().unwrap();
                let text_clone = field.text.clone();
                let line_spacing = field.fixed_line_space;
                let top_spacing = field.top_spacing;
                let scroll_top = field.scroll_top as i32;
                let wrap_width = field.width as i16;
                let word_wrap = field.word_wrap;
                let field_font_name = field.font.clone();
                let field_font_size = field.font_size;
                // Drop the immutable borrow on `member` (held implicitly
                // via `field`) before we mutably borrow font_manager.
                drop(field);
                drop(member);
                // Use the field's actual font (matching the renderer's
                // atlas) instead of the system font. Hit-testing against
                // system Arial widths while the renderer drew with PFR
                // Arial widths caused clicks to map to the wrong
                // underlined run (Fugue No.4: clicking visual "Christ's
                // passion" returned a char position in "the sign of the
                // cross").
                let field_font = player.font_manager.get_font_with_cast_and_bitmap(
                    &field_font_name,
                    &player.movie.cast_manager,
                    &mut player.bitmap_manager,
                    if field_font_size > 0 { Some(field_font_size) } else { None },
                    None,
                );
                let font_arc = field_font.or_else(|| player.font_manager.get_system_font());
                let font_rc = match font_arc {
                    Some(f) => f,
                    None => return Ok(player.alloc_datum(Datum::Int(0))),
                };
                let min_space_adv = {
                    let sz = font_rc.font_size.max(font_rc.char_height) as i32;
                    let v = ((sz as f32) * 0.30).round() as i16;
                    if v > 0 { Some(v) } else { None }
                };
                let params = DrawTextParams {
                    font: &font_rc,
                    line_height: None,
                    line_spacing,
                    top_spacing,
                    char_spacing: 0,
                    member_width: if word_wrap && wrap_width > 0 { Some(wrap_width) } else { None },
                    // Match the renderer's space clamp so locToCharPos
                    // returns positions consistent with the drawn layout.
                    min_space_advance: min_space_adv,
                    // TODO: build run-aware per-char advances here too
                    // (mirror compute_mouse_char). Without it, scripts
                    // calling `member.locToCharPos(point(x,y))` on a
                    // mixed-font field get the same wrap-drift bug that
                    // `the mouseChar` had before its per-run fix.
                    per_char_advances: None,
                };
                let index = get_text_index_at_pos(&text_clone, &params, x, y + scroll_top);
                Ok(player.alloc_datum(Datum::Int((index + 1) as i32)))
            }
            // `member.scrollByLine(amount)` — Director 11.5 Scripting
            // Dictionary p.618: "scrolls the specified field or text cast
            // member up or down by a specified number of lines. When amount
            // is positive, the field scrolls down. When amount is negative,
            // the field scrolls up." Fugue No.4's `fixSplitLines` passes
            // fractional amounts (±0.1) for sub-line nudges to align the
            // scroll position to a clean line boundary.
            "scrollByLine" => {
                let amount = player.get_datum(&args[0]).to_float()?;
                let member_ref = player.get_datum(datum).to_member_ref()?;
                let line_h = {
                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref).unwrap();
                    let field = member.member_type.as_field().unwrap();
                    if field.fixed_line_space > 0 {
                        field.fixed_line_space as f64
                    } else if field.font_size > 0 {
                        field.font_size as f64
                    } else {
                        12.0
                    }
                };
                let delta_px = (amount * line_h).round() as i32;
                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                    if let Some(field) = member.member_type.as_field_mut() {
                        let new_st = (field.scroll_top as i32 + delta_px).max(0) as u16;
                        field.scroll_top = new_st;
                    }
                }
                Ok(DatumRef::Void)
            }
            // Director 11.5 Scripting Dictionary p.443 / p.449.
            "linePosToLocV" | "locVToLinePos" => {
                let member_ref = player.get_datum(datum).to_member_ref()?;
                let member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .unwrap();
                let field = member.member_type.as_field().unwrap();
                let step = super::text::line_step_px(field.fixed_line_space, field.font_size).max(1);
                let arg = player.get_datum(&args[0]).int_value()?;
                if handler_name == "linePosToLocV" {
                    let line_num = arg.max(1);
                    // Baseline anchor — see text.rs linePosToLocV comment.
                    let baseline_offset = (step * 3) / 4;
                    let y = field.top_spacing as i32 + (line_num - 1) * step + baseline_offset;
                    Ok(player.alloc_datum(Datum::Int(y)))
                } else {
                    // Director uses bare \r as its line separator — see
                    // count_director_lines() comment in text.rs for why
                    // str::lines() is wrong here.
                    let line_count = super::text::count_director_lines(&field.text) as i32;
                    let line = ((arg - field.top_spacing as i32) / step) + 1;
                    Ok(player.alloc_datum(Datum::Int(line.clamp(1, line_count))))
                }
            }
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for field member type"
            ))),
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref)
            .unwrap();
        let field = member.member_type.as_field().unwrap();

        match prop {
            "text" => Ok(Datum::String(field.text.to_owned())),
            "font" => Ok(Datum::String(field.font.to_owned())),
            "fontSize" => Ok(Datum::Int(field.font_size as i32)),
            "fontStyle" => Ok(Datum::String(field.font_style.to_owned())),
            "width" => Ok(Datum::Int(field.width as i32)),
            "alignment" => Ok(Datum::String(field.alignment.to_owned())),
            "wordWrap" => Ok(datum_bool(field.word_wrap)),
            "fixedLineSpace" | "lineHeight" => Ok(Datum::Int(field.fixed_line_space as i32)),
            "topSpacing" => Ok(Datum::Int(field.top_spacing as i32)),
            "boxType" => Ok(Datum::String(field.box_type.to_owned())),
            "antialias" => Ok(datum_bool(field.anti_alias)),
            "autoTab" => Ok(datum_bool(field.auto_tab)),
            "editable" => Ok(datum_bool(field.editable)),
            "border" => Ok(Datum::Int(field.border as i32)),
            "margin" => Ok(Datum::Int(field.margin as i32)),
            "boxDropShadow" => Ok(Datum::Int(field.box_drop_shadow as i32)),
            "dropShadow" => Ok(Datum::Int(field.drop_shadow as i32)),
            "scrollTop" => Ok(Datum::Int(field.scroll_top as i32)),
            "hilite" => Ok(datum_bool(field.hilite)),
            "lineCount" => {
                if field.text.is_empty() {
                    Ok(Datum::Int(0))
                } else {
                    Ok(Datum::Int(field.text.lines().count().max(1) as i32))
                }
            }
            "line" => {
                let lines = string_get_lines(&field.text);
                let line_datums: VecDeque<_> = lines.into_iter().map(Datum::String).map(|d| player.alloc_datum(d)).collect();
                Ok(Datum::List(DatumType::List, line_datums, false))
            }
            "word" => {
                let words = string_get_words(&field.text);
                let word_datums: VecDeque<_> = words.into_iter().map(Datum::String).map(|d| player.alloc_datum(d)).collect();
                Ok(Datum::List(DatumType::List, word_datums, false))
            }
            // `member.char` / `member.item` — sibling of `member.line` /
            // `member.word`: returns the full list of chunks so the script
            // can index it. Narrative_Buttons#upCount uses
            // `member(nar).char[y..y+8]` to extract a sliding 8-char window;
            // without this, the get_prop call errors out and the button
            // scroll logic never runs. (Director 11.5 Scripting Dictionary
            // chunk-of-member entry.)
            "char" => {
                let chars: Vec<String> = field.text.chars().map(|c| c.to_string()).collect();
                let char_datums: VecDeque<_> = chars.into_iter()
                    .map(Datum::String)
                    .map(|d| player.alloc_datum(d))
                    .collect();
                Ok(Datum::List(DatumType::List, char_datums, false))
            }
            "item" => {
                let delim = player.movie.item_delimiter;
                let items: Vec<String> = field.text.split(delim).map(|s| s.to_string()).collect();
                let item_datums: VecDeque<_> = items.into_iter()
                    .map(Datum::String)
                    .map(|d| player.alloc_datum(d))
                    .collect();
                Ok(Datum::List(DatumType::List, item_datums, false))
            }
            "pageHeight" => {
                // Director 11.5 Scripting Dictionary p.1077: "returns the
                // height, in pixels, of the area of the field cast member
                // that is visible on the Stage." For paged/scrollable
                // field members (like Fugue No.4's Narrative — authored as
                // a 250×9442 scroll container so the entire 26k-char body
                // lives inside one member), the visible viewport is set
                // by the SPRITE that hosts the member, not by the
                // member's own rect or text_height. Look up the first
                // sprite on the current frame whose member matches and
                // use its height. Fall back to field.height / rect-derived
                // height if no sprite is using the member.
                let field_height = field.height as i32;
                let rect_h = (field.rect_bottom as i32 - field.rect_top as i32).max(0);
                let cm_ref = cast_member_ref.clone();
                let frame = player.movie.current_frame;
                let sprite_h: Option<i32> = player.movie.score
                    .get_sorted_channels(frame)
                    .iter()
                    .find_map(|ch| {
                        if ch.sprite.member.as_ref() == Some(&cm_ref) {
                            Some((ch.sprite.height as i32).max(0))
                        } else {
                            None
                        }
                    });
                // The text area inside the sprite excludes the field's
                // border and margin (Director's box chrome). For Fugue
                // No.4 Narrative (border=1, margin=5) that's 12px of
                // chrome — without subtracting it, each PageNext
                // over-scrolls by ~1 line. Match Director's reported
                // pageHeight which is the *text* area, not the sprite rect.
                let chrome = 2 * (field.border as i32) + 2 * (field.margin as i32);
                let raw = sprite_h
                    .filter(|h| *h > 0)
                    .unwrap_or(field_height.max(rect_h));
                Ok(Datum::Int((raw - chrome).max(1)))
            }
            "foreColor" => {
                match &field.fore_color {
                    Some(ColorRef::Rgb(r, _, _)) => Ok(Datum::Int(*r as i32)),
                    Some(ColorRef::PaletteIndex(idx)) => Ok(Datum::Int(*idx as i32)),
                    None => Ok(Datum::Int(255)), // default: palette index 255 = black
                }
            }
            "backColor" => {
                match &field.back_color {
                    Some(ColorRef::Rgb(r, _, _)) => Ok(Datum::Int(*r as i32)),
                    Some(ColorRef::PaletteIndex(idx)) => Ok(Datum::Int(*idx as i32)),
                    None => Ok(Datum::Int(0)), // default: palette index 0 = white
                }
            }
            "rect" | "height" | "picture" => {
                // Clone data to avoid borrow issues
                let text_clone = field.text.clone();
                let font_name = field.font.clone();
                let font_size = Some(field.font_size);
                let fixed_line_space = field.fixed_line_space;
                let top_spacing = field.top_spacing;
                let alignment = field.alignment.clone();
                let word_wrap = field.word_wrap;
                let fore_color = field.fore_color.clone().unwrap_or(ColorRef::PaletteIndex(255));
                let field_width = field.width;

                // Try to get custom font, fall back to system font
                let font = if !font_name.is_empty() {
                    player.font_manager.get_font_with_cast_and_bitmap(
                        &font_name,
                        &player.movie.cast_manager,
                        &mut player.bitmap_manager,
                        font_size,
                        None,
                    )
                } else {
                    None
                };

                let font = if let Some(f) = font {
                    f
                } else {
                    player
                        .font_manager
                        .get_system_font()
                        .ok_or_else(|| ScriptError::new("System font not available".to_string()))?
                };

                // Use wrap-aware measurement when word_wrap is on so the
                // rect/height getter reports the wrapped layout — Director's
                // `the rect of member` for a wrapped field returns the
                // authored width and the height of the wrapped text. Without
                // this, our rect/height returned `measure_text(...)` which
                // measures unwrapped (single-line) — a long-line field
                // reported a too-wide rect (right = unwrapped line width
                // instead of field.width) and a too-short height (line
                // count of unwrapped vs wrapped). `pageHeight` already does
                // the wrap-aware path.
                let (measured_w, measured_h) = if word_wrap && field_width > 0 {
                    measure_text_wrapped(
                        &text_clone, &font, field_width, true,
                        fixed_line_space, top_spacing, 0, 0,
                    )
                } else {
                    measure_text(&text_clone, &font, None, fixed_line_space, top_spacing, 0)
                };
                let width = if field_width > 0 {
                    // Clamp to authored field.width when set — a wrapped
                    // field's display width is the authored width, not the
                    // measured (which may be the longest unwrapped line).
                    if word_wrap { field_width } else { field_width.max(measured_w) }
                } else {
                    measured_w
                };
                let height = if fixed_line_space > 0 {
                    fixed_line_space.max(measured_h)
                } else {
                    measured_h
                };

                match prop {
                    "rect" => Ok(Datum::Rect([0.0, 0.0, width as f64, height as f64], 0)),
                    "height" => Ok(Datum::Int(height as i32)),
                    "picture" => {
                        let mut bitmap = Bitmap::new(
                            width,
                            height,
                            32,
                            8,
                            0,
                            PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
                        );

                        // Fill with transparent background
                        for y in 0..height {
                            for x in 0..width {
                                let index =
                                    ((y as usize * width as usize + x as usize) * 4) as usize;
                                if index + 3 < bitmap.data.len() {
                                    bitmap.data[index] = 0;
                                    bitmap.data[index + 1] = 0;
                                    bitmap.data[index + 2] = 0;
                                    bitmap.data[index + 3] = 0;
                                }
                            }
                        }

                        let font_bitmap = player
                            .bitmap_manager
                            .get_bitmap(font.bitmap_ref)
                            .ok_or_else(|| ScriptError::new("Font bitmap not found".to_string()))?;
                        let palettes = player.movie.cast_manager.palettes();

                        let params = CopyPixelsParams {
                            blend: 100,
                            ink: 36,
                            color: fore_color,
                            bg_color: ColorRef::Rgb(255, 255, 255),
                            mask_image: None,
                            is_text_rendering: true,
                            rotation: 0.0,
                            skew: 0.0,
                            sprite: None,
                            mask_offset: (0, 0),
                            original_dst_rect: None,
                            bg_color_explicit: false,
                            fore_color_explicit: false,
                            ink9_mask_bitmap: None, ink9_mask_offset: (0, 0),
                        };

                        bitmap.draw_text(
                            &text_clone,
                            &font,
                            &font_bitmap,
                            0,
                            top_spacing as i32,
                            params,
                            &palettes,
                            fixed_line_space,
                            top_spacing,
                        );

                        // Field `.image` returns a freshly rasterized snapshot
                        // of the text. Each call rasterizes again, so the
                        // bitmap isn't anchored — let the DatumRef refcount
                        // free it when the script drops the value.
                        let bitmap_ref = player.bitmap_manager.add_ephemeral_bitmap(bitmap);
                        Ok(Datum::BitmapRef(bitmap_ref))
                    }
                    _ => unreachable!(),
                }
            }
            "media" => Ok(Datum::Media(Media::Field(field.clone()))),
            // Chunk count shortcuts — computed from text string.
            "charCount" => {
                let delimiter = player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &field.text, StringChunkType::Char, delimiter,
                )?;
                Ok(Datum::Int(count as i32))
            }
            "wordCount" => {
                let delimiter = player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &field.text, StringChunkType::Word, delimiter,
                )?;
                Ok(Datum::Int(count as i32))
            }
            "paragraphCount" => {
                // Director treats paragraphs as \r-delimited, same as lines.
                let delimiter = player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &field.text, StringChunkType::Line, delimiter,
                )?;
                Ok(Datum::Int(count as i32))
            }
            "tabCount" => Ok(Datum::Int(0)), // Fields don't have tab stops in our model
            // Runtime selection state
            "selStart" => Ok(Datum::Int(field.sel_start)),
            "selEnd" => Ok(Datum::Int(field.sel_end)),
            // Director 11.5 Scripting Dictionary p.1164. Returns a two-element
            // linear list `[start, end]` so scripts can test for an active
            // selection via `selection[1] <> selection[2]`.
            "selection" => {
                let start_val = field.sel_start;
                let end_val = field.sel_end;
                let start = player.alloc_datum(Datum::Int(start_val));
                let end = player.alloc_datum(Datum::Int(end_val));
                Ok(Datum::List(DatumType::List, VecDeque::from(vec![start, end]), false))
            }
            "selectedText" => {
                let len = field.text.len() as i32;
                let lo = field.sel_start.min(field.sel_end).clamp(0, len);
                let hi = field.sel_start.max(field.sel_end).clamp(0, len);
                Ok(Datum::String(field.text[lo as usize..hi as usize].to_string()))
            }
            // `the selection of member` — Director 11.5 Scripting
            // Dictionary p.1187: "returns the offsets of the start and
            // end of the selection ... a linear list with two integers
            // (selectionStart, selectionEnd)." Fugue No.4's
            // Cues#AdvanceScroll reads `member(nar).selection`
            // immediately after `hilite member(nar).line[o].char[1..x]`
            // to get the char offsets of the hilited range, then uses
            // `charPosToLoc` to translate to screen y for scrolling.
            "selection" => {
                let lo = field.sel_start.min(field.sel_end);
                let hi = field.sel_start.max(field.sel_end);
                let mut items = std::collections::VecDeque::new();
                items.push_back(player.alloc_datum(Datum::Int(lo)));
                items.push_back(player.alloc_datum(Datum::Int(hi)));
                Ok(Datum::List(DatumType::List, items, false))
            }
            // Text rendering config
            "kerning" => Ok(datum_bool(field.kerning)),
            "kerningThreshold" => Ok(Datum::Int(field.kerning_threshold as i32)),
            "useHypertextStyles" => Ok(datum_bool(field.use_hypertext_styles)),
            "antiAliasType" => Ok(Datum::Symbol(field.anti_alias_type.clone())),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for field",
                prop
            ))),
        }
    }

    pub fn set_prop(
        member_ref: &CastMemberRef,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        match prop {
            "text" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    field.set_text_preserving_caret(value?.trim_end_matches('\0').to_string());
                    Ok(())
                },
            ),
            "rect" => borrow_member_mut(
                member_ref,
                |_player| -> Result<(i32, i32, i32, i32), ScriptError> {
                    let (vals, _flags) = value.to_rect_inline()?;

                    let x1 = vals[0] as i32;
                    let y1 = vals[1] as i32;
                    let x2 = vals[2] as i32;
                    let y2 = vals[3] as i32;

                    Ok((x1, y1, x2, y2))
                },
                |cast_member, rect_values: Result<(i32, i32, i32, i32), ScriptError>| {
                    let (x1, y1, x2, y2) = rect_values?;
                    let field_data = cast_member.member_type.as_field_mut().unwrap();
                    let w = (x2 - x1).max(0) as u16;
                    let h = (y2 - y1).max(0) as u16;
                    if w > 0 {
                        field_data.width = w;
                    }
                    if h > 0 {
                        // Field BOX height — NOT line stride. The earlier
                        // assignment `fixed_line_space = h` clobbered the
                        // per-line stride with the box height (Coke Studios
                        // InfoStandDescription: rect=(0,0,190,55) made
                        // fixed_line_space=55 which the renderer then used
                        // as the line stride, putting each glyph at the
                        // bottom of a 55-px cell — only line 1 fit in the
                        // texture and showed at the bottom of the field).
                        field_data.height = h;
                    }
                    // Keep rect_* in sync so callers like
                    // get_concrete_sprite_rect (which uses these for
                    // bounding-box arithmetic) see the script-authored
                    // rect, not the FieldMember::new() defaults.
                    field_data.rect_left = x1 as i16;
                    field_data.rect_top = y1 as i16;
                    field_data.rect_right = x2 as i16;
                    field_data.rect_bottom = y2 as i16;

                    Ok(())
                }
            ),
            "alignment" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().alignment = value?;
                    Ok(())
                },
            ),
            "wordWrap" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().word_wrap = value?;
                    Ok(())
                },
            ),
            "width" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let w = value? as u16;
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    field.width = w;
                    // Mirror into rect_* (right = left + w) so consumers
                    // that read rect_right/rect_left stay consistent.
                    field.rect_right = field.rect_left.saturating_add(w as i16);
                    Ok(())
                },
            ),
            "height" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let h = value? as u16;
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    // Field BOX height. Do NOT touch `fixed_line_space` —
                    // that's the per-line stride (e.g. 13 for 12pt) and
                    // setting it to the box height (e.g. 55) makes the
                    // renderer place each glyph at the bottom of a tall
                    // cell. See `rect` setter above for the same bug fix.
                    field.height = h;
                    field.rect_bottom = field.rect_top.saturating_add(h as i16);
                    Ok(())
                },
            ),
            "font" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().font = value?;
                    Ok(())
                },
            ),
            "fontSize" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let font_size = value? as u16;
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    field.font_size = font_size;
                    Ok(())
                },
            ),
            // Whole-field style set (`set the textStyle of field X to "plain"`).
            // Update both the member-wide font_style string AND every STXT
            // formatting run, so the change is visible (the renderer reads the
            // runs) and the `the textStyle of` getter agrees. The client
            // (issue-188) movie's `markLine` clears all line highlights via
            // `set the textStyle of field fieldname to "plain"`.
            "fontStyle" | "textStyle" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    use crate::player::cast_member::text_style_string_to_byte;
                    let s = value?;
                    let style_byte = text_style_string_to_byte(&s);
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    field.font_style = s;
                    let len = field.text.len() as u32;
                    field.apply_style_to_byte_range(0, len, style_byte);
                    Ok(())
                },
            ),
            "fixedLineSpace" | "lineHeight" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
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
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().top_spacing = value? as i16;
                    Ok(())
                },
            ),
            "boxType" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().box_type = value?;
                    Ok(())
                },
            ),
            "antialias" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().anti_alias = value?;
                    Ok(())
                },
            ),
            "autoTab" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().auto_tab = value?;
                    Ok(())
                },
            ),
            "editable" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().editable = value?;
                    Ok(())
                },
            ),
            "border" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().border = value? as u16;
                    Ok(())
                },
            ),
            "margin" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().margin = value? as u16;
                    Ok(())
                },
            ),
            "boxDropShadow" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().box_drop_shadow = value? as u16;
                    Ok(())
                },
            ),
            "dropShadow" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().drop_shadow = value? as u16;
                    Ok(())
                },
            ),
            "scrollTop" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    // Clamp negative values to 0 — `scrollTop` is a non-
                    // negative pixel offset (Director 11.5 Scripting
                    // Dictionary p.1158). Without the clamp, the bounceUp
                    // animation in Narrative_Buttons (which decrements
                    // scrollTop by 1 each step from the top of the field)
                    // wraps -1 into u16::MAX, producing a single-frame
                    // white-flash as the text scrolls off-bitmap.
                    let new_val = value?.max(0) as u16;
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    field.scroll_top = new_val;
                    Ok(())
                },
            ),
            "hilite" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().hilite = value?;
                    Ok(())
                },
            ),
            "foreColor" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let v = value? as u8;
                    cast_member.member_type.as_field_mut().unwrap().fore_color = Some(ColorRef::PaletteIndex(v));
                    Ok(())
                },
            ),
            "backColor" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let v = value? as u8;
                    cast_member.member_type.as_field_mut().unwrap().back_color = Some(ColorRef::PaletteIndex(v));
                    Ok(())
                },
            ),
            "media" => borrow_member_mut(
                member_ref,
                |player| value.media_value(),
                |cast_member, value| {
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    match value? {
                        Media::Field(new_field) => field.clone_from(&new_field),
                        _ => return Err(ScriptError::new("Invalid media value for field".to_string())),
                    };
                    Ok(())
                },
            ),
            "selStart" => borrow_member_mut(
                member_ref,
                |_player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().sel_start = value?;
                    Ok(())
                },
            ),
            "selEnd" => borrow_member_mut(
                member_ref,
                |_player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().sel_end = value?;
                    Ok(())
                },
            ),
            "selectedText" => borrow_member_mut(
                member_ref,
                |_player| value.string_value(),
                |cast_member, value| {
                    let s = value?;
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    let len = field.text.len() as i32;
                    let lo = field.sel_start.min(field.sel_end).clamp(0, len);
                    let hi = field.sel_start.max(field.sel_end).clamp(0, len);
                    field.text.replace_range(lo as usize..hi as usize, &s);
                    let new_caret = lo + s.len() as i32;
                    field.sel_start = new_caret;
                    field.sel_end = new_caret;
                    field.sel_anchor = new_caret;
                    Ok(())
                },
            ),
            "kerning" => borrow_member_mut(
                member_ref,
                |_player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().kerning = value?;
                    Ok(())
                },
            ),
            "kerningThreshold" => borrow_member_mut(
                member_ref,
                |_player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().kerning_threshold = value? as u16;
                    Ok(())
                },
            ),
            "useHypertextStyles" => borrow_member_mut(
                member_ref,
                |_player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().use_hypertext_styles = value?;
                    Ok(())
                },
            ),
            "antiAliasType" => borrow_member_mut(
                member_ref,
                |_player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().anti_alias_type = value?;
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
