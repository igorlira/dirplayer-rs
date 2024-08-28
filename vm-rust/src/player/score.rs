use std::cmp::max;

use itertools::Itertools;

use crate::{director::{chunks::score::FrameLabel, file::DirectorFile, lingo::datum::{datum_bool, Datum, DatumType}}, js_api::JsApi};

use super::{allocator::ScriptInstanceAllocatorTrait, cast_lib::{cast_member_ref, NULL_CAST_MEMBER_REF}, cast_member::CastMemberType, geometry::{IntRect, IntRectTuple}, handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers, reserve_player_mut, script::script_set_prop, script_ref::ScriptInstanceRef, sprite::{ColorRef, CursorRef, Sprite}, DirPlayer, ScriptError};

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
pub struct ScoreFrameScriptReference {
  pub start_frame: u32,
  pub end_frame: u32,
  pub cast_lib: u16,
  pub cast_member: u16,
}

pub struct Score {
  pub channels: Vec<SpriteChannel>,
  pub script_references: Vec<ScoreFrameScriptReference>,
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

impl Score {
  pub fn empty() -> Score {
    Score {
      channels: vec![],
      script_references: vec![],
      frame_labels: vec![],
    }
  }

  pub fn get_script_in_frame(&self, frame: u32) -> Option<ScoreFrameScriptReference> {
    return self.script_references.iter()
      .find(|x| frame >= x.start_frame && frame <= x.end_frame)
      .map(|x| x.clone())
  }

  pub fn get_channel_count(&self) -> usize {
    return self.channels.len();
  }

  pub fn set_channel_count(&mut self, new_count: usize) {
    if new_count > self.channels.len() {
      let base_number = self.channels.len() + 1;
      let add_count = max(0, new_count - self.channels.len());
      let mut add_channels = (0..add_count).map(|index| SpriteChannel::new(base_number + index)).collect_vec();
      self.channels.append(&mut add_channels);
    } else if new_count < self.channels.len() {
      let remove_count = self.channels.len() - new_count;
      for _ in 0..remove_count {
        self.channels.pop();
      }
    }

    JsApi::dispatch_score_changed();
  }

  #[allow(dead_code)]
  pub fn get_sprite(&self, number: i16) -> Option<&Sprite> {
    if number <= 0 || number as usize > self.channels.len() {
      return None;
    }
    let channel = &self.channels.get(number as usize - 1);
    return channel.map(|x| &x.sprite);
  }

  pub fn get_channel(&self, number: i16) -> &SpriteChannel {
    return &self.channels[number as usize - 1];
  }

  pub fn get_sprite_mut(&mut self, number: i16) -> &mut Sprite {
    let channel = &mut self.channels[number as usize - 1];
    return &mut channel.sprite;
  }

