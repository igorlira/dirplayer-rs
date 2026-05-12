#!/usr/bin/env python3
"""
Regenerate vm-rust/src/player/js_lingo/opcodes.rs from the SpiderMonkey 1.5
opcode table shipped with Director MX 2004 (jsdmx/src/jsopcode.tbl).

Usage:  python gen_js_opcodes.py [<path to jsopcode.tbl>]

Default path: E:/Documents/js/javascript15/jsdmx/src/jsopcode.tbl
Output:       ../src/player/js_lingo/opcodes.rs (relative to this script)
"""

import os
import re
import sys

DEFAULT_TBL = "E:/Documents/js/javascript15/jsdmx/src/jsopcode.tbl"
OUT_REL = "../src/player/js_lingo/opcodes.rs"


def pascal(s: str) -> str:
    return "".join(p.capitalize() for p in s.split("_"))


def fmt_to_variant(fmt: str) -> str:
    parts = [p.strip() for p in fmt.split("|")]
    if any(p in ("JOF_JUMP", "JOF_JUMPX") for p in parts):
        return "Jump"
    if any(p in ("JOF_TABLESWITCH", "JOF_TABLESWITCHX") for p in parts):
        return "Tableswitch"
    if any(p in ("JOF_LOOKUPSWITCH", "JOF_LOOKUPSWITCHX") for p in parts):
        return "Lookupswitch"
    if "JOF_QARG" in parts:
        return "Qarg"
    if "JOF_QVAR" in parts:
        return "Qvar"
    if "JOF_LOCAL" in parts:
        return "Local"
    if "JOF_OBJECT" in parts:
        return "Object"
    if "JOF_CONST" in parts:
        return "Const"
    if "JOF_UINT16" in parts:
        return "Uint16"
    return "Byte"


def parse_table(path):
    ops = []
    line_re = re.compile(
        r'OPDEF\(JSOP_(\w+),\s*(\d+),\s*(?:"([^"]*)"|(\w+)),\s*[^,]+,'
        r'\s*(-?\d+),\s*(-?\d+),\s*(-?\d+),\s*(-?\d+),\s*([^)]+)\)'
    )
    with open(path) as f:
        for line in f:
            if not line.strip().startswith("OPDEF"):
                continue
            m = line_re.match(line)
            if not m:
                print(f"WARNING: unparseable line: {line.rstrip()}", file=sys.stderr)
                continue
            name, num, m_str, m_sym, length, uses, defs, _prec, fmt = m.groups()
            ops.append({
                "num": int(num),
                "name": name,
                "mnem": m_str if m_str else m_sym,
                "length": int(length),
                "uses": int(uses),
                "defs": int(defs),
                "fmt": fmt.strip(),
            })
    ops.sort(key=lambda o: o["num"])
    return ops


def render(ops):
    out = []
    out.append("// Auto-generated from jsdmx/src/jsopcode.tbl")
    out.append("// (SpiderMonkey 1.5 modified by Macromedia for Director MX 2004).")
    out.append("//")
    out.append("// Source: E:/Documents/js/javascript15/jsdmx/src/jsopcode.tbl")
    out.append("// Do not edit by hand. Regenerate via tools/gen_js_opcodes.py.")
    out.append("")
    out.append("use num_derive::FromPrimitive;")
    out.append("")
    out.append("#[derive(Debug, Clone, Copy, PartialEq, Eq, FromPrimitive)]")
    out.append("#[repr(u8)]")
    out.append("pub enum JsOp {")
    for op in ops:
        out.append(f"    {pascal(op['name'])} = {op['num']},")
    out.append("}")
    out.append("")
    out.append("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    out.append("pub enum JsOpFormat {")
    out.append("    Byte, Uint16, Const, Jump, Local, Qarg, Qvar, Tableswitch, Lookupswitch, Object,")
    out.append("}")
    out.append("")
    out.append("#[derive(Debug, Clone, Copy)]")
    out.append("pub struct JsOpInfo {")
    out.append("    pub op: JsOp,")
    out.append("    pub mnemonic: &'static str,")
    out.append("    /// Fixed instruction length, or -1 if variable (table/lookup switch).")
    out.append("    pub length: i8,")
    out.append("    /// Stack operands consumed; -1 if variable (CALL/NEW).")
    out.append("    pub uses: i8,")
    out.append("    /// Stack operands defined.")
    out.append("    pub defs: i8,")
    out.append("    pub format: JsOpFormat,")
    out.append("}")
    out.append("")
    out.append("pub const JS_OP_INFO: &[JsOpInfo] = &[")
    for op in ops:
        pname = pascal(op["name"])
        fvar = fmt_to_variant(op["fmt"])
        out.append(
            f'    JsOpInfo {{ op: JsOp::{pname}, mnemonic: "{op["mnem"]}", '
            f"length: {op['length']}, uses: {op['uses']}, defs: {op['defs']}, "
            f"format: JsOpFormat::{fvar} }},"
        )
    out.append("];")
    out.append("")
    out.append("impl JsOp {")
    out.append("    pub fn info(self) -> &'static JsOpInfo { &JS_OP_INFO[self as usize] }")
    out.append("    pub fn from_byte(b: u8) -> Option<Self> { num::FromPrimitive::from_u8(b) }")
    out.append("}")
    out.append("")
    return "\n".join(out)


def main():
    tbl = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_TBL
    if not os.path.isfile(tbl):
        print(f"ERROR: opcode table not found: {tbl}", file=sys.stderr)
        sys.exit(1)
    ops = parse_table(tbl)
    here = os.path.dirname(os.path.abspath(__file__))
    out_path = os.path.normpath(os.path.join(here, OUT_REL))
    rendered = render(ops)
    with open(out_path, "w") as f:
        f.write(rendered)
    print(f"Wrote {out_path} ({len(ops)} opcodes)")


if __name__ == "__main__":
    main()
