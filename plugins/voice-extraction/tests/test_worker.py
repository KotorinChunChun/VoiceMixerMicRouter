import io
import json
from pathlib import Path
import struct
import sys
import tempfile
import time
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from engines import parameters
from model_manager import extract_checked, install, installed, validate_file, digest
from shared_audio import SharedAudioRing
from voice_extraction import Worker


def ring_file(path, capacity=128):
    path.write_bytes(struct.pack("<8sIIHH4xQQQ16s", b"ARPCM001", 1, 48000, 2, 0, capacity, 0, 0, bytes(16)) + bytes(capacity * 4))


class FakeEngine:
    context_frames = 2
    chunk_frames = 4
    latency_frames = 6
    def __init__(self, settings):
        self.settings = settings
    def process_window(self, samples):
        return [v * 0.5 for v in samples]
    def close(self):
        pass


class Tests(unittest.TestCase):
    def test_partial_parameters_validate_before_updating(self):
        values = parameters({"method": "htdemucs", "chunk_seconds": 2})
        self.assertEqual(parameters({"device": "cuda"}, values)["chunk_seconds"], 2)
        for invalid in ({"chunk_seconds": float("nan")}, {"method": "unknown"}, {"chunk_seconds": True}, {"x": 1}):
            with self.assertRaises(ValueError):
                parameters(invalid, values)
        self.assertEqual(values["method"], "htdemucs")

    def test_ring_wrap_is_stereo_aligned_and_bounded(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "ring"
            ring_file(path, 8)
            ring = SharedAudioRing(path)
            try:
                self.assertEqual(ring.push([1., 2., 3., 4., 5.]), 4)
                self.assertEqual(ring.pop(3), [1., 2.])
                self.assertEqual(ring.push([6., 7., 8., 9., 10., 11., 12., 13.]), 6)
                self.assertEqual(ring.pop(12), [3., 4., 6., 7., 8., 9., 10., 11.])
            finally:
                ring.close()

    def test_worker_protocol_and_audio_with_nonstandard_blocks(self):
        with tempfile.TemporaryDirectory() as temp:
            paths = [Path(temp) / name for name in ("in", "out")]
            for path in paths:
                ring_file(path)
            host_in, host_out = map(SharedAudioRing, paths)
            messages = io.StringIO()
            worker = Worker(messages, FakeEngine)
            try:
                worker.handle({"type":"set_parameters", "payload":{"method":"htdemucs"}})
                worker.handle({"type":"prepare", "payload":{"sample_rate":48000, "channels":2, "graph_block_frames":128, "transport":{"sample_rate":48000,"channels":2,"input_mapping_name":str(paths[0]),"output_mapping_name":str(paths[1])}}})
                worker.handle({"type":"start"})
                host_in.push([0.2, 0.4] * 3)
                host_in.push([0.2, 0.4] * 3)
                deadline = time.monotonic() + 2
                while host_out.available_read < 8 and time.monotonic() < deadline:
                    time.sleep(.01)
                for actual, expected in zip(host_out.pop(8), [0.1, 0.2] * 4):
                    self.assertAlmostEqual(actual, expected, places=6)
                self.assertGreater(worker.processed_frames, 0)
                worker.handle({"type":"stop"})
                self.assertEqual(worker.pending, [])
                worker.handle({"type":"start"})
                worker.handle({"type":"ping", "payload":{"sequence":12}})
                self.assertIn('"sequence": 12', messages.getvalue())
                self.assertLessEqual(len(worker.buffer), 16)
                self.assertFalse(worker.handle({"type":"shutdown"}))
            finally:
                worker.close()
                host_in.close()
                host_out.close()
            data = [json.loads(line) for line in messages.getvalue().splitlines()]
            self.assertEqual(next(m for m in data if m["type"] == "ready")["payload"]["latency_frames"], 6)

    def test_cancel_and_download_failure_do_not_publish_model(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            cancel = root / "cancel"
            cancel.touch()
            with self.assertRaises(InterruptedError):
                install(root, "deepfilternet3", cancel)
            def broken(*args):
                raise OSError("ネットワーク停止")
            with self.assertRaises(OSError):
                install(root, "deepfilternet3", fetch=broken)
            self.assertIsNone(installed(root, "deepfilternet3"))
            self.assertEqual(list(root.glob(".install-*")), [])
            (root / "deepfilternet3").mkdir()
            with self.assertRaises(ValueError):
                install(root, "deepfilternet3", fetch=broken)

    def test_hash_mismatch_and_zip_traversal_are_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = root / "data"
            path.write_bytes(b"correct")
            entry = {"bytes":7, "name":"data", "sha256":digest(path)}
            validate_file(path, entry)
            path.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                validate_file(path, entry)
            archive = root / "bad.zip"
            with zipfile.ZipFile(archive, "w") as zipped:
                zipped.writestr("../outside", "bad")
            with self.assertRaises(ValueError):
                extract_checked(archive, root, 100)


if __name__ == "__main__":
    unittest.main()
