from __future__ import annotations

import json
import io
import mmap
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

PLUGIN_DIR = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PLUGIN_DIR))

import shared_audio
import rvc_plugin


def create_ring(path: Path, capacity: int = 4096) -> None:
    with path.open("w+b") as file_object:
        file_object.truncate(shared_audio.HEADER_SIZE + capacity * 4)
        with mmap.mmap(file_object.fileno(), 0, access=mmap.ACCESS_WRITE) as mapping:
            shared_audio.HEADER.pack_into(
                mapping,
                0,
                shared_audio.MAGIC,
                shared_audio.VERSION,
                48_000,
                2,
                0,
                capacity,
                0,
                0,
                bytes(16),
            )


class ProcessProtocolTests(unittest.TestCase):
    def test_ports_rejects_unknown_or_malformed_input(self) -> None:
        worker = rvc_plugin.PluginWorker(io.StringIO(), "passthrough")
        with self.assertRaisesRegex(ValueError, "unsupported connected port"):
            worker.handle({"type": "ports", "payload": {"connected": ["future_input"]}})
        with self.assertRaisesRegex(ValueError, "array of strings"):
            worker.handle({"type": "ports", "payload": {"connected": "audio_in"}})

    def test_passthrough_process_protocol_and_audio(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_path = root / "input.pcmmap"
            output_path = root / "output.pcmmap"
            create_ring(input_path)
            create_ring(output_path)
            process = subprocess.Popen(
                [sys.executable, "rvc_plugin.py", "--engine", "passthrough"],
                cwd=PLUGIN_DIR,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                encoding="utf-8",
            )
            assert process.stdin is not None
            assert process.stdout is not None
            try:
                prepare = {
                    "type": "prepare",
                    "payload": {
                        "sample_rate": 48_000,
                        "channels": 2,
                        "graph_block_frames": 480,
                        "transport": {
                            "input_mapping_name": str(input_path),
                            "output_mapping_name": str(output_path),
                            "capacity_frames": 2048,
                            "sample_rate": 48_000,
                            "channels": 2,
                        },
                    },
                }
                process.stdin.write(json.dumps(prepare) + "\n")
                process.stdin.flush()
                hello = json.loads(process.stdout.readline())
                ready = json.loads(process.stdout.readline())
                self.assertEqual(hello["type"], "hello")
                self.assertEqual(ready["type"], "ready")

                # HostToPluginMessage::Ports is sent when the graph plan is
                # compiled. RVC has one graph input (audio_in); audio_out is
                # represented by the fixed v1 output shared-memory ring.
                process.stdin.write('{"type":"ports","payload":{"connected":["audio_in"]}}\n')
                process.stdin.flush()

                process.stdin.write('{"type":"start"}\n')
                process.stdin.flush()
                state = json.loads(process.stdout.readline())
                self.assertEqual(state["payload"]["state"], "running")

                expected = [index / 960.0 for index in range(960)]
                with shared_audio.SharedAudioRing(input_path) as input_ring:
                    self.assertEqual(input_ring.push(expected), len(expected))
                actual: list[float] = []
                deadline = time.monotonic() + 2.0
                with shared_audio.SharedAudioRing(output_path) as output_ring:
                    while len(actual) < len(expected) and time.monotonic() < deadline:
                        actual.extend(output_ring.pop(len(expected) - len(actual)))
                        if len(actual) < len(expected):
                            time.sleep(0.001)
                self.assertEqual(len(actual), len(expected))
                self.assertLess(
                    max(abs(left - right) for left, right in zip(actual, expected)), 1e-6
                )

                process.stdin.write('{"type":"shutdown"}\n')
                process.stdin.flush()
                process.stdin.close()
                self.assertEqual(process.wait(timeout=5.0), 0)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5.0)
                if process.returncode:
                    assert process.stderr is not None
                    self.fail(process.stderr.read())
                for stream in (process.stdin, process.stdout, process.stderr):
                    if stream is not None and not stream.closed:
                        stream.close()


if __name__ == "__main__":
    unittest.main()