  pub fn load_from_dir(&mut self, dir: &DirectorFile) {
    let score_chunk = dir.score.as_ref().unwrap();
    self.set_channel_count(score_chunk.frame_data.header.num_channels as usize);

    let frame_labels_chunk = dir.frame_labels.as_ref();
    if frame_labels_chunk.is_some() {
      self.frame_labels = frame_labels_chunk.unwrap().labels.clone();
    }

    for i in 0..score_chunk.frame_interval_primaries.len() {
      let primary = &score_chunk.frame_interval_primaries[i];
      let secondary = if i < score_chunk.frame_interval_secondaries.len() {
        &score_chunk.frame_interval_secondaries[i]
      } else {
        continue;
      };

      self.script_references.push(
        ScoreFrameScriptReference {
          start_frame: primary.start_frame, 
          end_frame: primary.end_frame, 
          cast_lib: secondary.cast_lib, 
          cast_member: secondary.cast_member,
        }
      );
    }

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
    return self.channels
      .iter()
      .filter(|x| x.sprite.member.is_some() && x.sprite.member.as_ref().unwrap().is_valid() && x.sprite.visible)
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
    "spriteNum" => Ok(Datum::Int(sprite.map_or(sprite_id as i32, |x| x.number as i32))),
    "loc" => Ok(Datum::IntPoint(sprite.map_or((0, 0), |sprite| (sprite.loc_h, sprite.loc_v)))),
    "width" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.width) as i32)),
    "height" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.height) as i32)),
    "blend" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.blend) as i32)),
    "ink" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.ink) as i32)),
    "left" => {
      let rect = get_sprite_rect(player, sprite_id);
      Ok(Datum::Int(rect.0 as i32))
    },
    "top" => {
      let rect = get_sprite_rect(player, sprite_id);
      Ok(Datum::Int(rect.1 as i32))
    },
    "right" => {
      let rect = get_sprite_rect(player, sprite_id);
      Ok(Datum::Int(rect.2 as i32))
    },
    "bottom" => {
      let rect = get_sprite_rect(player, sprite_id);
      Ok(Datum::Int(rect.3 as i32))
    },
    "rect" => {
      let rect = get_sprite_rect(player, sprite_id);
      Ok(Datum::IntRect(rect))
    },
    "bgColor" => Ok(Datum::ColorRef(sprite.map_or(ColorRef::PaletteIndex(0), |sprite| sprite.bg_color.clone()))),
    "skew" => Ok(Datum::Float(sprite.map_or(0.0, |sprite| sprite.skew))),
    "locH" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.loc_h) as i32)),
    "locV" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.loc_v) as i32)),
    "locZ" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.loc_z) as i32)),
    "member" => Ok(Datum::CastMember(
      sprite
        .and_then(|x| x.member.as_ref())
        .map(|x| x.clone())
        .unwrap_or(NULL_CAST_MEMBER_REF)
    )),
    "flipH" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.flip_h))),
    "flipV" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.flip_v))),
    "rotation" => Ok(Datum::Float(sprite.map_or(0.0, |sprite| sprite.rotation))),
    "scriptInstanceList" => {
      let instance_ids = sprite.map_or(vec![], |x| x.script_instance_list.clone());
      let instance_ids = instance_ids.iter().map(|x| player.alloc_datum(Datum::ScriptInstanceRef(x.clone()))).collect();
      Ok(Datum::List(DatumType::List, instance_ids, false))
    },
    "castNum" => Ok(
      Datum::Int(
        sprite.map_or(
          0, 
          |x| {
            x.member
              .as_ref()
              .map_or(
                0, 
                |y| CastMemberRefHandlers::get_cast_slot_number(y.cast_lib as u32, y.cast_member as u32) as i32
              )
          }
        )
      )
    ),
    "scriptNum" => {
      let script_num = sprite
        .and_then(|sprite| sprite.script_instance_list.first())
        .map(|script_instance_ref| player.allocator.get_script_instance(&script_instance_ref))
        .map(|script_instance| script_instance.script.cast_member);
      Ok(Datum::Int(script_num.unwrap_or(0)))
    },
    _ => Err(ScriptError::new(format!("Cannot get prop {} of sprite", prop_name))),
  }
}

pub fn borrow_sprite_mut<T1, F1, T2, F2>(
  sprite_id: i16,
  player_f: F2,
  f: F1,
) -> T1 where F1 : FnOnce(&mut Sprite, T2) -> T1, F2 : FnOnce(&DirPlayer) -> T2 {
  reserve_player_mut(|player| {
    let arg = player_f(player);
    let sprite = player.movie.score.get_sprite_mut(sprite_id);
    f(sprite, arg)
  })
}

