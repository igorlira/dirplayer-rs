use std::cmp::max;

use itertools::Itertools;
use wasm_bindgen::JsValue;

use crate::{
    director::{
        chunks::score::{FrameLabel, ScoreFrameChannelData},
        file::DirectorFile,
        lingo::datum::{datum_bool, Datum, DatumType},
    },
    js_api::JsApi,
    utils::log_i,
};

use super::{
    allocator::ScriptInstanceAllocatorTrait,
    cast_lib::{cast_member_ref, CastMemberRef, NULL_CAST_MEMBER_REF},
    cast_member::CastMemberType,
    datum_ref::DatumRef,
    events::{player_dispatch_event_to_sprite, player_dispatch_targeted_event},
    geometry::{IntRect, IntRectTuple},
    handlers::datum_handlers::{
        cast_member_ref::CastMemberRefHandlers,
        color::ColorDatumHandlers,
        script::{self, ScriptDatumHandlers},
    },
    movie::Movie,
    reserve_player_mut,
    script::{script_get_prop_opt, script_set_prop},
    script_ref::ScriptInstanceRef,
    sprite::{ColorRef, CursorRef, Sprite},
    DirPlayer, ScriptError,
};

pub enum ScoreRef {
    Stage,
    FilmLoop(CastMemberRef),
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct SpriteChannel {
    pub number: usize,
    pub name: String,
    pub scripted: bool,
    pub sprite: Sprite,
}

impl SpriteChannel {
    pub fn new(number: usize) -> SpriteChannel {
        SpriteChannel {
            number,
            name: "".to_owned(),
            scripted: false,
            sprite: Sprite::new(number),
        }
    }
}

#[derive(Clone)]
pub struct ScoreBehaviorReference {
    pub cast_lib: u16,
    pub cast_member: u16,
    pub parameter: Vec<DatumRef>,
}

#[derive(Clone)]
pub struct ScoreSpriteSpan {
    pub channel_number: u32,
    pub start_frame: u32,
    pub end_frame: u32,
    pub scripts: Vec<ScoreBehaviorReference>,
}

#[derive(Clone)]
pub struct Score {
    pub channels: Vec<SpriteChannel>,
    pub sprite_spans: Vec<ScoreSpriteSpan>,
    pub channel_initialization_data: Vec<(u32, u16, ScoreFrameChannelData)>,
    pub frame_labels: Vec<FrameLabel>,
}

fn get_sprite_rect(player: &DirPlayer, sprite_id: i16) -> IntRectTuple {
    let sprite = player.movie.score.get_sprite(sprite_id);
    let sprite = match sprite {
        Some(sprite) => sprite,
        None => return (0, 0, 0, 0),
    };
    let rect = get_concrete_sprite_rect(player, sprite);
    return (rect.left, rect.top, rect.right, rect.bottom);
}

pub fn get_channel_number_from_index(index: u32) -> u32 {
    match index {
        0 => 0,
        index => index - 5,
    }
}

impl Score {
    pub fn empty() -> Score {
        Score {
            channels: vec![],
            frame_labels: vec![],
            channel_initialization_data: vec![],
            sprite_spans: vec![],
        }
    }

    pub fn get_script_in_frame(&self, frame: u32) -> Option<ScoreBehaviorReference> {
        return self
            .sprite_spans
            .iter()
            .find(|span| {
                span.channel_number == 0 && frame >= span.start_frame && frame <= span.end_frame
            })
            .and_then(|span| span.scripts.first().cloned());
    }

    fn create_behavior(cast_lib: i32, cast_member: i32) -> (ScriptInstanceRef, DatumRef) {
        let script_ref = CastMemberRef {
            cast_lib,
            cast_member,
        };
        reserve_player_mut(|player| {
            let _ = player
                .movie
                .cast_manager
                .get_script_by_ref(&script_ref)
                .ok_or(ScriptError::new(format!("Script not found")));
        });
        let (script_instance_ref, datum_ref) =
            ScriptDatumHandlers::create_script_instance(&script_ref);
        (script_instance_ref.clone(), datum_ref.clone())
    }

    fn is_span_in_frame(span: &ScoreSpriteSpan, frame_num: u32) -> bool {
        span.start_frame <= frame_num && span.end_frame >= frame_num
    }

