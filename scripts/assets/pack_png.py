"""Write straight-alpha RGBA8 PNG strips for material packs."""

import struct
import zlib

def chunk(kind, data):
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data))
    )


def png(rgba, width, height):
    stride = width * 4
    rows = b"".join(b"\0" + rgba[start:start + stride] for start in range(0, len(rgba), stride))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"sRGB", b"\0")
        + chunk(b"IDAT", zlib.compress(rows, level=6))
        + chunk(b"IEND", b"")
    )
