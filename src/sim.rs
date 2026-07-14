// 水槽シミュレーション本体: 魚の遊泳・餌・薬・卵・気泡の更新、育成ロジック
// (空腹度・成長・産卵→孵化・病気・死亡)。端末描画には依存しない純粋なロジック。

use crate::fish::{den_sprite, rock_sprite, Fish, Species, Stage};
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

// --- サイズと機敏さの連動(新規): 小さい魚ほど通常時もキビキビ ---
// 「大きくなるほど遅くなる」(SIZE_SPEED_PENALTY_STEP)と対になる形で、稚魚(Fry)や
// まだ成長していない小さい個体ほど、特別なトリガー(空腹・逃走等)が無い通常の遊泳
// (ランダムウォーク+慣性)でもキビキビと素早く・方向転換も頻繁になるようにする。
// 稚魚は成魚(growth_stage=0)よりAGILITY_FRY_SIZE_STEPS段階分小さい扱いとし、
// growth_stage・(ピラニアのみ)kill_stageが上がるほど大きくなる分と同じ軸で滑らかに連動させる。
pub const AGILITY_STEP: f64 = 0.12; // サイズ1段階ぶんの機敏さ変化率
pub const AGILITY_FRY_SIZE_STEPS: f64 = 2.0; // 稚魚は成魚基準よりこの段階数分小さい扱い
pub const AGILITY_MULT_MIN: f64 = 0.4; // 大型個体でも遊泳ロジックが完全に死なないための下限
pub const AGILITY_MULT_MAX: f64 = 1.6; // 稚魚が過剰に暴れすぎないための上限

// --- ピラニアの捕食によるサイズ成長 ---
// ピラニアは魚を1匹捕食するごとに段階的に大きくなる(0..=3の4段階、上限あり)。
// 全種共通の成長段階とは別枠で、ピラニアの場合は両方が積み重なって見た目に反映される。
pub const PIRANHA_MAX_KILL_STAGE: u8 = 3;
pub const PIRANHA_KILL_GROWTH_SCALE_STEP: f64 = 0.18;

// --- 寿命・世代交代(全種共通。ピラニアも含む) ---
// 空腹度の時間感覚(満タン→0まで約60分)に合わせ、寿命は数時間〜半日のイメージで調整。
// 老齢(ELDERLY_AGE)に達すると、満腹状態などの条件を問わず「次世代を残す最後の
// チャンス」として確定で1回だけ産卵する(産卵確率アップではなく一度きりの確定イベント。
// 「老いると産卵確率が上がる」は生物学的に不自然という指摘を受けての方針)。
// ピラニアは対象外(ピラニアは`S`キー以外で増えない方針のため、老齢確定産卵イベントも発生しない)。
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
// 大型魚は Species::Piranha として通常の育成対象に統合された) ---
pub const CRAB_COUNT: usize = 3;
pub const CRAB_SPEED: f64 = 3.0; // 水底を歩く速さ
pub const CRAB_PAUSE_CHANCE_PER_SEC: f64 = 0.15; // 毎秒、立ち止まる確率
pub const CRAB_EAT_RADIUS: f64 = 3.0; // カニが水底の餌・薬を片付けられる距離

// --- 藻・水草・岩・タコつぼ(装飾。育成ロジックには参加しない静的な背景オブジェクト) ---
pub const PLANT_COUNT: usize = 5; // 水底に配置する藻・水草の本数
pub const PLANT_SWAY_FREQ: f64 = 1.2; // 揺れの速さ(見た目のみ・rad/秒相当)
pub const ROCK_COUNT: usize = 3; // 水底に配置する岩の数
pub const DEN_COUNT: usize = 1; // タコつぼの数(タコの数と1:1で対応する)
// 魚が藻・水草・岩に近いとき、視覚的に「隠れている」表現(色を背景に馴染ませる)にし、
// かつ実際にピラニア・タコから捕食対象にならなくなる距離。実機フィードバック
// (「藻・岩を魚が隠れられるくらい大きく」「隠れたら実際に捕食されなくなる機能化」)
// を受けて、旧サイズ(単一の細い水草・6.0)から、魚がすっぽり収まるくらいの
// 大きな藻の株・岩塊に描き直した上でこの距離も広げた(旧6.0)。
pub const ALGAE_HIDE_RADIUS: f64 = 9.0;
pub const ALGAE_HIDE_MIX: f64 = 0.55; // 隠れ表現の強さ(背景色へどれだけ寄せるか)

// --- タコの隠れる/出てくる状態遷移 ---
// 低頻度でつぼから出てきて泳ぎ、しばらくしたら戻る。実機フィードバック
// (「出たと思ったらすぐ消える。観察機会が少なすぎる」)を受けて、出ている時間を
// 長めに(旧8〜20秒→20〜40秒)、隠れている時間を短めに(旧30〜90秒→15〜40秒)調整した。
pub const OCTOPUS_HIDDEN_TIME_MIN: f64 = 15.0;
pub const OCTOPUS_HIDDEN_TIME_MAX: f64 = 40.0;
pub const OCTOPUS_EMERGE_TIME_MIN: f64 = 20.0;
pub const OCTOPUS_EMERGE_TIME_MAX: f64 = 40.0;
// 出ている残り時間がこの値未満になったら、巣へ戻る引力をかけて泳いで戻る様子を見せる
// (時間切れの瞬間に確実に隠れさせる処理自体は update_octopus() 側で保証している)。
pub const OCTOPUS_RETURN_WINDOW: f64 = 4.0;
// 実機フィードバック(「生き物の基本移動速度を4倍に。シミュレーション再生速度とは別物」)
// を受けて、fish.rsのmax_speed()/wander()/food_pull()と同じ考え方で4倍にした(旧60.0)。
pub const OCTOPUS_RETURN_PULL: f64 = 240.0;

// --- タコの足のうねうねアニメーション(見た目のみ。描画側=main.rsで使う) ---
// 頭部(マント)は静止させたまま、足の部分だけを時間経過でサイン波的に左右へ
// オフセットさせて波打つように見せる。足の付け根から先端に向かうほど振れを
// 大きくする(実際のオフセット計算はmain.rs側で行う)。
pub const OCTOPUS_LEG_WIGGLE_FREQ: f64 = 2.2; // 波打つ速さ
pub const OCTOPUS_LEG_WIGGLE_AMPLITUDE: f64 = 0.35; // 足の付け根1段あたりの振れ幅(論理ピクセル)

// --- タコ自身の捕食(ピラニアと同様に低頻度・空腹時のみ・クールダウンあり) ---
// ピラニアと同じ理由(GAIN=60が大きいため、閾値を高くしないと満腹状態が長時間続く)で99にする
pub const OCTOPUS_HUNT_HUNGER_THRESHOLD: f64 = 99.0;
pub const OCTOPUS_HUNT_RADIUS: f64 = 20.0;
// 実機フィードバック(「壁際で捕食できず振動する」)を受けてピラニア側と同様に広げた
// (旧3.0→4.5)。さらに実機フィードバック(「口基準にした上で判定距離自体も
// もっと広く」)を受けて再度拡大した(旧4.5→7.0)。さらに実機フィードバック
// (「タコのあたり判定もまだ狭い。ピラニアと同様にさらに広げてほしい」)を受けて
// 再度拡大した(旧7.0)。壁際で反発力と吸引力が拮抗して詰め切れない問題の
// 緩和策の一つでもある。
pub const OCTOPUS_STRIKE_RADIUS: f64 = 10.0;
pub const OCTOPUS_HUNT_COOLDOWN: f64 = 20.0;
pub const OCTOPUS_PREDATION_HUNGER_GAIN: f64 = 60.0;
// 実機フィードバック(「追跡中も速さが体感できない」)を受けてピラニア側と同様に強化(旧90.0)。
// さらに実機フィードバック(「生き物の基本移動速度を4倍に」)を受けて4倍にした(旧140.0)。
pub const OCTOPUS_HUNT_PULL: f64 = 560.0;

// --- 墨(タコがピラニアに追われると吐く) ---
// タコの近くに捕食モードのピラニアがいたら、逃走(既存のfear_target経由)に加えて墨を吐く。
pub const OCTOPUS_INK_TRIGGER_RADIUS: f64 = 26.0; // この距離以内に捕食モードのピラニアがいたら墨を吐く
pub const INK_COOLDOWN: f64 = 20.0; // 連発防止のクールダウン
// 墨のエフェクト: 血の滲みより広め・勢いよく拡散し、数秒(目安3〜5秒)残ってから薄れて消える。
pub const INK_LIFETIME: f64 = 4.5;
pub const INK_GROWTH_TIME: f64 = 1.2; // 血より大幅に速く広がる(「わーーーっと」拡散するイメージ)
pub const INK_MAX_RADIUS: f64 = 28.0; // 血の滲み(20.0)より広め
pub const INK_MIX: f64 = 0.9;
pub const INK_HOLD_FRACTION: f64 = 0.35;
// 墨が広がっている間、その範囲にいる捕食者(ピラニア等)は獲物を検知できなくなる
// (「視界が悪くなる」演出。捕食者側のchase_target判定を一時的に無効化する)。
// 実機フィードバック(「墨を吐いたら高確率で逃げ切れる、という結果まで保証してほしい」)
// を受けて、視界不良(検知不能)だけでなく以下も組み合わせる:
// (1)墨を吐いた瞬間の緊急ダッシュ(速度ブースト) (2)吐いた直後は捕食判定
// (strike radius)からも一時的に除外。タコが十分離れられるよう、視界不良の持続時間
// (INK_LIFETIME)より短い猶予として設定している。
pub const INK_ESCAPE_DURATION: f64 = 3.0;
pub const INK_ESCAPE_SPEED_MULT: f64 = 1.4; // 既存の逃走ブースト(1.6倍)にさらに掛かる緊急ダッシュ分
pub const INK_ESCAPE_STRENGTH_MULT: f64 = 1.8; // 逃走ベクトル自体もこの間はさらに強める
pub const SEABED_ITEM_CAP: usize = 30; // 水底に停留できる餌・薬それぞれの上限数(超過分は古い順に消える)
// 水底の食べ残しが「積もって山になる」見た目のためのパラメータ。
// 近くに既に着地済みのものが多いほど高く積み上げ、離れるほど低くなるので
// 自然と裾野の広がった山型になる。
pub const PILE_RADIUS: f64 = 2.5; // この距離以内を「同じ山」とみなす
pub const PILE_STACK_STEP: f64 = 0.6; // 近くの着地済み1個につき盛り上がる高さ
// 実機フィードバック(「山として認識できるレベルまで最大高さを上げてほしい」)を
// 受けて、旧3.0の2〜3倍程度である8.0まで引き上げた(視覚的インパクトを強化)。
pub const PILE_MAX_HEIGHT: f64 = 8.0;

// --- ピラニアの捕食 ---
// 実機フィードバック(「捕食頻度が低すぎる」)を受けて方針転換: 従来の「頻度は控えめに」
// から「ピラニアは頻繁に狩る」体感になるよう、閾値・クールダウン・検知範囲をまとめて強化した。
// (捕食1回の空腹度回復量=PIRANHA_PREDATION_HUNGER_GAIN=70が大きいため、閾値を上げないと
// 1回捕食するたびに長時間満腹状態が続いてしまう。閾値を引き上げることで、捕食後の
// 「次に狩れるようになるまでの実質的な待ち時間」を大幅に短縮している。
// 実機計測: 閾値95では捕食1回ごとに満腹(100)から95まで下がるのに約3分かかり、
// 「頻繁」というには程遠かったため、99まで引き上げて待ち時間を約36秒まで圧縮した)
pub const PIRANHA_HUNT_HUNGER_THRESHOLD: f64 = 99.0; // 空腹度がこれ未満のときだけ捕食行動を取る(旧70)
pub const PIRANHA_HUNT_RADIUS: f64 = 30.0; // この距離以内の獲物へ近づいていく(旧22)
// 実機フィードバック(「壁際に追い詰めた魚を永遠に捕食できない」)を受けて拡大した
// (旧3.5→5.0)。壁際では反発力(wall_push)と追跡吸引力(PIRANHA_HUNT_PULL)が拮抗して
// 詰め切れず振動する現象があったため、これに加えて壁際での反発力自体を弱める・
// 吸引力を強めるの3点セットで対応している(update_movement側を参照)。
// さらに実機フィードバック(「口基準にした上で判定距離自体ももっと広く。狙った
// 獲物にきちんと届くように」)を受けて再度拡大した(旧5.0)。
pub const PIRANHA_STRIKE_RADIUS: f64 = 8.0;
// 獲物へ近づく吸引の強さ。「追いかけている」のが見た目でわかるよう、通常の遊泳を
// 弱めた上でこれを強くかける(旧26.0→100.0→さらに強化)。実機フィードバック
// (「追跡中も速さが体感できない」)を受け、最高速度のクランプ(PIRANHA_CHASE_SPEED_MULT)
// だけでは短い追跡では速度差が体感できないため、加速度自体もさらに強化した。
// さらに実機フィードバック(「生き物の基本移動速度を4倍に」)を受けて4倍にした(旧160.0)。
pub const PIRANHA_HUNT_PULL: f64 = 640.0;
pub const PIRANHA_HUNT_COOLDOWN: f64 = 15.0; // 捕食後、次に捕食できるようになるまでの時間(旧45)
// ピラニアは追跡(捕食モード)中だけ、通常3種の最高速度(最速はネオンの22.0)より
// はっきり速くなるようにする倍率。巡回中(追跡していないとき)は通常のsp.max_speed()
// のままで特別早くしない。魚が機敏に逃げても追いつかれることがある緊張感を出す。
pub const PIRANHA_CHASE_SPEED_MULT: f64 = 1.8;
pub const PIRANHA_PREDATION_HUNGER_GAIN: f64 = 70.0; // 捕食による空腹度の回復量(餌より効率的)
// ピラニアのカニバリズム(新規): 「大きいピラニアは小さいピラニアを食ってもいい」という要望を受け、
// 同種(ピラニア)を無条件に対象外とするのではなく、十分サイズ差(成長段階+捕食成長段階の
// 合計)があるときだけ対象に含める。近いサイズ同士は対象外のまま(共食いの乱発を防ぐ)。
pub const PIRANHA_CANNIBALISM_MIN_SIZE_ADVANTAGE: i32 = 2;
// 血飛沫演出: 実機フィードバック(「もっと派手・グロテスクに強化してほしい」)を受けて、
// 単一の一瞬エフェクトから、複数粒子が散らばって尾を引くように少しずつ消える演出に強化した。
pub const BLOOD_EFFECT_LIFETIME: f64 = 1.6; // 表示時間(旧0.5秒→1〜2秒程度に延長)
pub const BLOOD_PARTICLE_COUNT: usize = 10; // 捕食1回あたりに散らす粒子数(旧: 1個のみ)
pub const BLOOD_SPREAD_RADIUS: f64 = 6.0; // 粒子が散らばる範囲(旧の波紋演出より広め)
// 血の滲み(範囲エフェクト): 捕食位置の周辺に赤みが水中に広がる演出。パーティクルより
// 長く残り、時間とともにゆっくりフェードアウトする(既存の水槽グラデーションに赤を
// 混ぜて表示するイメージ)。
// 実機フィードバック(「拡散が速すぎた」)を受けて、総表示時間を4.0→6.0秒に延ばし、
// 拡大にかける時間(BLOOD_STAIN_GROWTH_TIME)も総寿命の中でのウェイトを増やして
// もっとゆっくり広がる感じにした(拡大自体の実時間を伸ばす方向で調整)。
pub const BLOOD_STAIN_LIFETIME: f64 = 6.0;
// 実機フィードバック(「ぜんぜんグロテスクじゃない」)を受けて、固定半径のまま薄く
// フェードするだけの実装から、時間経過で半径が広がっていく同心円の波紋アニメーション
// に変更した。最大半径は旧定数(7.0)の約3倍・混色の強さも旧0.6→0.85に強化する
// (実際の拡大計算・混色計算は描画側=main.rsのdraw_species_dex付近で行う)。
pub const BLOOD_STAIN_MAX_RADIUS: f64 = 20.0;
// 半径が0→最大まで広がるのにかける実時間。BLOOD_STAIN_LIFETIME全体を使わず専用の
// タイマーにすることで、寿命を延ばさなくても拡大速度自体を遅くできるようにしている
// (寿命末期は最大半径のまま残り、フェードアウトだけが進む)。
pub const BLOOD_STAIN_GROWTH_TIME: f64 = 4.5;
// 実機フィードバック(「数秒間まっかに見えるくらい濃く」)を受けて0.85→0.93まで強化。
// はっきりインパクトのある赤にする(控えめにしない)。
pub const BLOOD_STAIN_MIX: f64 = 0.93;
// 発生から寿命のこの割合までは、広がりながらも混色の強さを最大近くで維持する
// (「数秒間まっか」に見せるための保持区間)。残りの区間でフェードアウトする。
pub const BLOOD_STAIN_HOLD_FRACTION: f64 = 0.5;
// 通常の魚が「今まさに捕食モードのピラニア」を検知して逃げる距離・強さ。
// 空腹でない/クールダウン中のピラニアは対象にならない(気にせず普段どおり泳ぐ)。
pub const PIRANHA_FEAR_RADIUS: f64 = 26.0;
// 実機フィードバック(「生き物の基本移動速度を4倍に」)を受けて4倍にした(旧90.0)。
pub const PIRANHA_FEAR_STRENGTH: f64 = 360.0;

