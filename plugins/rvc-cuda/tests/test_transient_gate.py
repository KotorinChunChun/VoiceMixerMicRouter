"""短音抑制の区間対応と、声を巻き込まない条件の回帰試験。"""
import pathlib
import sys
import unittest

import numpy as np

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from transient_gate import transient_gain

RATE = 48_000


def impact(signal, at):
    n = np.arange(480)
    signal[at:at + len(n)] += 0.5 * np.cos(2 * np.pi * 1200 * n / RATE) * np.exp(-n / 65)


class TransientTests(unittest.TestCase):
    def test_isolated_impact_is_suppressed_without_changing_length(self):
        raw = np.zeros(RATE, dtype=np.float32)
        impact(raw, 24_000)
        original = raw.copy()
        gain = transient_gain(raw, RATE, 0, len(raw))
        self.assertEqual(len(gain), len(raw))
        self.assertEqual(np.max(np.abs(raw * gain)), 0)
        self.assertTrue(np.array_equal(raw, original))
        self.assertTrue(np.all(gain[:20_000] == 1))
        self.assertGreater(np.count_nonzero((gain > 0) & (gain < 1)), 100)

    def test_speech_nearby_and_sustained_sound_are_protected(self):
        for offset, width in [(24_700, 8000), (18_000, 5500), (0, RATE)]:
            raw = np.zeros(RATE, dtype=np.float32)
            impact(raw, 24_000)
            raw[offset:offset + width] += 0.05 * np.sin(np.arange(width) * 2 * np.pi * 180 / RATE)
            self.assertTrue(np.all(transient_gain(raw, RATE, 0, len(raw)) == 1))

    def test_missing_future_is_passed_without_adding_a_buffer(self):
        raw = np.zeros(RATE, dtype=np.float32)
        impact(raw, RATE - 1000)
        self.assertTrue(np.all(transient_gain(raw, RATE, 0, len(raw)) == 1))

    def test_a_one_sample_click_is_not_lost_in_the_rms_average(self):
        raw = np.zeros(RATE, dtype=np.float32)
        raw[24_000] = 0.05
        gain = transient_gain(raw, RATE, 0, len(raw))
        self.assertEqual(gain[24_000], 0)
        self.assertEqual(len(gain), len(raw))

    def test_output_slice_and_sola_offset_follow_input_sample_positions(self):
        raw = np.zeros(RATE, dtype=np.float32)
        impact(raw, 24_000)
        entire = transient_gain(raw, RATE, 0, len(raw))
        for start in [20_000, 23_000, 24_150]:
            sliced = transient_gain(raw, RATE, start, 6000)
            self.assertTrue(np.array_equal(sliced, entire[start:start + 6000]))
        # 共有リングで後から再生しても同じ配列の同じ位置を抑制する。
        self.assertTrue(np.array_equal(entire, transient_gain(raw, RATE, 0, len(raw))))

    def test_low_level_silence_and_unusable_input_are_unchanged(self):
        for raw in [np.zeros(RATE), np.full(RATE, 0.001), np.full(RATE, np.nan)]:
            self.assertTrue(np.all(transient_gain(raw, RATE, 0, len(raw)) == 1))

    def test_quiet_click_with_long_reverberation_is_suppressed(self):
        raw = np.zeros(RATE, dtype=np.float32)
        n = np.arange(RATE // 5)
        ring = 0.012 * np.cos(2 * np.pi * 830 * n / RATE) * np.exp(-n / 750)
        raw[24_000:24_000 + len(n)] = ring
        gain = transient_gain(raw, RATE, 0, len(raw))
        self.assertLess(np.sum((raw * gain) ** 2), np.sum(raw ** 2) * 0.001)
        self.assertEqual(gain[24_000], 0)

    def test_reverberation_across_processing_boundaries(self):
        # 250ms処理・既存90ms先読みの各位置で、150msの余韻付き衝撃を渡す。
        for phase_ms in range(0, 250, 10):
            raw = np.zeros(RATE * 2, dtype=np.float32)
            at = 24_000 + phase_ms * 48
            n = np.arange(7200)
            raw[at:at + len(n)] = 0.12 * np.cos(n * 0.13) * np.exp(-n / 550)
            gain = np.ones(len(raw), dtype=np.float32)
            for end in range(12_000, len(raw) + 1, 12_000):
                start = end - 136_320
                history = np.pad(raw[max(0, start):end], (max(0, -start), 0))
                part = transient_gain(history, RATE, 120_000, 12_000)
                left, right = end - 16_320, end - 4320
                if right > 0:
                    gain[max(0, left):right] = part[max(0, -left):]
            self.assertLess(np.sum((raw * gain) ** 2), np.sum(raw ** 2) * 0.001, phase_ms)

    def test_nearby_speech_protects_quiet_word_ending(self):
        raw = np.zeros(RATE * 2, dtype=np.float32)
        raw[20_000:32_000] = 0.05 * np.sin(np.arange(12_000) * 2 * np.pi * 170 / RATE)
        impact(raw, 40_000)  # 前の継続音の終端から167ms。語尾を巻き込まない。
        self.assertTrue(np.all(transient_gain(raw, RATE, 0, len(raw)) == 1))

    def test_short_tonal_tail_is_not_mistaken_for_neighboring_speech(self):
        raw = np.zeros(RATE, dtype=np.float32)
        at = 24_000
        n = np.arange(2400)
        raw[at:at + len(n)] = 0.12 * np.cos(n * 0.13) * np.exp(-n / 450)
        tail_at = at + 3840  # 本体の後に30msの谷を挟む共鳴尾。
        raw[tail_at:tail_at + 1200] = 0.004 * np.sin(np.arange(1200) * 0.05)
        gain = transient_gain(raw, RATE, 0, len(raw))
        self.assertLess(np.sum((raw * gain) ** 2), np.sum(raw ** 2) * 0.001)

    def test_pre_gate_weak_following_voice_protects_a_short_onset(self):
        gated = np.zeros(RATE, dtype=np.float32)
        impact(gated, 24_000)
        reference = gated.copy()
        # 既存入力ゲートには消される小さな継続音。衝撃の減衰とは区別する。
        reference[24_480:29_000] += 0.0005 * np.sin(np.arange(4520) * 0.3)
        self.assertEqual(transient_gain(gated, RATE, 0, len(gated))[24_000], 0)
        gain = transient_gain(gated, RATE, 0, len(gated), protection_history=reference)
        self.assertTrue(np.all(gain == 1))

    def test_pre_gate_decaying_tail_does_not_protect_the_impact(self):
        gated = np.zeros(RATE, dtype=np.float32)
        impact(gated, 24_000)
        reference = gated.copy()
        n = np.arange(4520)
        reference[24_480:29_000] += 0.001 * np.sin(n * 0.3) * np.exp(-n / 1400)
        gain = transient_gain(gated, RATE, 0, len(gated), protection_history=reference)
        self.assertEqual(gain[24_000], 0)

    def test_sustained_vowel_and_continuous_noise_are_protected(self):
        rng = np.random.default_rng(2026)
        for signal in [0.03 * np.sin(np.arange(12_000) * 0.02), rng.normal(0, 0.03, 12_000)]:
            raw = np.zeros(RATE, dtype=np.float32)
            raw[18_000:30_000] = signal
            self.assertTrue(np.all(transient_gain(raw, RATE, 0, len(raw)) == 1))


if __name__ == '__main__':
    unittest.main()
