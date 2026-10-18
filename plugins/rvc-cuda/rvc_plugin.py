"""VoiceMixer Process Plugin Protocol v1 worker for RVC."""

from __future__ import annotations

import argparse
import json
import sys
import threading
import time
import traceback
from pathlib import Path
from typing import Any, Optional, TextIO

PROTOCOL_STDOUT = sys.stdout
sys.stdout = sys.stderr
PLUGIN_DIRECTORY = Path(__file__).resolve().parent
if str(PLUGIN_DIRECTORY) not in sys.path:
    sys.path.insert(0, str(PLUGIN_DIRECTORY))

from shared_audio import SharedAudioRing  # noqa: E402

PLUGIN_ID = "voicemixer.rvc.cuda"
PLUGIN_VERSION = "0.1.0"


class PassthroughEngine:
    """Protocol/Ring test engine used without importing RVC dependencies."""

    latency_frames = 0

    def prepare(self, _parameters: dict[str, Any], _sample_rate: int) -> None:
        return

    def warmup(self) -> None:
        return

    def set_parameters(self, _parameters: dict[str, Any]) -> None:
        return

    def process(self, samples: list[float], _channels: int) -> list[float]:
        return samples

    def close(self) -> None:
        return


class PluginWorker:
    def __init__(
        self,
        output: TextIO,
        engine_name: str,
        required_initial_model: Optional[str] = None,
    ) -> None:
        self.output = output
        self.engine_name = engine_name
        self.required_initial_model = required_initial_model
        self.parameters: dict[str, Any] = {}
        self.connected_ports: set[str] = set()
        self.input_ring: SharedAudioRing | None = None
        self.output_ring: SharedAudioRing | None = None
        self.sample_rate = 48_000
        self.channels = 2
        self.graph_block_frames = 480
        self.running = threading.Event()
        self.shutdown_requested = threading.Event()
        self.protocol_lock = threading.Lock()
        self.audio_thread: threading.Thread | None = None
        self.engine: Any = None
        self.last_inference_ms = 0.0
        self.prepare_ms = 0.0
        self.warmup_ms = 0.0
        self.stats_thread: threading.Thread | None = None

    def send(self, message_type: str, payload: dict[str, Any] | None = None) -> None:
        message: dict[str, Any] = {"type": message_type}
        if payload is not None:
            message["payload"] = payload
        with self.protocol_lock:
            self.output.write(json.dumps(message, ensure_ascii=False) + "\n")
            self.output.flush()

    def prepare(self, payload: dict[str, Any]) -> None:
        started = time.perf_counter()
        if self.audio_thread is not None:
            raise RuntimeError("prepare may only be sent once")
        if (
            self.required_initial_model is not None
            and self.parameters.get("model") != self.required_initial_model
        ):
            raise RuntimeError(
                "initial model parameter was not received before prepare: "
                f"expected={self.required_initial_model!r}, "
                f"actual={self.parameters.get('model')!r}"
            )
        transport = payload["transport"]
        self.sample_rate = int(payload["sample_rate"])
        self.channels = int(payload["channels"])
        self.graph_block_frames = int(payload["graph_block_frames"])
        if self.sample_rate != int(transport["sample_rate"]):
            raise ValueError("prepare sample rate does not match transport")
        if self.channels != int(transport["channels"]):
            raise ValueError("prepare channels do not match transport")
        self.input_ring = SharedAudioRing(transport["input_mapping_name"])
        self.output_ring = SharedAudioRing(transport["output_mapping_name"])
        if self.engine_name == "passthrough":
            self.engine = PassthroughEngine()
        else:
            from rvc_engine import RvcEngine

            self.engine = RvcEngine(Path(__file__).resolve().parent)
        self.engine.prepare(self.parameters, self.sample_rate)
        warmup_started = time.perf_counter()
        self.engine.warmup()
        self.warmup_ms = (time.perf_counter() - warmup_started) * 1000.0
        self.prepare_ms = (time.perf_counter() - started) * 1000.0
        latency_frames = int(self.engine.latency_frames)
        if latency_frames:
            silence = [0.0] * (latency_frames * self.channels)
            if self.output_ring.push(silence) != len(silence):
                raise RuntimeError("output ring is too small for plugin latency")
        self.audio_thread = threading.Thread(
            target=self.audio_loop, name="rvc-cuda-audio", daemon=True
        )
        self.audio_thread.start()
        self.stats_thread = threading.Thread(
            target=self.stats_loop, name="rvc-cuda-stats", daemon=True
        )
        self.stats_thread.start()
        self.send(
            "hello",
            {
                "protocol_version": 1,
                "plugin_id": PLUGIN_ID,
                "plugin_version": PLUGIN_VERSION,
            },
        )
        self.send(
            "ready",
            {
                "latency_frames": latency_frames,
                "preferred_block_frames": self.graph_block_frames,
            },
        )

    def audio_loop(self) -> None:
        try:
            maximum = max(2, self.graph_block_frames * self.channels)
            pending: list[float] = []
            while not self.shutdown_requested.is_set():
                if not self.running.is_set():
                    time.sleep(0.001)
                    continue
                if pending:
                    assert self.output_ring is not None
                    written = self.output_ring.push(pending)
                    if written:
                        del pending[:written]
                    else:
                        time.sleep(0.0002)
                    continue
                assert self.input_ring is not None
                source = self.input_ring.pop(maximum)
                if not source:
                    time.sleep(0.0002)
                    continue
                started = time.perf_counter()
                pending = self.engine.process(source, self.channels)
                self.last_inference_ms = (time.perf_counter() - started) * 1000.0
        except Exception as exc:  # keep the host alive and report a local failure
            traceback.print_exc(file=sys.stderr)
            self.running.clear()
            self.send("error", {"code": "audio_processing_failed", "message": str(exc)})

    def set_parameters(self, update: dict[str, Any]) -> None:
        self.parameters.update(update)
        if self.engine is not None:
            self.engine.set_parameters(update)

    def set_connected_ports(self, payload: dict[str, Any]) -> None:
        """Accept the v1 host notification for this plugin's sole audio input.

        The v1 shared-memory transport always has one input and one output ring,
        so `audio_out` is not part of the graph-side input connection list.  Do
        not silently accept arbitrary future port names: that would conceal an
        incompatible host/plugin graph contract.
        """
        connected = payload.get("connected")
        if not isinstance(connected, list) or any(
            not isinstance(port, str) for port in connected
        ):
            raise ValueError("ports.connected must be an array of strings")
        if len(set(connected)) != len(connected):
            raise ValueError("ports.connected must not contain duplicates")
        unknown = set(connected) - {"audio_in"}
        if unknown:
            raise ValueError(f"unsupported connected port: {sorted(unknown)[0]}")
        self.connected_ports = set(connected)

    def close(self) -> None:
        self.running.clear()
        self.shutdown_requested.set()
        if self.audio_thread is not None:
            self.audio_thread.join(timeout=5.0)
            self.audio_thread = None
        if self.stats_thread is not None:
            self.stats_thread.join(timeout=2.0)
            self.stats_thread = None
        if self.engine is not None:
            self.engine.close()
            self.engine = None
        if self.input_ring is not None:
            self.input_ring.close()
            self.input_ring = None
        if self.output_ring is not None:
            self.output_ring.close()
            self.output_ring = None

    def stats_loop(self) -> None:
        while not self.shutdown_requested.wait(1.0):
            if not self.running.is_set():
                continue
            payload: dict[str, Any] = {
                "inference_ms": float(self.last_inference_ms),
                "prepare_ms": self.prepare_ms,
                "warmup_ms": self.warmup_ms,
            }
            if self.input_ring is not None and self.channels:
                payload["queue_frames"] = int(self.input_ring.available_read // self.channels)
            payload.update(self._gpu_stats())
            try:
                self.send("stats", payload)
            except Exception:
                return

    def _gpu_stats(self) -> dict[str, Any]:
        torch = getattr(self.engine, "torch", None)
        if torch is None:
            return {}
        try:
            if not torch.cuda.is_available():
                return {}
            stats: dict[str, Any] = {
                "gpu_memory_mb": int(torch.cuda.memory_allocated() / (1024 * 1024))
            }
            utilization = getattr(torch.cuda, "utilization", None)
            if callable(utilization):
                stats["gpu_utilization"] = float(utilization()) / 100.0
            return stats
        except Exception:
            return {}

    def handle(self, message: dict[str, Any]) -> bool:
        message_type = message.get("type")
        payload = message.get("payload") or {}
        if message_type == "prepare":
            self.prepare(payload)
        elif message_type == "start":
            self.running.set()
            self.send("state", {"state": "running", "message": None})
        elif message_type == "stop":
            self.running.clear()
            self.send("state", {"state": "stopped", "message": None})
        elif message_type == "set_parameters":
            self.set_parameters(payload)
        elif message_type == "ports":
            self.set_connected_ports(payload)
        elif message_type == "ping":
            self.send("pong", {"sequence": int(payload["sequence"])})
        elif message_type == "shutdown":
            return False
        else:
            raise ValueError(f"unsupported control message: {message_type}")
        return True


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", choices=("rvc", "passthrough"), default="rvc")
    parser.add_argument("--require-initial-model", help=argparse.SUPPRESS)
    return parser.parse_args()


def main() -> int:
    arguments = parse_arguments()
    worker = PluginWorker(
        PROTOCOL_STDOUT, arguments.engine, arguments.require_initial_model
    )
    try:
        for line in sys.stdin:
            if not line.strip():
                continue
            try:
                if not worker.handle(json.loads(line)):
                    break
            except Exception as exc:
                traceback.print_exc(file=sys.stderr)
                worker.send("error", {"code": "plugin_failure", "message": str(exc)})
                return 1
        return 0
    finally:
        worker.close()


if __name__ == "__main__":
    raise SystemExit(main())
