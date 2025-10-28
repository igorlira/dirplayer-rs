use std::{collections::HashMap, sync::OnceLock};

use super::opcode::OpCode;

pub fn opcode_names() -> &'static HashMap<OpCode, Box<str>> {
    static MAP: OnceLock<HashMap<OpCode, Box<str>>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            // single-byte
            (OpCode::Ret, "ret".into()),
            (OpCode::RetFactory, "retfactory".into()),
            (OpCode::Mul, "mul".into()),
            (OpCode::PushZero, "pushzero".into()),
            (OpCode::Add, "add".into()),
            (OpCode::Sub, "sub".into()),
            (OpCode::Div, "div".into()),
            (OpCode::Mod, "mod".into()),
            (OpCode::Inv, "inv".into()),
            (OpCode::JoinStr, "joinstr".into()),
            (OpCode::JoinPadStr, "joinpadstr".into()),
            (OpCode::Lt, "lt".into()),
            (OpCode::LtEq, "lteq".into()),
            (OpCode::NtEq, "nteq".into()),
            (OpCode::Eq, "eq".into()),
            (OpCode::Gt, "gt".into()),
            (OpCode::GtEq, "gteq".into()),
            (OpCode::And, "and".into()),
            (OpCode::Or, "or".into()),
            (OpCode::Not, "not".into()),
            (OpCode::ContainsStr, "containsstr".into()),
            (OpCode::Contains0Str, "contains0str".into()),
            (OpCode::GetChunk, "getchunk".into()),
            (OpCode::HiliteChunk, "hilitechunk".into()),
            (OpCode::OntoSpr, "ontospr".into()),
            (OpCode::IntoSpr, "intospr".into()),
            (OpCode::GetField, "getfield".into()),
            (OpCode::StartTell, "starttell".into()),
            (OpCode::EndTell, "endtell".into()),
            (OpCode::PushList, "pushlist".into()),
            (OpCode::PushPropList, "pushproplist".into()),
            (OpCode::Swap, "swap".into()),
            (OpCode::CallJavaScript, "calljavascript".into()),
            // multi-byte
            (OpCode::PushInt8, "pushint8".into()),
            (OpCode::PushArgListNoRet, "pusharglistnoret".into()),
            (OpCode::PushArgList, "pusharglist".into()),
            (OpCode::PushCons, "pushcons".into()),
            (OpCode::PushSymb, "pushsymb".into()),
            (OpCode::PushVarRef, "pushvarref".into()),
            (OpCode::GetGlobal2, "getglobal2".into()),
            (OpCode::GetGlobal, "getglobal".into()),
            (OpCode::GetProp, "getprop".into()),
            (OpCode::GetParam, "getparam".into()),
            (OpCode::GetLocal, "getlocal".into()),
            (OpCode::SetGlobal2, "setglobal2".into()),
            (OpCode::SetGlobal, "setglobal".into()),
            (OpCode::SetProp, "setprop".into()),
            (OpCode::SetParam, "setparam".into()),
            (OpCode::SetLocal, "setlocal".into()),
            (OpCode::Jmp, "jmp".into()),
            (OpCode::EndRepeat, "endrepeat".into()),
            (OpCode::JmpIfZ, "jmpifz".into()),
            (OpCode::LocalCall, "localcall".into()),
            (OpCode::ExtCall, "extcall".into()),
            (OpCode::ObjCallV4, "objcallv4".into()),
            (OpCode::Put, "put".into()),
            (OpCode::PutChunk, "putchunk".into()),
            (OpCode::DeleteChunk, "deletechunk".into()),
            (OpCode::Get, "get".into()),
            (OpCode::Set, "set".into()),
            (OpCode::GetMovieProp, "getmovieprop".into()),
            (OpCode::SetMovieProp, "setmovieprop".into()),
            (OpCode::GetObjProp, "getobjprop".into()),
            (OpCode::SetObjProp, "setobjprop".into()),
            (OpCode::TellCall, "tellcall".into()),
            (OpCode::Peek, "peek".into()),
            (OpCode::Pop, "pop".into()),
            (OpCode::TheBuiltin, "thebuiltin".into()),
            (OpCode::ObjCall, "objcall".into()),
            (OpCode::PushChunkVarRef, "pushchunkvarref".into()),
            (OpCode::PushInt16, "pushint16".into()),
            (OpCode::PushInt32, "pushint32".into()),
            (OpCode::GetChainedProp, "getchainedprop".into()),
            (OpCode::PushFloat32, "pushfloat32".into()),
            (OpCode::GetTopLevelProp, "gettoplevelprop".into()),
            (OpCode::NewObj, "newobj".into()),
        ])
    })
}

