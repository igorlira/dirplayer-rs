use binary_reader::{BinaryReader, Endian};

/// Director `cupt` chunk — list of cue points attached to a Sound cast member.
///
/// Layout (big-endian):
///   u32        count
///   N records, each `record_size = (chunk_size - 4) / count`:
///     u32      time_ms
///     u8       name_len
///     [u8]     name (length-prefixed)
///     ...      zero padding to `record_size`
///
/// `record_size` is encoded implicitly. For the inspected sample
/// (Fugue No. 4 / i04.dir, member "Track  2") it's 36 — wide enough for
/// "Untitled Marker NNN " (20 chars) plus trailing extras whose meaning is
/// not yet known but appears to be all zero. We tolerate any per-record
/// width that's at least the time + name-length byte.
#[derive(Clone, Debug)]
pub struct CuePointsChunk {
    pub times_ms: Vec<u32>,
    pub names: Vec<String>,
}

impl CuePointsChunk {
    pub fn from_reader(reader: &mut BinaryReader) -> Result<CuePointsChunk, String> {
        let original = reader.endian;
        reader.endian = Endian::Big;

        let mut body = Vec::new();
        while let Ok(b) = reader.read_u8() {
            body.push(b);
        }

        reader.endian = original;

        if body.len() < 4 {
            return Err(format!("cupt chunk too small: {} bytes", body.len()));
        }
        let count = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
        if count == 0 {
            return Ok(CuePointsChunk { times_ms: Vec::new(), names: Vec::new() });
        }
        let payload = body.len() - 4;
        if payload < count {
            return Err(format!(
                "cupt chunk truncated: count={} but only {} payload bytes",
                count, payload
            ));
        }
        let per_record = payload / count;
        if per_record < 5 {
            return Err(format!(
                "cupt record too small: per_record={} count={} payload={}",
                per_record, count, payload
            ));
        }

        let mut times_ms = Vec::with_capacity(count);
        let mut names = Vec::with_capacity(count);
        for i in 0..count {
            let ofs = 4 + i * per_record;
            if ofs + per_record > body.len() {
                break;
            }
            let time = u32::from_be_bytes([
                body[ofs],
                body[ofs + 1],
                body[ofs + 2],
                body[ofs + 3],
            ]);
            let name_len = body[ofs + 4] as usize;
            let name_end = (ofs + 5 + name_len).min(ofs + per_record);
            let raw = &body[ofs + 5..name_end];
            // Trim Director's trailing space/null padding from the stored name.
            let name = String::from_utf8_lossy(raw)
                .trim_end_matches(|c: char| c == '\0' || c == ' ')
                .to_string();
            times_ms.push(time);
            names.push(name);
        }

        Ok(CuePointsChunk { times_ms, names })
    }
}