    pub fn begin_sprites(&mut self, frame_num: u32) {
        // clean up behaviors from previous frame
        let sprites_to_finish = reserve_player_mut(|player| {
            player
                .movie
                .score
                .channels
                .iter()
                .filter_map(|channel| channel.sprite.exited.then_some(channel.sprite.number))
                .collect_vec()
        });

        for sprite_num in sprites_to_finish {
            reserve_player_mut(|player| {
                let sprite: &mut Sprite = player.movie.score.get_sprite_mut(sprite_num as i16);
                sprite.reset();
            });
        }

        // Find spans that should be entered
        let spans_to_enter: Vec<_> = self
            .sprite_spans
            .iter()
            .filter(|span| Self::is_span_in_frame(span, frame_num))
            .filter(|span| {
                reserve_player_mut(|player| {
                    player
                        .movie
                        .score
                        .get_sprite(span.channel_number as i16)
                        .map_or(true, |sprite| !sprite.entered)
                })
            })
            .cloned()
            .collect();

        // Get initialization data for sprites
        let span_init_data: Vec<_> = spans_to_enter
            .iter()
            .filter_map(|span| {
                self.channel_initialization_data
                    .iter()
                    .find(|(_frame_index, channel_index, _data)| {
                        get_channel_number_from_index(*channel_index as u32)
                            == span.channel_number as u32
                            && _frame_index + 1 == span.start_frame
                    })
                    .map(|(_frame_index, _channel_index, data)| (span, data.clone()))
            })
            .collect();

        // Initialize sprite properties (member, position, etc.)
        for (span, data) in span_init_data.iter() {
            let sprite_num = span.channel_number as i16;
            let sprite: &mut Sprite = self.get_sprite_mut(sprite_num);
            sprite.entered = true;
            let is_sprite = span.channel_number > 0;
            if is_sprite {
                let member = CastMemberRef {
                    cast_lib: data.cast_lib as i32,
                    cast_member: data.cast_member as i32,
                };
                let _ = sprite_set_prop(sprite_num, "member", Datum::CastMember(member));
                sprite.ink = data.ink as i32;
                sprite.loc_h = data.pos_x as i32;
                sprite.loc_v = data.pos_y as i32;
                sprite.width = data.width as i32;
                sprite.height = data.height as i32;

                match data.color_flag {
                    // fore+back color has a PaletteIndex
                    0 => {
                        sprite.color = ColorRef::PaletteIndex(data.fore_color);
                        sprite.bg_color = ColorRef::PaletteIndex(data.back_color);
                    },
                    // only foreColor has hex
                    1 => {
                        sprite.color = ColorRef::Rgb(
                            data.fore_color,
                            data.fore_color_g,
                            data.fore_color_b
                        );
                        sprite.bg_color = ColorRef::PaletteIndex(data.back_color);
                    }
                    // only backColor has hex
                    2 => {
                        sprite.color = ColorRef::PaletteIndex(data.fore_color);
                        sprite.bg_color = ColorRef::Rgb(
                            data.back_color,
                            data.back_color_g,
                            data.back_color_b
                        );
                    }
                    // fore+back color has hex
                    3 => {
                        sprite.color = ColorRef::Rgb(
                            data.fore_color,
                            data.fore_color_g,
                            data.fore_color_b
                        );
                        sprite.bg_color = ColorRef::Rgb(
                            data.back_color,
                            data.back_color_g,
                            data.back_color_b
                        );
                    }
                    _ => {
                        web_sys::console::error_1(&JsValue::from_str(&format!(
                            "Unexpected color flag: {}",
                            data.color_flag
                        )));
                    }
                }
            }
        }

        // Attach behaviors and set their parameters - GROUP BY CHANNEL
        // Group spans by channel_number to process all behaviors for a sprite at once
        let spans_by_channel: std::collections::HashMap<u32, Vec<&ScoreSpriteSpan>> =
            spans_to_enter
                .iter()
                .fold(std::collections::HashMap::new(), |mut acc, span| {
                    acc.entry(span.channel_number)
                        .or_insert_with(Vec::new)
                        .push(span);
                    acc
                });

        for (channel_num, channel_spans) in spans_by_channel.iter() {
            web_sys::console::log_1(
                &format!(
                    "🔧 Attaching behaviors to channel {}: {} spans",
                    channel_num,
                    channel_spans.len()
                )
                .into(),
            );

            for span in channel_spans {
                if span.scripts.is_empty() {
                    continue;
                }

                for behavior_ref in &span.scripts {
                    // Create the behavior instance
                    let (script_instance_ref, datum_ref) = Self::create_behavior(
                        behavior_ref.cast_lib as i32,
                        behavior_ref.cast_member as i32,
                    );

                    // Extract the ScriptInstanceRef from datum_ref
                    let actual_instance_ref = reserve_player_mut(|player| {
                        let datum = player.get_datum(&datum_ref);
                        match datum {
                            Datum::ScriptInstanceRef(ref instance_ref) => Ok(instance_ref.clone()),
                            _ => Err(ScriptError::new("Expected ScriptInstanceRef".to_string())),
                        }
                    })
                    .expect("Failed to extract ScriptInstanceRef");

                    // Parameter setup
                    if !behavior_ref.parameter.is_empty() {
                        reserve_player_mut(|player| {
                            for param_ref in &behavior_ref.parameter {
                                let param_datum = player.get_datum(param_ref);
                                if let Datum::PropList(props, _) = param_datum {
                                    let props_to_set: Vec<(String, DatumRef)> = props
                                        .iter()
                                        .filter_map(|(key_ref, value_ref)| {
                                            let key = player.get_datum(key_ref);
                                            if let Datum::Symbol(key_name) = key {
                                                Some((key_name.clone(), value_ref.clone()))
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();

                                    for (prop_name, value_ref) in props_to_set {
                                        let _ = script_set_prop(
                                            player,
                                            &actual_instance_ref,
                                            &prop_name,
                                            &value_ref,
                                            false,
                                        );
                                    }
                                }
                            }
                            Ok::<(), ScriptError>(())
                        })
                        .expect("Failed to set behavior parameters");
                    }

                    // Attach behavior to sprite
                    reserve_player_mut(|player| {
                        let sprite_num = *channel_num as i16;

                        let current_list_datum =
                            sprite_get_prop(player, sprite_num, "scriptInstanceList");

                        let mut list: Vec<DatumRef> = match current_list_datum {
                            Ok(Datum::List(_, items, _)) => items.clone(),
                            _ => vec![],
                        };

                        list.push(datum_ref.clone());

                        let scripts = Datum::List(DatumType::List, list.clone(), false);
                        let _ = sprite_set_prop(sprite_num, "scriptInstanceList", scripts);
                        Ok::<(), ScriptError>(())
                    })
                    .expect("Failed to attach behavior to sprite");
                }
            }

            // Dispatch beginSprite once per channel after all behaviors are attached
            player_dispatch_event_to_sprite(
                &"beginSprite".to_owned(),
                &vec![],
                *channel_num as u16,
            );
        }
    }

    pub fn end_sprites(&mut self, prev_frame: u32, next_frame: u32) -> Vec<u32> {
        let channels_to_end: Vec<u32> = self
            .sprite_spans
            .iter()
            .filter(|span| {
                Self::is_span_in_frame(span, prev_frame)
                    && !Self::is_span_in_frame(span, next_frame)
            })
            .map(|span| span.channel_number)
            .collect_vec();

        for channel_num in channels_to_end.iter() {
            player_dispatch_event_to_sprite(
                &"endSprite".to_owned(),
                &vec![],
                channel_num.clone() as u16,
            );
        }
        channels_to_end
    }

    pub fn get_channel_count(&self) -> usize {
        return self.channels.len() - 1;
    }

    pub fn set_channel_count(&mut self, new_count: usize) {
        if new_count > self.channels.len() {
            let base_number = self.channels.len();
            let add_count = max(0, new_count - self.channels.len());
            let mut add_channels = (0..add_count)
                .map(|index| SpriteChannel::new(base_number + index))
                .collect_vec();
            self.channels.append(&mut add_channels);
        } else if new_count < self.channels.len() {
            let remove_count = self.channels.len() - new_count;
            for _ in 1..remove_count {
                self.channels.pop();
            }
        }

        JsApi::dispatch_score_changed();
    }

    #[allow(dead_code)]
    pub fn get_sprite(&self, number: i16) -> Option<&Sprite> {
        if number < 0 || number as usize > self.channels.len() - 1 {
            return None;
        }
        let channel = &self.channels.get(number as usize);
        return channel.map(|x| &x.sprite);
    }

    pub fn get_channel(&self, number: i16) -> &SpriteChannel {
        return &self.channels[number as usize];
    }

    pub fn get_sprite_mut(&mut self, number: i16) -> &mut Sprite {
        let channel = &mut self.channels[number as usize];
        return &mut channel.sprite;
    }

    pub fn load_from_score_chunk(
        &mut self,
        score_chunk: &crate::director::chunks::score::ScoreChunk,
    ) {
        self.set_channel_count(score_chunk.frame_data.header.num_channels as usize);

        self.channel_initialization_data = score_chunk.frame_data.frame_channel_data.clone();

        for (primary, secondary) in &score_chunk.frame_intervals {
            let is_frame_script_or_sprite_script =
                primary.channel_index == 0 || primary.channel_index > 5;
            if is_frame_script_or_sprite_script {
                // TODO support the other 5 reserved channels
                let sprite_span = ScoreSpriteSpan {
                    channel_number: get_channel_number_from_index(primary.channel_index),
                    start_frame: primary.start_frame,
                    end_frame: primary.end_frame,
                    scripts: match secondary {
                        Some(sec) => vec![ScoreBehaviorReference {
                            cast_lib: sec.cast_lib,
                            cast_member: sec.cast_member,
                            parameter: sec.parameter.clone(),
                        }],
                        None => Vec::new(),
                    },
                };
                self.sprite_spans.push(sprite_span);
            }
        }
    }

    pub fn load_from_dir(&mut self, dir: &DirectorFile) {
        let score_chunk = dir.score.as_ref().unwrap();
        let frame_labels_chunk = dir.frame_labels.as_ref();
        if frame_labels_chunk.is_some() {
            self.frame_labels = frame_labels_chunk.unwrap().labels.clone();
        }
        self.load_from_score_chunk(score_chunk);
        JsApi::dispatch_score_changed();
    }

    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            if channel.sprite.puppet {
                channel.sprite.reset();
            }
        }

        JsApi::dispatch_score_changed();
    }

    pub fn get_sorted_channels(&self) -> Vec<&SpriteChannel> {
        return self
            .channels
            .iter()
            .filter(|x| {
                x.sprite.member.is_some()
                    && x.sprite.member.as_ref().unwrap().is_valid()
                    && x.sprite.visible
            })
            .sorted_by(|a, b| {
                let res = a.sprite.loc_z.cmp(&b.sprite.loc_z);
                if res == std::cmp::Ordering::Equal {
                    a.number.cmp(&b.number)
                } else {
                    res
                }
            })
            .collect_vec();
    }

    pub fn get_active_script_instance_list(&self) -> Vec<ScriptInstanceRef> {
        let mut instance_list = vec![];
        for channel in &self.channels {
            for instance_ref in &channel.sprite.script_instance_list {
                instance_list.push(instance_ref.clone());
            }
        }
        return instance_list;
    }
}

pub fn sprite_get_prop(
    player: &mut DirPlayer,
    sprite_id: i16,
    prop_name: &str,
) -> Result<Datum, ScriptError> {
    let sprite = player.movie.score.get_sprite(sprite_id);
    match prop_name {
        "ilk" => Ok(Datum::Symbol("sprite".to_string())),
        "spriteNum" => Ok(Datum::Int(
            sprite.map_or(sprite_id as i32, |x| x.number as i32),
        )),
        "loc" => Ok(Datum::IntPoint(
            sprite.map_or((0, 0), |sprite| (sprite.loc_h, sprite.loc_v)),
        )),
        "width" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.width) as i32)),
        "height" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.height) as i32)),
        "blend" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.blend) as i32)),
        "ink" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.ink) as i32)),
        "left" => {
            let rect = get_sprite_rect(player, sprite_id);
            Ok(Datum::Int(rect.0 as i32))
        }
        "top" => {
            let rect = get_sprite_rect(player, sprite_id);
            Ok(Datum::Int(rect.1 as i32))
        }
        "right" => {
            let rect = get_sprite_rect(player, sprite_id);
            Ok(Datum::Int(rect.2 as i32))
        }
        "bottom" => {
            let rect = get_sprite_rect(player, sprite_id);
            Ok(Datum::Int(rect.3 as i32))
        }
        "rect" => {
            let rect = get_sprite_rect(player, sprite_id);
            Ok(Datum::IntRect(rect))
        }
        "bgColor" => Ok(Datum::ColorRef(
            sprite.map_or(ColorRef::PaletteIndex(0), |sprite| sprite.bg_color.clone()),
        )),
        "skew" => Ok(Datum::Float(sprite.map_or(0.0, |sprite| sprite.skew))),
        "locH" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.loc_h) as i32)),
        "locV" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.loc_v) as i32)),
        "locZ" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.loc_z) as i32)),
        "member" => Ok(Datum::CastMember(
            sprite
                .and_then(|x| x.member.as_ref())
                .map(|x| x.clone())
                .unwrap_or(NULL_CAST_MEMBER_REF),
        )),
        "flipH" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.flip_h))),
        "flipV" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.flip_v))),
        "rotation" => Ok(Datum::Float(sprite.map_or(0.0, |sprite| sprite.rotation))),
        "scriptInstanceList" => {
            let instance_ids = sprite.map_or(vec![], |x| x.script_instance_list.clone());
            let instance_ids = instance_ids
                .iter()
                .map(|x| player.alloc_datum(Datum::ScriptInstanceRef(x.clone())))
                .collect();
            Ok(Datum::List(DatumType::List, instance_ids, false))
        }
        "castNum" => Ok(Datum::Int(sprite.map_or(0, |x| {
            x.member.as_ref().map_or(0, |y| {
                CastMemberRefHandlers::get_cast_slot_number(y.cast_lib as u32, y.cast_member as u32)
                    as i32
            })
        }))),
        "scriptNum" => {
            let script_num = sprite
                .and_then(|sprite| sprite.script_instance_list.first())
                .map(|script_instance_ref| {
                    player.allocator.get_script_instance(&script_instance_ref)
                })
                .map(|script_instance| script_instance.script.cast_member);
            Ok(Datum::Int(script_num.unwrap_or(0)))
        }
        prop_name => {
            let datum_ref = sprite.and_then(|sprite| {
                reserve_player_mut(|player| {
                    sprite.script_instance_list.iter().find_map(|behavior| {
                        script_get_prop_opt(player, behavior, &prop_name.to_string())
                    })
                })
            });
            match datum_ref {
                Some(ref_) => Ok(player.get_datum(&ref_).clone()),
                None => {
                    return Err(ScriptError::new(format!(
                        "Cannot get prop {} of sprite",
                        prop_name
                    )))
                }
            }
        }
    }
}

