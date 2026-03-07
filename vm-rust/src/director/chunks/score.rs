use binary_reader::{BinaryReader, Endian};
use log::{debug, error, warn};

use crate::{io::reader::DirectorExt, utils::log_i};

use crate::player::datum_ref::DatumRef;
use crate::player::eval::eval_lingo_expr_static;

use web_sys;
use web_sys::console;

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
    /// In D6+, this is the high 16 bits of spriteListIdx (uint32)
    /// spriteListIdx indexes into sprite detail offsets for behaviors
    pub sprite_list_idx_hi: u16,
    /// In D6+, this is the low 16 bits of spriteListIdx
    pub sprite_list_idx_lo: u16,
    pub pos_y: i16,
    pub pos_x: i16,
    pub height: u16,
    pub width: u16,
    pub color_flag: u8,
    pub fore_color_g: u8,
    pub back_color_g: u8,
    pub fore_color_b: u8,
    pub back_color_b: u8,
    pub blend: u8,
    pub rotation: f64,
    pub skew: f64,
    pub moveable: bool,
    pub editable: bool,
    pub trails: bool,
}

impl ScoreFrameChannelData {
    /// Get the full 32-bit spriteListIdx value (D6+)
    /// This indexes into sprite detail offsets for behavior attachment
    pub fn sprite_list_idx(&self) -> u32 {
        ((self.sprite_list_idx_hi as u32) << 16) | (self.sprite_list_idx_lo as u32)
    }
}

impl ScoreFrameChannelData {
    pub fn read_with_size(reader: &mut BinaryReader, sprite_record_size: u16) -> Result<ScoreFrameChannelData, String> {
        let start_pos = reader.pos;
        let sz = sprite_record_size as usize;

        // Core fields: 20 bytes (always present for D5+)
        let sprite_type = reader
            .read_u8()
            .map_err(|e| format!("Failed to read sprite_type: {:?}", e))?;
        let raw_ink_byte = reader
            .read_u8()
            .map_err(|e| format!("Failed to read ink: {:?}", e))?;
        let raw_ink = raw_ink_byte & 0x3f;
        let trails = (raw_ink_byte & 0x40) != 0;
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
        let sprite_list_idx_hi = reader
            .read_u16()
            .map_err(|e| format!("Failed to read sprite_list_idx_hi: {:?}", e))?;
        let sprite_list_idx_lo = reader
            .read_u16()
            .map_err(|e| format!("Failed to read sprite_list_idx_lo: {:?}", e))?;
        let pos_y = reader
            .read_u16()
            .map_err(|e| format!("Failed to read pos_y: {:?}", e))? as i16;
        let pos_x = reader
            .read_u16()
            .map_err(|e| format!("Failed to read pos_x: {:?}", e))? as i16;
        let height = reader
            .read_u16()
            .map_err(|e| format!("Failed to read height: {:?}", e))?;
        let width = reader
            .read_u16()
            .map_err(|e| format!("Failed to read width: {:?}", e))?;
        // 20 bytes read

        // Extended fields: only present if sprite_record_size > 20
        let mut color_flag: u8 = 0;
        let mut blend_raw: u8 = 0;
        let mut fore_color_g: u8 = 0;
        let mut back_color_g: u8 = 0;
        let mut fore_color_b: u8 = 0;
        let mut back_color_b: u8 = 0;
        let mut rotation_angle: f64 = 0.0;
        let mut skew_angle: f64 = 0.0;

        let mut moveable = false;
        let mut editable = false;

        if sz >= 22 {
            let unk3 = reader.read_u8()
                .map_err(|e| format!("Failed to read unk3: {:?}", e))?;
            color_flag = (unk3 >> 4) & 0x03;  // bits 4-5 only (bits 6-7 are editable/moveable)
            editable = (unk3 & 0x40) != 0;  // bit 6
            moveable = (unk3 & 0x80) != 0;  // bit 7
            blend_raw = reader.read_u8()
                .map_err(|e| format!("Failed to read blend: {:?}", e))?;
        }

        if sz >= 24 {
            let _unk5 = reader.read_u8()
                .map_err(|e| format!("Failed to read unk5: {:?}", e))?;
            let _unk6 = reader.read_u8()
                .map_err(|e| format!("Failed to read unk6: {:?}", e))?;
        }

        // D8+ extended color fields
        if sz >= 28 {
            fore_color_g = reader.read_u8()
                .map_err(|e| format!("Failed to read fore_color_g: {:?}", e))?;
            back_color_g = reader.read_u8()
                .map_err(|e| format!("Failed to read back_color_g: {:?}", e))?;
            fore_color_b = reader.read_u8()
                .map_err(|e| format!("Failed to read fore_color_b: {:?}", e))?;
            back_color_b = reader.read_u8()
                .map_err(|e| format!("Failed to read back_color_b: {:?}", e))?;
        }

        // D8+ rotation/skew fields
        if sz >= 36 {
            let _unk7 = reader.read_u16()
                .map_err(|e| format!("Failed to read unk7: {:?}", e))?;
            let rotation_raw = reader.read_u16()
                .map_err(|e| format!("Failed to read rotation: {:?}", e))? as i16;
            if rotation_raw != 0 {
                rotation_angle = rotation_raw as f64 / 100.0;
            }
            let _unk8 = reader.read_u16()
                .map_err(|e| format!("Failed to read unk8: {:?}", e))?;
            let skew_raw = reader.read_u16()
                .map_err(|e| format!("Failed to read skew: {:?}", e))? as i16;
            if skew_raw != 0 {
                skew_angle = skew_raw as f64 / 100.0;
            }
        }

        Ok(ScoreFrameChannelData {
            sprite_type,
            ink: raw_ink,
            fore_color,
            back_color,
            cast_lib,
            cast_member,
            sprite_list_idx_hi,
            sprite_list_idx_lo,
            pos_y,
            pos_x,
            height,
            width,
            color_flag,
            fore_color_g,
            back_color_g,
            fore_color_b,
            back_color_b,
            blend: blend_raw,
            rotation: rotation_angle,
            skew: skew_angle,
            moveable,
            editable,
            trails,
        })
    }

