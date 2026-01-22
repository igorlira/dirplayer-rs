use manual_future::ManualFutureCompleter;

use crate::js_api::JsApi;

use super::{cast_lib::CastMemberRef, script::ScriptHandlerRef};

/// Represents the current step debugging mode
#[derive(Clone, PartialEq, Debug)]
pub enum StepMode {
    /// No step debugging active - normal execution
    None,
    /// Step Into: Stop at the next bytecode instruction (enters function calls)
    Into,
    /// Step Into Line: Stop at the next line (enters function calls)
    IntoLine { skip_bytecode_indices: Vec<usize> },
    /// Step Over: Stop at the next bytecode in the current scope (skips function calls)
    Over,
    /// Step Over Line: Stop at the next line (not in skip_bytecode_indices) in the current scope
    OverLine { skip_bytecode_indices: Vec<usize> },
    /// Step Out: Stop when returning from the current scope
    Out,
}

impl Default for StepMode {
    fn default() -> Self {
        StepMode::None
    }
}

#[derive(Clone)]
pub struct Breakpoint {
    pub script_name: String,
    pub handler_name: String,
    pub bytecode_index: usize,
}

pub struct BreakpointContext {
    pub breakpoint: Breakpoint,
    pub script_ref: CastMemberRef,
    pub handler_ref: ScriptHandlerRef,
    pub bytecode_index: usize,
    pub completer: ManualFutureCompleter<()>,
}

pub struct BreakpointManager {
    pub breakpoints: Vec<Breakpoint>,
}

impl BreakpointManager {
    pub fn new() -> BreakpointManager {
        BreakpointManager {
            breakpoints: vec![],
        }
    }

    pub fn add_breakpoint(
        &mut self,
        script_name: String,
        handler_name: String,
        bytecode_index: usize,
    ) {
        self.breakpoints.push(Breakpoint {
            script_name,
            handler_name,
            bytecode_index,
        });
        JsApi::dispatch_breakpoint_list_changed();
    }

    pub fn remove_breakpoint(
        &mut self,
        script_name: String,
        handler_name: String,
        bytecode_index: usize,
    ) {
        self.breakpoints.retain(|bp| {
            bp.script_name != script_name
                || bp.handler_name != handler_name
                || bp.bytecode_index != bytecode_index
        });
        JsApi::dispatch_breakpoint_list_changed();
    }

    pub fn toggle_breakpoint(
        &mut self,
        script_name: String,
        handler_name: String,
        bytecode_index: usize,
    ) {
        if self.has_breakpoint(&script_name, &handler_name, bytecode_index) {
            self.remove_breakpoint(script_name, handler_name, bytecode_index);
        } else {
            self.add_breakpoint(script_name, handler_name, bytecode_index);
        }
    }

    pub fn has_breakpoint(
        &self,
        script_name: &String,
        handler_name: &String,
        bytecode_index: usize,
    ) -> bool {
        self.breakpoints.iter().any(|bp| {
            bp.script_name == *script_name
                && bp.handler_name == *handler_name
                && bp.bytecode_index == bytecode_index
        })
    }

    pub fn find_breakpoint_for_bytecode(
        &self,
        script_name: &String,
        handler_name: &String,
        bytecode_index: usize,
    ) -> Option<&Breakpoint> {
        self.breakpoints.iter().find(|bp| {
            bp.script_name == *script_name
                && bp.handler_name == *handler_name
                && bp.bytecode_index == bytecode_index
        })
    }
}
