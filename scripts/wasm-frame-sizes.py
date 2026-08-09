"""Report the shadow-stack frame each named function reserves, straight from the
wasm binary. No wabt/binaryen needed.

LLVM's wasm prologue for a function with a real frame is:
    global.get 0        23 00
    i32.const N         41 <sleb128>
    i32.sub             6b
    local.tee/set       22|21 <uleb>
    global.set 0        24 00
N is the frame size in bytes. A function with no frame simply has no such prologue.
"""
import sys

def u32(b, i):
    r = 0; s = 0
    while True:
        x = b[i]; i += 1
        r |= (x & 0x7F) << s
        if not (x & 0x80): return r, i
        s += 7

def s32(b, i):
    r = 0; s = 0
    while True:
        x = b[i]; i += 1
        r |= (x & 0x7F) << s
        s += 7
        if not (x & 0x80):
            if s < 32 and (x & 0x40): r |= -(1 << s)
            return r, i

path = sys.argv[1]
targets = sys.argv[2:]
b = open(path, 'rb').read()
assert b[:4] == b'\0asm', 'not a wasm module'
i = 8

sections = []
while i < len(b):
    sid = b[i]; i += 1
    size, i = u32(b, i)
    sections.append((sid, i, size))
    i += size

names = {}          # func index -> name
n_imported_funcs = 0
code = None

for sid, off, size in sections:
    if sid == 2:  # imports
        j = off
        cnt, j = u32(b, j)
        for _ in range(cnt):
            ml, j = u32(b, j); j += ml
            nl, j = u32(b, j); j += nl
            kind = b[j]; j += 1
            if kind == 0:
                _, j = u32(b, j); n_imported_funcs += 1
            elif kind == 1:
                j += 1; lim = b[j]; j += 1
                _, j = u32(b, j)
                if lim: _, j = u32(b, j)
            elif kind == 2:
                lim = b[j]; j += 1
                _, j = u32(b, j)
                if lim: _, j = u32(b, j)
            elif kind == 3:
                j += 2
    elif sid == 10:  # code
        code = (off, size)
    elif sid == 0:  # custom
        j = off
        nl, j = u32(b, j)
        nm = b[j:j+nl]; j += nl
        if nm == b'name':
            end = off + size
            while j < end:
                sub = b[j]; j += 1
                sz, j = u32(b, j)
                nxt = j + sz
                if sub == 1:
                    cnt, j = u32(b, j)
                    for _ in range(cnt):
                        idx, j = u32(b, j)
                        l, j = u32(b, j)
                        names[idx] = b[j:j+l].decode('utf-8', 'replace'); j += l
                j = nxt

if code is None:
    print('no code section'); sys.exit(1)

off, size = code
j = off
cnt, j = u32(b, j)
rows = []
for k in range(cnt):
    body_size, j = u32(b, j)
    body_start = j
    body_end = j + body_size
    fidx = n_imported_funcs + k
    nm = names.get(fidx, '')
    if targets and not any(t in nm for t in targets):
        j = body_end; continue

    p = body_start
    ldecl, p = u32(b, p)
    n_locals = 0
    for _ in range(ldecl):
        c, p = u32(b, p)
        p += 1
        n_locals += c

    frame = 0
    if b[p] == 0x23:                       # global.get
        q = p + 1
        g, q = u32(b, q)
        if g == 0 and b[q] == 0x41:        # i32.const
            v, q = s32(b, q + 1)
            if b[q] == 0x6b:               # i32.sub
                frame = v
    rows.append((nm, frame, n_locals, body_size))
    j = body_end

rows.sort(key=lambda r: -r[1])
print(f"{'frame':>8}  {'locals':>6}  {'body':>8}  name")
for nm, frame, nl, bs in rows:
    print(f"{frame:>8}  {nl:>6}  {bs:>8}  {nm}")
