use binary_reader::BinaryReader;
use fxhash::FxHashMap;
use std::convert::TryInto;

use crate::director::lingo::{constants::opcode_names, opcode::OpCode, script::ScriptContext};

#[allow(dead_code)]
pub struct HandlerRecord {
    name_id: u16,
    vector_pos: u16,
    compiled_len: usize,
    compiled_offset: usize,
    argument_count: u16,
    argument_offset: usize,
    locals_count: u16,
    locals_offset: usize,
    globals_count: u16,
    globals_offset: usize,
    unknown1: u32,
    unknown2: u16,
    line_count: u16,
    line_offset: u32,
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct Bytecode {
    pub opcode: OpCode,
    pub obj: i64,
    pub pos: usize,
    // TODO BytecodeTag
    owner_loop: u32,
    // TODO translation
}

impl Bytecode {
    pub fn pos_to_str(pos: usize) -> String {
        format_args!("[{}]", pos).to_string()
    }

    pub fn to_bytecode_text(&self, lctx: &ScriptContext, handler: &HandlerDef) -> String {
        let op_id = num::ToPrimitive::to_u16(&self.opcode).unwrap();
        let opcode_name = get_opcode_name(op_id);

        let mut writer = String::new();
        writer.push_str(&Self::pos_to_str(self.pos).as_str());
        writer.push(' ');
        writer.push_str(opcode_name);
        match self.opcode {
            OpCode::Jmp | OpCode::JmpIfZ => {
                writer.push(' ');
                writer.push_str(&Self::pos_to_str(self.pos + self.obj as usize));
            }
            OpCode::EndRepeat => {
                writer.push(' ');
                writer.push_str(&Self::pos_to_str(self.pos - self.obj as usize));
            }
            OpCode::ObjCall
            | OpCode::ExtCall
            | OpCode::GetObjProp
            | OpCode::SetObjProp
            | OpCode::PushSymb
            | OpCode::GetProp
            | OpCode::GetChainedProp => {
                let name = lctx.names.get(self.obj as usize).unwrap();
                writer.push(' ');
                writer.push_str(name);
            }
            OpCode::SetLocal | OpCode::GetLocal => {
                let name_id = handler
                    .local_name_ids
                    .get(self.obj as usize)
                    .map(|x| *x as usize);
                let name = name_id
                    .and_then(|name_id| lctx.names.get(name_id).map(|x| x.as_str()))
                    .unwrap_or("UNKOWN_LOCAL");
                writer.push(' ');
                writer.push_str(name);
            }
            OpCode::PushFloat32 => {
                writer.push(' ');
                if let Ok(bits) = self.obj.try_into() {
                    let f = f32::from_bits(bits);
                    writer.push_str(&f.to_string());
                } else {
                    writer.push_str("[invalid float bits]");
                }
            }
            _ => {
                if op_id > 0x40 {
                    writer.push(' ');
                    writer.push_str(self.obj.to_string().as_str())
                }
            }
        }

        // TODO lingo translation

        return writer;
    }
}

pub fn get_opcode_name(mut id: u16) -> &'static str {
    if id >= 0x40 {
        id = 0x40 + (id % 0x40);
    }

    if let Some(r) = opcode_names().get(&OpCode::from(id)) {
        r.as_ref()
    } else {
        "UNKOWN_BYTECODE"
    }
}

#[derive(Clone)]
pub struct HandlerDef {
    pub name_id: u16,
    pub bytecode_array: Vec<Bytecode>,
    pub bytecode_index_map: FxHashMap<usize, usize>,
    pub argument_name_ids: Vec<u16>,
    pub local_name_ids: Vec<u16>,
    pub global_name_ids: Vec<u16>,
}

