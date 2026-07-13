// 水槽シミュレーション本体: 魚の遊泳・餌・薬・卵・気泡の更新、育成ロジック
// (空腹度・成長・産卵→孵化・病気・死亡)。端末描画には依存しない純粋なロジック。

use crate::fish::{Fish, Species, Stage};
use crate::rng::Rng;
use serde::{Deserialize, Serialize};

// --- 空腹度の段階しきい値(fish.rs の hunger_level が参照) ---
pub const MAX_HUNGER: f64 = 100.0;
pub const FULL_THRESHOLD: f64 = 75.0; // これ以上で「満腹」
pub const HUNGRY_THRESHOLD: f64 = 50.0; // これ未満で「腹ぺこ」= 餌を探す

// --- 育成パラメータ(すべて秒単位) ---
// 方針(死亡について v2・最終): 死亡ロジックは復活させるが、猶予を大幅に延ばし、
// 死亡時は仰向けで浮上する演出を挟んでから消える。
// 注記: 仕様書は「空腹度0→死亡の合計が最低10分以上」と明記する一方、内訳例
// (弱り2分+死亡3分=計5分)はその最低ラインと矛盾する。ここでは明記された
// 「最低10分以上」を優先し、STARVE_DEATH_TIME を starve_timer(空腹度0からの経過)
// そのものへの閾値として 600秒(10分)超に設定する(内訳例より安全側)。
// 空腹度の毎秒減少量。実機フィードバック(「減衰が早すぎる」)を受けて大幅に緩め、
// 満タン→0まで約60分(1時間)になるよう調整した(旧: 約10分/600秒)。
// 腹ぺこ閾値(50、満タンからの半分)到達は概ね30分の計算になる。
// 弱り・死亡までの猶予(STARVE_WEAK_TIME/STARVE_DEATH_TIME)は変更しない。
pub const HUNGER_DECAY: f64 = 100.0 / 3600.0;
pub const FEED_AMOUNT: f64 = 34.0; // 餌1粒で回復する空腹度
pub const WELL_FED_THRESHOLD: f64 = 60.0; // 成長・産卵の満腹判定
pub const GROW_TIME: f64 = 30.0; // 満腹維持で稚魚→成魚
pub const BREED_READY_TIME: f64 = 22.0; // 成魚が満腹維持でこの時間経つと産卵可能
// 産卵可能時、毎秒の産卵確率。実機フィードバック(「ポコポコ生まれすぎる」)を受けて
// 大幅に下げた(旧0.06/秒→数十分に1回程度のペースを狙う。実機体感で調整)。
pub const BREED_CHANCE_PER_SEC: f64 = 0.0008;
pub const EGG_HATCH_TIME: f64 = 14.0; // 卵が孵化するまでの時間
pub const STARVE_WEAK_TIME: f64 = 120.0; // 空腹度0からおよそ2分で「弱っている」
pub const STARVE_DEATH_TIME: f64 = 630.0; // 空腹度0からおよそ10.5分(>=10分)で力尽きる

// --- 成長段階(全種共通・成魚後のさらなるサイズ成長) ---
// 稚魚→成魚(Stage)とは別に、成魚になった後も満腹維持を続けると段階的にサイズが
// 大きくなる。上限を設けて無限に大きくならないようにする(0..=3の4段階)。
pub const GENERAL_MAX_GROWTH_STAGE: u8 = 3;
pub const SIZE_GROW_TIME: f64 = 90.0; // 満腹維持がこの時間続くごとに1段階サイズアップ
pub const GENERAL_GROWTH_SCALE_STEP: f64 = 0.15; // 1段階あたりの見た目拡大率
// 成長段階が上がるほど、泳ぐ速度がやや遅くなる(必須ではないが体感の変化として付与)
pub const SIZE_SPEED_PENALTY_STEP: f64 = 0.05;

// --- サメの捕食によるサイズ成長 ---
// サメは魚を1匹捕食するごとに段階的に大きくなる(0..=3の4段階、上限あり)。
// 全種共通の成長段階とは別枠で、サメの場合は両方が積み重なって見た目に反映される。
pub const SHARK_MAX_KILL_STAGE: u8 = 3;
pub const SHARK_KILL_GROWTH_SCALE_STEP: f64 = 0.18;

// --- 寿命・世代交代(全種共通。サメも含む) ---
// 空腹度の時間感覚(満タン→0まで約60分)に合わせ、寿命は数時間〜半日のイメージで調整。
// 老齢(ELDERLY_AGE)に達すると、満腹状態などの条件を問わず「次世代を残す最後の
// チャンス」として確定で1回だけ産卵する(産卵確率アップではなく一度きりの確定イベント。
// 「老いると産卵確率が上がる」は生物学的に不自然という指摘を受けての方針)。
// サメは対象外(サメは`S`キー以外で増えない方針のため、老齢確定産卵イベントも発生しない)。
// さらに年を取ると(LIFESPAN_DEATH_AGE)、老衰で寿命死する(死亡演出は既存のものを流用)。
pub const ELDERLY_AGE: f64 = 4.0 * 3600.0; // 4時間で老齢入り(この瞬間に確定で1回産卵)
pub const LIFESPAN_DEATH_AGE: f64 = 8.0 * 3600.0; // 8時間で老衰死(数時間〜半日の範囲)

// --- 病気パラメータ ---
pub const HUNGRY_SICK_TIME: f64 = 10.0; // 腹ぺこがこの時間続くと発症判定対象
// 個体数/上限がこれ以上で過密=発症判定対象。実機フィードバックを受けて0.9→0.95に上げ、
// 上限にかなり近づかないと過密判定に入らないようにした。
pub const OVERCROWD_RATIO: f64 = 0.95;
// 発症条件下での毎秒発症確率。実機フィードバック(「すぐ病気になりすぎる」)を受けて
// 大幅に下げた(旧0.03/秒)。指示の目安値0.005/秒で実機計測したところ、5匹が同時に
// 発症条件を満たす状況で75秒(x4速度)以内に2匹発症してしまい、目標(最高速度で
// 水槽全体10分に1回程度)よりかなり頻発することが判明したため、実測値から逆算して
// さらに約1/60に下げた(0.005→0.0001)。
pub const DISEASE_CHANCE_PER_SEC: f64 = 0.0001;
pub const SICK_WEAK_TIME: f64 = 60.0; // 病気からおよそ1分で「弱っている」
// 空腹側(STARVE_DEATH_TIME)と同水準の猶予にする(病気だけ極端に早死にしないよう揃える)
pub const SICK_DEATH_TIME: f64 = 630.0;

// --- 死亡演出パラメータ ---
pub const DEAD_FLOAT_SPEED: f64 = 5.0; // 死んだ魚が水面近くまで浮上する速度(px/秒)
pub const DEAD_SURFACE_MARGIN: f64 = 3.0; // 水面からこの位置まで浮いたら静止する
pub const DEAD_FLOAT_TIME: f64 = 16.0; // 浮いた状態を維持してから水槽から消えるまでの時間

// --- 観賞用の追加生物(育成ロジック対象外。方針転換によりカニのみ。
// 大型魚は Species::Shark として通常の育成対象に統合された) ---
pub const CRAB_COUNT: usize = 3;
pub const CRAB_SPEED: f64 = 3.0; // 水底を歩く速さ
pub const CRAB_PAUSE_CHANCE_PER_SEC: f64 = 0.15; // 毎秒、立ち止まる確率
pub const CRAB_EAT_RADIUS: f64 = 3.0; // カニが水底の餌・薬を片付けられる距離
pub const SEABED_ITEM_CAP: usize = 30; // 水底に停留できる餌・薬それぞれの上限数(超過分は古い順に消える)
// 水底の食べ残しが「積もって山になる」見た目のためのパラメータ。
// 近くに既に着地済みのものが多いほど高く積み上げ、離れるほど低くなるので
// 自然と裾野の広がった山型になる。
pub const PILE_RADIUS: f64 = 2.5; // この距離以内を「同じ山」とみなす
pub const PILE_STACK_STEP: f64 = 0.6; // 近くの着地済み1個につき盛り上がる高さ
pub const PILE_MAX_HEIGHT: f64 = 3.0; // 山の最大の高さ(水底からの盛り上がり量)

// --- サメの捕食 ---
pub const SHARK_HUNT_HUNGER_THRESHOLD: f64 = 70.0; // 空腹度がこれ未満のときだけ捕食行動を取る
pub const SHARK_HUNT_RADIUS: f64 = 22.0; // この距離以内の獲物へ近づいていく
pub const SHARK_STRIKE_RADIUS: f64 = 3.5; // この距離まで近づいたら捕食が成立する
// 獲物へ近づく吸引の強さ。「追いかけている」のが見た目でわかるよう、通常の遊泳を
// 弱めた上でこれを強くかける(旧26.0→大幅に強化)。
pub const SHARK_HUNT_PULL: f64 = 100.0;
pub const SHARK_HUNT_COOLDOWN: f64 = 45.0; // 捕食後、次に捕食できるようになるまでの時間
// サメは追跡(捕食モード)中だけ、通常3種の最高速度(最速はネオンの22.0)より
// はっきり速くなるようにする倍率。巡回中(追跡していないとき)は通常のsp.max_speed()
// のままで特別早くしない。魚が機敏に逃げても追いつかれることがある緊張感を出す。
pub const SHARK_CHASE_SPEED_MULT: f64 = 1.8;
pub const SHARK_PREDATION_HUNGER_GAIN: f64 = 70.0; // 捕食による空腹度の回復量(餌より効率的)
// 血飛沫演出: 実機フィードバック(「もっと派手・グロテスクに強化してほしい」)を受けて、
// 単一の一瞬エフェクトから、複数粒子が散らばって尾を引くように少しずつ消える演出に強化した。
pub const BLOOD_EFFECT_LIFETIME: f64 = 1.6; // 表示時間(旧0.5秒→1〜2秒程度に延長)
pub const BLOOD_PARTICLE_COUNT: usize = 10; // 捕食1回あたりに散らす粒子数(旧: 1個のみ)
pub const BLOOD_SPREAD_RADIUS: f64 = 6.0; // 粒子が散らばる範囲(旧の波紋演出より広め)
// 血の滲み(範囲エフェクト): 捕食位置の周辺に赤みが水中に広がる演出。パーティクルより
// 長く残り、時間とともにゆっくりフェードアウトする(既存の水槽グラデーションに赤を
// 混ぜて表示するイメージ)。
pub const BLOOD_STAIN_LIFETIME: f64 = 4.0; // 3〜5秒程度
// 実機フィードバック(「ぜんぜんグロテスクじゃない」)を受けて、固定半径のまま薄く
// フェードするだけの実装から、時間経過で半径が広がっていく同心円の波紋アニメーション
// に変更した。最大半径は旧定数(7.0)の約3倍・混色の強さも旧0.6→0.85に強化する
// (実際の拡大計算・混色計算は描画側=main.rsのdraw_species_dex付近で行う)。
pub const BLOOD_STAIN_MAX_RADIUS: f64 = 20.0;
// 実機フィードバック(「数秒間まっかに見えるくらい濃く」)を受けて0.85→0.93まで強化。
// はっきりインパクトのある赤にする(控えめにしない)。
pub const BLOOD_STAIN_MIX: f64 = 0.93;
// 発生から寿命のこの割合までは、広がりながらも混色の強さを最大近くで維持する
// (「数秒間まっか」に見せるための保持区間)。残りの区間でフェードアウトする。
pub const BLOOD_STAIN_HOLD_FRACTION: f64 = 0.5;
// 通常の魚が「今まさに捕食モードのサメ」を検知して逃げる距離・強さ。
// 空腹でない/クールダウン中のサメは対象にならない(気にせず普段どおり泳ぐ)。
pub const SHARK_FEAR_RADIUS: f64 = 26.0;
pub const SHARK_FEAR_STRENGTH: f64 = 90.0;

// --- 餌・薬・気泡パラメータ ---
pub const FOOD_SINK_SPEED: f64 = 7.0; // 餌の沈降速度(px/秒)
pub const FOOD_LIFETIME: f64 = 26.0; // 餌の寿命(秒)
pub const EAT_RADIUS: f64 = 3.2; // 魚が餌を食べられる距離
pub const MED_SINK_SPEED: f64 = 5.0; // 薬の沈降速度
pub const MED_LIFETIME: f64 = 26.0; // 薬の寿命
pub const CURE_RADIUS: f64 = 3.2; // 病気の魚が薬で治る距離
pub const NEIGHBOR_RADIUS: f64 = 16.0; // 群れ判定の近傍距離
// 腹ぺこ時、餌への吸引ベクトルを通常の遊泳ベクトルよりはっきり優先させるための係数。
// 実機フィードバック(「腹ペコの魚が餌にめっちゃ寄ってくる」ようにしてほしい)に対応。
pub const HUNGRY_FOOD_PULL_BOOST: f64 = 4.0; // 吸引ベクトル自体の倍率
pub const HUNGRY_NORMAL_MOVE_DAMP: f64 = 0.2; // 餌を追っている間、ランダムウォーク/群れを弱める倍率

// --- 投下エフェクト(f/m を押した瞬間、投下位置に一瞬だけ出る光/波紋) ---
pub const DROP_EFFECT_LIFETIME: f64 = 0.45; // 1秒未満で消える
pub const DROP_EFFECT_MAX_RADIUS: f64 = 3.5; // 波紋が広がる最大半径(論理ピクセル)

// --- ガラスを叩く(t キー) ---
pub const KNOCK_RADIUS: f64 = 18.0; // この距離以内の魚が驚いて逃げる
pub const FLEE_DURATION: f64 = 1.2; // 逃走状態を維持する時間(秒)
pub const FLEE_STRENGTH: f64 = 140.0; // 逃走方向への加速の強さ
// サメに驚いて逃げ続けている間、逃走状態を維持する最低時間(危険が去れば自然に減衰する)
pub const SHARK_FEAR_FLEE_MARK: f64 = 1.0;
// 逃走コスト: 逃げるのにエネルギーを使うという想定で、逃走が始まった瞬間(既に
// 逃走中でなければ)に空腹度を一定量消費する。ガラスの驚き逃げ・サメからの逃走の
// どちらも同じ考え方で消費する。連打・長時間の張り込みで無限に減り続けないよう、
// 「既に逃走中は再課金しない」ことで1回の危険イベントにつき1回だけ課金する。
pub const FLEE_HUNGER_COST: f64 = 6.0;
// 回避動作を「回り込み」らしく見せるための横(垂直)方向の切り返し成分。
// 逃走方向に対して垂直な成分を時間で振動させ、真っ直ぐ離れるだけでなく
// ジグザグに切り返しながら回り込むような、生き物らしい動きにする。
pub const ZIGZAG_FREQ: f64 = 3.0; // 切り返しの速さ
pub const ZIGZAG_RATIO: f64 = 0.6; // 主となる逃走ベクトルに対する垂直成分の強さの比率

// --- ランダムな瞬発ダッシュ(特定のトリガーが無い通常時の躍動感演出) ---
// サメ・餌などのトリガーが無い普段の遊泳中でも、低頻度・ランダムなタイミングで
// 一瞬だけ通常より速く動く「ダッシュ」を行う。頻発すると落ち着きがなく見えるため、
// 数十秒に1回あるかないか程度の頻度に抑える。
pub const DASH_CHANCE_PER_SEC: f64 = 0.02; // 期待間隔=約50秒に1回
pub const DASH_DURATION: f64 = 0.35; // ダッシュ自体は一瞬だけ
pub const DASH_STRENGTH: f64 = 160.0; // ダッシュ方向への加速の強さ
pub const DASH_SPEED_MULT: f64 = 1.5; // ダッシュ中、最高速度が一時的にこの倍率になる

// --- ガラスを叩く「叩きすぎ」ペナルティ(ストレス) ---
// 短時間に何度も t を連打すると、ストレスとして周辺の魚に病気発症のボーナスを与える。
// 稀にしか叩かない通常利用では閾値に届かず影響が出ない。
pub const KNOCK_SPAM_WINDOW: f64 = 45.0; // この秒数以内の叩き回数を数える
pub const KNOCK_SPAM_THRESHOLD: usize = 4; // この回数以上叩くと「叩きすぎ」
pub const KNOCK_STRESS_RADIUS: f64 = 24.0; // ストレスを与える範囲(KNOCK_RADIUSより少し広い)
pub const KNOCK_STRESS_DURATION: f64 = 60.0; // ストレス状態が続く時間
pub const KNOCK_STRESS_DISEASE_MULT: f64 = 6.0; // ストレス中、発症確率に掛かる倍率

// --- 自動モード(aキーでON/OFF。既定はOFF) ---
// ONの間、腹ペコ/病気の魚がいて漂っている餌・薬が少ない場合に、一定間隔で
// 自動的に餌やり・投薬を行う(f/m相当)。投下位置はカーソルにこだわらずランダム。
// 頻繁に投下しすぎないよう、それぞれ数十秒に1回程度のクールダウンを設ける。
pub const AUTO_FEED_COOLDOWN: f64 = 30.0;
pub const AUTO_FEED_FLOAT_THRESHOLD: usize = 3; // 漂っている(未着地の)餌がこれ未満なら投下対象
pub const AUTO_MEDICATE_COOLDOWN: f64 = 30.0;
pub const AUTO_MEDICATE_FLOAT_THRESHOLD: usize = 3;
// 自動ガラス叩き: ランダムな位置・タイミングで時々発生させる(頻度は低め・数分に1回程度)。
// 既存の「叩きすぎペナルティ」判定にも通常のtキーと同じくカウントされるが、
// 低頻度なので基本的にペナルティには引っかからない想定。
pub const AUTO_KNOCK_COOLDOWN: f64 = 180.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Food {
    pub x: f64,
    pub y: f64,
    pub vy: f64,
    pub life: f64,
    // 水底に着地したかどうか。着地後は寿命が減らず水底に停留する
    // (カニが片付けるか、水底の総数上限を超えた古いものから消える)。
    #[serde(default)]
    pub landed: bool,
}

// 薬(病気治療用の粒。餌とは別色)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Medicine {
    pub x: f64,
    pub y: f64,
    pub vy: f64,
    pub life: f64,
    // 水底に着地したかどうか(Food と同様、着地後は寿命が減らず停留する)
    #[serde(default)]
    pub landed: bool,
}

