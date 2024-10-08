use num_derive::{FromPrimitive, ToPrimitive};

#[derive(Copy, Clone, FromPrimitive, ToPrimitive, PartialEq, Eq, Hash)]
pub enum OpCode {
	// single-byte
	Invalid = 0x0,
	Ret = 0x01,
	RetFactory = 0x02,
	PushZero = 0x03,
	Mul = 0x04,
	Add = 0x05,
	Sub = 0x06,
	Div = 0x07,
	Mod = 0x08,
	Inv = 0x09,
	JoinStr = 0x0a,
	JoinPadStr = 0x0b,
	Lt = 0x0c,
	LtEq = 0x0d,
	NtEq = 0x0e,
	Eq = 0x0f,
	Gt = 0x10,
	GtEq = 0x11,
	And = 0x12,
	Or = 0x13,
	Not = 0x14,
	ContainsStr = 0x15,
	Contains0Str = 0x16,
	GetChunk = 0x17,
	HiliteChunk = 0x18,
	OntoSpr = 0x19,
	IntoSpr = 0x1a,
	GetField = 0x1b,
	StartTell = 0x1c,
	EndTell = 0x1d,
	PushList = 0x1e,
	PushPropList = 0x1f,
	Swap = 0x21,
	CallJavaScript = 0x26,

	// multi-byte
	PushInt8 = 0x41,
	PushArgListNoRet = 0x42,
	PushArgList = 0x43,
	PushCons = 0x44,
	PushSymb = 0x45,
	PushVarRef = 0x46,
	GetGlobal2 = 0x48,
	GetGlobal = 0x49,
	GetProp = 0x4a,
	GetParam = 0x4b,
	GetLocal = 0x4c,
	SetGlobal2 = 0x4e,
	SetGlobal = 0x4f,
	SetProp = 0x50,
	SetParam = 0x51,
	SetLocal = 0x52,
	Jmp = 0x53,
	EndRepeat = 0x54,
	JmpIfZ = 0x55,
	LocalCall = 0x56,
	ExtCall = 0x57,
	ObjCallV4 = 0x58,
	Put = 0x59,
	PutChunk = 0x5a,
	DeleteChunk = 0x5b,
	Get = 0x5c,
	Set = 0x5d,
	GetMovieProp = 0x5f,
	SetMovieProp = 0x60,
	GetObjProp = 0x61,
	SetObjProp = 0x62,
	TellCall = 0x63,
	Peek = 0x64,
	Pop = 0x65,
	TheBuiltin = 0x66,
	ObjCall = 0x67,
	PushChunkVarRef = 0x6d,
	PushInt16 = 0x6e,
	PushInt32 = 0x6f,
	GetChainedProp = 0x70,
	PushFloat32 = 0x71,
	GetTopLevelProp = 0x72,
	NewObj = 0x73
}

impl From<u16> for OpCode {
	fn from(value: u16) -> Self {
		if let Some(result) = num::FromPrimitive::from_u16(value) {
			result
		} else {
			panic!("Invalid OpCode: {}", value)
		}
	}
}