// --- スター(無敵アイテム)。マリオのスターのようなギミック: 低頻度・ランダムな
// 位置に出現し、触れた魚は一定時間無敵化する。無敵中は誰からも捕食されず、逆に
// 種類に関わらず触れた他の魚(ピラニア・タコを含む)を捕食できる。 ---
// 観賞用ツールの体験を壊さない程度に控えめな頻度(平均8分に1回程度)にする。
// 同時に複数出したいギミックではないので、既に1個ある間は追加抽選しない。
pub const STAR_SPAWN_CHANCE_PER_SEC: f64 = 1.0 / 480.0;
pub const STAR_LIFETIME: f64 = 45.0; // 誰も取りに来ないまま経過すると消える
pub const STAR_PICKUP_RADIUS: f64 = 5.0; // 魚がスターに触れて取得できる距離
pub const STAR_INVINCIBLE_DURATION_MIN: f64 = 10.0;
pub const STAR_INVINCIBLE_DURATION_MAX: f64 = 20.0;
// 無敵中の捕食判定距離。既存の捕食者(ピラニア・タコ)の口基準判定と同じ考え方で、
// 通常種のスプライトにも十分届くよう広めに取る。
pub const STAR_STRIKE_RADIUS: f64 = 7.0;
// 無敵中に捕食したときの空腹度回復・クールダウン(通常種には空腹閾値による
// 狩りゲートが無いため、連続捕食を抑える目的でクールダウンを設ける)。
pub const STAR_PREDATION_HUNGER_GAIN: f64 = 40.0;
pub const STAR_PREDATION_COOLDOWN: f64 = 2.0;
// 描画側(main.rs)のキラキラ点滅・無敵中の発光点滅の速さ(rad/秒相当)
pub const STAR_TWINKLE_FREQ: f64 = 4.0;
pub const INVINCIBLE_BLINK_FREQ: f64 = 6.0;

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
// 産卵時、生まれた卵の位置に出るキラキラ演出の持続時間(実機フィードバック
// 「産卵時にキラキラ光るフラッシュ演出を追加してほしい。目安2〜3秒程度」)。
pub const SPAWN_FLASH_LIFETIME: f64 = 2.5;

// --- ガラスを叩く(t キー) ---
pub const KNOCK_RADIUS: f64 = 18.0; // この距離以内の魚が驚いて逃げる
pub const FLEE_DURATION: f64 = 1.2; // 逃走状態を維持する時間(秒)
// 実機フィードバック(「生き物の基本移動速度を4倍に」)を受けて4倍にした(旧140.0)。
pub const FLEE_STRENGTH: f64 = 560.0; // 逃走方向への加速の強さ
// ピラニアに驚いて逃げ続けている間、逃走状態を維持する最低時間(危険が去れば自然に減衰する)
pub const PIRANHA_FEAR_FLEE_MARK: f64 = 1.0;
// 逃走コスト: 逃げるのにエネルギーを使うという想定で、逃走が始まった瞬間(既に
// 逃走中でなければ)に空腹度を一定量消費する。ガラスの驚き逃げ・ピラニアからの逃走の
// どちらも同じ考え方で消費する。連打・長時間の張り込みで無限に減り続けないよう、
// 「既に逃走中は再課金しない」ことで1回の危険イベントにつき1回だけ課金する。
pub const FLEE_HUNGER_COST: f64 = 6.0;
// 回避動作を「回り込み」らしく見せるための横(垂直)方向の切り返し成分。
// 逃走方向に対して垂直な成分を時間で振動させ、真っ直ぐ離れるだけでなく
// ジグザグに切り返しながら回り込むような、生き物らしい動きにする。
pub const ZIGZAG_FREQ: f64 = 3.0; // 切り返しの速さ
pub const ZIGZAG_RATIO: f64 = 0.6; // 主となる逃走ベクトルに対する垂直成分の強さの比率

// --- ランダムな瞬発ダッシュ(特定のトリガーが無い通常時の躍動感演出) ---
// ピラニア・餌などのトリガーが無い普段の遊泳中でも、低頻度・ランダムなタイミングで
// 一瞬だけ通常より速く動く「ダッシュ」を行う。頻発すると落ち着きがなく見えるため、
// 数十秒に1回あるかないか程度の頻度に抑える。
pub const DASH_CHANCE_PER_SEC: f64 = 0.02; // 期待間隔=約50秒に1回
pub const DASH_DURATION: f64 = 0.35; // ダッシュ自体は一瞬だけ
// 実機フィードバック(「生き物の基本移動速度を4倍に」)を受けて4倍にした(旧160.0)。
pub const DASH_STRENGTH: f64 = 640.0; // ダッシュ方向への加速の強さ
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
    Blood, // ピラニアの捕食時の血飛沫(死亡演出=仰向け浮上とは別の専用演出)
    Spawn, // 産卵時、生まれた卵の位置に数秒間出るキラキラ演出
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

// 墨(タコが吐く): 血の滲みと同じ「同心円状に広がって薄れて消える」構造を再利用するが、
// もっと広め・勢いよく広がり、黒っぽい色で描く(main.rs側で専用の色・半径定数を使う)。
#[derive(Clone, Debug)]
pub struct InkCloud {
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
    Predation,   // ピラニア・タコが獲物を捕食した音
    Ink,         // タコが墨を吐いた音
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

// 藻・水草(装飾。静的な背景オブジェクトで育成ロジックには参加しない。揺れるだけ)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plant {
    pub x: f64,
    pub y: f64,      // 根元のy座標(水底基準)
    pub height: f64, // 見た目の高さ(バリエーション用)
    pub phase: f64,  // 揺れの位相(個体差を出すためのオフセット)
}

// タコつぼ(装飾+タコの巣)。水底に置く壺型の静的オブジェクト。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Den {
    pub x: f64,
    pub y: f64,
}

// 岩(装飾+隠れ場所)。水底に置く丸みのある岩塊の静的オブジェクト。藻・水草と同様に
// 近くにいる魚を視覚的に隠し、かつ実際に捕食対象から除外する「隠れ場所」として機能する。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rock {
    pub x: f64,
    pub y: f64, // 中心のy座標(底面が水底にちょうど乗るように計算して配置する)
}

// スター(無敵アイテム)。低頻度でランダムな位置に出現し、触れた魚を一定時間無敵化する。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Star {
    pub x: f64,
    pub y: f64,
    pub life: f64,     // 残り寿命(尽きると誰にも取られず消える)
    pub phase: f64,    // キラキラ演出用の位相(個体差を出すためのオフセット)
}

pub struct Simulation {
    pub fish: Vec<Fish>,
    pub food: Vec<Food>,
    pub medicine: Vec<Medicine>,
    pub eggs: Vec<Egg>,
    pub bubbles: Vec<Bubble>,
    pub crabs: Vec<Crab>,
    // 藻・水草・岩・タコつぼ(装飾。育成ロジック非対応の静的オブジェクト)
    pub plants: Vec<Plant>,
    pub rocks: Vec<Rock>,
    pub dens: Vec<Den>,
    // スター(無敵アイテム)。餌・薬と同様に寿命があるため保存対象にする。
    pub stars: Vec<Star>,
    // 投下エフェクト(一瞬で消えるので保存対象にしない)
    pub drop_effects: Vec<DropEffect>,
    // 血の滲み(範囲エフェクト。数秒で消えるので保存対象にしない)
    pub blood_stains: Vec<BloodStain>,
    // 墨(タコが吐く。数秒で消えるので保存対象にしない)
    pub ink_clouds: Vec<InkCloud>,
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
// 実機フィードバック(「魚のドット絵を大幅拡大してほしい(1.5〜2倍では不十分)」)を
// 受けて魚のスプライトを大幅に拡大したため、画面が窮屈にならないよう除数を
// 700→2500に上げて収容密度を下げた(サイズを妥協するのではなく上限側で調整する方針)。
// その後の実機フィードバック(「標準的な端末サイズで上限が11匹程度まで下がった。
// もう少し多く収容できるようにしてほしい」)を受けて、2500→1200まで下げて再調整した
// (目安1000〜1500程度の指示を受けて実機で確認しながら選定)。
pub fn capacity(pix_w: usize, pix_h: usize) -> usize {
    ((pix_w * pix_h) / 1200).clamp(5, 100)
}

// `+`キー(デバッグ追加)の上限。これ以上は産卵→孵化を経由してのみ個体数上限まで増やせる。
// スプライト拡大に伴い、上限自体も50→25に下げて画面の窮屈さを緩和した。
pub const ADD_FISH_MANUAL_CAP: usize = 25;

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
            plants: Vec::new(),
            rocks: Vec::new(),
            dens: Vec::new(),
            stars: Vec::new(),
            drop_effects: Vec::new(),
            blood_stains: Vec::new(),
            ink_clouds: Vec::new(),
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
            // 初期配置はピラニアを含めない通常3種のみ(ピラニアの入手経路はSキーのみに限定する方針)
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
    pub fn ensure_decorative_entities(&mut self, pix_w: usize, pix_h: usize) {
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

        let sand_top = (pix_h as f64 - sand_height(pix_h) as f64).max(2.0);

        if self.plants.is_empty() {
            for _ in 0..PLANT_COUNT {
                let x = self.rng.range(3.0, (pix_w as f64 - 3.0).max(3.0));
                self.plants.push(Plant {
                    x,
                    y: sand_top,
                    // 実機フィードバック(「藻を魚が隠れられるくらい大きく」)対応:
                    // 旧サイズ(3.0〜7.0)から、魚がすっぽり収まる大きさまで拡大した。
                    height: self.rng.range(6.0, 11.0),
                    phase: self.rng.range(0.0, std::f64::consts::TAU),
                });
            }
        }

        if self.rocks.is_empty() {
            // 岩の底面が水底にちょうど乗るように、スプライトの実高さの半分を使って
            // 中心Yを計算する(タコつぼと同じ考え方)。
            let rock_half_h = rock_sprite().height as f64 / 2.0;
            let rock_y = (sand_top - rock_half_h + 1.0).max(1.0);
            for _ in 0..ROCK_COUNT {
                let x = self.rng.range(4.0, (pix_w as f64 - 4.0).max(4.0));
                self.rocks.push(Rock { x, y: rock_y });
            }
        }

        if self.dens.is_empty() {
            // タコつぼを大きく描き直したため(実機フィードバック対応)、中心Yは
            // スプライトの実高さの半分を使って底面が水底にちょうど乗るように計算する
            // (固定オフセットのままだと大きなスプライトが水底に深く埋まって見えてしまう)。
            let den_half_h = den_sprite().height as f64 / 2.0;
            let y = (sand_top - den_half_h + 1.0).max(1.0);
            for _ in 0..DEN_COUNT {
                let x = self.rng.range(4.0, (pix_w as f64 - 4.0).max(4.0));
                self.dens.push(Den { x, y });
            }
        }

        // 仕様変更(「デフォルトでタコは入れない」): 以前はタコつぼの数だけ自動的に
        // タコを隠れた状態で常駐させていたが、初期状態(起動時・グレートリセット時)では
        // タコを一切配置しないようにした。タコつぼ自体は空の装飾として初期配置される。
        // タコを水槽に出す唯一の方法はOキー(add_octopus)のみにする(ピラニアのSキーと
        // 同じ方針)。
    }

    // 端末のフォントサイズ変更等でウィンドウの論理ピクセルサイズが変わったときに呼ぶ。
    // タコつぼ・水草は生成時の水底位置(sand_top)を基準にしたY座標を絶対値で保持
    // しているため、サイズ変更で水底の位置がずれても追従せず、装飾が床に沈んだり
    // 浮いて見えてしまう(実機フィードバック「文字サイズを変更したらタコツボとか
    // 草とかが床に沈殿する。逆になったら浮くようにして」)。ここで新しい水底位置に
    // 合わせてY座標(タコつぼはX座標も画面内に収まるよう)を再計算する。
    pub fn resync_seabed_decor(&mut self, pix_w: usize, pix_h: usize) {
        let sand_top = (pix_h as f64 - sand_height(pix_h) as f64).max(2.0);

        for p in &mut self.plants {
            p.x = p.x.clamp(3.0, (pix_w as f64 - 3.0).max(3.0));
            p.y = sand_top;
        }

        let rock_half_h = rock_sprite().height as f64 / 2.0;
        let new_rock_y = (sand_top - rock_half_h + 1.0).max(1.0);
        for r in &mut self.rocks {
            r.x = r.x.clamp(4.0, (pix_w as f64 - 4.0).max(4.0));
            r.y = new_rock_y;
        }

        let den_half_h = den_sprite().height as f64 / 2.0;
        let new_den_y = (sand_top - den_half_h + 1.0).max(1.0);
        for den in &mut self.dens {
            let old_x = den.x;
            let old_y = den.y;
            den.x = den.x.clamp(4.0, (pix_w as f64 - 4.0).max(4.0));
            den.y = new_den_y;

            // このタコつぼを巣にしているタコの位置情報も新しい座標へ追従させる
            // (隠れている間は den_x/den_y がそのまま描画位置になるため)
            for f in &mut self.fish {
                if f.species == Species::Octopus && f.den_x == old_x && f.den_y == old_y {
                    f.den_x = den.x;
                    f.den_y = den.y;
                    if f.hidden {
                        f.x = den.x;
                        f.y = den.y;
                    }
                }
            }
        }
    }

    // `D`キー: タコつぼを再配置する(既存を全て消し、同じ数だけ新しい位置に生成し直す)。
    // O キー等で増設した分も含めて現在の個数をそのまま維持する(DEN_COUNT固定にすると
    // 増設分が消えてしまうため)。既存のタコがいれば、旧タコつぼ(index対応)から
    // 新タコつぼへden_x/den_yを追従させる(resync_seabed_decorと同じ考え方)。
    pub fn reposition_dens(&mut self, pix_w: usize, pix_h: usize) {
        if self.dens.is_empty() {
            self.set_message("タコつぼがありません");
            return;
        }
        let count = self.dens.len();
        let old_dens = std::mem::take(&mut self.dens);

        let sand_top = (pix_h as f64 - sand_height(pix_h) as f64).max(2.0);
        let den_half_h = den_sprite().height as f64 / 2.0;
        let y = (sand_top - den_half_h + 1.0).max(1.0);
        for _ in 0..count {
            let x = self.rng.range(4.0, (pix_w as f64 - 4.0).max(4.0));
            self.dens.push(Den { x, y });
        }

        for (i, den) in self.dens.iter().enumerate() {
            if let Some(old) = old_dens.get(i) {
                for f in &mut self.fish {
                    if f.species == Species::Octopus && f.den_x == old.x && f.den_y == old.y {
                        f.den_x = den.x;
                        f.den_y = den.y;
                        if f.hidden {
                            f.x = den.x;
                            f.y = den.y;
                        }
                    }
                }
            }
        }
        self.set_message("タコつぼを再配置しました");
    }

    // `P`キー: 藻・水草を再配置する(既存を全て消し、同じ数だけ新しい位置に生成し直す)。
    // 育成ロジックに参加しない装飾のため、タコつぼのような対応関係の追従は不要。
    pub fn reposition_plants(&mut self, pix_w: usize, pix_h: usize) {
        if self.plants.is_empty() {
            self.set_message("藻・水草がありません");
            return;
        }
        let count = self.plants.len();
        self.plants.clear();
        let sand_top = (pix_h as f64 - sand_height(pix_h) as f64).max(2.0);
        for _ in 0..count {
            let x = self.rng.range(3.0, (pix_w as f64 - 3.0).max(3.0));
            self.plants.push(Plant {
                x,
                y: sand_top,
                height: self.rng.range(3.0, 7.0),
                phase: self.rng.range(0.0, std::f64::consts::TAU),
            });
        }
        self.set_message("藻・水草を再配置しました");
    }