// 卵(水底付近に産まれ、一定時間で孵化して稚魚になる)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Egg {
    pub x: f64,
    pub y: f64,
    pub species: Species,
    pub hatch: f64, // 孵化までの残り時間
}

// 投下エフェクトの種類(餌/薬で色を変え、何を投げたか一目でわかるようにする)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum EffectKind {
    Food,
    Medicine,
    Knock, // ガラスを叩いた振動
    Blood, // サメの捕食時の血飛沫(死亡演出=仰向け浮上とは別の専用演出)
}

// f/m を押した瞬間、投下位置に一瞬だけ出る光/波紋の演出。常時表示のカーソルとは別物。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DropEffect {
    pub x: f64,
    pub y: f64,
    pub life: f64,     // 残り時間(0以下で消える)
    pub max_life: f64, // 生成時のlife(進行度=1.0-life/max_lifeの計算に使う。血飛沫は個体ごとに寿命がばらつくため必要)
    pub kind: EffectKind,
}

// 血の滲み(範囲エフェクト): 捕食位置の周辺に赤みが水中に広がって、時間とともに
// ゆっくりフェードアウトする(DropEffectの粒子より長く残る、背景寄りの演出)。
#[derive(Clone, Debug)]
pub struct BloodStain {
    pub x: f64,
    pub y: f64,
    pub life: f64,
    pub max_life: f64,
}

// 効果音(SE)の発火イベント。sim.rs は音の再生方法を知らず、main.rs 側の
// SoundEngine がこれを受け取って正弦波ビープを鳴らす(sim.rs は rodio に依存しない)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SfxEvent {
    Bubble,      // 気泡が上る音(頻度は控えめ)
    Feed,        // 餌を入れた音
    Medicate,    // 薬を入れた音
    SickOnset,   // 病気になった音
    Cured,       // 治療で回復した音
    HungryOnset, // 空腹(腹ぺこ)になった瞬間の音
    GlassKnock,  // ガラスを叩いた(こんこん)音
    Predation,   // サメが獲物を捕食した音
}

#[derive(Clone, Debug)]
pub struct Bubble {
    pub x: f64,
    pub y: f64,
    pub vy: f64,
}

// 観賞用のカニ。水底(砂の上)を左右に歩くだけで泳がない。育成ロジック対象外。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Crab {
    pub x: f64,
    pub dir: f64, // 歩く向き: +1.0=右, -1.0=左
    pub pause_timer: f64,
    pub facing_right: bool,
}

pub struct Simulation {
    pub fish: Vec<Fish>,
    pub food: Vec<Food>,
    pub medicine: Vec<Medicine>,
    pub eggs: Vec<Egg>,
    pub bubbles: Vec<Bubble>,
    pub crabs: Vec<Crab>,
    // 投下エフェクト(一瞬で消えるので保存対象にしない)
    pub drop_effects: Vec<DropEffect>,
    // 血の滲み(範囲エフェクト。数秒で消えるので保存対象にしない)
    pub blood_stains: Vec<BloodStain>,
    // このtickで発火した効果音イベント。main.rs 側が毎フレーム drain して再生する
    // (保存対象にしない。sim.rs は音の再生方法を知らない)。
    pub sound_events: Vec<SfxEvent>,
    pub rng: Rng,
    pub elapsed: f64,            // 累計経過秒
    pub message: Option<String>, // ステータスバー用の一言
    message_ttl: f64,
    bubble_timer: f64,
    bubble_sound_timer: f64, // 気泡音専用の間引きタイマー(見た目の気泡発生より控えめな頻度)
    knock_times: Vec<f64>, // 直近の「ガラスを叩く」タイムスタンプ(叩きすぎ判定用。保存対象外)
    // 自動モード(aキー)用のクールダウンタイマー。UI側のON/OFFはmain.rs(Ctl)が持ち、
    // ONの間だけ update_auto_care() を呼ぶ想定(保存対象外)。
    auto_feed_timer: f64,
    auto_medicate_timer: f64,
    auto_knock_timer: f64,
}

// 水底(砂)の高さ(論理ピクセル)
pub fn sand_height(pix_h: usize) -> usize {
    (pix_h / 12).max(2)
}

// 端末サイズに応じた個体数上限。大きい端末では最大100匹程度まで許容する。
// 実機フィードバック(「標準的な端末サイズだと+キーの上限50匹や最大100匹に届きにくい」)
// を受けて、除数を1400→700に下げて収容密度を上げた。
pub fn capacity(pix_w: usize, pix_h: usize) -> usize {
    ((pix_w * pix_h) / 700).clamp(5, 100)
}

// `+`キー(デバッグ追加)の上限。これ以上は産卵→孵化を経由してのみ個体数上限まで増やせる。
pub const ADD_FISH_MANUAL_CAP: usize = 50;

// `x.clamp(1.0, upper)` の upper が 1.0 未満(NaN含む)だと `min > max` で panic するため、
// upper を必ず 1.0 以上に補正してから渡すための安全弁。
// 端末が極端に小さく pix_w/pix_h が小さい場合の防御(main.rs 側の最小サイズ保証と二重で守る)。
fn safe_upper(v: f64) -> f64 {
    if v.is_finite() {
        v.max(1.0)
    } else {
        1.0
    }
}

// カーソル等、水槽内の任意の点を安全に範囲内へクランプする(main.rs から利用)。
pub fn clamp_point(x: f64, y: f64, pix_w: usize, pix_h: usize) -> (f64, f64) {
    let sand_top = (pix_h as f64 - sand_height(pix_h) as f64).max(2.0);
    (
        x.clamp(1.0, safe_upper(pix_w as f64 - 1.0)),
        y.clamp(1.0, safe_upper(sand_top - 1.0)),
    )
}

// 水底に着地した要素(is_landed)の総数が cap を超えたら、古いものから間引く。
// Vec は push で末尾に追加される前提なので、先頭側にあるものほど古い。
fn trim_landed<T>(items: &mut Vec<T>, is_landed: impl Fn(&T) -> bool, cap: usize) {
    let landed_count = items.iter().filter(|it| is_landed(it)).count();
    if landed_count > cap {
        let mut to_remove = landed_count - cap;
        items.retain(|it| {
            if to_remove > 0 && is_landed(it) {
                to_remove -= 1;
                false
            } else {
                true
            }
        });
    }
}

impl Simulation {
    pub fn new(rng: Rng) -> Self {
        Simulation {
            fish: Vec::new(),
            food: Vec::new(),
            medicine: Vec::new(),
            eggs: Vec::new(),
            bubbles: Vec::new(),
            crabs: Vec::new(),
            drop_effects: Vec::new(),
            blood_stains: Vec::new(),
            sound_events: Vec::new(),
            rng,
            elapsed: 0.0,
            message: None,
            message_ttl: 0.0,
            bubble_timer: 0.0,
            bubble_sound_timer: 0.0,
            knock_times: Vec::new(),
            auto_feed_timer: 0.0,
            auto_medicate_timer: 0.0,
            auto_knock_timer: 0.0,
        }
    }

    // 初期個体を撒く(セーブが無い初回起動 / リセット用)
    pub fn seed_initial(&mut self, pix_w: usize, pix_h: usize) {
        let n = 5.min(capacity(pix_w, pix_h));
        for i in 0..n {
            // 初期配置はサメを含めない通常3種のみ(サメの入手経路はSキーのみに限定する方針)
            let sp = Species::COMMON[i % Species::COMMON.len()];
            let stage = if i % 2 == 0 { Stage::Adult } else { Stage::Fry };
            let x = self.rng.range(6.0, (pix_w as f64 - 6.0).max(6.0));
            let y = self
                .rng
                .range(4.0, (pix_h as f64 - sand_height(pix_h) as f64 - 2.0).max(4.0));
            self.fish.push(Fish::new(sp, stage, x, y));
        }
        self.ensure_decorative_entities(pix_w, pix_h);
    }

    // 観賞用エンティティ(大型魚・カニ)が空なら初期数を補充する。
    // 初回起動時に加え、それらのフィールドを持たない旧セーブを読み込んだ直後にも呼ぶ。
    pub fn ensure_decorative_entities(&mut self, pix_w: usize, _pix_h: usize) {
        if self.crabs.is_empty() {
            for _ in 0..CRAB_COUNT {
                let x = self.rng.range(4.0, (pix_w as f64 - 4.0).max(4.0));
                let dir = if self.rng.next_f64() < 0.5 { 1.0 } else { -1.0 };
                self.crabs.push(Crab {
                    x,
                    dir,
                    pause_timer: 0.0,
                    facing_right: dir > 0.0,
                });
            }
        }
    }

