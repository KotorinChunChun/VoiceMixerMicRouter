"""RVCの取得済み履歴で衝撃の本体と余韻を判定し、対応する出力を抑制する。

追加の音声待ちは設けない。強い部分が短く減衰する音を対象とし、継続音や
その前後は声の可能性があるため保護する。声とノイズの完全な分離器ではない。
"""
from __future__ import annotations

import numpy as np


def _regions(mask: np.ndarray):
    edges = np.diff(np.r_[False, mask, False].astype(np.int8))
    return zip(np.flatnonzero(edges == 1), np.flatnonzero(edges == -1))


def transient_gain(
    history: np.ndarray, sample_rate: int, output_start: int, output_frames: int,
    *, protection_history: np.ndarray | None = None,
) -> np.ndarray:
    """history内のサンプル位置に対応したゲイン。入力・待ち時間・出力長は不変。"""
    gain = np.ones(output_frames, dtype=np.float32)
    if sample_rate <= 0 or output_start < 0 or output_frames == 0:
        return gain
    hop = max(1, sample_rate // 200)  # 5ms
    count = len(history) // hop
    if count < 30 or not np.isfinite(history).all():
        return gain
    frames = np.asarray(history[:count * hop], dtype=np.float32).reshape(count, hop)
    rms = np.sqrt(np.mean(frames * frames, axis=1))
    protection_rms = None
    if protection_history is not None and len(protection_history) == len(history) and np.isfinite(protection_history).all():
        unfiltered = np.asarray(protection_history[:count * hop], dtype=np.float32).reshape(count, hop)
        protection_rms = np.sqrt(np.mean(unfiltered * unfiltered, axis=1))
    # 小さいクリックも拾い、背景雑音そのものを候補にしない。
    floor = np.clip(np.percentile(rms, 20), 0.00015, 0.001)
    active = rms > max(0.0006, floor * 4)
    for first, last in list(_regions(~active)):
        if first > 0 and last < count and last - first <= 8:
            active[first:last] = True  # 40ms以下の谷は同じ衝撃の余韻に含める。

    candidates = []
    protected = []
    for first, last in _regions(active):
        envelope = rms[first:last]
        event = frames[first:last].reshape(-1)
        peak = float(np.max(np.abs(event)))
        if peak < 0.003:
            continue
        cumulative = np.cumsum(envelope * envelope)
        low, high = np.searchsorted(cumulative, cumulative[-1] * np.array([0.05, 0.95]))
        energy_width_ms = (high - low + 1) * 5
        strong_ms = np.count_nonzero(envelope > np.max(envelope) * 0.3) * 5
        crest = peak / max(float(np.sqrt(np.mean(event * event))), 1e-10)
        compact = (
            last - first <= 100  # 余韻・複合音を含め最大500ms。
            and energy_width_ms <= 120
            and strong_ms <= 80
            and crest >= 3
        )
        if not compact:
            protected.append((first, last))
            continue

        # 未取得の未来を無音とみなさない。終端未確定でも、既存の先読み内で
        # ピーク後20ms以上の減衰を確認できる強い衝撃だけは本体から抑制する。
        complete = last + 4 <= count
        decayed = (
            len(envelope) >= 10
            and np.argmax(envelope) <= len(envelope) - 5
            and crest >= 5
            and np.max(envelope[-4:]) < np.max(envelope) * 0.65
        )
        if complete or decayed:
            candidates.append((first, last))

    positions = np.arange(output_start, output_start + output_frames)
    fade = max(1, sample_rate * 5 // 1000)
    for first, last in candidates:
        if first < 20:  # 直前100msの取得済み履歴が必要。
            continue
        if any(end > first - 50 and start < last + 50 for start, end in protected):
            continue  # 継続音の前後250msは語頭・語尾も保護する。
        if protection_rms is not None and last + 4 <= count:
            previous = float(np.median(protection_rms[first - 20:first - 4]))
            following = protection_rms[last:min(count, last + 8)]
            level = float(np.mean(following))
            middle = len(following) // 2
            not_decaying = np.mean(following[middle:]) >= np.mean(following[:middle]) * 0.85
            # ゲート後は無音でも、直前より高い弱い音が一定に続く場合は語頭候補。
            # 衰える余韻と異なる手掛かりを、ゲート前の同位置から保護する。
            if level > max(0.00018, previous * 3) and np.std(following) < level * 0.4 and not_decaying:
                continue
        left = first * hop - sample_rate * 15 // 1000
        right = last * hop + sample_rate * 30 // 1000
        if right + fade <= output_start or left - fade >= output_start + output_frames:
            continue
        distance = np.maximum((left - positions) / fade, (positions - right) / fade)
        local = 0.5 - 0.5 * np.cos(np.pi * np.clip(distance, 0, 1))
        np.minimum(gain, local, out=gain)
    return gain
