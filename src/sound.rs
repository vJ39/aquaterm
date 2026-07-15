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
// feature切り分け: 音無しでもクロスコンパイルを通したいという要望への対応。
// Linux向けクロスコンパイル環境では rodio の依存(alsa-sys)が pkg-config 経由で
// ALSA を見つけられずビルドに失敗することがあるため、`sound` feature(既定でON)で
// rodio 依存自体を切り離せるようにする。呼び出し側(sim.rs/main.rs)は
// `SoundEngine::new()`/`.play()` という同じAPIをfeatureの有無に関わらず使えるよう、
// feature切り分けはこのファイル(sound.rs)の中だけに閉じ込める。

#[cfg(feature = "sound")]
mod real {
    use crate::rng::Rng;
    use crate::sim::SfxEvent;
    use rodio::{DeviceSinkBuilder, MixerDeviceSink, Source};
    use std::cell::RefCell;
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

    // 気泡音のバリエーション一覧。単一のスイープパターンだけだと毎回同じ音に聞こえて
    // 単調になるため、周波数帯・持続時間・減衰・音量を振った複数パターンを用意し、
    // 鳴らすたびにランダムに1つを選ぶ(choose_bubble_variant参照)。いずれも低め→高めへ
    // 短くピッチアップするスイープ・音量小さめという方向性は共通にして、気泡音として
    // 聞き分けられる範囲に収めている。
    const BUBBLE_VARIANTS: [ToneSpec; 5] = [
        (700.0, 1300.0, 55, 45.0, 0.09, 0), // 元の音: 中庸な高さ・速い減衰
        (420.0, 850.0, 75, 28.0, 0.10, 0),  // 低め・やや長め・ゆるい減衰(大きい気泡)
        (1000.0, 1750.0, 40, 62.0, 0.08, 0), // 高め・短め・速い減衰(小さい気泡)
        (600.0, 1080.0, 60, 36.0, 0.095, 0), // 中低音・中程度の減衰
        (880.0, 1550.0, 48, 50.0, 0.085, 0), // 中高音・やや速い減衰
    ];

    // 気泡音のバリエーションを1つランダムに選ぶ。RNGを外から渡す形にして、実際の音声
    // 再生(rodioのシンク)なしに選択ロジックだけを単体テストできるようにしている。
    fn choose_bubble_variant(rng: &mut Rng) -> ToneSpec {
        let idx = rng.range_usize(0, BUBBLE_VARIANTS.len() - 1);
        BUBBLE_VARIANTS[idx]
    }

    // イベントごとの音色。複数指定した場合は同時に鳴らし始める(delay_ms で時間差をつけられる)。
    // rng は気泡音のようにイベント自体に複数バリエーションを持たせる場合にのみ使う
    // (他のイベントは固定の音色のため未使用)。
    fn tones_for(event: SfxEvent, rng: &mut Rng) -> Vec<ToneSpec> {
        match event {
            // 気泡: BUBBLE_VARIANTSからランダムに1つ選ぶ(低め→高めへの短いピッチアップ
            // スイープ、音量小さめという方向性は全パターン共通)
            SfxEvent::Bubble => vec![choose_bubble_variant(rng)],
            // 餌: 高め→低めへ短時間で下降するピッチスイープ+速い減衰(「ぽちゃん」という水滴の質感)。
            // 音の余韻が長く、もっと短く乾いた音にすべきという指摘を受けて、
            // 持続時間・減衰(decay)をさらに切り詰めた(旧70ms・decay32.0)。
            SfxEvent::Feed => vec![(950.0, 380.0, 30, 90.0, 0.20, 0)],
            // 薬: 餌よりやや低め・やや長めのピッチダウンスイープ。餌と同じ指摘を
            // 受けて、持続時間・減衰をさらに切り詰めた(旧110ms・decay20.0)。
            SfxEvent::Medicate => vec![(620.0, 240.0, 45, 70.0, 0.20, 0)],
            // 浄化剤: 明るく弾ける(炭酸のような)上昇3音の短いチャイム。Cured(治療の
            // アルペジオ)・StarPickup(キラキラ)とは音程・減衰・音量で差別化する。
            SfxEvent::Purify => vec![
                (700.0, 780.0, 55, 26.0, 0.17, 0),
                (1000.0, 1080.0, 55, 26.0, 0.17, 45),
                (1500.0, 1620.0, 70, 22.0, 0.18, 90),
            ],
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
            // トントン(Tキー): GlassKnockの硬い一発音とは対照的に、狭いピッチ幅・
            // 低めの音量・ゆるい減衰の柔らかい2音を短い間隔で鳴らし、「トン・トン」
            // という軽く優しいノックの質感にする。
            SfxEvent::Tap => vec![
                (480.0, 410.0, 45, 30.0, 0.14, 0),
                (460.0, 390.0, 45, 30.0, 0.12, 95),
            ],
            // トグルON: 極短・高速減衰・小音量の乾いたクリック音(やかましくならないように)
            SfxEvent::UiClick => vec![(1200.0, 850.0, 12, 140.0, 0.09, 0)],
            // スター取得: 控えめな音量・柔らかい減衰の、明るく上昇する2音の
            // キラキラしたアルペジオ(やかましくならないように音量は抑えめにする)。
            SfxEvent::StarPickup => vec![
                (900.0, 1300.0, 70, 20.0, 0.14, 0),
                (1300.0, 1700.0, 90, 16.0, 0.12, 60),
            ],
            // クジラ大爆発: 低く重い「ドーン」という一発の巨大な爆発音。既存のどの音
            // よりも低い周波数・長い持続時間・大きい音量にして、ネタ枠の大事件だと
            // はっきり分かるようにする(2音を少し時間差で重ねて重量感を出す)。
            SfxEvent::WhaleExplosion => vec![
                (90.0, 28.0, 500, 6.0, 0.38, 0),
                (55.0, 18.0, 650, 4.0, 0.32, 40),
            ],
        }
    }