pub fn borrow_sprite_mut<T1, F1, T2, F2>(sprite_id: i16, player_f: F2, f: F1) -> T1
where
    F1: FnOnce(&mut Sprite, T2) -> T1,
    F2: FnOnce(&DirPlayer) -> T2,
{
    reserve_player_mut(|player| {
        let arg = player_f(player);
        let sprite = player.movie.score.get_sprite_mut(sprite_id);
        f(sprite, arg)
    })
}

pub fn sprite_set_prop(sprite_id: i16, prop_name: &str, value: Datum) -> Result<(), ScriptError> {
    let result = match prop_name {
        "visible" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite.visible = value.to_bool()?;
                Ok(())
            },
        ),
        "stretch" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.stretch = value?;
                Ok(())
            },
        ),
        "locH" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.loc_h = value?;
                Ok(())
            },
        ),
        "locV" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.loc_v = value?;
                Ok(())
            },
        ),
        "locZ" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.loc_z = value?;
                Ok(())
            },
        ),
        "width" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.width = value?;
                Ok(())
            },
        ),
        "height" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.height = value?;
                Ok(())
            },
        ),
        "ink" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.ink = value?;
                Ok(())
            },
        ),
        "blend" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.blend = value?;
                Ok(())
            },
        ),
        "rotation" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                if value.is_number() {
                    sprite.rotation = value.to_float()?;
                } else {
                    sprite.rotation = 0.0;
                }
                Ok(())
            },
        ),
        "skew" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                if value.is_number() {
                    sprite.skew = value.to_float()?;
                } else {
                    sprite.skew = 0.0;
                }
                Ok(())
            },
        ),
        "flipH" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                if value.is_number() {
                    sprite.flip_h = value.to_bool()?
                } else {
                    sprite.flip_h = false;
                }
                Ok(())
            },
        ),
        "flipV" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                if value.is_number() {
                    sprite.flip_v = value.to_bool()?
                } else {
                    sprite.flip_v = false;
                }
                Ok(())
            },
        ),
        "backColor" => borrow_sprite_mut(
            sprite_id,
            |player| {
                if value.is_int() {
                    value.int_value()
                } else {
                    Ok(0)
                }
            },
            |sprite, value| {
                let value = value?;
                sprite.back_color = value;
                Ok(())
            },
        ),
        "bgColor" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite.bg_color = value.to_color_ref()?.to_owned();
                Ok(())
            },
        ),
        "color" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite.color = value.to_color_ref()?.to_owned();
                Ok(())
            },
        ),
        "member" => borrow_sprite_mut(
            sprite_id,
            |player| {
                let mem_ref = if let Datum::CastMember(cast_member) = value {
                    Some(cast_member)
                } else if value.is_string() {
                    let member = player
                        .movie
                        .cast_manager
                        .find_member_ref_by_name(&value.string_value()?);
                    member
                } else if value.is_number() {
                    let member = player
                        .movie
                        .cast_manager
                        .find_member_ref_by_number(value.int_value()? as u32);
                    member
                } else {
                    None
                };
                let member = match &mem_ref {
                    Some(member_ref) => player.movie.cast_manager.find_member_by_ref(&member_ref),
                    None => None,
                };
                let (width, height) = member
                    .map(|x| match &x.member_type {
                        CastMemberType::Bitmap(bitmap) => {
                            let bitmap =
                                player.bitmap_manager.get_bitmap(bitmap.image_ref).unwrap();
                            (bitmap.width, bitmap.height)
                        }
                        CastMemberType::Shape(shape) => {
                            (shape.shape_info.width, shape.shape_info.height)
                        }
                        _ => (0, 0),
                    })
                    .unwrap_or((0, 0));
                Ok((mem_ref, width, height))
            },
            |sprite, value| {
                let (mem_ref, width, height) = value?;
                if mem_ref.is_some() && width > 0 && height > 0 {
                    sprite.width = width as i32;
                    sprite.height = height as i32;
                }
                sprite.member = mem_ref;
                JsApi::on_sprite_member_changed(sprite_id);
                Ok(())
            },
        ),
        "memberNum" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                let value = value?;
                let new_member_ref = match &sprite.member {
                    Some(member_ref) => cast_member_ref(member_ref.cast_lib, value),
                    None => CastMemberRefHandlers::member_ref_from_slot_number(value as u32),
                };
                sprite.member = Some(new_member_ref);
                JsApi::on_sprite_member_changed(sprite_id);
                Ok(())
            },
        ),
        "castNum" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                let value = value?;
                let new_member_ref =
                    CastMemberRefHandlers::member_ref_from_slot_number(value as u32);
                sprite.member = Some(new_member_ref);
                JsApi::on_sprite_member_changed(sprite_id);
                Ok(())
            },
        ),
        "cursor" => borrow_sprite_mut(
            sprite_id,
            |player| {
                if value.is_int() {
                    Ok(CursorRef::System(value.int_value()?))
                } else if value.is_list() {
                    let mut cursor_ids = vec![];
                    for cursor_id in value.to_list()? {
                        cursor_ids.push(player.get_datum(cursor_id).int_value()?);
                    }
                    Ok(CursorRef::Member(cursor_ids))
                } else {
                    Err(ScriptError::new(
                        "cursor must be a number or a list".to_string(),
                    ))
                }
            },
            |sprite, cursor_ref| {
                sprite.cursor_ref = Some(cursor_ref?);
                Ok(())
            },
        ),
        "loc" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| match value {
                Datum::IntPoint((x, y)) => {
                    sprite.loc_h = x;
                    sprite.loc_v = y;
                    Ok(())
                }
                Datum::Void => Ok(()),
                _ => Err(ScriptError::new(format!(
                    "loc must be a point (received {})",
                    value.type_str()
                ))),
            },
        ),
        "rect" => borrow_sprite_mut(
            sprite_id,
            |player| {
                let sprite = player.movie.score.get_sprite(sprite_id).unwrap();
                let member_ref = sprite.member.as_ref();
                let cast_member =
                    member_ref.and_then(|x| player.movie.cast_manager.find_member_by_ref(&x));
                let reg_point = cast_member
                    .map(|x| match &x.member_type {
                        CastMemberType::Bitmap(bitmap) => bitmap.reg_point,
                        _ => (0, 0),
                    })
                    .unwrap_or((0, 0));

                reg_point
            },
            |sprite, reg_point| match value {
                Datum::IntRect((left, top, right, bottom)) => {
                    sprite.loc_h = left + reg_point.0 as i32;
                    sprite.loc_v = top + reg_point.1 as i32;
                    sprite.width = right - left;
                    sprite.height = bottom - top;
                    Ok(())
                }
                _ => Err(ScriptError::new("rect must be a rect".to_string())),
            },
        ),
        "scriptInstanceList" => {
            let ref_list = value.to_list()?;
            let instance_refs = borrow_sprite_mut(
                sprite_id,
                |player| {
                    let mut instance_ids = vec![];
                    for ref_id in ref_list {
                        let datum = player.get_datum(ref_id);
                        match datum {
                            Datum::ScriptInstanceRef(instance_id) => {
                                instance_ids.push(instance_id.clone());
                            }
                            _ => {
                                return Err(ScriptError::new(
                                    "Cannot set non-script to scriptInstanceList".to_string(),
                                ))
                            }
                        }
                    }
                    Ok(instance_ids)
                },
                |sprite, value| {
                    let instance_ids = value?;
                    sprite.script_instance_list = instance_ids.to_owned();
                    Ok(instance_ids)
                },
            )?;
            reserve_player_mut(|player| {
                let value_ref = player.alloc_datum(Datum::Int(sprite_id as i32));
                for instance_ref in instance_refs {
                    script_set_prop(
                        player,
                        &instance_ref,
                        &"spriteNum".to_string(),
                        &value_ref,
                        false,
                    )?
                }
                Ok(())
            })
        }
        "editable" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite.editable = value.to_bool()?;
                Ok(())
            },
        ),
        "quad" => borrow_sprite_mut(
            sprite_id,
            |player| {
                // quad should be a list of 4 points: [topLeft, topRight, bottomRight, bottomLeft]
                if let Ok(list) = value.to_list() {
                    if list.len() == 4 {
                        let mut points = Vec::new();
                        for point_ref in list {
                            let point = player.get_datum(point_ref).to_int_point()?;
                            points.push(point);
                        }
                        Ok(points)
                    } else {
                        Err(ScriptError::new(
                            "quad must be a list of 4 points".to_string(),
                        ))
                    }
                } else {
                    Err(ScriptError::new("quad must be a list".to_string()))
                }
            },
            |sprite, points| {
                // TODO: update sprite position and size
                let points = points?;
                sprite.quad = Some([points[0], points[1], points[2], points[3]]);
                Ok(())
            },
        ),
        prop_name => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite
                    .script_instance_list
                    .iter()
                    .find_map(|behavior| {
                        reserve_player_mut(|player| {
                            let value_ref = player.alloc_datum(value.clone());
                            match script_set_prop(
                                player,
                                behavior,
                                &prop_name.to_string(),
                                &value_ref,
                                true,
                            ) {
                                Ok(_) => Some(Ok(())),
                                Err(_) => None,
                            }
                        })
                    })
                    .unwrap_or_else(|| {
                        Err(ScriptError::new(format!(
                            "Cannot set prop {} of sprite",
                            prop_name
                        )))
                    })
            },
        ),
    };
    if result.is_ok() {
        JsApi::dispatch_channel_changed(sprite_id);
    }
    result
}

