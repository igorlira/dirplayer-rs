use itertools::Itertools;

use crate::{
    director::lingo::datum::{Datum, StringChunkExpr, StringChunkSource, StringChunkType},
    player::{
        cast_lib::CastMemberRef,
        cast_member::CastMemberType,
        handlers::datum_handlers::{
            cast_member::font::{HtmlStyle, StyledSpan},
            string::string_get_words,
        },
        reserve_player_mut,
        sprite::ColorRef,
        DatumRef, DirPlayer, ScriptError,
    },
};

use super::string::{string_get_items, string_get_lines};

pub struct StringChunkHandlers {}
pub struct StringChunkUtils {}

/// Convert a CHAR range (0-based, half-open) into a BYTE range suitable
/// for `String::replace_range` / direct slicing. Without this, Director's
/// `char N of t = "x"` / `delete char N of t` would panic on any text
/// containing multi-byte codepoints (umlauts, accents, Euro, smart quotes,
/// etc.) -- the call sites collected `chars().count()` as the max but then
/// passed the resulting indices into byte-indexed APIs.
///
/// Out-of-range indices clamp to the string's byte length so the caller's
/// "delete chars 100..200 of a 5-char string" still produces an empty
/// range instead of panicking.
pub(crate) fn char_range_to_byte_range(s: &str, char_start: usize, char_end: usize) -> (usize, usize) {
    if char_start >= char_end {
        // Caller already handles the "empty range" case for ranges that
        // collapse during clamping; return a valid empty slice at the
        // appropriate boundary so replace_range is still a no-op.
        let bs = s.char_indices().nth(char_start).map(|(b, _)| b).unwrap_or(s.len());
        return (bs, bs);
    }
    let mut iter = s.char_indices();
    let byte_start = iter.by_ref().nth(char_start).map(|(b, _)| b).unwrap_or(s.len());
    // We've consumed `char_start + 1` items from iter. To advance to the
    // `char_end`th codepoint, step forward `char_end - char_start - 1` more.
    let extra = char_end - char_start - 1;
    let byte_end = iter.nth(extra).map(|(b, _)| b).unwrap_or(s.len());
    (byte_start, byte_end)
}

impl StringChunkUtils {
    pub fn delete(
        player: &mut DirPlayer,
        original_str_src: &StringChunkSource,
        chunk_expr: &StringChunkExpr,
    ) -> Result<(), ScriptError> {
        let new_string = {
            let original_str = match original_str_src {
                StringChunkSource::Datum(original_str_ref) => {
                    player.get_datum(original_str_ref).string_value()?
                }
                StringChunkSource::Member(member_ref) => player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .unwrap()
                    .member_type
                    .as_field()
                    .unwrap()
                    .text
                    .clone(),
            };
            Self::string_by_deleting_chunk(&original_str, &chunk_expr)
        }?;
        Self::set_value(player, original_str_src, chunk_expr, new_string)?;
        Ok(())
    }

    pub fn set_contents(
        player: &mut DirPlayer,
        original_str_src: &StringChunkSource,
        chunk_expr: &StringChunkExpr,
        new_string: String,
    ) -> Result<(), ScriptError> {
        let new_string = {
            let original_str = match original_str_src {
                StringChunkSource::Datum(original_str_ref) => {
                    player.get_datum(original_str_ref).string_value()?
                }
                StringChunkSource::Member(member_ref) => player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .unwrap()
                    .member_type
                    .as_field()
                    .unwrap()
                    .text
                    .clone(),
            };
            Self::string_by_setting_chunk(&original_str, &chunk_expr, &new_string)
        }?;
        Self::set_value(player, original_str_src, chunk_expr, new_string)?;
        Ok(())
    }

    pub fn set_value(
        player: &mut DirPlayer,
        original_str_src: &StringChunkSource,
        chunk_expr: &StringChunkExpr,
        new_string: String,
    ) -> Result<(), ScriptError> {
        match original_str_src {
            StringChunkSource::Datum(original_str_ref) => {
                // The datum might be a StringChunk (e.g. when trim() receives t.line[ln] as argument).
                // In Director, strings are value types, so mutating a StringChunk-backed variable
                // should "materialize" it into a plain String. Replace the datum entirely.
                let datum = player.get_datum_mut(original_str_ref);
                match datum {
                    Datum::String(s) => {
                        *s = new_string;
                    }
                    _ => {
                        *datum = Datum::String(new_string);
                    }
                }
            }
            StringChunkSource::Member(member_ref) => {
                let member = &mut player
                    .movie
                    .cast_manager
                    .find_mut_member_by_ref(&member_ref)
                    .unwrap()
                    .member_type;
                match member {
                    CastMemberType::Field(field) => field.set_text_preserving_caret(new_string),
                    CastMemberType::Text(member) => member.set_text_preserving_caret(new_string),
                    _ => {
                        return Err(ScriptError::new(
                            "Cannot set contents for non-text member".to_string(),
                        ))
                    }
                }
            }
        }
        Ok(())
    }