    // グレートリセット: 魚を初期構成へ、卵・餌・薬・経過時間を消去
    pub fn reset(&mut self, pix_w: usize, pix_h: usize) {
        self.fish.clear();
        self.food.clear();
        self.medicine.clear();
        self.eggs.clear();
        self.bubbles.clear();
        self.drop_effects.clear();
        self.blood_stains.clear();
        self.elapsed = 0.0;
        self.seed_initial(pix_w, pix_h);
        self.set_message("水槽をリセットしました");
    }

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
        self.message_ttl = 4.0;
    }

    // 餌やり: カーソルのX座標付近から3〜5粒を投下(Yは水面付近から沈み始める)。
    // 投下位置には一瞬だけ光/波紋の演出(DropEffect)を出し、何をどこに投げたか分かりやすくする。
    pub fn feed(&mut self, cursor_x: f64, pix_w: usize) {
        let count = self.rng.range_usize(3, 5);
        let cx = cursor_x.clamp(1.0, safe_upper(pix_w as f64 - 1.0));
        for _ in 0..count {
            self.food.push(Food {
                x: (cursor_x + self.rng.range(-6.0, 6.0)).clamp(1.0, safe_upper(pix_w as f64 - 1.0)),
                y: self.rng.range(1.0, 4.0),
                vy: FOOD_SINK_SPEED * self.rng.range(0.8, 1.2),
                life: FOOD_LIFETIME,
                landed: false,
            });
        }
        self.drop_effects.push(DropEffect {
            x: cx,
            y: 2.5,
            life: DROP_EFFECT_LIFETIME,
            max_life: DROP_EFFECT_LIFETIME,
            kind: EffectKind::Food,
        });
        self.sound_events.push(SfxEvent::Feed);
    }

    // 投薬: カーソルのX座標付近から数粒の薬を投下(Yは水面付近から沈み始める)。
    // 投下位置には一瞬だけ光/波紋の演出(DropEffect)を出し、何をどこに投げたか分かりやすくする。
    pub fn medicate(&mut self, cursor_x: f64, pix_w: usize) {
        let count = self.rng.range_usize(3, 5);
        let cx = cursor_x.clamp(1.0, safe_upper(pix_w as f64 - 1.0));
        for _ in 0..count {
            self.medicine.push(Medicine {
                x: (cursor_x + self.rng.range(-6.0, 6.0)).clamp(1.0, safe_upper(pix_w as f64 - 1.0)),
                y: self.rng.range(1.0, 4.0),
                vy: MED_SINK_SPEED * self.rng.range(0.8, 1.2),
                life: MED_LIFETIME,
                landed: false,
            });
        }
        self.drop_effects.push(DropEffect {
            x: cx,
            y: 2.5,
            life: DROP_EFFECT_LIFETIME,
            max_life: DROP_EFFECT_LIFETIME,
            kind: EffectKind::Medicine,
        });
        self.sound_events.push(SfxEvent::Medicate);
    }

    // ガラスを叩く(こんこん): カーソル位置に振動/波紋を一瞬出し、
    // 近くの(死んでいない)魚を短時間だけ驚いて逃げさせる。
    pub fn knock(&mut self, cursor_x: f64, cursor_y: f64, pix_w: usize, pix_h: usize) {
        let (cx, cy) = clamp_point(cursor_x, cursor_y, pix_w, pix_h);
        self.drop_effects.push(DropEffect {
            x: cx,
            y: cy,
            life: DROP_EFFECT_LIFETIME,
            max_life: DROP_EFFECT_LIFETIME,
            kind: EffectKind::Knock,
        });
        self.sound_events.push(SfxEvent::GlassKnock);

        for f in &mut self.fish {
            if f.dead {
                continue; // 死亡演出中の魚は驚かない
            }
            let dx = f.x - cx;
            let dy = f.y - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < KNOCK_RADIUS {
                let d = dist.max(0.001);
                f.flee_dx = dx / d;
                f.flee_dy = dy / d;
                if f.flee_timer <= 0.0 {
                    // 逃走コスト: 逃走開始の瞬間(既に逃走中でなければ)に空腹度を消費する
                    f.hunger = (f.hunger - FLEE_HUNGER_COST).max(0.0);
                }
                f.flee_timer = FLEE_DURATION;
            }
        }

        // 叩きすぎ判定: 直近 KNOCK_SPAM_WINDOW 秒以内の叩き回数が閾値を超えたら、
        // 周辺の魚にストレス(病気発症ボーナス)を一定時間与える。
        // 稀にしか叩かない通常利用では閾値に届かないため影響が出ない。
        self.knock_times.push(self.elapsed);
        let cutoff = self.elapsed - KNOCK_SPAM_WINDOW;
        self.knock_times.retain(|&t| t >= cutoff);
        if self.knock_times.len() >= KNOCK_SPAM_THRESHOLD {
            for f in &mut self.fish {
                if f.dead {
                    continue;
                }
                let dx = f.x - cx;
                let dy = f.y - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < KNOCK_STRESS_RADIUS {
                    f.stress_timer = KNOCK_STRESS_DURATION;
                }
            }
        }
    }

    // 自動モード(aキー): ON中、呼び出し側(main.rs)が毎tickこれを呼ぶ想定。
    // 腹ペコ/病気の魚がいて漂っている餌・薬が少ない場合に、一定間隔で自動的に
    // 餌やり・投薬(f/m相当)を行う。加えて、低頻度でランダムにガラスを叩く(t相当)。
    // 投下位置・叩く位置はカーソルにこだわらずランダムでよい(自動なので狙わなくてよい)。
    pub fn update_auto_care(&mut self, dt: f64, pix_w: usize, pix_h: usize) {
        self.auto_feed_timer = (self.auto_feed_timer - dt).max(0.0);
        self.auto_medicate_timer = (self.auto_medicate_timer - dt).max(0.0);
        self.auto_knock_timer = (self.auto_knock_timer - dt).max(0.0);

        if self.auto_feed_timer <= 0.0 {
            let hungry_exists = self
                .fish
                .iter()
                .any(|f| !f.dead && f.hunger < HUNGRY_THRESHOLD);
            let floating_food = self.food.iter().filter(|fd| !fd.landed).count();
            if hungry_exists && floating_food < AUTO_FEED_FLOAT_THRESHOLD {
                let x = self.rng.range(4.0, safe_upper(pix_w as f64 - 4.0));
                self.feed(x, pix_w);
                self.auto_feed_timer = AUTO_FEED_COOLDOWN;
            }
        }

        if self.auto_medicate_timer <= 0.0 {
            let sick_exists = self.fish.iter().any(|f| !f.dead && f.sick);
            let floating_med = self.medicine.iter().filter(|md| !md.landed).count();
            if sick_exists && floating_med < AUTO_MEDICATE_FLOAT_THRESHOLD {
                let x = self.rng.range(4.0, safe_upper(pix_w as f64 - 4.0));
                self.medicate(x, pix_w);
                self.auto_medicate_timer = AUTO_MEDICATE_COOLDOWN;
            }
        }

        if self.auto_knock_timer <= 0.0 {
            let x = self.rng.range(4.0, safe_upper(pix_w as f64 - 4.0));
            let y = self.rng.range(4.0, safe_upper(pix_h as f64 - 4.0));
            self.knock(x, y, pix_w, pix_h);
            self.auto_knock_timer = AUTO_KNOCK_COOLDOWN;
        }
    }

    // デバッグ: 魚を1匹追加。ADD_FISH_MANUAL_CAP(50匹)まで。
    // それ以上は産卵→孵化を経由してのみ個体数上限(端末サイズ依存・最大100)まで増やせる。
    // 死んで浮いている個体は数に入れない(居座りで詰まらせない)。
    // ランダム選択はサメを含まない通常3種のみ(サメの入手経路はSキーのみに限定する方針)。
    pub fn add_fish(&mut self, pix_w: usize, pix_h: usize) {
        let sp = Species::COMMON[self.rng.range_usize(0, Species::COMMON.len() - 1)];
        self.add_fish_of_species(sp, pix_w, pix_h);
    }

    // `S`キー: 種類を指定して確実にその種を1匹追加する(サメを狙って投入したい、という要望への対応)。
    // 上限(ADD_FISH_MANUAL_CAP・個体数上限)の扱いは add_fish と同じ。
    pub fn add_shark(&mut self, pix_w: usize, pix_h: usize) {
        self.add_fish_of_species(Species::Shark, pix_w, pix_h);
    }

    fn add_fish_of_species(&mut self, sp: Species, pix_w: usize, pix_h: usize) {
        if self.fish.len() >= ADD_FISH_MANUAL_CAP {
            self.set_message("これ以上は孵化でしか増えません");
            return;
        }
        if self.living_count() >= capacity(pix_w, pix_h) {
            self.set_message("水槽が満員です");
            return;
        }
        let x = self.rng.range(6.0, (pix_w as f64 - 6.0).max(6.0));
        let y = self
            .rng
            .range(4.0, (pix_h as f64 - sand_height(pix_h) as f64 - 2.0).max(4.0));
        self.fish.push(Fish::new(sp, Stage::Fry, x, y));
    }

    // 間引き: 通常の魚(サメ含む)がいればそれを1匹減らす。通常の魚が0匹になったら
    // カニを1匹減らす(観賞用生物だけ残って「0にしたのに何か残ってる」と
    // 分かりづらくならないようにするフォールバック)。追加(add_fish)は通常魚のみのまま。
    pub fn remove_fish(&mut self) {
        if !self.fish.is_empty() {
            self.fish.pop();
        } else {
            self.crabs.pop();
        }
    }

    pub fn fish_count(&self) -> usize {
        self.fish.len()
    }

    // 死亡演出中(仰向けで浮いている)ではない、育成対象として生きている魚の数。
    // 個体数上限のゲート判定に使い、死んで居座っている魚が繁殖を妨げないようにする。
    pub fn living_count(&self) -> usize {
        self.fish.iter().filter(|f| !f.dead).count()
    }

    pub fn food_count(&self) -> usize {
        self.food.len()
    }

    pub fn sick_count(&self) -> usize {
        self.fish.iter().filter(|f| f.sick).count()
    }

    // 1tick分の更新。dt=経過秒(速度倍率適用済み), (pix_w,pix_h)=論理ピクセル寸法。
    // dt=0(一時停止)なら時間経過ロジックは進まない。
    pub fn update(&mut self, dt: f64, pix_w: usize, pix_h: usize) {
        if dt <= 0.0 {
            return;
        }
        self.elapsed += dt;
        if self.message_ttl > 0.0 {
            self.message_ttl -= dt;
            if self.message_ttl <= 0.0 {
                self.message = None;
            }
        }
        let cap = capacity(pix_w, pix_h);
        // sand_height は pix_h に対して最大2までしか保証しないため、pix_h が極端に小さいと
        // sand_top が 0 以下になり得る。水面〜水底の描画領域として最低2px は確保する。
        let sand_top = (pix_h as f64 - sand_height(pix_h) as f64).max(2.0);

        self.update_movement(dt, pix_w as f64, sand_top);
        self.update_food(dt, sand_top);
        self.update_medicine(dt, sand_top);
        self.update_biology(dt, cap, pix_w as f64, sand_top);
        self.update_predation(dt);
        self.update_crabs(dt, pix_w as f64);
        self.update_bubbles(dt, pix_w as f64, pix_h as f64);
        self.update_effects(dt);
    }

    // 投下エフェクト(餌/薬を投げた瞬間の光/波紋)の残り時間を減らし、消えたものを取り除く
    fn update_effects(&mut self, dt: f64) {
        for e in &mut self.drop_effects {
            e.life -= dt;
        }
        self.drop_effects.retain(|e| e.life > 0.0);
        for s in &mut self.blood_stains {
            s.life -= dt;
        }
        self.blood_stains.retain(|s| s.life > 0.0);
    }

    // 遊泳: ランダムウォーク+慣性+壁反射+群れ+餌吸引(空腹度・病気で速度が変化)。
    // 死亡演出中の個体はここでは動かさず、水面近くまでゆっくり浮上して静止するだけにする。
    fn update_movement(&mut self, dt: f64, w: f64, sand_top: f64) {
        // 群れ計算のため位置・速度をスナップショット(self.fish とインデックスを揃えるため
        // 死亡個体もそのまま含め、死亡フラグで群れ対象から除外する)
        // hunger/predation_cooldown も持たせて、他の魚が「近くのサメが今まさに
        // 捕食モードかどうか」を判定できるようにする(逃走ベクトルの判定に使う)。
        let snap: Vec<(Species, f64, f64, f64, f64, bool, f64, f64)> = self
            .fish
            .iter()
            .map(|f| (f.species, f.x, f.y, f.vx, f.vy, f.dead, f.hunger, f.predation_cooldown))
            .collect();

        let margin = 4.0;
        let top_margin = 3.0;
        let wall_push = 70.0;

        for i in 0..self.fish.len() {
            if self.fish[i].dead {
                // 死んだ魚: 横移動はせず、水面近くまでゆっくり浮上して静止する
                let f = &mut self.fish[i];
                f.vx = 0.0;
                f.vy = if f.y > DEAD_SURFACE_MARGIN {
                    -DEAD_FLOAT_SPEED
                } else {
                    0.0
                };
                f.y += f.vy * dt;
                f.x = f.x.clamp(1.0, safe_upper(w - 1.0));
                f.y = f.y.clamp(1.0, safe_upper(sand_top - 1.0));
                continue;
            }
            let (
                sp,
                hunger,
                fx,
                fy,
                spd_mult,
                hungry,
                flee_timer,
                flee_dx,
                flee_dy,
                predation_cooldown,
                dash_timer,
                dash_dx,
                dash_dy,
            ) = {
                let f = &self.fish[i];
                (
                    f.species,
                    f.hunger,
                    f.x,
                    f.y,
                    f.speed_mult(),
                    f.hunger < HUNGRY_THRESHOLD,
                    f.flee_timer,
                    f.flee_dx,
                    f.flee_dy,
                    f.predation_cooldown,
                    f.dash_timer,
                    f.dash_dx,
                    f.dash_dy,
                )
            };
            let mut ax = 0.0;
            let mut ay = 0.0;

            // 腹ぺこなら最寄りの餌を先に探しておく。餌を追っている間は通常の遊泳
            // (ランダムウォーク・群れ)を大きく弱め、吸引ベクトルの方向へはっきり
            // 優先して進ませる(近くに餌があれば一直線に向かうくらいの強さにする)。
            let nearest_food = if hunger < HUNGRY_THRESHOLD && !self.food.is_empty() {
                let mut best = f64::INFINITY;
                let mut best_pos = None;
                for fd in &self.food {
                    let d = (fd.x - fx).powi(2) + (fd.y - fy).powi(2);
                    if d < best {
                        best = d;
                        best_pos = Some((fd.x, fd.y));
                    }
                }
                best_pos.map(|pos| (pos, best.sqrt().max(0.001)))
            } else {
                None
            };

            // サメの狩り: 空腹度が閾値未満・クールダウン明けなら、近くの獲物(サメ以外・
            // 生存個体)を先に探しておく。追いかけている間は通常の遊泳を弱め、
            // 吸引ベクトルをはっきり優先させる(「追いかけている」のが見た目でわかるように)。
            let chase_target = if sp.is_predator() && hunger < SHARK_HUNT_HUNGER_THRESHOLD && predation_cooldown <= 0.0 {
                let mut best = f64::INFINITY;
                let mut best_pos = None;
                for &(psp, px, py, _pvx, _pvy, pdead, ..) in snap.iter() {
                    if psp == Species::Shark || pdead {
                        continue;
                    }
                    let d = (px - fx).powi(2) + (py - fy).powi(2);
                    if d < best {
                        best = d;
                        best_pos = Some((px, py));
                    }
                }
                best_pos
                    .map(|pos| (pos, best.sqrt().max(0.001)))
                    .filter(|&(_, dist)| dist < SHARK_HUNT_RADIUS)
            } else {
                None
            };
            // 追跡中かどうか(捕食モードで獲物を追っている間だけ、後段で最高速度を
            // 通常3種よりはっきり速くブーストする)
            let is_chasing = chase_target.is_some();

            // 被食者側の警戒: 近くにサメがいたら常に検知する(方針変更: 「みんなサメが
            // 嫌い」という設定にするため、サメが捕食モードかどうかは問わない)。
            let fear_target = if !sp.is_predator() {
                let mut best = f64::INFINITY;
                let mut best_pos = None;
                for &(psp, px, py, _pvx, _pvy, pdead, _phunger, _pcooldown) in snap.iter() {
                    if psp != Species::Shark || pdead {
                        continue;
                    }
                    let d = (px - fx).powi(2) + (py - fy).powi(2);
                    if d < best {
                        best = d;
                        best_pos = Some((px, py));
                    }
                }
                best_pos
                    .map(|pos| (pos, best.sqrt()))
                    .filter(|&(_, dist)| dist < SHARK_FEAR_RADIUS)
            } else {
                None
            };
            let is_fleeing_shark = fear_target.is_some();

            // 餌を追っている・獲物を追っている・サメから逃げている、のいずれかの間は
            // 通常の遊泳(ランダムウォーク・群れ)を大きく弱め、該当のベクトルを
            // はっきり優先させる(「一直線に向かう/逃げる」のが見た目でわかるように)。
            let normal_move_mix = if nearest_food.is_some() || chase_target.is_some() || is_fleeing_shark {
                HUNGRY_NORMAL_MOVE_DAMP
            } else {
                1.0
            };

            // ランダムウォーク(縦は控えめ)。空腹度・病気に応じて活発さが変わる
            ax += self.rng.signed() * sp.wander() * spd_mult * normal_move_mix;
            ay += self.rng.signed() * sp.wander() * 0.55 * spd_mult * normal_move_mix;

            // 群れ: 同種近傍の平均速度に少し寄せる(死亡個体は対象外)
            let (mut svx, mut svy, mut cnt) = (0.0, 0.0, 0);
            for (j, &(osp, ox, oy, ovx, ovy, odead, _ohunger, _ocooldown)) in snap.iter().enumerate() {
                if j == i || osp != sp || odead {
                    continue;
                }
                let d = ((ox - fx).powi(2) + (oy - fy).powi(2)).sqrt();
                if d < NEIGHBOR_RADIUS && d > 0.001 {
                    svx += ovx;
                    svy += ovy;
                    cnt += 1;
                }
            }
            if cnt > 0 {
                ax += (svx / cnt as f64) * 0.8 * normal_move_mix;
                ay += (svy / cnt as f64) * 0.8 * normal_move_mix;
            }

            // 餌吸引: 腹ぺこほど強く最寄りの餌へ向かう(通常の遊泳ベクトルより
            // はっきり優先されるよう HUNGRY_FOOD_PULL_BOOST で大きく増幅する)
            if let Some(((bx, by), dist)) = nearest_food {
                let pull = sp.food_pull()
                    * (1.0 - hunger / HUNGRY_THRESHOLD)
                    * spd_mult
                    * HUNGRY_FOOD_PULL_BOOST;
                ax += (bx - fx) / dist * pull;
                ay += (by - fy) / dist * pull;
            }

            // サメの狩り: 探しておいた獲物へ向かって強く近づく(実際の捕食判定は
            // update_predation 側で行う)。通常の遊泳は上で damp 済みなので、
            // この吸引ベクトルが「追いかけている」動きとしてはっきり見えるようにする。
            if let Some(((bx, by), dist)) = chase_target {
                ax += (bx - fx) / dist * SHARK_HUNT_PULL;
                ay += (by - fy) / dist * SHARK_HUNT_PULL;
            }

            // 被食者側の逃走: 検知済みのサメから離れる方向へ強く加速する。通常の遊泳は
            // 上で damp 済み・最高速度も後段でブーストするため、「パッと反応して素早く
            // 逃げる」機敏な動きになる(ぬるっと逃げる感じにはならない)。真っ直ぐ離れる
            // だけでなく、垂直成分を振動させてジグザグに回り込むような動きを混ぜる。
            if let Some(((sx, sy), raw_dist)) = fear_target {
                let dist = raw_dist.max(0.001);
                // 距離が近いほど強く効かせる
                let strength = SHARK_FEAR_STRENGTH * (1.0 - raw_dist / SHARK_FEAR_RADIUS).max(0.0);
                let away_x = (fx - sx) / dist;
                let away_y = (fy - sy) / dist;
                ax += away_x * strength;
                ay += away_y * strength;
                // 回り込み: 逃走方向に対して垂直な成分を時間で振動させる
                let phase = self.elapsed * ZIGZAG_FREQ + i as f64 * 1.7;
                let wobble = phase.sin() * strength * ZIGZAG_RATIO;
                ax += -away_y * wobble;
                ay += away_x * wobble;
            }

            // 壁の手前で緩やかに向きを変える(反射)
            if fx < margin {
                ax += wall_push;
            } else if fx > w - margin {
                ax -= wall_push;
            }
            if fy < top_margin {
                ay += wall_push;
            } else if fy > sand_top - 1.0 {
                ay -= wall_push;
            }

            // ガラスを叩かれて驚いている間は、逃走方向へ強い加速を追加する。
            // ここにも回り込み(垂直方向の切り返し)成分を混ぜて、真っ直ぐ離れる
            // だけの単調な動きにならないようにする。
            let is_knock_fleeing = flee_timer > 0.0;
            if is_knock_fleeing {
                ax += flee_dx * FLEE_STRENGTH;
                ay += flee_dy * FLEE_STRENGTH;
                let phase = self.elapsed * ZIGZAG_FREQ + i as f64 * 1.7 + 0.8;
                let wobble = phase.sin() * FLEE_STRENGTH * ZIGZAG_RATIO;
                ax += -flee_dy * wobble;
                ay += flee_dx * wobble;
            }
            // ガラスの驚き逃げ・サメからの逃走のいずれかの間は、最高速度を一時的に
            // 上げて「パッと反応して素早く逃げる」機敏な動きにする(鈍くしない)。
            let is_fleeing = is_knock_fleeing || is_fleeing_shark;

            // ランダムな瞬発ダッシュ: サメ・餌などのトリガーが無い「通常時」だけ、
            // 低頻度・ランダムなタイミングで一瞬だけ速く動く演出を入れる(躍動感)。
            // 既に他の強い意図(餌を追う・追跡する・逃げる)がある間は割り込まない。
            let normal_state =
                nearest_food.is_none() && chase_target.is_none() && !is_fleeing_shark && !is_knock_fleeing;
            let mut new_dash_timer = (dash_timer - dt).max(0.0);
            let mut new_dash_dx = dash_dx;
            let mut new_dash_dy = dash_dy;
            if normal_state {
                if new_dash_timer <= 0.0 && self.rng.next_f64() < DASH_CHANCE_PER_SEC * dt {
                    let angle = self.rng.range(0.0, std::f64::consts::TAU);
                    new_dash_dx = angle.cos();
                    new_dash_dy = angle.sin();
                    new_dash_timer = DASH_DURATION;
                }
                if new_dash_timer > 0.0 {
                    ax += new_dash_dx * DASH_STRENGTH;
                    ay += new_dash_dy * DASH_STRENGTH * 0.6; // 縦方向は控えめ(既存のwanderと同じ考え方)
                }
            } else {
                new_dash_timer = 0.0; // 他の強い意図が入ったらダッシュは打ち切る
            }
            let is_dashing = normal_state && new_dash_timer > 0.0;

            let f = &mut self.fish[i];
            f.dash_timer = new_dash_timer;
            f.dash_dx = new_dash_dx;
            f.dash_dy = new_dash_dy;
            f.vx += ax * dt;
            f.vy += ay * dt;
            // 慣性(ドラッグ)。逃走中・ダッシュ中はドラッグ(ブレーキ)も弱めて反応を鈍らせない。
            let drag_rate = if is_fleeing || is_dashing { 0.5 } else { 0.9 };
            let drag = (1.0 - drag_rate * dt).clamp(0.0, 1.0);
            f.vx *= drag;
            f.vy *= drag;
            // 最高速度でクランプ(空腹度・病気で上限が変わる。逃走中は一時的に速く泳げる。
            // サメは追跡中だけ通常3種よりはっきり速くなるようブーストする。大きく育つほど
            // わずかに遅くなる(size_speed_mult、必須ではない体感の変化)。ランダムダッシュ中も
            // 一瞬だけ最高速度が上がる。
            let speed = (f.vx * f.vx + f.vy * f.vy).sqrt();
            let maxs = sp.max_speed()
                * spd_mult
                * f.size_speed_mult()
                * if is_fleeing { 1.6 } else { 1.0 }
                * if is_chasing { SHARK_CHASE_SPEED_MULT } else { 1.0 }
                * if is_dashing { DASH_SPEED_MULT } else { 1.0 };
            if speed > maxs {
                f.vx = f.vx / speed * maxs;
                f.vy = f.vy / speed * maxs;
            }
            // 逃走タイマーを進める。サメに驚いている間は、危険が続く限り逃走状態を
            // 維持する(逃走開始の瞬間だけ空腹度コストを課金し、居座られても再課金しない)。
            if is_fleeing_shark {
                if f.flee_timer <= 0.0 {
                    f.hunger = (f.hunger - FLEE_HUNGER_COST).max(0.0);
                }
                f.flee_timer = f.flee_timer.max(SHARK_FEAR_FLEE_MARK);
            } else {
                f.flee_timer = (f.flee_timer - dt).max(0.0);
            }
            // 積分
            f.x += f.vx * dt;
            f.y += f.vy * dt;
            // 位置クランプ
            f.x = f.x.clamp(1.0, safe_upper(w - 1.0));
            f.y = f.y.clamp(1.0, safe_upper(sand_top - 1.0));
            // 進行方向で左右反転(微小速度では維持)
            if f.vx > 0.6 {
                f.facing_right = true;
            } else if f.vx < -0.6 {
                f.facing_right = false;
            }
            let _ = hungry;
        }
    }

    // 餌: 沈降・着地(水底で停留)・捕食。
    // 捕食は「魚ごと」に最も近い未消費の餌を1粒だけ探して食べる(1tickで1魚1粒までの制限。
    // 従来は「餌ごと」に魚を探すループだったため、密集した餌の近くを1匹が通ると
    // 何十粒も一度に食べてしまうバグがあった)。
    fn update_food(&mut self, dt: f64, sand_top: f64) {
        // 積もる見た目のため、既に着地済みの位置をスナップショットしておく
        // (このtick中に新たに着地したものも互いに積み上がるよう別途記録する)
        let landed_snapshot: Vec<f64> = self.food.iter().filter(|fd| fd.landed).map(|fd| fd.x).collect();
        let mut new_landings: Vec<f64> = Vec::new();

        for fd in &mut self.food {
            if fd.landed {
                continue; // 着地済みは停留(寿命は減らさない)
            }
            fd.y += fd.vy * dt;
            fd.life -= dt;
            if fd.y >= sand_top {
                // 山になる演出: 近くに着地済みのものが多いほど高く盛り上げる
                let nearby = landed_snapshot.iter().filter(|&&x| (x - fd.x).abs() < PILE_RADIUS).count()
                    + new_landings.iter().filter(|&&x| (x - fd.x).abs() < PILE_RADIUS).count();
                let rise = (nearby as f64 * PILE_STACK_STEP).min(PILE_MAX_HEIGHT);
                fd.y = sand_top - rise;
                fd.vy = 0.0;
                fd.landed = true;
                new_landings.push(fd.x);
            }
        }

        // 捕食: 死亡演出中でない魚が、1tickにつき最も近い餌を1粒だけ食べる
        let mut eaten = vec![false; self.food.len()];
        for f in &mut self.fish {
            if f.dead {
                continue;
            }
            let mut best_dist = f64::INFINITY;
            let mut best_fi = None;
            for (fi, fd) in self.food.iter().enumerate() {
                if eaten[fi] {
                    continue;
                }
                let d = ((fd.x - f.x).powi(2) + (fd.y - f.y).powi(2)).sqrt();
                if d < EAT_RADIUS && d < best_dist {
                    best_dist = d;
                    best_fi = Some(fi);
                }
            }
            if let Some(fi) = best_fi {
                f.hunger = (f.hunger + FEED_AMOUNT).min(MAX_HUNGER);
                eaten[fi] = true;
            }
        }

        let mut idx = 0;
        self.food.retain(|fd| {
            let keep = if eaten[idx] {
                false
            } else if fd.landed {
                true // 着地後は寿命では消えない(水底キャパ管理は下で行う)
            } else {
                fd.life > 0.0
            };
            idx += 1;
            keep
        });

        // 水底の停留数が上限を超えたら、古いものから間引く
        trim_landed(&mut self.food, |fd| fd.landed, SEABED_ITEM_CAP);
    }

    // 薬: 沈降・着地(水底で停留)・治癒(病気の魚が触れると治る。健康な魚には無害)。
    // 治癒も餌と同様、1tickにつき1匹の魚が消費できる薬は1つまでに制限する。
    fn update_medicine(&mut self, dt: f64, sand_top: f64) {
        // 餌と同様、積もる見た目のため着地済みの位置をスナップショットしておく
        let landed_snapshot: Vec<f64> = self.medicine.iter().filter(|md| md.landed).map(|md| md.x).collect();
        let mut new_landings: Vec<f64> = Vec::new();

        for md in &mut self.medicine {
            if md.landed {
                continue;
            }
            md.y += md.vy * dt;
            md.life -= dt;
            if md.y >= sand_top {
                let nearby = landed_snapshot.iter().filter(|&&x| (x - md.x).abs() < PILE_RADIUS).count()
                    + new_landings.iter().filter(|&&x| (x - md.x).abs() < PILE_RADIUS).count();
                let rise = (nearby as f64 * PILE_STACK_STEP).min(PILE_MAX_HEIGHT);
                md.y = sand_top - rise;
                md.vy = 0.0;
                md.landed = true;
                new_landings.push(md.x);
            }
        }

        let mut used = vec![false; self.medicine.len()];
        let mut cured = false;
        for f in &mut self.fish {
            if !f.sick || f.dead {
                continue; // 健康な魚・死亡演出中の魚は薬に反応しない
            }
            let mut best_dist = f64::INFINITY;
            let mut best_mi = None;
            for (mi, md) in self.medicine.iter().enumerate() {
                if used[mi] {
                    continue;
                }
                let d = ((md.x - f.x).powi(2) + (md.y - f.y).powi(2)).sqrt();
                if d < CURE_RADIUS && d < best_dist {
                    best_dist = d;
                    best_mi = Some(mi);
                }
            }
            if let Some(mi) = best_mi {
                f.sick = false;
                f.sick_timer = 0.0;
                used[mi] = true;
                cured = true;
            }
        }
        if cured {
            self.set_message("薬で病気が治った");
            self.sound_events.push(SfxEvent::Cured);
        }

        let mut idx = 0;
        self.medicine.retain(|md| {
            let keep = if used[idx] {
                false
            } else if md.landed {
                true
            } else {
                md.life > 0.0
            };
            idx += 1;
            keep
        });

        trim_landed(&mut self.medicine, |md| md.landed, SEABED_ITEM_CAP);
    }

    // 育成: 空腹度減少・病気の発症/進行・成長・産卵・孵化・死亡(演出付き)
    fn update_biology(&mut self, dt: f64, cap: usize, w: f64, sand_top: f64) {
        // 過密判定・孵化の上限ゲートは、死んで浮いている個体(dead)を除いた
        // 「生きている」個体数を基準にする。居座る死骸が繁殖を止めないようにするため。
        let living = self.living_count();
        let overcrowded = living as f64 >= cap as f64 * OVERCROWD_RATIO;
        let mut messages: Vec<String> = Vec::new();
        let mut deaths: Vec<String> = Vec::new();
        // 産卵イベント: (親x, 親y, 種)。借用の都合で後からまとめて卵を生成する。
        let mut spawn_eggs: Vec<(f64, f64, Species)> = Vec::new();

        for f in &mut self.fish {
            if f.dead {
                // 死亡演出中は育成ロジックの対象外。浮上している時間だけ進める。
                f.dead_timer += dt;
                continue;
            }

            // 年齢を進める(寿命・老齢判定に使う。死亡演出中でない間だけ加算する)
            f.age += dt;

            // 空腹度の減少
            f.hunger = (f.hunger - HUNGER_DECAY * dt).max(0.0);

            // 腹ぺこ継続時間(0→増加へ転じた瞬間=腹ぺこになった瞬間として効果音を鳴らす)
            let was_not_hungry = f.hungry_timer <= 0.0;
            if f.hunger < HUNGRY_THRESHOLD {
                if was_not_hungry {
                    self.sound_events.push(SfxEvent::HungryOnset);
                }
                f.hungry_timer += dt;
            } else {
                f.hungry_timer = 0.0;
            }

            // 満腹維持タイマー
            if f.hunger >= WELL_FED_THRESHOLD {
                f.well_fed_timer += dt;
                // growth_stage 判定専用のタイマー(産卵・稚魚成長でのリセットに影響されない)
                f.size_timer += dt;
            } else {
                f.well_fed_timer = (f.well_fed_timer - dt).max(0.0);
                f.size_timer = (f.size_timer - dt).max(0.0);
            }

            // 衰弱(空腹度0の継続)の進行
            if f.hunger <= 0.0 {
                f.starve_timer += dt;
            } else {
                f.starve_timer = 0.0;
            }

            // 老齢に達した瞬間、満腹状態などの条件を問わず確定で1回だけ産卵する
            // (「老いると産卵確率が上がる」ではなく、次世代を残す最後のチャンスとしての
            // 一度きりの確定イベント。サメは対象外=`S`キー以外で増えない方針のため)。
            if !f.elderly_spawned && f.age >= ELDERLY_AGE && f.species.breeds() {
                f.elderly_spawned = true;
                spawn_eggs.push((f.x, f.y, f.species));
                messages.push(format!(
                    "{}が老齢に差し掛かり、最後の卵を産んだ",
                    species_name(f.species)
                ));
            }

            // ガラスの叩きすぎ(ストレス)の残り時間を進める
            f.stress_timer = (f.stress_timer - dt).max(0.0);

            // 病気の発症: 腹ぺこ長期 or 過密で確率的に発症。ガラスを叩きすぎた直後は
            // ストレスにより発症確率が一時的に上がる。
            if !f.sick {
                let eligible = f.hungry_timer >= HUNGRY_SICK_TIME || overcrowded;
                let stress_mult = if f.stress_timer > 0.0 {
                    KNOCK_STRESS_DISEASE_MULT
                } else {
                    1.0
                };
                if eligible && self.rng.next_f64() < DISEASE_CHANCE_PER_SEC * stress_mult * dt {
                    f.sick = true;
                    f.sick_timer = 0.0;
                    messages.push(format!("{}が病気になった…[m]で薬を", species_name(f.species)));
                    self.sound_events.push(SfxEvent::SickOnset);
                }
            }
            // 病気の進行
            if f.sick {
                f.sick_timer += dt;
            }

            // 成長・産卵は病気中は停止
            if !f.sick {
                // 成長: 稚魚→成魚
                if f.stage == Stage::Fry && f.well_fed_timer >= GROW_TIME {
                    f.stage = Stage::Adult;
                    f.well_fed_timer = 0.0;
                    messages.push(format!("{}が成魚に育った", species_name(f.species)));
                }

                // 成長: 成魚になった後も満腹維持を続けるとさらに段階的に大きくなる
                // (全種共通。上限 GENERAL_MAX_GROWTH_STAGE で打ち止め)
                if f.stage == Stage::Adult
                    && f.growth_stage < GENERAL_MAX_GROWTH_STAGE
                    && f.size_timer >= SIZE_GROW_TIME
                {
                    f.growth_stage += 1;
                    f.size_timer = 0.0;
                    messages.push(format!("{}がさらに大きく育った", species_name(f.species)));
                }

                // 産卵: 成魚が満腹維持で確率的に卵を産む(空腹度は消費しない)。
                // サメは産卵しない(サメを増やす唯一の方法はSキーにする方針のため)。
                if f.species.breeds()
                    && f.stage == Stage::Adult
                    && f.well_fed_timer >= BREED_READY_TIME
                    && self.rng.next_f64() < BREED_CHANCE_PER_SEC * dt
                {
                    spawn_eggs.push((f.x, f.y, f.species));
                    // 親は満腹タイマーを消費(連続産卵しない)
                    f.well_fed_timer = 0.0;
                }
            }

            // 死亡判定: 猶予(STARVE_DEATH_TIME / SICK_DEATH_TIME)を超えたら死亡演出へ移行する。
            // 死亡演出(仰向けで浮上→静止→消滅)は update_movement / retain 側で処理する。
            // 老衰(LIFESPAN_DEATH_AGE)も同じ死亡演出に乗せる(全種共通・サメも対象)。
            if f.starve_timer >= STARVE_DEATH_TIME || (f.sick && f.sick_timer >= SICK_DEATH_TIME) {
                f.dead = true;
                f.dead_timer = 0.0;
                deaths.push(format!("{}が力尽きた…", species_name(f.species)));
            } else if f.age >= LIFESPAN_DEATH_AGE {
                f.dead = true;
                f.dead_timer = 0.0;
                deaths.push(format!("{}が老衰で力尽きた…", species_name(f.species)));
            }
        }

        // 産卵イベントを卵に変換(2〜4個、水底付近に配置)
        for (px, _py, sp) in spawn_eggs {
            let n = self.rng.range_usize(2, 4);
            for _ in 0..n {
                let ex = (px + self.rng.range(-4.0, 4.0)).clamp(1.0, safe_upper(w - 1.0));
                let ey = (sand_top - self.rng.range(0.5, 2.5)).max(1.0);
                self.eggs.push(Egg {
                    x: ex,
                    y: ey,
                    species: sp,
                    hatch: EGG_HATCH_TIME,
                });
            }
            messages.push(format!("{}が卵を産んだ", species_name(sp)));
        }

        // 孵化: 時間経過した卵を稚魚にする。上限(生きている個体数基準)超過分は孵化しない(卵は消える)。
        let mut alive = self.living_count();
        let mut newborns: Vec<Fish> = Vec::new();
        let mut hatched_msg = false;
        for e in &mut self.eggs {
            e.hatch -= dt;
        }
        self.eggs.retain(|e| {
            if e.hatch > 0.0 {
                return true; // まだ孵化しない
            }
            // 孵化タイミング
            if alive + newborns.len() < cap {
                newborns.push(Fish::new(e.species, Stage::Fry, e.x, e.y));
                hatched_msg = true;
            }
            false // 孵化 or 上限超過 → 卵は消える
        });
        alive += newborns.len();
        let _ = alive;
        self.fish.extend(newborns);
        if hatched_msg {
            messages.push("卵が孵化した".to_string());
        }

        // 死亡演出(浮上)を DEAD_FLOAT_TIME だけ維持したら水槽から消す
        self.fish.retain(|f| !(f.dead && f.dead_timer >= DEAD_FLOAT_TIME));

        // メッセージ優先度: 死亡 > 発症/成長/産卵/孵化 > 弱り
        if let Some(m) = deaths.into_iter().last() {
            self.set_message(m);
        } else if let Some(m) = messages.into_iter().last() {
            self.set_message(m);
        } else if let Some(f) = self.fish.iter().find(|f| {
            !f.dead && ((f.starve_timer >= STARVE_WEAK_TIME) || (f.sick && f.sick_timer >= SICK_WEAK_TIME))
        }) {
            if self.message.is_none() {
                self.set_message(format!("{}が弱っている…", species_name(f.species)));
            }
        }
    }

    // 観賞用の大型魚: ランダムウォーク+壁反射でゆったり泳ぐだけ。育成ロジックには参加しない。
    // サメの捕食: 空腹度が閾値未満・クールダウン明けのサメが、最も近い獲物(サメ以外・
    // 生存個体)が捕食圏内(SHARK_STRIKE_RADIUS)にいれば捕食する。頻度を抑えるため
    // 空腹度条件とクールダウンの両方を課す(四六時中は狙わせない)。
    // 1tickにつき捕食は最大1件(複数サメの同時捕食による index shift の複雑化を避けるため)。
    fn update_predation(&mut self, dt: f64) {
        // クールダウンを進める(サメ以外は常に0のまま)
        for f in &mut self.fish {
            if f.species == Species::Shark && f.predation_cooldown > 0.0 {
                f.predation_cooldown = (f.predation_cooldown - dt).max(0.0);
            }
        }

        // 位置・種・生死のスナップショット(self.fish とインデックスを揃える)
        let snapshot: Vec<(Species, f64, f64, bool)> = self
            .fish
            .iter()
            .map(|f| (f.species, f.x, f.y, f.dead))
            .collect();

        let mut prey_index: Option<usize> = None;
        let mut shark_index: Option<usize> = None;

        'outer: for (i, f) in self.fish.iter().enumerate() {
            if f.species != Species::Shark
                || f.dead
                || f.predation_cooldown > 0.0
                || f.hunger >= SHARK_HUNT_HUNGER_THRESHOLD
            {
                continue;
            }
            let mut best_dist = f64::INFINITY;
            let mut best_j = None;
            for (j, &(psp, px, py, pdead)) in snapshot.iter().enumerate() {
                if j == i || psp == Species::Shark || pdead {
                    continue;
                }
                let d = ((px - f.x).powi(2) + (py - f.y).powi(2)).sqrt();
                if d < best_dist {
                    best_dist = d;
                    best_j = Some(j);
                }
            }
            if let Some(j) = best_j {
                if best_dist < SHARK_STRIKE_RADIUS {
                    shark_index = Some(i);
                    prey_index = Some(j);
                    break 'outer;
                }
            }
        }

        if let (Some(si), Some(pi)) = (shark_index, prey_index) {
            let prey_species = self.fish[pi].species;
            let prey_x = self.fish[pi].x;
            let prey_y = self.fish[pi].y;
            // サメの空腹度を大きく回復し、クールダウンを設定(先にサメ側を更新してから
            // 獲物を除去する。除去でインデックスがずれても si には影響しないようにするため)
            self.fish[si].hunger = (self.fish[si].hunger + SHARK_PREDATION_HUNGER_GAIN).min(MAX_HUNGER);
            self.fish[si].predation_cooldown = SHARK_HUNT_COOLDOWN;
            // サメは捕食するたびに段階的に大きくなる(上限 SHARK_MAX_KILL_STAGE で打ち止め)
            if self.fish[si].kill_stage < SHARK_MAX_KILL_STAGE {
                self.fish[si].kill_stage += 1;
            }
            self.fish.remove(pi);

            // 血飛沫: 複数の粒子を捕食位置の周囲に散らし、寿命をわずかにばらつかせることで
            // 一斉に消えず尾を引くように見せる(派手・グロテスクな見た目にする実機要望対応)。
            for _ in 0..BLOOD_PARTICLE_COUNT {
                let px = prey_x + self.rng.range(-BLOOD_SPREAD_RADIUS, BLOOD_SPREAD_RADIUS);
                let py = prey_y + self.rng.range(-BLOOD_SPREAD_RADIUS * 0.6, BLOOD_SPREAD_RADIUS * 0.6);
                let particle_life = BLOOD_EFFECT_LIFETIME * self.rng.range(0.6, 1.0);
                self.drop_effects.push(DropEffect {
                    x: px,
                    y: py,
                    life: particle_life,
                    max_life: particle_life,
                    kind: EffectKind::Blood,
                });
            }
            // 血の滲み: 破片パーティクルより長く水中に残り、ゆっくりフェードアウトする
            // 範囲エフェクト(「内臓がどろろってなって血が周囲に染み渡る」イメージ)。
            self.blood_stains.push(BloodStain {
                x: prey_x,
                y: prey_y,
                life: BLOOD_STAIN_LIFETIME,
                max_life: BLOOD_STAIN_LIFETIME,
            });
            self.sound_events.push(SfxEvent::Predation);
            self.set_message(format!("{}が食べられた…", species_name(prey_species)));
        }
    }

    // 観賞用のカニ: 水底を左右に歩き、時々立ち止まる。育成ロジックには参加しない。
    fn update_crabs(&mut self, dt: f64, w: f64) {
        let margin = 3.0;
        for c in &mut self.crabs {
            if c.pause_timer > 0.0 {
                c.pause_timer -= dt;
                continue;
            }
            c.x += c.dir * CRAB_SPEED * dt;
            if c.x < margin {
                c.x = margin;
                c.dir = 1.0;
            } else if c.x > w - margin {
                c.x = (w - margin).max(margin);
                c.dir = -1.0;
            }
            c.facing_right = c.dir > 0.0;
            // 時々立ち止まる
            if self.rng.next_f64() < CRAB_PAUSE_CHANCE_PER_SEC * dt {
                c.pause_timer = self.rng.range(1.0, 3.0);
            }
        }

        // カニの掃除役: 水底に着地した餌・薬に近づくと食べて片付ける
        // (カニ自身の空腹度等のロジックは追加しない。単に消費するだけ)。
        // 山になって盛り上がっている分の高さは無視し、X距離だけで判定する
        // (積もった山の頂上まで判定距離が届かなくなるのを避けるため)。
        // 1匹のカニが1tickで消費できる餌・薬はそれぞれ1つまで(範囲内に複数あっても
        // 最も近い1つだけ食べ、残りは次のtick以降に持ち越す)。魚側の「1tickで1粒まで」
        // (fish_eats_only_one_food_per_tick_even_when_surrounded 等)と同じ考え方で、
        // 山が一括で消えてしまう過剰消費バグを防ぐ。
        let mut food_eaten = vec![false; self.food.len()];
        let mut med_eaten = vec![false; self.medicine.len()];
        for c in &self.crabs {
            let mut best_dist = f64::INFINITY;
            let mut best_fi = None;
            for (fi, fd) in self.food.iter().enumerate() {
                if food_eaten[fi] || !fd.landed {
                    continue;
                }
                let d = (fd.x - c.x).abs();
                if d < CRAB_EAT_RADIUS && d < best_dist {
                    best_dist = d;
                    best_fi = Some(fi);
                }
            }
            if let Some(fi) = best_fi {
                food_eaten[fi] = true;
            }

            let mut best_dist_m = f64::INFINITY;
            let mut best_mi = None;
            for (mi, md) in self.medicine.iter().enumerate() {
                if med_eaten[mi] || !md.landed {
                    continue;
                }
                let d = (md.x - c.x).abs();
                if d < CRAB_EAT_RADIUS && d < best_dist_m {
                    best_dist_m = d;
                    best_mi = Some(mi);
                }
            }
            if let Some(mi) = best_mi {
                med_eaten[mi] = true;
            }
        }
        let mut fi = 0;
        self.food.retain(|_| {
            let keep = !food_eaten[fi];
            fi += 1;
            keep
        });
        let mut mi = 0;
        self.medicine.retain(|_| {
            let keep = !med_eaten[mi];
            mi += 1;
            keep
        });
    }

    // 気泡: 定期発生して上へ移動
    fn update_bubbles(&mut self, dt: f64, w: f64, h: f64) {
        self.bubble_timer -= dt;
        if self.bubble_timer <= 0.0 {
            self.bubble_timer = self.rng.range(0.3, 0.9);
            self.bubbles.push(Bubble {
                x: self.rng.range(2.0, (w - 2.0).max(2.0)),
                y: h - 2.0,
                vy: -self.rng.range(6.0, 12.0),
            });
        }
        // 気泡音は見た目の気泡発生よりさらに間引く(毎回鳴らすとうるさいため)
        self.bubble_sound_timer -= dt;
        if self.bubble_sound_timer <= 0.0 {
            self.bubble_sound_timer = self.rng.range(3.0, 6.0);
            self.sound_events.push(SfxEvent::Bubble);
        }
        for b in &mut self.bubbles {
            b.y += b.vy * dt;
            b.x += (self.rng.signed() * 4.0) * dt;
        }
        self.bubbles.retain(|b| b.y > 1.0);
        if self.bubbles.len() > 60 {
            let drop = self.bubbles.len() - 60;
            self.bubbles.drain(0..drop);
        }
    }
}

