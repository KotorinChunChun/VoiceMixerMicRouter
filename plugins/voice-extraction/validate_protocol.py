"""実モデルProcess/Ring試験。専用モデル導入後に明示実行する。"""
import argparse
import json
import math
from pathlib import Path
import queue
import struct
import subprocess
import sys
import tempfile
import threading
import time
from shared_audio import SharedAudioRing


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--method", required=True)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as temp:
        paths = [Path(temp) / name for name in ("input.pcmmap", "output.pcmmap")]
        for path in paths:
            path.write_bytes(struct.pack("<8sIIHH4xQQQ16s", b"ARPCM001", 1, 48000, 2, 0, 192000, 0, 0, bytes(16)) + bytes(192000 * 4))
        host_in, host_out = map(SharedAudioRing, paths)
        process = subprocess.Popen([sys.executable, str(Path(__file__).with_name("voice_extraction.py"))], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, encoding="utf-8")
        events = queue.Queue()
        all_events = []
        def read():
            for line in process.stdout:
                try:
                    events.put(json.loads(line))
                except Exception as error:
                    events.put({"type":"bad_stdout", "payload":str(error)})
        threading.Thread(target=read, daemon=True).start()
        def send(kind, payload=None):
            process.stdin.write(json.dumps({"type":kind,"payload":payload}) + "\n")
            process.stdin.flush()
        def wait_for(kind, timeout=30):
            until = time.monotonic() + timeout
            while time.monotonic() < until:
                event = events.get(timeout=max(.01, until-time.monotonic()))
                all_events.append(event)
                if event["type"] in ("error", "bad_stdout"):
                    raise AssertionError(event)
                if event["type"] == kind:
                    return event
            raise TimeoutError(kind)
        try:
            send("set_parameters", {"method":args.method,"model_root":str(args.root.resolve()),"chunk_seconds":1.0,"device":"cpu"})
            send("prepare", {"sample_rate":48000,"channels":2,"graph_block_frames":128,"transport":{"sample_rate":48000,"channels":2,"input_mapping_name":str(paths[0]),"output_mapping_name":str(paths[1])}})
            ready = wait_for("ready", 120)
            send("start")
            wait_for("state")
            source = [value for index in range(72000) for value in (0.1*math.sin(index*.04), 0.12*math.sin(index*.05))]
            assert host_in.push(source) == len(source)
            started = time.monotonic()
            time.sleep(.1)
            ping_started = time.monotonic()
            send("ping", {"sequence":46})
            wait_for("pong", 5)
            pong_seconds = time.monotonic() - ping_started
            until = time.monotonic() + 60
            output = []
            while len(output) < 96000 and time.monotonic() < until:
                output.extend(host_out.pop(96000-len(output)))
                time.sleep(.005)
            if len(output) != 96000 or not all(math.isfinite(v) for v in output):
                raise AssertionError("出力音声の長さ/有限値が不正")
            first_chunk_seconds = time.monotonic()-started
            if not any(event["type"] == "stats" for event in all_events):
                wait_for("stats")
            send("stop")
            # 能力不足のstate通知が先に届く場合があるので停止状態まで読む。
            while wait_for("state")["payload"]["state"] != "stopped":
                pass
            send("start")
            wait_for("state")
            send("shutdown")
            process.wait(timeout=10)
            report = {"method":args.method,"state":"passed","frames":len(output)//2,"ready":ready["payload"],"first_chunk_wall_seconds":first_chunk_seconds,"pong_seconds":pong_seconds,"events":all_events,"exit_code":process.returncode}
            args.output.mkdir(parents=True, exist_ok=True)
            (args.output / f"{args.method}-protocol.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
            print(json.dumps(report, ensure_ascii=False))
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            host_in.close()
            host_out.close()


if __name__ == "__main__":
    main()