    pub fn string_by_deleting_chunk(
        string: &str,
        chunk_expr: &StringChunkExpr,
    ) -> Result<String, ScriptError> {
        match chunk_expr.chunk_type {
            StringChunkType::Char => {
                let mut new_string = string.to_owned();
                let (start, end) =
                    Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), string.chars().count());
                // (start, end) are CHAR indices; replace_range wants BYTE
                // indices. Convert via char_indices so `delete char 3 of
                // "öäüß"` doesn't try to slice mid-codepoint.
                let (byte_start, byte_end) = char_range_to_byte_range(string, start, end);
                new_string.replace_range(byte_start..byte_end, "");
                Ok(new_string)
            }
            StringChunkType::Item => {
                let chunk_list = 
                    Self::resolve_chunk_list(string, chunk_expr.chunk_type.clone(), chunk_expr.item_delimiter)?;
                let (start, end) = 
                    Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), chunk_list.len());

                if chunk_list.len() == 0 {
                    return Ok(string.to_owned());
                }

                let mut new_chunks = chunk_list;
                new_chunks.drain(start..end);
                let delimiter = chunk_expr.item_delimiter.to_string();
                Ok(new_chunks.join(&delimiter))
            },
            StringChunkType::Word => {
                let chunk_list = 
                    Self::resolve_chunk_list(string, chunk_expr.chunk_type.clone(), chunk_expr.item_delimiter)?;
                let (start, end) = 
                    Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), chunk_list.len());

                if chunk_list.len() == 0 {
                    return Ok(string.to_owned());
                }

                let mut new_chunks = chunk_list;
                new_chunks.drain(start..end);
                Ok(new_chunks.join(" "))
            },
            StringChunkType::Line => {
                let chunk_list = 
                    Self::resolve_chunk_list(string, chunk_expr.chunk_type.clone(), chunk_expr.item_delimiter)?;
                let (start, end) = 
                    Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), chunk_list.len());

                if chunk_list.len() == 0 {
                    return Ok(string.to_owned());
                }

                let mut new_chunks = chunk_list;
                new_chunks.drain(start..end);
                Ok(new_chunks.join("\r\n"))
            },
        }
    }

    pub fn string_by_setting_chunk(
        string: &str,
        chunk_expr: &StringChunkExpr,
        replace_with: &str,
    ) -> Result<String, ScriptError> {
        match chunk_expr.chunk_type {
            StringChunkType::Char => {
                let mut new_string = string.to_owned();
                let (start, end) =
                    Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), string.chars().count());
                let (byte_start, byte_end) = char_range_to_byte_range(string, start, end);
                new_string.replace_range(byte_start..byte_end, replace_with);
                Ok(new_string)
            }
            _ => Err(ScriptError::new(
                "Only char chunk type is supported for string by setting chunk".to_string(),
            )),
        }
    }

    pub fn vm_range_to_host(range: (i32, i32), max_length: usize) -> (usize, usize) {
        let (start, end) = range;
        // Director's compiler uses -30000 as a sentinel for "the last element"
        // (e.g. `delete the last char of t` compiles to `delete char -30000 of t`)
        if start <= -30000 && max_length > 0 {
            let last = max_length - 1;
            return (last, max_length);
        }
        let start_index = std::cmp::max(0, start - 1) as usize;
        let end_index = if end == 0 {
            if start <= 0 {
                start_index
            } else {
                (start_index + 1) as usize
            }
        } else if end == -1 || end > max_length as i32 {
            max_length as usize
        } else {
            end as usize
        };
        let start_index = std::cmp::min(std::cmp::max(start_index, 0), max_length);
        let end_index = std::cmp::max(start_index, std::cmp::min(end_index, max_length));
        (start_index, end_index)
    }

    #[allow(dead_code)]
    fn host_range_to_vm(range: (i32, i32)) -> (i32, i32) {
        let (start, end) = range;
        (start + 1, end)
    }

    pub fn resolve_chunk_list(
        string: &str,
        chunk_type: StringChunkType,
        item_delimiter: char,
    ) -> Result<Vec<String>, ScriptError> {
        match chunk_type {
            StringChunkType::Item => Ok(string_get_items(string, item_delimiter)),
            StringChunkType::Word => {
                let words = string_get_words(string);
                Ok(words)
            }
            StringChunkType::Char => {
                let chars = string.chars().map(|c| c.to_string());
                Ok(chars.collect_vec())
            }
            StringChunkType::Line => {
                let lines = string_get_lines(string);
                Ok(lines)
            }
        }
    }

    pub fn resolve_last_chunk(
        string: &str,
        chunk_type: StringChunkType,
        item_delimiter: char,
    ) -> Result<String, ScriptError> {
        match chunk_type {
            StringChunkType::Item => {
                let items = string.split(item_delimiter).map(|x| x.to_string());
                Ok(items.last().unwrap_or("".to_string()).to_string())
            }
            StringChunkType::Word => {
                let words = string_get_words(string);
                Ok(words.last().unwrap_or(&"".to_string()).to_string())
            }
            StringChunkType::Char => Ok(string
                .chars()
                .last()
                .map(|x| x.to_string())
                .unwrap_or("".to_string())),
            StringChunkType::Line => {
                let lines = string_get_lines(string);
                Ok(lines.last().unwrap_or(&"".to_string()).to_string())
            }
        }
    }

    pub fn resolve_chunk_count(
        string: &str,
        chunk_type: StringChunkType,
        item_delimiter: char,
    ) -> Result<usize, ScriptError> {
        match chunk_type {
            StringChunkType::Item => {
                Ok(string.chars().filter(|c| item_delimiter == *c).count() + 1)
            }
            StringChunkType::Word => Ok(string_get_words(string).len()),
            StringChunkType::Char => Ok(string.chars().count()),
            StringChunkType::Line => Ok(string_get_lines(string).len()),
        }
    }

    pub fn resolve_chunk_expr_string(
        string: &str,
        chunk_expr: &StringChunkExpr,
    ) -> Result<String, ScriptError> {
        // let type_str: String = chunk_expr.chunk_type.to_owned().into();

        //warn!("-============ resolve_chunk_expr_string =============-");
        //warn!("input string: {}", string);
        //warn!("type: {}", type_str);
        //warn!("vm range ({}, {})", chunk_expr.start, chunk_expr.end);
        //warn!("host range ({}, {})", start, end);
        //warn!("delimiter: {} (len {})", chunk_expr.item_delimiter, chunk_expr.item_delimiter.len());
        //warn!("chunk list: {:?}", chunk_list);

        if string.len() == 0 {
            return Ok("".to_string());
        }

        let result = match chunk_expr.chunk_type {
            StringChunkType::Item => {
                let chunk_list = Self::resolve_chunk_list(
                    string,
                    chunk_expr.chunk_type.clone(),
                    chunk_expr.item_delimiter,
                )?;
                let (start, end) =
                    Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), chunk_list.len());

                if chunk_list.len() == 0 {
                    return Ok("".to_string());
                }
                let delimiter = chunk_expr.item_delimiter.to_string();
                chunk_list[start..end].join(&delimiter)
            }
            StringChunkType::Word => {
                let chunk_list = Self::resolve_chunk_list(
                    string,
                    chunk_expr.chunk_type.clone(),
                    chunk_expr.item_delimiter,
                )?;
                let (start, end) =
                    Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), chunk_list.len());

                if chunk_list.len() == 0 {
                    return Ok("".to_string());
                }
                chunk_list[start..end].join(" ")
            }
            StringChunkType::Char => {
                let (start, end) =
                    Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), string.chars().count());
                let chars = string.chars().skip(start).take(end - start).collect();
                chars
            }
            StringChunkType::Line => {
                let chunk_list = Self::resolve_chunk_list(
                    string,
                    chunk_expr.chunk_type.clone(),
                    chunk_expr.item_delimiter,
                )?;
                let (start, end) =
                    Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), chunk_list.len());

                if chunk_list.len() == 0 {
                    return Ok("".to_string());
                }
                chunk_list[start..end].join("\r\n")
            }
        };

        //warn!("result: {}", result);
        //warn!("-============  =============-");

        Ok(result)
    }

    pub fn string_by_putting_into_chunk(
        string: &str,
        chunk_expr: &StringChunkExpr,
        replace_with: &str,
    ) -> Result<String, ScriptError> {
        // Similar to string_by_setting_chunk but supports all chunk types
        match chunk_expr.chunk_type {
            StringChunkType::Char => {
                let mut new_string = string.to_owned();
                let (start, end) =
                    StringChunkUtils::vm_range_to_host((chunk_expr.start, chunk_expr.end), string.chars().count());
                let (byte_start, byte_end) = char_range_to_byte_range(string, start, end);
                new_string.replace_range(byte_start..byte_end, replace_with);
                Ok(new_string)
            }
            StringChunkType::Item | StringChunkType::Word | StringChunkType::Line => {
                let chunk_list = StringChunkUtils::resolve_chunk_list(
                    string,
                    chunk_expr.chunk_type.clone(),
                    chunk_expr.item_delimiter,
                )?;
                let (start, end) = StringChunkUtils::vm_range_to_host(
                    (chunk_expr.start, chunk_expr.end),
                    chunk_list.len(),
                );

                if chunk_list.is_empty() {
                    return Ok(string.to_owned());
                }

                let mut new_chunks = chunk_list;
                new_chunks.splice(start..end, [replace_with.to_owned()]);

                let delimiter = match chunk_expr.chunk_type {
                    StringChunkType::Item => chunk_expr.item_delimiter.to_string(),
                    StringChunkType::Word => " ".to_string(),
                    StringChunkType::Line => "\r\n".to_string(),
                    _ => unreachable!(),
                };
                Ok(new_chunks.join(&delimiter))
            }
        }
    }

    pub fn string_by_putting_before_chunk(
        string: &str,
        chunk_expr: &StringChunkExpr,
        insert_value: &str,
    ) -> Result<String, ScriptError> {
        match chunk_expr.chunk_type {
            StringChunkType::Char => {
                let mut new_string = string.to_owned();
                let (start, _) =
                    StringChunkUtils::vm_range_to_host((chunk_expr.start, chunk_expr.end), string.chars().count());
                new_string.insert_str(start, insert_value);
                Ok(new_string)
            }
            StringChunkType::Item | StringChunkType::Word | StringChunkType::Line => {
                let chunk_list = StringChunkUtils::resolve_chunk_list(
                    string,
                    chunk_expr.chunk_type.clone(),
                    chunk_expr.item_delimiter,
                )?;
                let (start, _) = StringChunkUtils::vm_range_to_host(
                    (chunk_expr.start, chunk_expr.end),
                    chunk_list.len(),
                );

                if chunk_list.is_empty() {
                    return Ok(insert_value.to_owned());
                }

                let mut new_chunks = chunk_list;
                
                // For "put before", prepend to the chunk at the start position
                if start < new_chunks.len() {
                    new_chunks[start] = format!("{}{}", insert_value, new_chunks[start]);
                } else {
                    // Fallback: append at the end
                    if let Some(last) = new_chunks.last_mut() {
                        last.push_str(insert_value);
                    }
                }

                let delimiter = match chunk_expr.chunk_type {
                    StringChunkType::Item => chunk_expr.item_delimiter.to_string(),
                    StringChunkType::Word => " ".to_string(),
                    StringChunkType::Line => "\r\n".to_string(),
                    _ => unreachable!(),
                };
                Ok(new_chunks.join(&delimiter))
            }
        }
    }

    pub fn string_by_putting_after_chunk(
        string: &str,
        chunk_expr: &StringChunkExpr,
        insert_value: &str,
    ) -> Result<String, ScriptError> {
        match chunk_expr.chunk_type {
            StringChunkType::Char => {
                let mut new_string = string.to_owned();
                let (_, end) =
                    StringChunkUtils::vm_range_to_host((chunk_expr.start, chunk_expr.end), string.chars().count());
                new_string.insert_str(end, insert_value);
                Ok(new_string)
            }
            StringChunkType::Item | StringChunkType::Word | StringChunkType::Line => {
                let chunk_list = StringChunkUtils::resolve_chunk_list(
                    string,
                    chunk_expr.chunk_type.clone(),
                    chunk_expr.item_delimiter,
                )?;
                let (start, end) = StringChunkUtils::vm_range_to_host(
                    (chunk_expr.start, chunk_expr.end),
                    chunk_list.len(),
                );

                if chunk_list.is_empty() {
                    return Ok(insert_value.to_owned());
                }

                let mut new_chunks = chunk_list;
                
                // For "put after", append to the last chunk in the range
                // rather than inserting as a new chunk
                if end > 0 && end <= new_chunks.len() {
                    new_chunks[end - 1].push_str(insert_value);
                } else {
                    // Fallback: append at the end
                    if let Some(last) = new_chunks.last_mut() {
                        last.push_str(insert_value);
                    }
                }

                let delimiter = match chunk_expr.chunk_type {
                    StringChunkType::Item => chunk_expr.item_delimiter.to_string(),
                    StringChunkType::Word => " ".to_string(),
                    StringChunkType::Line => "\r\n".to_string(),
                    _ => unreachable!(),
                };
                Ok(new_chunks.join(&delimiter))
            }
        }
    }
}