pub fn species_name(sp: Species) -> &'static str {
    match sp {
        Species::Neon => "ネオン",
        Species::Goldfish => "金魚",
        Species::Guppy => "グッピー",
        Species::Shark => "サメ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // dt刻みで t秒ぶん更新する。魚を毎ステップ満腹に保つか選べる。
    fn run(sim: &mut Simulation, t: f64, dt: f64, w: usize, h: usize, keep_fed: bool) {
        let steps = (t / dt).round() as usize;
        for _ in 0..steps {
            if keep_fed {
                for f in &mut sim.fish {
                    f.hunger = MAX_HUNGER;
                }
            }
            sim.update(dt, w, h);
        }
    }

    #[test]
    fn hungry_fish_moves_strongly_toward_nearby_food() {
        // 実機フィードバック対応: 腹ペコの魚は通常の遊泳よりはっきり優先して餌へ向かうこと。
        let mut sim = Simulation::new(Rng::new(75));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 10.0, 20.0);
        f.hunger = 5.0; // 腹ぺこ
        sim.fish.push(f);
        sim.food.push(Food {
            x: 50.0,
            y: 20.0,
            vy: 0.0,
            life: 30.0,
            landed: false,
        });
        let start_x = sim.fish[0].x;
        // わずかな時間(1秒)で餌方向へ大きく前進しているはず
        run(&mut sim, 1.0, 0.05, 80, 40, false);
        let moved = sim.fish[0].x - start_x;
        assert!(
            moved > 15.0,
            "腹ぺこの魚は1秒でも餌方向へ大きく進むはず: moved={moved}"
        );
    }

    #[test]
    fn hunting_shark_moves_strongly_toward_nearby_prey() {
        // 追加要望: サメが捕食モードのとき、獲物へ「追いかけている」のがわかるくらい
        // 強く近づく動きをすること。
        let mut sim = Simulation::new(Rng::new(104));
        // サメの検知範囲(SHARK_HUNT_RADIUS)内に獲物を置く
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 10.0, 20.0);
        shark.hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(shark);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 25.0, 20.0)); // 獲物(距離15)
        let start_x = sim.fish[0].x;
        for _ in 0..24 {
            sim.fish[0].hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食条件を維持
            sim.update(0.05, 80, 40);
            if sim.fish.len() < 2 {
                break; // 追いついて捕食してしまったら十分な証拠なのでそこで終了
            }
        }
        let moved = sim.fish[0].x - start_x;
        assert!(
            moved > 10.0,
            "捕食モードのサメは短時間でも獲物方向へ大きく進むはず: moved={moved}"
        );
    }

    #[test]
    fn prey_flees_from_nearby_hunting_shark() {
        // 追加要望: 近くにいる「捕食モードのサメ」を検知したら逃げる。
        // (サメ自身も追いかけて動くため、両者が同時に動く状況でも「獲物自身の速度が
        // サメと反対方向へはっきり向く」ことを直接確認する。ネット距離はサメの追跡が
        // 一時的に勝ることがあるため、ここでは検証しない)
        let mut sim = Simulation::new(Rng::new(105));
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        shark.hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(shark);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0)); // サメの右側にいる獲物
        sim.update(0.1, 80, 40);
        assert!(
            sim.fish[1].vx > 0.0,
            "サメが左側にいるので、獲物は右(サメと反対方向)へ逃げる速度になるはず: vx={}",
            sim.fish[1].vx
        );

        // 十分な時間が経てば、逃走の最高速度ブースト(1.6倍)によりサメより速く
        // 逃げられるはずなので、最終的に距離は広がる。
        let mut sim2 = Simulation::new(Rng::new(105));
        let mut shark2 = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        shark2.hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0;
        sim2.fish.push(shark2);
        sim2.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0));
        // 回り込み(ジグザグ)の追加により、まっすぐ逃げる場合より距離が広がるまで
        // 少し時間がかかるようになったため、時間軸を延ばして「十分な時間が経てば」を確認する。
        for _ in 0..80 {
            sim2.fish[0].hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0; // サメの捕食モードを維持
            sim2.update(0.05, 80, 40);
            if sim2.fish.len() < 2 {
                break;
            }
        }
        if sim2.fish.len() == 2 {
            let final_dist = ((sim2.fish[1].x - sim2.fish[0].x).powi(2)
                + (sim2.fish[1].y - sim2.fish[0].y).powi(2))
            .sqrt();
            assert!(
                final_dist > 5.0,
                "十分な時間が経てば逃走ブーストで距離が広がるはず: final_dist={final_dist}"
            );
        }
    }

    #[test]
    fn prey_flees_even_from_well_fed_or_cooldown_shark() {
        // 方針変更(「みんなサメが嫌い」): 通常の魚は、サメが捕食モードかどうかに関わらず
        // 近くにサメがいるだけで常に逃走する。満腹中・クールダウン中のサメでも同様に逃げる。
        let mut sim = Simulation::new(Rng::new(106));

        let mut full_shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        full_shark.hunger = MAX_HUNGER; // 満腹(捕食モードではない)でも逃げる対象になる
        sim.fish.push(full_shark);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 42.0, 20.0));
        sim.update(0.1, 80, 40);
        assert!(
            sim.fish[1].vx > 0.0,
            "満腹のサメからも逃げる方向へ加速するはず: vx={}",
            sim.fish[1].vx
        );

        let mut sim2 = Simulation::new(Rng::new(107));
        let mut cooldown_shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        cooldown_shark.hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0;
        cooldown_shark.predation_cooldown = SHARK_HUNT_COOLDOWN; // クールダウン中
        sim2.fish.push(cooldown_shark);
        sim2.fish.push(Fish::new(Species::Neon, Stage::Adult, 42.0, 20.0));
        sim2.update(0.1, 80, 40);
        assert!(
            sim2.fish[1].vx > 0.0,
            "クールダウン中のサメからも逃げる方向へ加速するはず: vx={}",
            sim2.fish[1].vx
        );
    }

    #[test]
    fn chasing_shark_moves_faster_than_common_species_max_speed() {
        // 「サメは追跡中は通常魚より速い」: 捕食モードで獲物を追っている間だけ、
        // サメの最高速度が通常3種の最速種(ネオン=22.0)よりはっきり速くなる。
        let mut sim = Simulation::new(Rng::new(210));
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 10.0, 20.0);
        shark.hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(shark);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 25.0, 20.0)); // 獲物(距離15、捕食圏内)
        for _ in 0..30 {
            sim.update(0.1, 80, 40);
            if sim.fish.len() < 2 {
                break; // 捕食されて消える前提のテストではないが、念のため打ち切る
            }
        }
        // 追跡中はサメの実測速度がネオンの最高速度(22.0)を上回るはず
        let shark_speed = (sim.fish[0].vx.powi(2) + sim.fish[0].vy.powi(2)).sqrt();
        assert!(
            shark_speed > Species::Neon.max_speed(),
            "追跡中のサメはネオンの最高速度より速いはず: shark_speed={shark_speed}"
        );
    }

    #[test]
    fn patrolling_shark_does_not_get_speed_boost() {
        // 巡回中(獲物を追っていない)のサメは特別早くしない。満腹で捕食モードに
        // 入らないサメの最高速度が、通常のsp.max_speed()の範囲に収まることを確認する。
        let mut sim = Simulation::new(Rng::new(211));
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 10.0, 20.0);
        shark.hunger = MAX_HUNGER; // 満腹=捕食モードではない
        sim.fish.push(shark);
        for _ in 0..40 {
            sim.update(0.1, 80, 40);
        }
        let shark_speed = (sim.fish[0].vx.powi(2) + sim.fish[0].vy.powi(2)).sqrt();
        let normal_cap = Species::Shark.max_speed() * 1.05; // 誤差余裕
        assert!(
            shark_speed <= normal_cap,
            "巡回中のサメは通常の最高速度を超えないはず: shark_speed={shark_speed} cap={normal_cap}"
        );
    }

    #[test]
    fn random_dash_eventually_boosts_speed_during_normal_swimming() {
        // ランダムな瞬発ダッシュ: サメ・餌などのトリガーが無い通常の遊泳中でも、
        // 十分な匹数・時間があれば低頻度でダッシュ(dash_timer>0)が発生するはず。
        let mut sim = Simulation::new(Rng::new(500));
        for i in 0..30 {
            sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 10.0 + i as f64, 20.0));
        }
        let mut saw_dash = false;
        for _ in 0..400 {
            // 400 * 0.1 = 40秒(30匹 × 期待間隔約50秒/匹なら十分観測できるはず)
            sim.update(0.1, 800, 40);
            if sim.fish.iter().any(|f| f.dash_timer > 0.0) {
                saw_dash = true;
                break;
            }
        }
        assert!(
            saw_dash,
            "十分な匹数・時間があれば、いずれかの魚がランダムダッシュするはず"
        );
    }

    #[test]
    fn shark_fear_flee_adds_zigzag_perpendicular_component() {
        // 回り込み(ジグザグ)確認: サメと魚が同じ高さ(y)にいる場合、真っ直ぐ離れる
        // だけのベクトルならvyは0のままのはずだが、垂直方向の切り返し成分が入るため
        // 十分な時間が経てばvyが動くはず。
        let mut sim = Simulation::new(Rng::new(501));
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        shark.hunger = MAX_HUNGER; // 追跡はさせず、常時逃走(fear)の対象になることだけを見る
        sim.fish.push(shark);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0)); // サメと同じy
        let mut saw_vertical_motion = false;
        for _ in 0..20 {
            sim.update(0.05, 80, 40);
            if sim.fish[1].vy.abs() > 0.5 {
                saw_vertical_motion = true;
                break;
            }
        }
        assert!(
            saw_vertical_motion,
            "回り込み成分により、真っ直ぐ逃げるだけでなくvyが動くはず"
        );
    }

    #[test]
    fn fish_eats_only_one_food_per_tick_even_when_surrounded() {
        // 過剰消費バグの回帰テスト: 密集した餌の近くを1匹が通っても、1tickで食べるのは1粒まで。
        let mut sim = Simulation::new(Rng::new(76));
        let mut f = Fish::new(Species::Guppy, Stage::Adult, 40.0, 20.0);
        f.hunger = 10.0;
        sim.fish.push(f);
        for i in 0..10 {
            sim.food.push(Food {
                x: 40.0 + i as f64 * 0.1, // すべて EAT_RADIUS 圏内に密集させる
                y: 20.0,
                vy: 0.0,
                life: 30.0,
                landed: false,
            });
        }
        sim.update(0.1, 80, 40);
        assert_eq!(sim.food_count(), 9, "1tickで消費されるのは1粒だけのはず");
    }

    #[test]
    fn fish_eats_only_one_medicine_per_tick_even_when_surrounded() {
        let mut sim = Simulation::new(Rng::new(77));
        let mut f = Fish::new(Species::Guppy, Stage::Adult, 40.0, 20.0);
        f.sick = true;
        sim.fish.push(f);
        for i in 0..10 {
            sim.medicine.push(Medicine {
                x: 40.0 + i as f64 * 0.1,
                y: 20.0,
                vy: 0.0,
                life: 30.0,
                landed: false,
            });
        }
        sim.update(0.1, 80, 40);
        assert_eq!(sim.medicine.len(), 9, "1tickで消費される薬も1つだけのはず");
        assert!(!sim.fish[0].sick, "1つ消費すれば治癒するはず");
    }

    #[test]
    fn food_lands_on_seabed_and_stops_decaying() {
        let (w, h) = (80, 40);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let mut sim = Simulation::new(Rng::new(78));
        sim.food.push(Food {
            x: 40.0,
            y: sand_top - 0.5, // ほぼ水底
            vy: FOOD_SINK_SPEED,
            life: 1.0, // 寿命が短くても着地後は消えないことを確認する
            landed: false,
        });
        sim.update(0.1, w, h);
        assert!(sim.food[0].landed, "水底に着いたら landed になるはず");
        // 寿命(life=1.0)よりずっと長く待っても、着地後は寿命で消えない
        for _ in 0..50 {
            sim.update(0.1, w, h);
        }
        assert_eq!(sim.food_count(), 1, "着地後は寿命では消えず水底に停留するはず");
    }

    #[test]
    fn food_piles_up_higher_when_landing_near_existing_pile() {
        // 「積もる見た目」: 同じあたりに複数着地すると、後から着地したものほど
        // 高く盛り上がる(山になる)ことを確認する。
        let (w, h) = (80, 40);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let mut sim = Simulation::new(Rng::new(95));

        // 1個目: 何もない場所に着地(盛り上がりなし)
        sim.food.push(Food {
            x: 40.0,
            y: sand_top - 0.05,
            vy: FOOD_SINK_SPEED,
            life: 999.0,
            landed: false,
        });
        sim.update(0.1, w, h);
        assert!(sim.food[0].landed);
        let first_y = sim.food[0].y;
        assert!(
            (first_y - sand_top).abs() < 0.01,
            "最初の1個は盛り上がらず水底の高さのはず: {first_y} vs {sand_top}"
        );

        // 2個目: すぐ近く(PILE_RADIUS以内)に着地 → 1個目より高く積もるはず
        sim.food.push(Food {
            x: 40.5,
            y: sand_top - 0.05,
            vy: FOOD_SINK_SPEED,
            life: 999.0,
            landed: false,
        });
        sim.update(0.1, w, h);
        let second = sim.food.iter().find(|fd| fd.x == 40.5).unwrap();
        assert!(second.landed);
        assert!(
            second.y < first_y,
            "近くに既にある山の上に着地したものはより高い(yがより小さい)はず: {} vs {}",
            second.y,
            first_y
        );

        // 3個目: 遠く離れた場所に着地 → 山とは無関係なので盛り上がらない
        sim.food.push(Food {
            x: 5.0,
            y: sand_top - 0.05,
            vy: FOOD_SINK_SPEED,
            life: 999.0,
            landed: false,
        });
        sim.update(0.1, w, h);
        let far = sim.food.iter().find(|fd| fd.x == 5.0).unwrap();
        assert!(
            (far.y - sand_top).abs() < 0.01,
            "離れた場所の1個目は盛り上がらないはず: {} vs {}",
            far.y,
            sand_top
        );
    }

    #[test]
    fn crab_cleans_up_landed_food_and_medicine() {
        let (w, h) = (80, 40);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let mut sim = Simulation::new(Rng::new(79));
        sim.food.push(Food {
            x: 20.0,
            y: sand_top,
            vy: 0.0,
            life: 999.0,
            landed: true,
        });
        sim.medicine.push(Medicine {
            x: 20.0,
            y: sand_top,
            vy: 0.0,
            life: 999.0,
            landed: true,
        });
        sim.crabs.push(Crab {
            x: 20.0,
            dir: 1.0,
            pause_timer: 0.0,
            facing_right: true,
        });
        sim.update(0.1, w, h);
        assert_eq!(sim.food_count(), 0, "カニが水底の餌を片付けるはず");
        assert_eq!(sim.medicine.len(), 0, "カニが水底の薬も片付けるはず");
    }

    #[test]
    fn crab_eats_only_one_landed_item_per_tick_even_when_surrounded() {
        // 過剰消費バグの再発防止: 山になった食べ残しに囲まれても、1匹のカニが
        // 1tickで片付けられる餌・薬はそれぞれ1つまで(一括で山が消えない)。
        let (w, h) = (80, 40);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let mut sim = Simulation::new(Rng::new(180));
        for i in 0..5 {
            sim.food.push(Food {
                x: 20.0 + i as f64 * 0.1, // CRAB_EAT_RADIUS圏内に密集(山を模す)
                y: sand_top,
                vy: 0.0,
                life: 999.0,
                landed: true,
            });
        }
        sim.crabs.push(Crab {
            x: 20.0,
            dir: 1.0,
            pause_timer: 0.0,
            facing_right: true,
        });
        sim.update(0.1, w, h);
        assert_eq!(
            sim.food_count(),
            4,
            "1tickにつき1匹のカニが片付けられる餌は1つまでのはず"
        );

        // 十分な回数tickを重ねれば、ゆっくりではあるがいずれ全部片付く
        for _ in 0..10 {
            sim.update(0.1, w, h);
        }
        assert_eq!(sim.food_count(), 0, "十分な時間が経てば山は片付くはず");
    }

    #[test]
    fn seabed_food_count_is_capped_removing_oldest_first() {
        let (w, h) = (80, 40);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let mut sim = Simulation::new(Rng::new(80));
        for i in 0..(SEABED_ITEM_CAP + 5) {
            sim.food.push(Food {
                x: 1.0 + i as f64 * 0.01,
                y: sand_top,
                vy: 0.0,
                life: 999.0,
                landed: true,
            });
        }
        sim.update(0.1, w, h);
        assert_eq!(sim.food_count(), SEABED_ITEM_CAP, "水底の停留数は上限を超えないはず");
    }

    #[test]
    fn hunger_decays_over_time() {
        let mut sim = Simulation::new(Rng::new(1));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0));
        let before = sim.fish[0].hunger;
        run(&mut sim, 5.0, 0.1, 80, 40, false);
        let after = sim.fish[0].hunger;
        assert!(after < before, "空腹度は時間で減るはず: {before} -> {after}");
        assert!((before - after - HUNGER_DECAY * 5.0).abs() < 1.0);
    }

    #[test]
    fn feeding_restores_hunger() {
        let mut sim = Simulation::new(Rng::new(2));
        let mut fish = Fish::new(Species::Guppy, Stage::Adult, 40.0, 20.0);
        fish.hunger = 10.0;
        sim.fish.push(fish);
        sim.food.push(Food {
            x: 40.0,
            y: 20.0,
            vy: 0.0,
            life: 10.0,
            landed: false,
        });
        sim.update(0.1, 80, 40);
        assert!(sim.fish[0].hunger > 10.0, "餌で空腹度が回復するはず");
        assert_eq!(sim.food_count(), 0, "食べられた餌は消えるはず");
    }

    #[test]
    fn feed_and_medicate_emit_sound_events() {
        let mut sim = Simulation::new(Rng::new(70));
        sim.feed(40.0, 80);
        assert!(
            sim.sound_events.contains(&SfxEvent::Feed),
            "餌やりで Feed イベントが発生するはず"
        );
        sim.sound_events.clear();
        sim.medicate(40.0, 80);
        assert!(
            sim.sound_events.contains(&SfxEvent::Medicate),
            "投薬で Medicate イベントが発生するはず"
        );
    }

    #[test]
    fn knock_emits_sound_and_drop_effect_at_cursor() {
        let mut sim = Simulation::new(Rng::new(80));
        sim.knock(40.0, 20.0, 80, 40);
        assert!(
            sim.sound_events.contains(&SfxEvent::GlassKnock),
            "ガラスを叩くと GlassKnock イベントが発生するはず"
        );
        assert_eq!(sim.drop_effects.len(), 1, "叩いた位置に波紋エフェクトが1つ出るはず");
        assert_eq!(sim.drop_effects[0].kind, EffectKind::Knock);
    }

    #[test]
    fn auto_care_feeds_hungry_fish_when_little_floating_food() {
        // 自動モード: 腹ぺこの魚がいて漂っている餌が少なければ自動で餌やりする
        let mut sim = Simulation::new(Rng::new(400));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        f.hunger = 5.0; // 腹ぺこ
        sim.fish.push(f);
        assert!(sim.food.is_empty());

        sim.update_auto_care(0.1, 80, 40);

        assert!(
            !sim.food.is_empty(),
            "腹ぺこの魚がいて餌が無ければ自動で餌やりするはず"
        );
        assert!(sim.sound_events.contains(&SfxEvent::Feed));
    }

    #[test]
    fn auto_care_does_not_feed_when_no_hungry_fish() {
        // 満腹の魚しかいなければ、自動モードでも餌やりしない
        let mut sim = Simulation::new(Rng::new(401));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        f.hunger = MAX_HUNGER;
        sim.fish.push(f);

        for _ in 0..5 {
            sim.update_auto_care(0.5, 80, 40);
        }

        assert!(
            sim.food.is_empty(),
            "腹ぺこの魚がいなければ自動餌やりはしないはず"
        );
    }

    #[test]
    fn auto_care_respects_feed_cooldown_and_does_not_spam() {
        // 短時間に連打相当で呼んでも、投下は最初の1回分だけで、氾濫しないこと
        let mut sim = Simulation::new(Rng::new(402));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        f.hunger = 5.0;
        sim.fish.push(f);

        for _ in 0..5 {
            sim.update_auto_care(0.1, 80, 40);
        }
        let count_after_burst = sim.food.len();
        assert!(
            (1..=5).contains(&count_after_burst),
            "1回の自動投下は3〜5粒のはず: {count_after_burst}"
        );

        // クールダウン(AUTO_FEED_COOLDOWN=30s)未満の間はさらに追加投下しない
        for _ in 0..50 {
            sim.update_auto_care(0.1, 80, 40); // 合計+5秒
        }
        assert_eq!(
            sim.food.len(),
            count_after_burst,
            "クールダウン中・餌が十分ある間は追加投下しないはず"
        );
    }

    #[test]
    fn auto_care_medicates_sick_fish_when_little_floating_medicine() {
        // 自動モード: 病気の魚がいて漂っている薬が少なければ自動で投薬する
        let mut sim = Simulation::new(Rng::new(403));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        f.sick = true;
        sim.fish.push(f);
        assert!(sim.medicine.is_empty());

        sim.update_auto_care(0.1, 80, 40);

        assert!(
            !sim.medicine.is_empty(),
            "病気の魚がいて薬が無ければ自動で投薬するはず"
        );
        assert!(sim.sound_events.contains(&SfxEvent::Medicate));
    }

    #[test]
    fn auto_care_eventually_knocks_at_low_frequency() {
        // 自動モード: 特定の条件を問わず、低頻度でランダムにガラスを叩く演出が入る
        let mut sim = Simulation::new(Rng::new(404));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));

        sim.update_auto_care(0.1, 80, 40); // タイマー初期値0のため最初のtickで発火するはず

        assert!(
            sim.sound_events.contains(&SfxEvent::GlassKnock),
            "自動モードは低頻度でガラスを叩くはず"
        );
        assert!(sim.drop_effects.iter().any(|e| e.kind == EffectKind::Knock));
    }

    #[test]
    fn auto_knock_cooldown_prevents_frequent_triggering() {
        // 自動ガラス叩きは数分に1回程度の低頻度で、短時間に連発しないこと
        let mut sim = Simulation::new(Rng::new(405));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));

        sim.update_auto_care(0.1, 80, 40); // 最初の1回で発火
        sim.sound_events.clear();

        for _ in 0..50 {
            sim.update_auto_care(1.0, 80, 40); // 合計50秒 < AUTO_KNOCK_COOLDOWN(180秒)
        }

        assert!(
            !sim.sound_events.contains(&SfxEvent::GlassKnock),
            "クールダウン中は自動ガラス叩きが再発火しないはず"
        );
    }

    #[test]
    fn knock_makes_nearby_fish_flee_but_not_distant_fish() {
        let mut sim = Simulation::new(Rng::new(81));
        // カーソルのすぐ近くの魚
        let near = Fish::new(Species::Neon, Stage::Adult, 42.0, 21.0);
        // 十分離れた魚(KNOCK_RADIUS=18より遠い)
        let far = Fish::new(Species::Goldfish, Stage::Adult, 79.0, 39.0);
        sim.fish.push(near);
        sim.fish.push(far);

        sim.knock(40.0, 20.0, 80, 40);

        assert!(sim.fish[0].flee_timer > 0.0, "近くの魚は驚いて逃走状態になるはず");
        assert_eq!(sim.fish[1].flee_timer, 0.0, "離れた魚は逃走状態にならないはず");
    }

    #[test]
    fn occasional_knock_does_not_cause_stress() {
        // 稀にしか叩かない通常利用では閾値に届かず、ストレス(病気ボーナス)は付かない。
        let mut sim = Simulation::new(Rng::new(84));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.5, 20.2));
        sim.knock(40.0, 20.0, 80, 40);
        assert_eq!(sim.fish[0].stress_timer, 0.0, "1回叩いただけではストレスは付かないはず");
    }

    #[test]
    fn spamming_knock_gives_nearby_fish_disease_stress() {
        // KNOCK_SPAM_THRESHOLD 回以上、KNOCK_SPAM_WINDOW 以内に連打すると
        // 「叩きすぎ」とみなされ、周辺の魚にストレス(病気発症ボーナス)が付く。
        let mut sim = Simulation::new(Rng::new(85));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.5, 20.2));
        for _ in 0..KNOCK_SPAM_THRESHOLD {
            sim.knock(40.0, 20.0, 80, 40);
        }
        assert!(
            sim.fish[0].stress_timer > 0.0,
            "叩きすぎると周辺の魚にストレスが付くはず"
        );
    }

    #[test]
    fn knock_spam_window_expires_old_knocks() {
        // ウィンドウ外の古い叩きはカウントされないため、間隔を空けて叩けば
        // 閾値回数を超えてもストレスは付かない。
        let mut sim = Simulation::new(Rng::new(86));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.5, 20.2));
        for _ in 0..KNOCK_SPAM_THRESHOLD {
            sim.knock(40.0, 20.0, 80, 40);
            // ウィンドウを超える間隔を空ける
            sim.update(KNOCK_SPAM_WINDOW + 1.0, 80, 40);
            sim.fish[0].stress_timer = 0.0; // 前回分をリセットして純粋に判定だけ見る
        }
        assert_eq!(
            sim.fish[0].stress_timer, 0.0,
            "間隔を空けて叩く分にはストレスは付かないはず"
        );
    }

    #[test]
    fn knock_ignores_dead_fish() {
        let mut sim = Simulation::new(Rng::new(82));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 41.0, 21.0);
        f.dead = true;
        sim.fish.push(f);
        sim.knock(40.0, 20.0, 80, 40);
        assert_eq!(sim.fish[0].flee_timer, 0.0, "死亡演出中の魚は驚かないはず");
    }

    #[test]
    fn fleeing_moves_fish_away_and_timer_expires() {
        let mut sim = Simulation::new(Rng::new(83));
        let fish = Fish::new(Species::Neon, Stage::Adult, 42.0, 20.0);
        sim.fish.push(fish);
        let start_x = sim.fish[0].x;
        sim.knock(40.0, 20.0, 80, 40); // 魚は knock 位置より x が大きい→逃走方向は+x
        assert!(sim.fish[0].flee_timer > 0.0);

        sim.update(0.1, 80, 40);
        assert!(
            sim.fish[0].x >= start_x,
            "驚いた魚は knock 位置から離れる方向へ動くはず: start={start_x} now={}",
            sim.fish[0].x
        );

        // 十分な時間が経てば逃走状態は終わる
        run(&mut sim, FLEE_DURATION + 1.0, 0.1, 80, 40, false);
        assert_eq!(sim.fish[0].flee_timer, 0.0, "十分時間が経てば逃走状態は終わるはず");
    }

    #[test]
    fn knock_flee_consumes_hunger_once_not_repeatedly() {
        // 逃走コスト: ガラスを叩かれて逃げ始めた瞬間に空腹度を消費する。
        // 既に逃走中のうちに再度叩いても、再課金はされない。
        let mut sim = Simulation::new(Rng::new(108));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 42.0, 20.0));
        sim.fish[0].hunger = MAX_HUNGER;
        sim.knock(40.0, 20.0, 80, 40);
        assert_eq!(
            sim.fish[0].hunger,
            MAX_HUNGER - FLEE_HUNGER_COST,
            "逃走開始の瞬間に空腹度が消費されるはず"
        );

        // まだ逃走中(flee_timerが残っている)うちに再度叩いても追加課金されない
        let hunger_after_first = sim.fish[0].hunger;
        sim.knock(40.0, 20.0, 80, 40);
        assert_eq!(
            sim.fish[0].hunger, hunger_after_first,
            "既に逃走中なら再度叩いても追加で空腹度は消費されないはず"
        );
    }

    #[test]
    fn shark_fear_flee_consumes_hunger_once_not_every_tick() {
        // 逃走コスト: サメから逃げ始めた瞬間に空腹度を消費するが、サメが居座って
        // 危険が続いている間、毎tick再課金されるわけではない。
        let mut sim = Simulation::new(Rng::new(109));
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        shark.hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(shark);
        let mut prey = Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0);
        prey.hunger = MAX_HUNGER;
        sim.fish.push(prey);

        sim.update(0.1, 80, 40);
        let hunger_after_one_tick = sim.fish[1].hunger;
        assert!(
            hunger_after_one_tick <= MAX_HUNGER - FLEE_HUNGER_COST + 0.01,
            "逃走開始の瞬間に空腹度が消費されるはず: hunger={hunger_after_one_tick}"
        );

        // サメが空腹状態を維持したまま(捕食モードのまま)何tickか経過しても、
        // 通常のゆっくりした空腹度減少以上には追加で大きく減らないはず
        for _ in 0..5 {
            sim.fish[0].hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0; // サメの捕食モードを維持
            sim.update(0.1, 80, 40);
            if sim.fish.len() < 2 {
                break; // 捕食されてしまったら以降のチェックは不要
            }
        }
        if sim.fish.len() == 2 {
            let hunger_after_more = sim.fish[1].hunger;
            assert!(
                hunger_after_more > hunger_after_one_tick - 1.0,
                "居座られても毎tickは再課金されないはず: {hunger_after_one_tick} -> {hunger_after_more}"
            );
        }
    }

    #[test]
    fn hungry_shark_eats_nearby_prey_and_recovers_hunger() {
        let mut sim = Simulation::new(Rng::new(100));
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        shark.hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食条件を満たす空腹度
        sim.fish.push(shark);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.5, 20.2)); // 捕食圏内

        sim.update(0.1, 80, 40);

        assert_eq!(sim.fish.len(), 1, "捕食された魚はその場で消えるはず");
        assert_eq!(sim.fish[0].species, Species::Shark, "残るのはサメのはず");
        assert!(sim.fish[0].hunger > SHARK_HUNT_HUNGER_THRESHOLD - 10.0, "捕食で空腹度が回復するはず");
        assert_eq!(sim.fish[0].predation_cooldown, SHARK_HUNT_COOLDOWN, "捕食後はクールダウンに入るはず");
        assert!(
            sim.sound_events.contains(&SfxEvent::Predation),
            "捕食で Predation イベントが発生するはず"
        );
        // 血飛沫は複数粒子を散らす強化演出になったため、1個ではなく
        // BLOOD_PARTICLE_COUNT個出るはず(派手・グロテスクに強化する実機要望対応)。
        assert_eq!(
            sim.drop_effects.len(),
            BLOOD_PARTICLE_COUNT,
            "捕食の瞬間に血飛沫エフェクトが複数粒子出るはず"
        );
        assert!(sim.drop_effects.iter().all(|e| e.kind == EffectKind::Blood));
        assert!(
            sim.message.as_deref().unwrap_or("").contains("食べられた"),
            "捕食メッセージが表示されるはず"
        );
        assert_eq!(sim.fish[0].kill_stage, 1, "捕食するたびにkill_stageが増えるはず");
        // 血の滲み(範囲エフェクト)も捕食位置に1つ出るはず
        assert_eq!(sim.blood_stains.len(), 1, "捕食で血の滲みが1つ出るはず");
        // このtick内で生成後すぐにdt(0.1)分減衰するため、ほぼ満タンのはず
        assert!(sim.blood_stains[0].life > BLOOD_STAIN_LIFETIME - 0.2);
    }

    #[test]
    fn blood_stain_fades_out_and_disappears_after_its_lifetime() {
        // 血の滲みはパーティクルより長く残り(3〜5秒程度)、その後は消える。
        let mut sim = Simulation::new(Rng::new(420));
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        shark.hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(shark);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.2, 20.1));
        sim.update(0.1, 80, 40);
        assert_eq!(sim.blood_stains.len(), 1);

        // 寿命の半分程度が経過した時点ではまだ残っている(徐々にフェードする途中)
        run(&mut sim, BLOOD_STAIN_LIFETIME / 2.0, 0.1, 80, 40, false);
        assert_eq!(sim.blood_stains.len(), 1, "寿命の途中ではまだ残っているはず");
        assert!(
            sim.blood_stains[0].life < BLOOD_STAIN_LIFETIME,
            "時間が経つほど残り時間(フェード度)は減っていくはず"
        );

        // 十分な時間が経てば消える
        run(&mut sim, BLOOD_STAIN_LIFETIME, 0.1, 80, 40, false);
        assert!(sim.blood_stains.is_empty(), "十分な時間が経てば血の滲みは消えるはず");
    }

    #[test]
    fn shark_grows_larger_with_each_kill_up_to_cap() {
        // サメは捕食するたびに段階的に大きくなる(上限 SHARK_MAX_KILL_STAGE で打ち止め)
        let mut sim = Simulation::new(Rng::new(304));
        let shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        sim.fish.push(shark);
        let base_scale = sim.fish[0].render_scale();

        for kill in 1..=(SHARK_MAX_KILL_STAGE as usize + 2) {
            sim.fish[0].hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モードに戻す
            sim.fish[0].predation_cooldown = 0.0; // クールダウン解除
            sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.2, 20.1)); // 捕食圏内
            sim.update(0.1, 80, 40);
            let expected_stage = (kill as u8).min(SHARK_MAX_KILL_STAGE);
            assert_eq!(
                sim.fish[0].kill_stage, expected_stage,
                "捕食{kill}回目でkill_stageが上限{SHARK_MAX_KILL_STAGE}まで積み上がるはず"
            );
        }
        assert!(
            sim.fish[0].render_scale() > base_scale,
            "捕食由来の成長で見た目の拡大率が上がるはず"
        );
    }

    #[test]
    fn well_fed_shark_does_not_hunt() {
        let mut sim = Simulation::new(Rng::new(101));
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        shark.hunger = MAX_HUNGER; // 満腹なので狩らない
        sim.fish.push(shark);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.2, 20.1));

        sim.update(0.1, 80, 40);

        assert_eq!(sim.fish.len(), 2, "満腹のサメは捕食しないはず");
    }

    #[test]
    fn shark_does_not_hunt_during_cooldown() {
        let mut sim = Simulation::new(Rng::new(102));
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        shark.hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0;
        shark.predation_cooldown = SHARK_HUNT_COOLDOWN; // クールダウン中
        sim.fish.push(shark);
        sim.fish.push(Fish::new(Species::Goldfish, Stage::Adult, 40.2, 20.1));

        sim.update(0.1, 80, 40);

        assert_eq!(sim.fish.len(), 2, "クールダウン中のサメは捕食しないはず");
    }

    #[test]
    fn shark_does_not_eat_other_sharks() {
        let mut sim = Simulation::new(Rng::new(103));
        let mut shark1 = Fish::new(Species::Shark, Stage::Adult, 40.0, 20.0);
        shark1.hunger = SHARK_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(shark1);
        sim.fish.push(Fish::new(Species::Shark, Stage::Adult, 40.2, 20.1));

        sim.update(0.1, 80, 40);

        assert_eq!(sim.fish.len(), 2, "サメ同士は捕食対象にならないはず");
    }

    #[test]
    fn sick_onset_and_cure_emit_sound_events() {
        // 腹ぺこを維持して発症条件(HUNGRY_SICK_TIME超)を満たし続け、発症するまで回す。
        // 実機フィードバックで発症確率を大幅に下げたため、多数の魚で試行数を稼ぐ。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(71));
        for i in 0..60 {
            sim.fish
                .push(Fish::new(Species::Neon, Stage::Adult, 40.0 + i as f64, 20.0));
        }
        let mut saw_sick_onset = false;
        for _ in 0..12000 {
            for f in &mut sim.fish {
                f.hunger = 5.0; // 腹ぺこ状態を維持
            }
            sim.update(0.1, w, h);
            if sim.sound_events.contains(&SfxEvent::SickOnset) {
                saw_sick_onset = true;
                break;
            }
        }
        assert!(saw_sick_onset, "発症時に SickOnset イベントが発生するはず");
        let sick_idx = sim.fish.iter().position(|f| f.sick).expect("発症した個体が1匹はいるはず");

        sim.sound_events.clear();
        sim.medicine.push(Medicine {
            x: sim.fish[sick_idx].x,
            y: sim.fish[sick_idx].y,
            vy: 0.0,
            life: 10.0,
            landed: false,
        });
        sim.update(0.1, w, h);
        assert!(
            sim.sound_events.contains(&SfxEvent::Cured),
            "治療で Cured イベントが発生するはず"
        );
    }

    #[test]
    fn hungry_onset_emits_sound_event_once() {
        let mut sim = Simulation::new(Rng::new(72));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));
        sim.fish[0].hunger = HUNGRY_THRESHOLD + 1.0; // まだ腹ぺこではない
        sim.update(0.1, 80, 40);
        assert!(
            !sim.sound_events.contains(&SfxEvent::HungryOnset),
            "腹ぺこになる前は HungryOnset は発生しないはず"
        );

        sim.fish[0].hunger = HUNGRY_THRESHOLD - 1.0; // 腹ぺこに転じる
        sim.update(0.1, 80, 40);
        assert!(
            sim.sound_events.contains(&SfxEvent::HungryOnset),
            "腹ぺこに転じた瞬間に HungryOnset が発生するはず"
        );

        sim.sound_events.clear();
        sim.update(0.1, 80, 40); // 引き続き腹ぺこのまま
        assert!(
            !sim.sound_events.contains(&SfxEvent::HungryOnset),
            "腹ぺこが継続しているだけでは再発火しないはず"
        );
    }

    #[test]
    fn bubble_sound_eventually_fires_but_is_throttled() {
        // 気泡音は見た目の気泡よりさらに間引かれる(3〜6秒に1回程度)。
        // タイマー初期値が0のため起動直後の最初のtickで1回鳴るのは仕様通り。
        // その直後はまだ次の間引き時間(3〜6秒)に達していないので鳴らず、
        // 十分な時間(10秒)経てば再び鳴ることを確認する。
        let mut sim = Simulation::new(Rng::new(73));
        sim.update(0.1, 80, 40); // 起動直後の最初のtickで1回鳴る(仕様通り)
        assert!(
            sim.sound_events.contains(&SfxEvent::Bubble),
            "最初のtickでは気泡音が鳴るはず"
        );
        sim.sound_events.clear();

        for _ in 0..9 {
            sim.update(0.1, 80, 40); // 追加で0.9秒(直後は間引き中のはず)
        }
        assert!(
            !sim.sound_events.contains(&SfxEvent::Bubble),
            "直後(1秒未満)はまだ次の気泡音は鳴らないはず"
        );

        let mut saw_bubble = false;
        for _ in 0..90 {
            sim.update(0.1, 80, 40); // さらに9秒(合計10秒)
            if sim.sound_events.contains(&SfxEvent::Bubble) {
                saw_bubble = true;
                break;
            }
        }
        assert!(saw_bubble, "十分な時間放置すれば気泡音が再び鳴るはず");
    }

    #[test]
    fn well_fed_fry_grows_to_adult() {
        let mut sim = Simulation::new(Rng::new(3));
        sim.fish.push(Fish::new(Species::Neon, Stage::Fry, 20.0, 10.0));
        assert_eq!(sim.fish[0].stage, Stage::Fry);
        run(&mut sim, GROW_TIME + 2.0, 0.1, 80, 40, true);
        assert_eq!(sim.fish[0].stage, Stage::Adult, "満腹維持で成魚になるはず");
    }

    #[test]
    fn adult_fish_gains_additional_size_stages_when_well_fed_then_caps() {
        // 成魚になった後も満腹維持を続けるとさらに段階的にサイズが大きくなる(全種共通)。
        // 上限 GENERAL_MAX_GROWTH_STAGE で打ち止めになることも確認する。
        let mut sim = Simulation::new(Rng::new(305));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0));
        assert_eq!(sim.fish[0].growth_stage, 0);
        let base_scale = sim.fish[0].render_scale();

        run(
            &mut sim,
            SIZE_GROW_TIME * (GENERAL_MAX_GROWTH_STAGE as f64 + 2.0),
            0.1,
            80,
            40,
            true,
        );
        assert_eq!(
            sim.fish[0].growth_stage, GENERAL_MAX_GROWTH_STAGE,
            "上限{GENERAL_MAX_GROWTH_STAGE}段階で打ち止めになるはず"
        );
        assert!(
            sim.fish[0].render_scale() > base_scale,
            "サイズ成長で見た目の拡大率が上がるはず"
        );
    }

    #[test]
    fn short_neglect_causes_no_weakness_or_death() {
        // 方針(死亡について v2): 猶予を大幅に延ばしたため、数十秒放置した程度では
        // 「弱っている」表示すら出ず、当然死亡もしない(旧仕様=85秒で死に始めた反省)。
        let mut sim = Simulation::new(Rng::new(4));
        let mut fish = Fish::new(Species::Goldfish, Stage::Adult, 20.0, 10.0);
        fish.hunger = 0.0;
        sim.fish.push(fish);
        run(&mut sim, 60.0, 0.1, 80, 40, false); // STARVE_WEAK_TIME(120s)未満
        assert_eq!(sim.fish_count(), 1, "短時間放置では消えない");
        assert!(!sim.fish[0].dead, "短時間放置では死亡しない");
        assert!(
            sim.fish[0].starve_timer < STARVE_WEAK_TIME,
            "短時間放置では「弱っている」閾値にも達しないはず"
        );
    }

    #[test]
    fn starving_fish_becomes_weak_within_grace_period_but_not_dead() {
        // 「弱っている」状態には入るが、死亡の猶予(STARVE_DEATH_TIME)にはまだ届かないことを確認する。
        let mut sim = Simulation::new(Rng::new(44));
        let mut fish = Fish::new(Species::Goldfish, Stage::Adult, 20.0, 10.0);
        fish.hunger = 0.0;
        sim.fish.push(fish);
        run(&mut sim, 200.0, 0.1, 80, 40, false); // STARVE_WEAK_TIME超・STARVE_DEATH_TIME未満
        assert_eq!(sim.fish_count(), 1, "猶予期間中は消えない");
        assert!(!sim.fish[0].dead, "猶予期間中は死亡しない");
        assert!(
            sim.fish[0].starve_timer >= STARVE_WEAK_TIME,
            "十分放置すれば「弱っている」閾値は超えるはず"
        );
    }

    #[test]
    fn starving_fish_dies_after_long_neglect_then_floats_and_is_removed() {
        // 猶予(STARVE_DEATH_TIME)を超えて放置すると死亡演出(仰向け浮上)に入り、
        // 浮いた状態を DEAD_FLOAT_TIME 維持したのち水槽から消えることを確認する。
        let mut sim = Simulation::new(Rng::new(45));
        let mut fish = Fish::new(Species::Goldfish, Stage::Adult, 20.0, 5.0);
        fish.hunger = 0.0;
        sim.fish.push(fish);
        run(&mut sim, STARVE_DEATH_TIME + 5.0, 0.1, 80, 40, false);
        assert_eq!(sim.fish_count(), 1, "死亡直後はまだ浮いていて水槽内に残る");
        assert!(sim.fish[0].dead, "猶予を超えたら死亡演出に入るはず");
        assert!(sim.fish[0].dead_timer < DEAD_FLOAT_TIME, "死亡直後はまだ浮遊時間内のはず");

        // さらに浮遊時間を超えて放置すると水槽から消える
        run(&mut sim, DEAD_FLOAT_TIME + 3.0, 0.1, 80, 40, false);
        assert_eq!(sim.fish_count(), 0, "浮遊時間を超えたら水槽から消えるはず");
    }

    #[test]
    fn dead_fish_floats_upward_and_stops_near_surface() {
        // 死亡演出中は横移動せず、水面近くまでゆっくり浮上して静止することを確認する。
        let mut sim = Simulation::new(Rng::new(46));
        let mut fish = Fish::new(Species::Goldfish, Stage::Adult, 20.0, 30.0);
        fish.dead = true;
        fish.dead_timer = 0.0;
        let start_y = fish.y;
        let start_x = fish.x;
        sim.fish.push(fish);
        // 浮上して水面近くで静止するのに十分な時間だけ進める(DEAD_FLOAT_TIME 未満)
        for _ in 0..80 {
            sim.update(0.1, 80, 40);
        }
        assert!(sim.fish[0].y < start_y, "死亡後は水面へ向けて浮上するはず");
        assert!(
            sim.fish[0].y <= DEAD_SURFACE_MARGIN + 0.5,
            "水面近くまで浮上したら静止するはず: y={}",
            sim.fish[0].y
        );
        assert_eq!(sim.fish[0].x, start_x, "死亡後は横移動しないはず");
    }

    #[test]
    fn starving_fish_recovers_when_fed() {
        // 弱った魚も餌を与えれば空腹度が回復し、starve_timer がリセットされること
        let mut sim = Simulation::new(Rng::new(41));
        let mut fish = Fish::new(Species::Goldfish, Stage::Adult, 40.0, 20.0);
        fish.hunger = 0.0;
        sim.fish.push(fish);
        run(&mut sim, 15.0, 0.1, 80, 40, false);
        assert!(sim.fish[0].starve_timer > 0.0, "空腹放置で衰弱タイマーが進んでいる前提");
        // 真上に餌を置いて食べさせる
        sim.food.push(Food {
            x: sim.fish[0].x,
            y: sim.fish[0].y,
            vy: 0.0,
            life: 10.0,
            landed: false,
        });
        sim.update(0.1, 80, 40);
        assert!(sim.fish[0].hunger > 0.0, "餌で空腹度が回復するはず");
        assert_eq!(sim.fish[0].starve_timer, 0.0, "回復後は衰弱タイマーがリセットされるはず");
        assert_eq!(sim.fish_count(), 1, "回復した魚はそのまま水槽に残る");
    }

    #[test]
    fn breeding_respects_capacity() {
        let (w, h) = (80, 40);
        let cap = capacity(w, h);
        let mut sim = Simulation::new(Rng::new(5));
        for i in 0..cap {
            let mut f = Fish::new(Species::Neon, Stage::Adult, 10.0 + i as f64, 10.0);
            f.well_fed_timer = BREED_READY_TIME + 5.0;
            sim.fish.push(f);
        }
        run(&mut sim, 40.0, 0.1, w, h, true);
        assert!(
            sim.fish_count() <= cap,
            "個体数は上限{}を超えないはず: {}",
            cap,
            sim.fish_count()
        );
    }

    #[test]
    fn breeding_still_works_when_weak_fish_are_present() {
        // 念のための確認: 猶予期間中の「弱った(空腹0/病気だがまだ死んでいない)個体」が
        // 水槽に居座っていても、それが健康な成魚の産卵→孵化を妨げず、
        // かつ個体数上限も引き続き守られることを確認する。
        // 十分大きい水槽(cap=100)にして、多数の健康個体を混ぜても上限に引っかからないようにする
        let (w, h) = (800, 200);
        let cap = capacity(w, h);
        let mut sim = Simulation::new(Rng::new(31));

        // 弱った(空腹0のまま放置=starve_timer進行中)個体を数匹混ぜる
        for i in 0..3 {
            let mut weak = Fish::new(Species::Goldfish, Stage::Adult, 5.0 + i as f64, 5.0);
            weak.hunger = 0.0;
            weak.starve_timer = STARVE_WEAK_TIME + 5.0;
            sim.fish.push(weak);
        }
        // 病気で弱っている個体も混ぜる
        let mut sick = Fish::new(Species::Guppy, Stage::Adult, 10.0, 5.0);
        sick.sick = true;
        sick.sick_timer = SICK_WEAK_TIME + 5.0;
        sim.fish.push(sick);

        // 産卵準備の整った健康な成魚を多数用意する(産卵頻度が低くなった=実機フィードバック
        // 対応のため、弱った個体が死亡猶予を超えてしまう前に孵化が起きるよう試行数を稼ぐ)
        let healthy_start = sim.fish.len();
        for i in 0..30 {
            let mut healthy = Fish::new(Species::Neon, Stage::Adult, 40.0 + i as f64, 30.0);
            healthy.hunger = MAX_HUNGER;
            healthy.well_fed_timer = BREED_READY_TIME + 5.0;
            sim.fish.push(healthy);
        }

        let start = sim.fish_count();
        // 弱った個体の死亡猶予(STARVE_DEATH_TIME/SICK_DEATH_TIME=630秒、既に+5秒消費済み)
        // より十分短い範囲(250秒=2500tick)に収め、弱った個体が死んで抜けることによる
        // 個体数減少とこのテストの主旨(孵化で増える)が混ざらないようにする。
        for _ in 0..2500 {
            for f in sim.fish.iter_mut().skip(healthy_start) {
                f.hunger = MAX_HUNGER;
                f.well_fed_timer = BREED_READY_TIME + 5.0;
            }
            sim.update(0.1, w, h);
            assert!(
                sim.fish_count() <= cap,
                "弱った個体が混在していても個体数上限{}は守られるはず: {}",
                cap,
                sim.fish_count()
            );
            if sim.fish_count() > start {
                break; // 孵化が起きたら以降のループは不要
            }
        }
        assert!(
            sim.fish_count() > start,
            "弱った個体が混在していても健康な成魚は産卵→孵化で増えるはず: {} -> {}",
            start,
            sim.fish_count()
        );
        // 弱った個体は死亡せずそのまま残っていること
        let weak_survivors = sim
            .fish
            .iter()
            .filter(|f| f.starve_timer > 0.0 || f.sick)
            .count();
        assert!(weak_survivors >= 4, "弱った個体4匹は死亡せず残っているはず");
    }

    #[test]
    fn floating_corpses_do_not_block_hatching_at_capacity() {
        // 念のための確認: 個体数上限ちょうどまで「死んで浮いている」個体で埋まっていても、
        // living_count (生きている個体数) が上限未満なら卵は孵化できることを確認する。
        // (死骸が居座って繁殖が進まなくならないようにする、という仕様上の要求への対応)
        let (w, h) = (80, 40);
        let cap = capacity(w, h);
        let mut sim = Simulation::new(Rng::new(51));

        // 上限ちょうどまで「死亡演出中」の個体で埋める(生きている個体はまだ0匹)
        for i in 0..cap {
            let mut dead_fish = Fish::new(Species::Neon, Stage::Adult, 5.0 + i as f64, 5.0);
            dead_fish.dead = true;
            dead_fish.dead_timer = 0.0;
            sim.fish.push(dead_fish);
        }
        assert_eq!(sim.fish_count(), cap, "総数は上限ちょうど(死骸含む)");
        assert_eq!(sim.living_count(), 0, "生きている個体はまだいない");

        // 孵化を試みる卵を1つ用意する
        sim.eggs.push(Egg {
            x: 10.0,
            y: (h as f64 - sand_height(h) as f64 - 1.0).max(1.0),
            species: Species::Neon,
            hatch: 0.05,
        });
        sim.update(0.1, w, h);
        assert!(
            sim.living_count() >= 1,
            "死骸が総数上限を埋めていても、生きている個体数に余裕があれば孵化できるはず"
        );
    }

    #[test]
    fn well_fed_adult_lays_eggs() {
        // 上限に余裕のある大きな水槽で、満腹の成魚が産卵することを確認。
        // 産卵は確率的(かつ実機フィードバックで大幅に低頻度化した)なので、
        // 複数匹を満腹維持したまま十分長く回して、いつかは卵が出ることを確認する。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(7));
        for i in 0..5 {
            sim.fish
                .push(Fish::new(Species::Guppy, Stage::Adult, 40.0 + i as f64, 30.0));
        }
        let mut saw_egg = false;
        for _ in 0..12000 {
            for f in &mut sim.fish {
                f.hunger = MAX_HUNGER;
                f.well_fed_timer = BREED_READY_TIME + 5.0;
            }
            sim.update(0.1, w, h);
            if !sim.eggs.is_empty() {
                saw_egg = true;
                break;
            }
        }
        assert!(saw_egg, "満腹の成魚は(低頻度になったが)いつかは産卵するはず");
    }

    #[test]
    fn egg_hatches_into_fry_when_below_capacity() {
        // 卵は一定時間で孵化して稚魚になる(2段階繁殖の後半)
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(11));
        sim.eggs.push(Egg {
            x: 40.0,
            y: 90.0,
            species: Species::Neon,
            hatch: 0.05,
        });
        let before = sim.fish_count();
        sim.update(0.1, w, h);
        assert_eq!(sim.fish_count(), before + 1, "卵は孵化して稚魚が1匹増えるはず");
        assert!(sim.eggs.is_empty(), "孵化した卵は消えるはず");
        assert_eq!(sim.fish[before].stage, Stage::Fry, "孵化直後は稚魚");
    }

    #[test]
    fn egg_does_not_hatch_at_capacity() {
        // 上限に達していると卵は孵化せず消える
        let (w, h) = (80, 40);
        let cap = capacity(w, h);
        let mut sim = Simulation::new(Rng::new(12));
        for i in 0..cap {
            sim.fish
                .push(Fish::new(Species::Neon, Stage::Adult, 10.0 + i as f64, 10.0));
        }
        sim.eggs.push(Egg {
            x: 40.0,
            y: 35.0,
            species: Species::Neon,
            hatch: 0.05,
        });
        sim.update(0.1, w, h);
        assert_eq!(sim.fish_count(), cap, "上限では孵化で増えない");
        assert!(sim.eggs.is_empty(), "孵化できない卵も消える");
    }

    #[test]
    fn disease_onset_and_medicine_cures() {
        // 病気の魚が薬に触れると治る
        let mut sim = Simulation::new(Rng::new(13));
        let mut f = Fish::new(Species::Goldfish, Stage::Adult, 40.0, 20.0);
        f.sick = true;
        f.sick_timer = 5.0;
        f.hunger = MAX_HUNGER;
        sim.fish.push(f);
        sim.medicine.push(Medicine {
            x: 40.0,
            y: 20.0,
            vy: 0.0,
            life: 10.0,
            landed: false,
        });
        sim.update(0.1, 80, 40);
        assert!(!sim.fish[0].sick, "薬で病気が治るはず");
        assert_eq!(sim.medicine.len(), 0, "使われた薬は消えるはず");
    }

    #[test]
    fn medicine_harmless_to_healthy_fish() {
        let mut sim = Simulation::new(Rng::new(17));
        let mut f = Fish::new(Species::Guppy, Stage::Adult, 40.0, 20.0);
        f.hunger = MAX_HUNGER;
        sim.fish.push(f);
        sim.medicine.push(Medicine {
            x: 40.0,
            y: 20.0,
            vy: 0.0,
            life: 10.0,
            landed: false,
        });
        sim.update(0.1, 80, 40);
        assert!(!sim.fish[0].sick, "健康な魚は病気にならない");
        // 健康な魚には反応しないので薬は残る(寿命内)
        assert_eq!(sim.medicine.len(), 1, "健康な魚には薬が消費されない");
    }

    #[test]
    fn hungry_fish_gets_sick_over_time() {
        // 腹ぺこ放置で発症すること(確率的だが十分な時間放置すれば発症する)。
        // 実機フィードバックで発症確率を大幅に下げた(旧仕様より低頻度)ため、
        // 多数の魚を同時に腹ぺこにして試行数を稼ぎ、現実的な時間で収束させる。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(19));
        for i in 0..60 {
            sim.fish
                .push(Fish::new(Species::Neon, Stage::Adult, 40.0 + i as f64, 20.0));
        }
        let mut got_sick = false;
        for _ in 0..12000 {
            // 腹ぺこ状態を維持する(hunger を毎ステップ低いまま保つ)
            for f in &mut sim.fish {
                f.hunger = 5.0;
            }
            sim.update(0.1, w, h);
            if sim.fish.iter().any(|f| f.sick) {
                got_sick = true;
                break;
            }
        }
        assert!(got_sick, "腹ぺこ長期放置で発症するはず");
    }

    #[test]
    fn sick_fish_recovers_with_medicine_before_death() {
        // 猶予期間中(SICK_DEATH_TIME未満)は病気を放置しても死亡しない。「弱っている」状態の
        // まま残り、薬を与えれば猶予中に治癒して通常状態に戻ることを検証する。
        let mut sim = Simulation::new(Rng::new(43));
        let mut f = Fish::new(Species::Guppy, Stage::Adult, 40.0, 20.0);
        f.sick = true;
        f.hunger = MAX_HUNGER;
        sim.fish.push(f);
        // SICK_WEAK_TIME は超えるが SICK_DEATH_TIME にはまだ届かない範囲で放置する
        for _ in 0..800 {
            sim.fish[0].hunger = MAX_HUNGER; // 空腹側の死亡経路は無関係にしておく
            sim.fish[0].sick = true; // 病気を維持
            sim.update(0.1, 80, 40);
        }
        assert_eq!(sim.fish_count(), 1, "猶予期間中は水槽から消えないはず");
        assert!(!sim.fish[0].dead, "猶予期間中は死亡しないはず");
        assert!(sim.fish[0].sick_timer >= SICK_WEAK_TIME, "長期放置で「弱っている」閾値は超えるはず");

        // 薬を与えると治癒する
        sim.medicine.push(Medicine {
            x: sim.fish[0].x,
            y: sim.fish[0].y,
            vy: 0.0,
            life: 10.0,
            landed: false,
        });
        sim.update(0.1, 80, 40);
        assert!(!sim.fish[0].sick, "薬で治癒するはず");
    }

    #[test]
    fn sick_fish_dies_after_long_neglect_then_floats_and_is_removed() {
        // 病気側も空腹側と同様、猶予(SICK_DEATH_TIME)を超えると死亡演出に入り、
        // 浮遊時間(DEAD_FLOAT_TIME)を経て水槽から消えることを確認する。
        let mut sim = Simulation::new(Rng::new(47));
        let mut f = Fish::new(Species::Guppy, Stage::Adult, 40.0, 5.0);
        f.sick = true;
        f.hunger = MAX_HUNGER; // 空腹側の死亡経路とは無関係にしておく
        sim.fish.push(f);
        let steps = ((SICK_DEATH_TIME + 5.0) / 0.1).round() as usize;
        for _ in 0..steps {
            sim.fish[0].hunger = MAX_HUNGER;
            sim.fish[0].sick = true;
            sim.update(0.1, 80, 40);
        }
        assert_eq!(sim.fish_count(), 1, "死亡直後はまだ浮いていて水槽内に残る");
        assert!(sim.fish[0].dead, "病気の猶予を超えたら死亡演出に入るはず");

        run(&mut sim, DEAD_FLOAT_TIME + 3.0, 0.1, 80, 40, false);
        assert_eq!(sim.fish_count(), 0, "浮遊時間を超えたら水槽から消えるはず");
    }

    #[test]
    fn pause_stops_simulation() {
        let mut sim = Simulation::new(Rng::new(23));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0));
        let before = sim.fish[0].hunger;
        // dt=0(一時停止相当)では何も進まない
        sim.update(0.0, 80, 40);
        assert_eq!(sim.fish[0].hunger, before, "一時停止中は空腹度が減らない");
        assert_eq!(sim.elapsed, 0.0, "一時停止中は経過時間が進まない");
    }

    #[test]
    fn reset_restores_initial_state() {
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(29));
        sim.seed_initial(w, h);
        sim.feed(w as f64 / 2.0, w);
        sim.medicate(w as f64 / 2.0, w);
        run(&mut sim, 3.0, 0.1, w, h, false);
        sim.reset(w, h);
        assert_eq!(sim.food_count(), 0, "リセットで餌が消える");
        assert_eq!(sim.medicine.len(), 0, "リセットで薬が消える");
        assert_eq!(sim.eggs.len(), 0, "リセットで卵が消える");
        assert_eq!(sim.elapsed, 0.0, "リセットで経過時間が0に戻る");
        assert!(sim.fish_count() > 0, "リセット後は初期個体が存在する");
    }

    #[test]
    fn add_fish_is_capped_at_manual_cap_even_with_room_in_tank_capacity() {
        // 十分大きい水槽(capacity=100近く)でも、+キーによる直接追加は
        // ADD_FISH_MANUAL_CAP(50匹)で頭打ちになり、それ以上は増えない。
        let (w, h) = (800, 200);
        assert!(capacity(w, h) > ADD_FISH_MANUAL_CAP, "テスト前提: 水槽容量は50より大きい");
        let mut sim = Simulation::new(Rng::new(91));
        for _ in 0..(ADD_FISH_MANUAL_CAP + 10) {
            sim.add_fish(w, h);
        }
        assert_eq!(
            sim.fish_count(),
            ADD_FISH_MANUAL_CAP,
            "+キーでの追加は50匹で頭打ちになるはず"
        );
    }

    #[test]
    fn seed_initial_never_includes_sharks() {
        // サメの入手経路はSキーのみに限定する方針: 初期配置にサメは含まれない。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(99));
        sim.seed_initial(w, h);
        assert!(!sim.fish.is_empty(), "テスト前提: 初期個体が存在すること");
        assert!(
            sim.fish.iter().all(|f| f.species != Species::Shark),
            "seed_initial はサメを含まないはず"
        );
    }

    #[test]
    fn reset_never_includes_sharks() {
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(100));
        // 一度サメを混ぜてからリセットする
        sim.add_shark(w, h);
        sim.reset(w, h);
        assert!(
            sim.fish.iter().all(|f| f.species != Species::Shark),
            "グレートリセット後の初期配置にサメは含まれないはず"
        );
    }

    #[test]
    fn add_fish_random_pick_never_includes_sharks() {
        // +キー(ランダム追加)は通常3種のみからのはず。十分回数試して確認する。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(101));
        for _ in 0..40 {
            sim.add_fish(w, h);
        }
        assert!(
            sim.fish.iter().all(|f| f.species != Species::Shark),
            "+キーのランダム追加はサメを選ばないはず"
        );
    }

    #[test]
    fn shark_never_lays_eggs_even_when_breed_ready() {
        // サメは産卵→孵化の繁殖ロジックから除外されている(Sキー以外で増えない方針)。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(102));
        let mut shark = Fish::new(Species::Shark, Stage::Adult, 40.0, 30.0);
        shark.hunger = MAX_HUNGER;
        shark.well_fed_timer = BREED_READY_TIME + 5.0;
        sim.fish.push(shark);
        for _ in 0..12000 {
            sim.fish[0].hunger = MAX_HUNGER;
            sim.fish[0].well_fed_timer = BREED_READY_TIME + 5.0;
            sim.update(0.1, w, h);
        }
        assert!(sim.eggs.is_empty(), "サメはどれだけ満腹維持しても産卵しないはず");
        assert_eq!(sim.fish_count(), 1, "サメが増えていないはず");
    }

    #[test]
    fn elderly_fish_spawns_exactly_once_regardless_of_hunger() {
        // 老齢に達した瞬間、満腹状態などの条件を問わず確定で1回だけ産卵する
        // (確率アップではなく一度きりの確定イベント)。
        let mut sim = Simulation::new(Rng::new(306));
        let mut f = Fish::new(Species::Neon, Stage::Fry, 20.0, 10.0); // 稚魚・満腹でもない
        f.hunger = 5.0; // 満腹条件を問わないことを確認するため、あえて腹ぺこにしておく
        f.age = ELDERLY_AGE - 0.05;
        sim.fish.push(f);
        assert!(sim.eggs.is_empty());

        sim.update(0.1, 80, 40); // ELDERLY_AGE を跨ぐ

        assert!(
            sim.fish[0].elderly_spawned,
            "老齢突入で確定産卵フラグが立つはず"
        );
        assert!(
            (2..=4).contains(&sim.eggs.len()),
            "老齢突入の瞬間に確定で(満腹でなくても)卵を産むはず: eggs={}",
            sim.eggs.len()
        );

        // フラグが立った後は、この確定イベント由来の追加産卵は起きない(1回のみ)
        let eggs_after_first_event = sim.eggs.len();
        for _ in 0..5 {
            sim.update(0.1, 80, 40);
        }
        assert!(
            sim.fish[0].elderly_spawned,
            "フラグは立ったまま維持されるはず"
        );
        // 短時間なので通常の確率的産卵はほぼ起こらない前提で、確定イベント分から
        // 大きく増えていないことを確認する(厳密な個数比較ではなく暴走チェック)
        assert!(sim.eggs.len() < eggs_after_first_event + 4);
    }

    #[test]
    fn fish_dies_of_old_age_after_lifespan() {
        // 寿命(LIFESPAN_DEATH_AGE)に達すると老衰で死亡演出に入る(全種共通・サメも対象)。
        let mut sim = Simulation::new(Rng::new(307));
        let mut f = Fish::new(Species::Shark, Stage::Adult, 20.0, 10.0);
        f.hunger = MAX_HUNGER; // 空腹による死亡ではないことを明確にする
        f.elderly_spawned = true; // 老齢確定産卵は済んだ前提にして、死亡判定だけを見る
        f.age = LIFESPAN_DEATH_AGE - 0.05;
        sim.fish.push(f);

        sim.update(0.1, 80, 40); // LIFESPAN_DEATH_AGE を跨ぐ

        assert!(sim.fish[0].dead, "寿命に達したら老衰で死亡演出に入るはず");
        assert_eq!(sim.fish[0].starve_timer, 0.0, "空腹による死亡ではないことの確認");
        assert!(!sim.fish[0].sick, "病気による死亡ではないことの確認");
        assert!(
            sim.message.as_deref().unwrap_or("").contains("老衰"),
            "老衰による死亡メッセージが表示されるはず"
        );
    }

    #[test]
    fn add_shark_always_adds_a_shark() {
        // Sキー: ランダムではなく確実にサメを追加できる。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(96));
        for _ in 0..5 {
            sim.add_shark(w, h);
        }
        assert_eq!(sim.fish_count(), 5, "5回呼べば5匹追加されるはず");
        assert!(
            sim.fish.iter().all(|f| f.species == Species::Shark),
            "add_shark で追加されるのは常にサメのはず"
        );
    }

    #[test]
    fn add_shark_is_capped_at_manual_cap() {
        let (w, h) = (800, 200);
        assert!(capacity(w, h) > ADD_FISH_MANUAL_CAP, "テスト前提: 水槽容量は50より大きい");
        let mut sim = Simulation::new(Rng::new(97));
        for _ in 0..(ADD_FISH_MANUAL_CAP + 10) {
            sim.add_shark(w, h);
        }
        assert_eq!(
            sim.fish_count(),
            ADD_FISH_MANUAL_CAP,
            "Sキーでの追加も+キーと同じく50匹で頭打ちになるはず"
        );
    }

    #[test]
    fn add_shark_respects_tank_capacity_too() {
        // ADD_FISH_MANUAL_CAP(50)より小さい水槽容量でも、そちらの上限が優先されて超えない。
        let (w, h) = (40, 20); // capacity は最小の5になる
        let cap = capacity(w, h);
        assert!(cap < ADD_FISH_MANUAL_CAP, "テスト前提: 水槽容量が50より小さいこと");
        let mut sim = Simulation::new(Rng::new(98));
        for _ in 0..20 {
            sim.add_shark(w, h);
        }
        assert_eq!(sim.fish_count(), cap, "水槽容量の上限で頭打ちになるはず");
    }

    #[test]
    fn remove_fish_falls_back_to_crabs() {
        // 「魚 0/N」まで減らしてもカニが残り続けて分かりづらい、という
        // フィードバックへの対応。- キーは通常魚(サメ含む)→カニの順にフォールバックする。
        let mut sim = Simulation::new(Rng::new(90));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 10.0, 10.0));
        sim.crabs.push(Crab {
            x: 10.0,
            dir: 1.0,
            pause_timer: 0.0,
            facing_right: true,
        });

        // 1回目: 通常魚がいるのでそれが減る
        sim.remove_fish();
        assert_eq!(sim.fish.len(), 0, "通常魚がいればまずそれが減るはず");
        assert_eq!(sim.crabs.len(), 1, "通常魚がいる間はカニは減らないはず");

        // 2回目: 通常魚が0なのでカニが減る
        sim.remove_fish();
        assert_eq!(sim.crabs.len(), 0, "通常魚が0ならカニが減るはず");

        // 3回目: 全部0の状態でも panic せず何も起きない
        sim.remove_fish();
        assert_eq!(sim.fish.len(), 0);
        assert_eq!(sim.crabs.len(), 0);
    }

    #[test]
    fn capacity_scales_with_size_and_is_bounded() {
        assert!(capacity(40, 20) >= 5);
        assert!(capacity(1000, 800) <= 100, "上限は100匹");
        assert_eq!(capacity(1000, 800), 100, "十分大きい端末では100匹に到達する");
        assert!(capacity(200, 100) >= capacity(80, 40));
    }

    // 回帰テスト: 疑似端末等で極端に小さい pix_w/pix_h が渡されても
    // `x.clamp(1.0, upper)` の upper < 1.0(min > max)で panic しないこと。
    // (実機で cell_rows=1 相当の疑似端末を起動して発見された panic の再発防止)
    #[test]
    fn update_does_not_panic_on_tiny_dimensions() {
        for h in 0..=3usize {
            for w in 0..=3usize {
                let mut sim = Simulation::new(Rng::new(42));
                sim.seed_initial(w, h);
                sim.feed(w as f64 / 2.0, w);
                sim.medicate(w as f64 / 2.0, w);
                for _ in 0..20 {
                    sim.update(0.1, w, h);
                }
            }
        }
    }

    #[test]
    fn safe_upper_never_returns_below_one() {
        assert_eq!(safe_upper(-5.0), 1.0);
        assert_eq!(safe_upper(0.0), 1.0);
        assert_eq!(safe_upper(0.5), 1.0);
        assert_eq!(safe_upper(f64::NAN), 1.0);
        assert_eq!(safe_upper(5.0), 5.0);
    }

    #[test]
    fn clamp_point_keeps_result_within_tank_bounds() {
        let (w, h) = (80, 40);
        // 範囲外の値を渡しても水槽内(かつ min<=max)に収まること
        let (x, y) = clamp_point(-100.0, -100.0, w, h);
        assert!((1.0..=(w as f64)).contains(&x));
        assert!((1.0..=(h as f64)).contains(&y));
        let (x2, y2) = clamp_point(9999.0, 9999.0, w, h);
        assert!((1.0..=(w as f64)).contains(&x2));
        assert!((1.0..=(h as f64)).contains(&y2));
        // 極端に小さい端末でも panic しないこと
        let _ = clamp_point(5.0, 5.0, 0, 0);
        let _ = clamp_point(5.0, 5.0, 1, 1);
    }

    #[test]
    fn ensure_decorative_entities_seeds_once_and_is_idempotent() {
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(60));
        assert!(sim.crabs.is_empty());
        sim.ensure_decorative_entities(w, h);
        assert_eq!(sim.crabs.len(), CRAB_COUNT, "カニは既定数だけ補充される");
        // 既に populated な状態で再度呼んでも増殖しない(旧セーブ復元後の二重補充防止)
        sim.ensure_decorative_entities(w, h);
        assert_eq!(sim.crabs.len(), CRAB_COUNT, "既に居る場合は補充されない");
    }

    #[test]
    fn crabs_stay_in_bounds_and_do_not_panic() {
        // 観賞用エンティティ(カニ)は育成ロジックの対象外だが、長時間の徘徊で
        // 座標が水槽の範囲を飛び出したり panic したりしないことを確認する。
        let (w, h) = (60, 30);
        let mut sim = Simulation::new(Rng::new(61));
        sim.seed_initial(w, h);
        for _ in 0..500 {
            sim.update(0.1, w, h);
        }
        for c in &sim.crabs {
            assert!(c.x >= 0.0 && c.x <= w as f64, "カニのxは範囲内: {}", c.x);
        }
    }
}
