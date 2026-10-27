//! 操作音を指定されたWindows再生デバイスへ送る。
use audio_mixer_backend::{
    audio::buffer::AudioRingBuffer,
    windows::{
        devices::{resolve_endpoint_id, DeviceDirection},
        wasapi_render::start_render,
    },
};
use std::{
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

pub const LABELS: [&str; 8] = ["なし", "音1", "音2", "音3", "音4", "音5", "音6", "音7"];
const FREQUENCIES: [f32; 7] = [440., 523.25, 587.33, 659.25, 739.99, 830.61, 987.77];

fn samples(choice: u8) -> Vec<f32> {
    let Some(frequency) = choice
        .checked_sub(1)
        .and_then(|i| FREQUENCIES.get(i as usize))
    else {
        return Vec::new();
    };
    let frames = 4800;
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let fade = (i.min(frames - 1 - i) as f32 / 320.0).min(1.0);
        let x = (2.0 * std::f32::consts::PI * frequency * i as f32 / 48_000.0).sin() * fade * 0.18;
        samples.extend_from_slice(&[x, x]);
    }
    samples
}

/// 完了通知は再生デバイスの送出完了後に返す。音声制御スレッドを待たせない。
pub fn play(choice: u8, device: String) -> Receiver<Result<(), String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let samples = samples(choice);
            if samples.is_empty() {
                return Ok(());
            }
            let endpoint =
                resolve_endpoint_id(&device, DeviceDirection::Render).map_err(|e| e.to_string())?;
            let ring = AudioRingBuffer::new(samples.len() + 1024);
            let render = start_render(endpoint, ring.clone()).map_err(|e| e.to_string())?;
            render
                .wait_until_running(Duration::from_secs(3))
                .map_err(|e| e.to_string())?;
            if ring.push_slice(&samples) != samples.len() {
                return Err("効果音の出力バッファが不足しました".into());
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            while ring.len_samples() > 0 {
                if Instant::now() >= deadline {
                    return Err("効果音の再生が完了しませんでした".into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            // WASAPIのデバイス内の待機を消化する。ループバック側の残りは呼び出し元が保護する。
            let padding = render
                .padding_frames()
                .load(std::sync::atomic::Ordering::Relaxed);
            let wait_ms = u64::from(padding)
                .saturating_mul(1000)
                .div_ceil(48_000)
                .min(500)
                + 40;
            thread::sleep(Duration::from_millis(wait_ms));
            render.stop();
            Ok(())
        })();
        let _ = tx.send(result);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tones_are_short_stereo_faded_and_distinct() {
        let a = samples(1);
        let b = samples(4);
        assert_eq!(a.len(), 9600);
        assert_eq!(a[0], 0.0);
        assert_eq!(*a.last().unwrap(), 0.0);
        assert_ne!(a, b);
        assert!(samples(0).is_empty());
        assert!(a.chunks_exact(2).all(|f| f[0] == f[1]));
    }
    #[test]
    #[ignore = "既定再生デバイスから短い試験音を再生する実機試験"]
    fn plays_short_tone_on_default_render_device() {
        play(1, "default://console".into())
            .recv_timeout(Duration::from_secs(8))
            .unwrap()
            .unwrap();
    }
}