    /// Read a D5 sprite record (frames_version <= 7, sprite_record_size=24).
    /// D5 layout readSpriteDataD5():
    ///   [0]:    spriteType (u8)
    ///   [1]:    inkData (u8: bits 0-5=ink, bit 6=trails, bit 7=stretch)
    ///   [2-3]:  castLib (i16 BE)
    ///   [4-5]:  castId/member (u16 BE)
    ///   [6-7]:  scriptId.castLib (i16 BE)
    ///   [8-9]:  scriptId.member (u16 BE)
    ///   [10]:   foreColor (u8)
    ///   [11]:   backColor (u8)
    ///   [12-13]: startPoint.y / posY (i16 BE)
    ///   [14-15]: startPoint.x / posX (i16 BE)
    ///   [16-17]: height (i16 BE)
    ///   [18-19]: width (i16 BE)
    ///   [20]:   colorcode (u8: bit 6=editable, bit 7=moveable)
    ///   [21]:   blendAmount (u8)
    ///   [22]:   thickness (u8)
    ///   [23]:   unused
    pub fn read_d5(reader: &mut BinaryReader, _sprite_record_size: u16) -> Result<ScoreFrameChannelData, String> {
        let sprite_type = reader.read_u8()
            .map_err(|e| format!("D5: spriteType: {:?}", e))?;          // byte 0
        let ink_data = reader.read_u8()
            .map_err(|e| format!("D5: inkData: {:?}", e))?;             // byte 1
        let ink_val = ink_data & 0x3f;
        let trails = (ink_data & 0x40) != 0;
        let cast_lib = reader.read_u16()
            .map_err(|e| format!("D5: castLib: {:?}", e))? as i16;      // bytes 2-3
        let cast_member = reader.read_u16()
            .map_err(|e| format!("D5: castId: {:?}", e))?;              // bytes 4-5
        let script_cast_lib = reader.read_u16()
            .map_err(|e| format!("D5: scriptCastLib: {:?}", e))?;       // bytes 6-7
        let script_member = reader.read_u16()
            .map_err(|e| format!("D5: scriptMember: {:?}", e))?;        // bytes 8-9
        let fore_color = reader.read_u8()
            .map_err(|e| format!("D5: foreColor: {:?}", e))?;           // byte 10
        let back_color = reader.read_u8()
            .map_err(|e| format!("D5: backColor: {:?}", e))?;           // byte 11
        let pos_y = reader.read_u16()
            .map_err(|e| format!("D5: posY: {:?}", e))? as i16;         // bytes 12-13
        let pos_x = reader.read_u16()
            .map_err(|e| format!("D5: posX: {:?}", e))? as i16;         // bytes 14-15
        let height = reader.read_u16()
            .map_err(|e| format!("D5: height: {:?}", e))?;              // bytes 16-17
        let width = reader.read_u16()
            .map_err(|e| format!("D5: width: {:?}", e))?;               // bytes 18-19
        let colorcode = reader.read_u8()
            .map_err(|e| format!("D5: colorcode: {:?}", e))?;           // byte 20
        let editable = (colorcode & 0x40) != 0;  // bit 6
        let moveable = (colorcode & 0x80) != 0;  // bit 7
        let blend_raw = reader.read_u8()
            .map_err(|e| format!("D5: blend: {:?}", e))?;               // byte 21
        let _thickness = reader.read_u8()
            .map_err(|e| format!("D5: thickness: {:?}", e))?;           // byte 22
        let _unused = reader.read_u8()
            .map_err(|e| format!("D5: unused: {:?}", e))?;              // byte 23

        Ok(ScoreFrameChannelData {
            sprite_type,
            ink: ink_val,
            fore_color,
            back_color,
            cast_lib: cast_lib as u16,
            cast_member,
            sprite_list_idx_hi: script_cast_lib,
            sprite_list_idx_lo: script_member,
            pos_y,
            pos_x,
            height,
            width,
            color_flag: 0,
            fore_color_g: 0,
            back_color_g: 0,
            fore_color_b: 0,
            back_color_b: 0,
            blend: blend_raw,
            rotation: 0.0,
            skew: 0.0,
            moveable,
            editable,
            trails,
        })
    }

    pub fn read(reader: &mut BinaryReader) -> Result<ScoreFrameChannelData, String> {
        // Legacy: assume large record size for backward compat with afterburner D8+ files
        Self::read_with_size(reader, 36)
    }

    fn decode_scaled(value: u8, scale: u8) -> u8 {
        value / scale
    }
}

#[derive(Clone, Debug)]
pub struct ScoreFrameData {
    pub header: ScoreFrameDataHeader,
    pub decompressed_data: Vec<u8>,
    pub frame_channel_data: Vec<(u32, u16, ScoreFrameChannelData)>,
    pub sound_channel_data: Vec<(u32, u16, SoundChannelData)>,
    pub tempo_channel_data: Vec<(u32, TempoChannelData)>,
    pub palette_channel_data: Vec<(u32, i16, i16)>,
}