    // グレートリセット: 魚を初期構成へ、卵・餌・薬・経過時間を消去
    pub fn reset(&mut self, pix_w: usize, pix_h: usize) {
        self.fish.clear();
        self.food.clear();
        self.medicine.clear();
        self.eggs.clear();
        self.bubbles.clear();
        self.stars.clear();
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

    // デバッグ: 魚を1匹追加。ADD_FISH_MANUAL_CAP(25匹)まで。
    // それ以上は産卵→孵化を経由してのみ個体数上限(端末サイズ依存・最大100)まで増やせる。
    // 死んで浮いている個体は数に入れない(居座りで詰まらせない)。
    // ランダム選択はピラニアを含まない通常3種のみ(ピラニアの入手経路はSキーのみに限定する方針)。
    pub fn add_fish(&mut self, pix_w: usize, pix_h: usize) {
        let sp = Species::COMMON[self.rng.range_usize(0, Species::COMMON.len() - 1)];
        self.add_fish_of_species(sp, pix_w, pix_h);
    }

    // `S`キー: 種類を指定して確実にその種を1匹追加する(ピラニアを狙って投入したい、という要望への対応)。
    // 上限(ADD_FISH_MANUAL_CAP・個体数上限)の扱いは add_fish と同じ。
    pub fn add_piranha(&mut self, pix_w: usize, pix_h: usize) {
        self.add_fish_of_species(Species::Piranha, pix_w, pix_h);
    }

    // `O`キー: タコを1匹、確実に水槽に投入する。タコは通常の`+`キー(ランダム追加)の
    // 対象には含まれない特殊入手種のため、ピラニアの`S`キーと同様の専用ショートカット
    // を用意する。上限(ADD_FISH_MANUAL_CAP・個体数上限)の扱いはadd_fish/add_piranhaと
    // 同じ。新しいタコつぼも1つ増設し、そこを専用の巣にする(投入直後は見える状態に
    // して手応えが分かるようにし、しばらくしたら通常のタコと同様に自分で巣へ戻る)。
    pub fn add_octopus(&mut self, pix_w: usize, pix_h: usize) {
        if self.fish.len() >= ADD_FISH_MANUAL_CAP {
            self.set_message("これ以上は孵化でしか増えません");
            return;
        }
        if self.living_count() >= capacity(pix_w, pix_h) {
            self.set_message("水槽が満員です");
            return;
        }

        // 仕様変更(「デフォルトでタコは入れない」): 初期配置のタコつぼは空の装飾として
        // 生成されるため、Oキーで追加するタコは、まず空いている(まだタコが紐づいて
        // いない)タコつぼを探して、そこに住まわせる。空きが無ければ新しいタコつぼを
        // 1つ増設する。
        let occupied_dens: Vec<(f64, f64)> = self
            .fish
            .iter()
            .filter(|f| f.species == Species::Octopus)
            .map(|f| (f.den_x, f.den_y))
            .collect();
        let empty_den = self
            .dens
            .iter()
            .find(|d| !occupied_dens.iter().any(|&(dx, dy)| dx == d.x && dy == d.y))
            .cloned();

        let (den_x, den_y) = if let Some(den) = empty_den {
            (den.x, den.y)
        } else {
            let sand_top = (pix_h as f64 - sand_height(pix_h) as f64).max(2.0);
            let den_half_h = den_sprite().height as f64 / 2.0;
            let x = self.rng.range(4.0, (pix_w as f64 - 4.0).max(4.0));
            let y = (sand_top - den_half_h + 1.0).max(1.0);
            self.dens.push(Den { x, y });
            (x, y)
        };

        let mut octo = Fish::new(Species::Octopus, Stage::Fry, den_x, den_y);
        octo.hidden = false; // 投入直後は見える状態にする
        octo.den_x = den_x;
        octo.den_y = den_y;
        octo.hidden_timer = self.rng.range(OCTOPUS_EMERGE_TIME_MIN, OCTOPUS_EMERGE_TIME_MAX);
        self.fish.push(octo);
        self.set_message("タコを1匹投入しました");
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

    // 間引き: 通常の魚(ピラニア含む)がいればそれを1匹減らす。通常の魚が0匹になったら
    // カニを1匹減らす(観賞用生物だけ残って「0にしたのに何か残ってる」と
    // 分かりづらくならないようにするフォールバック)。追加(add_fish)は通常魚のみのまま。
    pub fn remove_fish(&mut self) {
        if !self.fish.is_empty() {
            self.fish.pop();
            // 実機フィードバック対応: タコが通常のfish扱いで間引かれて0匹になった
            // ときに、対応するタコつぼ(dens)だけが空のまま取り残されると不自然
            // なので、一緒に消す。
            if !self.fish.iter().any(|f| f.species == Species::Octopus) {
                self.dens.clear();
            }
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

        self.update_octopus(dt);
        self.update_movement(dt, pix_w as f64, sand_top);
        self.update_food(dt, sand_top);
        self.update_medicine(dt, sand_top);
        self.update_stars(dt, pix_w as f64, sand_top);
        self.update_biology(dt, cap, pix_w as f64, sand_top);
        self.update_predation(dt);
        self.update_crabs(dt, pix_w as f64);
        self.update_bubbles(dt, pix_w as f64, pix_h as f64);
        self.update_effects(dt);
    }

    // タコの隠れる/出てくる状態遷移。低頻度でつぼから出てきて泳ぎ、しばらくしたら戻る
    // (隠れている間は巣に留まり、非表示・移動しない・捕食対象にもならない)。
    // 加えて、近くに捕食モードのピラニアがいれば(「追われている」とみなし)墨を吐く。
    fn update_octopus(&mut self, dt: f64) {
        // ピラニアの位置・捕食モード判定用スナップショット(墨のトリガー判定に使う)
        let piranhas: Vec<(f64, f64, bool)> = self
            .fish
            .iter()
            .filter(|f| f.species == Species::Piranha && !f.dead)
            .map(|f| {
                (
                    f.x,
                    f.y,
                    f.hunger < PIRANHA_HUNT_HUNGER_THRESHOLD && f.predation_cooldown <= 0.0,
                )
            })
            .collect();

        for f in &mut self.fish {
            if f.species != Species::Octopus || f.dead {
                continue;
            }
            f.hidden_timer = (f.hidden_timer - dt).max(0.0);
            f.ink_cooldown = (f.ink_cooldown - dt).max(0.0);

            if f.hidden {
                // 隠れている間は巣にとどまる(非表示・移動なし)
                f.x = f.den_x;
                f.y = f.den_y;
                f.vx = 0.0;
                f.vy = 0.0;
                if f.hidden_timer <= 0.0 {
                    f.hidden = false;
                    f.hidden_timer =
                        self.rng.range(OCTOPUS_EMERGE_TIME_MIN, OCTOPUS_EMERGE_TIME_MAX);
                }
            } else if f.hidden_timer <= 0.0 {
                // 出ている時間が終わったら巣へ戻って隠れる(update_movement側の巣への
                // 引力で近づいていても、時間切れの瞬間に確実に隠れさせる)
                f.hidden = true;
                f.x = f.den_x;
                f.y = f.den_y;
                f.vx = 0.0;
                f.vy = 0.0;
                f.hidden_timer = self.rng.range(OCTOPUS_HIDDEN_TIME_MIN, OCTOPUS_HIDDEN_TIME_MAX);
            }

            // 墨: 出ている間、近くに捕食モードのピラニアがいたら(追われているとみなし)吐く
            if !f.hidden && f.ink_cooldown <= 0.0 {
                let threatened = piranhas.iter().any(|&(sx, sy, hunting)| {
                    hunting
                        && ((sx - f.x).powi(2) + (sy - f.y).powi(2)).sqrt()
                            < OCTOPUS_INK_TRIGGER_RADIUS
                });
                if threatened {
                    self.ink_clouds.push(InkCloud {
                        x: f.x,
                        y: f.y,
                        life: INK_LIFETIME,
                        max_life: INK_LIFETIME,
                    });
                    self.sound_events.push(SfxEvent::Ink);
                    f.ink_cooldown = INK_COOLDOWN;
                    // 「墨を吐いたら高確率で逃げ切れる」を結果として保証するため、
                    // 緊急ダッシュ(速度・逃走ベクトル強化)と捕食判定からの一時除外を付与する。
                    f.ink_escape_timer = INK_ESCAPE_DURATION;
                    self.message = Some("タコが墨を吐いて逃げた!".to_string());
                    self.message_ttl = 4.0;
                }
            }
        }
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
        for c in &mut self.ink_clouds {
            c.life -= dt;
        }
        self.ink_clouds.retain(|c| c.life > 0.0);
    }

    // 遊泳: ランダムウォーク+慣性+壁反射+群れ+餌吸引(空腹度・病気で速度が変化)。
    // 死亡演出中の個体はここでは動かさず、水面近くまでゆっくり浮上して静止するだけにする。
    fn update_movement(&mut self, dt: f64, w: f64, sand_top: f64) {
        // 群れ計算のため位置・速度をスナップショット(self.fish とインデックスを揃えるため
        // 死亡個体もそのまま含め、死亡フラグで群れ対象から除外する)
        // hunger/predation_cooldown も持たせて、他の魚が「近くのピラニアが今まさに
        // 捕食モードかどうか」を判定できるようにする(逃走ベクトルの判定に使う)。
        let snap: Vec<(Species, f64, f64, f64, f64, bool, f64, f64, bool, u8, u8, bool, bool)> = self
            .fish
            .iter()
            .map(|f| {
                (
                    f.species,
                    f.x,
                    f.y,
                    f.vx,
                    f.vy,
                    f.dead,
                    f.hunger,
                    f.predation_cooldown,
                    f.hidden, // タコが隠れている間は捕食対象・逃走対象にしない
                    f.growth_stage, // ピラニアの共食い(サイズ差判定)に使う
                    f.kill_stage,
                    f.is_invincible(), // 無敵中の魚は誰からも捕食対象・追跡対象にしない
                    self.is_hidden_in_cover(f.x, f.y), // 藻・水草・岩に隠れている魚も同様
                )
            })
            .collect();

        let margin: f64 = 4.0;
        let top_margin: f64 = 3.0;
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
            if self.fish[i].species == Species::Octopus && self.fish[i].hidden {
                // 隠れている間は update_octopus() 側で巣の位置に固定済み。ここでは動かさない。
                continue;
            }
            let (
                sp,
                hunger,
                fx,
                fy,
                spd_mult,
                agility,
                hungry,
                flee_timer,
                flee_dx,
                flee_dy,
                predation_cooldown,
                dash_timer,
                dash_dx,
                dash_dy,
                den_x,
                den_y,
                hidden_timer,
                ink_escape_timer,
                growth_stage_self,
                kill_stage_self,
                is_invincible_self,
                half_w,
                half_h,
            ) = {
                let f = &self.fish[i];
                let sprite = f.sprite();
                let scale = f.render_scale();
                (
                    f.species,
                    f.hunger,
                    f.x,
                    f.y,
                    f.speed_mult(),
                    f.agility_mult(),
                    f.hunger < HUNGRY_THRESHOLD,
                    f.flee_timer,
                    f.flee_dx,
                    f.flee_dy,
                    f.predation_cooldown,
                    f.dash_timer,
                    f.dash_dx,
                    f.dash_dy,
                    f.den_x,
                    f.den_y,
                    f.hidden_timer,
                    f.ink_escape_timer,
                    f.growth_stage, // ピラニアの共食い(サイズ差判定)に使う
                    f.kill_stage,
                    f.is_invincible(), // 無敵中は同種・タコ対ピラニア等の通常ルールを無視して追跡できる
                    // 実機フィードバック(「魚が水底に張り付いて見える」)対応: 成長段階で
                    // 拡大されたスプライトの実サイズ(render_scale倍後)の半分を、壁際の
                    // 可動範囲マージンとして使う。固定値のままだと大きく育った魚の中心座標が
                    // 水底ぎりぎりまで許容されてしまい、拡大後のスプライト下半分が水底に
                    // 埋まって描画されてしまう。
                    (sprite.width as f64 * scale) / 2.0,
                    (sprite.height as f64 * scale) / 2.0,
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

            // 捕食者(ピラニア・タコ)ごとの狩りパラメータ。タコもピラニアと同じ「頻繁に狩る」
            // 方針を共有しつつ、専用の定数で独立に調整できるようにしている。
            let (hunt_threshold, hunt_radius, hunt_pull) = match sp {
                Species::Piranha => (PIRANHA_HUNT_HUNGER_THRESHOLD, PIRANHA_HUNT_RADIUS, PIRANHA_HUNT_PULL),
                Species::Octopus => (
                    OCTOPUS_HUNT_HUNGER_THRESHOLD,
                    OCTOPUS_HUNT_RADIUS,
                    OCTOPUS_HUNT_PULL,
                ),
                _ => (0.0, 0.0, 0.0), // 非捕食者では使わない(is_predator()でガードされる)
            };
            // 墨が近くに広がっている間、捕食者は獲物を検知できない(「視界が悪くなる」演出)。
            // 描画側のアニメーション曲線とは独立に、ゲームロジック側はINK_MAX_RADIUS基準で
            // シンプルに判定する。
            let blinded_by_ink = sp.is_predator()
                && self
                    .ink_clouds
                    .iter()
                    .any(|c| ((c.x - fx).powi(2) + (c.y - fy).powi(2)).sqrt() < INK_MAX_RADIUS);
            // 捕食者の狩り: 空腹度が閾値未満・クールダウン明けなら、近くの獲物を先に探しておく
            // (自分と同種・ピラニア同士・タコからピラニアは対象外。タコが隠れている間も対象外)。
            // 追いかけている間は通常の遊泳を弱め、吸引ベクトルをはっきり優先させる。
            let chase_target = if sp.is_predator() && !blinded_by_ink && hunger < hunt_threshold && predation_cooldown <= 0.0 {
                let mut best = f64::INFINITY;
                let mut best_pos = None;
                for (
                    j,
                    &(
                        psp,
                        px,
                        py,
                        _pvx,
                        _pvy,
                        pdead,
                        _phunger,
                        _pcooldown,
                        phidden,
                        pgrowth,
                        pkill,
                        pinvincible,
                        pcover,
                    ),
                ) in snap.iter().enumerate()
                {
                    if is_excluded_as_prey(
                        sp,
                        growth_stage_self,
                        kill_stage_self,
                        is_invincible_self,
                        i,
                        j,
                        psp,
                        pdead,
                        phidden,
                        pinvincible,
                        pcover,
                        pgrowth,
                        pkill,
                    ) {
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
                    .filter(|&(_, dist)| dist < hunt_radius)
            } else {
                None
            };
            // 追跡中かどうか(捕食モードで獲物を追っている間だけ、後段で最高速度を
            // 通常3種よりはっきり速くブーストする)
            let is_chasing = chase_target.is_some();

            // 被食者側の警戒: 近くにピラニアがいたら常に検知する(方針変更: 「みんなピラニアが
            // 嫌い」という設定にするため、ピラニアが捕食モードかどうかは問わない)。
            // タコもピラニアに襲われる対象なので、ピラニア自身以外は全種がこの警戒に参加する。
            // スター(無敵アイテム)取得中は、逆にピラニアを捕食できる側になるため怖がらない。
            let fear_target = if sp != Species::Piranha && !is_invincible_self {
                let mut best = f64::INFINITY;
                let mut best_pos = None;
                for &(
                    psp,
                    px,
                    py,
                    _pvx,
                    _pvy,
                    pdead,
                    _phunger,
                    _pcooldown,
                    _phidden,
                    _pgrowth,
                    _pkill,
                    _pinvincible,
                    _pcover,
                ) in snap.iter()
                {
                    if psp != Species::Piranha || pdead {
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
                    .filter(|&(_, dist)| dist < PIRANHA_FEAR_RADIUS)
            } else {
                None
            };
            let is_fleeing_piranha = fear_target.is_some();

            // 餌を追っている・獲物を追っている・ピラニアから逃げている、のいずれかの間は
            // 通常の遊泳(ランダムウォーク・群れ)を大きく弱め、該当のベクトルを
            // はっきり優先させる(「一直線に向かう/逃げる」のが見た目でわかるように)。
            let normal_move_mix = if nearest_food.is_some() || chase_target.is_some() || is_fleeing_piranha {
                HUNGRY_NORMAL_MOVE_DAMP
            } else {
                1.0
            };

            // ランダムウォーク(縦は控えめ)。空腹度・病気に応じて活発さが変わるほか、
            // サイズが小さいほど(稚魚・成長段階が低いほど)agilityが1.0を超えて強くなり、
            // 大きいほど1.0未満になって弱まる(通常の遊泳だけに効かせる)
            ax += self.rng.signed() * sp.wander() * spd_mult * agility * normal_move_mix;
            ay += self.rng.signed() * sp.wander() * 0.55 * spd_mult * agility * normal_move_mix;

            // 群れ: 同種近傍の平均速度に少し寄せる(死亡個体は対象外)
            let (mut svx, mut svy, mut cnt) = (0.0, 0.0, 0);
            for (
                j,
                &(
                    osp,
                    ox,
                    oy,
                    ovx,
                    ovy,
                    odead,
                    _ohunger,
                    _ocooldown,
                    _ohidden,
                    _ogrowth,
                    _okill,
                    _oinvincible,
                    _ocover,
                ),
            ) in snap.iter().enumerate()
            {
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

            // 捕食者の狩り: 探しておいた獲物へ向かって強く近づく(実際の捕食判定は
            // update_predation 側で行う)。通常の遊泳は上で damp 済みなので、
            // この吸引ベクトルが「追いかけている」動きとしてはっきり見えるようにする。
            if let Some(((bx, by), dist)) = chase_target {
                ax += (bx - fx) / dist * hunt_pull;
                ay += (by - fy) / dist * hunt_pull;
            }

            // タコ: 出ている残り時間が少なくなったら巣へ戻る引力をかける(実際の
            // 「時間切れで確実に隠れる」処理は update_octopus() 側で保証している。
            // ここでは戻っていく様子が見えるようにするだけ)。
            if sp == Species::Octopus && hidden_timer < OCTOPUS_RETURN_WINDOW {
                let ddx = den_x - fx;
                let ddy = den_y - fy;
                let ddist = (ddx * ddx + ddy * ddy).sqrt().max(0.001);
                ax += ddx / ddist * OCTOPUS_RETURN_PULL;
                ay += ddy / ddist * OCTOPUS_RETURN_PULL;
            }

            // 被食者側の逃走: 検知済みのピラニアから離れる方向へ強く加速する。通常の遊泳は
            // 上で damp 済み・最高速度も後段でブーストするため、「パッと反応して素早く
            // 逃げる」機敏な動きになる(ぬるっと逃げる感じにはならない)。真っ直ぐ離れる
            // だけでなく、垂直成分を振動させてジグザグに回り込むような動きを混ぜる。
            if let Some(((sx, sy), raw_dist)) = fear_target {
                let dist = raw_dist.max(0.001);
                // 距離が近いほど強く効かせる。墨を吐いた直後(ink_escape_timer)は
                // タコの緊急ダッシュとしてさらに強める(「墨を吐いたら逃げ切れる」を
                // 結果として保証するための後押し)。
                let escape_boost = if ink_escape_timer > 0.0 {
                    INK_ESCAPE_STRENGTH_MULT
                } else {
                    1.0
                };
                let strength =
                    PIRANHA_FEAR_STRENGTH * (1.0 - raw_dist / PIRANHA_FEAR_RADIUS).max(0.0) * escape_boost;
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

            // 壁の手前で緩やかに向きを変える(反射)。マージンはスプライトの実サイズ
            // (半分)を下限として広げ、大きく育った魚でも壁際で不自然に窮屈にならない
            // ようにする(固定値のままだと拡大後の見た目が壁・水底に埋まってしまう)。
            let size_x_margin = margin.max(half_w + 1.0);
            let size_top_margin = top_margin.max(half_h + 1.0);
            let size_bottom_margin = 1.0f64.max(half_h + 1.0);
            // 実機フィードバック(「壁際に追い詰めた魚を永遠に捕食できない」)対応:
            // 追跡中(捕食者側)・逃走中(被食者側、ピラニアからの逃走・ガラスの驚き逃げ)は
            // 反発力を無効化するだけでなく、マージン自体も基本値(サイズ非依存)に戻す。
            // サイズ基準マージンのままだと、捕食者と被食者でスプライトの大きさが違う場合に
            // 壁際・角で「自分の取れる位置」の余白が個体ごとに変わってしまい、大きい捕食者
            // だけが十分壁・角に近づけず、獲物との間に絶対に詰め切れない隙間が残ってしまう
            // (実際に発生した根本原因: 追跡・逃走中もサイズ基準マージンのままだったため)。
            // 追跡・逃走中だけ基本マージンに揃えることで、サイズに関わらず同じだけ
            // 壁・角へ詰められるようにする。壁の外に出ないこと自体は後段の位置クランプ
            // (ハード上限。ここで決めたマージンをそのまま使う)が保証する。
            let wall_push_suppressed =
                is_chasing || is_fleeing_piranha || flee_timer > 0.0 || ink_escape_timer > 0.0;
            let x_margin = if wall_push_suppressed { margin } else { size_x_margin };
            let top_edge_margin = if wall_push_suppressed { top_margin } else { size_top_margin };
            let bottom_edge_margin = if wall_push_suppressed { 1.0 } else { size_bottom_margin };
            let effective_wall_push = if wall_push_suppressed { 0.0 } else { wall_push };
            if fx < x_margin {
                ax += effective_wall_push;
            } else if fx > w - x_margin {
                ax -= effective_wall_push;
            }
            if fy < top_edge_margin {
                ay += effective_wall_push;
            } else if fy > sand_top - bottom_edge_margin {
                ay -= effective_wall_push;
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
            // ガラスの驚き逃げ・ピラニアからの逃走のいずれかの間は、最高速度を一時的に
            // 上げて「パッと反応して素早く逃げる」機敏な動きにする(鈍くしない)。
            let is_fleeing = is_knock_fleeing || is_fleeing_piranha;

            // ランダムな瞬発ダッシュ: ピラニア・餌などのトリガーが無い「通常時」だけ、
            // 低頻度・ランダムなタイミングで一瞬だけ速く動く演出を入れる(躍動感)。
            // 既に他の強い意図(餌を追う・追跡する・逃げる)がある間は割り込まない。
            let normal_state =
                nearest_food.is_none() && chase_target.is_none() && !is_fleeing_piranha && !is_knock_fleeing;
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
            // 慣性(ドラッグ)。逃走中・ダッシュ中・追跡中はドラッグ(ブレーキ)も弱めて
            // 反応を鈍らせない。実機フィードバック(「追跡中も速さが体感できない」)対応:
            // 最高速度のクランプ(PIRANHA_CHASE_SPEED_MULT)だけでは、方向転換や短い追跡の
            // 間に最高速度へ到達しきれず速度差が体感しにくかったため、追跡中はブレーキも
            // 弱めて加速がしっかり速度に乗るようにする。通常時は、サイズが小さいほど
            // (agility>1.0)ブレーキも強く効いて速度がすぐ収まる=キビキビ方向転換に
            // 見える。大きいほど(agility<1.0)ブレーキが弱まり、惰性で滑るような
            // ゆったりした動きになる。
            let drag_rate = if is_fleeing || is_dashing || is_chasing || ink_escape_timer > 0.0 {
                0.5
            } else {
                0.9 * agility
            };
            let drag = (1.0 - drag_rate * dt).clamp(0.0, 1.0);
            f.vx *= drag;
            f.vy *= drag;
            // 最高速度でクランプ(空腹度・病気で上限が変わる。逃走中は一時的に速く泳げる。
            // ピラニアは追跡中だけ通常3種よりはっきり速くなるようブーストする。大きく育つほど
            // わずかに遅くなる(size_speed_mult、必須ではない体感の変化)。ランダムダッシュ中も
            // 一瞬だけ最高速度が上がる。
            let speed = (f.vx * f.vx + f.vy * f.vy).sqrt();
            // 追跡中は「満腹による減速」で緊迫感が薄れないよう、spd_multの下限を1.0にする
            // (満腹ぎりぎりでも捕食モードに入れるようPIRANHA_HUNT_HUNGER_THRESHOLDを上げた
            // ことで、満腹減速(0.72倍)域と捕食モード域が重なりやすくなったための調整。
            // 腹ぺこによる加速(1.3倍)はそのまま乗る=より飢えているほど速く追う)
            let chase_spd_mult = if is_chasing { spd_mult.max(1.0) } else { spd_mult };
            let maxs = sp.max_speed()
                * chase_spd_mult
                * f.size_speed_mult()
                * if is_fleeing { 1.6 } else { 1.0 }
                * if is_chasing { PIRANHA_CHASE_SPEED_MULT } else { 1.0 }
                * if is_dashing { DASH_SPEED_MULT } else { 1.0 }
                * if ink_escape_timer > 0.0 { INK_ESCAPE_SPEED_MULT } else { 1.0 };
            if speed > maxs {
                f.vx = f.vx / speed * maxs;
                f.vy = f.vy / speed * maxs;
            }
            // 逃走タイマーを進める。ピラニアに驚いている間は、危険が続く限り逃走状態を
            // 維持する(逃走開始の瞬間だけ空腹度コストを課金し、居座られても再課金しない)。
            if is_fleeing_piranha {
                if f.flee_timer <= 0.0 {
                    f.hunger = (f.hunger - FLEE_HUNGER_COST).max(0.0);
                }
                f.flee_timer = f.flee_timer.max(PIRANHA_FEAR_FLEE_MARK);
            } else {
                f.flee_timer = (f.flee_timer - dt).max(0.0);
            }
            // 墨を吐いた直後の緊急脱出時間を減らす
            f.ink_escape_timer = (f.ink_escape_timer - dt).max(0.0);
            // 積分
            f.x += f.vx * dt;
            f.y += f.vy * dt;
            // 位置クランプ: スプライトの実サイズ(半分)を下限マージンにして、拡大後の
            // 見た目が壁・水底に埋まらないようにする(サイズ拡大と可動範囲拡張はセットで
            // 直す。固定値のままだと大きく育った魚ほど水底に張り付いて見えてしまう)。
            let x_bound = safe_upper((w - x_margin).max(x_margin));
            let y_bottom_bound = safe_upper((sand_top - bottom_edge_margin).max(top_edge_margin));
            f.x = f.x.clamp(x_margin.min(x_bound), x_bound);
            f.y = f.y.clamp(top_edge_margin.min(y_bottom_bound), y_bottom_bound);
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

    // スター(無敵アイテム): 低頻度でランダムな位置に出現し、寿命が尽きると誰にも
    // 取られず消える。触れた魚(通常種・ピラニア・タコいずれでも)は一定時間無敵化する。
    fn update_stars(&mut self, dt: f64, w: f64, sand_top: f64) {
        // 既にスターが出ている間は追加抽選しない(同時に複数出す演出ではないため)
        if self.stars.is_empty() && self.rng.next_f64() < STAR_SPAWN_CHANCE_PER_SEC * dt {
            let x = self.rng.range(4.0, (w - 4.0).max(4.0));
            let y = self.rng.range(3.0, (sand_top - 3.0).max(3.0));
            self.stars.push(Star {
                x,
                y,
                life: STAR_LIFETIME,
                phase: self.rng.range(0.0, std::f64::consts::TAU),
            });
        }

        for s in &mut self.stars {
            s.life -= dt;
        }
        self.stars.retain(|s| s.life > 0.0);

        // 無敵タイマーの減衰(全種共通。通常はスター取得時のみ0より大きくなる)
        for f in &mut self.fish {
            if f.invincible_timer > 0.0 {
                f.invincible_timer = (f.invincible_timer - dt).max(0.0);
            }
        }

        if self.stars.is_empty() {
            return;
        }
        // 1tickで1匹だけ取得できるようにする(複数匹が同時に群がって取り合うのは
        // 不自然なため。餌・薬と同じ「1tickで1つまで」の考え方)
        let mut best_dist = f64::INFINITY;
        let mut best_fi: Option<usize> = None;
        let mut best_si: Option<usize> = None;
        for (fi, f) in self.fish.iter().enumerate() {
            if f.dead || (f.species == Species::Octopus && f.hidden) {
                continue;
            }
            for (si, s) in self.stars.iter().enumerate() {
                let d = ((f.x - s.x).powi(2) + (f.y - s.y).powi(2)).sqrt();
                if d < STAR_PICKUP_RADIUS && d < best_dist {
                    best_dist = d;
                    best_fi = Some(fi);
                    best_si = Some(si);
                }
            }
        }
        if let (Some(fi), Some(si)) = (best_fi, best_si) {
            self.stars.remove(si);
            let duration = self.rng.range(STAR_INVINCIBLE_DURATION_MIN, STAR_INVINCIBLE_DURATION_MAX);
            self.fish[fi].invincible_timer = duration;
            self.set_message("スターを取得!無敵状態になった");
        }
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
            // 一度きりの確定イベント。ピラニアは対象外=`S`キー以外で増えない方針のため)。
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
                // ピラニアは産卵しない(ピラニアを増やす唯一の方法はSキーにする方針のため)。
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
            // 老衰(LIFESPAN_DEATH_AGE)も同じ死亡演出に乗せる(全種共通・ピラニアも対象)。
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
            let flash_y = (sand_top - 1.5).max(1.0);
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
            // 実機フィードバック(「産卵時にキラキラ光るフラッシュ演出を追加してほしい」)
            // 対応: 生まれた卵のクラスタの位置に、数秒間キラキラ光る演出を出す。
            self.drop_effects.push(DropEffect {
                x: px,
                y: flash_y,
                life: SPAWN_FLASH_LIFETIME,
                max_life: SPAWN_FLASH_LIFETIME,
                kind: EffectKind::Spawn,
            });
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
    // ピラニアの捕食: 空腹度が閾値未満・クールダウン明けのピラニアが、最も近い獲物(ピラニア以外・
    // 生存個体)が捕食圏内(PIRANHA_STRIKE_RADIUS)にいれば捕食する。頻度を抑えるため
    // 空腹度条件とクールダウンの両方を課す(四六時中は狙わせない)。
    // 藻・水草・岩に十分近いか(隠れているとみなす距離内か)を判定する。実機フィードバック
    // (「隠れたら実際に捕食されなくなる機能化」)対応: 従来は見た目だけの演出だったが、
    // タコがタコつぼに隠れている間は誰からも捕食対象にならないのと同じ考え方を、
    // 藻・水草・岩の近くにいる魚にも適用する(捕食判定・追跡判定の両方から使う)。
    fn is_hidden_in_cover(&self, x: f64, y: f64) -> bool {
        self.plants
            .iter()
            .any(|p| ((p.x - x).powi(2) + (p.y - y).powi(2)).sqrt() < ALGAE_HIDE_RADIUS)
            || self
                .rocks
                .iter()
                .any(|r| ((r.x - x).powi(2) + (r.y - y).powi(2)).sqrt() < ALGAE_HIDE_RADIUS)
    }

    // 1tickにつき捕食は最大1件(複数ピラニアの同時捕食による index shift の複雑化を避けるため)。
    fn update_predation(&mut self, dt: f64) {
        // クールダウンを進める(捕食者=ピラニア・タコ、およびスターで無敵中の
        // 一時的な捕食者はこれを使う。他の魚は常に0のまま無害)
        for f in &mut self.fish {
            if f.predation_cooldown > 0.0 {
                f.predation_cooldown = (f.predation_cooldown - dt).max(0.0);
            }
        }

        // 位置・種・生死・隠れ状態(タコつぼ/藻・水草・岩)・墨の緊急脱出状態・成長段階・
        // 無敵状態のスナップショット(self.fish とインデックスを揃える。成長段階は
        // ピラニアの共食いのサイズ差判定に使う)
        let snapshot: Vec<(Species, f64, f64, bool, bool, f64, u8, u8, bool, bool)> = self
            .fish
            .iter()
            .map(|f| {
                (
                    f.species,
                    f.x,
                    f.y,
                    f.dead,
                    f.hidden,
                    f.ink_escape_timer,
                    f.growth_stage,
                    f.kill_stage,
                    f.is_invincible(),
                    self.is_hidden_in_cover(f.x, f.y),
                )
            })
            .collect();

        let mut prey_index: Option<usize> = None;
        let mut predator_index: Option<usize> = None;

        'outer: for (i, f) in self.fish.iter().enumerate() {
            // スター(無敵アイテム)取得中は、通常種でも一時的に捕食者になれる
            // (「普段は捕食されない側の魚が、この間だけ逆転して捕食者になれる」ギミック)。
            let is_temp_predator = f.is_invincible();
            if (!f.species.is_predator() && !is_temp_predator) || f.dead || f.predation_cooldown > 0.0 {
                continue;
            }
            if f.species == Species::Octopus && f.hidden {
                continue; // タコは隠れている間は捕食しない
            }
            // 無敵中は空腹度による狩りゲートを無視して、触れたら誰でも捕食できる。
            let strike_radius = match f.species {
                Species::Piranha => {
                    if f.hunger >= PIRANHA_HUNT_HUNGER_THRESHOLD && !is_temp_predator {
                        continue;
                    }
                    PIRANHA_STRIKE_RADIUS
                }
                Species::Octopus => {
                    if f.hunger >= OCTOPUS_HUNT_HUNGER_THRESHOLD && !is_temp_predator {
                        continue;
                    }
                    OCTOPUS_STRIKE_RADIUS
                }
                _ => {
                    if !is_temp_predator {
                        continue;
                    }
                    STAR_STRIKE_RADIUS
                }
            };
            // 実機フィードバック(「あたり判定を胴体でなく口に」)対応: 捕食判定は
            // 中心(胴体)ではなく、進行方向側のスプライト前端=口の位置を基準にする。
            let (mouth_x, mouth_y) = f.mouth_position();
            let mut best_dist = f64::INFINITY;
            let mut best_j = None;
            for (
                j,
                &(psp, px, py, pdead, phidden, p_ink_escape, p_growth, p_kill, p_invincible, p_cover),
            ) in snapshot.iter().enumerate()
            {
                if is_excluded_as_prey(
                    f.species,
                    f.growth_stage,
                    f.kill_stage,
                    is_temp_predator,
                    i,
                    j,
                    psp,
                    pdead,
                    phidden,
                    p_invincible,
                    p_cover,
                    p_growth,
                    p_kill,
                ) {
                    continue;
                }
                if p_ink_escape > 0.0 {
                    continue; // 墨を吐いた直後は捕食判定(strike radius)から一時的に除外
                }
                let d = ((px - mouth_x).powi(2) + (py - mouth_y).powi(2)).sqrt();
                if d < best_dist {
                    best_dist = d;
                    best_j = Some(j);
                }
            }
            if let Some(j) = best_j {
                if best_dist < strike_radius {
                    predator_index = Some(i);
                    prey_index = Some(j);
                    break 'outer;
                }
            }
        }

        if let (Some(si), Some(pi)) = (predator_index, prey_index) {
            let predator_species = self.fish[si].species;
            let prey_species = self.fish[pi].species;
            let prey_x = self.fish[pi].x;
            let prey_y = self.fish[pi].y;
            let (gain, cooldown) = match predator_species {
                Species::Piranha => (PIRANHA_PREDATION_HUNGER_GAIN, PIRANHA_HUNT_COOLDOWN),
                Species::Octopus => (OCTOPUS_PREDATION_HUNGER_GAIN, OCTOPUS_HUNT_COOLDOWN),
                // 無敵中の通常種による一時的な捕食(スターギミック)
                _ => (STAR_PREDATION_HUNGER_GAIN, STAR_PREDATION_COOLDOWN),
            };
            // 捕食者の空腹度を回復し、クールダウンを設定(先に捕食者側を更新してから
            // 獲物を除去する。除去でインデックスがずれても si には影響しないようにするため)
            self.fish[si].hunger = (self.fish[si].hunger + gain).min(MAX_HUNGER);
            self.fish[si].predation_cooldown = cooldown;
            // ピラニアは捕食するたびに段階的に大きくなる(上限 PIRANHA_MAX_KILL_STAGE で打ち止め。
            // タコはこの成長ボーナスの対象外)
            if predator_species == Species::Piranha && self.fish[si].kill_stage < PIRANHA_MAX_KILL_STAGE {
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

// 捕食者(predator_species等)にとって、あるcandidateが捕食対象から除外されるべきか
// 判定する。chase_target(狩りの吸引ベクトル)・実際の捕食判定(strike radius)の
// 両方から呼ばれる共通ロジック(整合性を保つため一箇所にまとめている)。
// 基本方針: 自分自身・死亡個体・隠れているタコ・タコから見たピラニアは除外。同種は基本的に
// 対象外だが、ピラニアだけは「十分サイズ差(成長段階+捕食成長段階)がある」場合に限り
// 共食いの対象に含める(近いサイズ同士は対象外のまま)。
// スター(無敵アイテム)取得中の魚は、この通常ルールに関わらず捕食側・被食側の
// どちらでも特別扱いになる: 無敵中の魚は誰からも捕食対象にならず(candidate側の
// チェック)、逆に無敵中の魚が捕食側のときは、同種・タコ対ピラニア等の通常の
// 種別ルールを無視して誰でも対象にできる(一時的な捕食者反転ギミック)。
#[allow(clippy::too_many_arguments)]
fn is_excluded_as_prey(
    predator_species: Species,
    predator_growth_stage: u8,
    predator_kill_stage: u8,
    predator_invincible: bool,
    self_index: usize,
    candidate_index: usize,
    candidate_species: Species,
    candidate_dead: bool,
    candidate_hidden: bool,
    candidate_invincible: bool,
    candidate_hidden_in_cover: bool,
    candidate_growth_stage: u8,
    candidate_kill_stage: u8,
) -> bool {
    if self_index == candidate_index || candidate_dead {
        return true;
    }
    if candidate_species == Species::Octopus && candidate_hidden {
        return true; // タコは隠れている間は誰からも捕食対象にならない
    }
    if candidate_hidden_in_cover {
        // 藻・水草・岩に十分近い(隠れている)魚は、タコがタコつぼに隠れているのと
        // 同様に誰からも捕食対象にならない(無敵の一時的捕食者でも見つけられない、
        // という物理的な「隠れている」扱いのため、無敵バイパスより先に判定する)。
        return true;
    }
    if candidate_invincible {
        return true; // 無敵中の魚は誰からも捕食対象にならない
    }
    if predator_invincible {
        // 無敵中は種別ごとの通常ルール(同種は襲わない・タコはピラニアを襲わない等)を
        // 無視して、種類に関わらず誰でも捕食対象にできる。
        return false;
    }
    if predator_species == Species::Octopus && candidate_species == Species::Piranha {
        return true; // タコはピラニアを襲わない
    }
    if candidate_species == predator_species {
        if predator_species == Species::Piranha {
            // カニバリズム: 捕食側のサイズ指標が対象より十分大きい場合のみ対象に含める
            let predator_size = predator_growth_stage as i32 + predator_kill_stage as i32;
            let candidate_size = candidate_growth_stage as i32 + candidate_kill_stage as i32;
            return predator_size - candidate_size < PIRANHA_CANNIBALISM_MIN_SIZE_ADVANTAGE;
        }
        return true; // ピラニア以外は同種を襲わない(既存方針)
    }
    false
}

pub fn species_name(sp: Species) -> &'static str {
    match sp {
        Species::Neon => "ネオン",
        Species::Goldfish => "金魚",
        Species::Guppy => "グッピー",
        Species::Piranha => "ピラニア",
        Species::Angelfish => "エンゼルフィッシュ",
        Species::Betta => "ベタ",
        Species::Octopus => "タコ",
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
    fn hunting_piranha_moves_strongly_toward_nearby_prey() {
        // 追加要望: ピラニアが捕食モードのとき、獲物へ「追いかけている」のがわかるくらい
        // 強く近づく動きをすること。
        let mut sim = Simulation::new(Rng::new(104));
        // ピラニアの検知範囲(PIRANHA_HUNT_RADIUS)内に獲物を置く
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 10.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(piranha);
        // 実機フィードバック(「あたり判定を胴体でなく口に」)対応: 口(進行方向側の
        // スプライト前端)が獲物側へ張り出す分、中心間の距離が近いと初動でほぼ即座に
        // 届いてしまう。実際に「追いかけて近づく」動きを検証できるよう、口の張り出し分
        // (スプライト半幅+strike radius)を超える距離に獲物を置く。
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 38.0, 20.0)); // 獲物(距離28)
        let start_x = sim.fish[0].x;
        for _ in 0..24 {
            sim.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食条件を維持
            sim.update(0.05, 80, 40);
            if sim.fish.len() < 2 {
                break; // 追いついて捕食してしまったら十分な証拠なのでそこで終了
            }
        }
        let moved = sim.fish[0].x - start_x;
        assert!(
            moved > 10.0,
            "捕食モードのピラニアは短時間でも獲物方向へ大きく進むはず: moved={moved}"
        );
    }

    #[test]
    fn cornered_prey_is_eventually_caught_even_at_the_tank_corner() {
        // 実機フィードバック(「壁際に追い詰めた魚を永遠に捕食できない」)の再発防止。
        // 実機で発見した具体的な現象: サイズ基準の壁際マージンが捕食者・被食者で
        // 異なるため、両者がそれぞれ自分のマージンの角に追い詰められて、その間に
        // strike radiusより広い隙間が残ったまま完全に固まってしまうことがあった
        // (追跡・逃走中の反発力を弱めるだけでは不十分で、マージン自体もサイズ非依存の
        // 基本値に戻す必要があった)。左上の角に近い場所で再現し、十分な時間内に
        // 必ず捕食されることを確認する。
        let (w, h) = (100, 40);
        let mut sim = Simulation::new(Rng::new(613));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 3.0, 8.0)); // 左上の角付近
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 15.0, 12.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        piranha.predation_cooldown = 0.0;
        sim.fish.push(piranha);

        let mut caught = false;
        for _ in 0..300 {
            // 300 * 0.1 = 30秒。以前は無限に固まっていた現象の再発防止として、
            // 十分な猶予(実機で確認済みの時間より長め)を確保する。
            sim.fish[1].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // ピラニアの捕食モードを維持
            sim.update(0.1, w, h);
            if sim.fish.len() < 2 {
                caught = true;
                break;
            }
        }
        assert!(
            caught,
            "角に追い詰められた魚も、十分な時間内に確実に捕食されるはず"
        );
    }

    #[test]
    fn prey_flees_from_nearby_hunting_piranha() {
        // 追加要望: 近くにいる「捕食モードのピラニア」を検知したら逃げる。
        // (ピラニア自身も追いかけて動くため、両者が同時に動く状況でも「獲物自身の速度が
        // ピラニアと反対方向へはっきり向く」ことを直接確認する。ネット距離はピラニアの追跡が
        // 一時的に勝ることがあるため、ここでは検証しない)
        let mut sim = Simulation::new(Rng::new(105));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(piranha);
        // ピラニアの右側、捕食圏内だが1tickでは捕食されない距離に獲物を置く
        // (PIRANHA_STRIKE_RADIUSを実機フィードバックで2段階広げたため、近すぎると
        // 初回updateで即捕食されて配列外アクセスになる)
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 64.0, 20.0));
        sim.update(0.1, 80, 40);
        assert!(
            sim.fish[1].vx > 0.0,
            "ピラニアが左側にいるので、獲物は右(ピラニアと反対方向)へ逃げる速度になるはず: vx={}",
            sim.fish[1].vx
        );

        // 十分な時間が経てば、逃走の最高速度ブースト(1.6倍)によりピラニアより速く
        // 逃げられるはずなので、最終的に距離は広がる。
        let mut sim2 = Simulation::new(Rng::new(105));
        let mut piranha2 = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha2.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim2.fish.push(piranha2);
        sim2.fish.push(Fish::new(Species::Neon, Stage::Adult, 55.0, 20.0));
        // 回り込み(ジグザグ)の追加により、まっすぐ逃げる場合より距離が広がるまで
        // 少し時間がかかるようになったため、時間軸を延ばして「十分な時間が経てば」を確認する。
        for _ in 0..80 {
            sim2.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // ピラニアの捕食モードを維持
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
    fn prey_flees_even_from_well_fed_or_cooldown_piranha() {
        // 方針変更(「みんなピラニアが嫌い」): 通常の魚は、ピラニアが捕食モードかどうかに関わらず
        // 近くにピラニアがいるだけで常に逃走する。満腹中・クールダウン中のピラニアでも同様に逃げる。
        let mut sim = Simulation::new(Rng::new(106));

        let mut full_piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        full_piranha.hunger = MAX_HUNGER; // 満腹(捕食モードではない)でも逃げる対象になる
        sim.fish.push(full_piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 42.0, 20.0));
        sim.update(0.1, 80, 40);
        assert!(
            sim.fish[1].vx > 0.0,
            "満腹のピラニアからも逃げる方向へ加速するはず: vx={}",
            sim.fish[1].vx
        );

        let mut sim2 = Simulation::new(Rng::new(107));
        let mut cooldown_piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        cooldown_piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        cooldown_piranha.predation_cooldown = PIRANHA_HUNT_COOLDOWN; // クールダウン中
        sim2.fish.push(cooldown_piranha);
        sim2.fish.push(Fish::new(Species::Neon, Stage::Adult, 42.0, 20.0));
        sim2.update(0.1, 80, 40);
        assert!(
            sim2.fish[1].vx > 0.0,
            "クールダウン中のピラニアからも逃げる方向へ加速するはず: vx={}",
            sim2.fish[1].vx
        );
    }

    #[test]
    fn chasing_piranha_moves_faster_than_common_species_max_speed() {
        // 「ピラニアは追跡中は通常魚より速い」: 捕食モードで獲物を追っている間だけ、
        // ピラニアの最高速度が通常3種の最速種(ネオン=22.0)よりはっきり速くなる。
        let mut sim = Simulation::new(Rng::new(210));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 10.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 35.0, 20.0)); // 獲物(距離25、捕食圏内)
        // PIRANHA_STRIKE_RADIUSを実機フィードバックで拡大したため、捕食されるまでの
        // 時間が短くなり、ループ終了直後の1点計測では既に捕食後(追跡解除後)の
        // 巡回速度に戻っていて計測できないことがある。そのため各tickの実測速度の
        // 最大値を追いかけ、追跡中に一度でも通常最高速度を上回ったことを確認する。
        let mut max_speed = 0.0_f64;
        for _ in 0..30 {
            sim.update(0.1, 80, 40);
            let piranha_speed = (sim.fish[0].vx.powi(2) + sim.fish[0].vy.powi(2)).sqrt();
            max_speed = max_speed.max(piranha_speed);
            if sim.fish.len() < 2 {
                break; // 捕食されて消える前提のテストではないが、念のため打ち切る
            }
        }
        // 追跡中はピラニアの実測速度がネオンの最高速度(22.0)を上回るはず
        assert!(
            max_speed > Species::Neon.max_speed(),
            "追跡中のピラニアはネオンの最高速度より速いはず: max_speed={max_speed}"
        );
    }

    #[test]
    fn patrolling_piranha_does_not_get_speed_boost() {
        // 巡回中(獲物を追っていない)のピラニアは特別早くしない。満腹で捕食モードに
        // 入らないピラニアの最高速度が、通常のsp.max_speed()の範囲に収まることを確認する。
        let mut sim = Simulation::new(Rng::new(211));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 10.0, 20.0);
        piranha.hunger = MAX_HUNGER; // 満腹=捕食モードではない
        sim.fish.push(piranha);
        for _ in 0..40 {
            sim.update(0.1, 80, 40);
        }
        let piranha_speed = (sim.fish[0].vx.powi(2) + sim.fish[0].vy.powi(2)).sqrt();
        let normal_cap = Species::Piranha.max_speed() * 1.05; // 誤差余裕
        assert!(
            piranha_speed <= normal_cap,
            "巡回中のピラニアは通常の最高速度を超えないはず: piranha_speed={piranha_speed} cap={normal_cap}"
        );
    }

    #[test]
    fn random_dash_eventually_boosts_speed_during_normal_swimming() {
        // ランダムな瞬発ダッシュ: ピラニア・餌などのトリガーが無い通常の遊泳中でも、
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
    fn piranha_fear_flee_adds_zigzag_perpendicular_component() {
        // 回り込み(ジグザグ)確認: ピラニアと魚が同じ高さ(y)にいる場合、真っ直ぐ離れる
        // だけのベクトルならvyは0のままのはずだが、垂直方向の切り返し成分が入るため
        // 十分な時間が経てばvyが動くはず。
        let mut sim = Simulation::new(Rng::new(501));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = MAX_HUNGER; // 追跡はさせず、常時逃走(fear)の対象になることだけを見る
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0)); // ピラニアと同じy
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
    fn heavily_fed_pile_reaches_visually_significant_height() {
        // 実機フィードバック(「山として認識できるレベルまで最大高さを上げてほしい」)対応。
        // 餌を同じ場所へ大量に投入し続けたとき、盛り上がりが旧上限(3.0)を大きく超えて
        // 新しい上限(PILE_MAX_HEIGHT)近くまで達することを確認する。
        let (w, h) = (80, 40);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let mut sim = Simulation::new(Rng::new(614));

        // PILE_RADIUS以内の同じ場所へ、上限に届くだけの個数を着地させる
        let n = ((PILE_MAX_HEIGHT / PILE_STACK_STEP).ceil() as usize) + 5;
        for i in 0..n {
            sim.food.push(Food {
                x: 40.0 + (i as f64 * 0.01), // ほぼ同じ場所(PILE_RADIUS=2.5以内)
                y: sand_top - 0.05,
                vy: FOOD_SINK_SPEED,
                life: 999.0,
                landed: false,
            });
            sim.update(0.1, w, h);
        }

        let min_y = sim
            .food
            .iter()
            .filter(|fd| fd.landed)
            .map(|fd| fd.y)
            .fold(f64::INFINITY, f64::min);
        let rise = sand_top - min_y;
        assert!(
            rise > 3.0,
            "旧上限(3.0)を超えて盛り上がっているはず: rise={rise}"
        );
        assert!(
            rise <= PILE_MAX_HEIGHT + 0.01,
            "新しい上限を超えて盛り上がりすぎないはず: rise={rise} max={PILE_MAX_HEIGHT}"
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
    fn piranha_fear_flee_consumes_hunger_once_not_every_tick() {
        // 逃走コスト: ピラニアから逃げ始めた瞬間に空腹度を消費するが、ピラニアが居座って
        // 危険が続いている間、毎tick再課金されるわけではない。
        let mut sim = Simulation::new(Rng::new(109));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(piranha);
        // 捕食圏内だが1tickでは捕食されない距離に置く(PIRANHA_STRIKE_RADIUSを実機
        // フィードバックで2段階広げたため、近すぎると初回updateで即捕食されてしまう)
        let mut prey = Fish::new(Species::Neon, Stage::Adult, 64.0, 20.0);
        prey.hunger = MAX_HUNGER;
        sim.fish.push(prey);

        sim.update(0.1, 80, 40);
        let hunger_after_one_tick = sim.fish[1].hunger;
        assert!(
            hunger_after_one_tick <= MAX_HUNGER - FLEE_HUNGER_COST + 0.01,
            "逃走開始の瞬間に空腹度が消費されるはず: hunger={hunger_after_one_tick}"
        );

        // ピラニアが空腹状態を維持したまま(捕食モードのまま)何tickか経過しても、
        // 通常のゆっくりした空腹度減少以上には追加で大きく減らないはず
        for _ in 0..5 {
            sim.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // ピラニアの捕食モードを維持
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
    fn hungry_piranha_eats_nearby_prey_and_recovers_hunger() {
        let mut sim = Simulation::new(Rng::new(100));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食条件を満たす空腹度
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0)); // 捕食圏内

        // 実機フィードバック(「あたり判定を胴体でなく口に」)対応: 捕食判定は口(進行
        // 方向側のスプライト前端)基準になったため、1tickの中心距離だけでは判定できず、
        // 追跡して口が届くまで数tick必要になった。
        for _ in 0..30 {
            sim.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.update(0.1, 80, 40);
            if sim.fish.len() < 2 {
                break;
            }
        }

        assert_eq!(sim.fish.len(), 1, "捕食された魚はその場で消えるはず");
        assert_eq!(sim.fish[0].species, Species::Piranha, "残るのはピラニアのはず");
        assert!(sim.fish[0].hunger > PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0, "捕食で空腹度が回復するはず");
        assert_eq!(sim.fish[0].predation_cooldown, PIRANHA_HUNT_COOLDOWN, "捕食後はクールダウンに入るはず");
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
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        let (mx, my) = piranha.mouth_position();
        sim.fish.push(piranha);
        // 実機フィードバック(「あたり判定を胴体でなく口に」)対応: 口の位置に配置する
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, mx, my));
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
    fn piranha_grows_larger_with_each_kill_up_to_cap() {
        // ピラニアは捕食するたびに段階的に大きくなる(上限 PIRANHA_MAX_KILL_STAGE で打ち止め)
        let mut sim = Simulation::new(Rng::new(304));
        let piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        sim.fish.push(piranha);
        let base_scale = sim.fish[0].render_scale();

        for kill in 1..=(PIRANHA_MAX_KILL_STAGE as usize + 2) {
            sim.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モードに戻す
            sim.fish[0].predation_cooldown = 0.0; // クールダウン解除
            // 実機フィードバック(「あたり判定を胴体でなく口に」)対応: 捕食判定は口
            // (進行方向側のスプライト前端)基準になったため、口の現在位置に配置する
            // (捕食成長でピラニアが大きくなるたびに口の位置も変わるため、都度計算する)。
            let (mx, my) = sim.fish[0].mouth_position();
            sim.fish.push(Fish::new(Species::Neon, Stage::Adult, mx, my));
            // 実機フィードバック(「生き物の基本移動速度を4倍に」)対応で移動時の加速度も
            // 4倍になったため、口の位置に配置してもdt=0.1だと1tickの間にピラニア自身が
            // 大きく動いてしまい外れることがある。判定自体はdtに依存しないため、
            // 小さいdtでずれを抑える。
            sim.update(0.01, 80, 40);
            let expected_stage = (kill as u8).min(PIRANHA_MAX_KILL_STAGE);
            assert_eq!(
                sim.fish[0].kill_stage, expected_stage,
                "捕食{kill}回目でkill_stageが上限{PIRANHA_MAX_KILL_STAGE}まで積み上がるはず"
            );
        }
        assert!(
            sim.fish[0].render_scale() > base_scale,
            "捕食由来の成長で見た目の拡大率が上がるはず"
        );
    }

    #[test]
    fn well_fed_piranha_does_not_hunt() {
        let mut sim = Simulation::new(Rng::new(101));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = MAX_HUNGER; // 満腹なので狩らない
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.2, 20.1));

        sim.update(0.1, 80, 40);

        assert_eq!(sim.fish.len(), 2, "満腹のピラニアは捕食しないはず");
    }

    #[test]
    fn piranha_does_not_hunt_during_cooldown() {
        let mut sim = Simulation::new(Rng::new(102));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        piranha.predation_cooldown = PIRANHA_HUNT_COOLDOWN; // クールダウン中
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Goldfish, Stage::Adult, 40.2, 20.1));

        sim.update(0.1, 80, 40);

        assert_eq!(sim.fish.len(), 2, "クールダウン中のピラニアは捕食しないはず");
    }

    #[test]
    fn piranha_does_not_eat_same_size_piranhas() {
        // カニバリズム(新規)は「十分サイズ差がある」場合のみ許可する方針なので、
        // 近い/同じサイズのピラニア同士はこれまで通り捕食対象にならない。
        let mut sim = Simulation::new(Rng::new(103));
        let mut piranha1 = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha1.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha1);
        sim.fish.push(Fish::new(Species::Piranha, Stage::Adult, 40.2, 20.1));

        sim.update(0.1, 80, 40);

        assert_eq!(sim.fish.len(), 2, "近い/同じサイズのピラニア同士は捕食対象にならないはず");
    }

    #[test]
    fn large_piranha_can_cannibalize_a_much_smaller_piranha() {
        // カニバリズム(新規): 「大きいピラニアは小さいピラニアを食ってもいい」。
        // 十分サイズ差(成長段階+捕食成長段階)がある同種(ピラニア)は捕食対象に含める。
        let mut sim = Simulation::new(Rng::new(610));
        let mut big_piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        big_piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        big_piranha.growth_stage = GENERAL_MAX_GROWTH_STAGE; // 最大まで成長した大きいピラニア
        // 実機フィードバック(「あたり判定を胴体でなく口に」)対応: 口の位置に配置する
        let (mx, my) = big_piranha.mouth_position();
        sim.fish.push(big_piranha);
        // サイズ指標0(成長段階・捕食成長段階ともに0)の小さいピラニア
        sim.fish.push(Fish::new(Species::Piranha, Stage::Adult, mx, my));

        sim.update(0.1, 80, 40);

        assert_eq!(
            sim.fish.len(),
            1,
            "十分サイズ差のある大きいピラニアは、小さいピラニアを捕食してよいはず"
        );
        assert_eq!(sim.fish[0].species, Species::Piranha);
        assert_eq!(
            sim.fish[0].growth_stage, GENERAL_MAX_GROWTH_STAGE,
            "残るのは大きい方のピラニアのはず"
        );
        assert!(
            sim.sound_events.contains(&SfxEvent::Predation),
            "共食いでも通常の捕食演出(効果音)が出るはず"
        );
        assert!(!sim.blood_stains.is_empty(), "共食いでも血の演出が出るはず");
    }

    #[test]
    fn piranha_of_slightly_larger_size_still_cannot_cannibalize() {
        // サイズ差がPIRANHA_CANNIBALISM_MIN_SIZE_ADVANTAGE未満なら、大きい方でも
        // 小さい方を捕食対象にしない(近いサイズ同士の共食い乱発を防ぐ)。
        let mut sim = Simulation::new(Rng::new(611));
        let mut bigger_piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        bigger_piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        bigger_piranha.growth_stage = (PIRANHA_CANNIBALISM_MIN_SIZE_ADVANTAGE - 1).max(0) as u8;
        sim.fish.push(bigger_piranha);
        sim.fish.push(Fish::new(Species::Piranha, Stage::Adult, 40.2, 20.1));

        sim.update(0.1, 80, 40);

        assert_eq!(
            sim.fish.len(),
            2,
            "サイズ差が閾値未満なら共食いは起きないはず"
        );
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
    fn agility_mult_increases_for_fry_and_decreases_for_grown_fish() {
        // 「大きくなるほど遅くなる」(size_speed_mult)と対になる形で、小さいほど
        // 機敏(agility_mult>1.0)・大きいほどゆったり(agility_mult<1.0)に滑らかに連動する。
        let fry = Fish::new(Species::Neon, Stage::Fry, 0.0, 0.0);
        let adult = Fish::new(Species::Neon, Stage::Adult, 0.0, 0.0);
        assert!(
            fry.agility_mult() > adult.agility_mult(),
            "稚魚は成魚より機敏なはず: fry={} adult={}",
            fry.agility_mult(),
            adult.agility_mult()
        );
        assert!(
            (adult.agility_mult() - 1.0).abs() < 1e-9,
            "成魚(成長段階0)は基準の1.0倍のはず: {}",
            adult.agility_mult()
        );

        let mut grown = Fish::new(Species::Neon, Stage::Adult, 0.0, 0.0);
        grown.growth_stage = GENERAL_MAX_GROWTH_STAGE;
        assert!(
            grown.agility_mult() < adult.agility_mult(),
            "成長した個体ほどゆったり(機敏さが下がる)はず: grown={} adult={}",
            grown.agility_mult(),
            adult.agility_mult()
        );

        let mut piranha_grown = Fish::new(Species::Piranha, Stage::Adult, 0.0, 0.0);
        piranha_grown.kill_stage = PIRANHA_MAX_KILL_STAGE;
        let piranha_baseline = Fish::new(Species::Piranha, Stage::Adult, 0.0, 0.0);
        assert!(
            piranha_grown.agility_mult() < piranha_baseline.agility_mult(),
            "捕食で大きくなったピラニアほどゆったりのはず: grown={} baseline={}",
            piranha_grown.agility_mult(),
            piranha_baseline.agility_mult()
        );
    }

    #[test]
    fn fry_moves_faster_on_average_than_adult_in_normal_swimming() {
        // 統計的確認: 特別なトリガー(空腹・逃走等)が無い通常の遊泳では、
        // 十分な匹数・時間で見ると稚魚の平均速度(機敏さ)は成魚より高いはず。
        let mut sim = Simulation::new(Rng::new(600));
        for i in 0..40 {
            let mut f = Fish::new(Species::Neon, Stage::Fry, 5.0 + i as f64, 10.0);
            f.hunger = 55.0; // 満腹判定(60)未満に固定し、成長(Fry→Adult)が起きないようにする
            sim.fish.push(f);
        }
        for i in 0..40 {
            let mut f = Fish::new(Species::Neon, Stage::Adult, 5.0 + i as f64, 30.0);
            f.hunger = 55.0; // 稚魚側と空腹度の条件を揃える(spd_multを同一にする)
            sim.fish.push(f);
        }
        for _ in 0..100 {
            // 100 * 0.1 = 10秒。GROW_TIME(30秒)より十分短く、稚魚が成魚化しない範囲で
            // ドラッグによる速度の定常状態には十分な時間。
            for f in &mut sim.fish {
                f.hunger = 55.0; // 空腹度を維持(成長・腹ぺこ化を防ぐ)
            }
            sim.update(0.1, 800, 60);
        }

        let (mut fry_speed_sum, mut fry_n) = (0.0, 0usize);
        let (mut adult_speed_sum, mut adult_n) = (0.0, 0usize);
        for f in &sim.fish {
            if f.dead {
                continue;
            }
            let speed = (f.vx * f.vx + f.vy * f.vy).sqrt();
            if f.stage == Stage::Fry {
                fry_speed_sum += speed;
                fry_n += 1;
            } else {
                adult_speed_sum += speed;
                adult_n += 1;
            }
        }
        assert!(fry_n > 0 && adult_n > 0, "稚魚・成魚どちらも残っているはず");
        let fry_avg = fry_speed_sum / fry_n as f64;
        let adult_avg = adult_speed_sum / adult_n as f64;
        assert!(
            fry_avg > adult_avg,
            "稚魚の平均速度は成魚より高いはず: fry_avg={fry_avg} adult_avg={adult_avg}"
        );
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
        // 実機フィードバック(「産卵時にキラキラ光るフラッシュ演出を追加してほしい」)対応:
        // 産卵と同時にSpawn種のDropEffectが出るはず。
        assert!(
            sim.drop_effects.iter().any(|e| e.kind == EffectKind::Spawn),
            "産卵時にキラキラ演出(Spawn)のDropEffectが出るはず"
        );
    }

    #[test]
    fn spawn_flash_has_the_expected_lifetime_and_position() {
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(717));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 40.0, 30.0));
        let mut saw_flash = false;
        for _ in 0..12000 {
            sim.fish[0].hunger = MAX_HUNGER;
            sim.fish[0].well_fed_timer = BREED_READY_TIME + 5.0;
            sim.update(0.1, w, h);
            if let Some(flash) = sim.drop_effects.iter().find(|e| e.kind == EffectKind::Spawn) {
                assert_eq!(flash.max_life, SPAWN_FLASH_LIFETIME, "持続時間は仕様どおりのはず");
                saw_flash = true;
                break;
            }
        }
        assert!(saw_flash, "十分な時間が経てばキラキラ演出が出るはず");
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
        // ADD_FISH_MANUAL_CAP(25匹)で頭打ちになり、それ以上は増えない。
        let (w, h) = (800, 200);
        assert!(capacity(w, h) > ADD_FISH_MANUAL_CAP, "テスト前提: 水槽容量はADD_FISH_MANUAL_CAPより大きい");
        let mut sim = Simulation::new(Rng::new(91));
        for _ in 0..(ADD_FISH_MANUAL_CAP + 10) {
            sim.add_fish(w, h);
        }
        assert_eq!(
            sim.fish_count(),
            ADD_FISH_MANUAL_CAP,
            "+キーでの追加はADD_FISH_MANUAL_CAPで頭打ちになるはず"
        );
    }

    #[test]
    fn seed_initial_never_includes_piranhas() {
        // ピラニアの入手経路はSキーのみに限定する方針: 初期配置にピラニアは含まれない。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(99));
        sim.seed_initial(w, h);
        assert!(!sim.fish.is_empty(), "テスト前提: 初期個体が存在すること");
        assert!(
            sim.fish.iter().all(|f| f.species != Species::Piranha),
            "seed_initial はピラニアを含まないはず"
        );
    }

    #[test]
    fn seed_initial_never_includes_octopus() {
        // 仕様変更(「デフォルトでタコは入れない」): タコの入手経路はOキーのみに限定する
        // 方針: 起動時(seed_initial)にタコは含まれない。タコつぼ自体は空の装飾として
        // 配置されてよい。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(714));
        sim.seed_initial(w, h);
        assert!(!sim.fish.is_empty(), "テスト前提: 初期個体が存在すること");
        assert!(
            sim.fish.iter().all(|f| f.species != Species::Octopus),
            "seed_initial はタコを含まないはず"
        );
        assert!(!sim.dens.is_empty(), "タコつぼ自体は装飾として配置されるはず");
    }

    #[test]
    fn add_octopus_uses_an_existing_empty_den_before_creating_a_new_one() {
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(715));
        sim.ensure_decorative_entities(w, h);
        let den_count_before = sim.dens.len();
        assert!(den_count_before > 0, "テスト前提: タコつぼが空の状態で既に存在すること");

        sim.add_octopus(w, h);

        assert_eq!(sim.dens.len(), den_count_before, "空きタコつぼがあれば新設しないはず");
        let octo = sim
            .fish
            .iter()
            .find(|f| f.species == Species::Octopus)
            .expect("タコが1匹追加されているはず");
        assert!(
            sim.dens.iter().any(|d| d.x == octo.den_x && d.y == octo.den_y),
            "追加されたタコは既存のタコつぼのいずれかに紐づくはず"
        );
    }

    #[test]
    fn add_octopus_creates_a_new_den_when_all_existing_ones_are_occupied() {
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(716));
        sim.ensure_decorative_entities(w, h);
        let den_count_before = sim.dens.len();

        // 既存のタコつぼをすべてタコで埋める
        for _ in 0..den_count_before {
            sim.add_octopus(w, h);
        }
        assert_eq!(sim.dens.len(), den_count_before, "ここまでは新設されないはず");

        // もう一度追加すると、空きが無いので新しいタコつぼが1つ増設されるはず
        sim.add_octopus(w, h);
        assert_eq!(sim.dens.len(), den_count_before + 1, "空きが無ければ新設されるはず");
    }

    #[test]
    fn reset_never_includes_piranhas() {
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(100));
        // 一度ピラニアを混ぜてからリセットする
        sim.add_piranha(w, h);
        sim.reset(w, h);
        assert!(
            sim.fish.iter().all(|f| f.species != Species::Piranha),
            "グレートリセット後の初期配置にピラニアは含まれないはず"
        );
    }

