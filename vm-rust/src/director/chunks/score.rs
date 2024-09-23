use binary_reader::{BinaryReader, Endian};
use itertools::Itertools;
use log::error;

use crate::{io::{list_readers::read_u16, reader::DirectorExt}, utils::log_i};

#[allow(dead_code)]
pub struct ScoreFrameDelta {
  offset: u32,
  data: Vec<u8>,
}

#[allow(dead_code)]
impl ScoreFrameDelta {
  pub fn new(offset: u32, data: Vec<u8>) -> Self {
    ScoreFrameDelta { offset, data }
  }
}

#[allow(dead_code)]
const K_CHANNEL_DATA_SIZE: usize = 38664; // (25 * 50);

#[allow(dead_code)]
#[derive(Clone, Default, PartialEq)]
pub struct ScoreFrameChannelData {
  pub sprite_type: u8,
  pub ink: u8,
  pub fore_color: u8,
  pub back_color: u8,
  pub cast_lib: u16,
  pub cast_member: u16,
  pub unk1: u16,
  pub unk2: u16,
  pub pos_y: u16,
  pub pos_x: u16,
  pub height: u16,
  pub width: u16,
}

impl ScoreFrameChannelData {
  pub fn read(reader: &mut BinaryReader) -> ScoreFrameChannelData {
    let sprite_type = reader.read_u8().unwrap();
    let ink = reader.read_u8().unwrap();
    let fore_color = reader.read_u8().unwrap();
    let back_color = reader.read_u8().unwrap();
    let cast_lib = reader.read_u16().unwrap();
    let cast_member = reader.read_u16().unwrap();
    let unk1 = reader.read_u16().unwrap();
    let unk2 = reader.read_u16().unwrap();
    let pos_y = reader.read_u16().unwrap();
    let pos_x = reader.read_u16().unwrap();
    let height = reader.read_u16().unwrap();
    let width = reader.read_u16().unwrap();

    ScoreFrameChannelData { sprite_type, ink, fore_color, back_color, cast_lib, cast_member, unk1, unk2, pos_y, pos_x, height, width }
  }
}

pub struct ScoreFrameData {
  pub header: ScoreFrameDataHeader,
  pub decompressed_data: Vec<u8>,
  pub frame_channel_data: Vec<(u32, u16, ScoreFrameChannelData)>,
}

pub struct ScoreFrameDataHeader {
  pub frame_count: u32,
  pub sprite_record_size: u16,
  pub num_channels: u16,
}

impl ScoreFrameData {
  #[allow(unused_variables)]
  pub fn read(reader: &mut BinaryReader) -> ScoreFrameData {
    let header = Self::read_header(reader);
    log_i(format_args!("ScoreFrameData {} {} {}", header.frame_count, header.num_channels, header.sprite_record_size).to_string().as_str());

    let mut channel_data = vec![0u8; (header.frame_count as usize) * (header.num_channels as usize) * (header.sprite_record_size as usize)];
    
    let mut frame_index = 0;
    while !reader.eof() {
      let length = reader.read_u16().unwrap();

      if length == 0 {
        break;
      }

      let frame_length = length - 2;
      if frame_length > 0 {
        let chunk_data = reader.read_bytes(frame_length as usize).unwrap();
        let mut frame_chunk_reader = BinaryReader::from_u8(chunk_data);
        frame_chunk_reader.set_endian(Endian::Big);

        // director reserves the first 6 channels:
        // note that channel indices are different than channel numbers
        // ┌───────┬─────────────────┐
        // │ index │                 │
        // ├───────┼─────────────────┤
        // │     0 │ frame script    │
        // │     1 │ palette         │
        // │     2 │ transition      │
        // │     3 │ sound 1         │
        // │     4 │ sound 2         │
        // │     5 │ tempo           │
        // │   N>5 │ sprites         │
        // └───────┴─────────────────┘
        let mut channel_index = 0;
        while !frame_chunk_reader.eof() {
          channel_index = channel_index + 1;
          let channel_size = frame_chunk_reader.read_u16().unwrap() as usize;
          let channel_offset = frame_chunk_reader.read_u16().unwrap() as usize;
          let channel_delta = frame_chunk_reader.read_bytes(channel_size).unwrap();

          let frame_offset = (frame_index as usize) * (header.num_channels as usize) * (header.sprite_record_size as usize);
          channel_data[frame_offset + channel_offset..frame_offset + channel_offset + channel_size].copy_from_slice(channel_delta);
        }
      }
      frame_index = frame_index + 1;
    }

    let (decompressed_data, frame_channel_data) = {
      let mut frame_channel_data = vec![];
      let decompressed_data = channel_data;
      let mut channel_reader = BinaryReader::from_vec(&decompressed_data);
      channel_reader.set_endian(Endian::Big);
      for frame_index in 0..header.frame_count {
        for channel_index in 0..header.num_channels {
          let pos = channel_reader.pos;
          let data = ScoreFrameChannelData::read(&mut channel_reader);
          channel_reader.jmp(pos + header.sprite_record_size as usize);
          if data != ScoreFrameChannelData::default() {
            log_i(format_args!("frame_index={frame_index} channel_index={channel_index} sprite_type={} ink={} fore_color={} back_color={} pos_y={} pos_x={} height={} width={}", data.sprite_type, data.ink, data.fore_color, data.back_color, data.pos_y, data.pos_x, data.height, data.width).to_string().as_str());
            frame_channel_data.push((frame_index, channel_index, data));
          }
        }
      }
      (decompressed_data, frame_channel_data)
    };

    ScoreFrameData {
      header,
      decompressed_data,
      frame_channel_data
    }
  }