pub fn sprite_set_prop(
  sprite_id: i16,
  prop_name: &String,
  value: Datum,
) -> Result<(), ScriptError> {
  let result = match prop_name.as_str() {
    "visible" => borrow_sprite_mut(
      sprite_id,
      |_| {}, 
      |sprite, _| {
        sprite.visible = value.to_bool()?;
        Ok(())
      }
    ),
    "stretch" => borrow_sprite_mut(
      sprite_id, 
      |player| value.int_value(),
      |sprite, value| {
        sprite.stretch = value?;
        Ok(())
      }
    ),
    "locH" => borrow_sprite_mut(
      sprite_id, 
      |player| value.int_value(),
      |sprite, value| {
        sprite.loc_h = value?;
        Ok(())
      }
    ),
    "locV" => borrow_sprite_mut(
      sprite_id, 
      |player| value.int_value(),
      |sprite, value| {
        sprite.loc_v = value?;
        Ok(())
      }
    ),
    "locZ" => borrow_sprite_mut(
      sprite_id, 
      |player| value.int_value(),
      |sprite, value| {
        sprite.loc_z = value?;
        Ok(())
      }
    ),
    "width" => borrow_sprite_mut(
      sprite_id, 
      |player| value.int_value(),
      |sprite, value| {
        sprite.width = value?;
        Ok(())
      }
    ),
    "height" => borrow_sprite_mut(
      sprite_id, 
      |player| value.int_value(),
      |sprite, value| {
        sprite.height = value?;
        Ok(())
      }
    ),
    "ink" => borrow_sprite_mut(
      sprite_id, 
      |player| value.int_value(),
      |sprite, value| {
        sprite.ink = value?;
        Ok(())
      }
    ),
    "blend" => borrow_sprite_mut(
      sprite_id, 
      |player| value.int_value(),
      |sprite, value| {
        sprite.blend = value?;
        Ok(())
      }
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
      }
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
      }
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
      }
    ),
    "flipV" =>  borrow_sprite_mut(
      sprite_id,
      |_| {},
      |sprite, _| {
        if value.is_number() {
          sprite.flip_v = value.to_bool()?
        } else {
          sprite.flip_v = false;
        }
        Ok(())
      }
    ),
    "backColor" =>  borrow_sprite_mut(
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
      }
    ),
    "bgColor" => borrow_sprite_mut(
      sprite_id,
      |_| {},
      |sprite, _| {
        sprite.bg_color = value.to_color_ref()?.to_owned();
        Ok(())
      }
    ),
    "color" => borrow_sprite_mut(
      sprite_id,
      |_| {},
      |sprite, _| {
        sprite.color = value.to_color_ref()?.to_owned();
        Ok(())
      }
    ),
    "member" => borrow_sprite_mut(
      sprite_id, 
      |player| {
        let mem_ref = if let Datum::CastMember(cast_member) = value {
          Some(cast_member)
        } else if value.is_string() {
          let member = player.movie.cast_manager.find_member_ref_by_name(&value.string_value()?);
          member
        } else if value.is_number() {
          let member = player.movie.cast_manager.find_member_ref_by_number(value.int_value()? as u32);
          member
        } else {
          None
        };
        let member = match &mem_ref {
          Some(member_ref) => player.movie.cast_manager.find_member_by_ref(&member_ref),
          None => None,
        };
        let (width, height) = member.map(
          |x| {
            match &x.member_type {
              CastMemberType::Bitmap(bitmap) => {
                let bitmap = player.bitmap_manager.get_bitmap(bitmap.image_ref).unwrap();
                (bitmap.width, bitmap.height)
              }
              CastMemberType::Shape(shape) => (shape.shape_info.width, shape.shape_info.height),
              _ => (0, 0),
            }
          }
        ).unwrap_or((0, 0));
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
      }
    ),
    "memberNum" => borrow_sprite_mut(
      sprite_id, 
      |player| value.int_value(),
      |sprite, value| {
        let value = value?;
        let new_member_ref = match &sprite.member {
          Some(member_ref) => cast_member_ref(member_ref.cast_lib, value),
          None => CastMemberRefHandlers::member_ref_from_slot_number(value as u32)
        };  
        sprite.member = Some(new_member_ref);
        JsApi::on_sprite_member_changed(sprite_id);
        Ok(())
      }
    ),
    "castNum" => borrow_sprite_mut(
      sprite_id, 
      |player| value.int_value(),
      |sprite, value| {
        let value = value?;
        let new_member_ref = CastMemberRefHandlers::member_ref_from_slot_number(value as u32);  
        sprite.member = Some(new_member_ref);
        JsApi::on_sprite_member_changed(sprite_id);
        Ok(())
      }
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
          Err(ScriptError::new("cursor must be a number or a list".to_string()))
        }
      },
      |sprite, cursor_ref| {
        sprite.cursor_ref = Some(cursor_ref?);
        Ok(())
      }
    ),
    "loc" => borrow_sprite_mut(
      sprite_id, 
      |_| {},
      |sprite, _| {
        match value {
          Datum::IntPoint((x, y)) => {
            sprite.loc_h = x;
            sprite.loc_v = y;
            Ok(())
          },
          Datum::Void => Ok(()),
          _ => Err(ScriptError::new(format!("loc must be a point (received {})", value.type_str()))),
        }
      }
    ),
    "rect" => borrow_sprite_mut(
      sprite_id, 
      |player| {
        let sprite = player.movie.score.get_sprite(sprite_id).unwrap();
        let member_ref = sprite.member.as_ref();
        let cast_member = member_ref.and_then(|x| player.movie.cast_manager.find_member_by_ref(&x));
        let reg_point = cast_member.map(
          |x| {
            match &x.member_type {
              CastMemberType::Bitmap(bitmap) => bitmap.reg_point,
              _ => (0, 0),
            }
          }
        ).unwrap_or((0, 0));

        reg_point
      },
      |sprite, reg_point| {
        match value {
          Datum::IntRect((left, top, right, bottom)) => {
            sprite.loc_h = left + reg_point.0 as i32;
            sprite.loc_v = top + reg_point.1 as i32;
            sprite.width = right - left;
            sprite.height = bottom - top;
            Ok(())
          },
          _ => Err(ScriptError::new("rect must be a rect".to_string())),
        }
      }
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
              },
              _ => {
                return Err(ScriptError::new("Cannot set non-script to scriptInstanceList".to_string()))
              },
            }
          }
          Ok(instance_ids)
        },
        |sprite, value| {
          let instance_ids = value?;
          sprite.script_instance_list = instance_ids.to_owned();
          Ok(instance_ids)
        }
      )?;
      reserve_player_mut(|player| {
        let value_ref = player.alloc_datum(Datum::Int(sprite_id as i32));
        for instance_ref in instance_refs {
          script_set_prop(
            player, 
            &instance_ref, 
            &"spriteNum".to_string(),
            &value_ref, 
            false
          )?
        }
        Ok(())
      })
    },
    "editable" => borrow_sprite_mut(
      sprite_id, 
      |_| {},
      |sprite, _| {
        sprite.editable = value.to_bool()?;
        Ok(())
      }
    ),
    _ => Err(ScriptError::new(format!("Cannot set prop {} of sprite", prop_name))),
  };
  if result.is_ok() {
    JsApi::dispatch_channel_changed(sprite_id);
  }
  result
}

