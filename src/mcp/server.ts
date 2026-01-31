// MCP Server for DirPlayer VM debugging
// Listens on WebSocket and provides VM debugging tools to Claude Code

import { mcpTools, McpToolName } from './tools';

// Import vm-rust WASM module functions
// These will be available after WASM initialization
type WasmModule = typeof import('vm-rust');

const MCP_PORT = 9847;

// MCP Protocol types
interface McpRequest {
  jsonrpc: '2.0';
  id: string | number;
  method: string;
  params?: Record<string, unknown>;
}

interface McpResponse {
  jsonrpc: '2.0';
  id: string | number;
  result?: unknown;
  error?: {
    code: number;
    message: string;
    data?: unknown;
  };
}

interface McpNotification {
  jsonrpc: '2.0';
  method: string;
  params?: Record<string, unknown>;
}

// MCP Server implementation
export class McpServer {
  private wasm: WasmModule | null = null;
  private ws: WebSocket | null = null;
  private serverInfo = {
    name: 'dirplayer-vm',
    version: '1.0.0',
  };

  constructor() {}

  setWasm(wasm: WasmModule) {
    this.wasm = wasm;
  }

  async start(): Promise<void> {
    // In the browser/Electron renderer context, we can't create an HTTP server directly.
    // Instead, we communicate with the main process which hosts the HTTP server.

    // Check if we're in Electron
    if (typeof window !== 'undefined' && (window as any).require) {
      const { ipcRenderer } = (window as any).require('electron');

      // Request the main process to start the HTTP server
      ipcRenderer.send('mcp:start-server', { port: MCP_PORT });

      // Listen for incoming MCP requests from the main process
      ipcRenderer.on('mcp:request', (_event: any, data: { requestId: string; request: McpRequest }) => {
        this.handleRequest(data.request).then((response) => {
          ipcRenderer.send('mcp:response', { requestId: data.requestId, response });
        });
      });

      console.log(`MCP server bridge initialized, requesting main process to listen on http://localhost:${MCP_PORT}`);
    } else {
      console.warn('MCP server can only run in Electron environment');
    }
  }

  private async handleRequest(request: McpRequest): Promise<McpResponse> {
    try {
      switch (request.method) {
        case 'initialize':
          return this.handleInitialize(request);
        case 'tools/list':
          return this.handleListTools(request);
        case 'tools/call':
          return this.handleToolCall(request);
        default:
          return {
            jsonrpc: '2.0',
            id: request.id,
            error: {
              code: -32601,
              message: `Method not found: ${request.method}`,
            },
          };
      }
    } catch (error) {
      return {
        jsonrpc: '2.0',
        id: request.id,
        error: {
          code: -32603,
          message: error instanceof Error ? error.message : 'Internal error',
        },
      };
    }
  }

  private handleInitialize(request: McpRequest): McpResponse {
    return {
      jsonrpc: '2.0',
      id: request.id,
      result: {
        protocolVersion: '2024-11-05',
        serverInfo: this.serverInfo,
        capabilities: {
          tools: {},
        },
      },
    };
  }

  private handleListTools(request: McpRequest): McpResponse {
    return {
      jsonrpc: '2.0',
      id: request.id,
      result: {
        tools: mcpTools,
      },
    };
  }

  private handleToolCall(request: McpRequest): McpResponse {
    const params = request.params as { name: string; arguments?: Record<string, unknown> };
    const toolName = params.name as McpToolName;
    const args = params.arguments || {};

    if (!this.wasm) {
      return {
        jsonrpc: '2.0',
        id: request.id,
        error: {
          code: -32603,
          message: 'WASM module not initialized',
        },
      };
    }

    try {
      const result = this.callTool(toolName, args);
      return {
        jsonrpc: '2.0',
        id: request.id,
        result: {
          content: [
            {
              type: 'text',
              text: result,
            },
          ],
        },
      };
    } catch (error) {
      return {
        jsonrpc: '2.0',
        id: request.id,
        error: {
          code: -32603,
          message: error instanceof Error ? error.message : 'Tool execution failed',
        },
      };
    }
  }

