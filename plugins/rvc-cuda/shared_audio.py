"""Shared Audio Ring v1 access for the VoiceMixer process plugin."""

from __future__ import annotations

import mmap
import struct
from pathlib import Path
from typing import Iterable

MAGIC = b"ARPCM001"
VERSION = 1
HEADER_SIZE = 64
HEADER = struct.Struct("<8sIIHH4xQQQ16s")
READ_INDEX_OFFSET = 32
WRITE_INDEX_OFFSET = 40


class SharedAudioRing:
    """Single-producer/single-consumer Float32 PCM ring."""

    def __init__(self, path: str | Path) -> None:
        self.path = Path(path)
        self._file = self.path.open("r+b", buffering=0)
        self._map = mmap.mmap(self._file.fileno(), 0, access=mmap.ACCESS_WRITE)
        (
            magic,
            version,
            self.sample_rate,
            self.channels,
            _reserved0,
            self.capacity_samples,
            _read,
            _write,
            _reserved,
        ) = HEADER.unpack_from(self._map, 0)
        if magic != MAGIC or version != VERSION:
            self.close()
            raise ValueError(f"invalid shared audio ring header: {self.path}")
        expected = HEADER_SIZE + self.capacity_samples * 4
        if len(self._map) < expected:
            self.close()
            raise ValueError(f"truncated shared audio ring: {self.path}")

    @property
    def read_index(self) -> int:
        return struct.unpack_from("<Q", self._map, READ_INDEX_OFFSET)[0]

    @property
    def write_index(self) -> int:
        return struct.unpack_from("<Q", self._map, WRITE_INDEX_OFFSET)[0]

    @property
    def available_read(self) -> int:
        return min(max(0, self.write_index - self.read_index), self.capacity_samples)

    @property
    def available_write(self) -> int:
        return self.capacity_samples - self.available_read

    def pop(self, maximum_samples: int) -> list[float]:
        count = min(maximum_samples, self.available_read)
        if count <= 0:
            return []
        read = self.read_index
        first = min(count, self.capacity_samples - (read % self.capacity_samples))
        start = HEADER_SIZE + (read % self.capacity_samples) * 4
        values = list(struct.unpack_from(f"<{first}f", self._map, start))
        remaining = count - first
        if remaining:
            values.extend(struct.unpack_from(f"<{remaining}f", self._map, HEADER_SIZE))
        struct.pack_into("<Q", self._map, READ_INDEX_OFFSET, read + count)
        return values

    def push(self, samples: Iterable[float]) -> int:
        values = list(samples)
        count = min(len(values), self.available_write)
        if count <= 0:
            return 0
        write = self.write_index
        first = min(count, self.capacity_samples - (write % self.capacity_samples))
        start = HEADER_SIZE + (write % self.capacity_samples) * 4
        struct.pack_into(f"<{first}f", self._map, start, *values[:first])
        remaining = count - first
        if remaining:
            struct.pack_into(
                f"<{remaining}f", self._map, HEADER_SIZE, *values[first:count]
            )
        struct.pack_into("<Q", self._map, WRITE_INDEX_OFFSET, write + count)
        return count

    def close(self) -> None:
        mapped = getattr(self, "_map", None)
        if mapped is not None:
            mapped.close()
            self._map = None
        file_object = getattr(self, "_file", None)
        if file_object is not None:
            file_object.close()
            self._file = None

    def __enter__(self) -> "SharedAudioRing":
        return self

    def __exit__(self, *_args: object) -> None:
        self.close()