pub fn concrete_sprite_hit_test(
  player: &DirPlayer,
  sprite: &Sprite,
  x: i32,
  y: i32,
) -> bool {
  let rect = get_concrete_sprite_rect(player, sprite);
  let left = rect.left;
  let top = rect.top;
  let right = rect.right;
  let bottom = rect.bottom;
  return x >= left && x < right && y >= top && y < bottom;
}

pub fn get_sprite_at(player: &DirPlayer, x: i32, y: i32, scripted: bool) -> Option<u32> {
  for channel in player.movie.score.get_sorted_channels().iter().rev() {
    if concrete_sprite_hit_test(player, &channel.sprite, x, y) && (!scripted || channel.sprite.script_instance_list.len() > 0) {
      return Some(channel.sprite.number as u32);
    }
  }

  return None;
}

pub fn get_concrete_sprite_rect(player: &DirPlayer, sprite: &Sprite) -> IntRect {
  let member = sprite.member.as_ref().and_then(|member_ref| 
    player
      .movie
      .cast_manager
      .find_member_by_ref(member_ref)
  );
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
        let reg_x = if sprite.flip_h { src_bitmap.width as i16 - bitmap_member.reg_point.0 } else { bitmap_member.reg_point.0 };
        let reg_y = if sprite.flip_v { src_bitmap.height as i16 - bitmap_member.reg_point.1 } else { bitmap_member.reg_point.1 };

        let dst_rect = IntRect::from(
            sprite.loc_h - reg_x as i32,
            sprite.loc_v - reg_y as i32, 
            sprite.loc_h - reg_x as i32 + sprite.width, 
            sprite.loc_v - reg_y as i32 + sprite.height
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
    CastMemberType::Field(field_member) => IntRect::from_size(sprite.loc_h, sprite.loc_v, field_member.width as i32, 12), // TODO
    CastMemberType::Text(text_member) => IntRect::from_size(sprite.loc_h, sprite.loc_v, text_member.width as i32, 12), // TODO
    _ => IntRect::from_size(sprite.loc_h, sprite.loc_v, sprite.width, sprite.height)
  }
}