  #[allow(unused_variables)]
  fn read_header(reader: &mut BinaryReader) -> ScoreFrameDataHeader {
    let actual_length = reader.read_u32().unwrap();
    let unk1 = reader.read_u32().unwrap();
    let frame_count = reader.read_u32().unwrap();
    let frames_version = reader.read_u16().unwrap();
    let sprite_record_size = reader.read_u16().unwrap();
    let num_channels = reader.read_u16().unwrap();
    let _num_channels_displayed: u16;

    if frames_version > 13 {
      _num_channels_displayed = reader.read_u16().unwrap();
    } else {
      if frames_version <= 7 {
        _num_channels_displayed = 48;
      } else {
        _num_channels_displayed = 120;
      }
      reader.read_u16().unwrap();  // Skip
    }

    ScoreFrameDataHeader { 
      frame_count,
      sprite_record_size,
      num_channels,
    }
  }
}

pub struct FrameIntervalPrimary {
  pub start_frame: u32,
  pub end_frame: u32,
  pub unk0: u32,
  pub unk1: u32,
  pub channel_index: u32,
  pub unk2: u16,
  pub unk3: u32,
  pub unk4: u16,
  pub unk5: u32,
  pub unk6: u32,
  pub unk7: u32,
  pub unk8: u32,
}

impl FrameIntervalPrimary {
  pub fn read(reader: &mut BinaryReader) -> Result<Self, ()> {
    Ok(FrameIntervalPrimary {
      start_frame: reader.read_u32().map_err(|_| ())?,
      end_frame: reader.read_u32().map_err(|_| ())?,
      unk0: reader.read_u32().map_err(|_| ())?,
      unk1: reader.read_u32().map_err(|_| ())?,
      channel_index: reader.read_u32().map_err(|_| ())?,
      unk2: reader.read_u16().map_err(|_| ())?,
      unk3: reader.read_u32().map_err(|_| ())?,
      unk4: reader.read_u16().map_err(|_| ())?,
      unk5: reader.read_u32().map_err(|_| ())?,
      unk6: reader.read_u32().map_err(|_| ())?,
      unk7: reader.read_u32().map_err(|_| ())?,
      unk8: reader.read_u32().map_err(|_| ())?,
    })
  }
}

pub struct FrameIntervalSecondary {
  pub cast_lib: u16,
  pub cast_member: u16,
  pub unk0: u32,
}

impl FrameIntervalSecondary {
  pub fn read(reader: &mut BinaryReader) -> Self {
    FrameIntervalSecondary {
      cast_lib: reader.read_u16().unwrap(),
      cast_member: reader.read_u16().unwrap(),
      unk0: reader.read_u32().unwrap(),
    }
  }
}

pub struct ScoreChunkHeader {
  pub total_length: u32,
  pub unk1: u32,
  pub unk2: u32,
  pub entry_count: u32,
  pub unk3: u32,
  pub entry_size_sum: u32,
}

