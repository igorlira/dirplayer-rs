use crate::director::{
    chunks::handler::HandlerDef,
    lingo::{datum::Datum, opcode::OpCode, script::ScriptContext},
};
use std::convert::TryInto;

pub struct StackExpressionTracker {
    stack: Vec<String>,
    last_arg_count: usize, // Track argument count from PushArgList
}

impl StackExpressionTracker {
    pub fn new() -> Self {
        Self { 
            stack: Vec::new(),
            last_arg_count: 0,
        }
    }

    pub fn clear(&mut self) {
        self.stack.clear();
        self.last_arg_count = 0;
    }

    /// Process a bytecode and return its annotation
    pub fn process_bytecode(
        &mut self,
        bytecode: &crate::director::chunks::handler::Bytecode,
        lctx: &ScriptContext,
        handler: &HandlerDef,
        multiplier: u32,
        literals: &[Datum],
    ) -> String {
        match bytecode.opcode {
            // ============================================================
            // PUSH OPERATIONS
            // ============================================================
            
            OpCode::PushInt8 | OpCode::PushInt16 | OpCode::PushInt32 => {
                let expr = format!("{}", bytecode.obj);
                self.stack.push(expr.clone());
                format!("<{}>", expr)
            }

            OpCode::PushFloat32 => {
                if let Ok(bits) = bytecode.obj.try_into() {
                    let f = f32::from_bits(bits);
                    let expr = format!("{}", f);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    self.stack.push("[invalid float]".to_string());
                    String::new()
                }
            }

            OpCode::PushZero => {
                self.stack.push("0".to_string());
                format!("<0>")
            }

            OpCode::PushSymb => {
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    let expr = format!("#{}", name);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    self.stack.push("#UNKNOWN".to_string());
                    String::new()
                }
            }

            OpCode::PushCons => {
                let literal_id = (bytecode.obj as u32 / multiplier) as usize;
                
                if let Some(literal) = literals.get(literal_id) {
                    let expr = Self::format_literal(literal);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    self.stack.push(format!("CONST[{}]", literal_id)).clone();
                    format!("<CONST[{}]>", literal_id)
                }
            }

            // ============================================================
            // VARIABLE ACCESS - LOCAL
            // ============================================================
            
            OpCode::GetLocal => {
                let local_index = (bytecode.obj as u32 / multiplier) as usize;
                let name = handler
                    .local_name_ids
                    .get(local_index)
                    .and_then(|&name_id| lctx.names.get(name_id as usize))
                    .map(|s| s.as_str())
                    .unwrap_or("UNKNOWN");
                self.stack.push(name.to_string());
                format!("<{}>", name)
            }

            OpCode::SetLocal => {
                let local_index = (bytecode.obj as u32 / multiplier) as usize;
                let name = handler
                    .local_name_ids
                    .get(local_index)
                    .and_then(|&name_id| lctx.names.get(name_id as usize))
                    .map(|s| s.as_str())
                    .unwrap_or("UNKNOWN");

                if let Some(value) = self.stack.last() {
                    format!("<{} = {}>", name, value)
                } else {
                    format!("<{} = ?>", name)
                }
            }

            // ============================================================
            // VARIABLE ACCESS - PARAMETER
            // ============================================================
            
            OpCode::GetParam => {
                let param_index = (bytecode.obj as u32 / multiplier) as usize;
                let name = handler
                    .argument_name_ids
                    .get(param_index)
                    .and_then(|&name_id| lctx.names.get(name_id as usize))
                    .map(|s| s.as_str())
                    .unwrap_or("UNKNOWN");
                self.stack.push(name.to_string());
                format!("<{}>", name)
            }

            OpCode::SetParam => {
                let param_index = (bytecode.obj as u32 / multiplier) as usize;
                let name = handler
                    .argument_name_ids
                    .get(param_index)
                    .and_then(|&name_id| lctx.names.get(name_id as usize))
                    .map(|s| s.as_str())
                    .unwrap_or("UNKNOWN");

                if let Some(value) = self.stack.last() {
                    format!("<{} = {}>", name, value)
                } else {
                    format!("<{} = ?>", name)
                }
            }

            // ============================================================
            // VARIABLE ACCESS - GLOBAL
            // ============================================================
            
            OpCode::GetGlobal => {
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    self.stack.push(name.to_string());
                    format!("<{}>", name)
                } else {
                    self.stack.push("UNKNOWN_GLOBAL".to_string());
                    String::new()
                }
            }