pub fn concrete_sprite_hit_test(player: &DirPlayer, sprite: &Sprite, x: i32, y: i32) -> bool {
    // Don't test collision for invisible sprites
    if !sprite.visible {
        return false;
    }

    let rect = get_concrete_sprite_rect(player, sprite);

    // Don't test collision for sprites positioned far off-screen
    // If the entire rect is in negative space, it's intentionally hidden
    if rect.right < 0 || rect.bottom < 0 {
        return false;
    }

    let left = rect.left;
    let top = rect.top;
    let right = rect.right;
    let bottom = rect.bottom;
    return x >= left && x < right && y >= top && y < bottom;
}

pub fn get_sprite_at(player: &DirPlayer, x: i32, y: i32, scripted: bool) -> Option<u32> {
    for channel in player.movie.score.get_sorted_channels().iter().rev() {
        if concrete_sprite_hit_test(player, &channel.sprite, x, y)
            && (!scripted || channel.sprite.script_instance_list.len() > 0)
        {
            return Some(channel.sprite.number as u32);
        }
    }

    return None;
}

pub fn get_concrete_sprite_rect(player: &DirPlayer, sprite: &Sprite) -> IntRect {
    let member = sprite
        .member
        .as_ref()
        .and_then(|member_ref| player.movie.cast_manager.find_member_by_ref(member_ref));
    if member.is_none() {
        return IntRect::from_size(sprite.loc_h, sprite.loc_v, sprite.width, sprite.height);
    }
    let member = member.unwrap();

    match &member.member_type {
        CastMemberType::Bitmap(bitmap_member) => {
            let sprite_bitmap = player.bitmap_manager.get_bitmap(bitmap_member.image_ref);
            if sprite_bitmap.is_none() {
                return IntRect::from(sprite.loc_h, sprite.loc_v, sprite.width, sprite.height);
            }
            let src_bitmap = sprite_bitmap.unwrap();
            let reg_x = if sprite.flip_h {
                src_bitmap.width as i16 - bitmap_member.reg_point.0
            } else {
                bitmap_member.reg_point.0
            };
            let reg_y = if sprite.flip_v {
                src_bitmap.height as i16 - bitmap_member.reg_point.1
            } else {
                bitmap_member.reg_point.1
            };

            let dst_rect = IntRect::from(
                sprite.loc_h - reg_x as i32,
                sprite.loc_v - reg_y as i32,
                sprite.loc_h - reg_x as i32 + sprite.width,
                sprite.loc_v - reg_y as i32 + sprite.height,
            );
            dst_rect
        }
        CastMemberType::Shape(shape_member) => {
            let reg_x = shape_member.shape_info.reg_point.0;
            let reg_y = shape_member.shape_info.reg_point.1;
            IntRect::from(
                sprite.loc_h - reg_x as i32,
                sprite.loc_v - reg_y as i32,
                sprite.width + sprite.loc_h - reg_x as i32,
                sprite.height + sprite.loc_v - reg_y as i32,
            )
        }
        CastMemberType::Field(field_member) => {
            IntRect::from_size(sprite.loc_h, sprite.loc_v, field_member.width as i32, 12)
        } // TODO
        CastMemberType::Text(text_member) => {
            IntRect::from_size(sprite.loc_h, sprite.loc_v, text_member.width as i32, 12)
        } // TODO
        _ => IntRect::from_size(sprite.loc_h, sprite.loc_v, sprite.width, sprite.height),
    }
}