    #[test]
    fn add_fish_random_pick_never_includes_piranhas() {
        // +キー(ランダム追加)は通常3種のみからのはず。十分回数試して確認する。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(101));
        for _ in 0..40 {
            sim.add_fish(w, h);
        }
        assert!(
            sim.fish.iter().all(|f| f.species != Species::Piranha),
            "+キーのランダム追加はピラニアを選ばないはず"
        );
    }

    #[test]
    fn piranha_never_lays_eggs_even_when_breed_ready() {
        // ピラニアは産卵→孵化の繁殖ロジックから除外されている(Sキー以外で増えない方針)。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(102));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 30.0);
        piranha.hunger = MAX_HUNGER;
        piranha.well_fed_timer = BREED_READY_TIME + 5.0;
        sim.fish.push(piranha);
        for _ in 0..12000 {
            sim.fish[0].hunger = MAX_HUNGER;
            sim.fish[0].well_fed_timer = BREED_READY_TIME + 5.0;
            sim.update(0.1, w, h);
        }
        assert!(sim.eggs.is_empty(), "ピラニアはどれだけ満腹維持しても産卵しないはず");
        assert_eq!(sim.fish_count(), 1, "ピラニアが増えていないはず");
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
        // 寿命(LIFESPAN_DEATH_AGE)に達すると老衰で死亡演出に入る(全種共通・ピラニアも対象)。
        let mut sim = Simulation::new(Rng::new(307));
        let mut f = Fish::new(Species::Piranha, Stage::Adult, 20.0, 10.0);
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
    fn add_piranha_always_adds_a_piranha() {
        // Sキー: ランダムではなく確実にピラニアを追加できる。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(96));
        for _ in 0..5 {
            sim.add_piranha(w, h);
        }
        assert_eq!(sim.fish_count(), 5, "5回呼べば5匹追加されるはず");
        assert!(
            sim.fish.iter().all(|f| f.species == Species::Piranha),
            "add_piranha で追加されるのは常にピラニアのはず"
        );
    }

    #[test]
    fn add_piranha_is_capped_at_manual_cap() {
        let (w, h) = (800, 200);
        assert!(capacity(w, h) > ADD_FISH_MANUAL_CAP, "テスト前提: 水槽容量はADD_FISH_MANUAL_CAPより大きい");
        let mut sim = Simulation::new(Rng::new(97));
        for _ in 0..(ADD_FISH_MANUAL_CAP + 10) {
            sim.add_piranha(w, h);
        }
        assert_eq!(
            sim.fish_count(),
            ADD_FISH_MANUAL_CAP,
            "Sキーでの追加も+キーと同じくADD_FISH_MANUAL_CAPで頭打ちになるはず"
        );
    }

    #[test]
    fn add_piranha_respects_tank_capacity_too() {
        // ADD_FISH_MANUAL_CAP(25)より小さい水槽容量でも、そちらの上限が優先されて超えない。
        let (w, h) = (40, 20); // capacity は最小の5になる
        let cap = capacity(w, h);
        assert!(cap < ADD_FISH_MANUAL_CAP, "テスト前提: 水槽容量がADD_FISH_MANUAL_CAPより小さいこと");
        let mut sim = Simulation::new(Rng::new(98));
        for _ in 0..20 {
            sim.add_piranha(w, h);
        }
        assert_eq!(sim.fish_count(), cap, "水槽容量の上限で頭打ちになるはず");
    }

    #[test]
    fn remove_fish_falls_back_to_crabs() {
        // 「魚 0/N」まで減らしてもカニが残り続けて分かりづらい、という
        // フィードバックへの対応。- キーは通常魚(ピラニア含む)→カニの順にフォールバックする。
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
    fn remove_fish_clears_dens_once_the_last_octopus_is_thinned_out() {
        // 実機フィードバック対応: タコが通常のfish扱いで-キーにより間引かれて0匹に
        // なったら、対応するタコつぼ(dens)も一緒に消える(空のタコつぼだけが
        // 取り残されると不自然)。
        let mut sim = Simulation::new(Rng::new(91));
        sim.fish.push(Fish::new(Species::Octopus, Stage::Adult, 10.0, 10.0));
        sim.dens.push(Den { x: 10.0, y: 10.0 });
        assert_eq!(sim.dens.len(), 1);

        sim.remove_fish();
        assert!(
            sim.fish.iter().all(|f| f.species != Species::Octopus),
            "タコが間引かれているはず"
        );
        assert!(sim.dens.is_empty(), "タコが0匹になったらタコつぼも消えるはず");
    }

    #[test]
    fn remove_fish_keeps_dens_while_another_octopus_still_lives() {
        // タコが複数いる場合、1匹間引かれただけではまだ残っているのでタコつぼは消さない。
        let mut sim = Simulation::new(Rng::new(92));
        sim.fish.push(Fish::new(Species::Octopus, Stage::Adult, 10.0, 10.0));
        sim.fish.push(Fish::new(Species::Octopus, Stage::Adult, 20.0, 10.0));
        sim.dens.push(Den { x: 10.0, y: 10.0 });
        sim.dens.push(Den { x: 20.0, y: 10.0 });

        sim.remove_fish();
        assert_eq!(
            sim.fish.iter().filter(|f| f.species == Species::Octopus).count(),
            1,
            "タコが1匹減っているはず"
        );
        assert_eq!(sim.dens.len(), 2, "タコがまだ1匹残っているのでタコつぼは消さないはず");
    }

    #[test]
    fn reposition_dens_keeps_the_same_count_and_moves_existing_octopus_with_it() {
        // Dキー: タコつぼは同じ数だけ新しい位置に生成し直され、既存のタコ(隠れて
        // いる巣)もden_x/den_yが新しい座標に追従するはず。
        // 仕様変更(「デフォルトでタコは入れない」)対応: Oキー(add_octopus)で
        // タコつぼにタコを紐づけてから検証する(旧: 自動配置されたものを使っていた)。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(93));
        sim.ensure_decorative_entities(w, h);
        sim.add_octopus(w, h);
        let old_den = sim.dens[0].clone();
        let old_count = sim.dens.len();

        sim.reposition_dens(w, h);

        assert_eq!(sim.dens.len(), old_count, "タコつぼの数は変わらないはず");
        let new_den = sim.dens[0].clone();
        assert!(
            (new_den.x - old_den.x).abs() > 0.001 || (new_den.y - old_den.y).abs() > 0.001,
            "タコつぼは新しい位置に再配置されるはず"
        );

        let octo = sim
            .fish
            .iter()
            .find(|f| f.species == Species::Octopus)
            .expect("タコが1匹いるはず");
        assert_eq!(octo.den_x, new_den.x, "タコのden_xも新しいタコつぼに追従するはず");
        assert_eq!(octo.den_y, new_den.y, "タコのden_yも新しいタコつぼに追従するはず");
    }

    #[test]
    fn reposition_dens_does_nothing_and_messages_when_there_are_no_dens() {
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(94));
        assert!(sim.dens.is_empty());
        sim.reposition_dens(w, h);
        assert!(sim.dens.is_empty());
    }

    #[test]
    fn reposition_plants_keeps_the_same_count_and_moves_them() {
        // Pキー: 藻・水草は同じ数だけ新しい位置に生成し直される。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(95));
        sim.ensure_decorative_entities(w, h);
        let old_plant = sim.plants[0].clone();
        let old_count = sim.plants.len();

        sim.reposition_plants(w, h);

        assert_eq!(sim.plants.len(), old_count, "藻・水草の数は変わらないはず");
        let new_plant = sim.plants[0].clone();
        assert!(
            (new_plant.x - old_plant.x).abs() > 0.001,
            "藻・水草は新しい位置に再配置されるはず"
        );
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
    fn large_grown_fish_gets_more_bottom_and_wall_clearance_than_small_fish() {
        // 実機フィードバック(「魚が水底に張り付いて見える。UFOのように動かない」)の
        // 再発防止: 拡大表示(render_scale)された魚ほど、可動範囲のマージン(壁際・
        // 水底)もスプライトの実サイズに応じて広がり、中心座標が水底ぎりぎりまで
        // 許容されて拡大後の見た目が水底に埋まる、ということが起きないようにする。
        let (w, h) = (200, 100);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);

        let mut sim_small = Simulation::new(Rng::new(700));
        let mut small = Fish::new(Species::Piranha, Stage::Adult, 100.0, sand_top - 0.5);
        small.growth_stage = 0;
        sim_small.fish.push(small);
        sim_small.update(0.1, w, h);
        let small_y = sim_small.fish[0].y;

        let mut sim_large = Simulation::new(Rng::new(700));
        let mut large = Fish::new(Species::Piranha, Stage::Adult, 100.0, sand_top - 0.5);
        large.growth_stage = GENERAL_MAX_GROWTH_STAGE;
        large.kill_stage = PIRANHA_MAX_KILL_STAGE;
        sim_large.fish.push(large);
        sim_large.update(0.1, w, h);
        let large_y = sim_large.fish[0].y;

        assert!(
            large_y < small_y,
            "大きく育った魚ほど、水底からのクリアランス(中心座標の上限)が大きいはず(yが小さい方が上): small_y={small_y} large_y={large_y}"
        );

        // 拡大後のスプライトの下端が水底(sand_top)に埋まっていないことも確認する
        let sprite = sim_large.fish[0].sprite();
        let scale = sim_large.fish[0].render_scale();
        let half_h = (sprite.height as f64 * scale) / 2.0;
        assert!(
            large_y + half_h <= sand_top + 0.5, // 多少の誤差は許容
            "拡大後のスプライト下端が水底に埋まっていないはず: large_y={large_y} half_h={half_h} sand_top={sand_top}"
        );
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
    fn ensure_decorative_entities_seeds_plants_dens_and_rocks_but_no_octopus() {
        // 仕様変更(「デフォルトでタコは入れない」)対応: タコつぼ自体は空の装飾として
        // 初期配置されるが、タコは一切自動配置されない(Oキーでのみ出せる)。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(600));
        assert!(sim.plants.is_empty());
        assert!(sim.rocks.is_empty());
        assert!(sim.dens.is_empty());
        sim.ensure_decorative_entities(w, h);
        assert_eq!(sim.plants.len(), PLANT_COUNT, "藻・水草は既定数だけ補充される");
        assert_eq!(sim.rocks.len(), ROCK_COUNT, "岩は既定数だけ補充される");
        assert_eq!(sim.dens.len(), DEN_COUNT, "タコつぼは既定数だけ補充される");
        let octopus_count = sim
            .fish
            .iter()
            .filter(|f| f.species == Species::Octopus)
            .count();
        assert_eq!(octopus_count, 0, "タコは初期状態では一切配置されないはず");

        // 再度呼んでも増殖しない
        sim.ensure_decorative_entities(w, h);
        assert_eq!(sim.plants.len(), PLANT_COUNT);
        assert_eq!(sim.rocks.len(), ROCK_COUNT);
        assert_eq!(sim.dens.len(), DEN_COUNT);
        let octopus_count2 = sim
            .fish
            .iter()
            .filter(|f| f.species == Species::Octopus)
            .count();
        assert_eq!(octopus_count2, 0, "再度呼んでもタコは補充されないはず");
    }

    #[test]
    fn den_sprite_is_much_larger_than_the_old_small_size() {
        // 実機フィードバック(「タコつぼが小さく目立たなかった。壺らしい形がはっきり
        // 分かるサイズに」)対応: 旧サイズ(6幅x5高)よりはっきり大きく描き直した。
        let sprite = den_sprite();
        assert!(
            sprite.width >= 10 && sprite.height >= 8,
            "タコつぼのスプライトは十分大きいはず: width={} height={}",
            sprite.width,
            sprite.height
        );
    }

    #[test]
    fn den_placement_does_not_bury_the_larger_sprite_deep_in_the_sand() {
        // タコつぼを大きく描き直した際、中心Yの計算が旧サイズ用の固定値のままだと、
        // 拡大後のスプライトが水底に深く埋まって見えてしまう。底面が水底のすぐ近くに
        // 来るように配置されていることを確認する(多少埋まる程度は許容)。
        let (w, h) = (200, 100);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let mut sim = Simulation::new(Rng::new(615));
        sim.ensure_decorative_entities(w, h);
        let den = &sim.dens[0];
        let half_h = den_sprite().height as f64 / 2.0;
        let bottom = den.y + half_h;
        assert!(
            (bottom - sand_top).abs() < 2.0,
            "タコつぼの底面は水底のすぐ近くにあるはず: bottom={bottom} sand_top={sand_top}"
        );
    }

    #[test]
    fn resync_seabed_decor_follows_a_new_sand_top_after_a_resize() {
        // 実機フィードバック(「文字サイズを変更したらタコツボとか草とかが床に
        // 沈殿する。逆になったら浮くようにして」)対応: タコつぼ・水草は生成時の
        // sand_topを基準にしたY座標を絶対値で保持しているため、端末サイズ変更で
        // 水底位置がずれても追従しない。resync_seabed_decorで新しい水底に合わせて
        // 再配置できることを確認する。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(620));
        sim.ensure_decorative_entities(w, h);
        // 仕様変更(「デフォルトでタコは入れない」)対応: タコつぼに隠れたタコを
        // このテストで直接用意する(旧: 自動配置されたものを使っていた)。
        let seed_den = sim.dens[0].clone();
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, seed_den.x, seed_den.y);
        octo.hidden = true;
        octo.den_x = seed_den.x;
        octo.den_y = seed_den.y;
        sim.fish.push(octo);
        let old_plant_y = sim.plants[0].y;
        let old_rock_y = sim.rocks[0].y;
        let old_den_y = sim.dens[0].y;

        // 高さを大きく変える(=水底の位置が大きく変わる)ことをシミュレートする
        let (w2, h2) = (200, 40);
        let sand_top2 = (h2 as f64 - sand_height(h2) as f64).max(2.0);
        assert!(
            (sand_top2 - old_plant_y).abs() > 3.0,
            "この検証には水底位置が十分に変わるサイズ差が必要"
        );

        sim.resync_seabed_decor(w2, h2);

        assert_eq!(
            sim.plants[0].y, sand_top2,
            "水草は新しい水底のY座標に一致するはず"
        );
        assert_ne!(sim.plants[0].y, old_plant_y, "リサイズ前のY座標のままではないはず");

        let rock_half_h = rock_sprite().height as f64 / 2.0;
        let expected_rock_y = (sand_top2 - rock_half_h + 1.0).max(1.0);
        assert_eq!(
            sim.rocks[0].y, expected_rock_y,
            "岩も新しい水底に合わせた高さになるはず"
        );
        assert_ne!(sim.rocks[0].y, old_rock_y, "リサイズ前のY座標のままではないはず");

        let den_half_h = den_sprite().height as f64 / 2.0;
        let expected_den_y = (sand_top2 - den_half_h + 1.0).max(1.0);
        assert_eq!(
            sim.dens[0].y, expected_den_y,
            "タコつぼも新しい水底に合わせた高さになるはず"
        );
        assert_ne!(sim.dens[0].y, old_den_y, "リサイズ前のY座標のままではないはず");

        // そのタコつぼを巣にしているタコ(隠れている個体)も新しい座標に追従するはず
        let den_x = sim.dens[0].x;
        let den_y = sim.dens[0].y;
        let octo = sim
            .fish
            .iter()
            .find(|f| f.species == Species::Octopus && f.hidden)
            .expect("隠れているタコが1匹いるはず");
        assert_eq!(octo.den_x, den_x, "タコのden_xも新しいタコつぼ座標に追従するはず");
        assert_eq!(octo.den_y, den_y, "タコのden_yも新しいタコつぼ座標に追従するはず");
        assert_eq!(octo.x, den_x, "隠れているタコの表示位置も追従するはず");
        assert_eq!(octo.y, den_y, "隠れているタコの表示位置も追従するはず");
    }

    #[test]
    fn octopus_emerges_and_eventually_returns_to_den() {
        // 低頻度でつぼから出てきて泳ぎ、しばらくしたら戻る。
        // 仕様変更(「デフォルトでタコは入れない」)対応: ensure_decorative_entities()は
        // もうタコを自動配置しないため、タコつぼに紐づく隠れたタコをこのテストで直接
        // 用意する(旧: 自動配置されたものをそのまま使っていた箇所)。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(601));
        sim.ensure_decorative_entities(w, h);
        let den = sim.dens[0].clone();
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, den.x, den.y);
        octo.hidden = true;
        octo.den_x = den.x;
        octo.den_y = den.y;
        octo.hidden_timer = OCTOPUS_HIDDEN_TIME_MAX;
        sim.fish.push(octo);

        // 隠れている間は巣に固定され、動かない
        assert!(sim.fish[0].hidden);
        assert_eq!(sim.fish[0].x, den.x);
        assert_eq!(sim.fish[0].y, den.y);

        let mut saw_emerged = false;
        for _ in 0..1000 {
            // 1000 * 0.1 = 100秒。OCTOPUS_HIDDEN_TIME_MAX(40秒)より十分長い。
            sim.update(0.1, w, h);
            if !sim.fish[0].hidden {
                saw_emerged = true;
                break;
            }
        }
        assert!(saw_emerged, "十分な時間が経てば一度はつぼから出てくるはず");

        // さらに時間を進めれば、出ている時間(最大40秒)を超えて必ず巣へ戻るはず
        for _ in 0..700 {
            // 700 * 0.1 = 70秒。OCTOPUS_EMERGE_TIME_MAX(40秒)より十分長い。
            sim.update(0.1, w, h);
            if sim.fish[0].hidden {
                break;
            }
        }
        assert!(sim.fish[0].hidden, "出ている時間が終われば必ず巣へ戻って隠れるはず");
        assert_eq!(sim.fish[0].x, den.x, "隠れた瞬間、巣の位置に戻っているはず");
        assert_eq!(sim.fish[0].y, den.y);
    }

