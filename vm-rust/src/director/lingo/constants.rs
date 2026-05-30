use std::{collections::HashMap, sync::OnceLock};

use crate::player::symbols::{builtin::BuiltInSymbol, symbol::Symbol};

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

fn anim_prop_names() -> &'static HashMap<u16, BuiltInSymbol> {
    static MAP: OnceLock<HashMap<u16, BuiltInSymbol>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (0x01, BuiltInSymbol::BeepOn),
            (0x02, BuiltInSymbol::ButtonStyle),
            (0x03, BuiltInSymbol::CenterStage),
            (0x04, BuiltInSymbol::CheckBoxAccess),
            (0x05, BuiltInSymbol::CheckBoxType),
            (0x06, BuiltInSymbol::ColorDepth),
            (0x07, BuiltInSymbol::ColorQD),
            (0x08, BuiltInSymbol::ExitLock),
            (0x09, BuiltInSymbol::FixStageSize),
            (0x0a, BuiltInSymbol::FullColorPermit),
            (0x0b, BuiltInSymbol::ImageDirect),
            (0x0c, BuiltInSymbol::DoubleClick),
            (0x0d, BuiltInSymbol::Key),
            (0x0e, BuiltInSymbol::LastClick),
            (0x0f, BuiltInSymbol::LastEvent),
            (0x10, BuiltInSymbol::KeyCode),
            (0x11, BuiltInSymbol::LastKey),
            (0x12, BuiltInSymbol::LastRoll),
            (0x13, BuiltInSymbol::TimeoutLapsed),
            (0x14, BuiltInSymbol::MultiSound),
            (0x15, BuiltInSymbol::PauseState),
            (0x16, BuiltInSymbol::QuickTimePresent),
            (0x17, BuiltInSymbol::SelEnd),
            (0x18, BuiltInSymbol::SelStart),
            (0x19, BuiltInSymbol::SoundEnabled),
            (0x1a, BuiltInSymbol::SoundLevel),
            (0x1b, BuiltInSymbol::StageColor),
            // 0x1c indicates dontPassEvent was called.
            // It doesn't seem to have a Lingo-accessible name.
            (0x1d, BuiltInSymbol::SwitchColorDepth),
            (0x1e, BuiltInSymbol::TimeoutKeyDown),
            (0x1f, BuiltInSymbol::TimeoutLength),
            (0x20, BuiltInSymbol::TimeoutMouse),
            (0x21, BuiltInSymbol::TimeoutPlay),
            (0x22, BuiltInSymbol::Timer),
            (0x23, BuiltInSymbol::PreLoadRAM),
            (0x24, BuiltInSymbol::VideoForWindowsPresent),
            (0x25, BuiltInSymbol::NetPresent),
            (0x26, BuiltInSymbol::SafePlayer),
            (0x27, BuiltInSymbol::SoundKeepDevice),
            (0x28, BuiltInSymbol::SoundMixMedia),
        ])
    })
}

fn anim2_prop_names() -> &'static HashMap<u16, BuiltInSymbol> {
    static MAP: OnceLock<HashMap<u16, BuiltInSymbol>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (0x01, BuiltInSymbol::PerFrameHook),
            (0x02, BuiltInSymbol::NumberOfCastMembers),
            (0x03, BuiltInSymbol::NumberOfMenus),
            (0x04, BuiltInSymbol::NumberOfCastLibs),
            (0x05, BuiltInSymbol::NumberOfXtras),
        ])
    })
}

pub fn movie_prop_names() -> &'static HashMap<u16, BuiltInSymbol> {
    static MAP: OnceLock<HashMap<u16, BuiltInSymbol>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (0x00, BuiltInSymbol::FloatPrecision),
            (0x01, BuiltInSymbol::MouseDownScript),
            (0x02, BuiltInSymbol::MouseUpScript),
            (0x03, BuiltInSymbol::KeyDownScript),
            (0x04, BuiltInSymbol::KeyUpScript),
            (0x05, BuiltInSymbol::TimeoutScript),
            (0x06, BuiltInSymbol::ShortTime),
            (0x07, BuiltInSymbol::AbbrTime),
            (0x08, BuiltInSymbol::LongTime),
            (0x09, BuiltInSymbol::ShortDate),
            (0x0a, BuiltInSymbol::AbbrDate),
            (0x0b, BuiltInSymbol::LongDate),
        ])
    })
}