impl HandlerRecord {
    #[allow(unused_variables)]
    pub fn read_record(
        reader: &mut BinaryReader,
        dir_version: u16,
        capital_x: bool,
    ) -> Result<HandlerRecord, String> {
        let name_id = reader.read_u16().unwrap();
        let vector_pos = reader.read_u16().unwrap();
        let compiled_len = reader.read_u32().unwrap() as usize;
        let compiled_offset = reader.read_u32().unwrap() as usize;
        let argument_count = reader.read_u16().unwrap();
        let argument_offset = reader.read_u32().unwrap() as usize;
        let locals_count = reader.read_u16().unwrap();
        let locals_offset = reader.read_u32().unwrap() as usize;
        let globals_count = reader.read_u16().unwrap();
        let globals_offset = reader.read_u32().unwrap() as usize;
        let unknown1 = reader.read_u32().unwrap();
        let unknown2 = reader.read_u16().unwrap();
        let line_count = reader.read_u16().unwrap();
        let line_offset = reader.read_u32().unwrap();
        // yet to implement
        if capital_x {
            let stack_height = reader.read_u32().unwrap();
        }

        // log_i(format_args!("Handler_record name_id: {name_id} compiled_len: {compiled_len} compiled_offset: {compiled_offset} globals_count: {globals_count} argument_count: {argument_count}").to_string().as_str());

        Ok(HandlerRecord {
            name_id,
            vector_pos,
            compiled_len,
            compiled_offset,
            argument_count,
            argument_offset,
            locals_count,
            locals_offset,
            globals_count,
            globals_offset,
            unknown1,
            unknown2,
            line_count,
            line_offset,
        })
    }

    pub fn read_data(
        reader: &mut BinaryReader,
        record: &HandlerRecord,
    ) -> Result<HandlerDef, String> {
        let mut bytecode_array: Vec<Bytecode> = Vec::new();
        let mut bytecode_index_map: FxHashMap<usize, usize> = FxHashMap::default();

        reader.jmp(record.compiled_offset);

        while reader.pos < record.compiled_offset + record.compiled_len {
            let pos = reader.pos - record.compiled_offset;
            let op = reader.read_u8().unwrap() as u16;
            let opcode = OpCode::from(if op >= 0x40 { 0x40 + op % 0x40 } else { op });
            // argument can be one, two or four bytes
            let mut obj: i64 = 0;
            if op >= 0xc0 {
                // four bytes
                obj = reader.read_i32().unwrap() as i64;
            } else if op >= 0x80 {
                // two bytes
                obj = match opcode {
                    OpCode::PushInt16 | OpCode::PushInt8 => {
                        // treat pushint's arg as signed
                        // pushint8 may be used to push a 16-bit int in older Lingo
                        reader.read_i16().unwrap() as i64
                    }
                    _ => reader.read_u16().unwrap() as i64,
                };
            } else if op >= 0x40 {
                // one byte
                if let OpCode::PushInt8 = opcode {
                    // treat pushint's arg as signed
                    obj = reader.read_i8().unwrap() as i64;
                } else {
                    obj = reader.read_u8().unwrap() as i64;
                }
            }

            let bytecode = Bytecode {
                opcode,
                obj,
                pos,
                owner_loop: u32::MAX,
            };

            bytecode_array.push(bytecode);
            bytecode_index_map.insert(pos, bytecode_array.len() - 1);
        }

        let argument_name_ids = read_varnames_table(
            reader,
            record.argument_count as usize,
            record.argument_offset,
        );
        let local_name_ids =
            read_varnames_table(reader, record.locals_count as usize, record.locals_offset);
        let global_name_ids =
            read_varnames_table(reader, record.globals_count as usize, record.globals_offset);

        return Ok(HandlerDef {
            name_id: record.name_id,
            argument_name_ids,
            bytecode_array,
            bytecode_index_map,
            local_name_ids,
            global_name_ids,
        });
    }
}

fn read_varnames_table(reader: &mut BinaryReader, count: usize, offset: usize) -> Vec<u16> {
    reader.jmp(offset);
    return (0..count).map(|_| reader.read_u16().unwrap()).collect();
}