impl Default for ScoreFrameData {
    fn default() -> Self {
        Self {
            header: ScoreFrameDataHeader::default(),
            decompressed_data: Vec::new(),
            frame_channel_data: Vec::new(),
            sound_channel_data: Vec::new(),
            tempo_channel_data: Vec::new(),
            palette_channel_data: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScoreFrameDataHeader {
    pub frame_count: u32,
    pub sprite_record_size: u16,
    pub num_channels: u16,
    pub frames_version: u16,
}

#[derive(Clone, Debug)]
pub struct SoundChannelData {
    pub cast_member: u8,
}

impl SoundChannelData {
    pub fn read(reader: &mut BinaryReader) -> Result<SoundChannelData, String> {
        let _unk0 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read unk0: {:?}", e))?;
        let _unk1 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read unk1: {:?}", e))?;
        let _unk2 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read unk2: {:?}", e))?;
        let cast_member = reader
            .read_u8()
            .map_err(|e| format!("Failed to read cast_member: {:?}", e))?;
        
        Ok(SoundChannelData {
            cast_member,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TempoChannelData {
    pub tempo: u8,            // Byte 6: tempo mode/value (D6+: 246=FPS, 247=delay, 248=wait click, etc.)
    pub tempo_cue_point: u16, // Bytes 4-5: FPS value or delay (when tempo==246 or 247)
    pub flags1: u8,           // Byte 0
    pub flags2: u8,           // Byte 1
    pub unk3: u8,             // Byte 2
    pub unk4: u8,             // Byte 3
    pub color_tempo: u8,      // Byte 7
    pub wait_flags: u16,      // Bytes 8-9
    pub channel_flags: u16,   // Bytes 10-11
    pub frame_data: u16,      // Bytes 18-19
}

impl TempoChannelData {
    pub fn read(reader: &mut BinaryReader) -> Result<TempoChannelData, String> {
        // Bytes 0-3: tempoSpriteListIdx (u32) - split into individual bytes for compatibility
        let flags1 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read tempo flags1: {:?}", e))?;
        let flags2 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read tempo flags2: {:?}", e))?;
        let unk3 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read tempo unk3: {:?}", e))?;
        let unk4 = reader
            .read_u8()
            .map_err(|e| format!("Failed to read tempo unk4: {:?}", e))?;

        // Bytes 4-5: tempoCuePoint (u16) - FPS value when tempo==246, delay when tempo==247
        let tempo_cue_point = reader
            .read_u16()
            .map_err(|e| format!("Failed to read tempo_cue_point: {:?}", e))?;

        // Byte 6: tempo mode byte (D6+: 246=FPS, 247=delay, 248=wait click, etc.)
        let tempo = reader
            .read_u8()
            .map_err(|e| format!("Failed to read tempo: {:?}", e))?;

        // Byte 7: colorTempo
        let color_tempo = reader
            .read_u8()
            .map_err(|e| format!("Failed to read color_tempo: {:?}", e))?;
        
        // Bytes 8-9 - Wait flags
        let wait_flags = reader
            .read_u16()
            .map_err(|e| format!("Failed to read tempo wait_flags: {:?}", e))?;
        
        // Bytes 10-11 - Channel flags
        let channel_flags = reader
            .read_u16()
            .map_err(|e| format!("Failed to read tempo channel_flags: {:?}", e))?;
        
        // Skip bytes 12-17
        for i in 0..6 {
            reader
                .read_u8()
                .map_err(|e| format!("Failed to read tempo skip{}: {:?}", i + 4, e))?;
        }
        
        // Bytes 18-19 - Frame-specific data
        let frame_data = reader
            .read_u16()
            .map_err(|e| format!("Failed to read tempo frame_data: {:?}", e))?;
        
        Ok(TempoChannelData {
            tempo,
            tempo_cue_point,
            flags1,
            flags2,
            unk3,
            unk4,
            color_tempo,
            wait_flags,
            channel_flags,
            frame_data,
        })
    }
    
    pub fn is_default(&self) -> bool {
        // Check if this is a "no change" marker (0xff 0xfe pattern)
        self.flags1 == 0xff && self.flags2 == 0xfe
    }
    
    pub fn is_empty(&self) -> bool {
        // Check if all fields are zero (no tempo data)
        self.flags1 == 0 && self.flags2 == 0 && self.tempo == 0
    }
}

impl ScoreFrameData {
    #[allow(unused_variables)]
    pub fn read(reader: &mut BinaryReader) -> Result<ScoreFrameData, String> {
        let mut header = Self::read_header(reader)?;

        // "numOfFrames in the header is often incorrect"
        // Pre-scan the frame delta stream to count actual frames
        let frame_data_start = reader.pos;
        let mut actual_frame_count: u32 = 0;
        while !reader.eof() {
            match reader.read_u16() {
                Ok(length) => {
                    if length == 0 { break; }
                    let skip = (length as usize).saturating_sub(2);
                    if reader.pos + skip > reader.length { break; }
                    reader.jmp(reader.pos + skip);
                    actual_frame_count += 1;
                }
                Err(_) => break,
            }
        }
        reader.jmp(frame_data_start);

        if header.frame_count == 0 || header.frame_count != actual_frame_count {
            console::log_1(&format!(
                "ScoreFrameData: header frame_count={} but actual_frame_count={}, using actual",
                header.frame_count, actual_frame_count
            ).into());
            header.frame_count = actual_frame_count;
        }

        debug!(
            "ScoreFrameData {} {} {}",
            header.frame_count, header.num_channels, header.sprite_record_size
        );

        let mut channel_data = vec![
            0u8;
            (header.frame_count as usize)
                * (header.num_channels as usize)
                * (header.sprite_record_size as usize)
        ];

        let frame_size = (header.num_channels as usize) * (header.sprite_record_size as usize);

        let mut frame_index: u32 = 0;
        while !reader.eof() && frame_index < header.frame_count {
            let length = reader
                .read_u16()
                .map_err(|e| format!("Failed to read frame length: {:?}", e))?;

            if length == 0 {
                break;
            }

            // Copy entire previous frame first (carry-forward).
            // Deltas will overwrite only the bytes that changed.
            // This is correct for both D5 (48-byte main channels) and D6+ (uniform channels).
            if frame_index > 0 {
                let prev_frame_offset = ((frame_index - 1) as usize) * frame_size;
                let curr_frame_offset = (frame_index as usize) * frame_size;
                channel_data.copy_within(
                    prev_frame_offset..prev_frame_offset + frame_size,
                    curr_frame_offset,
                );
            }

            let frame_length = length - 2;
            if frame_length > 0 {
                let chunk_data = reader
                    .read_bytes(frame_length as usize)
                    .map_err(|e| format!("Failed to read chunk data: {:?}", e))?;
                let mut frame_chunk_reader = BinaryReader::from_u8(chunk_data);
                frame_chunk_reader.set_endian(Endian::Big);

                // Apply deltas on top of carried-forward data.
                // Delta stream: (channel_size: u16, channel_offset: u16, data: [u8; channel_size])
                // channel_offset is a raw byte offset within the frame buffer.
                while !frame_chunk_reader.eof() {
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

                    let frame_offset = (frame_index as usize) * frame_size;
                    let end_offset = frame_offset + channel_offset + channel_size;
                    if end_offset > channel_data.len() {
                        error!("Channel data copy out of bounds. Frame offset: {}, Channel offset: {}, Channel size: {}, Total len: {}",
                            frame_offset, channel_offset, channel_size, channel_data.len());
                        return Err("Channel data copy out of bounds".to_string());
                    }
                    channel_data[frame_offset + channel_offset..end_offset]
                        .copy_from_slice(&channel_delta);
                }
            }
            frame_index = frame_index + 1;
        }

        // D4/D5 (frames_version <= 7): first 48 bytes are packed main channels
        // (script, sound types, transition, tempo, sounds, palette, etc.),
        // followed by sprite channels at sprite_record_size bytes each.
        // D6+ (frames_version > 7): all channels are uniform at sprite_record_size each.
        let main_channels_size: usize = if header.frames_version <= 7 { 48 } else { 0 };
        let is_d5 = main_channels_size > 0;

        let (decompressed_data, frame_channel_data, sound_channel_data, tempo_channel_data, palette_channel_data) = {
            let mut frame_channel_data = vec![];
            let mut sound_channel_data = vec![];
            let mut tempo_channel_data = vec![];
            let mut palette_channel_data: Vec<(u32, i16, i16)> = vec![];
            let decompressed_data = channel_data;
            let mut channel_reader = BinaryReader::from_vec(&decompressed_data);
            channel_reader.set_endian(Endian::Big);

            for frame_index in 0..header.frame_count {
                let frame_start = (frame_index as usize) * frame_size;

                if is_d5 {
                    // D5: Main channels packed in first 48 bytes
                    // Layout: actionId.castLib(2) actionId.member(2)
                    //         sound1.castLib(2) sound1.member(2)
                    //         sound2.castLib(2) sound2.member(2)
                    //         trans.castLib(2) trans.member(2)
                    //         colorTempo(1) colorSound1(1) colorSound2(1) colorScript(1) colorTrans(1)
                    //         tempo(1) unk(2)
                    //         palette(24 bytes)

                    // Frame script (actionId) at bytes 0-3: castLib(2) + member(2)
                    channel_reader.jmp(frame_start);
                    let action_cast_lib = channel_reader.read_u16().unwrap_or(0);
                    let action_member = channel_reader.read_u16().unwrap_or(0);
                    if action_member != 0 {
                        // Store frame script as channel 0
                        frame_channel_data.push((frame_index, 0_u16, ScoreFrameChannelData {
                            sprite_type: 0,
                            ink: 0,
                            fore_color: 0,
                            back_color: 0,
                            cast_lib: action_cast_lib,
                            cast_member: action_member,
                            sprite_list_idx_hi: 0,
                            sprite_list_idx_lo: 0,
                            pos_y: 0,
                            pos_x: 0,
                            height: 0,
                            width: 0,
                            color_flag: 0,
                            fore_color_g: 0,
                            back_color_g: 0,
                            fore_color_b: 0,
                            back_color_b: 0,
                            blend: 0,
                            rotation: 0.0,
                            skew: 0.0,
                            moveable: false,
                            editable: false,
                            trails: false,
                        }));
                    }

                    // Tempo at byte 21
                    channel_reader.jmp(frame_start + 21);
                    let tempo_val = channel_reader.read_u8().unwrap_or(0);
                    if tempo_val > 0 {
                        tempo_channel_data.push((frame_index, TempoChannelData {
                            tempo: tempo_val,
                            tempo_cue_point: 0,
                            flags1: 0,
                            flags2: 0,
                            unk3: 0,
                            unk4: 0,
                            color_tempo: 0,
                            wait_flags: 0,
                            channel_flags: 0,
                            frame_data: 0,
                        }));
                    }

                    // Sound 1: castLib at bytes 4-5, member at bytes 6-7
                    channel_reader.jmp(frame_start + 6);
                    let sound1_member = channel_reader.read_u16().unwrap_or(0);
                    if sound1_member != 0 {
                        debug!("Sound 1 in frame {}: cast_member={}", frame_index, sound1_member);
                        sound_channel_data.push((frame_index, 3, SoundChannelData {
                            cast_member: sound1_member as u8,
                        }));
                    }

                    // Sound 2: castLib at bytes 8-9, member at bytes 10-11
                    channel_reader.jmp(frame_start + 10);
                    let sound2_member = channel_reader.read_u16().unwrap_or(0);
                    if sound2_member != 0 {
                        debug!("Sound 2 in frame {}: cast_member={}", frame_index, sound2_member);
                        sound_channel_data.push((frame_index, 4, SoundChannelData {
                            cast_member: sound2_member as u8,
                        }));
                    }

                    // Palette: castLib at bytes 24-25, member at bytes 26-27
                    channel_reader.jmp(frame_start + 24);
                    let palette_cast_lib = channel_reader.read_i16().unwrap_or(0);
                    let palette_member = channel_reader.read_i16().unwrap_or(0);
                    if palette_cast_lib != 0 || palette_member != 0 {
                        palette_channel_data.push((frame_index, palette_cast_lib, palette_member));
                    }

                    // Sprite channels start at byte 48 within the frame
                    let num_sprites = (frame_size - main_channels_size) / (header.sprite_record_size as usize);
                    for sprite_idx in 0..num_sprites {
                        let channel_index = (sprite_idx + 6) as u16;
                        let pos = frame_start + main_channels_size + sprite_idx * (header.sprite_record_size as usize);

                        channel_reader.jmp(pos);
                        let data = ScoreFrameChannelData::read_d5(&mut channel_reader, header.sprite_record_size)?;

                        let has_sprite_data = data.cast_member != 0
                            || data.blend != 0
                            || data.width != 0
                            || data.height != 0
                            || data.pos_x != 0
                            || data.pos_y != 0
                            || data.ink != 0
                            || data.sprite_type != 0;

                        if has_sprite_data {
                            debug!("frame_index={frame_index} channel_index={channel_index} cast_member={} sprite_type={} ink={} pos_y={} pos_x={} height={} width={} blend={}",
                                data.cast_member, data.sprite_type, data.ink, data.pos_y, data.pos_x, data.height, data.width, data.blend);
                            frame_channel_data.push((frame_index, channel_index, data));
                        }
                    }
                } else {
                    // D6+: All channels use sprite_record_size bytes each
                    for channel_index in 0..header.num_channels {
                        let pos = frame_start + (channel_index as usize) * (header.sprite_record_size as usize);
                        channel_reader.jmp(pos);

                        if channel_index == 0 || channel_index == 2 {
                            // Channel 0 = Script, Channel 2 = Transition (skip)
                        } else if channel_index == 1 {
                            // Channel 1 = Tempo
                            let tempo_data = TempoChannelData::read(&mut channel_reader)?;
                            if !tempo_data.is_default() && !tempo_data.is_empty() {
                                debug!("Frame {} Tempo: fps={}", frame_index, tempo_data.tempo);
                                tempo_channel_data.push((frame_index, tempo_data));
                            }
                        } else if channel_index == 3 || channel_index == 4 {
                            // Channel 3 = Sound2, Channel 4 = Sound1
                            let sound_data = SoundChannelData::read(&mut channel_reader)?;
                            if sound_data.cast_member != 0 {
                                debug!("Sound {} in frame {}: cast_member={}",
                                    channel_index - 2, frame_index, sound_data.cast_member);
                                sound_channel_data.push((frame_index, channel_index, sound_data));
                            }
                        } else if channel_index == 5 {
                            // Channel 5 = Palette: castLib at bytes 0-1, member at bytes 2-3
                            let palette_cast_lib = channel_reader.read_i16()
                                .map_err(|e| format!("Failed to read palette castLib: {:?}", e))?;
                            let palette_member = channel_reader.read_i16()
                                .map_err(|e| format!("Failed to read palette member: {:?}", e))?;
                            if palette_cast_lib != 0 || palette_member != 0 {
                                palette_channel_data.push((frame_index, palette_cast_lib, palette_member));
                            }
                        } else {
                            let data = ScoreFrameChannelData::read_with_size(&mut channel_reader, header.sprite_record_size)?;

                            let has_sprite_data = data.cast_member != 0
                                || data.rotation != 0.0
                                || data.skew != 0.0
                                || data.blend != 0
                                || data.width != 0
                                || data.height != 0
                                || data.pos_x != 0
                                || data.pos_y != 0
                                || data.ink != 0
                                || data.sprite_type != 0
                                || data.color_flag != 0
                                || data.fore_color != 0
                                || data.fore_color_g != 0
                                || data.fore_color_b != 0
                                || data.back_color != 0
                                || data.back_color_g != 0
                                || data.back_color_b != 0;

                            if has_sprite_data {
                                debug!("frame_index={frame_index} channel_index={channel_index} cast_lib={} cast_member={} sprite_type={} ink={} fore_color={} back_color={} pos_y={} pos_x={} height={} width={} blend={}",
                                    data.cast_lib, data.cast_member, data.sprite_type, data.ink, data.fore_color, data.back_color, data.pos_y, data.pos_x, data.height, data.width, data.blend);
                                frame_channel_data.push((frame_index, channel_index, data));
                            }
                        }
                    }
                }
            }

            console::log_1(&format!(
                "🏁 Finished processing {} frames. Sprites: {}, Sounds: {}, Tempo changes: {}, Palette changes: {}",
                header.frame_count, frame_channel_data.len(), sound_channel_data.len(), tempo_channel_data.len(), palette_channel_data.len()
            ).into());

            (decompressed_data, frame_channel_data, sound_channel_data, tempo_channel_data, palette_channel_data)
        };

        Ok(ScoreFrameData {
            header,
            decompressed_data,
            frame_channel_data,
            sound_channel_data,
            tempo_channel_data,
            palette_channel_data,
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

        console::log_1(&format!(
            "ScoreFrameData::read_header: actual_length={}, unk1(frame1Offset)={}, frame_count={}, frames_version={}, sprite_record_size={}, num_channels={}, reader_len={}",
            actual_length, unk1, frame_count, frames_version, sprite_record_size, num_channels, reader.length
        ).into());

        Ok(ScoreFrameDataHeader {
            frame_count,
            sprite_record_size,
            num_channels,
            frames_version,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct TweenInfo {
    pub curvature: u32,
    pub flags: u32,
    pub ease_in: u32,
    pub ease_out: u32,
    pub padding: u32,
}

impl TweenInfo {
    pub fn read(reader: &mut BinaryReader) -> Result<TweenInfo, String> {
        Ok(TweenInfo {
            curvature: reader.read_u32()
                .map_err(|e| format!("Failed to read tween curvature: {:?}", e))?,
            flags: reader.read_u32()
                .map_err(|e| format!("Failed to read tween flags: {:?}", e))?,
            ease_in: reader.read_u32()
                .map_err(|e| format!("Failed to read tween ease_in: {:?}", e))?,
            ease_out: reader.read_u32()
                .map_err(|e| format!("Failed to read tween ease_out: {:?}", e))?,
            padding: reader.read_u32()
                .map_err(|e| format!("Failed to read tween padding: {:?}", e))?,
        })
    }
       
    pub fn is_path_tweened(&self) -> bool {
        (self.flags & 0x00000004) != 0  // Bit 2
    }
    
    pub fn is_size_tweened(&self) -> bool {
        (self.flags & 0x00000008) != 0  // Bit 3
    }
    
    pub fn is_forecolor_tweened(&self) -> bool {
        (self.flags & 0x00000010) != 0  // Bit 4
    }
    
    pub fn is_backcolor_tweened(&self) -> bool {
        (self.flags & 0x00000020) != 0  // Bit 5
    }
    
    pub fn is_blend_tweened(&self) -> bool {
        (self.flags & 0x00000040) != 0  // Bit 6
    }
    
    pub fn is_rotation_tweened(&self) -> bool {
        (self.flags & 0x00000080) != 0  // Bit 7
    }
    
    pub fn is_skew_tweened(&self) -> bool {
        (self.flags & 0x00000100) != 0  // Bit 8
    }
       
    pub fn is_continuous(&self) -> bool {
        // Continuous at endpoints (for circular paths)
        (self.flags & 0x00000002) != 0  // Bit 1
    }
    
    pub fn is_smooth_speed(&self) -> bool {
        // Smooth vs Sharp speed changes
        (self.flags & 0x00000400) != 0  // Bit 10
    }
}

/// FrameIntervalPrimary - sprite span information
#[derive(Clone, Debug)]
pub struct FrameIntervalPrimary {
    pub start_frame: u32,
    pub end_frame: u32,
    pub xtra_info: u32,
    pub sprite_flags: u32,
    pub channel_index: u32,
    pub tween_info: TweenInfo,
}

impl FrameIntervalPrimary {
    pub fn read(reader: &mut BinaryReader) -> Result<FrameIntervalPrimary, String> {
        let start_frame = reader.read_u32()
            .map_err(|e| format!("Failed to read start_frame: {:?}", e))?;
        let end_frame = reader.read_u32()
            .map_err(|e| format!("Failed to read end_frame: {:?}", e))?;
        let xtra_info = reader.read_u32()
            .map_err(|e| format!("Failed to read xtra_info: {:?}", e))?;
        let sprite_flags = reader.read_u32()
            .map_err(|e| format!("Failed to read sprite_flags: {:?}", e))?;
        let channel_index = reader.read_u32()
            .map_err(|e| format!("Failed to read channel_index: {:?}", e))?;
        
        // Read TweenInfo (20 bytes = 5 x u32)
        let tween_info = TweenInfo::read(reader)?;
        
        // Debug logging
        if tween_info.flags != 0 {
            debug!(
                "📊 Sprite Span: channel={} frames={}-{} tween_flags=0x{:08x} curvature={} ease_in={} ease_out={}",
                channel_index, start_frame, end_frame, 
                tween_info.flags, tween_info.curvature,
                tween_info.ease_in, tween_info.ease_out
            );
            
            debug!(
                "   Properties: path={} size={} rotation={} skew={} blend={} fg={} bg={}",
                tween_info.is_path_tweened(),
                tween_info.is_size_tweened(),
                tween_info.is_rotation_tweened(),
                tween_info.is_skew_tweened(),
                tween_info.is_blend_tweened(),
                tween_info.is_forecolor_tweened(),
                tween_info.is_backcolor_tweened()
            );
            
            debug!(
                "   Settings: smooth={} continuous={}",
                tween_info.is_smooth_speed(),
                tween_info.is_continuous()
            );
        }
        
        Ok(FrameIntervalPrimary {
            start_frame,
            end_frame,
            xtra_info,
            sprite_flags,
            channel_index,
            tween_info,
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

/// Behavior element parsed from sprite detail data (D6+)
#[derive(Clone, Debug)]
pub struct SpriteBehavior {
    pub cast_lib: u16,
    pub cast_member: u16,
    pub parameter: Vec<DatumRef>,
}

/// Sprite detail info parsed from sprite detail offset (D6+)
#[derive(Clone, Debug, Default)]
pub struct SpriteDetailInfo {
    pub behaviors: Vec<SpriteBehavior>,
}

#[derive(Clone)]
pub struct ScoreChunk {
    pub header: ScoreChunkHeader,
    pub entries: Vec<Vec<u8>>,
    pub frame_intervals: Vec<(FrameIntervalPrimary, Option<FrameIntervalSecondary>)>,
    pub frame_data: ScoreFrameData,
    /// Sprite detail offsets for D6+ behavior attachment
    /// Key is sprite_list_idx, value is the sprite detail info with behaviors
    pub sprite_details: std::collections::HashMap<u32, SpriteDetailInfo>,
}

impl ScoreChunk {
    #[allow(unused_variables)]
    pub fn read(reader: &mut BinaryReader, dir_version: u16, after_burned: bool) -> Result<Self, String> {
        // Use the same parsing logic for both afterburner and non-afterburner files.
        // The VWSC chunk format is the same regardless of container format.
        Self::read_standard(reader, dir_version)
    }

    /// Non-afterburner VWSC reading
    /// For D6+ (version >= 600): reads the sprite detail table header, then extracts entries.
    /// For D4/D5 (version >= 400): reads frame data directly from position 0.
    fn read_standard(reader: &mut BinaryReader, dir_version: u16) -> Result<Self, String> {
        reader.set_endian(Endian::Big);

        if dir_version >= 600 {
            // D6+ format: sprite detail table + frame data
            let frames_stream_size = reader.read_u32()
                .map_err(|e| format!("Failed to read framesStreamSize: {:?}", e))?;
            let ver = reader.read_u32()
                .map_err(|e| format!("Failed to read ver: {:?}", e))?;
            let list_start = reader.read_u32()
                .map_err(|e| format!("Failed to read listStart: {:?}", e))? as usize;

            reader.jmp(list_start);
            let num_entries = reader.read_u32()
                .map_err(|e| format!("Failed to read numEntries: {:?}", e))? as usize;
            let list_size = reader.read_u32()
                .map_err(|e| format!("Failed to read listSize: {:?}", e))? as usize;
            let max_data_len = reader.read_u32()
                .map_err(|e| format!("Failed to read maxDataLen: {:?}", e))? as usize;

            let index_start = list_start + 12; // After the 3 header u32s
            let frame_data_offset = index_start + list_size * 4; // After the offset table

            console::log_1(&format!(
                "VWSC D6+: framesStreamSize={}, ver={}, listStart=0x{:x}",
                frames_stream_size, ver, list_start
            ).into());
            console::log_1(&format!(
                "VWSC D6+: numEntries={}, listSize={}, maxDataLen=0x{:x}, indexStart={}, frameDataOffset={}",
                num_entries, list_size, max_data_len, index_start, frame_data_offset
            ).into());

            // Read raw offsets (numEntries of them, relative to frameDataOffset)
            let mut raw_offsets = Vec::with_capacity(num_entries);
            for _ in 0..num_entries {
                raw_offsets.push(reader.read_u32()
                    .map_err(|e| format!("Failed to read sprite detail offset: {:?}", e))? as usize);
            }

            // Extract entries using consecutive offset pairs
            // The last offset serves as sentinel for sizing the previous entry
            // So we get numEntries - 1 usable entries
            let entry_count = num_entries.saturating_sub(1);
            let mut entries = Vec::with_capacity(entry_count);

            for i in 0..entry_count {
                let entry_start = frame_data_offset + raw_offsets[i];
                let entry_end = frame_data_offset + raw_offsets[i + 1];

                if entry_start > reader.length || entry_end > reader.length {
                    console::warn_1(&format!(
                        "VWSC: Entry {} out of bounds: start={}, end={}, stream_len={}",
                        i, entry_start, entry_end, reader.length
                    ).into());
                    entries.push(Vec::new());
                    continue;
                }

                reader.jmp(entry_start);
                let len = entry_end.saturating_sub(entry_start);
                if len > 0 {
                    entries.push(reader.read_bytes(len)
                        .map_err(|e| format!("Failed to read entry {}: {:?}", i, e))?
                        .to_vec());
                } else {
                    entries.push(Vec::new());
                }
            }

            let entry_sizes: Vec<usize> = entries.iter().take(10).map(|e| e.len()).collect();
            console::log_1(&format!(
                "VWSC D6+: extracted {} entries, sizes: {:?}",
                entries.len(), entry_sizes
            ).into());

            // Entry[0] = frame data (the main score data with channel deltas)
            let frame_data = if !entries.is_empty() && !entries[0].is_empty() {
                let mut delta_reader = BinaryReader::from_vec(&entries[0]);
                delta_reader.set_endian(Endian::Big);
                ScoreFrameData::read(&mut delta_reader)?
            } else {
                ScoreFrameData::default()
            };

            let frame_intervals = Self::analyze_behavior_attachment_entries(&entries)?;
            let sprite_details = Self::parse_sprite_details_from_entries(&entries);

            Ok(ScoreChunk {
                header: ScoreChunkHeader::default(),
                entries,
                frame_intervals,
                frame_data,
                sprite_details,
            })
        } else if dir_version >= 400 {
            // D4/D5 format: frame data directly at position 0
            console::log_1(&format!(
                "VWSC D4/D5 standard: reading frame data directly (version {})",
                dir_version
            ).into());

            let frame_data = ScoreFrameData::read(reader)?;

            Ok(ScoreChunk {
                header: ScoreChunkHeader::default(),
                entries: vec![],
                frame_intervals: vec![],
                frame_data,
                sprite_details: std::collections::HashMap::new(),
            })
        } else {
            Err(format!("Unsupported Director version for VWSC: {}", dir_version))
        }
    }

    /// Parse sprite details directly from extracted VWSC entries.
    /// For spriteListIdx = N, entries[N] is the 44-byte sprite info,
    /// and entries[N+1] contains behaviors (8 bytes each: cast_lib u16, cast_member u16, initializer_idx u32).
    fn parse_sprite_details_from_entries(entries: &[Vec<u8>]) -> std::collections::HashMap<u32, SpriteDetailInfo> {
        let mut details = std::collections::HashMap::new();
        let mut behavior_count = 0;

        // For spriteListIdx = N, behaviors are at entries[N+1].
        // We try every possible N and parse entries[N+1] as behavior data.
        for idx in 1..entries.len().saturating_sub(1) {
            let behavior_data = &entries[idx + 1];

            // Behaviors are 8 bytes each, need at least one
            if behavior_data.len() < 8 || behavior_data.len() % 8 != 0 {
                continue;
            }

            let mut reader = BinaryReader::from_u8(behavior_data);
            reader.set_endian(Endian::Big);

            let mut info = SpriteDetailInfo::default();
            while reader.pos + 8 <= behavior_data.len() {
                let cast_lib = match reader.read_u16() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let cast_member = match reader.read_u16() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let initializer_idx = match reader.read_u32() {
                    Ok(v) => v,
                    Err(_) => break,
                };

                if cast_lib == 0 && cast_member == 0 {
                    break;
                }

                if cast_member > 0 && cast_member < 10000 {
                    let mut parameter = Vec::new();
                    // Parse initializer data from entries[initializer_idx]
                    if initializer_idx > 0 && (initializer_idx as usize) < entries.len() {
                        if let Ok(proplist_string) = String::from_utf8(entries[initializer_idx as usize].clone()) {
                            let clean = proplist_string.trim_end_matches('\0');
                            if clean.starts_with('[') {
                                match eval_lingo_expr_static(clean.to_owned()) {
                                    Ok(proplist) => {
                                        parameter.push(proplist);
                                    }
                                    Err(e) => {
                                        console::warn_1(&format!(
                                            "Failed to parse sprite detail initializer: {}", e.message
                                        ).into());
                                    }
                                }
                            }
                        }
                    }
                    info.behaviors.push(SpriteBehavior { cast_lib, cast_member, parameter });
                }
            }

            if !info.behaviors.is_empty() {
                behavior_count += info.behaviors.len();
                if details.len() < 30 {
                    let behavior_strs: Vec<String> = info.behaviors.iter()
                        .map(|b| if b.cast_lib == 65535 { format!("(-1)/{}", b.cast_member) } else { format!("{}/{}", b.cast_lib, b.cast_member) })
                        .collect();
                    console::log_1(&format!(
                        "sprite_details: spriteListIdx {} -> {} behaviors [{}]",
                        idx, info.behaviors.len(), behavior_strs.join(", ")
                    ).into());
                }
                details.insert(idx as u32, info);
            }
        }

        if !details.is_empty() {
            console::log_1(&format!(
                "Parsed {} sprite details with {} total behaviors from entries",
                details.len(), behavior_count
            ).into());
        }

        // Find all sprite detail entries for channel 40 (0x28)
        for i in 2..entries.len() {
            let e = &entries[i];
            if e.len() >= 40 {
                let channel = u32::from_be_bytes([e[16], e[17], e[18], e[19]]);
                if channel == 40 {
                    let sf = u32::from_be_bytes([e[0], e[1], e[2], e[3]]);
                    let ef = u32::from_be_bytes([e[4], e[5], e[6], e[7]]);
                    let next_size = if i + 1 < entries.len() { entries[i+1].len() } else { 0 };
                    let next_hex: String = if i + 1 < entries.len() {
                        entries[i+1].iter().take(16).map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ")
                    } else { String::new() };
                    console::log_1(&format!(
                        "CH40_ENTRY {}: size={} frames={}-{} next_size={} next_hex: {}",
                        i, e.len(), sf, ef, next_size, next_hex
                    ).into());
                }
            }
        }

        details
    }

    /// Parse sprite detail offsets from Entry[0] (D6+)
    ///
    /// - Bytes 0-3: framesStreamSize
    /// - Bytes 4-7: version
    /// - Bytes 8-11: listStart (position of sprite detail info)
    ///
    /// At listStart:
    /// - numEntries (u32): count of sprite detail offset entries
    /// - listSize (u32): size of the offset index (should equal numEntries)
    /// - maxDataLen (u32): max data size
    /// - Offset table: numEntries x u32 (relative offsets from frameDataOffset)
    ///
    /// The offsets point to sprite detail data. For a sprite with spriteListIdx = N:
    /// - Sprite info is at offset[N]
    /// - Behaviors are at offset[N+1]
    /// - Sprite name is at offset[N+2]
    fn parse_sprite_details(entries: &Vec<Vec<u8>>) -> std::collections::HashMap<u32, SpriteDetailInfo> {
        let mut details = std::collections::HashMap::new();

        // Log entry count for diagnosis
        debug!(
            "parse_sprite_details: {} entries, entry0 size: {}",
            entries.len(),
            if entries.is_empty() { 0 } else { entries[0].len() }
        );

        // Entry[0] contains the frames stream with sprite detail offsets embedded
        if entries.is_empty() || entries[0].len() < 12 {
            debug!("   Entry[0] missing or too small");
            return details;
        }

        let entry0 = &entries[0];
        let entry0_len = entry0.len();

        let mut reader = BinaryReader::from_u8(entry0);
        reader.set_endian(Endian::Big);

        // Read header: framesStreamSize, version, listStart
        let frames_stream_size = match reader.read_u32() {
            Ok(n) => n as usize,
            Err(_) => return details,
        };
        let version = match reader.read_u32() {
            Ok(n) => n,
            Err(_) => return details,
        };
        let list_start = match reader.read_u32() {
            Ok(n) => n as usize,
            Err(_) => return details,
        };

        debug!(
            "   framesStreamSize={} version={} listStart={}",
            frames_stream_size, version, list_start
        );

        // listStart of 0 means no sprite details present
        if list_start == 0 {
            debug!("   listStart=0, no sprite details");
            return details;
        }

        // Validate listStart - it should be within entry0 bounds
        // Note: listStart is an ABSOLUTE position in the stream. It can point anywhere
        // within Entry[0], including inside the frame data region.
        if list_start >= entry0_len {
            return details;
        }

        // Need enough space for the sprite detail header (3 x u32 = 12 bytes)
        if list_start + 12 > entry0_len {
            return details;
        }

        reader.jmp(list_start);

        // Read sprite detail header at listStart
        let num_entries = match reader.read_u32() {
            Ok(n) => n as usize,
            Err(_) => return details,
        };
        let list_size = match reader.read_u32() {
            Ok(n) => n as usize,
            Err(_) => return details,
        };
        let max_data_len = match reader.read_u32() {
            Ok(n) => n as usize,
            Err(_) => return details,
        };

        if num_entries == 0 {
            return details;
        }

        // Sanity check
        if num_entries > 100000 {
            debug!(
                "⚠️ Sprite detail numEntries {} too large, skipping",
                num_entries
            );
            return details;
        }

        // Calculate key positions:
        // - index_start: where the offset table begins (after the 3 u32 header values)
        // - frame_data_offset: where the actual sprite detail data begins (after the offset table)
        let index_start = list_start + 12; // After numEntries, listSize, maxDataLen
        let frame_data_offset = index_start + list_size * 4; // After the offset table

        debug!(
            "📋 Sprite detail table: framesStreamSize={} version={} listStart={} numEntries={} listSize={} maxDataLen={} frameDataOffset={}",
            frames_stream_size, version, list_start, num_entries, list_size, max_data_len, frame_data_offset
        );

        // Read all offsets and convert to absolute positions
        let mut absolute_offsets: Vec<usize> = Vec::with_capacity(num_entries);
        for i in 0..num_entries {
            match reader.read_u32() {
                Ok(relative_off) => {
                    let abs_off = frame_data_offset + relative_off as usize;
                    absolute_offsets.push(abs_off);
                }
                Err(_) => {
                    debug!(
                        "⚠️ Failed to read offset {}/{}", i, num_entries
                    );
                    break;
                }
            }
        }

        if absolute_offsets.len() != num_entries {
            debug!(
                "⚠️ Only read {} of {} sprite detail offsets",
                absolute_offsets.len(), num_entries
            );
        }

        // Log first few absolute offsets for debugging
        if !absolute_offsets.is_empty() {
            let first_few: Vec<String> = absolute_offsets.iter().take(10)
                .map(|&off| format!("{}", off))
                .collect();
            debug!(
                "   First {} absolute offsets: [{}]",
                first_few.len(), first_few.join(", ")
            );
        }

        // For a sprite with sprite_list_idx = N:
        //   - Behaviors are at getSpriteDetailsStream(N + 1), i.e., absolute_offsets[N + 1]
        //
        // So we iterate through sprite_list_idx values (0, 1, 2, ...) and look up behaviors
        // at index sprite_list_idx + 1
        let mut behavior_count = 0;
        for sprite_list_idx in 0..num_entries.saturating_sub(1) {
            // Behavior stream is at index sprite_list_idx + 1
            let behavior_stream_idx = sprite_list_idx + 1;

            if behavior_stream_idx >= absolute_offsets.len() {
                continue;
            }

            let behavior_start = absolute_offsets[behavior_stream_idx];

            // Calculate behavior stream size (to next offset or end of entry0)
            let behavior_end = if behavior_stream_idx + 1 < absolute_offsets.len() {
                absolute_offsets[behavior_stream_idx + 1]
            } else {
                entry0_len
            };

            // Validate bounds
            if behavior_start >= entry0_len || behavior_start >= behavior_end {
                continue;
            }

            let behavior_size = behavior_end - behavior_start;
            if behavior_size < 8 {
                // Need at least 8 bytes for one behavior element
                continue;
            }

            // Parse behaviors from this stream
            let behavior_data = &entry0[behavior_start..behavior_end];
            let mut behavior_reader = BinaryReader::from_u8(behavior_data);
            behavior_reader.set_endian(Endian::Big);

            let mut info = SpriteDetailInfo::default();

            // BehaviorElement format
            //   castLib (u16), member (u16), initializerIndex (u32)
            while behavior_reader.pos + 8 <= behavior_size {
                let cast_lib = match behavior_reader.read_u16() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let cast_member = match behavior_reader.read_u16() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let _initializer_idx = match behavior_reader.read_u32() {
                    Ok(v) => v,
                    Err(_) => break,
                };

                // Check for valid behavior reference
                // cast_lib == 0 && cast_member == 0 is end marker
                if cast_lib == 0 && cast_member == 0 {
                    break;
                }

                // cast_lib can be 65535 (-1 as u16) meaning "use parent cast lib"
                // cast_member should be > 0 for a valid reference
                if cast_member > 0 && cast_member < 10000 {
                    info.behaviors.push(SpriteBehavior { cast_lib, cast_member, parameter: Vec::new() });
                }
            }

            if !info.behaviors.is_empty() {
                behavior_count += info.behaviors.len();
                // Log behaviors for debugging (limit to first 30 to avoid spam)
                if details.len() < 30 {
                    let behavior_strs: Vec<String> = info.behaviors.iter()
                        .map(|b| {
                            if b.cast_lib == 65535 {
                                format!("(-1)/{}", b.cast_member)
                            } else {
                                format!("{}/{}", b.cast_lib, b.cast_member)
                            }
                        })
                        .collect();
                    debug!(
                        "   sprite_list_idx {}: {} behaviors [{}] (stream at {}..{})",
                        sprite_list_idx, info.behaviors.len(),
                        behavior_strs.join(", "),
                        behavior_start, behavior_end
                    );
                }
                details.insert(sprite_list_idx as u32, info);
            }
        }

        if !details.is_empty() {
            debug!(
                "✅ Parsed {} sprite details with {} total behaviors from Entry[0]",
                details.len(), behavior_count
            );
        } else if num_entries > 0 {
            debug!(
                "⚠️ No behaviors found despite {} sprite detail entries",
                num_entries
            );
        }

        details
    }

    /// Analyze score entries beyond Entry[0] for behavior attachment data
    fn analyze_behavior_attachment_entries(
        entries: &Vec<Vec<u8>>,
    ) -> Result<Vec<(FrameIntervalPrimary, Option<FrameIntervalSecondary>)>, String> {
        let mut results = vec![];
        let mut i = 2; // Start at 2, skip entries 0 and 1

        debug!("Starting to analyze {} entries", entries.len());

        // Log all entry sizes for debugging filmloop behavior issues
        if entries.len() > 2 && entries.len() < 50 {
            let sizes: Vec<usize> = entries.iter().map(|e| e.len()).collect();
            debug!(
                "analyze_behavior_attachment_entries: {} entries, sizes: {:?}",
                entries.len(), sizes
            );
        }

        // Log all entry sizes for debugging filmloop behavior issues
        if entries.len() > 2 && entries.len() < 50 {
            let sizes: Vec<usize> = entries.iter().map(|e| e.len()).collect();
            debug!(
                "📊 analyze_behavior_attachment_entries: {} entries, sizes: {:?}",
                entries.len(), sizes
            );
        }

        while i < entries.len() {
            let entry_bytes = &entries[i];

            if entry_bytes.is_empty() {
                i += 1;
                continue;
            }

            match entry_bytes.len() {
                // Primary entries: 40 bytes base (10 x u32) + optional trailing data.
                // D6/D7 typically produce 44 bytes, D8+ may produce 40 or 48.
                40 | 44 | 48 => {
                    // Primary entry
                    let mut reader = BinaryReader::from_u8(entry_bytes);
                    reader.set_endian(Endian::Big);

                    if let Ok(primary) = FrameIntervalPrimary::read(&mut reader) {
                        debug!(
                            "🎯 Found primary at entry {}: channel={}, frames={}-{}",
                            i, primary.channel_index, primary.start_frame, primary.end_frame
                        );

                        // Log primary entries for filmloop debugging
                        // Only log for main movie entries (high entry numbers, channels 55-100)
                        let is_main_movie_digit_channel = i > 3500 && primary.channel_index >= 55 && primary.channel_index <= 100;
                        if is_main_movie_digit_channel {
                            debug!(
                                "Primary entry {}: channel={} frames={}-{}",
                                i, primary.channel_index, primary.start_frame, primary.end_frame
                            );

                            // Log the secondary entry sizes for debugging
                            if i + 1 < entries.len() {
                                let sec_size = entries[i + 1].len();
                                let sec_bytes: String = entries[i + 1].iter()
                                    .take(32)
                                    .map(|b| format!("{:02x}", b))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                debug!(
                                    "   Secondary entry {} size={} bytes: {}",
                                    i + 1, sec_size, sec_bytes
                                );
                            }
                        }

                        // Look ahead to collect ALL secondary entries for this primary
                        let mut secondaries = Vec::new();
                        let mut j = i + 1;

                        // Keep reading secondary entries until we hit a non-secondary entry
                        while j < entries.len() {
                            let next_size = entries[j].len();

                            debug!("  🔎 Checking entry {} (size={})", j, next_size);

                            // Check if this could be a behavior entry
                            // Pattern: 8 bytes per behavior (cast_lib u16, cast_member u16, unk0 u32)
                            // Disambiguate from primary entries (40/44/48 bytes): peek at the
                            // content — a primary's first two u32s are start_frame/end_frame
                            // (small sequential numbers), while a behavior's first two u16s are
                            // cast_lib/cast_member.  If the first u32 looks like a valid
                            // start_frame (1–10000) and the second u32 >= first, treat it as a
                            // primary rather than a behavior list.
                            let looks_like_primary = if (next_size == 40 || next_size == 44 || next_size == 48) && next_size >= 8 {
                                let b = &entries[j];
                                let first_u32 = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
                                let second_u32 = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
                                first_u32 >= 1 && first_u32 <= 10000 && second_u32 >= first_u32
                            } else {
                                false
                            };
                            if next_size >= 8 && next_size % 8 == 0 && !looks_like_primary {
                                let behavior_count = next_size / 8;
                                let mut sec_reader = BinaryReader::from_u8(&entries[j]);
                                sec_reader.set_endian(Endian::Big);

                                debug!(
                                    "  📦 Entry {} has {} bytes = {} potential behaviors",
                                    j, next_size, behavior_count
                                );

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
                                                            debug!("parsed param string: {}", clean);
                                                            if clean.starts_with('[') {
                                                                // TODO: Replace `eval_lingo` with a parser
                                                                match eval_lingo_expr_static(clean.to_owned()) {
                                                                    Ok(proplist) => {
                                                                        debug!("eval_lingo_expr_static succeeded");
                                                                        secondary.parameter.push(proplist);
                                                                        debug!("parameter vector now has {} items", secondary.parameter.len());
                                                                    }
                                                                    Err(e) => {
                                                                        web_sys::console::error_1(&format!("eval_lingo_expr_static ERROR: {}", e.message).into());
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }

                                                    debug!(
                                                        "    ✅ Behavior {}: cast={}/{}, unk0={}",
                                                        behavior_idx + 1,
                                                        cast_lib,
                                                        cast_member,
                                                        unk0
                                                    );
                                                    secondaries.push(secondary);
                                                    found_valid_behavior = true;
                                                } else {
                                                    warn!("    ⏭️ Skipping invalid behavior {}: cast={}/{}", 
                                                        behavior_idx + 1, cast_lib, cast_member);
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
                                debug!("  ⏹️ Not a behavior entry size, stopping");
                                // Log entries that don't match behavior pattern
                                debug!(
                                    "   ⏹️ Entry {} size {} doesn't match behavior pattern (8-byte multiple)",
                                    j, next_size
                                );
                                break; // Not a behavior entry size
                            }
                        }

                        debug!(
                            "📊 Primary for channel {} has {} behaviors total",
                            primary.channel_index,
                            secondaries.len()
                        );

                        // Log behaviors found
                        debug!(
                            "Channel {} has {} behaviors",
                            primary.channel_index, secondaries.len()
                        );
                        for sec in &secondaries {
                            debug!(
                                "   Behavior: cast {}/{}",
                                sec.cast_lib, sec.cast_member
                            );
                        }

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
                    if entry_bytes.len() > 0 {
                        warn!(
                            "⚠️ Skipping entry {} with unexpected size {} bytes (expected 40/44/48 for primary). First bytes: {}",
                            i,
                            entry_bytes.len(),
                            entry_bytes.iter()
                                .take(20)
                                .map(|b| format!("{:02x}", b))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                    }
                }
            }

            i += 1;
        }

        debug!("🏁 Finished analyzing. Created {} results", results.len());
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