impl StringChunkHandlers {
    pub fn count(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let value = player.get_datum(datum).string_value()?;
            let operand = player.get_datum(&args[0]).string_value()?;
            let delimiter = player.movie.item_delimiter;
            let count = StringChunkUtils::resolve_chunk_count(
                &value,
                StringChunkType::from(&operand),
                delimiter,
            )?;
            Ok(player.alloc_datum(Datum::Int(count as i32)))
        })
    }

    pub fn get_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Self::get_prop_inner(datum, args, false)
    }

    pub fn get_prop_ref(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Self::get_prop_inner(datum, args, true)
    }

    fn get_prop_inner(datum: &DatumRef, args: &Vec<DatumRef>, as_ref: bool) -> Result<DatumRef, ScriptError> {
        let datum = datum.clone();
        reserve_player_mut(|player| {
            let datum_val = player.get_datum(&datum);
            let parent_str = datum_val.string_value()?;
            let prop_name = player.get_datum(&args[0]).string_value()?;
            let start = player.get_datum(&args[1]).int_value()?;
            let end = if args.len() > 2 {
                player.get_datum(&args[2]).int_value()?
            } else {
                start
            };
            let chunk_expr = StringChunkExpr {
                chunk_type: StringChunkType::from(&prop_name),
                start,
                end,
                item_delimiter: player.movie.item_delimiter,
            };

            let str_value =
                StringChunkUtils::resolve_chunk_expr_string(&parent_str, &chunk_expr)?;
            if as_ref {
                // Nested chunks must chain via `StringChunkSource::Datum` so the
                // outer operation (e.g. `.line[n]`) is preserved. Using the outer
                // chunk's own source flattens the chain, losing context —
                // `listmember.line[35].item[1]` would then resolve to item 1 of
                // the *full* text, not of line 35. Coke Studios' per-line colour
                // on the roomlist requires the chain to be preserved so each
                // line's item picks up the right character range.
                Ok(player.alloc_datum(Datum::StringChunk(
                    StringChunkSource::Datum(datum.clone()),
                    chunk_expr,
                    str_value,
                )))
            } else {
                Ok(player.alloc_datum(Datum::String(str_value)))
            }
        })
    }

    /// Resolve the character range (start inclusive, end exclusive) that
    /// `chunk_expr` selects from `text`. Used by styled-span chunk setters.
    pub fn resolve_chunk_char_range(
        text: &str,
        chunk_expr: &StringChunkExpr,
    ) -> (usize, usize) {
        let total_chars = text.chars().count();
        match chunk_expr.chunk_type {
            StringChunkType::Char => {
                StringChunkUtils::vm_range_to_host((chunk_expr.start, chunk_expr.end), total_chars)
            }
            StringChunkType::Item => {
                resolve_delimited_char_range_single(
                    text,
                    chunk_expr.item_delimiter,
                    chunk_expr.start,
                    chunk_expr.end,
                )
            }
            StringChunkType::Word => resolve_word_char_range(text, chunk_expr.start, chunk_expr.end),
            StringChunkType::Line => resolve_line_char_range(text, chunk_expr.start, chunk_expr.end),
        }
    }

    /// Walk the nested-chunk chain from `datum_ref` back to the originating
    /// `Member`, accumulating the cumulative character range so styled-span
    /// setters know which slice of the member's text to retouch.
    ///
    /// Returns `(member_ref, char_start, char_end)` where the range is in the
    /// member's full text and uses character indices (not bytes).
    pub fn walk_chunk_to_member_range(
        player: &DirPlayer,
        datum_ref: &DatumRef,
    ) -> Option<(CastMemberRef, usize, usize)> {
        // Collect the chunk chain outermost-last by walking the source chain
        // inward: the datum itself holds the innermost expr, its source holds
        // the next, and so on until we hit a Member.
        let mut chain: Vec<StringChunkExpr> = Vec::new();
        let mut current_ref = datum_ref.clone();
        let member_ref;
        loop {
            let datum = player.get_datum(&current_ref);
            match datum {
                Datum::StringChunk(source, expr, _) => {
                    chain.push(expr.clone());
                    match source {
                        StringChunkSource::Member(mref) => {
                            member_ref = mref.clone();
                            break;
                        }
                        StringChunkSource::Datum(parent_ref) => {
                            current_ref = parent_ref.clone();
                        }
                    }
                }
                _ => return None,
            }
        }

        let member = player.movie.cast_manager.find_member_by_ref(&member_ref)?;
        let text = match &member.member_type {
            CastMemberType::Text(t) => t.text.clone(),
            CastMemberType::Field(f) => f.text.clone(),
            _ => return None,
        };

        // Apply chunks outermost → innermost to narrow the range inside the
        // full member text.
        let mut range_start = 0usize;
        let mut range_end = text.chars().count();
        for expr in chain.into_iter().rev() {
            let slice: String = text
                .chars()
                .skip(range_start)
                .take(range_end - range_start)
                .collect();
            let (s, e) = Self::resolve_chunk_char_range(&slice, &expr);
            range_end = range_start + e;
            range_start += s;
        }
        Some((member_ref, range_start, range_end))
    }

    /// Split `html_styled_spans` at the boundaries [start, end) and apply
    /// `modifier` to the style of the segment inside. Existing spans outside
    /// the range are preserved; spans that straddle a boundary are split so
    /// the boundary pixels don't inherit the new style.
    pub fn apply_styled_span_range<F: FnMut(&mut HtmlStyle)>(
        full_text: &str,
        spans: &mut Vec<StyledSpan>,
        start: usize,
        end: usize,
        default_style: HtmlStyle,
        mut modifier: F,
    ) {
        if start >= end {
            return;
        }
        // Seed with a single span covering the full text when nothing is set
        // yet, otherwise we'd have nowhere to split.
        if spans.is_empty() {
            spans.push(StyledSpan {
                text: full_text.to_string(),
                style: default_style,
            });
        }

        let mut new_spans: Vec<StyledSpan> = Vec::new();
        let mut char_pos = 0usize;
        for span in spans.drain(..) {
            let span_len = span.text.chars().count();
            if span_len == 0 {
                new_spans.push(span);
                continue;
            }
            let span_end = char_pos + span_len;

            if span_end <= start || char_pos >= end {
                new_spans.push(span);
                char_pos = span_end;
                continue;
            }

            // Partition this span into up-to-three pieces.
            let local_start = start.saturating_sub(char_pos).min(span_len);
            let local_end = end.saturating_sub(char_pos).min(span_len);
            let chars: Vec<char> = span.text.chars().collect();

            if local_start > 0 {
                let before: String = chars[..local_start].iter().collect();
                new_spans.push(StyledSpan {
                    text: before,
                    style: span.style.clone(),
                });
            }
            if local_end > local_start {
                let inside: String = chars[local_start..local_end].iter().collect();
                let mut new_style = span.style.clone();
                modifier(&mut new_style);
                new_spans.push(StyledSpan {
                    text: inside,
                    style: new_style,
                });
            }
            if local_end < span_len {
                let after: String = chars[local_end..].iter().collect();
                new_spans.push(StyledSpan {
                    text: after,
                    style: span.style.clone(),
                });
            }

            char_pos = span_end;
        }
        *spans = new_spans;
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum_ref: &DatumRef,
        prop: &str,
        value_ref: &DatumRef,
    ) -> Result<(), ScriptError> {
        match prop {
            "font" | "fontStyle" | "color" => {
                return Self::set_chunk_style_prop(player, datum_ref, prop, value_ref);
            }
            "fontSize" => {
                let new_val = player.get_datum(value_ref).int_value()?;
                let datum = player.get_datum(datum_ref).clone();
                if let Datum::StringChunk(source, _, _) = datum {
                    let mut current_source = source;
                    loop {
                        match current_source {
                            StringChunkSource::Member(member_ref) => {
                                if let Some(member) = player.movie.cast_manager.find_member_by_ref_mut(&member_ref) {
                                    if let CastMemberType::Text(ref mut text) = member.member_type {
                                        text.font_size = new_val as u16;
                                        for span in text.html_styled_spans.iter_mut() {
                                            span.style.font_size = Some(new_val);
                                        }
                                    } else if let CastMemberType::Field(ref mut field) = member.member_type {
                                        field.font_size = new_val as u16;
                                    }
                                }
                                break;
                            }
                            StringChunkSource::Datum(ref source_datum_ref) => {
                                let source_datum = player.get_datum(source_datum_ref).clone();
                                if let Datum::StringChunk(inner_source, _, _) = source_datum {
                                    current_source = inner_source;
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            "charSpacing" => {
                // Update the source member's char_spacing
                // Walk the source chain to find the originating member
                let new_val = player.get_datum(value_ref).int_value()?;
                let datum = player.get_datum(datum_ref).clone();
                if let Datum::StringChunk(source, _, _) = datum {
                    let mut current_source = source;
                    loop {
                        match current_source {
                            StringChunkSource::Member(member_ref) => {
                                if let Some(member) = player.movie.cast_manager.find_member_by_ref_mut(&member_ref) {
                                    if let CastMemberType::Text(ref mut text) = member.member_type {
                                        text.char_spacing = new_val;
                                        for span in text.html_styled_spans.iter_mut() {
                                            span.style.char_spacing = new_val;
                                        }
                                    }
                                }
                                break;
                            }
                            StringChunkSource::Datum(ref source_datum_ref) => {
                                let source_datum = player.get_datum(source_datum_ref).clone();
                                if let Datum::StringChunk(inner_source, _, _) = source_datum {
                                    current_source = inner_source;
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                return Err(ScriptError::new(format!(
                    "Cannot set property {prop} for string chunk datum"
                )))
            }
        }
        Ok(())
    }

    /// Apply a font / fontStyle / color change to a nested string-chunk datum
    /// by splitting the source member's `html_styled_spans` at the chunk
    /// boundaries. Used by Coke Studios for per-line colour + bold/underline
    /// on the roomlist audition rows; without it the whole member's style
    /// would change (or worse, nothing would happen).
    fn set_chunk_style_prop(
        player: &mut DirPlayer,
        datum_ref: &DatumRef,
        prop: &str,
        value_ref: &DatumRef,
    ) -> Result<(), ScriptError> {
        // Resolve the chunk range up-front (read-only borrows) before taking
        // the mutable borrow on the target member.
        let resolved = Self::walk_chunk_to_member_range(player, datum_ref);
        let Some((member_ref, start, end)) = resolved else { return Ok(()); };
        if start >= end {
            return Ok(());
        }

        // Build the style modifier from the incoming value.
        enum StyleChange {
            Font(String),
            FontStyle { bold: bool, italic: bool, underline: bool },
            Color(u32),
        }
        let value_datum = player.get_datum(value_ref).clone();
        let change = match prop {
            "font" => StyleChange::Font(value_datum.string_value()?),
            "fontStyle" => {
                // Director accepts either a single symbol (#bold) or a list
                // of symbols ([#bold, #underline]). #plain resets the style.
                let mut bold = false;
                let mut italic = false;
                let mut underline = false;
                let symbols: Vec<String> = match &value_datum {
                    Datum::Symbol(s) => vec![s.clone()],
                    Datum::List(_, items, _) => {
                        let mut out = Vec::new();
                        for item_ref in items.iter() {
                            if let Datum::Symbol(s) = player.get_datum(item_ref) {
                                out.push(s.clone());
                            }
                        }
                        out
                    }
                    _ => Vec::new(),
                };
                for s in symbols.iter() {
                    match s.to_ascii_lowercase().as_str() {
                        "bold" => bold = true,
                        "italic" => italic = true,
                        "underline" => underline = true,
                        "plain" => {
                            bold = false;
                            italic = false;
                            underline = false;
                        }
                        _ => {}
                    }
                }
                StyleChange::FontStyle { bold, italic, underline }
            }
            "color" => {
                let color_ref = value_datum.to_color_ref()?.to_owned();
                let rgb = match color_ref {
                    ColorRef::Rgb(r, g, b) => {
                        ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
                    }
                    ColorRef::PaletteIndex(_) => {
                        // Resolve palette index to RGB using the system palette
                        // so styled-span color (which is a plain u32 RGB) ends
                        // up matching what the renderer would draw.
                        let palettes = player.movie.cast_manager.palettes();
                        let bitmap_palette = crate::player::bitmap::bitmap::PaletteRef::BuiltIn(
                            crate::player::bitmap::bitmap::get_system_default_palette(),
                        );
                        let (r, g, b) = crate::player::bitmap::bitmap::resolve_color_ref(
                            &palettes, &color_ref, &bitmap_palette, 8,
                        );
                        ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
                    }
                };
                StyleChange::Color(rgb)
            }
            _ => return Ok(()),
        };

        // Now mutate the target member.
        let Some(member) = player.movie.cast_manager.find_member_by_ref_mut(&member_ref) else {
            return Ok(());
        };
        let mem_name = member.name.clone();
        match &mut member.member_type {
            CastMemberType::Text(text) => {
                let full_text = text.text.clone();
                let default_style = HtmlStyle {
                    font_face: if text.font.is_empty() { None } else { Some(text.font.clone()) },
                    font_size: if text.font_size > 0 { Some(text.font_size as i32) } else { None },
                    bold: text.font_style.iter().any(|s| s == "bold"),
                    italic: text.font_style.iter().any(|s| s == "italic"),
                    underline: text.font_style.iter().any(|s| s == "underline"),
                    ..HtmlStyle::default()
                };
                Self::apply_styled_span_range(
                    &full_text,
                    &mut text.html_styled_spans,
                    start,
                    end,
                    default_style,
                    |style| match &change {
                        StyleChange::Font(f) => style.font_face = Some(f.clone()),
                        StyleChange::FontStyle { bold, italic, underline } => {
                            style.bold = *bold;
                            style.italic = *italic;
                            style.underline = *underline;
                        }
                        StyleChange::Color(rgb) => style.color = Some(*rgb),
                    },
                );
                let _ = mem_name;
            }
            CastMemberType::Field(field) => {
                // FieldMember has no per-char style storage (just one string
                // of font_style for the whole member), so a chunk-level font/
                // fontStyle write on a field applies to the entire field. This
                // matches Director's own fallback for fields that don't track
                // styled runs.
                match &change {
                    StyleChange::Font(f) => field.font = f.clone(),
                    StyleChange::FontStyle { bold, italic, underline } => {
                        let mut parts: Vec<&str> = Vec::new();
                        if *bold { parts.push("bold"); }
                        if *italic { parts.push("italic"); }
                        if *underline { parts.push("underline"); }
                        field.font_style = if parts.is_empty() { "plain".to_string() } else { parts.join(",") };
                    }
                    StyleChange::Color(rgb) => {
                        let r = ((*rgb >> 16) & 0xFF) as u8;
                        let g = ((*rgb >> 8) & 0xFF) as u8;
                        let b = (*rgb & 0xFF) as u8;
                        field.fore_color = Some(ColorRef::Rgb(r, g, b));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn delete(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (original_str_ref, chunk_expr, ..) = player.get_datum(datum).to_string_chunk()?;
            StringChunkUtils::delete(player, &original_str_ref.clone(), &chunk_expr.clone())?;
            Ok(DatumRef::Void)
        })
    }

    fn set_contents(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (original_str_ref, chunk_expr, ..) = player.get_datum(datum).to_string_chunk()?;
            let new_str = player.get_datum(&args[0]).string_value()?;
            StringChunkUtils::set_contents(
                player,
                &original_str_ref.clone(),
                &chunk_expr.clone(),
                new_str,
            )?;
            Ok(DatumRef::Void)
        })
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "count" => Self::count(datum, args),
            "getProp" => Self::get_prop(datum, args),
            "getPropRef" => Self::get_prop_ref(datum, args),
            "delete" => Self::delete(datum, args),
            "setContents" => Self::set_contents(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for string chunk datum"
            ))),
        }
    }
}

/// Convert a 1-based inclusive VM index range into the character-index range
/// `[start, end)` of the Nth..Mth segment in `text`, where segments are
/// delimited by a single `delim` character (used for item chunks). If the
/// requested index is out of bounds an empty range at the text's end is
/// returned so callers can safely pass it to `apply_styled_span_range` without
/// corrupting unrelated text.
fn resolve_delimited_char_range_single(
    text: &str,
    delim: char,
    start: i32,
    end: i32,
) -> (usize, usize) {
    // Build segment starts/ends as char indices by walking the text once.
    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut seg_start = 0usize;
    let mut idx = 0usize;
    for ch in text.chars() {
        if ch == delim {
            segments.push((seg_start, idx));
            seg_start = idx + 1;
        }
        idx += 1;
    }
    segments.push((seg_start, idx));
    let total_chars = idx;
    if segments.is_empty() {
        return (total_chars, total_chars);
    }
    let (s, e) = StringChunkUtils::vm_range_to_host((start, end), segments.len());
    if s >= segments.len() {
        return (total_chars, total_chars);
    }
    let range_start = segments[s].0;
    let range_end = segments[e.saturating_sub(1).min(segments.len() - 1)].1;
    if e <= s {
        (range_start, range_start)
    } else {
        (range_start, range_end)
    }
}

/// Line-chunk char range. Mirrors `string_get_lines`: detects a single line
/// break style per text (\r\n, \n, or \r). Returns the char range that would
/// be selected by `line[start..end]` in Director semantics.
fn resolve_line_char_range(text: &str, start: i32, end: i32) -> (usize, usize) {
    let total_chars = text.chars().count();
    if text.is_empty() {
        return (0, 0);
    }
    let contains_crlf = text.contains("\r\n");
    let contains_lf = !contains_crlf && text.contains('\n');
    // contains_cr true when neither of the above match
    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut seg_start = 0usize;
    let mut idx = 0usize;
    let mut prev_was_cr = false;
    for ch in text.chars() {
        let is_break = if contains_crlf {
            // Treat \r\n as a single break: end the line on \r, then skip \n.
            if prev_was_cr && ch == '\n' {
                seg_start = idx + 1;
                prev_was_cr = false;
                idx += 1;
                continue;
            }
            prev_was_cr = ch == '\r';
            ch == '\r'
        } else if contains_lf {
            ch == '\n'
        } else {
            ch == '\r'
        };
        if is_break {
            segments.push((seg_start, idx));
            seg_start = idx + 1;
        }
        idx += 1;
    }
    segments.push((seg_start, idx));
    if segments.is_empty() {
        return (total_chars, total_chars);
    }
    let (s, e) = StringChunkUtils::vm_range_to_host((start, end), segments.len());
    if s >= segments.len() {
        return (total_chars, total_chars);
    }
    let range_start = segments[s].0;
    let range_end = segments[e.saturating_sub(1).min(segments.len() - 1)].1;
    if e <= s {
        (range_start, range_start)
    } else {
        (range_start, range_end)
    }
}

/// Word-chunk char range. Words are runs of non-whitespace (matches
/// `string_get_words` which splits on Director whitespace).
fn resolve_word_char_range(text: &str, start: i32, end: i32) -> (usize, usize) {
    let is_ws = |c: char| c.is_ascii_control() || c.is_ascii_whitespace();
    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut seg_start: Option<usize> = None;
    let mut idx = 0usize;
    for ch in text.chars() {
        if is_ws(ch) {
            if let Some(s) = seg_start.take() {
                segments.push((s, idx));
            }
        } else if seg_start.is_none() {
            seg_start = Some(idx);
        }
        idx += 1;
    }
    if let Some(s) = seg_start {
        segments.push((s, idx));
    }
    let total_chars = idx;
    if segments.is_empty() {
        return (total_chars, total_chars);
    }
    let (s, e) = StringChunkUtils::vm_range_to_host((start, end), segments.len());
    if s >= segments.len() {
        return (total_chars, total_chars);
    }
    let range_start = segments[s].0;
    let range_end = segments[e.saturating_sub(1).min(segments.len() - 1)].1;
    if e <= s {
        (range_start, range_start)
    } else {
        (range_start, range_end)
    }
}
