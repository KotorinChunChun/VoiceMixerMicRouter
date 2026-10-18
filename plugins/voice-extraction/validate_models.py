"""明示実行の実モデル試験。公開fixtureと合成雑音を使いマイクは開かない。"""
import argparse
import json
from pathlib import Path
import time
import sys

from engines import ExtractionEngine


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--method", required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--seconds", type=float, default=4.0)
    args = parser.parse_args()
    if not 0.25 <= args.seconds <= 60:
        parser.error("--secondsは0.25〜60です")
    import numpy as np
    import soundfile as sf
    import torch
    import torchaudio
    args.output.mkdir(parents=True, exist_ok=True)
    voice, rate = sf.read(args.fixture, always_2d=True, dtype="float32")
    if voice.shape[1] == 1:
        voice = np.repeat(voice, 2, axis=1)
    voice = torchaudio.functional.resample(torch.from_numpy(voice[:, :2].T.copy()), rate, 48000).T.numpy()[:round(args.seconds*48000)]
    voice = voice / max(float(np.max(np.abs(voice))), 1e-6) * .35
    rng = np.random.default_rng(46)
    axis = np.arange(len(voice)) / 48000
    background = sum(.035 * np.sin(2 * np.pi * hz * axis) for hz in [130.81, 164.81, 196.0, 261.63])
    conditions = {"room_noise": rng.normal(0, .035, voice.shape).astype("float32"), "synthetic_music": np.repeat(background[:, None], 2, axis=1).astype("float32")}
    loaded = time.perf_counter()
    engine = ExtractionEngine({"method":args.method,"model_root":str(args.root.resolve()),"device":args.device,"chunk_seconds":1.0})
    report = {"method":args.method,"device":args.device,"load_seconds":time.perf_counter()-loaded,"declared_latency_frames":engine.latency_frames,"declared_latency_seconds":engine.latency_frames/48000,"torch":torch.__version__,"conditions":[]}
    try:
        for name, noise in conditions.items():
            mixed = voice + noise
            padding = engine.context_frames
            padded = np.pad(mixed, ((padding, padding+engine.chunk_frames), (0,0)))
            output, durations = [], []
            for offset in range(0, len(voice), engine.chunk_frames):
                window = padded[offset:offset+engine.chunk_frames+2*padding].reshape(-1).tolist()
                started = time.perf_counter()
                result = np.array(engine.process_window(window), dtype="float32").reshape(-1,2)
                durations.append(time.perf_counter()-started)
                output.append(result[padding:padding+engine.chunk_frames])
            result = np.concatenate(output)[:len(voice)]
            if not np.isfinite(result).all() or len(result) != len(mixed):
                raise AssertionError("出力長または有限値検証に失敗")
            sf.write(args.output / f"{args.method}-{name}-input.wav", mixed, 48000, subtype="FLOAT")
            sf.write(args.output / f"{args.method}-{name}-output.wav", result, 48000, subtype="FLOAT")
            report["conditions"].append({"name":name,"frames":len(result),"mean_chunk_seconds":float(np.mean(durations)),"max_chunk_seconds":max(durations),"real_time_factor":sum(durations)/(len(result)/48000),"input_rms":float(np.sqrt(np.mean(mixed**2))),"output_rms":float(np.sqrt(np.mean(result**2))),"difference_rms":float(np.sqrt(np.mean((result-mixed)**2))),"reference_error_rms":float(np.sqrt(np.mean((result-voice)**2))),"input_reference_error_rms":float(np.sqrt(np.mean((mixed-voice)**2)))})
        if args.device == "cuda":
            report["peak_gpu_bytes"] = torch.cuda.max_memory_allocated()
        if sys.platform == "win32":
            import ctypes
            from ctypes import wintypes
            class MemoryCounters(ctypes.Structure):
                _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [(name, ctypes.c_size_t) for name in ("PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage")]
            counters = MemoryCounters()
            counters.cb = ctypes.sizeof(counters)
            if ctypes.windll.psapi.GetProcessMemoryInfo(ctypes.c_void_p(-1), ctypes.byref(counters), counters.cb):
                report["peak_process_working_set_bytes"] = counters.PeakWorkingSetSize
        (args.output / f"{args.method}-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(report, ensure_ascii=False))
    finally:
        engine.close()


if __name__ == "__main__":
    main()
