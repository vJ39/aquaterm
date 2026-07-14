// 効果音(SE): 外部の音源ファイル(wav/mp3等)は一切使わず、正弦波の周波数スイープ+
// 速い減衰(ディケイ)をコード側でその場合成して鳴らすだけの短いビープ音。
//
// 音質メモ: 単一の定常サイン波を鳴らしっぱなしにすると「笛のような単調な音」に
// 聞こえて安っぽくなるため、必ず (a) 時間とともにピッチが変化するスイープと
// (b) 立ち上がりが速く・すぐ減衰するエンベロープ、の組み合わせにする。
//
// 重要: オーディオ出力デバイスが無い/初期化に失敗する環境(この開発環境のような
// コンテナ/CI相当を含む)でも panic・クラッシュせず、SEなしで通常通り動作する。
// 初期化やその後の再生に失敗した場合は、そのエラーを握りつぶして黙って何もしない。
//
// feature切り分け: 実機フィードバック(「音無しでもクロスコンパイルを通したい」)対応。
// Linux向けクロスコンパイル環境では rodio の依存(alsa-sys)が pkg-config 経由で
// ALSA を見つけられずビルドに失敗することがあるため、`sound` feature(既定でON)で
// rodio 依存自体を切り離せるようにする。呼び出し側(sim.rs/main.rs)は
// `SoundEngine::new()`/`.play()` という同じAPIをfeatureの有無に関わらず使えるよう、
// feature切り分けはこのファイル(sound.rs)の中だけに閉じ込める。

#[cfg(feature = "sound")]
mod real {
    use crate::sim::SfxEvent;
    use rodio::{DeviceSinkBuilder, MixerDeviceSink, Source};
    use std::num::NonZero;
    use std::time::Duration;

    const SAMPLE_RATE: u32 = 44100;

    // 周波数スイープ(start_freq→end_freqへ指数補間)+ アタック→指数減衰のエンベロープを持つ
    // 短いビープ音。delay_ms 分の無音を先頭に挟めることで、Mixer::add で同時に鳴らした
    // 複数の音を疑似的に時間差(アルペジオ等)で聞かせられる。
    struct Tone {
        start_freq: f32,
        end_freq: f32,
        total_samples: usize, // 無音の遅延ぶんも含めた総サンプル数
        delay_samples: usize,
        decay: f32, // 大きいほど速く減衰する(1/秒)
        volume: f32,
        i: usize,
        phase: f32,
    }

    impl Tone {
        fn new(start_freq: f32, end_freq: f32, duration_ms: u64, decay: f32, volume: f32, delay_ms: u64) -> Self {
            let delay_samples = (SAMPLE_RATE as u64 * delay_ms / 1000) as usize;
            let tone_samples = (SAMPLE_RATE as u64 * duration_ms / 1000).max(1) as usize;
            Tone {
                start_freq,
                end_freq,
                total_samples: delay_samples + tone_samples,
                delay_samples,
                decay,
                volume,
                i: 0,
                phase: 0.0,
            }
        }
    }

    impl Iterator for Tone {
        type Item = f32;

        fn next(&mut self) -> Option<f32> {
            if self.i >= self.total_samples {
                return None;
            }
            if self.i < self.delay_samples {
                self.i += 1;
                return Some(0.0); // アルペジオ等のための先頭無音
            }
            let tone_i = (self.i - self.delay_samples) as f32;
            let tone_len = (self.total_samples - self.delay_samples) as f32;
            let t = tone_i / SAMPLE_RATE as f32;
            let progress = (tone_i / tone_len).clamp(0.0, 1.0);

            // ピッチスイープ: 周波数は指数(対数)補間で変化させる方が音程として自然に聞こえる
            let ratio = (self.end_freq / self.start_freq).max(1e-6);
            let freq_now = self.start_freq * ratio.powf(progress);

            // 周波数が時間変化するため、位相は毎サンプル積算して求める(チャープの正しい生成法)
            self.phase += 2.0 * std::f32::consts::PI * freq_now / SAMPLE_RATE as f32;
            if self.phase > 2.0 * std::f32::consts::PI {
                self.phase -= 2.0 * std::f32::consts::PI;
            }

            // アタック(3ms程度)で立ち上がり、その後は指数減衰(ディケイ)で速く減衰する。
            // 定常音にならないため、単調な「笛のような音」に聞こえるのを防ぐ。
            let attack = 0.003f32;
            let env = if t < attack {
                t / attack
            } else {
                (-self.decay * (t - attack)).exp()
            };

            self.i += 1;
            Some(self.phase.sin() * env * self.volume)
        }
    }

    impl Source for Tone {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> NonZero<u16> {
            // 1(モノラル)は常に非ゼロなので unwrap は安全
            NonZero::new(1).unwrap()
        }
        fn sample_rate(&self) -> NonZero<u32> {
            // SAMPLE_RATE は非ゼロの定数なので unwrap は安全
            NonZero::new(SAMPLE_RATE).unwrap()
        }
        fn total_duration(&self) -> Option<Duration> {
            Some(Duration::from_secs_f32(
                self.total_samples as f32 / SAMPLE_RATE as f32,
            ))
        }
    }

