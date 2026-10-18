"""RVC推論アダプター。既定CUDA、独立ツールからはCPUを明示指定可能。"""

from __future__ import annotations

import math
import os
import sys
from pathlib import Path
from typing import Any
from types import SimpleNamespace
from queue import Queue as ThreadQueue
from threading import Thread, Lock


DEFAULT_PARAMETERS: dict[str, Any] = {
    "model": "models/KahoA.pth",
    "index": "models/Kaho.index",
    "pitch": 0.0,
    "formant": 0.0,
    "f0_method": "fcpe",
    "index_rate": 0.0,
    "block_time": 0.25,
    "crossfade_time": 0.05,
    "extra_time": 2.5,
    "rms_mix_rate": 0.0,
    "threshold_db": -60.0,
    "n_cpu": 4,
    "use_jit": False,
    "input_noise_reduction": False,
    "output_noise_reduction": False,
    "transient_noise_suppression": False,
}


class RvcEngine:
    def __init__(self, plugin_directory: Path, *, device: str | None = None) -> None:
        self.plugin_directory = plugin_directory
        self.parameters = dict(DEFAULT_PARAMETERS)
        self.latency_frames = 0
        self.pending_input: list[float] = []
        self.rvc: Any = None
        self.torch: Any = None
        self.requested_device = device
        self.harvest_threads = []

    def _resolve(self, value: str) -> Path:
        path = Path(value)
        return path if path.is_absolute() else self.plugin_directory / path

    def prepare(self, parameters: dict[str, Any], sample_rate: int) -> None:
        if sample_rate != 48_000:
            raise ValueError(f"RVC CUDA requires 48000 Hz host audio, got {sample_rate}")
        self.parameters.update(parameters)
        os.environ.setdefault("OMP_NUM_THREADS", str(min(int(self.parameters["n_cpu"]), 4)))

        import numpy as np
        import librosa
        import torch
        import torch.nn.functional as torch_functional
        import torchaudio.transforms as audio_transforms

        from spectral_gate import SpectralGate
        from transient_gate import transient_gain

        self.transient_gain = transient_gain

        if self.requested_device != "cpu" and not torch.cuda.is_available():
            raise RuntimeError("NVIDIA CUDA is required by the RVC CUDA plugin")

        model_path = self._resolve(str(self.parameters["model"]))
        index_value = str(self.parameters["index"])
        index_path = self._resolve(index_value) if index_value else Path()
        hubert_path = self.plugin_directory / "assets" / "hubert" / "hubert_base.pt"
        for label, path in (
            ("RVC model", model_path),
            ("HuBERT model", hubert_path),
        ):
            if not path.is_file():
                raise FileNotFoundError(f"{label} does not exist: {path}")
        if float(self.parameters["index_rate"]) != 0.0 and not index_path.is_file():
            raise FileNotFoundError(
                f"RVC index is required when index_rate is non-zero: {index_value}"
            )

        from configs.config import Config
        from infer.lib.rtrvc import RVC

        if self.requested_device is not None:
            # オフライン利用ではConfigの共有ファイル書換えを避ける。
            config = SimpleNamespace(device=self.requested_device, is_half=False,
                                     use_jit=False, dml=False)
        else:
            original_arguments = sys.argv
            try:
                sys.argv = [original_arguments[0]]
                config = Config()
            finally:
                sys.argv = original_arguments
        if self.requested_device is None and not str(config.device).startswith("cuda"):
            raise RuntimeError(f"RVC selected a non-CUDA device: {config.device}")
        config.use_jit = bool(self.parameters["use_jit"])

        self.harvest_input = ThreadQueue()
        self.harvest_output = ThreadQueue()
        self.harvest_lock = Lock()
        self.harvest_error = None
        for _ in range(min(max(1, int(self.parameters["n_cpu"])), 4)):
            thread = Thread(target=self._harvest_worker, daemon=True)
            thread.start()
            self.harvest_threads.append(thread)
        self.rvc = RVC(
            float(self.parameters["pitch"]),
            float(self.parameters["formant"]),
            str(model_path),
            str(index_path) if index_value else "",
            float(self.parameters["index_rate"]),
            min(max(1, int(self.parameters["n_cpu"])), 4),
            self.harvest_input,
            self.harvest_output,
            config,
            None,
        )
        if not hasattr(self.rvc, "tgt_sr") or not hasattr(self.rvc, "net_g"):
            raise RuntimeError("RVC model initialization did not produce a synthesizer")

        self.np = np
        self.librosa = librosa
        self.torch = torch
        self.functional = torch_functional
        self.device = config.device
        self.host_sample_rate = sample_rate
        self.zc = sample_rate // 100
        self.block_frame = self._aligned_frames(float(self.parameters["block_time"]))
        self.block_frame_16k = 160 * self.block_frame // self.zc
        self.crossfade_frame = self._aligned_frames(
            float(self.parameters["crossfade_time"])
        )
        self.sola_buffer_frame = min(self.crossfade_frame, 4 * self.zc)
        self.sola_search_frame = self.zc
        self.extra_frame = self._aligned_frames(float(self.parameters["extra_time"]))
        input_length = (
            self.extra_frame
            + self.crossfade_frame
            + self.sola_search_frame
            + self.block_frame
        )
        self.input_wav = torch.zeros(input_length, device=self.device, dtype=torch.float32)
        # 入力ゲートで失われる微小な後続音も、短音と語頭の区別に使う。
        # 同じサンプル位置の参照履歴であり、音声を待たせるキューではない。
        self.transient_reference = np.zeros(input_length, dtype=np.float32)
        self.input_wav_denoise = self.input_wav.clone()
        self.input_wav_res = torch.zeros(
            160 * input_length // self.zc, device=self.device, dtype=torch.float32
        )
        self.sola_buffer = torch.zeros(
            self.sola_buffer_frame, device=self.device, dtype=torch.float32
        )
        self.noise_reduction_buffer = self.sola_buffer.clone()
        self.output_buffer = self.input_wav.clone()
        self.rms_buffer = np.zeros(4 * self.zc, dtype=np.float32)
        self.skip_head = self.extra_frame // self.zc
        self.return_length = (
            self.block_frame + self.sola_buffer_frame + self.sola_search_frame
        ) // self.zc
        self.fade_in_window = (
            torch.sin(
                0.5
                * np.pi
                * torch.linspace(
                    0.0,
                    1.0,
                    steps=self.sola_buffer_frame,
                    device=self.device,
                    dtype=torch.float32,
                )
            )
            ** 2
        )
        self.fade_out_window = 1 - self.fade_in_window
        self.resampler = audio_transforms.Resample(
            orig_freq=sample_rate, new_freq=16_000, dtype=torch.float32
        ).to(self.device)
        self.output_resampler = (
            audio_transforms.Resample(
                orig_freq=int(self.rvc.tgt_sr),
                new_freq=sample_rate,
                dtype=torch.float32,
            ).to(self.device)
            if int(self.rvc.tgt_sr) != sample_rate
            else None
        )
        self.spectral_gate = SpectralGate(
            sample_rate=sample_rate,
            n_fft=4 * self.zc,
            proportion_decrease=0.9,
        ).to(self.device)
        self.latency_frames = self.block_frame + self.crossfade_frame + self.sola_search_frame
        self.pending_input.clear()
        if str(self.device).startswith("cuda"):
            torch.cuda.empty_cache()

    def warmup(self) -> None:
        """初回推論まで準備中に済ませ、練習音と履歴を実音声へ渡さない。"""
        if self.rvc is None:
            raise RuntimeError("RVC must be prepared before warmup")
        self._reset_stream_history()
        try:
            # 無音でも通常の変換経路を通し、選択中のF0モデルやCUDAを初期化する。
            # この戻り値は共有リングへ送らず破棄する。
            output = self.process([0.0] * self.block_frame, 1)
            if len(output) != self.block_frame or not self.np.isfinite(output).all():
                raise RuntimeError("RVC warmup did not produce one finite audio block")
            if str(self.device).startswith("cuda"):
                self.torch.cuda.synchronize(self.device)
        finally:
            self._reset_stream_history()

    def _reset_stream_history(self) -> None:
        self.pending_input.clear()
        for history in (
            self.input_wav,
            self.input_wav_denoise,
            self.input_wav_res,
            self.sola_buffer,
            self.noise_reduction_buffer,
            self.output_buffer,
            self.rvc.cache_pitch,
            self.rvc.cache_pitchf,
        ):
            history.zero_()
        self.rms_buffer.fill(0)
        self.transient_reference.fill(0)
        self.harvest_error = None
        # HuBERT・合成器・F0モデル・リサンプル核は音声履歴を持たないため保持する。

    def _harvest_worker(self) -> None:
        import numpy as np
        import pyworld
        while True:
            item = self.harvest_input.get()
            if item is None:
                return
            idx, audio, results, count, stamp = item
            try:
                f0, _ = pyworld.harvest(audio.astype(np.double), fs=16000,
                                       f0_ceil=1100, f0_floor=50, frame_period=10)
                with self.harvest_lock:
                    results[idx] = f0
                    if len(results) == count:
                        self.harvest_output.put(stamp)
            except Exception as exc:
                self.harvest_error = exc
                # 推論側を待機させ続けない。欠落結果は呼出元でエラーにする。
                self.harvest_output.put(stamp)

    def _aligned_frames(self, seconds: float) -> int:
        return max(self.zc, int(round(seconds * self.host_sample_rate / self.zc)) * self.zc)

    def set_parameters(self, update: dict[str, Any]) -> None:
        self.parameters.update(update)
        if self.rvc is None:
            return
        if "pitch" in update:
            self.rvc.change_key(float(update["pitch"]))
        if "formant" in update:
            self.rvc.change_formant(float(update["formant"]))
        if "index_rate" in update:
            index_path = self._resolve(str(self.parameters["index"]))
            if float(update["index_rate"]) != 0.0 and not index_path.is_file():
                raise FileNotFoundError(
                    f"RVC index is required when index_rate is non-zero: {index_path}"
                )
            self.rvc.change_index_rate(float(update["index_rate"]))

    def process(self, samples: list[float], channels: int) -> list[float]:
        if channels <= 0 or len(samples) % channels:
            raise ValueError("input sample count is not aligned to channels")
        array = self.np.asarray(samples, dtype=self.np.float32).reshape((-1, channels))
        mono = array.mean(axis=1)
        self.pending_input.extend(mono.tolist())
        output: list[float] = []
        while len(self.pending_input) >= self.block_frame:
            source = self.pending_input[: self.block_frame]
            del self.pending_input[: self.block_frame]
            converted = self._convert_block(self.np.asarray(source, dtype=self.np.float32))
            stereo = self.np.repeat(converted[:, None], channels, axis=1).reshape(-1)
            output.extend(stereo.astype(self.np.float32, copy=False).tolist())
        return output

    def _convert_block(self, mono: Any) -> Any:
        torch = self.torch
        reference = mono
        if float(self.parameters["threshold_db"]) > -60.0:
            reference = self.np.concatenate((self.rms_buffer, mono))[
                2 * self.zc:2 * self.zc + len(mono)
            ]
        self.transient_reference[:-self.block_frame] = self.transient_reference[self.block_frame:]
        self.transient_reference[-self.block_frame:] = reference
        mono = self._apply_input_gate(mono)
        self.input_wav[: -self.block_frame] = self.input_wav[self.block_frame :].clone()
        self.input_wav[-self.block_frame :] = torch.from_numpy(mono).to(self.device)
        self.input_wav_res[: -self.block_frame_16k] = self.input_wav_res[
            self.block_frame_16k :
        ].clone()
        if bool(self.parameters["input_noise_reduction"]):
            self.input_wav_denoise[: -self.block_frame] = self.input_wav_denoise[
                self.block_frame :
            ].clone()
            denoised = self.spectral_gate(
                self.input_wav[-self.sola_buffer_frame - self.block_frame :].unsqueeze(0),
                self.input_wav.unsqueeze(0),
            ).squeeze(0)
            denoised[: self.sola_buffer_frame] *= self.fade_in_window
            denoised[: self.sola_buffer_frame] += (
                self.noise_reduction_buffer * self.fade_out_window
            )
            self.input_wav_denoise[-self.block_frame :] = denoised[: self.block_frame]
            self.noise_reduction_buffer[:] = denoised[
                self.block_frame : self.block_frame + self.sola_buffer_frame
            ]
            resample_source = self.input_wav_denoise[
                -self.block_frame - 2 * self.zc :
            ]
        else:
            resample_source = self.input_wav[-self.block_frame - 2 * self.zc :]
        resampled = self.resampler(resample_source)[160:]
        self.input_wav_res[-self.block_frame_16k :] = resampled[-self.block_frame_16k :]

        inferred = self.rvc.infer(
            self.input_wav_res,
            self.block_frame_16k,
            self.skip_head,
            self.return_length,
            str(self.parameters["f0_method"]),
        )
        if self.harvest_error is not None:
            raise RuntimeError("Harvest Worker failed") from self.harvest_error
        if self.output_resampler is not None:
            inferred = self.output_resampler(inferred)
        inferred = inferred.float().flatten()
        required = self.sola_buffer_frame + self.sola_search_frame + self.block_frame
        if inferred.numel() < required:
            inferred = self.functional.pad(inferred, (0, required - inferred.numel()))

        if bool(self.parameters["output_noise_reduction"]):
            self.output_buffer[: -self.block_frame] = self.output_buffer[
                self.block_frame :
            ].clone()
            self.output_buffer[-self.block_frame :] = inferred[-self.block_frame :]
            inferred = self.spectral_gate(
                inferred.unsqueeze(0), self.output_buffer.unsqueeze(0)
            ).squeeze(0)

        rms_mix_rate = float(self.parameters["rms_mix_rate"])
        if rms_mix_rate < 1.0:
            rms_source = (
                self.input_wav_denoise
                if bool(self.parameters["input_noise_reduction"])
                else self.input_wav
            )[self.extra_frame :]
            input_rms = self._rms_envelope(rms_source[: inferred.numel()], inferred.numel())
            output_rms = self._rms_envelope(inferred, inferred.numel()).clamp_min(1e-3)
            inferred *= torch.pow(input_rms / output_rms, 1.0 - rms_mix_rate)

        search = inferred[None, None, : self.sola_buffer_frame + self.sola_search_frame]
        numerator = self.functional.conv1d(search, self.sola_buffer[None, None, :])
        denominator = torch.sqrt(
            self.functional.conv1d(
                search**2,
                torch.ones(1, 1, self.sola_buffer_frame, device=self.device),
            )
            + 1e-8
        )
        sola_offset = int(torch.argmax(numerator[0, 0] / denominator[0, 0]).item())
        inferred = inferred[sola_offset:]
        if inferred.numel() < self.block_frame + self.sola_buffer_frame:
            inferred = self.functional.pad(
                inferred, (0, self.block_frame + self.sola_buffer_frame - inferred.numel())
            )
        inferred[: self.sola_buffer_frame] *= self.fade_in_window
        inferred[: self.sola_buffer_frame] += self.sola_buffer * self.fade_out_window
        self.sola_buffer[:] = inferred[
            self.block_frame : self.block_frame + self.sola_buffer_frame
        ]
        result = inferred[: self.block_frame].detach().float().cpu().numpy()
        if not self.np.isfinite(result).all():
            raise RuntimeError("RVC produced non-finite audio")
        if bool(self.parameters["transient_noise_suppression"]):
            result = self._suppress_transients(result, sola_offset)
        return result

    def _suppress_transients(self, result: Any, sola_offset: int) -> Any:
        # 推論が参照した履歴と同じ位置に合わせる。壁時計の遅延や共有リングの
        # 滞留量には依存しない。SOLA用の履歴は原音のまま保持して位置合わせを守る。
        reference = (
            self.input_wav_denoise
            if bool(self.parameters["input_noise_reduction"])
            else self.input_wav
        ).detach().float().cpu().numpy()
        gain = self.transient_gain(
            reference,
            self.host_sample_rate,
            self.extra_frame + sola_offset,
            self.block_frame,
            protection_history=self.transient_reference,
        )
        return result * gain

    def _apply_input_gate(self, mono: Any) -> Any:
        threshold_db = float(self.parameters["threshold_db"])
        if threshold_db <= -60.0:
            return mono
        buffered = self.np.append(self.rms_buffer, mono)
        rms = self.librosa.feature.rms(
            y=buffered,
            frame_length=4 * self.zc,
            hop_length=self.zc,
        )[:, 2:]
        self.rms_buffer[:] = buffered[-4 * self.zc :]
        gated = buffered[2 * self.zc - self.zc // 2 :].copy()
        below_threshold = (
            self.librosa.amplitude_to_db(rms, ref=1.0)[0] < threshold_db
        )
        for index, is_below in enumerate(below_threshold):
            if is_below:
                gated[index * self.zc : (index + 1) * self.zc] = 0
        return gated[self.zc // 2 : self.zc // 2 + mono.shape[0]]

    def _rms_envelope(self, waveform: Any, output_length: int) -> Any:
        rms = self.librosa.feature.rms(
            y=waveform.detach().float().cpu().numpy(),
            frame_length=4 * self.zc,
            hop_length=self.zc,
        )
        rms_tensor = self.torch.from_numpy(rms).to(self.device)
        return self.functional.interpolate(
            rms_tensor.unsqueeze(0),
            size=output_length + 1,
            mode="linear",
            align_corners=True,
        )[0, 0, :-1]

    def close(self) -> None:
        for _ in self.harvest_threads:
            self.harvest_input.put(None)
        for thread in self.harvest_threads:
            thread.join(timeout=5)
        self.harvest_threads.clear()
        self.pending_input.clear()
        self.rvc = None
        if (self.torch is not None and self.requested_device != "cpu"
                and self.torch.cuda.is_available()):
            self.torch.cuda.empty_cache()
