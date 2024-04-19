use binary_reader::{BinaryReader, Endian};
use itertools::Itertools;

use crate::{utils::log_i, io::reader::DirectorExt};

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
pub struct ScoreFrameChannelData {
  flags: u16,
  unk0: u16,
  cast_lib: u16,
  cast_member: u16,
  unk1: u16,
  pos_y: u16,
  pos_x: u16,
  height: u16,
  width: u16,
}

impl ScoreFrameChannelData {
  pub fn read(reader: &mut BinaryReader) -> ScoreFrameChannelData {
    let flags = reader.read_u16().unwrap();
    let unk0 = reader.read_u16().unwrap();
    let cast_lib = reader.read_u16().unwrap();
    let cast_member = reader.read_u16().unwrap();
    let unk1 = reader.read_u16().unwrap();
    let pos_y = reader.read_u16().unwrap();
    let pos_x = reader.read_u16().unwrap();
    let height = reader.read_u16().unwrap();
    let width = reader.read_u16().unwrap();

    ScoreFrameChannelData { flags, unk0, cast_lib, cast_member, unk1, pos_y, pos_x, height, width }
  }
}

pub struct ScoreFrameData {
  pub header: ScoreFrameDataHeader,
  pub uncompressed_data: Vec<u8>,
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
    
    while !reader.eof() {
      let fd_start = reader.pos as usize;
      let length = reader.read_u16().unwrap();

      if length == 0 {
        break;
      }

      let frame_length = length - 2;
      if frame_length > 0 {
        let chunk_data = reader.read_bytes(frame_length as usize).unwrap();
        let mut frame_chunk_reader = BinaryReader::from_u8(chunk_data);
        frame_chunk_reader.set_endian(Endian::Big);

        while !frame_chunk_reader.eof() {
          let channel_size = frame_chunk_reader.read_u16().unwrap() as usize;
          let channel_offset = frame_chunk_reader.read_u16().unwrap() as usize;
          let channel_delta = frame_chunk_reader.read_bytes(channel_size).unwrap();

          channel_data[channel_offset..channel_offset + channel_size].copy_from_slice(channel_delta);
        }
      }
    }

    let uncompressed_data = channel_data;

    let mut channel_reader = BinaryReader::from_vec(&uncompressed_data);
    channel_reader.set_endian(Endian::Big);
    for i in 0..header.frame_count {
      for j in 0..header.num_channels {
        let pos = channel_reader.pos;
        let channel_frame_data = ScoreFrameChannelData::read(&mut channel_reader);
        channel_reader.jmp(pos + header.sprite_record_size as usize);
        if channel_frame_data.flags != 0 {
          log_i(format_args!("frame {i} channel {j} flags={}", channel_frame_data.flags).to_string().as_str());
        }
      }
    }

    ScoreFrameData {
      header,
      uncompressed_data
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
  pub sprite_number: u32,
  pub unk2: u16,
  pub unk3: u32,
  pub unk4: u16,
  pub unk5: u32,
  pub unk6: u32,
  pub unk7: u32,
  pub unk8: u32,
}

impl FrameIntervalPrimary {
  pub fn read(reader: &mut BinaryReader) -> Self {
    FrameIntervalPrimary {
      start_frame: reader.read_u32().unwrap(),
      end_frame: reader.read_u32().unwrap(),
      unk0: reader.read_u32().unwrap(),
      unk1: reader.read_u32().unwrap(),
      sprite_number: reader.read_u32().unwrap(),
      unk2: reader.read_u16().unwrap(),
      unk3: reader.read_u32().unwrap(),
      unk4: reader.read_u16().unwrap(),
      unk5: reader.read_u32().unwrap(),
      unk6: reader.read_u32().unwrap(),
      unk7: reader.read_u32().unwrap(),
      unk8: reader.read_u32().unwrap(),
    }
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
  pub frame_interval_secondaries: Vec<FrameIntervalSecondary>,
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

    for (i, entry) in frame_interval_entries.iter().enumerate() {
      if entry.is_empty() {
        continue;
      }
      let is_primary = i % 3 == 0;
      let is_secondary = i % 3 == 1;
      let is_tertiary = i % 3 == 2;

      let mut frame_interval_reader = BinaryReader::from_u8(entry);
      frame_interval_reader.set_endian(Endian::Big);
      if is_primary {
        frame_interval_primaries.push(FrameIntervalPrimary::read(&mut frame_interval_reader));
      } else if is_secondary {
        frame_interval_secondaries.push(FrameIntervalSecondary::read(&mut frame_interval_reader));
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
      unk3: reader.read_u32().unwrap(),
      entry_size_sum: reader.read_u32().unwrap(),
    }
  }
}