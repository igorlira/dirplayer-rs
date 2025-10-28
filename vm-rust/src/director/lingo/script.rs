use std::collections::HashMap;

use crate::director::chunks::script::ScriptChunk;
#[derive(Clone)]
pub struct ScriptContext {
    pub names: Vec<String>,
    pub scripts: HashMap<u32, ScriptChunk>,
}
