from __future__ import annotations

import importlib.util
import mmap
import struct
import tempfile
import unittest
from pathlib import Path

PLUGIN_DIR = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("shared_audio", PLUGIN_DIR / "shared_audio.py")
assert SPEC is not None and SPEC.loader is not None
SHARED_AUDIO = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SHARED_AUDIO)


def create_ring(path: Path, capacity: int = 8) -> None:
    with path.open("w+b") as file_object:
        file_object.truncate(64 + capacity * 4)
        with mmap.mmap(file_object.fileno(), 0, access=mmap.ACCESS_WRITE) as mapping:
            SHARED_AUDIO.HEADER.pack_into(
                mapping,
                0,
                SHARED_AUDIO.MAGIC,
                SHARED_AUDIO.VERSION,
                48_000,
                2,
                0,
                capacity,
                0,
                0,
                bytes(16),
            )


class SharedAudioRingTests(unittest.TestCase):
    def test_push_pop_and_wrap_around(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "audio.pcmmap"
            create_ring(path)
            with SHARED_AUDIO.SharedAudioRing(path) as ring:
                self.assertEqual(ring.push([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), 6)
                self.assertEqual(ring.pop(4), [1.0, 2.0, 3.0, 4.0])
                self.assertEqual(ring.push([7.0, 8.0, 9.0, 10.0, 11.0, 12.0]), 6)
                self.assertEqual(
                    ring.pop(8), [5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0]
                )
                self.assertEqual(ring.available_read, 0)

    def test_full_ring_refuses_additional_samples(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "audio.pcmmap"
            create_ring(path, capacity=4)
            with SHARED_AUDIO.SharedAudioRing(path) as ring:
                self.assertEqual(ring.push([1.0, 2.0, 3.0, 4.0, 5.0]), 4)
                self.assertEqual(ring.push([6.0]), 0)


if __name__ == "__main__":
    unittest.main()
