// Lingo decompiler enums
// Ported from ProjectorRays

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BytecodeTag {
    #[default]
    None,
    Skip,
    RepeatWhile,
    RepeatWithIn,
    RepeatWithTo,
    RepeatWithDownTo,
    NextRepeatTarget,
    EndCase,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DatumType {
    Void,
    Symbol,
    VarRef,
    String,
    Int,
    Float,
    List,
    ArgList,
    ArgListNoRet,
    PropList,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChunkExprType {
    Char = 0x01,
    Word = 0x02,
    Item = 0x03,
    Line = 0x04,
}

impl ChunkExprType {
    pub fn name(&self) -> &'static str {
        match self {
            ChunkExprType::Char => "char",
            ChunkExprType::Word => "word",
            ChunkExprType::Item => "item",
            ChunkExprType::Line => "line",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PutType {
    Into = 0x01,
    After = 0x02,
    Before = 0x03,
}

impl PutType {
    pub fn name(&self) -> &'static str {
        match self {
            PutType::Into => "into",
            PutType::After => "after",
            PutType::Before => "before",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaseExpect {
    End,
    Or,
    Next,
    Otherwise,
}
