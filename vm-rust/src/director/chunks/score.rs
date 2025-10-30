use binary_reader::{BinaryReader, Endian};
use log::error;

use crate::{io::reader::DirectorExt, utils::log_i};

use crate::player::datum_ref::DatumRef;
use crate::player::eval::eval_lingo;

use web_sys;
use web_sys::console;

use crate::PLAYER_OPT;

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
#[derive(Clone, Default, PartialEq, Debug)]
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
    pub color_flag: u8,
    pub fore_color_g: u8,
    pub back_color_g: u8,
    pub fore_color_b: u8,
    pub back_color_b: u8,
}

impl ScoreFrameChannelData {
    pub fn read(reader: &mut BinaryReader) -> Result<ScoreFrameChannelData, String> {
        let sprite_type = reader
            .read_u8()
            .map_err(|e| format!("Failed to read sprite_type: {:?}", e))?;
        let ink = reader
            .read_u8()
            .map_err(|e| format!("Failed to read ink: {:?}", e))?;
        let fore_color = reader
            .read_u8()
            .map_err(|e| format!("Failed to read fore_color: {:?}", e))?;
        let back_color = reader
            .read_u8()
            .map_err(|e| format!("Failed to read back_color: {:?}", e))?;
        let cast_lib = reader
            .read_u16()
            .map_err(|e| format!("Failed to read cast_lib: {:?}", e))?;
        let cast_member = reader
            .read_u16()
            .map_err(|e| format!("Failed to read cast_member: {:?}", e))?;
        let unk1 = reader
            .read_u16()
            .map_err(|e| format!("Failed to read unk1: {:?}", e))?;
        let unk2 = reader
            .read_u16()
            .map_err(|e| format!("Failed to read unk2: {:?}", e))?;
        let pos_y = reader
            .read_u16()
            .map_err(|e| format!("Failed to read pos_y: {:?}", e))?;
        let pos_x = reader
            .read_u16()
            .map_err(|e| format!("Failed to read pos_x: {:?}", e))?;
        let height = reader
            .read_u16()
            .map_err(|e| format!("Failed to read height: {:?}", e))?;
        let width = reader
            .read_u16()
            .map_err(|e| format!("Failed to read width: {:?}", e))?;

        let unk3 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read unk3: {:?}", e))?;
        let color_flag = (unk3 & 0xF0) >> 4;
        let unk3_2  = unk3 & 0x0F;

        let unk4 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read unk4: {:?}", e))?;
        let unk5 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read unk5: {:?}", e))?;
        let unk6 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read unk6: {:?}", e))?;
        let fore_color_g = reader
            .read_u8()
            .map_err(|e| format!("Failed to read fore_color_g: {:?}", e))?;
        let back_color_g = reader
            .read_u8()
            .map_err(|e| format!("Failed to read back_color_g: {:?}", e))?;
        let fore_color_b = reader
            .read_u8()
            .map_err(|e| format!("Failed to read fore_color_b: {:?}", e))?;
        let back_color_b = reader
            .read_u8()
            .map_err(|e| format!("Failed to read back_color_b: {:?}", e))?;

        Ok(ScoreFrameChannelData {
            sprite_type,
            ink,
            fore_color,
            back_color,
            cast_lib,
            cast_member,
            unk1,
            unk2,
            pos_y,
            pos_x,
            height,
            width,
            color_flag,
            fore_color_g,
            back_color_g,
            fore_color_b,
            back_color_b,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ScoreFrameData {
    pub header: ScoreFrameDataHeader,
    pub decompressed_data: Vec<u8>,
    pub frame_channel_data: Vec<(u32, u16, ScoreFrameChannelData)>,
}

impl Default for ScoreFrameData {
    fn default() -> Self {
        Self {
            header: ScoreFrameDataHeader::default(),
            decompressed_data: Vec::new(),
            frame_channel_data: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScoreFrameDataHeader {
    pub frame_count: u32,
    pub sprite_record_size: u16,
    pub num_channels: u16,
}

impl ScoreFrameData {
    #[allow(unused_variables)]
    pub fn read(reader: &mut BinaryReader) -> Result<ScoreFrameData, String> {
        let header = Self::read_header(reader)?;
        log_i(
            format_args!(
                "ScoreFrameData {} {} {}",
                header.frame_count, header.num_channels, header.sprite_record_size
            )
            .to_string()
            .as_str(),
        );

        let mut channel_data = vec![
            0u8;
            (header.frame_count as usize)
                * (header.num_channels as usize)
                * (header.sprite_record_size as usize)
        ];

        let mut frame_index = 0;
        while !reader.eof() {
            let length = reader
                .read_u16()
                .map_err(|e| format!("Failed to read frame length: {:?}", e))?;

            if length == 0 {
                break;
            }

            let frame_length = length - 2;
            if frame_length > 0 {
                let chunk_data = reader
                    .read_bytes(frame_length as usize)
                    .map_err(|e| format!("Failed to read chunk data: {:?}", e))?;
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
                    let channel_size = frame_chunk_reader
                        .read_u16()
                        .map_err(|e| format!("Failed to read channel size: {:?}", e))?
                        as usize;
                    let channel_offset = frame_chunk_reader
                        .read_u16()
                        .map_err(|e| format!("Failed to read channel offset: {:?}", e))?
                        as usize;
                    let channel_delta = frame_chunk_reader
                        .read_bytes(channel_size)
                        .map_err(|e| format!("Failed to read channel delta: {:?}", e))?;

                    let frame_offset = (frame_index as usize)
                        * (header.num_channels as usize)
                        * (header.sprite_record_size as usize);
                    let end_offset = frame_offset + channel_offset + channel_size;
                    if end_offset > channel_data.len() {
                        error!("❌ Channel data copy out of bounds. Frame offset: {}, Channel offset: {}, Channel size: {}, Total len: {}", 
                    frame_offset, channel_offset, channel_size, channel_data.len());
                        return Err("Channel data copy out of bounds".to_string());
                    }
                    channel_data[frame_offset + channel_offset..end_offset]
                        .copy_from_slice(&channel_delta);
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
                    let data = ScoreFrameChannelData::read(&mut channel_reader)?;
                    channel_reader.jmp(pos + header.sprite_record_size as usize);
                    if data != ScoreFrameChannelData::default() {
                        log_i(format_args!("frame_index={frame_index} channel_index={channel_index} sprite_type={} ink={} fore_color={} back_color={} pos_y={} pos_x={} height={} width={}", data.sprite_type, data.ink, data.fore_color, data.back_color, data.pos_y, data.pos_x, data.height, data.width).to_string().as_str());
                        frame_channel_data.push((frame_index, channel_index, data));
                    }
                }
            }
            (decompressed_data, frame_channel_data)
        };

        Ok(ScoreFrameData {
            header,
            decompressed_data,
            frame_channel_data,
        })
    }

    #[allow(unused_variables)]
    fn read_header(reader: &mut BinaryReader) -> Result<ScoreFrameDataHeader, String> {
        let actual_length = reader
            .read_u32()
            .map_err(|e| format!("Failed to read actual_length: {:?}", e))?;
        let unk1 = reader
            .read_u32()
            .map_err(|e| format!("Failed to read unk1: {:?}", e))?;
        let frame_count = reader
            .read_u32()
            .map_err(|e| format!("Failed to read frame_count: {:?}", e))?;
        let frames_version = reader
            .read_u16()
            .map_err(|e| format!("Failed to read frames_version: {:?}", e))?;
        let sprite_record_size = reader
            .read_u16()
            .map_err(|e| format!("Failed to read sprite_record_size: {:?}", e))?;
        let num_channels = reader
            .read_u16()
            .map_err(|e| format!("Failed to read num_channels: {:?}", e))?;
        let _num_channels_displayed: u16;

        if frames_version > 13 {
            _num_channels_displayed = reader
                .read_u16()
                .map_err(|e| format!("Failed to read _num_channels_displayed: {:?}", e))?;
        } else {
            if frames_version <= 7 {
                _num_channels_displayed = 48;
            } else {
                _num_channels_displayed = 120;
            }
            reader
                .read_u16()
                .map_err(|e| format!("Failed to skip u16: {:?}", e))?; // Skip
        }

        Ok(ScoreFrameDataHeader {
            frame_count,
            sprite_record_size,
            num_channels,
        })
    }
}

#[derive(Clone, Debug)]
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
    pub fn read(reader: &mut BinaryReader) -> Result<FrameIntervalPrimary, String> {
        let start_frame = reader
            .read_u32()
            .map_err(|e| format!("Failed to read start_frame: {:?}", e))?;
        let end_frame = reader
            .read_u32()
            .map_err(|e| format!("Failed to read end_frame: {:?}", e))?;
        let unk0 = reader
            .read_u32()
            .map_err(|e| format!("Failed to read unk0: {:?}", e))?;
        let unk1 = reader
            .read_u32()
            .map_err(|e| format!("Failed to read unk1: {:?}", e))?;
        let channel_index = reader
            .read_u32()
            .map_err(|e| format!("Failed to read channel_index: {:?}", e))?;
        let unk2 = reader
            .read_u16()
            .map_err(|e| format!("Failed to read unk2: {:?}", e))?;
        let unk3 = reader
            .read_u32()
            .map_err(|e| format!("Failed to read unk3: {:?}", e))?;
        let unk4 = reader
            .read_u16()
            .map_err(|e| format!("Failed to read unk4: {:?}", e))?;
        let unk5 = reader
            .read_u32()
            .map_err(|e| format!("Failed to read unk5: {:?}", e))?;
        let unk6 = reader
            .read_u32()
            .map_err(|e| format!("Failed to read unk6: {:?}", e))?;
        let unk7 = reader
            .read_u32()
            .map_err(|e| format!("Failed to read unk7: {:?}", e))?;
        let unk8 = reader
            .read_u32()
            .map_err(|e| format!("Failed to read unk8: {:?}", e))?;

        Ok(FrameIntervalPrimary {
            start_frame,
            end_frame,
            unk0,
            unk1,
            channel_index,
            unk2,
            unk3,
            unk4,
            unk5,
            unk6,
            unk7,
            unk8,
        })
    }
}

#[derive(Clone, Debug)]
pub struct FrameIntervalSecondary {
    pub cast_lib: u16,
    pub cast_member: u16,
    pub unk0: u32,
    pub parameter: Vec<DatumRef>,
}

impl FrameIntervalSecondary {
    pub fn read(reader: &mut BinaryReader) -> Result<FrameIntervalSecondary, String> {
        let cast_lib = reader
            .read_u16()
            .map_err(|e| format!("Failed to read cast_lib: {:?}", e))?;
        let cast_member = reader
            .read_u16()
            .map_err(|e| format!("Failed to read cast_member: {:?}", e))?;
        let unk0 = reader
            .read_u32()
            .map_err(|e| format!("Failed to read unk0: {:?}", e))?;

        let parameter = vec![];

        Ok(FrameIntervalSecondary {
            cast_lib,
            cast_member,
            unk0,
            parameter,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScoreChunkHeader {
    pub total_length: u32,
    pub unk1: u32,
    pub unk2: u32,
    pub entry_count: u32,
    pub unk3: u32,
    pub entry_size_sum: u32,
}

impl ScoreChunkHeader {
    pub fn read(reader: &mut BinaryReader) -> Result<Self, String> {
        Ok(ScoreChunkHeader {
            total_length: reader
                .read_u32()
                .map_err(|e| format!("Failed to read total_length: {:?}", e))?,
            unk1: reader
                .read_u32()
                .map_err(|e| format!("Failed to read unk1: {:?}", e))?,
            unk2: reader
                .read_u32()
                .map_err(|e| format!("Failed to read unk2: {:?}", e))?,
            entry_count: reader
                .read_u32()
                .map_err(|e| format!("Failed to read entry_count: {:?}", e))?,
            unk3: reader
                .read_u32()
                .map_err(|e| format!("Failed to read unk3: {:?}", e))?, // entry_count + 1
            entry_size_sum: reader
                .read_u32()
                .map_err(|e| format!("Failed to read entry_size_sum: {:?}", e))?,
        })
    }
}

#[derive(Clone)]
pub struct ScoreChunk {
    pub header: ScoreChunkHeader,
    pub entries: Vec<Vec<u8>>,
    pub frame_intervals: Vec<(FrameIntervalPrimary, Option<FrameIntervalSecondary>)>,
    pub frame_data: ScoreFrameData,
}

impl ScoreChunk {
    #[allow(unused_variables)]
    pub fn read(reader: &mut BinaryReader, dir_version: u16) -> Result<Self, String> {
        reader.set_endian(Endian::Big);

        // Read and analyze header
        let header = ScoreChunkHeader::read(reader)
            .map_err(|e| format!("Failed to read ScoreChunkHeader: {}", e))?;

        // Read offsets table
        let offsets_result: Result<Vec<usize>, String> = (0..header.entry_count + 1)
            .map(|i| {
                let offset = reader
                    .read_u32()
                    .map_err(|e| format!("Failed to read offset: {:?}", e))?
                    as usize;
                Ok(offset)
            })
            .collect();
        let offsets = offsets_result?;

        // Validate offsets
        if offsets.len() != (header.entry_count + 1) as usize {
            console::error_1(
                &format!(
                    "❌ Offsets count mismatch! Expected {}, Got {}",
                    header.entry_count + 1,
                    offsets.len()
                )
                .into(),
            );
            return Err("Offsets count mismatch.".to_string());
        }

        for j in 0..offsets.len().saturating_sub(1) {
            if offsets[j] > offsets[j + 1] {
                console::error_1(
                    &format!(
                        "❌ Offsets are not monotonic: offsets[{}] ({}) > offsets[{}] ({})",
                        j,
                        offsets[j],
                        j + 1,
                        offsets[j + 1]
                    )
                    .into(),
                );
                return Err("Offsets are not monotonic.".to_string());
            }
        }

        // Read all entries
        let entries_result: Result<Vec<Vec<u8>>, String> = (0..header.entry_count as usize)
    .map(|index| {
      let current_offset = offsets[index];
      let next_offset = offsets[index + 1];
      let length = next_offset - current_offset;

      if length > 0 {
        if reader.pos + length > reader.length {
          console::error_1(
            &format!(
              "❌ Calculated entry {} length ({}) + current pos ({}) exceeds total reader length ({}).",
              index, length, reader.pos, reader.length
            ).into()
          );
          return Err("Entry exceeds total reader length.".to_string());
        }

        let bytes = reader
          .read_bytes(length)
          .map_err(|e| format!("Failed to read bytes: {:?}", e))?
          .to_vec();

        Ok(bytes)
      } else {
        // empty entry
        Ok(Vec::new())
      }
    }).collect();

        let entries = entries_result?;

        // Process frame data from first entry - this contains sprite positioning and frame script information
        let frame_data = if !entries.is_empty() && !entries[0].is_empty() {
            let mut delta_reader = BinaryReader::from_vec(&entries[0]);
            delta_reader.set_endian(Endian::Big);
            ScoreFrameData::read(&mut delta_reader)?
        } else {
            ScoreFrameData::default()
        };
        let frame_intervals = Self::analyze_behavior_attachment_entries(&entries)?;

        Ok(ScoreChunk {
            header,
            entries,
            frame_intervals,
            frame_data,
        })
    }

    /// Analyze score entries beyond Entry[0] for behavior attachment data
    fn analyze_behavior_attachment_entries(
        entries: &Vec<Vec<u8>>,
    ) -> Result<Vec<(FrameIntervalPrimary, Option<FrameIntervalSecondary>)>, String> {
        let mut results = vec![];
        let mut i = 2; // Start at 2, skip entries 0 and 1

        log_i(&format!("🔍 Starting to analyze {} entries", entries.len()));

        while i < entries.len() {
            let entry_bytes = &entries[i];

            if entry_bytes.is_empty() {
                i += 1;
                continue;
            }

            match entry_bytes.len() {
                44 => {
                    // Primary entry
                    let mut reader = BinaryReader::from_u8(entry_bytes);
                    reader.set_endian(Endian::Big);

                    if let Ok(primary) = FrameIntervalPrimary::read(&mut reader) {
                        log_i(&format!(
                            "🎯 Found primary at entry {}: channel={}, frames={}-{}",
                            i, primary.channel_index, primary.start_frame, primary.end_frame
                        ));

                        // Look ahead to collect ALL secondary entries for this primary
                        let mut secondaries = Vec::new();
                        let mut j = i + 1;

                        // Keep reading secondary entries until we hit a non-secondary entry
                        while j < entries.len() {
                            let next_size = entries[j].len();

                            log_i(&format!("  🔎 Checking entry {} (size={})", j, next_size));

                            // Check if this could be a behavior entry
                            // Pattern: 8 bytes per behavior (cast_lib u16, cast_member u16, unk0 u32)
                            if next_size >= 8 && next_size % 8 == 0 {
                                let behavior_count = next_size / 8;
                                let mut sec_reader = BinaryReader::from_u8(&entries[j]);
                                sec_reader.set_endian(Endian::Big);

                                log_i(&format!(
                                    "  📦 Entry {} has {} bytes = {} potential behaviors",
                                    j, next_size, behavior_count
                                ));

                                let mut found_valid_behavior = false;

                                // Read all behaviors from this entry
                                for behavior_idx in 0..behavior_count {
                                    if let Ok(cast_lib) = sec_reader.read_u16() {
                                        if let Ok(cast_member) = sec_reader.read_u16() {
                                            if let Ok(unk0) = sec_reader.read_u32() {
                                                // Only add if it looks like a valid behavior reference
                                                if cast_lib > 0 && cast_member > 0 {
                                                    let mut secondary = FrameIntervalSecondary {
                                                        cast_lib,
                                                        cast_member,
                                                        unk0,
                                                        parameter: vec![],
                                                    };

                                                    // Handle parameters
                                                    if secondary.unk0 > 0
                                                        && (secondary.unk0 as usize) < entries.len()
                                                    {
                                                        let proplist_idx = secondary.unk0 as usize;
                                                        if let Ok(proplist_string) =
                                                            String::from_utf8(
                                                                entries[proplist_idx].clone(),
                                                            )
                                                        {
                                                            let clean = proplist_string
                                                                .trim_end_matches('\0');
                                                            if clean.starts_with('[') {
                                                                let player = unsafe {
                                                                    PLAYER_OPT.as_mut().unwrap()
                                                                };
                                                                // TODO: Replace `eval_lingo` with a parser
                                                                if let Ok(proplist) = eval_lingo(
                                                                    clean.to_owned(),
                                                                    player,
                                                                ) {
                                                                    secondary
                                                                        .parameter
                                                                        .push(proplist);
                                                                }
                                                            }
                                                        }
                                                    }

                                                    log_i(&format!(
                                                        "    ✅ Behavior {}: cast={}/{}, unk0={}",
                                                        behavior_idx + 1,
                                                        cast_lib,
                                                        cast_member,
                                                        unk0
                                                    ));
                                                    secondaries.push(secondary);
                                                    found_valid_behavior = true;
                                                } else {
                                                    log_i(&format!("    ⏭️ Skipping invalid behavior {}: cast={}/{}", 
                            behavior_idx + 1, cast_lib, cast_member));
                                                }
                                            }
                                        }
                                    }
                                }

                                if found_valid_behavior {
                                    j += 1;
                                } else {
                                    break;
                                }
                            } else {
                                log_i(&format!("  ⏹️ Not a behavior entry size, stopping"));
                                break; // Not a behavior entry size
                            }
                        }

                        log_i(&format!(
                            "📊 Primary for channel {} has {} behaviors total",
                            primary.channel_index,
                            secondaries.len()
                        ));

                        // Create a separate result entry for EACH secondary
                        if secondaries.is_empty() {
                            results.push((primary, None));
                        } else {
                            for secondary in secondaries {
                                results.push((primary.clone(), Some(secondary)));
                            }
                        }

                        // Skip all the secondary entries we processed
                        i = j;
                        continue;
                    }
                }
                _ => {
                    // Skip other entry types
                }
            }

            i += 1;
        }

        log_i(&format!(
            "🏁 Finished analyzing. Created {} results",
            results.len()
        ));
        Ok(results)
    }
}
// Frame Labels

#[derive(Clone)]
pub struct FrameLabel {
    pub frame_num: i32,
    pub label: String,
}

pub struct FrameLabelsChunk {
    pub labels: Vec<FrameLabel>,
}

impl FrameLabelsChunk {
    pub fn from_reader(
        reader: &mut BinaryReader,
        _dir_version: u16,
    ) -> Result<FrameLabelsChunk, String> {
        reader.set_endian(binary_reader::Endian::Big);

        let labels_count = reader
            .read_u16()
            .map_err(|e| format!("Error reading labels_count: {:?}", e))?
            as usize;
        let label_frames: Vec<(usize, usize)> = (0..labels_count)
            .map(|_i| {
                let frame_num = reader
                    .read_u16()
                    .map_err(|e| format!("Error reading frame_num: {:?}", e))?
                    as usize;
                let label_offset = reader
                    .read_u16()
                    .map_err(|e| format!("Error reading label_offset: {:?}", e))?
                    as usize;
                Ok((label_offset, frame_num))
            })
            .collect::<Result<Vec<_>, String>>()?;

        let labels_size: usize = reader
            .read_u32()
            .map_err(|e| format!("Error reading labels_size: {:?}", e))?
            as usize;
        let labels: Vec<FrameLabel> = (0..labels_count)
            .map(|i| {
                let (label_offset, frame_num) = label_frames[i];
                let label_len = if i < labels_count - 1 {
                    label_frames[i + 1].0 - label_offset
                } else {
                    labels_size - label_offset
                };
                let label_str = reader
                    .read_string(label_len)
                    .map_err(|e| format!("Error reading label_str: {:?}", e))?;
                // info!("label: {}", label_str);
                Ok(FrameLabel {
                    frame_num: frame_num as i32,
                    label: label_str.to_string(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(FrameLabelsChunk { labels })
    }
}