pub fn sprite_prop_names() -> &'static HashMap<u16, BuiltInSymbol> {
    static MAP: OnceLock<HashMap<u16, BuiltInSymbol>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (0x01, BuiltInSymbol::Type),
            (0x02, BuiltInSymbol::BackColor),
            (0x03, BuiltInSymbol::Bottom),
            (0x04, BuiltInSymbol::CastNum),
            (0x05, BuiltInSymbol::Constraint),
            (0x06, BuiltInSymbol::Cursor),
            (0x07, BuiltInSymbol::ForeColor),
            (0x08, BuiltInSymbol::Height),
            (0x09, BuiltInSymbol::Immediate),
            (0x0a, BuiltInSymbol::Ink),
            (0x0b, BuiltInSymbol::Left),
            (0x0c, BuiltInSymbol::LineSize),
            (0x0d, BuiltInSymbol::LocH),
            (0x0e, BuiltInSymbol::LocV),
            (0x0f, BuiltInSymbol::MovieRate),
            (0x10, BuiltInSymbol::MovieTime),
            (0x11, BuiltInSymbol::Pattern),
            (0x12, BuiltInSymbol::Puppet),
            (0x13, BuiltInSymbol::Right),
            (0x14, BuiltInSymbol::StartTime),
            (0x15, BuiltInSymbol::StopTime),
            (0x16, BuiltInSymbol::Stretch),
            (0x17, BuiltInSymbol::Top),
            (0x18, BuiltInSymbol::Trails),
            (0x19, BuiltInSymbol::Visible),
            (0x1a, BuiltInSymbol::Volume),
            (0x1b, BuiltInSymbol::Width),
            (0x1c, BuiltInSymbol::Blend),
            (0x1d, BuiltInSymbol::ScriptNum),
            (0x1e, BuiltInSymbol::MoveableSprite),
            (0x1f, BuiltInSymbol::EditableText),
            (0x20, BuiltInSymbol::ScoreColor),
            (0x21, BuiltInSymbol::Loc),
            (0x22, BuiltInSymbol::Rect),
            (0x23, BuiltInSymbol::MemberNum),
            (0x24, BuiltInSymbol::CastLibNum),
            (0x25, BuiltInSymbol::Member),
            (0x26, BuiltInSymbol::ScriptInstanceList),
            (0x27, BuiltInSymbol::CurrentTime),
            (0x28, BuiltInSymbol::MostRecentCuePoint),
            (0x29, BuiltInSymbol::Tweened),
            (0x2a, BuiltInSymbol::Name),
        ])
    })
}

#[inline]
pub fn get_opcode_name(opcode: OpCode) -> &'static str {
    opcode_names().get(&opcode).unwrap().as_ref()
}

#[inline]
pub fn get_anim_prop_name(name_id: u16) -> BuiltInSymbol {
    *anim_prop_names().get(&name_id).unwrap()
}

#[inline]
pub fn get_anim2_prop_name(name_id: u16) -> BuiltInSymbol {
    *anim2_prop_names().get(&name_id).unwrap()
}

pub fn get_sprite_prop_name(name_id: u16) -> BuiltInSymbol {
    *sprite_prop_names().get(&name_id).unwrap()
}

fn cast_member_prop_names() -> &'static HashMap<u16, BuiltInSymbol> {
    static MAP: OnceLock<HashMap<u16, BuiltInSymbol>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (0x01, BuiltInSymbol::Name),
            (0x02, BuiltInSymbol::Text),
            (0x03, BuiltInSymbol::FontStyle),
            (0x04, BuiltInSymbol::Font),
            (0x05, BuiltInSymbol::Height),
            (0x06, BuiltInSymbol::Alignment),
            (0x07, BuiltInSymbol::FontSize),
            (0x08, BuiltInSymbol::Picture),
            (0x09, BuiltInSymbol::Hilite),
            (0x0a, BuiltInSymbol::Number),
            (0x0b, BuiltInSymbol::Size),
            (0x11, BuiltInSymbol::ForeColor),
            (0x12, BuiltInSymbol::BackColor),
        ])
    })
}

pub fn get_cast_member_prop_name(name_id: u16) -> BuiltInSymbol {
    *cast_member_prop_names().get(&name_id).unwrap_or(&BuiltInSymbol::Unknown)
}

pub fn get_sound_prop_name(property_id: u16) -> BuiltInSymbol {
    match property_id {
        0x01 => BuiltInSymbol::Volume,
        0x02 => BuiltInSymbol::Pan,
        0x03 => BuiltInSymbol::LoopCount,
        0x04 => BuiltInSymbol::StartTime,
        0x05 => BuiltInSymbol::EndTime,
        0x06 => BuiltInSymbol::LoopStartTime,
        0x07 => BuiltInSymbol::LoopEndTime,
        _ => BuiltInSymbol::UnknownSoundProp,
    }
}