            OpCode::SetGlobal => {
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    if let Some(value) = self.stack.last() {
                        format!("<{} = {}>", name, value)
                    } else {
                        format!("<{} = ?>", name)
                    }
                } else {
                    String::new()
                }
            }

            // ============================================================
            // PROPERTY ACCESS
            // ============================================================
            
            OpCode::GetProp => {
                // GetProp ALWAYS gets property from 'me' (scope receiver)
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    let expr = format!("me.{}", name);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    // Name not found - show ID
                    let expr = format!("me.prop{}", bytecode.obj);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                }
            }

            OpCode::SetProp => {
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    if self.stack.len() >= 2 {
                        // Has both value and object
                        let value = self.stack.pop().unwrap();
                        let obj = self.stack.pop().unwrap();
                        format!("<{}.{} = {}>", obj, name, value)
                    } else if self.stack.len() == 1 {
                        // Only value on stack - implicit me
                        let value = self.stack.pop().unwrap();
                        format!("<me.{} = {}>", name, value)
                    } else {
                        String::new()
                    }
                } else {
                    // Name not found - show ID
                    if self.stack.len() >= 2 {
                        let value = self.stack.pop().unwrap();
                        let obj = self.stack.pop().unwrap();
                        format!("<{}.prop{} = {}>", obj, bytecode.obj, value)
                    } else if self.stack.len() == 1 {
                        let value = self.stack.pop().unwrap();
                        format!("<me.prop{} = {}>", bytecode.obj, value)
                    } else {
                        String::new()
                    }
                }
            }

            OpCode::GetChainedProp => {
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    if self.stack.is_empty() {
                        // No object on stack - implicit "me"
                        let expr = format!("me.{}", name);
                        self.stack.push(expr.clone());
                        format!("<{}>", expr)
                    } else {
                        // Explicit object on stack
                        let obj = self.stack.pop().unwrap();
                        let expr = format!("{}.{}", obj, name);
                        self.stack.push(expr.clone());
                        format!("<{}>", expr)
                    }
                } else {
                    // Name not found - show ID
                    if self.stack.is_empty() {
                        let expr = format!("me.prop{}", bytecode.obj);
                        self.stack.push(expr.clone());
                        format!("<{}>", expr)
                    } else {
                        let obj = self.stack.pop().unwrap();
                        let expr = format!("{}.prop{}", obj, bytecode.obj);
                        self.stack.push(expr.clone());
                        format!("<{}>", expr)
                    }
                }
            }

            OpCode::GetObjProp => {
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    if self.stack.is_empty() {
                        // No object on stack - implicit "me"
                        let expr = format!("me[#{}]", name);
                        self.stack.push(expr.clone());
                        format!("<{}>", expr)
                    } else {
                        // Explicit object on stack
                        let obj = self.stack.pop().unwrap();
                        let expr = format!("{}[#{}]", obj, name);
                        self.stack.push(expr.clone());
                        format!("<{}>", expr)
                    }
                } else {
                    // Name not found - show ID
                    if self.stack.is_empty() {
                        let expr = format!("me[#prop{}]", bytecode.obj);
                        self.stack.push(expr.clone());
                        format!("<{}>", expr)
                    } else {
                        let obj = self.stack.pop().unwrap();
                        let expr = format!("{}[#prop{}]", obj, bytecode.obj);
                        self.stack.push(expr.clone());
                        format!("<{}>", expr)
                    }
                }
            }

            OpCode::SetObjProp => {
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    if self.stack.len() >= 2 {
                        // Has both value and object
                        let value = self.stack.pop().unwrap();
                        let obj = self.stack.pop().unwrap();
                        format!("<{}[#{}] = {}>", obj, name, value)
                    } else if self.stack.len() == 1 {
                        // Only value on stack - implicit me
                        let value = self.stack.pop().unwrap();
                        format!("<me[#{}] = {}>", name, value)
                    } else {
                        String::new()
                    }
                } else {
                    // Name not found - show ID
                    if self.stack.len() >= 2 {
                        let value = self.stack.pop().unwrap();
                        let obj = self.stack.pop().unwrap();
                        format!("<{}[#prop{}] = {}>", obj, bytecode.obj, value)
                    } else if self.stack.len() == 1 {
                        let value = self.stack.pop().unwrap();
                        format!("<me[#prop{}] = {}>", bytecode.obj, value)
                    } else {
                        String::new()
                    }
                }
            }

            OpCode::GetTopLevelProp => {
                // This one is fine as-is - it's always _global
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    let expr = format!("_global.{}", name);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    let expr = format!("_global.prop{}", bytecode.obj);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                }
            }

            // ============================================================
            // THE BUILTIN & MOVIE PROPERTIES
            // ============================================================
            
            OpCode::TheBuiltin => {
                let prop_name = self.get_builtin_name(bytecode.obj as u16);
                let expr = format!("the {}", prop_name);
                self.stack.push(expr.clone());
                format!("<{}>", expr)
            }

            OpCode::GetMovieProp => {
                let prop_name = self.get_movie_prop_name(bytecode.obj as u16);
                let expr = format!("the {}", prop_name);
                self.stack.push(expr.clone());
                format!("<{}>", expr)
            }

            OpCode::SetMovieProp => {
                let prop_name = self.get_movie_prop_name(bytecode.obj as u16);
                if let Some(value) = self.stack.last() {
                    format!("<the {} = {}>", prop_name, value)
                } else {
                    String::new()
                }
            }

            // ============================================================
            // ARITHMETIC OPERATIONS
            // ============================================================
            
            OpCode::Add => self.binary_op("+"),
            OpCode::Sub => self.binary_op("-"),
            OpCode::Mul => self.binary_op("*"),
            OpCode::Div => self.binary_op("/"),
            OpCode::Mod => self.binary_op("mod"),
            
            OpCode::Inv => {
                if let Some(a) = self.stack.pop() {
                    let expr = format!("-({})", a);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    String::new()
                }
            }

            // ============================================================
            // STRING OPERATIONS
            // ============================================================
            
            OpCode::JoinStr => self.binary_op("&"),
            OpCode::JoinPadStr => self.binary_op("&&"),
            OpCode::ContainsStr => self.binary_op("contains"),
            OpCode::Contains0Str => self.binary_op("starts"),
            
            OpCode::GetChunk => {
                // Pop chunk expression components
                if self.stack.len() >= 3 {
                    let end_idx = self.stack.pop().unwrap();
                    let start_idx = self.stack.pop().unwrap();
                    let obj = self.stack.pop().unwrap();
                    let expr = format!("char {} to {} of {}", start_idx, end_idx, obj);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    String::new()
                }
            }

            OpCode::Put => {
                // put <source> into/after/before <dest>
                if self.stack.len() >= 2 {
                    let dest = self.stack.pop().unwrap();
                    let source = self.stack.pop().unwrap();
                    format!("<put {} into {}>", source, dest)
                } else {
                    String::new()
                }
            }

            OpCode::PutChunk => {
                // put <value> into char X to Y of <string>
                if self.stack.len() >= 2 {
                    let chunk = self.stack.pop().unwrap();
                    let value = self.stack.pop().unwrap();
                    format!("<put {} into {}>", value, chunk)
                } else {
                    String::new()
                }
            }

            OpCode::DeleteChunk => {
                if let Some(chunk) = self.stack.pop() {
                    format!("<delete {}>", chunk)
                } else {
                    String::new()
                }
            }

            // ============================================================
            // COMPARISON OPERATIONS
            // ============================================================
            
            OpCode::Eq => self.binary_op("="),
            OpCode::NtEq => self.binary_op("<>"),
            OpCode::Lt => self.binary_op("<"),
            OpCode::LtEq => self.binary_op("<="),
            OpCode::Gt => self.binary_op(">"),
            OpCode::GtEq => self.binary_op(">="),

            // ============================================================
            // LOGICAL OPERATIONS
            // ============================================================
            
            OpCode::And => self.binary_op("and"),
            OpCode::Or => self.binary_op("or"),
            
            OpCode::Not => {
                if let Some(a) = self.stack.pop() {
                    let expr = format!("not ({})", a);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    String::new()
                }
            }

            // ============================================================
            // LIST OPERATIONS
            // ============================================================
            
            OpCode::PushList => {
                let count = self.last_arg_count;
                let mut items = Vec::new();
                for _ in 0..count {
                    if let Some(item) = self.stack.pop() {
                        items.push(item);
                    }
                }
                items.reverse();
                let expr = format!("[{}]", items.join(", "));
                self.stack.push(expr.clone());
                format!("<{}>", expr)
            }

            OpCode::PushPropList => {
                let count = self.last_arg_count / 2; // Property lists have key-value pairs
                let mut items = Vec::new();
                for _ in 0..count {
                    if self.stack.len() >= 2 {
                        let value = self.stack.pop().unwrap();
                        let key = self.stack.pop().unwrap();
                        items.push(format!("{}: {}", key, value));
                    }
                }
                items.reverse();
                let expr = format!("[{}]", items.join(", "));
                self.stack.push(expr.clone());
                format!("<{}>", expr)
            }

            OpCode::PushArgList => {
                self.last_arg_count = bytecode.obj as usize;
                format!("<{}>", bytecode.obj)
            }

            OpCode::PushArgListNoRet => {
                self.last_arg_count = bytecode.obj as usize;
                format!("<{}>", bytecode.obj)
            }

            // ============================================================
            // FUNCTION CALLS
            // ============================================================
            
            OpCode::ExtCall => {
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    let count = self.last_arg_count;
                    let mut args = Vec::new();
                    for _ in 0..count {
                        if let Some(arg) = self.stack.pop() {
                            args.push(arg);
                        }
                    }
                    args.reverse();
                    let expr = format!("{}({})", name, args.join(", "));
                    self.stack.push(expr.clone());
                    format!("{}", expr)
                } else {
                    format!("UNKNOWN()")
                }
            }

            OpCode::LocalCall => {
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    let count = self.last_arg_count;
                    let mut args = Vec::new();
                    for _ in 0..count {
                        if let Some(arg) = self.stack.pop() {
                            args.push(arg);
                        }
                    }
                    args.reverse();
                    let expr = format!("{}({})", name, args.join(", "));
                    format!("{}", expr)
                } else {
                    format!("UNKNOWN()")
                }
            }

            OpCode::ObjCall => {
                if let Some(name) = lctx.names.get(bytecode.obj as usize) {
                    let count = self.last_arg_count;
                    let mut args = Vec::new();
                    for _ in 0..count {
                        if let Some(arg) = self.stack.pop() {
                            args.push(arg);
                        }
                    }
                    if let Some(obj) = self.stack.pop() {
                        args.reverse();
                        let expr = format!("{}.{}({})", obj, name, args.join(", "));
                        self.stack.push(expr.clone());
                        format!("<{}>", expr)
                    } else {
                        format!("<?.{}(...)>", name)
                    }
                } else {
                    String::new()
                }
            }

            // ============================================================
            // OBJECT CREATION
            // ============================================================
            
            OpCode::NewObj => {
                if let Some(obj_type) = self.stack.pop() {
                    let count = self.last_arg_count;
                    let mut args = Vec::new();
                    for _ in 0..count {
                        if let Some(arg) = self.stack.pop() {
                            args.push(arg);
                        }
                    }
                    args.reverse();
                    let expr = if args.is_empty() {
                        format!("new({})", obj_type)
                    } else {
                        format!("new({}, {})", obj_type, args.join(", "))
                    };
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    String::new()
                }
            }

            // ============================================================
            // CONTROL FLOW
            // ============================================================
            
            OpCode::Ret => {
                "exit".to_string()
            }

            OpCode::JmpIfZ => {
                if let Some(condition) = self.stack.last() {
                    format!("if {} then", condition)
                } else {
                    "if ? then".to_string()
                }
            }

            OpCode::Jmp => {
                String::new()
            }

            OpCode::EndRepeat => {
                String::new()
            }

            // ============================================================
            // STACK MANIPULATION
            // ============================================================
            
            OpCode::Pop => {
                let count = bytecode.obj as usize;
                for _ in 0..count {
                    self.stack.pop();
                }
                if count == 1 {
                    "end case".to_string()
                } else {
                    String::new()
                }
            }

            OpCode::Swap => {
                if self.stack.len() >= 2 {
                    let len = self.stack.len();
                    self.stack.swap(len - 1, len - 2);
                }
                String::new()
            }

            OpCode::Peek => {
                let offset = bytecode.obj as usize;
                if offset < self.stack.len() {
                    let idx = self.stack.len() - 1 - offset;
                    if let Some(item) = self.stack.get(idx) {
                        self.stack.push(item.clone());
                    }
                }
                format!("<peek {}>", offset)
            }

            // ============================================================
            // FIELD OPERATIONS
            // ============================================================
            
            OpCode::GetField => {
                // This is complex - field references
                String::new()
            }

            OpCode::Set => {
                // Generic set operation
                if self.stack.len() >= 2 {
                    let value = self.stack.pop().unwrap();
                    let target = self.stack.pop().unwrap();
                    format!("<{} = {}>", target, value)
                } else {
                    String::new()
                }
            }

            OpCode::Get => {
                // Generic get operation
                String::new()
            }

            // ============================================================
            // SPRITE OPERATIONS
            // ============================================================
            
            OpCode::OntoSpr => {
                if self.stack.len() >= 2 {
                    let sprite = self.stack.pop().unwrap();
                    let point = self.stack.pop().unwrap();
                    let expr = format!("{} within {}", point, sprite);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    String::new()
                }
            }

            OpCode::IntoSpr => {
                if self.stack.len() >= 2 {
                    let sprite = self.stack.pop().unwrap();
                    let point = self.stack.pop().unwrap();
                    let expr = format!("{} intersects {}", point, sprite);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    String::new()
                }
            }

            // OpCode::HiliteSpr => {
            //     if let Some(sprite) = self.stack.pop() {
            //         let expr = format!("hilite({})", sprite);
            //         self.stack.push(expr.clone());
            //         format!("<{}>", expr)
            //     } else {
            //         String::new()
            //     }
            // }

            // ============================================================
            // CHUNK VARIABLE REFERENCES
            // ============================================================
            
            OpCode::PushChunkVarRef => {
                // Push a reference to a chunk for later assignment
                if self.stack.len() >= 3 {
                    let end_idx = self.stack.pop().unwrap();
                    let start_idx = self.stack.pop().unwrap();
                    let obj = self.stack.pop().unwrap();
                    let expr = format!("char {} to {} of {}", start_idx, end_idx, obj);
                    self.stack.push(expr.clone());
                    format!("<{}>", expr)
                } else {
                    String::new()
                }
            }

            // ============================================================
            // DEFAULT - UNHANDLED OPCODES
            // ============================================================
            
            _ => {
                // For any unhandled opcode, just show the opcode name
                String::new()
            }
        }
    }

    fn binary_op(&mut self, op: &str) -> String {
        if self.stack.len() >= 2 {
            let b = self.stack.pop().unwrap();
            let a = self.stack.pop().unwrap();
            let expr = format!("{} {} {}", a, op, b);
            self.stack.push(expr.clone());
            format!("<{}>", expr)
        } else {
            String::new()
        }
    }

    fn format_literal(literal: &Datum) -> String {
        match literal {
            Datum::String(s) => format!("\"{}\"", s.replace("\"", "\\\"")),
            Datum::Symbol(s) => format!("#{}", s),
            Datum::Int(i) => format!("{}", i),
            Datum::Float(f) => format!("{}", f),
            Datum::List(_, items, _) => {
                format!("[...]")
            }
            Datum::PropList(items, _) => {
                format!("[...]")
            }
            Datum::Void => "VOID".to_string(),
            _ => format!("?"),
        }
    }

    fn get_builtin_name(&self, id: u16) -> String {
        match id {
            0x00 => "floatPrecision".to_string(),
            0x01 => "mouseDownScript".to_string(),
            0x02 => "mouseUpScript".to_string(),
            0x03 => "keyDownScript".to_string(),
            0x04 => "keyUpScript".to_string(),
            0x05 => "timeoutScript".to_string(),
            0x06 => "updateMovieEnabled".to_string(),
            0x07 => "selStart".to_string(),
            0x08 => "selEnd".to_string(),
            0x09 => "soundLevel".to_string(),
            0x0A => "fixStageSize".to_string(),
            0x0B => "searchCurrentFolder".to_string(),
            0x0C => "searchPaths".to_string(),
            0x0D => "lastClick".to_string(),
            0x0E => "lastRoll".to_string(),
            0x0F => "lastEvent".to_string(),
            0x10 => "lastKey".to_string(),
            0x11 => "timeoutLapsed".to_string(),
            0x12 => "multiSound".to_string(),
            0x13 => "soundKeepDevice".to_string(),
            0x14 => "soundMixMedia".to_string(),
            0x15 => "freeBytes".to_string(),
            0x16 => "freeBLock".to_string(),
            0x17 => "maxInteger".to_string(),
            0x18 => "pi".to_string(),
            0x19 => "rightMouseDown".to_string(),
            0x1A => "optionDown".to_string(),
            0x1B => "commandDown".to_string(),
            0x1C => "controlDown".to_string(),
            0x1D => "shiftDown".to_string(),
            0x1E => "platform".to_string(),
            0x1F => "colorDepth".to_string(),
            0x20 => "frame".to_string(),
            0x21 => "movie".to_string(),
            0x22 => "beepOn".to_string(),
            0x23 => "movieName".to_string(),
            0x24 => "moviePath".to_string(),
            0x25 => "movieFileFreeSize".to_string(),
            0x26 => "movieFileSize".to_string(),
            0x27 => "pathName".to_string(),
            0x28 => "systemDate".to_string(),
            0x29 => "applicationPath".to_string(),
            0x2A => "machinetype".to_string(),
            0x2B => "productVersion".to_string(),
            0x2C => "romanLingo".to_string(),
            0x2D => "version".to_string(),
            0x2E => "environment".to_string(),
            0x2F => "deskTopRectList".to_string(),
            0x30 => "colorQD".to_string(),
            0x31 => "quickTimePresent".to_string(),
            0x32 => "memorySize".to_string(),
            0x33 => "checkBoxAccess".to_string(),
            0x34 => "checkBoxType".to_string(),
            0x35 => "lastFrame".to_string(),
            0x36 => "lastClick".to_string(),
            0x37 => "lastRoll".to_string(),
            0x38 => "lastEvent".to_string(),
            0x39 => "lastKey".to_string(),
            0x3A => "doubleClick".to_string(),
            0x3B => "keyCode".to_string(),
            0x3C => "key".to_string(),
            0x3D => "mouseH".to_string(),
            0x3E => "mouseV".to_string(),
            0x3F => "mouseDown".to_string(),
            0x40 => "ticks".to_string(),
            0x41 => "timer".to_string(),
            0x42 => "clickLoc".to_string(),
            0x43 => "rollover".to_string(),
            0x44 => "centerStage".to_string(),
            0x45 => "exitLock".to_string(),
            0x46 => "runMode".to_string(),
            0x47 => "windowPresent".to_string(),
            0x48 => "currentSpriteNum".to_string(),
            0x49 => "puppetSprite".to_string(),
            0x4A => "pauseState".to_string(),
            0x4B => "timeoutKeyDown".to_string(),
            0x4C => "timeoutLength".to_string(),
            0x4D => "timeoutMouse".to_string(),
            0x4E => "timeoutPlay".to_string(),
            0x4F => "perFrameHook".to_string(),
            0x50 => "alertHook".to_string(),
            0x51 => "updateLock".to_string(),
            0x52 => "itemDelimiter".to_string(),
            0x53 => "colorDepth".to_string(),
            0x54 => "switchColorDepth".to_string(),
            0x55 => "maxInteger".to_string(),
            0x56 => "preLoadRAM".to_string(),
            0x57 => "cursor".to_string(),
            0x58 => "keyDownScript".to_string(),
            0x59 => "keyUpScript".to_string(),
            0x5A => "mouseDownScript".to_string(),
            0x5B => "mouseUpScript".to_string(),
            0x5C => "timeoutScript".to_string(),
            0x5D => "buttonStyle".to_string(),
            0x5E => "selStart".to_string(),
            0x5F => "selEnd".to_string(),
            0x60 => "videoForWindowsPresent".to_string(),
            0x61 => "quickTimeVersion".to_string(),
            0x62 => "soundDevice".to_string(),
            0x63 => "soundEnabled".to_string(),
            0x64 => "traceLoad".to_string(),
            0x65 => "traceLogFile".to_string(),
            0x66 => "stageColor".to_string(),
            0x67 => "paramCount".to_string(),
            0x68 => "mouseItem".to_string(),
            0x69 => "mouseWord".to_string(),
            0x6A => "mouseLine".to_string(),
            0x6B => "mouseChar".to_string(),
            0x6C => "menu".to_string(),
            0x6D => "menuItems".to_string(),
            0x6E => "locToCharPos".to_string(),
            0x6F => "charToLoc".to_string(),
            0x70 => "frameTempo".to_string(),
            0x71 => "framePalette".to_string(),
            0x72 => "frameLabel".to_string(),
            0x73 => "frameScript".to_string(),
            0x74 => "scriptExecutionStyle".to_string(),
            0x75 => "selection".to_string(),
            0x76 => "stillDown".to_string(),
            0x77 => "result".to_string(),
            0x78 => "number of castMembers".to_string(),
            0x79 => "number of menus".to_string(),
            0x7A => "number of menuItems".to_string(),
            0x7B => "number of chars".to_string(),
            0x7C => "number of words".to_string(),
            0x7D => "number of items".to_string(),
            0x7E => "number of lines".to_string(),
            0x7F => "number of castLibs".to_string(),
            _ => format!("builtin{}", id),
        }
    }

    fn get_movie_prop_name(&self, id: u16) -> String {
        match id {
            0x00 => "beepOn".to_string(),
            0x01 => "buttonStyle".to_string(),
            0x02 => "centerStage".to_string(),
            0x03 => "checkBoxAccess".to_string(),
            0x04 => "checkBoxType".to_string(),
            0x06 => "colorDepth".to_string(),
            0x07 => "colorQD".to_string(),
            0x08 => "exitLock".to_string(),
            0x09 => "floatPrecision".to_string(),
            0x0A => "frameLabel".to_string(),
            0x0B => "framePalette".to_string(),
            0x0C => "frameScript".to_string(),
            0x0D => "frameTempo".to_string(),
            0x0E => "itemDelimiter".to_string(),
            0x0F => "keyDownScript".to_string(),
            0x10 => "keyUpScript".to_string(),
            0x11 => "lastClick".to_string(),
            0x12 => "lastEvent".to_string(),
            0x13 => "lastFrame".to_string(),
            0x14 => "lastKey".to_string(),
            0x15 => "lastRoll".to_string(),
            0x16 => "locToCharPos".to_string(),
            0x17 => "menuItems".to_string(),
            0x18 => "menu".to_string(),
            0x19 => "mouseChar".to_string(),
            0x1A => "mouseDown".to_string(),
            0x1B => "mouseDownScript".to_string(),
            0x1C => "mouseH".to_string(),
            0x1D => "mouseItem".to_string(),
            0x1E => "mouseLine".to_string(),
            0x1F => "mouseMember".to_string(),
            0x20 => "mouseUpScript".to_string(),
            0x21 => "mouseV".to_string(),
            0x22 => "mouseWord".to_string(),
            0x23 => "movieFileFreeSize".to_string(),
            0x24 => "movieFileFreeSize".to_string(),
            0x25 => "movieFileSize".to_string(),
            0x26 => "movieName".to_string(),
            0x27 => "moviePath".to_string(),
            0x28 => "paramCount".to_string(),
            0x29 => "pauseState".to_string(),
            0x2A => "perFrameHook".to_string(),
            0x2B => "preloadRAM".to_string(),
            0x2C => "quickTimePresent".to_string(),
            0x2D => "rollover".to_string(),
            0x2E => "romanLingo".to_string(),
            0x2F => "runMode".to_string(),
            0x30 => "scriptExecutionStyle".to_string(),
            0x31 => "selEnd".to_string(),
            0x32 => "selStart".to_string(),
            0x33 => "soundDevice".to_string(),
            0x34 => "soundEnabled".to_string(),
            0x35 => "soundKeepDevice".to_string(),
            0x36 => "soundLevel".to_string(),
            0x37 => "soundMixMedia".to_string(),
            0x38 => "stageColor".to_string(),
            0x49 => "switchColorDepth".to_string(),
            0x4A => "timeoutKeyDown".to_string(),
            0x4B => "timeoutLapsed".to_string(),
            0x4C => "timeoutLength".to_string(),
            0x4D => "timeoutMouse".to_string(),
            0x4E => "timeoutPlay".to_string(),
            0x4F => "timeoutScript".to_string(),
            0x50 => "timer".to_string(),
            0x51 => "traceLoad".to_string(),
            0x52 => "traceLogFile".to_string(),
            0x53 => "updateMovieEnabled".to_string(),
            0x54 => "videoForWindowsPresent".to_string(),
            0x55 => "floatPrecision".to_string(),
            0xB1 => "currentSpriteNum".to_string(),
            _ => format!("movieProp{}", id),
        }
    }

    pub fn get_stack_top(&self) -> Option<&String> {
        self.stack.last()
    }
}