    // (開始周波数, 終了周波数, 音の長さms, 減衰の速さ, 音量, 開始までの遅延ms)
    type ToneSpec = (f32, f32, u64, f32, f32, u64);

    // イベントごとの音色。複数指定した場合は同時に鳴らし始める(delay_ms で時間差をつけられる)。
    fn tones_for(event: SfxEvent) -> Vec<ToneSpec> {
        match event {
            // 気泡: 低め→高めへの短いピッチアップスイープ、音量小さめ
            SfxEvent::Bubble => vec![(700.0, 1300.0, 55, 45.0, 0.09, 0)],
            // 餌: 高め→低めへ短時間で下降するピッチスイープ+速い減衰(「ぽちゃん」という水滴の質感)。
            // 実機フィードバック(「まだ音の余韻が長い。もっと短く乾いた音に」)を受けて、
            // 持続時間・減衰(decay)をさらに切り詰めた(旧70ms・decay32.0)。
            SfxEvent::Feed => vec![(950.0, 380.0, 30, 90.0, 0.20, 0)],
            // 薬: 餌よりやや低め・やや長めのピッチダウンスイープ。餌と同じ実機フィードバックを
            // 受けて、持続時間・減衰をさらに切り詰めた(旧110ms・decay20.0)。
            SfxEvent::Medicate => vec![(620.0, 240.0, 45, 70.0, 0.20, 0)],
            // 病気: 2音のわずかに不協和な音程で、ゆっくり下降
            SfxEvent::SickOnset => vec![
                (260.0, 225.0, 260, 9.0, 0.16, 0),
                (247.0, 210.0, 260, 9.0, 0.13, 0),
            ],
            // 回復: 明るい2〜3音の短いアルペジオ(上昇。delay_msで時間差をつける)
            SfxEvent::Cured => vec![
                (520.0, 560.0, 90, 18.0, 0.20, 0),
                (660.0, 700.0, 90, 18.0, 0.20, 70),
                (880.0, 940.0, 130, 14.0, 0.22, 140),
            ],
            // 空腹警告: 短く低めの一発音
            SfxEvent::HungryOnset => vec![(300.0, 260.0, 130, 16.0, 0.15, 0)],
            // ガラスを叩く: 立ち上がりが速く、すぐ減衰する短くパーカッシブな硬い音
            SfxEvent::GlassKnock => vec![(900.0, 140.0, 45, 55.0, 0.22, 0)],
            // ピラニアの捕食: 低くて速い「がぶっ」という一瞬の噛みつき音
            SfxEvent::Predation => vec![(480.0, 90.0, 70, 42.0, 0.24, 0)],
            // 墨: こもった低い「ぶしゅっ」という一発音(勢いよく吐き出すイメージ)
            SfxEvent::Ink => vec![(220.0, 60.0, 40, 30.0, 0.30, 0)],
        }
    }

    // 効果音の再生エンジン。デバイス初期化に失敗した場合は `sink` が None になり、
    // 以降の play() 呼び出しは何もせず静かに無視する。
    pub struct SoundEngine {
        // MixerDeviceSink 自体を保持し続けないと(Drop で)出力ストリームが止まってしまうため保持する。
        sink: Option<MixerDeviceSink>,
    }

    impl SoundEngine {
        pub fn new() -> Self {
            match DeviceSinkBuilder::open_default_sink() {
                Ok(mut sink) => {
                    // デバイス切断等でこのシンクが drop される際、既定だと stderr にログを
                    // 出す仕様になっている。ターミナルUIの邪魔になるため無効化する。
                    sink.log_on_drop(false);
                    SoundEngine { sink: Some(sink) }
                }
                // オーディオ出力デバイスが無い/初期化失敗。SEなしで動作を続ける。
                Err(_) => SoundEngine { sink: None },
            }
        }

        // イベントに対応する音を鳴らす。デバイスが無い場合は何もしない。
        pub fn play(&self, event: SfxEvent) {
            let Some(sink) = &self.sink else {
                return;
            };
            let mixer = sink.mixer();
            for (start_freq, end_freq, dur_ms, decay, vol, delay_ms) in tones_for(event) {
                mixer.add(Tone::new(start_freq, end_freq, dur_ms, decay, vol, delay_ms));
            }
        }
    }

    impl Default for SoundEngine {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(feature = "sound")]
pub use real::SoundEngine;

// `sound` feature無効時のダミー実装。rodio依存を一切持たず、常に何もしない
// (音無しビルド)。呼び出し側から見て real::SoundEngine と同じAPI形状にすることで、
// sim.rs/main.rs 側のコードは無変更で済むようにしている。
#[cfg(not(feature = "sound"))]
mod dummy {
    use crate::sim::SfxEvent;

    pub struct SoundEngine;

    impl SoundEngine {
        pub fn new() -> Self {
            SoundEngine
        }

        // 音無しビルドでは常に何もしない。
        pub fn play(&self, _event: SfxEvent) {}
    }

    impl Default for SoundEngine {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(not(feature = "sound"))]
pub use dummy::SoundEngine;
