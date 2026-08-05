"""
Synthetic FL Studio project (``.flp``) factory.

WHY THIS EXISTS
    Same reason as ``factories/als.py``: a ``.flp`` is a DAW project file and must
    never enter this repository. The one real ``.flp`` used during development is
    FL 10 era and cannot be redistributed.

THE FORMAT (as implemented by experiments/flp_parse.py)
    b'FLhd' + u32 length(=6) + { i16 format, u16 channels, u16 ppq }
    b'FLdt' + u32 length      + event stream

    Event = u8 id, then a payload whose size is implied by the id:

        id   0- 63 : 1 byte      ("byte events")
        id  64-127 : 2 bytes     ("word events")
        id 128-191 : 4 bytes     ("dword events")
        id 192-255 : varint length, then that many bytes ("variable events")

    Text events (ids 192-207) are NUL-terminated: latin-1 in older files,
    UTF-16LE in newer ones.

WHAT IT GIVES TESTS
    - exact control of every byte, so varint boundaries (127/128/16383/16384) can
      be hit deliberately;
    - both text encodings, so the decoder heuristic can be pinned;
    - deliberately corrupt output (``truncate``, ``with_magic``, ``with_data_length``,
      ``raw_event`` with a lying size) for the robustness tests.
"""

from __future__ import annotations

import struct
from typing import List, Optional, Tuple

__all__ = [
    "FlpBuilder",
    "encode_varint",
    "decode_varint",
    "latin1_text",
    "utf16_text",
    "BYTE_EVENT",
    "WORD_EVENT",
    "DWORD_EVENT",
    "VAR_EVENT",
    "truncate",
]

# One representative id from each width class, chosen to be a real id from
# experiments/flp_parse.py's EVENT_NAMES table where possible.
BYTE_EVENT = 0  # ChanEnabled       -> 1 byte
WORD_EVENT = 64  # NewChan          -> 2 bytes
DWORD_EVENT = 128  # (unnamed)      -> 4 bytes
VAR_EVENT = 209  # PluginParams     -> varint length


def encode_varint(n: int) -> bytes:
    """FL's LEB128-style unsigned varint, low 7 bits first, high bit = continue."""
    if n < 0:
        raise ValueError("varint must be non-negative")
    out = bytearray()
    while True:
        byte = n & 0x7F
        n >>= 7
        if n:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def decode_varint(data: bytes, pos: int = 0) -> Tuple[int, int]:
    """Inverse of :func:`encode_varint`. Returns (value, bytes_consumed)."""
    value = 0
    shift = 0
    consumed = 0
    while True:
        byte = data[pos + consumed]
        consumed += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            return value, consumed


def latin1_text(s: str) -> bytes:
    """Old-style FL text payload: latin-1, NUL terminated."""
    return s.encode("latin-1") + b"\x00"


def utf16_text(s: str) -> bytes:
    """Modern FL text payload: UTF-16LE, NUL terminated."""
    return s.encode("utf-16-le") + b"\x00\x00"


def truncate(data: bytes, n: int) -> bytes:
    """First ``n`` bytes — a file that stopped mid-write."""
    return data[:n]


