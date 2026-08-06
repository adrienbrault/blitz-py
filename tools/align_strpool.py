"""Align the LC_SYMTAB string pool to 8 bytes for macOS 27 beta dyld.

Inserts padding before the string table, bumps stroff and the __LINKEDIT
segment sizes. Signature must be removed first and re-added afterwards.
"""

import struct
import sys

path = sys.argv[1]
data = bytearray(open(path, "rb").read())

magic, cputype, cpusubtype, filetype, ncmds, sizeofcmds, flags, reserved = (
    struct.unpack_from("<IiiIIIII", data, 0)
)
assert magic == 0xFEEDFACF, hex(magic)

LC_SYMTAB = 0x2
LC_SEGMENT_64 = 0x19

off = 32
symtab_off = None
linkedit_off = None
stroff = strsize = None

for _ in range(ncmds):
    cmd, cmdsize = struct.unpack_from("<II", data, off)
    if cmd == LC_SYMTAB:
        symtab_off = off
        _, _, symoff, nsyms, stroff, strsize = struct.unpack_from("<IIIIII", data, off)
    elif cmd == LC_SEGMENT_64:
        name = data[off + 8 : off + 24].rstrip(b"\0").decode()
        if name == "__LINKEDIT":
            linkedit_off = off
    off += cmdsize

assert symtab_off is not None and linkedit_off is not None

pad = (8 - stroff % 8) % 8
if pad == 0:
    print("already aligned")
    sys.exit(0)

# Bump stroff in LC_SYMTAB
struct.pack_into("<I", data, symtab_off + 16, stroff + pad)

# Bump __LINKEDIT filesize; keep vmsize page-rounded
vmsize, filesize = struct.unpack_from("<QQ", data, linkedit_off + 32 + 8)
filesize += pad
PAGE = 0x4000
vmsize = max(vmsize, (filesize + PAGE - 1) // PAGE * PAGE)
struct.pack_into("<QQ", data, linkedit_off + 32 + 8, vmsize, filesize)

# Insert padding bytes before the string pool
data[stroff:stroff] = b"\0" * pad

open(path, "wb").write(bytes(data))
print(f"padded {pad} bytes at {stroff:#x}; new stroff {stroff + pad:#x}")