    // 効果音の再生エンジン。デバイス初期化に失敗した場合は `sink` が None になり、
    // 以降の play() 呼び出しは何もせず静かに無視する。
    pub struct SoundEngine {
        // MixerDeviceSink 自体を保持し続けないと(Drop で)出力ストリームが止まってしまうため保持する。
        sink: Option<MixerDeviceSink>,
        // 気泡音などのバリエーション選択専用のRNG。sim.rs側のRNG(決定的・シミュレーション
        // 状態の再現性が必要)とは無関係な、鳴らす音を毎回変える演出用の乱数なので、
        // 独立して保持する(RefCell: play()は&selfのため内部で可変に使う)。
        rng: RefCell<Rng>,
    }

    impl SoundEngine {
        pub fn new() -> Self {
            match DeviceSinkBuilder::open_default_sink() {
                Ok(mut sink) => {
                    // デバイス切断等でこのシンクが drop される際、既定だと stderr にログを
                    // 出す仕様になっている。ターミナルUIの邪魔になるため無効化する。
                    sink.log_on_drop(false);
                    SoundEngine {
                        sink: Some(sink),
                        rng: RefCell::new(Rng::from_time()),
                    }
                }
                // オーディオ出力デバイスが無い/初期化失敗。SEなしで動作を続ける。
                Err(_) => SoundEngine {
                    sink: None,
                    rng: RefCell::new(Rng::from_time()),
                },
            }
        }

        // イベントに対応する音を鳴らす。デバイスが無い場合は何もしない。
        pub fn play(&self, event: SfxEvent) {
            let Some(sink) = &self.sink else {
                return;
            };
            let mixer = sink.mixer();
            let mut rng = self.rng.borrow_mut();
            for (start_freq, end_freq, dur_ms, decay, vol, delay_ms) in tones_for(event, &mut rng) {
                mixer.add(Tone::new(start_freq, end_freq, dur_ms, decay, vol, delay_ms));
            }
        }
    }

    impl Default for SoundEngine {
        fn default() -> Self {
            Self::new()
        }
    }

    // 実際の音声出力(rodioのシンク)が無いheadless環境でも実行できる、パラメータ
    // 選択ロジックのテスト。音そのものの聞こえ方は検証できないため、
    // 「複数バリエーションが用意されていて、実際にランダムに選ばれること」を確認する。
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn bubble_has_multiple_distinct_variants() {
            // 聞き分けられる程度のバリエーション数が欲しい(1種類だけの単調さを解消する)。
            assert!(
                BUBBLE_VARIANTS.len() >= 3,
                "気泡音のバリエーションは複数(3種以上)欲しい"
            );
            // 開始周波数が全パターンで重複しない(=見た目上、別々の音色として
            // 定義されている)ことを確認する。
            let mut start_freqs: Vec<u32> = BUBBLE_VARIANTS.iter().map(|v| v.0.to_bits()).collect();
            start_freqs.sort_unstable();
            start_freqs.dedup();
            assert_eq!(
                start_freqs.len(),
                BUBBLE_VARIANTS.len(),
                "各バリエーションの開始周波数は重複しないはず"
            );
        }

        #[test]
        fn choose_bubble_variant_actually_varies_across_many_draws() {
            // 固定シードのRNGで十分な回数選ばせ、複数の異なるバリエーションが
            // 実際に選ばれること(=常に同じ1パターンに固定されていないこと)を確認する。
            let mut rng = Rng::new(42);
            let mut seen: Vec<u32> = Vec::new();
            for _ in 0..200 {
                let spec = choose_bubble_variant(&mut rng);
                let key = spec.0.to_bits();
                if !seen.contains(&key) {
                    seen.push(key);
                }
            }
            assert!(
                seen.len() > 1,
                "200回選べば複数のバリエーションが選ばれるはず(実際に選ばれたのは{}種類)",
                seen.len()
            );
            // BUBBLE_VARIANTSの範囲外を指していないことも確認する。
            for key in &seen {
                assert!(
                    BUBBLE_VARIANTS.iter().any(|v| v.0.to_bits() == *key),
                    "選ばれたバリエーションはBUBBLE_VARIANTSの範囲内であるはず"
                );
            }
        }

        #[test]
        fn choose_bubble_variant_never_panics_regardless_of_seed() {
            // range_usize の範囲指定ミス(境界外インデックス)が無いことを、複数シードで
            // 回してみて確認する(パニックしなければOK)。
            for seed in [1u64, 2, 100, 999_999, u64::MAX] {
                let mut rng = Rng::new(seed);
                for _ in 0..20 {
                    let _ = choose_bubble_variant(&mut rng);
                }
            }
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