    #[test]
    fn hidden_octopus_is_not_hunted_by_a_hungry_piranha() {
        // 隠れている間は捕食対象にならない(見えていないので狩れない)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(602));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = true;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        octo.hidden_timer = 999.0; // 出てこないようにする
        sim.fish.push(octo);
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 41.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);

        for _ in 0..50 {
            sim.fish[1].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.fish[1].predation_cooldown = 0.0;
            sim.update(0.1, w, h);
        }
        assert_eq!(sim.fish_count(), 2, "隠れているタコは捕食されないはず");
    }

    #[test]
    fn emerged_octopus_can_be_eaten_by_a_hungry_piranha() {
        // 出ているタコはピラニアの捕食対象になってよい。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(603));
        let piranha_probe = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        let (mx, my) = piranha_probe.mouth_position();
        // 実機フィードバック(「あたり判定を胴体でなく口に」)対応: 口の位置に配置する
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, mx, my);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = mx;
        octo.den_y = my;
        // このテストは「捕食対象になれること」自体を見たいので、墨の緊急脱出免除
        // (実機フィードバック対応で新設)が働かないよう、墨をクールダウン中にしておく
        // (ピラニアがすぐ近くにいるため、素で置くと墨を吐いて一時的に捕食免除になってしまう)。
        octo.ink_cooldown = 999.0;
        sim.fish.push(octo);
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);

        sim.update(0.1, w, h);

        assert_eq!(sim.fish_count(), 1, "出ているタコはピラニアに捕食されてよいはず");
        assert_eq!(sim.fish[0].species, Species::Piranha);
    }

    #[test]
    fn fish_touching_a_star_becomes_invincible_and_the_star_disappears() {
        // スターに触れた魚は一定時間無敵化し、スター自体は取得されて消える。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(700));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));
        sim.stars.push(Star {
            x: 40.0,
            y: 20.0,
            life: STAR_LIFETIME,
            phase: 0.0,
        });

        sim.update(0.1, w, h);

        assert!(sim.stars.is_empty(), "取得されたスターは消えるはず");
        assert!(
            sim.fish[0].is_invincible(),
            "触れた魚は無敵状態になるはず: invincible_timer={}",
            sim.fish[0].invincible_timer
        );
        assert!(
            sim.fish[0].invincible_timer >= STAR_INVINCIBLE_DURATION_MIN
                && sim.fish[0].invincible_timer <= STAR_INVINCIBLE_DURATION_MAX,
            "無敵時間は既定の範囲内のはず: {}",
            sim.fish[0].invincible_timer
        );
    }

    #[test]
    fn star_out_of_pickup_range_is_not_collected() {
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(701));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));
        sim.stars.push(Star {
            x: 40.0 + STAR_PICKUP_RADIUS + 5.0,
            y: 20.0,
            life: STAR_LIFETIME,
            phase: 0.0,
        });

        sim.update(0.1, w, h);

        assert_eq!(sim.stars.len(), 1, "取得範囲外のスターは残るはず");
        assert!(!sim.fish[0].is_invincible());
    }

    #[test]
    fn uncollected_star_disappears_after_its_lifetime() {
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(702));
        // 誰も近くにいない位置に置く
        sim.stars.push(Star {
            x: 5.0,
            y: 5.0,
            life: 0.2,
            phase: 0.0,
        });
        sim.update(0.3, w, h);
        assert!(sim.stars.is_empty(), "寿命が尽きたスターは誰にも取られず消えるはず");
    }

    #[test]
    fn invincible_candidate_is_always_excluded_as_prey() {
        // 無敵中の魚は、通常なら捕食されるはずの状況(捕食モードのピラニアが口に
        // 触れる距離)でも捕食対象から除外される。update_predation全体を回すと、
        // 無敵の魚自身も一時的捕食者として相手(ピラニア)を逆に捕食してしまい、
        // 「捕食されない」側だけを他の効果と混ざらず単独で確認できないため、
        // 判定ロジック(is_excluded_as_prey)を直接呼んで検証する。
        assert!(
            is_excluded_as_prey(
                Species::Piranha,
                0,
                0,
                false, // 捕食側(ピラニア)は無敵ではない
                0,
                1,
                Species::Neon,
                false,
                false,
                true,  // 対象(ネオン)が無敵
                false, // 藻・岩に隠れているわけではない
                0,
                0,
            ),
            "無敵中の魚は誰からも捕食対象にならないはず"
        );
    }

    #[test]
    fn invincible_common_fish_can_prey_on_a_piranha() {
        // スター取得中は、普段は捕食されない側の通常種でも、触れたピラニア(捕食者)を
        // 逆に捕食できる(一時的な捕食者反転ギミック)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(704));
        let mut hero = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        hero.invincible_timer = 10.0;
        let (mx, my) = hero.mouth_position();
        sim.fish.push(hero);
        // ピラニアは満腹(捕食モードではない)にしておき、「ピラニア自身の狩り」ではなく
        // 「無敵の通常種による捕食」であることを明確にする。
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, mx, my);
        piranha.hunger = MAX_HUNGER;
        sim.fish.push(piranha);

        // どちらも相手に向かって寄っていく力(狙う理由)が無い組み合わせのため、
        // 通常のランダムウォークによるわずかなずれで接触が外れないよう小さいdtにする
        // (実機フィードバック「生き物の基本移動速度を4倍に」対応でwander()の加速度も
        // 4倍になったため、旧dtよりさらに小さくして同程度のずれに抑える)。
        sim.update(0.005, w, h);

        assert_eq!(sim.fish_count(), 1, "無敵中のネオンがピラニアを捕食できるはず");
        assert_eq!(sim.fish[0].species, Species::Neon);
        assert!(sim.fish[0].is_invincible());
    }

    #[test]
    fn invincible_common_fish_can_prey_on_an_emerged_octopus() {
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(705));
        let mut hero = Fish::new(Species::Goldfish, Stage::Adult, 40.0, 20.0);
        hero.invincible_timer = 10.0;
        let (mx, my) = hero.mouth_position();
        sim.fish.push(hero);
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, mx, my);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = mx;
        octo.den_y = my;
        octo.ink_cooldown = 999.0;
        sim.fish.push(octo);

        // タコは無敵の金魚を捕食対象にできない(除外される)ため寄ってくる力が無く、
        // 通常のランダムウォークで接触が外れないよう小さいdtにする。
        sim.update(0.01, w, h);

        assert_eq!(sim.fish_count(), 1, "無敵中の金魚が出ているタコを捕食できるはず");
        assert_eq!(sim.fish[0].species, Species::Goldfish);
    }

    #[test]
    fn invincibility_expires_and_fish_becomes_vulnerable_again() {
        let mut sim = Simulation::new(Rng::new(706));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        f.invincible_timer = 0.05;
        sim.fish.push(f);

        sim.update(0.2, 80, 40);

        assert!(
            !sim.fish[0].is_invincible(),
            "無敵時間が尽きたら通常状態に戻るはず: {}",
            sim.fish[0].invincible_timer
        );
    }

    #[test]
    fn two_invincible_fish_do_not_prey_on_each_other() {
        // 無敵中の魚は誰からも捕食対象にならないため、無敵同士が触れても何も起きない。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(707));
        let mut a = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        a.invincible_timer = 10.0;
        let (mx, my) = a.mouth_position();
        sim.fish.push(a);
        let mut b = Fish::new(Species::Goldfish, Stage::Adult, mx, my);
        b.invincible_timer = 10.0;
        sim.fish.push(b);

        sim.update(0.1, w, h);

        assert_eq!(sim.fish_count(), 2, "無敵同士は互いに捕食しないはず");
    }

    #[test]
    fn is_hidden_in_cover_detects_proximity_to_plants_and_rocks() {
        let mut sim = Simulation::new(Rng::new(710));
        sim.plants.push(Plant {
            x: 10.0,
            y: 10.0,
            height: 8.0,
            phase: 0.0,
        });
        sim.rocks.push(Rock { x: 50.0, y: 10.0 });

        assert!(sim.is_hidden_in_cover(10.0, 10.0), "藻の真上は隠れているはず");
        assert!(sim.is_hidden_in_cover(50.0, 10.0), "岩の真上は隠れているはず");
        assert!(
            !sim.is_hidden_in_cover(30.0, 10.0),
            "藻・岩どちらからも離れた位置は隠れていないはず"
        );
    }

    #[test]
    fn fish_hidden_near_a_plant_is_not_eaten_by_a_hunting_piranha() {
        // 実機フィードバック(「隠れたら実際に捕食されなくなる機能化」)対応: 藻に
        // 十分近い(隠れている)魚は、口が触れる距離にいる捕食モードのピラニアからも
        // 捕食されない。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(711));
        let piranha_probe = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        let (mx, my) = piranha_probe.mouth_position();
        sim.plants.push(Plant {
            x: mx,
            y: my,
            height: 8.0,
            phase: 0.0,
        });
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, mx, my));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);

        sim.update(0.1, w, h);

        assert_eq!(sim.fish_count(), 2, "藻に隠れている魚はピラニアに捕食されないはず");
    }

    #[test]
    fn fish_hidden_near_a_rock_is_not_eaten_by_a_hungry_emerged_octopus() {
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(712));
        let octo_probe = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        let (mx, my) = octo_probe.mouth_position();
        sim.rocks.push(Rock { x: mx, y: my });
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, mx, my));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        octo.hunger = OCTOPUS_HUNT_HUNGER_THRESHOLD - 10.0;
        octo.ink_cooldown = 999.0;
        sim.fish.push(octo);

        sim.update(0.1, w, h);

        assert_eq!(sim.fish_count(), 2, "岩に隠れている魚はタコに捕食されないはず");
    }

    #[test]
    fn fish_away_from_cover_is_still_eaten_normally() {
        // 隠れ場所ロジックを入れても、隠れ場所が無い/遠い通常の状況では
        // 相変わらず捕食が成立することの回帰確認。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(713));
        let piranha_probe = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        let (mx, my) = piranha_probe.mouth_position();
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, mx, my));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);

        sim.update(0.1, w, h);

        assert_eq!(sim.fish_count(), 1, "隠れ場所が無ければ通常どおり捕食されるはず");
    }

    #[test]
    fn octopus_does_not_prey_on_piranhas() {
        // タコはピラニアを襲わない(タコ自身の捕食対象からピラニアは除外)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(604));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        octo.hunger = OCTOPUS_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(octo);
        // ピラニア自身は満腹にしておき、ピラニア→タコの捕食が起きないようにして
        // 「タコ→ピラニア」の一方向だけを見る(ピラニアの捕食閾値はPIRANHA_HUNT_HUNGER_THRESHOLD)。
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.2, 20.1);
        piranha.hunger = MAX_HUNGER;
        sim.fish.push(piranha);

        for _ in 0..50 {
            sim.fish[0].hunger = OCTOPUS_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.fish[0].predation_cooldown = 0.0;
            sim.fish[1].hunger = MAX_HUNGER;
            sim.update(0.1, w, h);
        }
        assert_eq!(sim.fish_count(), 2, "タコがピラニアを襲うことはないはず");
    }

    #[test]
    fn octopus_preys_on_regular_fish_when_hungry_and_emerged() {
        // タコ自身の捕食行動: 出ていて空腹ならクールダウン明けで通常の魚を襲える。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(605));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        octo.hunger = OCTOPUS_HUNT_HUNGER_THRESHOLD - 10.0;
        // 実機フィードバック(「あたり判定を胴体でなく口に」)対応: 口の位置に配置する
        let (mx, my) = octo.mouth_position();
        sim.fish.push(octo);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, mx, my));

        sim.update(0.1, w, h);

        assert_eq!(sim.fish_count(), 1, "空腹で出ているタコは近くの魚を捕食できるはず");
        assert_eq!(sim.fish[0].species, Species::Octopus);
        assert!(
            sim.fish[0].hunger > OCTOPUS_HUNT_HUNGER_THRESHOLD - 10.0,
            "捕食で空腹度が回復するはず"
        );
    }

    #[test]
    fn octopus_inks_when_a_hunting_piranha_is_nearby() {
        // ピラニアに追われると(近くに捕食モードのピラニアがいると)、逃走に加えて墨を吐く。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(606));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        sim.fish.push(octo);
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 45.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(piranha);

        sim.update(0.1, w, h);

        assert!(!sim.ink_clouds.is_empty(), "ピラニアに追われたタコは墨を吐くはず");
        assert!(sim.sound_events.contains(&SfxEvent::Ink));
        assert!(sim.fish[0].ink_cooldown > 0.0, "墨を吐いた後はクールダウンに入るはず");
    }

    #[test]
    fn octopus_ink_has_a_cooldown_and_does_not_spam() {
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(607));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        sim.fish.push(octo);
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 45.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);

        sim.update(0.1, w, h);
        let ink_count_after_first = sim.ink_clouds.len();
        assert!(ink_count_after_first > 0);

        for _ in 0..30 {
            // 合計3秒。INK_COOLDOWN(20秒)より十分短い。
            sim.fish[1].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // ピラニアの捕食モードを維持
            sim.fish[1].predation_cooldown = 0.0;
            sim.update(0.1, w, h);
        }
        assert_eq!(
            sim.ink_clouds.len(),
            ink_count_after_first,
            "クールダウン中は追加で墨を吐かないはず"
        );
    }

    #[test]
    fn octopus_ink_grants_temporary_strike_immunity_and_speed_boost() {
        // 実機フィードバック(「墨を吐いたら高確率で逃げ切れる、という結果まで保証」)対応:
        // 墨を吐いた直後は、たとえピラニアが捕食圏内にいても一時的に捕食されない
        // (INK_ESCAPE_DURATIONの間、strike radiusの判定から除外される)。
        // 加えて、緊急ダッシュにより通常の最高速度を上回る速度が出るはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(612));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        sim.fish.push(octo);
        // ピラニアを捕食圏内(strike radius)かつ墨の誘発範囲内に置く
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.1, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);

        sim.update(0.1, w, h);

        assert_eq!(
            sim.fish.len(),
            2,
            "墨を吐いた直後は捕食圏内でも一時的に捕食されないはず"
        );
        let octo_idx = sim
            .fish
            .iter()
            .position(|f| f.species == Species::Octopus)
            .expect("タコが残っているはず");
        assert!(
            sim.fish[octo_idx].ink_escape_timer > 0.0,
            "緊急脱出タイマーが立っているはず"
        );

        let escape_speed =
            (sim.fish[octo_idx].vx.powi(2) + sim.fish[octo_idx].vy.powi(2)).sqrt();
        assert!(
            escape_speed > Species::Octopus.max_speed(),
            "緊急ダッシュで通常の最高速度より速く逃げているはず: speed={escape_speed}"
        );
    }

    #[test]
    fn predator_loses_chase_target_while_inside_an_ink_cloud() {
        // 墨が広がっている間、その範囲にいる捕食者は獲物を検知できない(視界が悪くなる)。
        // タンクの中央付近に配置し、壁際の可動範囲マージン(拡大されたスプライトの
        // 実サイズに応じて広がった分)による加速が測定値に混ざらないようにする。
        let (w, h) = (160, 60);
        let mut sim = Simulation::new(Rng::new(608));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 80.0, 30.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 90.0, 30.0)); // 獲物(捕食圏内)
        sim.ink_clouds.push(InkCloud {
            x: 80.0,
            y: 30.0,
            life: INK_LIFETIME,
            max_life: INK_LIFETIME,
        });

        sim.update(0.1, w, h);

        // 墨の中にいるピラニアは獲物へ向かう吸引ベクトルが働かないため、
        // 追跡している場合に比べて移動量がはっきり小さいはず。
        let moved = (sim.fish[0].vx.powi(2) + sim.fish[0].vy.powi(2)).sqrt();
        assert!(
            moved < PIRANHA_HUNT_PULL * 0.1 * 0.5,
            "墨の中では獲物を追跡できず、加速がはっきり小さいはず: moved={moved}"
        );
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