fn anim_prop_names() -> &'static HashMap<u16, Box<str>> {
    static MAP: OnceLock<HashMap<u16, Box<str>>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (0x01, "beepOn".into()),
            (0x02, "buttonStyle".into()),
            (0x03, "centerStage".into()),
            (0x04, "checkBoxAccess".into()),
            (0x05, "checkboxType".into()),
            (0x06, "colorDepth".into()),
            (0x07, "colorQD".into()),
            (0x08, "exitLock".into()),
            (0x09, "fixStageSize".into()),
            (0x0a, "fullColorPermit".into()),
            (0x0b, "imageDirect".into()),
            (0x0c, "doubleClick".into()),
            (0x0d, "key".into()),
            (0x0e, "lastClick".into()),
            (0x0f, "lastEvent".into()),
            (0x10, "keyCode".into()),
            (0x11, "lastKey".into()),
            (0x12, "lastRoll".into()),
            (0x13, "timeoutLapsed".into()),
            (0x14, "multiSound".into()),
            (0x15, "pauseState".into()),
            (0x16, "quickTimePresent".into()),
            (0x17, "selEnd".into()),
            (0x18, "selStart".into()),
            (0x19, "soundEnabled".into()),
            (0x1a, "soundLevel".into()),
            (0x1b, "stageColor".into()),
            // 0x1c indicates dontPassEvent was called.
            // It doesn't seem to have a Lingo-accessible name.
            (0x1d, "switchColorDepth".into()),
            (0x1e, "timeoutKeyDown".into()),
            (0x1f, "timeoutLength".into()),
            (0x20, "timeoutMouse".into()),
            (0x21, "timeoutPlay".into()),
            (0x22, "timer".into()),
            (0x23, "preLoadRAM".into()),
            (0x24, "videoForWindowsPresent".into()),
            (0x25, "netPresent".into()),
            (0x26, "safePlayer".into()),
            (0x27, "soundKeepDevice".into()),
            (0x28, "soundMixMedia".into()),
        ])
    })
}

fn anim2_prop_names() -> &'static HashMap<u16, Box<str>> {
    static MAP: OnceLock<HashMap<u16, Box<str>>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (0x01, "perFrameHook".into()),
            (0x02, "number of castMembers".into()),
            (0x03, "number of menus".into()),
            (0x04, "number of castLibs".into()),
            (0x05, "number of xtras".into()),
        ])
    })
}

pub fn movie_prop_names() -> &'static HashMap<u16, Box<str>> {
    static MAP: OnceLock<HashMap<u16, Box<str>>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (0x00, "floatPrecision".into()),
            (0x01, "mouseDownScript".into()),
            (0x02, "mouseUpScript".into()),
            (0x03, "keyDownScript".into()),
            (0x04, "keyUpScript".into()),
            (0x05, "timeoutScript".into()),
            (0x06, "short time".into()),
            (0x07, "abbr time".into()),
            (0x08, "long time".into()),
            (0x09, "short date".into()),
            (0x0a, "abbr date".into()),
            (0x0b, "long date".into()),
        ])
    })
}

pub fn sprite_prop_names() -> &'static HashMap<u16, Box<str>> {
    static MAP: OnceLock<HashMap<u16, Box<str>>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (0x01, "type".into()),
            (0x02, "backColor".into()),
            (0x03, "bottom".into()),
            (0x04, "castNum".into()),
            (0x05, "constraint".into()),
            (0x06, "cursor".into()),
            (0x07, "foreColor".into()),
            (0x08, "height".into()),
            (0x09, "immediate".into()),
            (0x0a, "ink".into()),
            (0x0b, "left".into()),
            (0x0c, "lineSize".into()),
            (0x0d, "locH".into()),
            (0x0e, "locV".into()),
            (0x0f, "movieRate".into()),
            (0x10, "movieTime".into()),
            (0x11, "pattern".into()),
            (0x12, "puppet".into()),
            (0x13, "right".into()),
            (0x14, "startTime".into()),
            (0x15, "stopTime".into()),
            (0x16, "stretch".into()),
            (0x17, "top".into()),
            (0x18, "trails".into()),
            (0x19, "visible".into()),
            (0x1a, "volume".into()),
            (0x1b, "width".into()),
            (0x1c, "blend".into()),
            (0x1d, "scriptNum".into()),
            (0x1e, "moveableSprite".into()),
            (0x1f, "editableText".into()),
            (0x20, "scoreColor".into()),
            (0x21, "loc".into()),
            (0x22, "rect".into()),
            (0x23, "memberNum".into()),
            (0x24, "castLibNum".into()),
            (0x25, "member".into()),
            (0x26, "scriptInstanceList".into()),
            (0x27, "currentTime".into()),
            (0x28, "mostRecentCuePoint".into()),
            (0x29, "tweened".into()),
            (0x2a, "name".into()),
        ])
    })
}

#[inline]
pub fn get_opcode_name(opcode: OpCode) -> &'static str {
    opcode_names().get(&opcode).unwrap().as_ref()
}

#[inline]
pub fn get_anim_prop_name(name_id: u16) -> &'static str {
    anim_prop_names().get(&name_id).unwrap().as_ref()
}

#[inline]
pub fn get_anim2_prop_name(name_id: u16) -> &'static str {
    anim2_prop_names().get(&name_id).unwrap().as_ref()
}

pub fn get_sprite_prop_name(name_id: u16) -> &'static str {
    sprite_prop_names().get(&name_id).unwrap().as_ref()
}
