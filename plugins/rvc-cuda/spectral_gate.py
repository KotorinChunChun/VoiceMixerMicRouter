"""CUDA spectral gate adapted from the reference RVC realtime GUI."""

from __future__ import annotations

import torch
from torch.nn.functional import conv2d


def _linspace(
    start: float, stop: float, count: int, *, endpoint: bool = True
) -> torch.Tensor:
    if endpoint:
        return torch.linspace(start, stop, count)
    return torch.linspace(start, stop, count + 1)[:-1]


def _amplitude_to_db(value: torch.Tensor, top_db: float = 40.0) -> torch.Tensor:
    epsilon = torch.finfo(torch.float64).eps
    decibels = 20 * torch.log10(value.abs() + epsilon)
    return torch.maximum(
        decibels, (decibels.max(-1).values - top_db).unsqueeze(-1)
    )


class SpectralGate(torch.nn.Module):
    """Stationary spectral gate used by the source RVC realtime client."""

    def __init__(
        self,
        sample_rate: int,
        n_fft: int,
        proportion_decrease: float = 0.9,
        standard_deviations: float = 1.5,
        frequency_smoothing_hz: float = 500.0,
        time_smoothing_ms: float = 50.0,
    ) -> None:
        super().__init__()
        if not 0.0 <= proportion_decrease <= 1.0:
            raise ValueError("proportion_decrease must be between 0 and 1")
        self.sample_rate = sample_rate
        self.n_fft = n_fft
        self.window_length = n_fft
        self.hop_length = n_fft // 4
        self.proportion_decrease = proportion_decrease
        self.standard_deviations = standard_deviations

        frequency_bins = max(
            1,
            int(frequency_smoothing_hz / (sample_rate / (n_fft / 2))),
        )
        time_bins = max(
            1,
            int(time_smoothing_ms / ((self.hop_length / sample_rate) * 1000)),
        )
        frequency_window = torch.cat(
            [
                _linspace(0, 1, frequency_bins + 1, endpoint=False),
                _linspace(1, 0, frequency_bins + 2),
            ]
        )[1:-1]
        time_window = torch.cat(
            [
                _linspace(0, 1, time_bins + 1, endpoint=False),
                _linspace(1, 0, time_bins + 2),
            ]
        )[1:-1]
        smoothing_filter = torch.outer(frequency_window, time_window)[None, None]
        self.register_buffer("smoothing_filter", smoothing_filter / smoothing_filter.sum())

    @torch.no_grad()
    def forward(self, signal: torch.Tensor, noise: torch.Tensor) -> torch.Tensor:
        window = torch.hann_window(
            self.window_length, device=signal.device, dtype=signal.dtype
        )
        spectrum = torch.stft(
            signal,
            n_fft=self.n_fft,
            hop_length=self.hop_length,
            win_length=self.window_length,
            return_complex=True,
            pad_mode="constant",
            center=True,
            window=window,
        )
        noise_spectrum = torch.stft(
            noise,
            n_fft=self.n_fft,
            hop_length=self.hop_length,
            win_length=self.window_length,
            return_complex=True,
            pad_mode="constant",
            center=True,
            window=window,
        )
        signal_db = _amplitude_to_db(spectrum)
        noise_db = _amplitude_to_db(noise_spectrum).to(dtype=signal_db.dtype)
        deviation, mean = torch.std_mean(noise_db, dim=-1)
        threshold = mean + deviation * self.standard_deviations
        mask = signal_db > threshold.unsqueeze(2)
        mask = self.proportion_decrease * (mask.float() - 1.0) + 1.0
        mask = conv2d(
            mask.unsqueeze(1),
            self.smoothing_filter.to(device=signal.device, dtype=mask.dtype),
            padding="same",
        ).squeeze(1)
        filtered = spectrum * mask
        return torch.istft(
            filtered,
            n_fft=self.n_fft,
            hop_length=self.hop_length,
            win_length=self.window_length,
            center=True,
            window=window,
            length=signal.shape[-1],
        ).to(dtype=signal.dtype)