class FlpBuilder:
    """Builds a byte-exact ``.flp``. Every method returns self, so calls chain."""

    def __init__(self, fmt: int = 0, channels: int = 1, ppq: int = 96) -> None:
        self.fmt = fmt
        self.channels = channels
        self.ppq = ppq
        self._events: List[bytes] = []
        # (event id, payload) for every well-formed event, so a test can assert
        # what the parser found against what was actually written.
        self.log: List[Tuple[int, bytes]] = []
        self._magic_head = b"FLhd"
        self._magic_data = b"FLdt"
        self._header_length: Optional[int] = None
        self._data_length: Optional[int] = None

    # -- well-formed events ------------------------------------------------- #

    def byte(self, ev: int = BYTE_EVENT, value: int = 0) -> "FlpBuilder":
        if not 0 <= ev < 64:
            raise ValueError("byte events use ids 0-63, got %d" % ev)
        payload = struct.pack("<B", value & 0xFF)
        self._events.append(bytes([ev]) + payload)
        self.log.append((ev, payload))
        return self

    def word(self, ev: int = WORD_EVENT, value: int = 0) -> "FlpBuilder":
        if not 64 <= ev < 128:
            raise ValueError("word events use ids 64-127, got %d" % ev)
        payload = struct.pack("<H", value & 0xFFFF)
        self._events.append(bytes([ev]) + payload)
        self.log.append((ev, payload))
        return self

    def dword(self, ev: int = DWORD_EVENT, value: int = 0) -> "FlpBuilder":
        if not 128 <= ev < 192:
            raise ValueError("dword events use ids 128-191, got %d" % ev)
        payload = struct.pack("<I", value & 0xFFFFFFFF)
        self._events.append(bytes([ev]) + payload)
        self.log.append((ev, payload))
        return self

    def var(self, ev: int = VAR_EVENT, payload: bytes = b"") -> "FlpBuilder":
        if not 192 <= ev < 256:
            raise ValueError("variable events use ids 192-255, got %d" % ev)
        self._events.append(bytes([ev]) + encode_varint(len(payload)) + payload)
        self.log.append((ev, payload))
        return self

    def text(self, ev: int, s: str, encoding: str = "utf-16") -> "FlpBuilder":
        if encoding in ("utf-16", "utf-16-le"):
            payload = utf16_text(s)
        elif encoding in ("latin-1", "latin1", "ascii"):
            payload = latin1_text(s)
        else:
            raise ValueError("unsupported text encoding %r" % encoding)
        return self.var(ev, payload)

    def blob(self, ev: int, size: int, fill: int = 0xAB) -> "FlpBuilder":
        """A variable event of exactly ``size`` payload bytes — opaque plugin state."""
        return self.var(ev, bytes([fill]) * size)

    # -- deliberately malformed -------------------------------------------- #

    def raw_event(self, ev: int, payload: bytes, declared_size: int) -> "FlpBuilder":
        """A variable event whose declared size does not match its payload."""
        if not 192 <= ev < 256:
            raise ValueError("raw_event is for variable-width ids only")
        self._events.append(bytes([ev]) + encode_varint(declared_size) + payload)
        return self

    def raw_bytes(self, data: bytes) -> "FlpBuilder":
        """Append arbitrary bytes into the event stream."""
        self._events.append(data)
        return self

    def with_magic(self, head: bytes = b"FLhd", data: bytes = b"FLdt") -> "FlpBuilder":
        self._magic_head = head
        self._magic_data = data
        return self

    def with_header_length(self, n: int) -> "FlpBuilder":
        self._header_length = n
        return self

    def with_data_length(self, n: int) -> "FlpBuilder":
        """Override the declared FLdt length (a lying length field)."""
        self._data_length = n
        return self

    # -- output ------------------------------------------------------------- #

    @property
    def event_bytes(self) -> bytes:
        return b"".join(self._events)

    @property
    def event_count(self) -> int:
        return len(self._events)

    def to_bytes(self) -> bytes:
        header_body = struct.pack("<hHH", self.fmt, self.channels, self.ppq)
        hlen = len(header_body) if self._header_length is None else self._header_length
        events = self.event_bytes
        dlen = len(events) if self._data_length is None else self._data_length
        return (
            self._magic_head
            + struct.pack("<I", hlen)
            + header_body
            + self._magic_data
            + struct.pack("<I", dlen)
            + events
        )

    def write(self, path) -> str:
        path = str(path)
        with open(path, "wb") as fh:
            fh.write(self.to_bytes())
        return path


def realistic_project(path=None) -> FlpBuilder:
    """
    A small but structurally complete project: every width class, both text
    encodings, an opaque plugin blob, and a sample reference with an FL path token.
    """
    b = (
        FlpBuilder(fmt=0, channels=2, ppq=96)
        .text(199, "10.0.0", encoding="latin-1")  # Version
        .byte(0, 1)  # ChanEnabled
        .byte(9, 1)  # LoopActive
        .word(64, 0)  # NewChan
        .word(66, 1)  # PatNew
        .dword(128, 0xDEADBEEF)
        .dword(159, 44100)
        .text(194, "Wit synthetic project", encoding="utf-16")  # Title
        .text(192, "Kick", encoding="latin-1")  # ChanName
        .text(196, r"%FLStudioData%\Patches\kick.wav", encoding="latin-1")
        .blob(209, 128)  # PluginParams — opaque
        .blob(213, 4096)  # unnamed blob — opaque
    )
    if path is not None:
        b.write(path)
    return b
