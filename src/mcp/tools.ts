// MCP Tool definitions for DirPlayer VM debugging

export interface McpTool {
  name: string;
  description: string;
  inputSchema: {
    type: 'object';
    properties: Record<string, {
      type: string;
      description: string;
    }>;
    required: string[];
  };
}

export const mcpTools: McpTool[] = [
  // Script tools
  {
    name: 'list_scripts',
    description: 'List Lingo scripts in the movie with their cast references and handler names. Supports pagination and filtering by cast library.',
    inputSchema: {
      type: 'object',
      properties: {
        cast_lib: { type: 'number', description: 'Filter to a specific cast library number (omit or -1 for all libraries)' },
        limit: { type: 'number', description: 'Maximum number of scripts to return (omit or -1 for all)' },
        offset: { type: 'number', description: 'Number of scripts to skip for pagination (omit or -1 for none)' }
      },
      required: []
    }
  },
  {
    name: 'get_script',
    description: 'Get detailed information about a specific script including handlers, arguments, locals, and properties',
    inputSchema: {
      type: 'object',
      properties: {
        cast_lib: { type: 'number', description: 'Cast library number (1-indexed)' },
        cast_member: { type: 'number', description: 'Cast member number' }
      },
      required: ['cast_lib', 'cast_member']
    }
  },
  {
    name: 'disassemble_handler',
    description: 'Get bytecode disassembly for a handler, showing low-level opcodes and operands',
    inputSchema: {
      type: 'object',
      properties: {
        cast_lib: { type: 'number', description: 'Cast library number' },
        cast_member: { type: 'number', description: 'Cast member number' },
        handler_name: { type: 'string', description: 'Handler name (e.g., "mouseDown", "enterFrame")' }
      },
      required: ['cast_lib', 'cast_member', 'handler_name']
    }
  },
  {
    name: 'decompile_handler',
    description: 'Get decompiled Lingo source code for a handler',
    inputSchema: {
      type: 'object',
      properties: {
        cast_lib: { type: 'number', description: 'Cast library number' },
        cast_member: { type: 'number', description: 'Cast member number' },
        handler_name: { type: 'string', description: 'Handler name (e.g., "mouseDown", "enterFrame")' }
      },
      required: ['cast_lib', 'cast_member', 'handler_name']
    }
  },

  // Execution tools
  {
    name: 'get_console_output',
    description: 'Get the last N lines of debug console output from the player',
    inputSchema: {
      type: 'object',
      properties: {
        last_n_lines: { type: 'number', description: 'Number of last lines to retrieve from console output' }
      },
      required: ['last_n_lines']
    }
  },
  {
    name: 'get_call_stack',
    description: 'Get the current call stack. By default returns a lightweight summary (handler names, script refs, bytecode positions). Use include_locals=true to also get local variables and arguments.',
    inputSchema: {
      type: 'object',
      properties: {
        depth: { type: 'number', description: 'Maximum number of scopes to return from the top of the stack (omit or -1 for all scopes)' },
        include_locals: { type: 'boolean', description: 'Whether to include local variables and arguments for each scope (default: false)' }
      },
      required: []
    }
  },
  {
    name: 'get_context',
    description: 'Get a lightweight execution context: current frame, handler name, script name, bytecode position, and player state. Single call replacement for get_execution_state + get_call_stack when you just need current position.',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },
  {
    name: 'get_execution_state',
    description: 'Get player execution state including: is_playing, is_paused, current_frame, total_frames, at_breakpoint, movie_title, stage dimensions',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },
  {
    name: 'eval_lingo',
    description: 'Evaluate a Lingo expression or command and return the result. Can be used to inspect variables or execute code.',
    inputSchema: {
      type: 'object',
      properties: {
        code: { type: 'string', description: 'Lingo code to evaluate (e.g., "put the stage", "put gMyGlobal")' }
      },
      required: ['code']
    }
  },
  {
    name: 'pause',
    description: 'Pause script execution',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },
  {
    name: 'resume',
    description: 'Resume execution from pause or breakpoint',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },
  {
    name: 'step_over',
    description: 'Step over one bytecode instruction (does not enter handler calls)',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },
  {
    name: 'step_into',
    description: 'Step into a handler call',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },
  {
    name: 'step_out',
    description: 'Step out of the current handler',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },

  // Variable tools
  {
    name: 'get_globals',
    description: 'Get all global variables with their current values, types, and datum IDs',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },
  {
    name: 'get_locals',
    description: 'Get local variables and arguments in a specific scope (defaults to current scope)',
    inputSchema: {
      type: 'object',
      properties: {
        scope_index: { type: 'number', description: 'Scope index (0 = bottom of stack, -1 or omit for current scope)' }
      },
      required: []
    }
  },
  {
    name: 'inspect_datum',
    description: 'Inspect a datum by its ID, showing type, value, and properties (for objects/lists)',
    inputSchema: {
      type: 'object',
      properties: {
        datum_id: { type: 'number', description: 'Datum ID (can be obtained from get_globals, get_locals, etc.)' }
      },
      required: ['datum_id']
    }
  },

  // Cast tools
  {
    name: 'list_cast_libs',
    description: 'List all cast libraries in the movie with their names and member/script counts',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },
  {
    name: 'list_cast_members',
    description: 'List all cast members, optionally filtered by cast library',
    inputSchema: {
      type: 'object',
      properties: {
        cast_lib: { type: 'number', description: 'Cast library number to filter (omit or -1 for all libraries)' }
      },
      required: []
    }
  },
  {
    name: 'inspect_cast_member',
    description: 'Get detailed information about a cast member including type, name, and script info if applicable',
    inputSchema: {
      type: 'object',
      properties: {
        cast_lib: { type: 'number', description: 'Cast library number' },
        cast_member: { type: 'number', description: 'Cast member number' }
      },
      required: ['cast_lib', 'cast_member']
    }
  },

  // Breakpoint tools
  {
    name: 'set_breakpoint',
    description: 'Set a breakpoint at a specific bytecode position in a handler',
    inputSchema: {
      type: 'object',
      properties: {
        script_name: { type: 'string', description: 'Script name' },
        handler_name: { type: 'string', description: 'Handler name' },
        bytecode_index: { type: 'number', description: 'Bytecode index (use disassemble_handler to find valid indices)' }
      },
      required: ['script_name', 'handler_name', 'bytecode_index']
    }
  },
  {
    name: 'remove_breakpoint',
    description: 'Remove a breakpoint',
    inputSchema: {
      type: 'object',
      properties: {
        script_name: { type: 'string', description: 'Script name' },
        handler_name: { type: 'string', description: 'Handler name' },
        bytecode_index: { type: 'number', description: 'Bytecode index' }
      },
      required: ['script_name', 'handler_name', 'bytecode_index']
    }
  },
  {
    name: 'list_breakpoints',
    description: 'List all active breakpoints',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  }
];

// Tool name type for type safety
export type McpToolName =
  | 'list_scripts'
  | 'get_script'
  | 'disassemble_handler'
  | 'decompile_handler'
  | 'get_console_output'
  | 'get_call_stack'
  | 'get_context'
  | 'get_execution_state'
  | 'eval_lingo'
  | 'pause'
  | 'resume'
  | 'step_over'
  | 'step_into'
  | 'step_out'
  | 'get_globals'
  | 'get_locals'
  | 'inspect_datum'
  | 'list_cast_libs'
  | 'list_cast_members'
  | 'inspect_cast_member'
  | 'set_breakpoint'
  | 'remove_breakpoint'
  | 'list_breakpoints';