pub struct ScoreChunk {
  pub header: ScoreChunkHeader,
  pub entries: Vec<Vec<u8>>,
  pub frame_interval_primaries: Vec<FrameIntervalPrimary>,
  pub frame_interval_secondaries: Vec<Option<FrameIntervalSecondary>>,
  pub frame_data: ScoreFrameData,
}

impl ScoreChunk {
  #[allow(unused_variables)]
  pub fn read(reader: &mut BinaryReader, dir_version: u16) -> Result<Self, ()> {
    let header = Self::read_header(reader);

    let offsets: Vec<usize> = (0..header.entry_count+1)
      .map(|_| reader.read_u32().unwrap() as usize)
      .collect();

    let mut entries = (0..header.entry_count as usize).map(|index| {
      let next_offset = offsets[index + 1];
      let length = next_offset - offsets[index];

      return reader.read_bytes(length).unwrap().to_vec();
    }).collect_vec();

    let mut delta_reader = BinaryReader::from_vec(&entries[0]);
    delta_reader.set_endian(Endian::Big);

    let frame_data = ScoreFrameData::read(&mut delta_reader);

    let frame_interval_entries = entries.split_off(3);
    let mut frame_interval_primaries = vec![];
    let mut frame_interval_secondaries = vec![];

    for i in (0..frame_interval_entries.len()).step_by(3) {
      let primary_entry = &frame_interval_entries[i];
      if primary_entry.is_empty() {
        continue;
      }
      let mut primary_reader = BinaryReader::from_u8(primary_entry);
      if let Ok(item) = FrameIntervalPrimary::read(&mut primary_reader) {
        frame_interval_primaries.push(item);
      } else {
        error!("Failed to read FrameIntervalPrimary at index {}", i);
        break;
      }
      let secondary_entry = &frame_interval_entries[i+1];
      if secondary_entry.is_empty() {
        frame_interval_secondaries.push(None);
      } else {
        let mut secondary_reader = BinaryReader::from_u8(secondary_entry);
        frame_interval_secondaries.push(Some(FrameIntervalSecondary::read(&mut secondary_reader)));
      }
      // TODO tertiary
    }

    Ok(ScoreChunk {
      header,
      entries,
      frame_interval_primaries,
      frame_interval_secondaries,
      frame_data,
    })
  }

  fn read_header(reader: &mut BinaryReader) -> ScoreChunkHeader {
    reader.set_endian(Endian::Big);
    ScoreChunkHeader {
      total_length: reader.read_u32().unwrap(),
      unk1: reader.read_u32().unwrap(),
      unk2: reader.read_u32().unwrap(),
      entry_count: reader.read_u32().unwrap(),
      unk3: reader.read_u32().unwrap(), // entry_count + 1
      entry_size_sum: reader.read_u32().unwrap(),
    }
  }
}

#[derive(Clone)]
pub struct FrameLabel {
    pub frame_num: i32,
    pub label: String,
}

pub struct FrameLabelsChunk {
  pub labels: Vec<FrameLabel>,
}

impl FrameLabelsChunk {
  pub fn from_reader(reader: &mut BinaryReader, _dir_version: u16) -> Result<FrameLabelsChunk, String> {
    reader.set_endian(binary_reader::Endian::Big);

    let labels_count = reader.read_u16().unwrap() as usize;
    let label_frames: Vec<(usize, usize)> = (0..labels_count)
      .map(|_i| {
          let frame_num = reader.read_u16().unwrap() as usize;
          let label_offset = reader.read_u16().unwrap() as usize;
          (label_offset, frame_num)
      })
      .collect();

    let labels_size: usize = reader.read_u32().unwrap() as usize;
    let labels: Vec<FrameLabel> = (0..labels_count)
      .map(|i| {
          let (label_offset, frame_num) = label_frames[i];
          let label_len = if i < labels_count - 1 {
              label_frames[i + 1].0 - label_offset
          } else {
              labels_size - label_offset
          };
          let label_str = reader.read_string(label_len).unwrap();
          // info!("label: {}", label_str);
          FrameLabel {
              frame_num: frame_num as i32,
              label: label_str.to_string()
          }
      })
      .collect();

    Ok(FrameLabelsChunk { labels })
  }
}