  private callTool(name: McpToolName, args: Record<string, unknown>): string {
    if (!this.wasm) {
      throw new Error('WASM module not initialized');
    }

    switch (name) {
      // Script tools
      case 'list_scripts':
        return this.wasm.mcp_list_scripts();

      case 'get_script':
        return this.wasm.mcp_get_script(
          args.cast_lib as number,
          args.cast_member as number
        );

      case 'disassemble_handler':
        return this.wasm.mcp_disassemble_handler(
          args.cast_lib as number,
          args.cast_member as number,
          args.handler_name as string
        );

      case 'decompile_handler':
        return this.wasm.mcp_decompile_handler(
          args.cast_lib as number,
          args.cast_member as number,
          args.handler_name as string
        );

      // Execution tools
      case 'get_console_output':
        return this.wasm.mcp_get_console_output(
          args.last_n_lines as number
        );

      case 'get_call_stack':
        return this.wasm.mcp_get_call_stack();

      case 'get_execution_state':
        return this.wasm.mcp_get_execution_state();

      case 'eval_lingo':
        // eval_command is async and uses callbacks, so we call it but can't get sync result
        this.wasm.eval_command(args.code as string);
        return JSON.stringify({ status: 'command sent', note: 'Results will appear in the debug console' });

      case 'pause':
        this.wasm.stop();
        return JSON.stringify({ status: 'paused' });

      case 'resume':
        this.wasm.resume_breakpoint();
        return JSON.stringify({ status: 'resumed' });

      case 'step_over':
        this.wasm.step_over();
        return JSON.stringify({ status: 'stepped over' });

      case 'step_into':
        this.wasm.step_into();
        return JSON.stringify({ status: 'stepped into' });

      case 'step_out':
        this.wasm.step_out();
        return JSON.stringify({ status: 'stepped out' });

      // Variable tools
      case 'get_globals':
        return this.wasm.mcp_get_globals();

      case 'get_locals':
        return this.wasm.mcp_get_locals(
          args.scope_index !== undefined ? (args.scope_index as number) : -1
        );

      case 'inspect_datum':
        return this.wasm.mcp_inspect_datum(args.datum_id as number);

      // Cast tools
      case 'list_cast_libs':
        return this.wasm.mcp_list_cast_libs();

      case 'list_cast_members':
        return this.wasm.mcp_list_cast_members(
          args.cast_lib !== undefined ? (args.cast_lib as number) : -1
        );

      case 'inspect_cast_member':
        return this.wasm.mcp_inspect_cast_member(
          args.cast_lib as number,
          args.cast_member as number
        );

      // Breakpoint tools
      case 'set_breakpoint':
        this.wasm.add_breakpoint(
          args.script_name as string,
          args.handler_name as string,
          args.bytecode_index as number
        );
        return JSON.stringify({ status: 'breakpoint set' });

      case 'remove_breakpoint':
        this.wasm.remove_breakpoint(
          args.script_name as string,
          args.handler_name as string,
          args.bytecode_index as number
        );
        return JSON.stringify({ status: 'breakpoint removed' });

      case 'list_breakpoints':
        return this.wasm.mcp_list_breakpoints();

      default:
        throw new Error(`Unknown tool: ${name}`);
    }
  }

  stop() {
    if (typeof window !== 'undefined' && (window as any).require) {
      const { ipcRenderer } = (window as any).require('electron');
      ipcRenderer.send('mcp:stop-server');
    }
  }
}

// Singleton instance
let mcpServerInstance: McpServer | null = null;

export function getMcpServer(): McpServer {
  if (!mcpServerInstance) {
    mcpServerInstance = new McpServer();
  }
  return mcpServerInstance;
}

export function initMcpServer(wasm: WasmModule): McpServer {
  const server = getMcpServer();
  server.setWasm(wasm);
  return server;
}
