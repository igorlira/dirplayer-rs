use std::collections::HashMap;

use lazy_static::lazy_static;

use super::opcode::OpCode;

lazy_static! {
  pub static ref OPCODE_NAMES: HashMap<OpCode, String> = HashMap::from([
    // single-byte
    (OpCode::Ret, "ret".to_owned()),
    (OpCode::RetFactory, "retfactory".to_owned()),
    (OpCode::Mul, "mul".to_owned()),
    (OpCode::PushZero, "pushzero".to_owned()),
    (OpCode::Add, "add".to_owned()),
    (OpCode::Sub, "sub".to_owned()),
    (OpCode::Div, "div".to_owned()),
    (OpCode::Mod, "mod".to_owned()),
    (OpCode::Inv, "inv".to_owned()),
    (OpCode::JoinStr, "joinstr".to_owned()),
    (OpCode::JoinPadStr, "joinpadstr".to_owned()),
    (OpCode::Lt, "lt".to_owned()),
    (OpCode::LtEq, "lteq".to_owned()),
    (OpCode::NtEq, "nteq".to_owned()),
    (OpCode::Eq, "eq".to_owned()),
    (OpCode::Gt, "gt".to_owned()),
    (OpCode::GtEq, "gteq".to_owned()),
    (OpCode::And, "and".to_owned()),
    (OpCode::Or, "or".to_owned()),
    (OpCode::Not, "not".to_owned()),
    (OpCode::ContainsStr, "containsstr".to_owned()),
    (OpCode::Contains0Str, "contains0str".to_owned()),
    (OpCode::GetChunk, "getchunk".to_owned()),
    (OpCode::HiliteChunk, "hilitechunk".to_owned()),
    (OpCode::OntoSpr, "ontospr".to_owned()),
    (OpCode::IntoSpr, "intospr".to_owned()),
    (OpCode::GetField, "getfield".to_owned()),
    (OpCode::StartTell, "starttell".to_owned()),
    (OpCode::EndTell, "endtell".to_owned()),
    (OpCode::PushList, "pushlist".to_owned()),
    (OpCode::PushPropList, "pushproplist".to_owned()),
    (OpCode::Swap, "swap".to_owned()),
    (OpCode::CallJavaScript, "calljavascript".to_owned()),

    // multi-byte
    (OpCode::PushInt8, "pushint8".to_owned()),
    (OpCode::PushArgListNoRet, "pusharglistnoret".to_owned()),
    (OpCode::PushArgList, "pusharglist".to_owned()),
    (OpCode::PushCons, "pushcons".to_owned()),
    (OpCode::PushSymb, "pushsymb".to_owned()),
    (OpCode::PushVarRef, "pushvarref".to_owned()),
    (OpCode::GetGlobal2, "getglobal2".to_owned()),
    (OpCode::GetGlobal, "getglobal".to_owned()),
    (OpCode::GetProp, "getprop".to_owned()),
    (OpCode::GetParam, "getparam".to_owned()),
    (OpCode::GetLocal, "getlocal".to_owned()),
    (OpCode::SetGlobal2, "setglobal2".to_owned()),
    (OpCode::SetGlobal, "setglobal".to_owned()),
    (OpCode::SetProp, "setprop".to_owned()),
    (OpCode::SetParam, "setparam".to_owned()),
    (OpCode::SetLocal, "setlocal".to_owned()),
    (OpCode::Jmp, "jmp".to_owned()),
    (OpCode::EndRepeat, "endrepeat".to_owned()),
    (OpCode::JmpIfZ, "jmpifz".to_owned()),
    (OpCode::LocalCall, "localcall".to_owned()),
    (OpCode::ExtCall, "extcall".to_owned()),
    (OpCode::ObjCallV4, "objcallv4".to_owned()),
    (OpCode::Put, "put".to_owned()),
    (OpCode::PutChunk, "putchunk".to_owned()),
    (OpCode::DeleteChunk, "deletechunk".to_owned()),
    (OpCode::Get, "get".to_owned()),
    (OpCode::Set, "set".to_owned()),
    (OpCode::GetMovieProp, "getmovieprop".to_owned()),
    (OpCode::SetMovieProp, "setmovieprop".to_owned()),
    (OpCode::GetObjProp, "getobjprop".to_owned()),
    (OpCode::SetObjProp, "setobjprop".to_owned()),
    (OpCode::TellCall, "tellcall".to_owned()),
    (OpCode::Peek, "peek".to_owned()),
    (OpCode::Pop, "pop".to_owned()),
    (OpCode::TheBuiltin, "thebuiltin".to_owned()),
    (OpCode::ObjCall, "objcall".to_owned()),
    (OpCode::PushChunkVarRef, "pushchunkvarref".to_owned()),
    (OpCode::PushInt16, "pushint16".to_owned()),
    (OpCode::PushInt32, "pushint32".to_owned()),
    (OpCode::GetChainedProp, "getchainedprop".to_owned()),
    (OpCode::PushFloat32, "pushfloat32".to_owned()),
    (OpCode::GetTopLevelProp, "gettoplevelprop".to_owned()),
    (OpCode::NewObj, "newobj".to_owned()),
  ]);

  pub static ref ANIM_PROP_NAMES: HashMap<u16, String> = HashMap::from([
    (0x01, "beepOn".to_owned()),
    (0x02, "buttonStyle".to_owned()),
    (0x03, "centerStage".to_owned()),
    (0x04, "checkBoxAccess".to_owned()),
    (0x05, "checkboxType".to_owned()),
    (0x06, "colorDepth".to_owned()),
    (0x07, "colorQD".to_owned()),
    (0x08, "exitLock".to_owned()),
    (0x09, "fixStageSize".to_owned()),
    (0x0a, "fullColorPermit".to_owned()),
    (0x0b, "imageDirect".to_owned()),
    (0x0c, "doubleClick".to_owned()),
    (0x0d, "key".to_owned()),
    (0x0e, "lastClick".to_owned()),
    (0x0f, "lastEvent".to_owned()),
    (0x10, "keyCode".to_owned()),
    (0x11, "lastKey".to_owned()),
    (0x12, "lastRoll".to_owned()),
    (0x13, "timeoutLapsed".to_owned()),
    (0x14, "multiSound".to_owned()),
    (0x15, "pauseState".to_owned()),
    (0x16, "quickTimePresent".to_owned()),
    (0x17, "selEnd".to_owned()),
    (0x18, "selStart".to_owned()),
    (0x19, "soundEnabled".to_owned()),
    (0x1a, "soundLevel".to_owned()),
    (0x1b, "stageColor".to_owned()),
    // 0x1c indicates dontPassEvent was called.
    // It doesn't seem to have a Lingo-accessible name.
    (0x1d, "switchColorDepth".to_owned()),
    (0x1e, "timeoutKeyDown".to_owned()),
    (0x1f, "timeoutLength".to_owned()),
    (0x20, "timeoutMouse".to_owned()),
    (0x21, "timeoutPlay".to_owned()),
    (0x22, "timer".to_owned()),
    (0x23, "preLoadRAM".to_owned()),
    (0x24, "videoForWindowsPresent".to_owned()),
    (0x25, "netPresent".to_owned()),
    (0x26, "safePlayer".to_owned()),
    (0x27, "soundKeepDevice".to_owned()),
    (0x28, "soundMixMedia".to_owned()),
  ]);

  pub static ref ANIM2_PROP_NAMES: HashMap<u16, String> = HashMap::from([
    (0x01, "perFramework".to_owned()),
    (0x02, "number of castMembers".to_owned()),
    (0x03, "number of menus".to_owned()),
    (0x04, "number of castLibs".to_owned()),
    (0x05, "number of xtras".to_owned()),
  ]);

  pub static ref MOVIE_PROP_NAMES: HashMap<u16, String> = HashMap::from([
    (0x00, "floatPrecision".to_owned()),
    (0x01, "mouseDownScript".to_owned()),
    (0x02, "mouseUpScript".to_owned()),
    (0x03, "keyDownScript".to_owned()),
    (0x04, "keyUpScript".to_owned()),
    (0x05, "timeoutScript".to_owned()),
    (0x06, "short time".to_owned()),
    (0x07, "abbr time".to_owned()),
    (0x08, "long time".to_owned()),
    (0x09, "short date".to_owned()),
    (0x0a, "abbr date".to_owned()),
    (0x0b, "long date".to_owned()),
  ]);
}

pub fn get_opcode_name(opcode: &OpCode) -> String {
  OPCODE_NAMES.get(opcode).unwrap().to_owned()
}

pub fn get_anim_prop_name(name_id: u16) -> String {
  ANIM_PROP_NAMES.get(&name_id).unwrap().to_owned()
}

pub fn get_anim2_prop_name(name_id: u16) -> String {
  ANIM2_PROP_NAMES.get(&name_id).unwrap().to_owned()
}