pub fn get_score<'a>(movie: &'a Movie, score_source: &ScoreRef) -> Option<&'a Score> {
    match score_source {
        ScoreRef::Stage => Some(&movie.score),
        ScoreRef::FilmLoop(member_ref) => {
            let member = movie.cast_manager.find_member_by_ref(member_ref);
            if member.is_none() {
                return None;
            }
            let member = member.unwrap();
            match &member.member_type {
                CastMemberType::FilmLoop(film_loop_member) => Some(&film_loop_member.score),
                _ => return None,
            }
        }
    }
}

pub fn get_score_mut<'a>(movie: &'a mut Movie, score_source: &ScoreRef) -> Option<&'a mut Score> {
    match score_source {
        ScoreRef::Stage => Some(&mut movie.score),
        ScoreRef::FilmLoop(member_ref) => {
            let member = movie.cast_manager.find_mut_member_by_ref(member_ref);
            if member.is_none() {
                return None;
            }
            let member = member.unwrap();
            match &mut member.member_type {
                CastMemberType::FilmLoop(film_loop_member) => Some(&mut film_loop_member.score),
                _ => return None,
            }
        }
    }
}

pub fn get_score_sprite<'a>(
    movie: &'a Movie,
    score_source: &ScoreRef,
    channel_num: i16,
) -> Option<&'a Sprite> {
    let score = get_score(movie, score_source)?;
    score.get_sprite(channel_num)
}

pub fn get_score_sprite_mut<'a>(
    movie: &'a mut Movie,
    score_source: &ScoreRef,
    channel_num: i16,
) -> Option<&'a mut Sprite> {
    let score = get_score_mut(movie, score_source)?;
    Some(score.get_sprite_mut(channel_num))
}
