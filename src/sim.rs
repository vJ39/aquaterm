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
// 空腹度の毎秒減少量。減少が早すぎるとの指摘を受けて大幅に緩め、
// 満タン→0まで約60分(1時間)になるよう調整した(旧: 約10分/600秒)。
// 腹ぺこ閾値(50、満タンからの半分)到達は概ね30分の計算になる。
// 弱り・死亡までの猶予(STARVE_WEAK_TIME/STARVE_DEATH_TIME)は変更しない。
pub const HUNGER_DECAY: f64 = 100.0 / 3600.0;
pub const FEED_AMOUNT: f64 = 34.0; // 餌1粒で回復する空腹度
pub const WELL_FED_THRESHOLD: f64 = 60.0; // 成長・産卵の満腹判定
pub const GROW_TIME: f64 = 600.0; // 満腹維持で稚魚→成魚(10分。旧30秒は寿命8時間に対して短すぎた)
pub const BREED_READY_TIME: f64 = 22.0; // 成魚が満腹維持でこの時間経つと産卵可能
// 産卵可能時、毎秒の産卵確率。産卵頻度が高すぎるとの指摘を受けて
// 大幅に下げた(旧0.06/秒→数十分に1回程度のペースを狙う。実機体感で調整)。
pub const BREED_CHANCE_PER_SEC: f64 = 0.0008;
pub const EGG_HATCH_TIME: f64 = 14.0; // 卵が孵化するまでの時間
// 水質悪化・浄化剤の副作用として、孵化タイミングに達した卵が孵化に失敗して
// そのまま消えることがあるようにする。pollutionがPOLLUTION_MAXのときの失敗確率が
// EGG_HATCH_FAIL_POLLUTION_MAX_CHANCE、purifier_concentrationは100%(=1.0)ごとに
// EGG_HATCH_FAIL_PURIFIER_MULTずつ失敗確率を加算する(浄化剤を連投して100%を超える
// ほど、他の副作用同様どんどん悪化する)。両者は合算し、1.0(必ず失敗)で頭打ちにする。
pub const EGG_HATCH_FAIL_POLLUTION_MAX_CHANCE: f64 = 0.6;
pub const EGG_HATCH_FAIL_PURIFIER_MULT: f64 = 0.5;
// --- つがい・交尾(新規・2026/07/15) ---
// つがい形成・交尾・産卵・羽化のアニメーションを実装してほしいという要望への対応。
// 従来は個体ごとの独立したランダム産卵(ペアリング無し)だったが、同種の産卵可能な
// 成魚2匹が近づいて「つがい」になり、
// 十分接近したら交尾→産卵、というプロセスに変更する。
pub const COURTSHIP_RADIUS: f64 = 18.0; // この距離以内に産卵可能な同種がいたら惹かれ合う
pub const COURTSHIP_PULL: f64 = 70.0; // 惹かれ合う力(通常の遊泳より少し強い程度。狂喜して追い回すほど強くはしない)
pub const MATE_RADIUS: f64 = 6.0; // この距離まで近づいたら交尾成立(産卵判定に入る)
pub const MATE_EFFECT_LIFETIME: f64 = 1.2; // 交尾演出(ハート)の表示時間
pub const HATCH_EFFECT_LIFETIME: f64 = 1.0; // 孵化(羽化)演出の表示時間
pub const STARVE_WEAK_TIME: f64 = 120.0; // 空腹度0からおよそ2分で「弱っている」
pub const STARVE_DEATH_TIME: f64 = 630.0; // 空腹度0からおよそ10.5分(>=10分)で力尽きる

// --- 成長段階(全種共通・成魚後のさらなるサイズ成長) ---
// 稚魚→成魚(Stage)とは別に、成魚になった後も満腹維持を続けると段階的にサイズが
// 大きくなる。上限を設けて無限に大きくならないようにする(0..=3の4段階)。
pub const GENERAL_MAX_GROWTH_STAGE: u8 = 3;
// 個体差(growth_cap_variance、下記)により、通常より1段階分だけ大きくまで育つ個体が
// いる。render_scale等のサイズ計算側のクランプ上限もここまで広げる。
pub const GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE: u8 = GENERAL_MAX_GROWTH_STAGE + 1;
pub const SIZE_GROW_TIME: f64 = 90.0; // 満腹維持がこの時間続くごとに1段階サイズアップ
pub const GENERAL_GROWTH_SCALE_STEP: f64 = 0.15; // 1段階あたりの見た目拡大率
// 成長段階が上がるほど、泳ぐ速度がやや遅くなる(必須ではないが体感の変化として付与)
pub const SIZE_SPEED_PENALTY_STEP: f64 = 0.05;
// タコはデフォルトで他種より大きく見せる(成長段階によるスケールとは別枠のベース倍率)。
pub const OCTOPUS_BASE_SCALE_BONUS: f64 = 0.5;
// クジラはネタ枠の巨大魚として、他のどの種よりもずば抜けて大きく見せる(成長段階による
// スケールとは別枠のベース倍率)。1.0のベースに加算されるため、見た目は通常成魚の約3.5倍になる。
pub const WHALE_BASE_SCALE_BONUS: f64 = 2.5;

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

// --- 個体差(新規・2026/07/14) ---
// 魚・タコ・ピラニアそれぞれについて、空腹の減り方や満腹時の回復量、成長サイズ、寿命に
// 個体差を持たせ、稀に大食いの個体が出るようにしてほしいという要望への対応。
// 種による違いではなく、
// 同じ種の中でも個体ごとにばらつきを持たせる。Fish::new()自体はニュートラル値(1.0/1.0/1.0/0、
// 変化なし)のままにし、Simulation側の実際の生成箇所(seed_initial/add_fish_of_species/
// add_octopus/卵の孵化)でのみランダムに割り振る(既存テスト・既存セーブの挙動は変えない)。
pub const INDIVIDUALITY_HUNGER_MULT_MIN: f64 = 0.85; // 空腹になる速さの個体差(通常個体)
pub const INDIVIDUALITY_HUNGER_MULT_MAX: f64 = 1.15;
// 「たまに大食い」個体: 通常よりはっきり空腹になりやすく、食べた時の満たされ方も大きい
// (よく食べてよく満たされる、大食いらしいキャラクター)。
pub const GOURMAND_CHANCE_PER_SPAWN: f64 = 0.08;
pub const GOURMAND_HUNGER_MULT_MIN: f64 = 1.5;
pub const GOURMAND_HUNGER_MULT_MAX: f64 = 2.0;
pub const GOURMAND_FEED_MULT_MIN: f64 = 1.3;
pub const GOURMAND_FEED_MULT_MAX: f64 = 1.6;
pub const INDIVIDUALITY_LIFESPAN_MULT_MIN: f64 = 0.8; // 寿命(ELDERLY_AGE/LIFESPAN_DEATH_AGE)の個体差
pub const INDIVIDUALITY_LIFESPAN_MULT_MAX: f64 = 1.3;

// --- 病気パラメータ ---
pub const HUNGRY_SICK_TIME: f64 = 10.0; // 腹ぺこがこの時間続くと発症判定対象
// 個体数/上限がこれ以上で過密=発症判定対象。実機フィードバックを受けて0.9→0.95に上げ、
// 上限にかなり近づかないと過密判定に入らないようにした。
pub const OVERCROWD_RATIO: f64 = 0.95;
// 発症条件下での毎秒発症確率。発症が早すぎるとの指摘を受けて
// 大幅に下げた(旧0.03/秒)。指示の目安値0.005/秒で実機計測したところ、5匹が同時に
// 発症条件を満たす状況で75秒(x4速度)以内に2匹発症してしまい、目標(最高速度で
// 水槽全体10分に1回程度)よりかなり頻発することが判明したため、実測値から逆算して
// さらに約1/60に下げた(0.005→0.0001)。
pub const DISEASE_CHANCE_PER_SEC: f64 = 0.0001;
pub const SICK_WEAK_TIME: f64 = 60.0; // 病気からおよそ1分で「弱っている」
// 空腹側(STARVE_DEATH_TIME)と同水準の猶予にする(病気だけ極端に早死にしないよう揃える)
pub const SICK_DEATH_TIME: f64 = 630.0;

// --- 水質パラメータ(新規) ---
// 薬・餌・肉餌を放置すると水槽内の生物が死にやすくなるまで水質が悪化し、餌が消費されると
// 改善する。また病気の個体や死んだ個体を放置しても水質が急激に悪化するべき、という要望
// への対応。0(綺麗)〜100(最悪)のパラメータで、堆積した食べ残し・病気・
// 死亡個体の放置で悪化し、常時弱い自然浄化で改善する。悪化がゼロなら自然浄化により
// 相対的にどんどん綺麗になっていく。
// 最悪値・悪化/浄化レートは全て同じ倍率(5倍)でスケールしてあり、フラクション
// (pollution/POLLUTION_MAX)の時間変化・タイミングは以前と同じになるようにしている。
pub const POLLUTION_MAX: f64 = 500.0;
// 全体の進行速度(悪化・浄化とも)を当初比1/40まで下げてあり、水質はゆっくり
// 変化する(ただし正味の悪化・浄化のバランス=フラクションの変化の仕方は
// 変えていないつもり。悪化・浄化を比例的に下げてバランスを保っている)。
pub const POLLUTION_PER_LANDED_FOOD: f64 = 0.015; // 水底に堆積した餌1個あたり/秒
pub const POLLUTION_PER_LANDED_MEDICINE: f64 = 0.015; // 水底に堆積した薬1個あたり/秒
pub const POLLUTION_PER_LANDED_MEAT: f64 = 0.025; // 水底に堆積した肉餌1個あたり/秒(食べ残りやすいぶん重め)
// 病気・死亡個体は堆積物より一段強い悪化レートにする(急激に悪化させる)。
pub const POLLUTION_PER_SICK_FISH: f64 = 0.05;
// 死骸放置による悪化が自然浄化(POLLUTION_NATURAL_DECAY)を大きく上回り、死骸が
// 1体いるだけで正味悪化に転じてしまうとの指摘を受け、1/10に引き下げた(旧0.1)。
pub const POLLUTION_PER_DEAD_FISH: f64 = 0.01;
pub const POLLUTION_NATURAL_DECAY: f64 = 0.075; // 常時かかる自然浄化(悪化要因が無ければ改善していく)
// 捕食が成立した瞬間に一気に加算する悪化量(血肉が水中に飛び散る一発イベントの
// ため、継続的なレートではなく単発の大きな加算にしてある)。
pub const POLLUTION_PREDATION_SPIKE: f64 = 0.375;
// 病気の発症確率(DISEASE_CHANCE_PER_SEC)にかかる乗数の上限。水質最悪(POLLUTION_MAX)で
// 発症確率が最大でこの倍になる(水質の悪化を病気ルート経由で生存に影響させる仕組み)。
pub const POLLUTION_SICK_CHANCE_MAX_MULT: f64 = 4.0;
// 水質がこの割合(POLLUTION_MAXに対する比率)以上悪化すると、腹ぺこ継続・過密で
// なくても発症判定の対象に含める(満腹を保ってさえいれば水質が最悪でも病気に
// ならない、という抜け穴を防ぐため)。
pub const POLLUTION_SICK_ELIGIBLE_FRAC: f64 = 0.5;
// 水質が悪化している間、捕食者(ピラニア・タコ)以外の通常種は食欲そのものを
// 失う(空腹度の減りが速まり、update_movement側では餌を探して寄っていく
// 誘引ベクトルも止める)。水質最悪でHUNGER_DECAYがこの倍になる。
pub const POLLUTION_HUNGER_DECAY_MAX_MULT: f64 = 3.0;

// --- 水流(水槽内を回転する渦・トルネード) ---
// 水槽内をゆっくり周遊する中心点のまわりに、位置に応じて向きが変わる接線方向の
// 力場を作る。全体を同じ向きに押す1本のベクトルではないため、水槽の場所ごとに
// 押される向きが異なり、魚が片側の壁際に溜まり続けにくい。魚の遊泳・落下中の
// 餌/薬/肉餌・気泡のドリフト・藻の揺れに、小さな加速度として作用する(見た目だけの
// 演出ではなく実際に位置を動かす)。
// 水流を導入してもなお画面端に魚が滞留するとの再指摘を受けて、CURRENT_FISH_MULT・
// CURRENT_STRENGTH・CURRENT_FALLOFF_RADIUS・CURRENT_CENTER_MARGIN_FRACのバランスを
// 洗い直した。調査の結果、水流(渦)の中心が壁際まで実際に寄れるかどうか自体は
// 大きな要因ではなく、実機の滞留は主に「同種の群れ(schooling)が壁際で減速する
// 個体を含むと、その平均速度に他個体も引き寄せられて壁に張り付いたまま集団で
// 居座り続ける」ことが主因と判明した(多数の魚を長時間走らせる新規テストで、
// 水流側だけ強めても滞留率がほとんど下がらないことを確認済み)。そのため水流の
// パラメータ自体は既存のバランス(遠方でほぼ無風になる減衰カーブ含む)を保った
// ままとし、実効的な対策はupdate_movement側の壁際の反発(wall_push)を「margin
// 直下でだけ立つ硬い壁」から「margin手前から緩やかに立ち上がる壁」に変更する
// 側で行った(壁に張り付く前に早めに向きを変えさせ、群れが壁に固まりにくくする)。
pub const CURRENT_STRENGTH: f64 = 8.0; // 渦の接線方向の力の強さ(px/秒²相当の加速度)
// 渦(トルネード)の中心が水槽内をゆっくり周遊する周期。X/Yで別周期にすることで
// リサージュ曲線状にゆっくり動き回り、単純な往復にならないようにする。
pub const CURRENT_CENTER_DRIFT_PERIOD_X: f64 = 240.0;
pub const CURRENT_CENTER_DRIFT_PERIOD_Y: f64 = 180.0;
// 中心が水槽の縁ぎりぎりまで寄りすぎないための余白(水槽サイズに対する比率)。
pub const CURRENT_CENTER_MARGIN_FRAC: f64 = 0.25;
// 魚自身の遊泳意思(壁の反発・餌への吸引・逃走等)が水流に負けて自由に動けなくなり
// 壁際に滞留する、との実機フィードバックへの対応。魚への適用だけ半分の強さにし、
// 魚が遊泳意思で水流に逆らって自由に行き来できるようにする。他の要素
// (餌・薬・肉餌・気泡・藻の揺れ・水流の筋)は水流の存在感を出すため全力のまま。
pub const CURRENT_FISH_MULT: f64 = 0.5;
// 渦の中心付近だけ強く、離れるほど指数関数的に弱まる半径(トルネードの目のように
// 中心近くだけ渦巻き、水槽の大部分は穏やかにするための減衰)。中心が水槽内を
// 漂うため、その場その場では大半の時間は水流をほとんど感じず自由に泳げるが、
// 渦の目が通りかかると一時的に強く巻き込まれる、という体感にする。
pub const CURRENT_FALLOFF_RADIUS: f64 = 22.0;
// 水流を可視化する筋(CurrentStreak)の生成間隔・寿命。
pub const CURRENT_STREAK_SPAWN_INTERVAL_MIN: f64 = 1.2;
pub const CURRENT_STREAK_SPAWN_INTERVAL_MAX: f64 = 2.5;
pub const CURRENT_STREAK_LIFETIME: f64 = 3.5;

// --- 死亡演出パラメータ ---
// 死んだ魚は、体内のガスによる浮力(時間とともに減衰する)と重力・水の抵抗を
// 簡易的に力学計算し、「最初は浮いて漂うが、やがて浮力が失われて沈み、水底の
// 亡骸になる」という連続的な動きにしている(状態を切り替えるのではなく、
// 毎tick加速度から速度・位置を積分する)。水底の亡骸はCRAB_EAT_RADIUS以内に
// カニが来ると片付ける(分解演出つき)。カニが来なければCORPSE_REMOVE_TIMEの
// 経過で自動的に消える。
pub const DEAD_SURFACE_MARGIN: f64 = 3.0; // 水面からこの位置まで浮いたら静止する
// 死亡直後の浮力(上向き加速度)。重力より大きく、最初ははっきり浮上する。
pub const CORPSE_BUOYANCY_INITIAL: f64 = 10.0;
// 常時かかる重力(下向き加速度)。浮力が重力を下回った時点で沈み始める。
pub const CORPSE_GRAVITY_ACCEL: f64 = 5.0;
// 浮力が指数関数的に減衰する時定数(秒)。この値だと死亡からおよそ3分
// (180秒=DEAD_FLOAT_TIMEの目安)で浮力が重力を下回り、沈み始める計算になる。
pub const CORPSE_BUOYANCY_DECAY_TAU: f64 = 260.0;
// 本番ロジックでは閾値として直接は使わない(浮力減衰の連続計算に委ねている)が、
// おおよその沈み始めタイミングの目安としてテスト・ドキュメントで参照する。
#[allow(dead_code)]
pub const DEAD_FLOAT_TIME: f64 = 180.0;
pub const CORPSE_REMOVE_TIME: f64 = 86400.0; // 死亡からこの時間(約24時間)経過で自動的に消える
// 速度の減衰(水の抵抗)。大きいほど目標速度への追従が速く、振動しにくくなる。
pub const CORPSE_DRAG_PER_SEC: f64 = 1.0;
// 漂っている間・沈んでいる間、左右にゆらゆらと揺れる動きのパラメータ
// (餌・薬の蛇行(SPRINKLE_SWAY_*)より緩やかで、力なく漂う印象にしてある)。
pub const DEAD_SWAY_AMPLITUDE: f64 = 2.5;
pub const DEAD_SWAY_ANGULAR_SPEED: f64 = 1.4;
// カニが亡骸を片付けた瞬間に出す分解演出の持続時間
pub const CORPSE_DECOMPOSE_EFFECT_LIFETIME: f64 = 1.5;

// --- 観賞用の追加生物(育成ロジック対象外。カニ・エビ・タツノオトシゴ。
// 大型魚は Species::Piranha として通常の育成対象に統合された) ---
pub const CRAB_COUNT: usize = 3;
pub const CRAB_SPEED: f64 = 3.0; // 水底を歩く速さ
pub const CRAB_PAUSE_CHANCE_PER_SEC: f64 = 0.15; // 毎秒、立ち止まる確率
pub const CRAB_EAT_RADIUS: f64 = 3.0; // カニが水底の餌・薬を片付けられる距離

// エビ: カニと同じ位置づけの観賞用背景生物(育成ロジック対象外・捕食対象外・
// 自身も捕食しない)。挙動もカニと同様(左右に歩き、時々立ち止まる)でよい。
pub const SHRIMP_COUNT: usize = 2;
pub const SHRIMP_SPEED: f64 = 2.0; // カニよりゆっくり歩く
pub const SHRIMP_PAUSE_CHANCE_PER_SEC: f64 = 0.25; // カニより頻繁に立ち止まる

// タツノオトシゴ: 藻に絡みつくようにゆっくり動き、あまり大きく移動しない
// (藻の周辺に留まる傾向)。育成ロジック対象外・捕食対象外・自身も捕食しない。
pub const SEAHORSE_COUNT: usize = 2;
pub const SEAHORSE_DRIFT_AMPLITUDE: f64 = 3.0; // 基準位置(藻の近く)からの振れ幅
pub const SEAHORSE_DRIFT_FREQ: f64 = 0.25; // ゆらゆら動く速さ(rad/秒相当。カニよりゆっくり)

// --- 藻・水草・岩・タコつぼ(装飾。育成ロジックには参加しない静的な背景オブジェクト) ---
pub const PLANT_COUNT: usize = 5; // 水底に配置する藻・水草の本数
pub const PLANT_SWAY_FREQ: f64 = 1.2; // 揺れの速さ(見た目のみ・rad/秒相当)
pub const ROCK_COUNT: usize = 3; // 水底に配置する岩の数
pub const DEN_COUNT: usize = 1; // タコつぼの数(タコの数と1:1で対応する)
// 魚が藻・水草・岩に近いとき、視覚的に「隠れている」表現(色を背景に馴染ませる)にし、
// かつ実際にピラニア・タコから捕食対象にならなくなる距離。藻・岩を魚が隠れられる大きさに
// し、隠れた状態を実際の捕食免除として機能させてほしいという要望を受けて、旧サイズ
// (単一の細い水草・6.0)から段階的に拡大してきた(6.0→9.0→14.0)。まだ小さいとの
// 再指摘を受けて、藻・岩の見た目の拡大(height/ROCK_SCALE)に合わせてさらに広げた。
pub const ALGAE_HIDE_RADIUS: f64 = 20.0;
pub const ALGAE_HIDE_MIX: f64 = 0.55; // 隠れ表現の強さ(背景色へどれだけ寄せるか)

// --- タコの隠れる/出てくる状態遷移 ---
// 低頻度でつぼから出てきて泳ぎ、しばらくしたら戻る。追跡している様子を観察する機会を
// もっと増やしたいとの指摘を受けて、出ている時間をさらに長めに(旧20〜40秒→30〜55秒)、
// 隠れている時間をさらに短めに(旧15〜40秒→10〜25秒)調整した。
pub const OCTOPUS_HIDDEN_TIME_MIN: f64 = 10.0;
pub const OCTOPUS_HIDDEN_TIME_MAX: f64 = 25.0;
pub const OCTOPUS_EMERGE_TIME_MIN: f64 = 30.0;
pub const OCTOPUS_EMERGE_TIME_MAX: f64 = 55.0;
// 出ている残り時間がこの値未満になったら、巣へ戻る引力をかけて泳いで戻る様子を見せる
// (時間切れの瞬間に確実に隠れさせる処理自体は update_octopus() 側で保証している)。
pub const OCTOPUS_RETURN_WINDOW: f64 = 4.0;
// 生き物の基本移動速度を(シミュレーション再生速度とは別に)全体的に4倍にすべきという
// 要望を受けて、fish.rsのmax_speed()/wander()/food_pull()と同じ考え方で4倍にした(旧60.0)。
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
// 追跡している様子をもっと目立たせたいとの要望を受けて、検知範囲をピラニアと同じ
// 広さに揃えた(旧20.0→PIRANHA_HUNT_RADIUSと同じ30.0)。
pub const OCTOPUS_HUNT_RADIUS: f64 = 30.0;
// 壁際で捕食できず振動するとの指摘を受けてピラニア側と同様に広げた(旧3.0→4.5)。
// さらに口基準にした上で判定距離自体も広げるべきとの指摘を受けて再度拡大した
// (旧4.5→7.0)。さらにタコの当たり判定もピラニアと同様にもっと広げてほしいという
// 指摘を受けて再度拡大した(旧7.0)。壁際で反発力と吸引力が拮抗して詰め切れない問題の
// 緩和策の一つでもある。
pub const OCTOPUS_STRIKE_RADIUS: f64 = 10.0;
pub const OCTOPUS_HUNT_COOLDOWN: f64 = 20.0;
pub const OCTOPUS_PREDATION_HUNGER_GAIN: f64 = 60.0;
// 追跡中も速さが体感できないとの指摘を受けてピラニア側と同様に強化(旧90.0)。
// さらに基本移動速度を全体的に4倍にする方針を受けて4倍にした(旧140.0)。さらに追跡を
// もっと目立たせたいとの要望を受けて、吸引の強さもピラニアと同じ値に揃えた
// (旧560.0→PIRANHA_HUNT_PULLと同じ640.0)。
pub const OCTOPUS_HUNT_PULL: f64 = 640.0;

// --- 墨(タコがピラニアに追われると吐く) ---
// タコの近くに捕食モードのピラニアがいたら、逃走(既存のfear_target経由)に加えて墨を吐く。
pub const OCTOPUS_INK_TRIGGER_RADIUS: f64 = 26.0; // この距離以内に捕食モードのピラニアがいたら墨を吐く
// 捕食モードのピラニアに限らず、種類を問わず魚がすぐ目の前まで近づいてきた場合にも
// 墨を吐く。遠くの脅威に反応するピラニア用の半径より狭くし、「触れそうな距離まで
// 寄られた」ときの反応として使う。
pub const OCTOPUS_INK_NEARBY_FISH_RADIUS: f64 = 12.0;
pub const INK_COOLDOWN: f64 = 20.0; // 連発防止のクールダウン
// 墨のエフェクト: 血の滲みより広め・勢いよく拡散し、数秒(目安3〜5秒)残ってから薄れて消える。
pub const INK_LIFETIME: f64 = 4.5;
pub const INK_GROWTH_TIME: f64 = 1.2; // 血より大幅に速く広がる(「わーーーっと」拡散するイメージ)
// 色が薄く範囲も狭いとの指摘を受けて、もっと濃く広範囲に真っ黒になるようさらに拡大した(旧28.0)。
pub const INK_MAX_RADIUS: f64 = 42.0; // 血の滲み(20.0)より広め
// 同フィードバックを受けて、ほぼ完全に塗りつぶすレベルまで混合率を上げた(旧0.9)。
pub const INK_MIX: f64 = 0.98;
pub const INK_HOLD_FRACTION: f64 = 0.35;
// 浄化剤の着水演出(浄化ブルーム): 墨と同じ「同心円状に勢いよく広がって薄れて消える」
// 構造を使い、墨に近い派手さで一気に広がる明るい水色の演出にする(main.rs側で専用の
// 色・半径定数を使う)。着水した瞬間に水底から一気に広がる。
// 着水直後は勢いよくではなく、もわっと広がってしばらく尾を引くように長めにした
// (墨のような一瞬の勢いではなく、じわじわ長く漂う拡散にしてほしいという指摘への対応)。
// 着水地点から水槽全体を覆うまで同心円状に広がる紫の波(main.rs側の描画で
// 対角線の長さを基準に半径を計算する)にかける時間。この間は着水地点ほど濃く、
// 波の先端はグラデーションでぼかす。広がりきった瞬間に効果(濃度加算)が発動する。
pub const PURIFY_BLOOM_LIFETIME: f64 = 14.0;
pub const PURIFY_BLOOM_GROWTH_TIME: f64 = 6.0;
// 墨が広がっている間、その範囲にいる捕食者(ピラニア等)は獲物を検知できなくなる
// (「視界が悪くなる」演出。捕食者側のchase_target判定を一時的に無効化する)。
// 墨を吐いたら高確率で逃げ切れるという結果まで保証してほしいという要望を
// 受けて、視界不良(検知不能)だけでなく以下も組み合わせる:
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
// 山として認識できるレベルまで最大高さを上げてほしいという要望を
// 受けて、旧3.0の2〜3倍程度である8.0まで引き上げた(視覚的インパクトを強化)。
pub const PILE_MAX_HEIGHT: f64 = 8.0;

// --- ピラニアの捕食 ---
// 捕食頻度が低すぎるとの指摘を受けて方針転換: 従来の「頻度は控えめに」
// から「ピラニアは頻繁に狩る」体感になるよう、閾値・クールダウン・検知範囲をまとめて強化した。
// (捕食1回の空腹度回復量=PIRANHA_PREDATION_HUNGER_GAIN=70が大きいため、閾値を上げないと
// 1回捕食するたびに長時間満腹状態が続いてしまう。閾値を引き上げることで、捕食後の
// 「次に狩れるようになるまでの実質的な待ち時間」を大幅に短縮している。
// 実機計測: 閾値95では捕食1回ごとに満腹(100)から95まで下がるのに約3分かかり、
// 「頻繁」というには程遠かったため、99まで引き上げて待ち時間を約36秒まで圧縮した)
pub const PIRANHA_HUNT_HUNGER_THRESHOLD: f64 = 99.0; // 空腹度がこれ未満のときだけ捕食行動を取る(旧70)
pub const PIRANHA_HUNT_RADIUS: f64 = 30.0; // この距離以内の獲物へ近づいていく(旧22)
// 壁際に追い詰めた魚を永遠に捕食できないとの指摘を受けて拡大した
// (旧3.5→5.0)。壁際では反発力(wall_push)と追跡吸引力(PIRANHA_HUNT_PULL)が拮抗して
// 詰め切れず振動する現象があったため、これに加えて壁際での反発力自体を弱める・
// 吸引力を強めるの3点セットで対応している(update_movement側を参照)。
// さらに口基準にした上で判定距離自体も、狙った獲物にきちんと届くようもっと
// 広くすべきとの指摘を受けて再度拡大した(旧5.0)。
pub const PIRANHA_STRIKE_RADIUS: f64 = 8.0;
// 獲物へ近づく吸引の強さ。「追いかけている」のが見た目でわかるよう、通常の遊泳を
// 弱めた上でこれを強くかける(旧26.0→100.0→さらに強化)。追跡中も速さが体感できない
// という指摘を受け、最高速度のクランプ(PIRANHA_CHASE_SPEED_MULT)
// だけでは短い追跡では速度差が体感できないため、加速度自体もさらに強化した。
// さらに基本移動速度を全体的に4倍にする方針を受けて4倍にした(旧160.0)。
pub const PIRANHA_HUNT_PULL: f64 = 640.0;
pub const PIRANHA_HUNT_COOLDOWN: f64 = 15.0; // 捕食後、次に捕食できるようになるまでの時間(旧45)
// ピラニアは追跡(捕食モード)中だけ、通常3種の最高速度(最速はネオンの22.0)より
// はっきり速くなるようにする倍率。巡回中(追跡していないとき)は通常のsp.max_speed()
// のままで特別早くしない。魚が機敏に逃げても追いつかれることがある緊張感を出す。
pub const PIRANHA_CHASE_SPEED_MULT: f64 = 1.8;
pub const PIRANHA_PREDATION_HUNGER_GAIN: f64 = 70.0; // 捕食による空腹度の回復量(餌より効率的。殺した(3発目)ときの全回復量)
// 噛みついて殺すまで至らなかった噛みつき(1発目・2発目)は、殺した(3発目)ときと
// 同じだけ満腹になってしまっていた(1発でPIRANHA_PREDATION_HUNGER_GAIN分そのまま
// 回復)。生かしたまま弱らせるだけの浅い一噛みが、殺すのと同じ満腹効果を持つのは
// 不自然という指摘への対応で、殺すまでの噛みつきは全回復量の1/3だけ回復する
// 「部分満腹化」にした(3発目=殺した瞬間だけ従来通りの全回復)。
pub const PIRANHA_PARTIAL_BITE_HUNGER_FRAC: f64 = 1.0 / 3.0;
// 食欲をもっと旺盛にし、3匹食べないと満腹にならないようにしてほしいという要望への対応。
// PIRANHA_PREDATION_HUNGER_GAINが大きく1回の捕食でhungerが即座に満腹相当まで戻るため、
// hungerの閾値だけでは「1匹食べたら即やめる」動きになってしまう。そこで、満腹になって
// からの捕食数(piranha_meals_since_full)を別途カウントし、これがこの値に達するまでは
// hungerが満腹相当でも狩りをやめないようにする。
pub const PIRANHA_KILLS_TO_FULL: u32 = 3;
// ピラニアが食欲がなくても魚を追いかけまわし体力を減らしてしまうという指摘への対応。
// 上記の「3匹食べるまでやめない」にタイムアウトが無かったため、
// 1匹食べた後、獲物が上手く逃げ続けて2匹目・3匹目を捕食できないままだと、
// piranha_meals_since_fullが永久にリセットされず、hungerが満腹でも無限に追跡し
// 続けてしまっていた(見た目の空腹度は満タンなのに、内部的にはまだ狩りをやめない
// 状態が続く)。このグレースピリオドを超えて2匹目・3匹目を捕食できなかった場合は、
// 諦めてmeals_since_fullを0に戻し、通常のhunger基準の狩りだけに戻す。
pub const PIRANHA_QUOTA_GRACE_PERIOD: f64 = 60.0;
// ピラニアのカニバリズム(新規): 「大きいピラニアは小さいピラニアを食ってもいい」という要望を受け、
// 同種(ピラニア)を無条件に対象外とするのではなく、十分サイズ差(成長段階+捕食成長段階の
// 合計)があるときだけ対象に含める。近いサイズ同士は対象外のまま(共食いの乱発を防ぐ)。
pub const PIRANHA_CANNIBALISM_MIN_SIZE_ADVANTAGE: i32 = 2;
// ピラニアに噛まれた回数がこの値に達すると死亡演出に入る(1発ごとに弱る3段階制)。
pub const PIRANHA_BITES_TO_KILL: u8 = 3;
// 噛まれてから何もなければ、この間隔(秒)ごとにpiranha_bite_countが1段階ずつ回復する。
// PIRANHA_HUNT_COOLDOWN(15秒)より十分長くして、居座る1匹のピラニアが連続で
// 弱らせ切れる程度の猶予は残しつつ、逃げ切れれば時間経過で癒えるようにする。
pub const PIRANHA_BITE_RECOVER_INTERVAL: f64 = 45.0;
// 負傷段階に応じた遊泳速度の倍率(弱るほど逃げ足が遅くなり、追加で噛まれやすくなる)。
// インデックスはpiranha_bite_count(0=無傷)に対応する。
pub const PIRANHA_BITE_SPEED_MULT: [f64; 3] = [1.0, 0.8, 0.55];
// タコをかじって弱らせる仕組み(ピラニアの被噛みつきと対になる、役割が逆の仕組み)。
// かじる側は生きている成魚全体(ピラニアを含む、種を問わない)。5回かじられると死亡する。
pub const OCTOPUS_BITES_TO_DIE: u8 = 5;
// かじられてから何もなければ、この間隔(秒)ごとにoctopus_bite_countが1段階ずつ回復する。
pub const OCTOPUS_BITE_RECOVER_INTERVAL: f64 = 45.0;
// かじられた段階に応じた遊泳速度の倍率(弱るほど逃げ足が遅くなる)。
// インデックスはoctopus_bite_count(0=無傷)に対応する。段階が多い(5段階)ぶん
// ピラニアの3段階よりなだらかに落ちるようにする。
pub const OCTOPUS_BITE_SPEED_MULT: [f64; 5] = [1.0, 0.85, 0.7, 0.55, 0.4];
// かじる側(成魚)がタコに近づいてかじりつく距離。
pub const OCTOPUS_BITE_RADIUS: f64 = 10.0;
// 近くにいる間、毎秒この確率でかじられる(乱発防止の免疫時間と合わせて頻度を調整する)。
pub const OCTOPUS_BITE_CHANCE_PER_SEC: f64 = 0.15;
// 一度かじられたら、この時間はどの魚からも新たなかじり判定を受けない
// (同時に何匹もいる場合に一瞬で殺されてしまわないようにする猶予)。
pub const OCTOPUS_BITE_IMMUNITY_TIME: f64 = 3.0;
// 血飛沫演出: もっと派手・グロテスクに強化してほしいという要望を受けて、
// 単一の一瞬エフェクトから、複数粒子が散らばって尾を引くように少しずつ消える演出に強化した。
pub const BLOOD_EFFECT_LIFETIME: f64 = 1.6; // 表示時間(旧0.5秒→1〜2秒程度に延長)
pub const BLOOD_PARTICLE_COUNT: usize = 10; // 捕食1回あたりに散らす粒子数(旧: 1個のみ)
pub const BLOOD_SPREAD_RADIUS: f64 = 6.0; // 粒子が散らばる範囲(旧の波紋演出より広め)
// 負傷中(piranha_bite_count>0)の魚が、噛まれた瞬間の大きな血飛沫とは別に、
// 治るまでの間ずっと少量の血を滲ませ続けるための演出パラメータ。
pub const BLEED_TRICKLE_INTERVAL_MIN: f64 = 1.5; // 次の血だまりを出すまでの間隔(秒)の最小
pub const BLEED_TRICKLE_INTERVAL_MAX: f64 = 3.0; // 同・最大
pub const BLEED_TRICKLE_PARTICLE_COUNT: usize = 2; // 1回に出す粒子数(噛みつき時のBLOOD_PARTICLE_COUNTより少なめ)
// 血の滲み(範囲エフェクト): 捕食位置の周辺に赤みが水中に広がる演出。パーティクルより
// 長く残り、時間とともにゆっくりフェードアウトする(既存の水槽グラデーションに赤を
// 混ぜて表示するイメージ)。
// 拡散が速すぎるとの指摘を受けて、総表示時間を4.0→6.0秒に延ばし、
// 拡大にかける時間(BLOOD_STAIN_GROWTH_TIME)も総寿命の中でのウェイトを増やして
// もっとゆっくり広がる感じにした(拡大自体の実時間を伸ばす方向で調整)。
pub const BLOOD_STAIN_LIFETIME: f64 = 6.0;
// グロテスクさが足りないとの指摘を受けて、固定半径のまま薄く
// フェードするだけの実装から、時間経過で半径が広がっていく同心円の波紋アニメーション
// に変更した。最大半径は旧定数(7.0)の約3倍・混色の強さも旧0.6→0.85に強化する
// (実際の拡大計算・混色計算は描画側=main.rsのdraw_species_dex付近で行う)。
pub const BLOOD_STAIN_MAX_RADIUS: f64 = 20.0;
// 半径が0→最大まで広がるのにかける実時間。BLOOD_STAIN_LIFETIME全体を使わず専用の
// タイマーにすることで、寿命を延ばさなくても拡大速度自体を遅くできるようにしている
// (寿命末期は最大半径のまま残り、フェードアウトだけが進む)。
pub const BLOOD_STAIN_GROWTH_TIME: f64 = 4.5;
// 数秒間まっかに見えるくらい濃くすべきとの指摘を受けて0.85→0.93まで強化。
// はっきりインパクトのある赤にする(控えめにしない)。
pub const BLOOD_STAIN_MIX: f64 = 0.93;
// 発生から寿命のこの割合までは、広がりながらも混色の強さを最大近くで維持する
// (「数秒間まっか」に見せるための保持区間)。残りの区間でフェードアウトする。
pub const BLOOD_STAIN_HOLD_FRACTION: f64 = 0.5;

// --- 血の匂い(新規): ピラニアが噛みついて出血させた瞬間(獲物を殺した時・殺すまで
// 至らなかった負傷時のいずれも)、その位置に「血の匂い」が発生したものとみなす。
// 見た目の血の滲み(BloodStain)とは別に、ピラニアだけが追跡できる無形のソースとして
// 管理し、時間経過で薄れて消える。満腹中・クールダウン中のピラニアも含め、検知範囲内の
// 全ピラニアがその出血元を優先的に追いかけるようにする(既存のchase_target=獲物本体の
// 追跡とは独立な、追加の吸引ベクトルとして働く)。
pub const BLOOD_SCENT_LIFETIME: f64 = 20.0; // 匂いが薄れて消えるまでの時間
// 視覚(PIRANHA_HUNT_RADIUS=30.0)より「匂い」は遠くまで届くという想定で、検知範囲を広めにする。
pub const PIRANHA_BLOOD_SCENT_RADIUS: f64 = 60.0;
// 優先的に追跡するのがわかるよう、通常の狩りの追跡(PIRANHA_HUNT_PULL)と同程度の
// 強さにする(満腹中・クールダウン中でもこれだけは効く)。
pub const PIRANHA_BLOOD_SCENT_PULL: f64 = PIRANHA_HUNT_PULL;

// 通常の魚が「今まさに捕食モードのピラニア」を検知して逃げる距離・強さ。
// 空腹でない/クールダウン中のピラニアは対象にならない(気にせず普段どおり泳ぐ)。
pub const PIRANHA_FEAR_RADIUS: f64 = 26.0;
// 基本移動速度を全体的に4倍にする方針を受けて4倍にした(旧90.0)。
pub const PIRANHA_FEAR_STRENGTH: f64 = 360.0;
// ピラニアが近くにいるのを検知しても実際にはフラフラ近づいてしまう(危険域に
// 入ってしまう)という指摘への対応。以前は逃走の強さを検知距離(dist)に比例させて
// おり、検知範囲(PIRANHA_FEAR_RADIUS)の縁ではdist≈radiusで強さがほぼ0になる
// ため、そこではランダムウォーク・群れ行動(通常の遊泳)の方が実質的に勝って
// しまい、検知していても危険域へ流されていくことがあった。距離に応じた強弱は
// 残しつつ、下限としてこの割合(PIRANHA_FEAR_STRENGTH比)は縁でも必ず確保する。
pub const PIRANHA_FEAR_MIN_STRENGTH_FRAC: f64 = 0.45;
// ピラニアから逃走中は、通常の遊泳(ランダムウォーク・群れ)をHUNGRY_NORMAL_MOVE_DAMP
// よりさらに強く抑え、逃走ベクトルが確実に他の意思決定へ勝つようにする
// (「検知したら確実に距離を取る」ことを保証するための減衰)。
pub const PIRANHA_FEAR_MOVE_DAMP: f64 = 0.05;

// --- スター(無敵アイテム、ネタ機能)。マリオのスターのようなギミック。取得できる
// のは捕食者でない通常種のみで、無敵中は誰からも捕食されず、逆に触れた捕食者
// (ピラニア・タコ)を倒せる(通常の魚同士を襲うことはない)。ランダムな自然発生は
// させず、`Z`キー(debug_spawn_star)経由でのみ出現する。 ---
pub const STAR_LIFETIME: f64 = 45.0; // 誰も取りに来ないまま経過すると消える
// デバッグ投入(`Z`キー)時、カーソル位置から最低限ずらす距離。カーソルもスターも
// 同じ十字形を描くため、ずらさないとカーソルに完全に隠れて見えなくなる。
pub const STAR_CURSOR_OFFSET: f64 = 4.0;
// カーソル周辺に散らす追加の距離の幅(何個も押して増やせるよう、毎回ランダムな
// 方向・距離に置いて重なりにくくする)。
pub const STAR_SPAWN_SCATTER_RADIUS: f64 = 6.0;
pub const STAR_PICKUP_RADIUS: f64 = 5.0; // 魚がスターに触れて取得できる距離
// スター(まだ取得していない側)への誘引の強さ。バグ修正: 以前はこの誘引ベクトルが
// 存在せず、偶然の遊泳で触れない限り誰もスターに近づいて行かなかった。ピラニアの
// 狩りと同程度の強さにして、はっきり泳いで向かっている様子が見えるようにする。
pub const STAR_ATTRACT_PULL: f64 = PIRANHA_HUNT_PULL;
pub const STAR_INVINCIBLE_DURATION_MIN: f64 = 60.0; // 1分固定
pub const STAR_INVINCIBLE_DURATION_MAX: f64 = 60.0;
// 無敵中、捕食者(ピラニア・タコ)を検知して積極的に追いかけ回すための吸引パラメータ。
// ピラニアの狩りと同程度の強さにして、はっきり追い回している様子が見えるようにする。
pub const STAR_HUNT_RADIUS: f64 = PIRANHA_HUNT_RADIUS;
pub const STAR_HUNT_PULL: f64 = PIRANHA_HUNT_PULL;
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

// --- カメオ生物(ウミガメ・クラゲ・小魚の群れ。完全観賞用) ---
// 低頻度で画面の端に出現し、反対側の端までゆっくり通過して消える。育成ロジック・
// 捕食判定のいずれにも参加しない(見た目のバリエーションを増やすためだけの存在)。
// タコの低頻度出現と同じ考え方で、同時に1匹だけに絞って抽選する(Starと同様)。
pub const CAMEO_SPAWN_CHANCE_PER_SEC: f64 = 1.0 / 400.0;
pub const CAMEO_SPEED_MIN: f64 = 6.0; // 水平方向の移動速度(px/秒)
pub const CAMEO_SPEED_MAX: f64 = 14.0;
pub const CAMEO_BOB_FREQ: f64 = 0.6; // 縦のふらつきの速さ(rad/秒相当)
pub const CAMEO_BOB_AMPLITUDE: f64 = 3.0; // 縦のふらつきの振れ幅(論理ピクセル)
// 画面外に十分出たら消える、というマージン(スプライトが完全に見えなくなってから消す)
pub const CAMEO_DESPAWN_MARGIN: f64 = 20.0;

// --- 餌・薬・気泡パラメータ ---
pub const FOOD_SINK_SPEED: f64 = 7.0; // 餌の沈降速度(px/秒)
pub const FOOD_LIFETIME: f64 = 26.0; // 餌の寿命(秒)
pub const EAT_RADIUS: f64 = 3.2; // 魚が餌を食べられる距離
pub const MED_SINK_SPEED: f64 = 5.0; // 薬の沈降速度
pub const MED_LIFETIME: f64 = 26.0; // 薬の寿命
pub const CURE_RADIUS: f64 = 3.2; // 病気の魚が薬で治る距離
// 餌・薬が沈みながら左右に揺れる(螺旋階段のようにサラサラと蛇行する)動きのパラメータ。
// 揺れの位相は粒ごとにランダムな初期値を持つため、複数粒がバラバラに揺れて見える。
pub const SPRINKLE_SWAY_AMPLITUDE: f64 = 4.0; // 揺れの左右の振れ幅(px)
pub const SPRINKLE_SWAY_ANGULAR_SPEED: f64 = 3.2; // 揺れの角速度(rad/秒。大きいほど速く蛇行する)
// ピラニア専用の肉餌(`M`キー)。餌より重みのある肉の塊のイメージで、餌よりゆっくり沈む。
pub const MEAT_SINK_SPEED: f64 = 4.0;
pub const MEAT_LIFETIME: f64 = 40.0; // 肉餌の寿命(秒)。ピラニアが自動では狩らないぶん長めに漂う
// 肉餌1個で回復する空腹度の基準量(feed_efficiency_multを乗算する。標準個体=1.0なら
// MAX_HUNGERに達するので実質満腹保証、個体差で満たされ方が控えめな個体は届かない場合もある)。
pub const MEAT_SATIATION_AMOUNT: f64 = MAX_HUNGER;

// --- 浄化剤(`C`キー)パラメータ ---
// 薬と同程度の沈降速度で沈み、着水した瞬間に水質を一気に浄化する劇薬。着水後は
// 停留せず即座に消え、代わりに「浄化剤の濃度」という全体状態を最大(1.0)に立てる。
pub const PURIFIER_SINK_SPEED: f64 = 5.0; // 薬(MED_SINK_SPEED)と同程度の沈降速度
// 浄化剤の効果は着水直後が最大(1.0)で、この時間(秒)をかけて線形に0まで薄まる。
pub const PURIFIER_DILUTION_TIME: f64 = 600.0; // 10分
// 濃度100%のときの秒間浄化量(水質を直接押し下げる)。
pub const PURIFIER_MAX_CLEAN_RATE: f64 = 3.0;
// 濃度100%のときの、通常種(捕食者以外)の空腹減衰倍率。水質最悪時の
// POLLUTION_HUNGER_DECAY_MAX_MULTと同じ考え方(食欲不振)を、水質とは別の
// 発生源(浄化剤の濃度)からも起こせるようにする。
pub const PURIFIER_HUNGER_DECAY_MAX_MULT: f64 = 3.0;
// 濃度100%のときの老化速度の倍率(全種対象)。1.0+この値の効果が乗り、
// 2.0倍で老化が2倍速く進む=実質的に寿命が半分になる。強力な浄化の代償として
// 全生物の老化を早める劇薬という設計。
pub const PURIFIER_LIFESPAN_MAX_MULT: f64 = 2.0;

pub const NEIGHBOR_RADIUS: f64 = 16.0; // 群れ判定の近傍距離

// 死骸(dead=trueの個体)を怖がって近づかないようにするための忌避力。
// ピラニア・タコからの逃走(FLEE_STRENGTH/PIRANHA_FEAR_STRENGTH)ほど強くはせず、
// あくまで本能的に距離を置く程度の弱い力にとどめる。
pub const CORPSE_AVOID_RADIUS: f64 = 12.0;
pub const CORPSE_AVOID_STRENGTH: f64 = 90.0;
// 腹ぺこ時、餌への吸引ベクトルを通常の遊泳ベクトルよりはっきり優先させるための係数。
// 腹ぺこの魚がはっきりと餌に寄ってくるようにしてほしいという要望への対応。
pub const HUNGRY_FOOD_PULL_BOOST: f64 = 4.0; // 吸引ベクトル自体の倍率
pub const HUNGRY_NORMAL_MOVE_DAMP: f64 = 0.2; // 餌を追っている間、ランダムウォーク/群れを弱める倍率

// 餌やり(`f`キー・自動餌やり共通)の投下量設定。最大投下時は水槽の1/3程度が埋まる
// 量になるようにしている。5段階(0..=4)。各レベルの粒数は10倍に再調整済み。
pub const FEED_AMOUNT_LEVELS: usize = 5;
pub const FEED_AMOUNT_DEFAULT: usize = 1;
pub const FEED_AMOUNT_LABELS: [&str; FEED_AMOUNT_LEVELS] =
    ["少なめ", "ふつう", "多め", "かなり多め", "どっぱー(MAX)"];

// レベルごとの投下粒数(両端含む範囲)。
fn feed_amount_count_range(level: usize) -> (usize, usize) {
    match level {
        0 => (5, 10),
        1 => (15, 25),
        2 => (40, 60),
        3 => (100, 140),
        4 => (250, 350),
        _ => (15, 25),
    }
}

// レベルごとの横方向の散らばり幅(カーソル位置から±この値の範囲に投下する)。
// レベル4(MAX)だけ端末幅に比例させ、投下範囲が水槽横幅の約1/3になるようにする。
fn feed_amount_spread(level: usize, pix_w: usize) -> f64 {
    match level {
        0 => 3.0,
        1 => 6.0,
        2 => 16.0,
        3 => 30.0,
        4 => (pix_w as f64) / 6.0,
        _ => 6.0,
    }
}

// --- 投下エフェクト(f/m を押した瞬間、投下位置に一瞬だけ出る光/波紋) ---
pub const DROP_EFFECT_LIFETIME: f64 = 0.45; // 1秒未満で消える
pub const DROP_EFFECT_MAX_RADIUS: f64 = 3.5; // 波紋が広がる最大半径(論理ピクセル)
// 産卵時、生まれた卵の位置に出るキラキラ演出の持続時間(産卵時にキラキラ光るフラッシュ
// 演出を追加してほしい、目安2〜3秒程度という要望への対応)。
pub const SPAWN_FLASH_LIFETIME: f64 = 2.5;

// --- ガラスを叩く(t キー) ---
pub const KNOCK_RADIUS: f64 = 18.0; // この距離以内の魚が驚いて逃げる
pub const FLEE_DURATION: f64 = 1.2; // 逃走状態を維持する時間(秒)
// 基本移動速度を全体的に4倍にする方針を受けて4倍にした(旧140.0)。
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

// --- トントン(Tキー・引き寄せ) ---
// `t`(コンコン・驚いて逃走)の逆効果: カーソル位置を軽くノックすると、近くの魚が
// 興味を持ってカーソル位置へ穏やかに近づいてくる。逃走(flee_*)と対になる仕組みで、
// ジグザグの回り込み(ZIGZAG_*)は使わず、素直にカーソル方向へ寄っていくだけの
// 優しい動きにする。
pub const TAP_RADIUS: f64 = 18.0; // この距離以内の魚が興味を持って寄ってくる(KNOCK_RADIUSと同じ)
pub const TAP_ATTRACT_DURATION: f64 = 1.2; // 引き寄せ状態を維持する時間(秒。FLEE_DURATIONと同じ)
// 「軽く優しく」という位置づけのため、FLEE_STRENGTH(560.0)よりかなり弱めにしてある。
// 実機フィードバック対応で最初220.0にしたところ、tap位置(固定方向)を通り越して
// 反対側まで飛んで行ってしまい「軽く寄ってくる」には強すぎたため、通常のwander()と
// 同程度のオーダーまで弱め、カーソル付近でゆっくり収まるようにした(旧220.0)。
pub const TAP_ATTRACT_STRENGTH: f64 = 50.0;

// --- なつき度(T=トントンへの反応を積み重ねて上がる、時間経過でゆっくり下がる) ---
pub const AFFINITY_MAX: f64 = 100.0;
pub const AFFINITY_GAIN_PER_TAP: f64 = 4.0; // 1回のトントンで上がる量(クールダウン内は上がらない)
pub const AFFINITY_GAIN_COOLDOWN: f64 = 3.0; // 連打での瞬時カンストを防ぐクールダウン(秒)
pub const AFFINITY_DECAY_PER_SEC: f64 = 0.02; // 放置時に自然に下がる速度(満点から0まで約83分)
pub const AFFINITY_MARK_THRESHOLD: f64 = 60.0; // これ以上でステータスオーバーレイにマークを表示

// --- ランダムな瞬発ダッシュ(特定のトリガーが無い通常時の躍動感演出) ---
// ピラニア・餌などのトリガーが無い普段の遊泳中でも、低頻度・ランダムなタイミングで
// 一瞬だけ通常より速く動く「ダッシュ」を行う。頻発すると落ち着きがなく見えるため、
// 数十秒に1回あるかないか程度の頻度に抑える。
pub const DASH_CHANCE_PER_SEC: f64 = 0.02; // 期待間隔=約50秒に1回
pub const DASH_DURATION: f64 = 0.35; // ダッシュ自体は一瞬だけ
// 基本移動速度を全体的に4倍にする方針を受けて4倍にした(旧160.0)。
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
// 自動餌やり・自動投薬は、実際に空腹/病気な個体数から乖離した固定量(旧: 常に3〜5粒)
// になっていたとの指摘への対応。餌1粒はFEED_AMOUNT(34)回復しHUNGRY_THRESHOLD(50)を
// 十分越えられるので「空腹な個体数=必要な餌の数」、薬1粒は病気1匹をちょうど治すので
// 「病気の個体数=必要な薬の数」として計算する。ただし水質悪化・処理落ちを防ぐため
// 上限で頭打ちにする。
// 上限が低すぎると、大きい水槽(最大100匹規模)で空腹な個体数がすぐこれを
// 超えてしまい、慢性的な餌不足になるとの指摘を受けて引き上げた(旧12/8)。
pub const AUTO_FEED_COUNT_CAP: usize = 30;
pub const AUTO_MEDICATE_COUNT_CAP: usize = 20;
// 自動ガラス叩き: ランダムな位置・タイミングで時々発生させる(頻度は低め・数分に1回程度)。
// 既存の「叩きすぎペナルティ」判定にも通常のtキーと同じくカウントされるが、
// 低頻度なので基本的にペナルティには引っかからない想定。
pub const AUTO_KNOCK_COOLDOWN: f64 = 180.0;

// --- 自動魚補充(Aキーでon/off。既定OFF。自動モード(aキー)とは別トグル) ---
// 捕食する種(ピラニア・タコ)ばかりだと魚がいなくなるという指摘への対応。
// 通常魚(捕食対象になる種。ピラニア・タコ・カメオ生物は含まない)の生存数がこの数
// 以下になったら、+キー相当(ランダムな通常種を1匹追加)を自動的に行う。
pub const AUTO_REPLENISH_THRESHOLD: usize = 3;
// 頻発しすぎないよう、実際に補充した後はこのクールダウンを置く(自動餌やり等と同程度)。
pub const AUTO_REPLENISH_COOLDOWN: f64 = 30.0;

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
    // 沈降中に左右へ揺れる動き(螺旋階段のような蛇行)の位相。粒ごとにランダムな
    // 初期値を持たせ、複数粒が同じタイミングで揺れないようにする。
    #[serde(default)]
    pub sway_phase: f64,
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
    // Food と同様、沈降中の左右への揺れ(蛇行)の位相
    #[serde(default)]
    pub sway_phase: f64,
}

// 浄化剤(`C`キー): 薬と同様に沈むが、着水した瞬間に効果を発動して即座に消える
// 一発物のアイテム。Food/Medicine/Meat のように水底へ停留せず、寿命(life)も
// 着地フラグ(landed)も持たない(沈む間だけ存在する)。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Purifier {
    pub x: f64,
    pub y: f64,
    pub vy: f64,
    pub sway_phase: f64,
}

// ピラニア専用の肉餌(`M`キーのみで投下・自動モードには組み込まない)。
// ピラニア以外の魚は一切近づかず消費もしない。ピラニアは空腹の間だけ食いつき、
// 満腹の間は肉餌とはいえ食いつかない(通常の狩りhunger判定と同じ基準)。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meat {
    pub x: f64,
    pub y: f64,
    pub vy: f64,
    pub life: f64,
    // Food/Medicine と同様、着地後は寿命が減らず停留する
    #[serde(default)]
    pub landed: bool,
    // Food/Medicine と同様、沈降中の左右への揺れ(蛇行)の位相
    #[serde(default)]
    pub sway_phase: f64,
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
    Tap,   // `T`キー(トントン)。Knockより柔らかい色の波紋
    Meat,  // `M`キー(ピラニア専用の肉餌)投下時の演出
    Purify, // `C`キー(浄化剤)投下時の演出
    Mate,  // つがいが出会って交尾する瞬間の演出(ハート)
    Hatch, // 卵が孵化(羽化)する瞬間の演出
    Decompose, // カニが水底の亡骸を片付ける瞬間の演出(分解して崩れる)
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

// 血の匂い(無形): 見た目のBloodStainとは別に、ピラニアの追跡ロジックだけが参照する
// 「出血元」の座標を保持する。描画側(main.rs)は参照しない、育成ロジック専用の
// エンティティ(表示はBloodStain/DropEffectが担う)。時間経過で薄れて消える
// (max_lifeは持たない。BloodStainのようなフェード表現の計算に使わないため不要)。
#[derive(Clone, Debug)]
pub struct BloodScent {
    pub x: f64,
    pub y: f64,
    pub life: f64,
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

// 浄化ブルーム(浄化剤の着水演出): 墨と同じ「同心円状に広がって薄れて消える」構造を
// 再利用し、明るい水色で一気に広がる(main.rs側で専用の色・半径定数を使う)。
// 数秒で消える短命の演出なので保存対象にはしない。
#[derive(Clone, Debug)]
pub struct PurifyBloom {
    pub x: f64,
    pub y: f64,
    pub life: f64,
    pub max_life: f64,
    // 拡散(PURIFY_BLOOM_GROWTH_TIME)が完了して、浄化剤の効果(濃度加算)を
    // 発動済みかどうか。着水した瞬間ではなく、拡散し終わったタイミングで
    // 効果が始まるようにするためのフラグ。
    pub activated: bool,
}

// 効果音(SE)の発火イベント。sim.rs は音の再生方法を知らず、main.rs 側の
// SoundEngine がこれを受け取って正弦波ビープを鳴らす(sim.rs は rodio に依存しない)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SfxEvent {
    Bubble,      // 気泡が上る音(頻度は控えめ)
    Feed,        // 餌を入れた音
    Medicate,    // 薬を入れた音
    Purify,      // 浄化剤を入れた・着水した音(明るく弾ける短いチャイム)
    SickOnset,   // 病気になった音
    Cured,       // 治療で回復した音
    HungryOnset, // 空腹(腹ぺこ)になった瞬間の音
    GlassKnock,  // ガラスを叩いた(こんこん)音
    Predation,   // ピラニア・タコが獲物を捕食した音
    Ink,         // タコが墨を吐いた音
    Tap,         // カーソル位置を軽くノックした(とんとん)音。GlassKnockより柔らかく短い
    UiClick,     // 各種トグルをONにした瞬間の、乾いた小さいクリック音(やかましくないもの)
    StarPickup,  // スターを取得して無敵になった瞬間の、控えめなキラキラ音
}

#[derive(Clone, Debug)]
pub struct Bubble {
    pub x: f64,
    pub y: f64,
    pub vy: f64,
}

// 水流を可視化するための短い横線状の筋。渦の力場に沿って流されて曲線を描き、
// フェードアウトしながら消える(見た目のみ・育成ロジックには参加しない)。血飛沫・墨・
// 気泡などと同じ短命の演出なので、保存対象にはしない(再起動時に作り直す)。
#[derive(Clone, Debug)]
pub struct CurrentStreak {
    pub x: f64,
    pub y: f64,
    pub life: f64,
    pub max_life: f64,
}

// 観賞用のカニ。水底(砂の上)を左右に歩くだけで泳がない。育成ロジック対象外。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Crab {
    pub x: f64,
    pub dir: f64, // 歩く向き: +1.0=右, -1.0=左
    pub pause_timer: f64,
    pub facing_right: bool,
}

// 観賞用のエビ。カニと同じ位置づけ(育成ロジック対象外・捕食対象外・自身も
// 捕食しない)で、水底や藻の近くをゆっくり歩く/漂う。挙動もカニと同様でよい。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shrimp {
    pub x: f64,
    pub dir: f64, // 歩く向き: +1.0=右, -1.0=左
    pub pause_timer: f64,
    pub facing_right: bool,
}

// 観賞用のタツノオトシゴ。カニ・エビと同じ位置づけ。藻に絡みつくようにゆっくり
// 動き、あまり大きく移動しない(基準位置=藻の近くから離れず、ゆらゆら漂うだけ)。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Seahorse {
    pub anchor_x: f64, // 絡みつく藻の近くの基準位置(ここから大きく離れない)
    pub anchor_y: f64,
    pub x: f64,
    pub y: f64,
    pub phase: f64, // ゆらゆら動く位相(個体差を出すためのオフセット)
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

// カメオ生物の種類(完全観賞用。育成ロジック・捕食判定のいずれにも参加しない)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum CameoKind {
    Turtle,
    Jellyfish,
    FishSchool, // 小魚の群れ。描画側(main.rs)で複数の小さな魚として描く
}

// カメオ生物: ウミガメ・クラゲ・小魚の群れなど、低頻度で出現して画面の端から端まで
// 通過するだけの完全観賞用エンティティ。魚(Fish)とは完全に独立しており、
// 捕食対象にならず自身も捕食しない(育成ロジック・捕食判定のいずれにも参加しない)。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cameo {
    pub kind: CameoKind,
    pub x: f64,
    pub y: f64,
    pub vx: f64, // 水平方向の移動速度(px/秒。符号が進行方向)
    pub vy: f64, // ゆったりした縦のふらつき
    pub phase: f64, // 縦のふらつき・見た目の演出用の位相
}

pub struct Simulation {
    pub fish: Vec<Fish>,
    pub food: Vec<Food>,
    pub medicine: Vec<Medicine>,
    // ピラニア専用の肉餌(`M`キーのみで投下。自動モードには組み込まない)
    pub meat: Vec<Meat>,
    // 浄化剤(`C`キー)。沈む間だけ存在し、着水した瞬間に消えてpurifier_concentrationを
    // 最大に立てる(水底には停留しない)。保存対象にする(沈下中の個体を復元できるように)。
    pub purifiers: Vec<Purifier>,
    // 通常5種(Species::COMMONと同じ並び順)ごとに、新規生成の抽選対象にするかどうかの
    // トグル。既定は全てtrue(全種抽選対象)。設定画面から切り替える(保存対象外)。
    pub species_toggle: [bool; 5],
    // 餌やり(`f`キー・自動餌やり共通)の投下量レベル(0..=4)。既定は1(従来どおりの量)。
    pub feed_amount: usize,
    // 水質(0=綺麗〜POLLUTION_MAX=最悪)。堆積した食べ残し・病気・死亡個体の放置で
    // 悪化し、自然浄化で改善する。保存対象(state.jsonに保存し、次回起動時も継続)。
    pub pollution: f64,
    // 浄化剤の濃度(0=効果なし〜1.0=着水直後の最大)。着水でPURIFIER_DILUTION_TIMEをかけて
    // 線形に0まで薄まる。濃度に比例して(1)水質を急速に浄化し(2)通常種の食欲を抑え
    // (3)全種の老化を早める。保存対象(効果継続中に再起動しても効果が続くように)。
    pub purifier_concentration: f64,
    // カニの表示ON/OFF。設定画面から切り替える。OFFにすると自動的に消え、
    // ONに戻すと初期数(CRAB_COUNT)が再配置される。
    pub crab_toggle: bool,
    pub eggs: Vec<Egg>,
    pub bubbles: Vec<Bubble>,
    // 水流を可視化する筋(気泡等と同じ短命の演出なので保存対象にしない)。
    pub current_streaks: Vec<CurrentStreak>,
    pub crabs: Vec<Crab>,
    pub shrimp: Vec<Shrimp>,
    pub seahorses: Vec<Seahorse>,
    // 藻・水草・岩・タコつぼ(装飾。育成ロジック非対応の静的オブジェクト)
    pub plants: Vec<Plant>,
    pub rocks: Vec<Rock>,
    pub dens: Vec<Den>,
    // スター(無敵アイテム)。餌・薬と同様に寿命があるため保存対象にする。
    pub stars: Vec<Star>,
    // カメオ生物(完全観賞用。画面を通過するだけで数十秒で消えるため、
    // 血飛沫等と同様に保存対象にしない)。
    pub cameos: Vec<Cameo>,
    // 投下エフェクト(一瞬で消えるので保存対象にしない)
    pub drop_effects: Vec<DropEffect>,
    // 血の滲み(範囲エフェクト。数秒で消えるので保存対象にしない)
    pub blood_stains: Vec<BloodStain>,
    // 血の匂い(ピラニアの追跡ロジック専用の無形ソース。BLOOD_SCENT_LIFETIMEで
    // 消えるので保存対象にしない)
    pub blood_scents: Vec<BloodScent>,
    // 墨(タコが吐く。数秒で消えるので保存対象にしない)
    pub ink_clouds: Vec<InkCloud>,
    // 浄化ブルーム(浄化剤の着水演出。数秒で消えるので保存対象にしない)
    pub purify_blooms: Vec<PurifyBloom>,
    // このtickで発火した効果音イベント。main.rs 側が毎フレーム drain して再生する
    // (保存対象にしない。sim.rs は音の再生方法を知らない)。
    pub sound_events: Vec<SfxEvent>,
    pub rng: Rng,
    pub elapsed: f64,            // 累計経過秒
    // 渦の中心座標。elapsedと水槽サイズから毎tick決まるだけの派生値なので保存しない
    // (再起動時はupdate_current()で同じ値が再計算される)。
    pub current_center_x: f64,
    pub current_center_y: f64,
    pub message: Option<String>, // ステータスバー用の一言
    message_ttl: f64,
    bubble_timer: f64,
    // 水流の筋の生成間隔タイマー(気泡と同じく保存対象外の一時的な演出用)。
    current_streak_timer: f64,
    // 水流の筋(見た目だけの演出)専用の乱数生成器。共有のrngとは分けており、
    // 装飾用の乱数消費が育成ロジック側の決定論(シード固定テスト)に影響しないようにする。
    current_streak_rng: Rng,
    bubble_sound_timer: f64, // 気泡音専用の間引きタイマー(見た目の気泡発生より控えめな頻度)
    knock_times: Vec<f64>, // 直近の「ガラスを叩く」タイムスタンプ(叩きすぎ判定用。保存対象外)
    // 自動モード(aキー)用のクールダウンタイマー。UI側のON/OFFはmain.rs(Ctl)が持ち、
    // ONの間だけ update_auto_care() を呼ぶ想定(保存対象外)。
    auto_feed_timer: f64,
    auto_medicate_timer: f64,
    auto_knock_timer: f64,
    // 自動魚補充(Aキー)用のクールダウンタイマー。UI側のON/OFFはmain.rs(Ctl)が持ち、
    // ONの間だけ update_auto_replenish() を呼ぶ想定(保存対象外)。
    auto_replenish_timer: f64,
}

// 水底(砂)の高さ(論理ピクセル)
pub fn sand_height(pix_h: usize) -> usize {
    (pix_h / 12).max(2)
}

// 端末サイズに応じた個体数上限。大きい端末では最大100匹程度まで許容する。
// 魚のドット絵を大幅拡大してほしい(1.5〜2倍では不十分)という要望を
// 受けて魚のスプライトを大幅に拡大したため、画面が窮屈にならないよう除数を
// 700→2500に上げて収容密度を下げた(サイズを妥協するのではなく上限側で調整する方針)。
// その後、標準的な端末サイズで上限が11匹程度まで下がったため、もう少し多く収容できる
// ようにしてほしいという指摘を受けて、2500→1200まで下げて再調整した
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

// 個体差(新規): 空腹になる速さ・食べた時の満たされ方・寿命・成長できる上限サイズに、
// 個体ごとのばらつきを与える。`&mut Rng` を直接取る自由関数にしてあるのは、
// `self.eggs.retain(...)` のクロージャ内など、`self`全体ではなく`self.rng`だけを
// 借用したい箇所からも呼べるようにするため(Simulation::roll_individualityはこれに委譲する)。
fn roll_individuality_with_rng(rng: &mut Rng, mut f: Fish) -> Fish {
    // 「たまに大食い」個体: 通常よりはっきり空腹になりやすく、食べた時の満たされ方も大きい。
    if rng.next_f64() < GOURMAND_CHANCE_PER_SPAWN {
        f.hunger_decay_mult = rng.range(GOURMAND_HUNGER_MULT_MIN, GOURMAND_HUNGER_MULT_MAX);
        f.feed_efficiency_mult = rng.range(GOURMAND_FEED_MULT_MIN, GOURMAND_FEED_MULT_MAX);
    } else {
        f.hunger_decay_mult = rng.range(INDIVIDUALITY_HUNGER_MULT_MIN, INDIVIDUALITY_HUNGER_MULT_MAX);
        f.feed_efficiency_mult = rng.range(INDIVIDUALITY_HUNGER_MULT_MIN, INDIVIDUALITY_HUNGER_MULT_MAX);
    }
    f.lifespan_mult = rng.range(INDIVIDUALITY_LIFESPAN_MULT_MIN, INDIVIDUALITY_LIFESPAN_MULT_MAX);
    f.growth_cap_variance = (rng.range_usize(0, 2) as i8) - 1; // range_usizeは両端含む(0..=2) -> -1, 0, +1
    f
}

// ピラニアが「まだ狩りをやめない(空腹とみなす)」かどうか。通常の空腹度判定
// (hunger < PIRANHA_HUNT_HUNGER_THRESHOLD)に加えて、今回の満腹サイクルで既に
// 1匹以上捕食済み(piranha_meals_since_full > 0)かつPIRANHA_KILLS_TO_FULL未満の
// 間は、hungerが満腹相当でも狩りを継続させる(食欲を旺盛にする実機フィードバック
// 対応)。「> 0」の条件により、まだ一度も捕食していない(通常のhunger判定だけで
// 満腹とみなされている)ピラニアの挙動は変えない。
fn piranha_still_hungry(f: &Fish) -> bool {
    f.hunger < PIRANHA_HUNT_HUNGER_THRESHOLD
        || (f.piranha_meals_since_full > 0 && f.piranha_meals_since_full < PIRANHA_KILLS_TO_FULL)
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
            meat: Vec::new(),
            purifiers: Vec::new(),
            species_toggle: [true; 5],
            feed_amount: FEED_AMOUNT_DEFAULT,
            pollution: 0.0,
            purifier_concentration: 0.0,
            crab_toggle: true,
            eggs: Vec::new(),
            bubbles: Vec::new(),
            current_streaks: Vec::new(),
            crabs: Vec::new(),
            shrimp: Vec::new(),
            seahorses: Vec::new(),
            plants: Vec::new(),
            rocks: Vec::new(),
            dens: Vec::new(),
            stars: Vec::new(),
            cameos: Vec::new(),
            drop_effects: Vec::new(),
            blood_stains: Vec::new(),
            blood_scents: Vec::new(),
            ink_clouds: Vec::new(),
            purify_blooms: Vec::new(),
            sound_events: Vec::new(),
            rng,
            elapsed: 0.0,
            current_center_x: 0.0,
            current_center_y: 0.0,
            message: None,
            message_ttl: 0.0,
            bubble_timer: 0.0,
            current_streak_timer: 0.0,
            // 共有rngとは独立の固定シード(mainのrngを一切消費しないので、
            // シード固定テストの決定論に影響しない)。
            current_streak_rng: Rng::new(0x00C0_FFEE),
            bubble_sound_timer: 0.0,
            knock_times: Vec::new(),
            auto_feed_timer: 0.0,
            auto_medicate_timer: 0.0,
            auto_knock_timer: 0.0,
            auto_replenish_timer: 0.0,
        }
    }

    // 個体差(新規): 空腹になる速さ・食べた時の満たされ方・寿命・成長できる上限サイズに、
    // 個体ごとのばらつきを与える。実際に魚を生成する箇所(seed_initial/add_fish_of_species/
    // add_octopus/卵の孵化)からのみ呼ぶこと。Fish::new()自体はニュートラル値のままなので、
    // これを呼ばずに直接pushした場合は個体差なし(既存テスト等はそのまま影響を受けない)。
    fn roll_individuality(&mut self, f: Fish) -> Fish {
        roll_individuality_with_rng(&mut self.rng, f)
    }

    // 新規生成(seed_initial/add_fish)の抽選対象になる通常種の一覧。species_toggleで
    // OFFにした種は除く。全部OFFにした場合は安全側で通常5種全部にフォールバックする
    // (空の抽選プールで固まらないようにするため)。
    fn spawn_pool(&self) -> Vec<Species> {
        let pool: Vec<Species> = Species::COMMON
            .iter()
            .zip(self.species_toggle.iter())
            .filter(|(_, &enabled)| enabled)
            .map(|(&sp, _)| sp)
            .collect();
        if pool.is_empty() {
            Species::COMMON.to_vec()
        } else {
            pool
        }
    }

    // 設定画面から呼ぶ: 通常5種(Species::COMMONの並び順)のうちidx番目の生成トグルを
    // 反転する。範囲外のidxは何もしない。
    pub fn toggle_common_species(&mut self, idx: usize) {
        if let Some(v) = self.species_toggle.get_mut(idx) {
            *v = !*v;
        }
    }

    // 設定画面から呼ぶ: 餌やりの投下量を次の段階に循環させる(MAXの次は最小に戻る)。
    pub fn cycle_feed_amount(&mut self) {
        self.feed_amount = (self.feed_amount + 1) % FEED_AMOUNT_LEVELS;
    }

    // 設定画面から呼ぶ: カニの表示をトグルする。OFFにすると即座に全て消し、
    // ONに戻すと初期数(CRAB_COUNT)を再配置する。
    pub fn toggle_crabs(&mut self, pix_w: usize) {
        self.crab_toggle = !self.crab_toggle;
        if !self.crab_toggle {
            self.crabs.clear();
        } else if self.crabs.is_empty() {
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

    // 初期個体を撒く(セーブが無い初回起動 / リセット用)
    pub fn seed_initial(&mut self, pix_w: usize, pix_h: usize) {
        // 初期配置はピラニアを含めない通常種のみ(ピラニアの入手経路はSキーのみに限定する方針)。
        // species_toggleでOFFにした種は選ばれない(spawn_pool経由)。
        let pool = self.spawn_pool();
        let cap = capacity(pix_w, pix_h);

        // 水槽が極端に小さくつがい(2匹)すら入らない場合は、空にしないよう1匹だけ配置する。
        if cap < 2 {
            let x = self.rng.range(6.0, (pix_w as f64 - 6.0).max(6.0));
            let y = self
                .rng
                .range(4.0, (pix_h as f64 - sand_height(pix_h) as f64 - 2.0).max(4.0));
            let fish = self.roll_individuality(Fish::new(pool[0], Stage::Adult, x, y));
            self.fish.push(fish);
            self.ensure_decorative_entities(pix_w, pix_h);
            return;
        }

        // 種ごとに同種の成魚2匹を「つがい」として撒く。2匹は共通の基準点の近く
        // (数ピクセルのゆらぎ内)に置くので、最初から求愛範囲に収まり、相手を探して
        // 水槽中を泳ぎ回らなくても繁殖に入れる。成魚同士なので満腹タイマーが溜まれば
        // すぐつがい候補になる(以前の成魚/稚魚交互配置はやめた)。
        let max_pairs = (cap / 2).max(1);
        let num_pairs = pool.len().min(max_pairs);
        for i in 0..num_pairs {
            let sp = pool[i % pool.len()];
            let base_x = self.rng.range(6.0, (pix_w as f64 - 6.0).max(6.0));
            let base_y = self
                .rng
                .range(4.0, (pix_h as f64 - sand_height(pix_h) as f64 - 2.0).max(4.0));
            for _ in 0..2 {
                // 基準点に数ピクセルのゆらぎを加える(求愛範囲COURTSHIP_RADIUSより十分小さい)。
                let jitter = 3.0;
                let x = (base_x + self.rng.range(-jitter, jitter))
                    .clamp(6.0, (pix_w as f64 - 6.0).max(6.0));
                let y = (base_y + self.rng.range(-jitter, jitter))
                    .clamp(4.0, (pix_h as f64 - sand_height(pix_h) as f64 - 2.0).max(4.0));
                let fish = self.roll_individuality(Fish::new(sp, Stage::Adult, x, y));
                self.fish.push(fish);
            }
        }
        self.ensure_decorative_entities(pix_w, pix_h);
    }

    // 観賞用エンティティ(大型魚・カニ)が空なら初期数を補充する。
    // 初回起動時に加え、それらのフィールドを持たない旧セーブを読み込んだ直後にも呼ぶ。
    pub fn ensure_decorative_entities(&mut self, pix_w: usize, pix_h: usize) {
        if self.crab_toggle && self.crabs.is_empty() {
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

        if self.shrimp.is_empty() {
            for _ in 0..SHRIMP_COUNT {
                let x = self.rng.range(4.0, (pix_w as f64 - 4.0).max(4.0));
                let dir = if self.rng.next_f64() < 0.5 { 1.0 } else { -1.0 };
                self.shrimp.push(Shrimp {
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
                    // 藻を魚が隠れられるくらい大きくしてほしいという要望に対し、まだ小さい
                    // との再指摘を受けてさらに拡大した(旧3.0〜7.0→6.0〜11.0→12.0〜20.0)。
                    height: self.rng.range(20.0, 32.0),
                    phase: self.rng.range(0.0, std::f64::consts::TAU),
                });
            }
        }

        if self.seahorses.is_empty() {
            // 藻の近くを基準位置にする(藻に絡みつくイメージ)。藻が無ければ水底付近の
            // ランダムな位置を基準にする。
            for _ in 0..SEAHORSE_COUNT {
                let (anchor_x, anchor_y) = if !self.plants.is_empty() {
                    let idx = self.rng.range_usize(0, self.plants.len() - 1);
                    let p = &self.plants[idx];
                    (p.x, (p.y - p.height * 0.5).max(2.0))
                } else {
                    (
                        self.rng.range(4.0, (pix_w as f64 - 4.0).max(4.0)),
                        (sand_top - 6.0).max(2.0),
                    )
                };
                self.seahorses.push(Seahorse {
                    anchor_x,
                    anchor_y,
                    x: anchor_x,
                    y: anchor_y,
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
    // 浮いて見えてしまう(文字サイズを変更した際にタコつぼや水草が床に沈む、または
    // 逆に浮いてしまう現象への対応)。ここで新しい水底位置に
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
        // 再配置と同時にタコつぼの数を現在のタコの数に整理し、タコより多いタコつぼは
        // 削除・少なければ追加してほしいという要望への対応: タコつぼの数を
        // 常に「現在生きているタコの数」に一致させる。
        let octo_count = self
            .fish
            .iter()
            .filter(|f| f.species == Species::Octopus && !f.dead)
            .count();
        if self.dens.is_empty() && octo_count == 0 {
            self.set_message("タコつぼがありません");
            return;
        }

        self.dens.clear();
        let sand_top = (pix_h as f64 - sand_height(pix_h) as f64).max(2.0);
        let den_half_h = den_sprite().height as f64 / 2.0;
        let y = (sand_top - den_half_h + 1.0).max(1.0);
        for _ in 0..octo_count {
            let x = self.rng.range(4.0, (pix_w as f64 - 4.0).max(4.0));
            self.dens.push(Den { x, y });
        }

        // タコつぼの数=タコの数にしたので、生きているタコを1匹ずつ新しいタコつぼへ
        // 割り当て直す(隠れているタコは表示位置も新しい巣の位置へ移す)。
        let mut den_iter = self.dens.iter();
        for f in &mut self.fish {
            if f.species != Species::Octopus || f.dead {
                continue;
            }
            if let Some(den) = den_iter.next() {
                f.den_x = den.x;
                f.den_y = den.y;
                if f.hidden {
                    f.x = den.x;
                    f.y = den.y;
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
                // ensure_decorative_entities側と同じ大きさに揃える。
                height: self.rng.range(20.0, 32.0),
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
        self.meat.clear();
        self.purifiers.clear();
        self.purifier_concentration = 0.0;
        self.eggs.clear();
        self.bubbles.clear();
        self.stars.clear();
        self.drop_effects.clear();
        self.blood_stains.clear();
        self.blood_scents.clear();
        self.ink_clouds.clear();
        self.purify_blooms.clear();
        // Rキーでのグレートリセット時に蛸壺もリセットしてほしいという要望への対応。
        // 従来はdens(タコつぼ)を含む装飾エンティティ(plants/rocks/crabs/shrimp/
        // seahorses)がクリアされておらず、ensure_decorative_entities側の
        // 「空の時だけ補充する」ガードによりリセット後も古い配置がそのまま残って
        // いた(同種のバグ)。他の装飾もまとめてクリアし、seed_initial経由で
        // 再配置させる。
        self.dens.clear();
        self.plants.clear();
        self.rocks.clear();
        self.crabs.clear();
        self.shrimp.clear();
        self.seahorses.clear();
        self.elapsed = 0.0;
        self.seed_initial(pix_w, pix_h);
        self.set_message("水槽をリセットしました");
    }

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
        self.message_ttl = 4.0;
    }

    // 餌やり(`f`キー): カーソルのX座標付近から粒を投下(Yは水面付近から沈み始める)。
    // 粒数・散らばり幅は`feed_amount`(設定画面で切替)に応じて変わる。
    pub fn feed(&mut self, cursor_x: f64, pix_w: usize) {
        let (lo, hi) = feed_amount_count_range(self.feed_amount);
        let count = self.rng.range_usize(lo, hi);
        let spread = feed_amount_spread(self.feed_amount, pix_w);
        self.drop_food(cursor_x, pix_w, count, spread);
        if self.feed_amount == FEED_AMOUNT_LEVELS - 1 {
            self.set_message("どっぱー！餌を大量投入した");
        }
    }

    // 餌の投下本体。自動餌やり(`update_auto_care`)は`feed_amount`設定の影響を受けず
    // 従来どおりの控えめな量(3〜5粒)に固定するため、量・散らばりを引数で受け取る形に
    // 分離している(自動モードで大量投入すると水質が悪化しすぎるため)。
    // 投下位置には一瞬だけ光/波紋の演出(DropEffect)を出し、何をどこに投げたか分かりやすくする。
    fn drop_food(&mut self, cursor_x: f64, pix_w: usize, count: usize, spread: f64) {
        let cx = cursor_x.clamp(1.0, safe_upper(pix_w as f64 - 1.0));
        for _ in 0..count {
            self.food.push(Food {
                x: (cursor_x + self.rng.range(-spread, spread))
                    .clamp(1.0, safe_upper(pix_w as f64 - 1.0)),
                y: self.rng.range(1.0, 4.0),
                vy: FOOD_SINK_SPEED * self.rng.range(0.8, 1.2),
                life: FOOD_LIFETIME,
                landed: false,
                sway_phase: self.rng.range(0.0, std::f64::consts::TAU),
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
        self.drop_medicine(cursor_x, pix_w, count);
    }

    // 投薬の本体。自動投薬(`update_auto_care`)は実際に病気の個体数に合わせた量を
    // 渡すため、量を引数で受け取る形に分離している(薬1粒で病気1匹をちょうど治せる
    // ので、病気の個体数と同じ量を投下すれば過不足なく対応できる)。
    fn drop_medicine(&mut self, cursor_x: f64, pix_w: usize, count: usize) {
        let cx = cursor_x.clamp(1.0, safe_upper(pix_w as f64 - 1.0));
        for _ in 0..count {
            self.medicine.push(Medicine {
                x: (cursor_x + self.rng.range(-6.0, 6.0)).clamp(1.0, safe_upper(pix_w as f64 - 1.0)),
                y: self.rng.range(1.0, 4.0),
                vy: MED_SINK_SPEED * self.rng.range(0.8, 1.2),
                life: MED_LIFETIME,
                landed: false,
                sway_phase: self.rng.range(0.0, std::f64::consts::TAU),
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

    // ピラニア専用の肉餌(`M`キー): カーソルX座標付近に1個投下する。餌・薬とは違い
    // 自動モードには絶対に組み込まない(呼び出しはキー入力からのみ)。ピラニアは
    // 空腹の間だけ食いつき(満腹の間は無視する)、ピラニア以外の魚は一切近づかず
    // 消費しない(update_meat側で種族・空腹度をチェックする)。
    pub fn drop_meat(&mut self, cursor_x: f64, pix_w: usize) {
        let cx = cursor_x.clamp(1.0, safe_upper(pix_w as f64 - 1.0));
        self.meat.push(Meat {
            x: cx,
            y: self.rng.range(1.0, 4.0),
            vy: MEAT_SINK_SPEED * self.rng.range(0.8, 1.2),
            life: MEAT_LIFETIME,
            landed: false,
            sway_phase: self.rng.range(0.0, std::f64::consts::TAU),
        });
        self.drop_effects.push(DropEffect {
            x: cx,
            y: 2.5,
            life: DROP_EFFECT_LIFETIME,
            max_life: DROP_EFFECT_LIFETIME,
            kind: EffectKind::Meat,
        });
        // 専用の効果音は用意せず、既存のFeedを再利用する(投下音として意味は通る)。
        self.sound_events.push(SfxEvent::Feed);
    }

    // 浄化剤(`C`キー): カーソルX座標付近に1個投下する。薬と同様に沈むが、着水した
    // 瞬間に消えてpurifier_concentrationを最大に立てる(update_purifiers側で処理)。
    // 自動モードには組み込まない(呼び出しはキー入力からのみ)。
    pub fn drop_purifier(&mut self, cursor_x: f64, pix_w: usize) {
        let cx = cursor_x.clamp(1.0, safe_upper(pix_w as f64 - 1.0));
        self.purifiers.push(Purifier {
            x: cx,
            y: self.rng.range(1.0, 4.0),
            vy: PURIFIER_SINK_SPEED * self.rng.range(0.8, 1.2),
            sway_phase: self.rng.range(0.0, std::f64::consts::TAU),
        });
        self.drop_effects.push(DropEffect {
            x: cx,
            y: 2.5,
            life: DROP_EFFECT_LIFETIME,
            max_life: DROP_EFFECT_LIFETIME,
            kind: EffectKind::Purify,
        });
        self.sound_events.push(SfxEvent::Purify);
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
                // 死亡演出中の魚は驚かないが、カーソル近くの死骸は「つついた」扱いに
                // して、以降は浮力を無視して沈降を早める(既に沈み切った死骸に対しては
                // 無害な無操作になる)。
                let dx = f.x - cx;
                let dy = f.y - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < KNOCK_RADIUS {
                    f.sink_forced = true;
                }
                continue;
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

    // トントン(`T`キー): `knock`(こんこん・驚いて逃走)の逆効果。カーソル位置を
    // 軽くノックすると、近くの(死んでいない)魚が興味を持ってカーソル位置へ穏やかに
    // 近づいてくる(逃走ベクトルの代わりに引き寄せベクトルを一時的に加算する)。
    // 見た目(EffectKind::Tap)・音(SfxEvent::Tap)はknockと差別化した柔らかいものにする。
    // knockの「叩きすぎでストレス」のような負の副作用は設けない(優しい操作のため)。
    pub fn tap_attract(&mut self, cursor_x: f64, cursor_y: f64, pix_w: usize, pix_h: usize) {
        let (cx, cy) = clamp_point(cursor_x, cursor_y, pix_w, pix_h);
        self.drop_effects.push(DropEffect {
            x: cx,
            y: cy,
            life: DROP_EFFECT_LIFETIME,
            max_life: DROP_EFFECT_LIFETIME,
            kind: EffectKind::Tap,
        });
        self.sound_events.push(SfxEvent::Tap);

        for f in &mut self.fish {
            if f.dead {
                continue; // 死亡演出中の魚は反応しない
            }
            let dx = cx - f.x;
            let dy = cy - f.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < TAP_RADIUS {
                let d = dist.max(0.001);
                f.attract_dx = dx / d;
                f.attract_dy = dy / d;
                f.attract_timer = TAP_ATTRACT_DURATION;
                if f.affinity_cooldown <= 0.0 {
                    f.affinity = (f.affinity + AFFINITY_GAIN_PER_TAP).min(AFFINITY_MAX);
                    f.affinity_cooldown = AFFINITY_GAIN_COOLDOWN;
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
            let hungry_count = self.fish.iter().filter(|f| !f.dead && f.hunger < HUNGRY_THRESHOLD).count();
            let floating_food = self.food.iter().filter(|fd| !fd.landed).count();
            if hungry_count > 0 && floating_food < AUTO_FEED_FLOAT_THRESHOLD {
                let x = self.rng.range(4.0, safe_upper(pix_w as f64 - 4.0));
                // 自動餌やりは`feed_amount`設定の影響を受けず、実際に空腹な個体数に
                // 合わせた量(1匹あたり1粒。水質悪化を防ぐため上限で頭打ち)にする
                // (大量投入すると水質が悪化しすぎるため、自動モードの量は手動投下と
                // 分離している)。散らばりは従来どおり±6px。
                let count = hungry_count.min(AUTO_FEED_COUNT_CAP);
                self.drop_food(x, pix_w, count, 6.0);
                self.auto_feed_timer = AUTO_FEED_COOLDOWN;
            }
        }

        if self.auto_medicate_timer <= 0.0 {
            let sick_count = self.fish.iter().filter(|f| !f.dead && f.sick).count();
            let floating_med = self.medicine.iter().filter(|md| !md.landed).count();
            if sick_count > 0 && floating_med < AUTO_MEDICATE_FLOAT_THRESHOLD {
                let x = self.rng.range(4.0, safe_upper(pix_w as f64 - 4.0));
                // 薬1粒は病気1匹をちょうど治すので、病気の個体数に合わせた量にする
                // (水質悪化・処理落ちを防ぐため上限で頭打ち)。
                let count = sick_count.min(AUTO_MEDICATE_COUNT_CAP);
                self.drop_medicine(x, pix_w, count);
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

    // 自動魚補充(Aキー): ON中、呼び出し側(main.rs)が毎tickこれを呼ぶ想定。
    // 既存の自動モード(aキー・update_auto_care)とは別のトグルにする(捕食する種ばかりだと
    // 魚がいなくなるという指摘への対応)。通常魚(捕食対象になる種。
    // ピラニア・タコ・カメオ生物は含まない)の生存数がAUTO_REPLENISH_THRESHOLD以下に
    // なったら、+キー相当(add_fish)で1匹補充する。頻発しすぎないようクールダウンを
    // 設ける(ADD_FISH_MANUAL_CAP・個体数上限はadd_fish側でそのまま適用される)。
    pub fn update_auto_replenish(&mut self, dt: f64, pix_w: usize, pix_h: usize) {
        self.auto_replenish_timer = (self.auto_replenish_timer - dt).max(0.0);
        if self.auto_replenish_timer > 0.0 {
            return;
        }
        let common_count = self
            .fish
            .iter()
            .filter(|f| !f.dead && Species::COMMON.contains(&f.species))
            .count();
        if common_count <= AUTO_REPLENISH_THRESHOLD {
            self.add_fish(pix_w, pix_h);
            self.auto_replenish_timer = AUTO_REPLENISH_COOLDOWN;
        }
    }

    // デバッグ: 魚を1匹追加。ADD_FISH_MANUAL_CAP(25匹)まで。
    // それ以上は産卵→孵化を経由してのみ個体数上限(端末サイズ依存・最大100)まで増やせる。
    // 死んで浮いている個体は数に入れない(居座りで詰まらせない)。
    // ランダム選択はピラニアを含まない通常3種のみ(ピラニアの入手経路はSキーのみに限定する方針)。
    pub fn add_fish(&mut self, pix_w: usize, pix_h: usize) {
        let pool = self.spawn_pool();
        let sp = pool[self.rng.range_usize(0, pool.len() - 1)];
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

        let mut octo = self.roll_individuality(Fish::new(Species::Octopus, Stage::Fry, den_x, den_y));
        octo.hidden = false; // 投入直後は見える状態にする
        octo.den_x = den_x;
        octo.den_y = den_y;
        octo.hidden_timer = self.rng.range(OCTOPUS_EMERGE_TIME_MIN, OCTOPUS_EMERGE_TIME_MAX);
        self.fish.push(octo);
        self.set_message("タコを1匹投入しました");
    }

    // `W`キー: クジラを1匹、確実に水槽に投入する。クジラはネタ枠の巨大魚として、通常の
    // `+`キー(ランダム追加)・初期配置・孵化のいずれからも生成されない特殊入手種のため、
    // ピラニアの`S`キー・タコの`O`キーと同様の専用ショートカットを用意する。巣や隠れる
    // 挙動は持たず、無害な通常魚として振る舞うだけなので、生成処理は add_piranha と同じ形。
    // 上限(ADD_FISH_MANUAL_CAP・個体数上限)の扱いも add_fish/add_piranha と同じ。
    pub fn add_whale(&mut self, pix_w: usize, pix_h: usize) {
        self.add_fish_of_species(Species::Whale, pix_w, pix_h);
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
        let fish = self.roll_individuality(Fish::new(sp, Stage::Fry, x, y));
        self.fish.push(fish);
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

    // デバッグ用(`H`キー): 生きている全個体の空腹度を即座に0にする。腹ぺこ・病気・
    // 死亡までのカウントダウン等、空腹度が絡む挙動をすぐ試したい時のショートカット。
    // 死亡演出中(dead)の個体は対象外(既に育成ロジックから外れているため)。
    // デバッグ用(`K`キー): 産卵可能な条件(成魚・非病気・well_fed_timer>=BREED_READY_TIME)を
    // 満たす同種ペアを、種ごとに先頭から2匹ずつ組んで即座に交尾成立させる(確率判定を
    // 無視して、通常のupdate_breeding_pairsの成立処理と同じことを同期的に行う)。
    // ペアの中間地点にハート演出(Mate)と卵を出し、両方のwell_fed_timerをリセットする。
    // 求愛の接近待ち・確率待ちを飛ばして、交尾演出・産卵をすぐ試したい時のショートカット。
    pub fn debug_force_courtship_proximity(&mut self, pix_w: usize, pix_h: usize) {
        let sand_top = (pix_h as f64 - sand_height(pix_h) as f64).max(2.0);
        let ready: Vec<(usize, f64, f64, Species)> = self
            .fish
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                !f.dead
                    && !f.sick
                    && f.species.breeds()
                    && f.stage == Stage::Adult
                    && f.well_fed_timer >= BREED_READY_TIME
            })
            .map(|(i, f)| (i, f.x, f.y, f.species))
            .collect();

        let mut done_species: Vec<Species> = Vec::new();
        let mut mated = false;
        for &(_, _, _, sp) in &ready {
            if done_species.contains(&sp) {
                continue;
            }
            done_species.push(sp);
            let indices: Vec<usize> = ready
                .iter()
                .filter(|&&(_, _, _, s)| s == sp)
                .map(|&(i, _, _, _)| i)
                .collect();
            for pair in indices.chunks(2) {
                if let [i, j] = pair {
                    let mid_x = self.fish[*i].x;
                    let mid_y = self.fish[*i].y;
                    self.fish[*j].x = mid_x;
                    self.fish[*j].y = (mid_y + MATE_RADIUS * 0.5).max(1.0);
                    self.fish[*i].well_fed_timer = 0.0;
                    self.fish[*j].well_fed_timer = 0.0;
                    self.lay_egg_cluster(mid_x, sp, sand_top, pix_w as f64, true);
                    mated = true;
                }
            }
        }
        if mated {
            self.set_message("つがいを即座に交尾させた(デバッグ)");
        } else {
            self.set_message("産卵可能な同種ペアがいない(デバッグ)");
        }
    }

    pub fn debug_starve_all(&mut self) {
        for f in &mut self.fish {
            if !f.dead {
                f.hunger = 0.0;
            }
        }
        self.set_message("全員を空腹にした(デバッグ)");
    }

    // デバッグ用: 生きている全ての稚魚(Stage::Fry)を即座に成魚(Stage::Adult)にする。
    // 通常の成長遷移(update_biology)と同じくwell_fed_timerを0にリセットする。
    pub fn debug_grow_all_to_adult(&mut self) {
        let mut count = 0;
        for f in &mut self.fish {
            if !f.dead && f.stage == Stage::Fry {
                f.stage = Stage::Adult;
                f.well_fed_timer = 0.0;
                count += 1;
            }
        }
        self.set_message(format!("稚魚{count}匹を成魚にした(デバッグ)"));
    }

    // デバッグ用: 水質(pollution)を押すたびに0とPOLLUTION_MAXの間でトグルする。
    // 水質悪化の見た目・病気連動をすぐ試したい時のショートカット。
    pub fn debug_toggle_pollution(&mut self) {
        self.pollution = if self.pollution >= POLLUTION_MAX / 2.0 {
            0.0
        } else {
            POLLUTION_MAX
        };
        self.set_message(if self.pollution >= POLLUTION_MAX / 2.0 {
            "水質を最悪にした(デバッグ)"
        } else {
            "水質を綺麗にした(デバッグ)"
        });
    }

    // デバッグ用: 生きている個体からランダムに1匹選び、即座に死亡させる。
    // 死亡演出(浮上→沈降)・カニによる片付け・水質悪化をすぐ試したい時のショートカット。
    pub fn debug_kill_random_fish(&mut self) {
        let alive_indices: Vec<usize> =
            self.fish.iter().enumerate().filter(|(_, f)| !f.dead).map(|(i, _)| i).collect();
        if alive_indices.is_empty() {
            self.set_message("生きている個体がいない(デバッグ)");
            return;
        }
        let idx = alive_indices[self.rng.range_usize(0, alive_indices.len() - 1)];
        let sp = self.fish[idx].species;
        self.fish[idx].dead = true;
        self.fish[idx].dead_timer = 0.0;
        self.set_message(format!("{}を強制的に死亡させた(デバッグ)", species_name(sp)));
    }

    // デバッグ用: ランダムな生存個体を1匹選び、即死させるのではなく寿命(老衰死)の
    // 残りをちょうど10秒にする。老衰死の経路(LIFESPAN_DEATH_AGE)を待たずに
    // 手早く確認できるようにするためのXキーの寿命版。
    pub fn debug_age_random_fish_near_death(&mut self) {
        let alive_indices: Vec<usize> =
            self.fish.iter().enumerate().filter(|(_, f)| !f.dead).map(|(i, _)| i).collect();
        if alive_indices.is_empty() {
            self.set_message("生きている個体がいない(デバッグ)");
            return;
        }
        let idx = alive_indices[self.rng.range_usize(0, alive_indices.len() - 1)];
        let sp = self.fish[idx].species;
        let lifespan_mult = self.fish[idx].lifespan_mult;
        self.fish[idx].age = (LIFESPAN_DEATH_AGE * lifespan_mult - 10.0).max(0.0);
        self.set_message(format!("{}の寿命を残り10秒にした(デバッグ)", species_name(sp)));
    }

    // デバッグ用: スター(無敵アイテム)をカーソル位置に確実に投入する。既にスターが
    // 出ている場合は何もしない(update_starsの「同時に複数出さない」方針に合わせる)。
    // 押すたびにカーソル周辺へ1個ずつ追加する(何個でも重ねて投入できる。ネタ機能の
    // ため出現数を制限する必要はない)。カーソルとスターは同じ十字形を描くため、
    // カーソルにちょうど重なる位置(距離0)には置かず、STAR_CURSOR_OFFSET以上
    // 離れたランダムな方向・距離に散らして置く。
    pub fn debug_spawn_star(&mut self, cursor_x: f64, cursor_y: f64, pix_w: usize, pix_h: usize) {
        let angle = self.rng.range(0.0, std::f64::consts::TAU);
        let dist = self.rng.range(STAR_CURSOR_OFFSET, STAR_CURSOR_OFFSET + STAR_SPAWN_SCATTER_RADIUS);
        let (x, y) = clamp_point(
            cursor_x + dist * angle.cos(),
            cursor_y + dist * angle.sin(),
            pix_w,
            pix_h,
        );
        self.stars.push(Star {
            x,
            y,
            life: STAR_LIFETIME,
            phase: self.rng.range(0.0, std::f64::consts::TAU),
        });
        self.set_message("スターを投入した(デバッグ)");
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

        // 渦の中心を先に更新する。後続の各update_*が同じtickの中心座標を使って
        // current_at()で位置ごとの力場を求めるため、必ず最初に呼ぶ。
        // (水流の筋の管理update_current_streaksは見た目だけの演出なので、気泡
        // (update_bubbles)と並べて描画系エンティティの更新側でまとめて呼ぶ)。
        self.update_current(pix_w as f64, pix_h as f64);
        self.update_octopus(dt);
        // 出ているタコを成魚がかじって弱らせる判定。update_octopusの直後に置いて、
        // このtickの隠れ/出現状態を反映した上で判定する。
        self.update_octopus_bites(dt);
        self.update_courtship(dt);
        self.update_movement(dt, pix_w as f64, sand_top);
        self.update_food(dt, sand_top, pix_w);
        self.update_medicine(dt, sand_top, pix_w);
        self.update_meat(dt, sand_top, pix_w);
        // 浄化剤は着水した瞬間にpurifier_concentrationを立てるため、その濃度を読む
        // update_biology(老化・食欲)より前に処理しておく。
        self.update_purifiers(dt, sand_top, pix_w);
        self.update_stars(dt);
        self.update_cameos(dt, pix_w as f64, sand_top);
        self.update_biology(dt, cap, pix_w as f64, sand_top);
        self.update_pollution(dt);
        // 浄化剤による水質浄化+濃度の希釈。水質と相互作用するのでupdate_pollutionの直後に置く。
        self.update_purifier_concentration(dt);
        self.update_predation(dt);
        self.update_crabs(dt, pix_w as f64, sand_top);
        self.update_shrimp(dt, pix_w as f64);
        self.update_seahorses(dt);
        self.update_bubbles(dt, pix_w as f64, pix_h as f64);
        self.update_current_streaks(dt, pix_w as f64, pix_h as f64);
        self.update_effects(dt);
    }

    // つがい形成(新規): 産卵可能(満腹維持でBREED_READY_TIME経過)な成魚が、近くに
    // 同種で同じく産卵可能な相手がいれば、緩やかに惹かれ合う(通常の遊泳より少し
    // 強い程度の吸引ベクトル。update_movement側の各種ベクトルとは別に、ここで
    // vx/vyへ直接加算する。同フレームのupdate_movement側の減衰・最高速度クランプが
    // そのまま乗るので、暴走して追い回すような強さにはならない)。
    fn update_courtship(&mut self, dt: f64) {
        let ready: Vec<(usize, f64, f64, Species)> = self
            .fish
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                !f.dead
                    && !f.sick
                    && f.species.breeds()
                    && f.stage == Stage::Adult
                    && f.well_fed_timer >= BREED_READY_TIME
            })
            .map(|(i, f)| (i, f.x, f.y, f.species))
            .collect();

        if ready.len() < 2 {
            return;
        }

        for &(i, fx, fy, sp) in &ready {
            let mut best = f64::INFINITY;
            let mut best_pos = None;
            for &(j, ox, oy, osp) in &ready {
                if j == i || osp != sp {
                    continue;
                }
                let d = (ox - fx).powi(2) + (oy - fy).powi(2);
                if d < best {
                    best = d;
                    best_pos = Some((ox, oy));
                }
            }
            let Some((tx, ty)) = best_pos else { continue };
            let dist = best.sqrt();
            if dist >= COURTSHIP_RADIUS || dist < 0.001 {
                continue;
            }
            let pull = COURTSHIP_PULL * (1.0 - dist / COURTSHIP_RADIUS);
            self.fish[i].vx += (tx - fx) / dist * pull * dt;
            self.fish[i].vy += (ty - fy) / dist * pull * dt;
        }
    }

    // つがいが十分接近したら交尾成立→産卵(新規)。産卵可能な同種2匹がMATE_RADIUS以内
    // まで近づいたタイミングでBREED_CHANCE_PER_SECの確率判定を行い、成立したら
    // 2匹の中間地点に卵を産む(両方の満腹タイマーを消費する)。1tickで1匹の魚が
    // 複数のペアに重複して参加しないよう、既に成立した相手はpairedで除外する。
    fn update_breeding_pairs(&mut self, dt: f64, spawn_eggs: &mut Vec<(f64, f64, Species, bool)>) {
        let ready: Vec<(usize, f64, f64, Species)> = self
            .fish
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                !f.dead
                    && !f.sick
                    && f.species.breeds()
                    && f.stage == Stage::Adult
                    && f.well_fed_timer >= BREED_READY_TIME
            })
            .map(|(i, f)| (i, f.x, f.y, f.species))
            .collect();

        let mut paired = vec![false; self.fish.len()];
        for &(i, fx, fy, sp) in &ready {
            if paired[i] {
                continue;
            }
            let mut best = f64::INFINITY;
            let mut best_j = None;
            for &(j, ox, oy, osp) in &ready {
                if j == i || osp != sp || paired[j] {
                    continue;
                }
                let d = ((ox - fx).powi(2) + (oy - fy).powi(2)).sqrt();
                if d < MATE_RADIUS && d < best {
                    best = d;
                    best_j = Some((j, ox, oy));
                }
            }
            let Some((j, ox, oy)) = best_j else { continue };
            if self.rng.next_f64() >= BREED_CHANCE_PER_SEC * dt {
                continue;
            }
            paired[i] = true;
            paired[j] = true;
            self.fish[i].has_mated = true;
            self.fish[j].has_mated = true;
            let mid_x = (fx + ox) / 2.0;
            let mid_y = (fy + oy) / 2.0;
            self.fish[i].well_fed_timer = 0.0;
            self.fish[j].well_fed_timer = 0.0;
            // ハート演出は交尾した実際の位置(mid_x, mid_y。水中のどこでもありえる)
            // ではなく、卵が実際に現れる水底付近の位置に合わせて出す必要があるため、
            // ここでは出さずlay_egg_cluster側に委ねる(mated=trueを渡し、卵と同じ
            // 座標にハートを出させる)。
            spawn_eggs.push((mid_x, mid_y, sp, true));
            self.set_message(format!("{}のつがいが交尾した", species_name(sp)));
        }
    }

    // 産卵イベントを卵に変換する(2〜4個、指定位置の周辺・水底付近に配置)。
    // 産卵時にキラキラ光るフラッシュ演出(Spawn)も一緒に出す。`mated`がtrueの
    // 場合(つがいの交尾が成立した場合)は、交尾のハート演出(Mate)も卵と全く同じ
    // 座標(px, flash_y)に出す。以前は交尾した実際の位置(水中のどこでもありえる)に
    // ハートを出していたため、卵(常に水底付近)と場所がズレて不自然に見えるバグが
    // あった。戻り値はステータスバー用のメッセージ(呼び出し側でまとめて処理するため)。
    fn lay_egg_cluster(&mut self, px: f64, sp: Species, sand_top: f64, pix_w: f64, mated: bool) -> String {
        let n = self.rng.range_usize(2, 4);
        let flash_y = (sand_top - 1.5).max(1.0);
        for _ in 0..n {
            let ex = (px + self.rng.range(-4.0, 4.0)).clamp(1.0, safe_upper(pix_w - 1.0));
            let ey = (sand_top - self.rng.range(0.5, 2.5)).max(1.0);
            self.eggs.push(Egg {
                x: ex,
                y: ey,
                species: sp,
                hatch: EGG_HATCH_TIME,
            });
        }
        self.drop_effects.push(DropEffect {
            x: px,
            y: flash_y,
            life: SPAWN_FLASH_LIFETIME,
            max_life: SPAWN_FLASH_LIFETIME,
            kind: EffectKind::Spawn,
        });
        if mated {
            self.drop_effects.push(DropEffect {
                x: px,
                y: flash_y,
                life: MATE_EFFECT_LIFETIME,
                max_life: MATE_EFFECT_LIFETIME,
                kind: EffectKind::Mate,
            });
        }
        format!("{}が卵を産んだ", species_name(sp))
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
                    piranha_still_hungry(f) && f.predation_cooldown <= 0.0,
                )
            })
            .collect();
        // 種類を問わず「すぐ近くまで寄ってきた魚」の位置スナップショット(墨のもう一つの
        // トリガー判定に使う)。自分を含むタコ全種・死骸・隠れている魚・藻や岩に隠れている
        // 魚は対象から除く。
        let nearby_fish: Vec<(f64, f64)> = self
            .fish
            .iter()
            .filter(|f| {
                f.species != Species::Octopus
                    && !f.dead
                    && !f.hidden
                    && !self.is_hidden_in_cover(f.x, f.y)
            })
            .map(|f| (f.x, f.y))
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

            // 墨: 出ている間、(1)近くに捕食モードのピラニアがいる(追われているとみなす)、
            // または(2)種類を問わず魚がすぐ目の前まで寄ってきた、のいずれかで吐く。
            if !f.hidden && f.ink_cooldown <= 0.0 {
                let threatened = piranhas.iter().any(|&(sx, sy, hunting)| {
                    hunting
                        && ((sx - f.x).powi(2) + (sy - f.y).powi(2)).sqrt()
                            < OCTOPUS_INK_TRIGGER_RADIUS
                });
                let fish_approached = nearby_fish.iter().any(|&(sx, sy)| {
                    ((sx - f.x).powi(2) + (sy - f.y).powi(2)).sqrt() < OCTOPUS_INK_NEARBY_FISH_RADIUS
                });
                if threatened || fish_approached {
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

    // 出ているタコを、近くにいる生きた成魚(種を問わない=ピラニアも含む)がかじって
    // 弱らせる仕組み(ピラニアの被噛みつきと対になる、役割が逆の仕組み)。
    // OCTOPUS_BITES_TO_DIE回かじられると力尽きる。同時に何匹もいても一瞬で殺され
    // ないよう、一度かじられたらOCTOPUS_BITE_IMMUNITY_TIMEの間は追加のかじり判定を受けない。
    fn update_octopus_bites(&mut self, dt: f64) {
        // 生きて出ている(隠れていない)タコのスナップショット(index, x, y, かじられ回数,
        // 免疫残り時間)。借用の都合で先に集めてから本体を書き換える(このコードベース
        // 共通のパターン)。
        let octopuses: Vec<(usize, f64, f64, u8, f64)> = self
            .fish
            .iter()
            .enumerate()
            .filter(|(_, f)| f.species == Species::Octopus && !f.dead && !f.hidden)
            .map(|(i, f)| {
                (
                    i,
                    f.x,
                    f.y,
                    f.octopus_bite_count,
                    f.octopus_bite_immunity_timer,
                )
            })
            .collect();

        for (oi, ox, oy, bite_count, immunity_timer) in octopuses {
            if immunity_timer <= 0.0 {
                // 近くに、生きている成魚(種を問わない)がいるか探す。ここはピラニアが
                // 捕食者ではなく「かじる側」として参加する唯一の仕組みなので、捕食者
                // だからといって除外しない。
                let biter_in_range = self.fish.iter().enumerate().any(|(j, f)| {
                    if j == oi
                        || f.species == Species::Octopus
                        || f.dead
                        || f.stage != Stage::Adult
                    {
                        return false;
                    }
                    let dx = f.x - ox;
                    let dy = f.y - oy;
                    dx * dx + dy * dy <= OCTOPUS_BITE_RADIUS * OCTOPUS_BITE_RADIUS
                });
                if biter_in_range && self.rng.next_f64() < OCTOPUS_BITE_CHANCE_PER_SEC * dt {
                    // かじられた: 回復タイマーをリセットし、免疫時間を立てる。
                    self.fish[oi].octopus_bite_recover_timer = 0.0;
                    self.fish[oi].octopus_bite_immunity_timer = OCTOPUS_BITE_IMMUNITY_TIME;
                    // 何回目のかじられか(増分前の値+1)をメッセージの出し分けに使う。
                    let bite_number = bite_count + 1;
                    if bite_count + 1 >= OCTOPUS_BITES_TO_DIE {
                        // 最後の一かじり: Xキー(debug_kill_random_fish)と同じ死亡状態にする。
                        // タコつぼの後始末は死因を問わない既存の汎用経路(update_biologyの
                        // CORPSE_REMOVE_TIME経過時・update_crabsのカニ片付け時)に任せる。
                        self.fish[oi].dead = true;
                        self.fish[oi].dead_timer = 0.0;
                    } else {
                        self.fish[oi].octopus_bite_count += 1;
                    }
                    // 小さな血しぶき(噛みつき捕食時のBLOOD_PARTICLE_COUNTほど派手にしない)。
                    for _ in 0..BLEED_TRICKLE_PARTICLE_COUNT {
                        let px = ox + self.rng.range(-BLOOD_SPREAD_RADIUS, BLOOD_SPREAD_RADIUS);
                        let py =
                            oy + self.rng.range(-BLOOD_SPREAD_RADIUS * 0.6, BLOOD_SPREAD_RADIUS * 0.6);
                        let particle_life = BLOOD_EFFECT_LIFETIME * self.rng.range(0.6, 1.0);
                        self.drop_effects.push(DropEffect {
                            x: px,
                            y: py,
                            life: particle_life,
                            max_life: particle_life,
                            kind: EffectKind::Blood,
                        });
                    }
                    self.sound_events.push(SfxEvent::Predation);
                    // かじられ段階に応じてメッセージを変える。最後の一かじり(死亡)は
                    // 他の死因と同じ「力尽きた」の言い回しにそろえる。
                    let msg = if bite_number >= OCTOPUS_BITES_TO_DIE {
                        "タコが魚にかじられ力尽きた…".to_string()
                    } else if bite_number == 1 {
                        "タコが魚にかじられた…".to_string()
                    } else {
                        "タコが魚にかじられ弱ってきた…".to_string()
                    };
                    self.set_message(msg);
                }
            }
            // かじられたかどうかに関わらず、全タコの免疫時間を進める。
            self.fish[oi].octopus_bite_immunity_timer =
                (self.fish[oi].octopus_bite_immunity_timer - dt).max(0.0);
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
        for s in &mut self.blood_scents {
            s.life -= dt;
        }
        self.blood_scents.retain(|s| s.life > 0.0);
        for c in &mut self.ink_clouds {
            c.life -= dt;
        }
        self.ink_clouds.retain(|c| c.life > 0.0);
        for b in &mut self.purify_blooms {
            b.life -= dt;
            // 拡散(PURIFY_BLOOM_GROWTH_TIME)が完了した瞬間に、着水時ではなく
            // ここで初めて濃度を加算する(見た目の拡散と効果の発動を揃えるため)。
            // 連投すると1.0を超えて積み上がる(上限なし)。水質浄化・食欲不振・
            // 老化加速のいずれも濃度に比例するだけの式なので、1.0超でも自然に
            // 効果が強まる(薄まりきるまでの時間も連投した分だけ長引く)。
            if !b.activated && b.max_life - b.life >= PURIFY_BLOOM_GROWTH_TIME {
                b.activated = true;
                self.purifier_concentration += 1.0;
            }
        }
        // 発動済み(activated)のブルームは、以降は同心円状の広がり演出(main.rs側)にも
        // 均一な紫染め(purifier_concentration)にも二重に寄与しないよう、その場で取り除く。
        self.purify_blooms.retain(|b| !b.activated && b.life > 0.0);
    }

    // 遊泳: ランダムウォーク+慣性+壁反射+群れ+餌吸引(空腹度・病気で速度が変化)。
    // 死亡演出中の個体はここでは動かさず、水面近くまでゆっくり浮上して静止するだけにする。
    fn update_movement(&mut self, dt: f64, w: f64, sand_top: f64) {
        // 群れ計算のため位置・速度をスナップショット(self.fish とインデックスを揃えるため
        // 死亡個体もそのまま含め、死亡フラグで群れ対象から除外する)
        // hunger/predation_cooldown も持たせて、他の魚が「近くのピラニアが今まさに
        // 捕食モードかどうか」を判定できるようにする(逃走ベクトルの判定に使う)。
        let snap: Vec<(
            Species,
            f64,
            f64,
            f64,
            f64,
            bool,
            f64,
            f64,
            bool,
            u8,
            u8,
            bool,
            bool,
            Stage,
            bool,
        )> = self
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
                    f.stage,                           // タコの捕食対象制限(稚魚のみ対象)に使う
                    f.sick,                            // 病気はタコの捕食対象判定には使わなくなった(引数の並び維持のため渡すのみ)
                )
            })
            .collect();

        let margin: f64 = 4.0;
        let top_margin: f64 = 3.0;
        let wall_push = 70.0;
        // 水質が悪化していると、捕食者でない通常種は食欲そのものを失い、餌を
        // 探して寄っていかなくなる(空腹度自体は別途update_biology側でより速く
        // 減っていくため、餌を放置すれば結果的に餓死しやすくなる)。浄化剤の効果中も
        // 同様に食欲を失う(濃度が残っている間は餌に寄っていかない)。
        let food_appetite_lost = self.pollution >= POLLUTION_MAX * POLLUTION_SICK_ELIGIBLE_FRAC
            || self.purifier_concentration > 0.0;

        for i in 0..self.fish.len() {
            if self.fish[i].dead {
                // 死んだ魚: 体内のガスによる浮力(時間とともに指数関数的に減衰)と
                // 重力・水の抵抗を積分し、「最初は浮いて漂うが、やがて沈んで水底の
                // 亡骸になる」動きを連続的に計算する(状態を切り替える作りにはしない)。
                // 縦の移動中・静止中を問わず、ゆらゆらと左右に揺れながら漂う。
                let f = &mut self.fish[i];
                let bottom_y = safe_upper(sand_top - 1.0);
                // カーソル近くで叩かれた(つつかれた)死骸は、以降は浮力を無視して
                // 重力だけで沈む(つついたらすぐ沈められるようにする要望への対応)。
                let buoyancy = if f.sink_forced {
                    0.0
                } else {
                    CORPSE_BUOYANCY_INITIAL * (-f.dead_timer / CORPSE_BUOYANCY_DECAY_TAU).exp()
                };
                // y は下向きが正のため、浮力優勢(浮上)ならvyは負(減少)方向、
                // 重力優勢(沈降)ならvyは正(増加)方向に加速する必要がある。
                let net_accel = CORPSE_GRAVITY_ACCEL - buoyancy; // 正なら沈降側、負なら浮上側に加速
                f.vy += net_accel * dt;
                f.vy *= (1.0 - CORPSE_DRAG_PER_SEC * dt).clamp(0.0, 1.0);
                f.y += f.vy * dt;
                // 水面・水底に達したら、それ以上その方向へ押す速度成分だけ打ち消す
                // (水面に浮力で押し付けられている間はそこに留まり、浮力が重力を
                // 下回れば自然に沈み始める)。
                if f.y <= DEAD_SURFACE_MARGIN && f.vy < 0.0 {
                    f.y = DEAD_SURFACE_MARGIN;
                    f.vy = 0.0;
                }
                if f.y >= bottom_y && f.vy > 0.0 {
                    f.y = bottom_y;
                    f.vy = 0.0;
                }
                // ゆらゆらと左右に揺れるのは、浮上中・沈降中のみ。水底に沈み切って
                // 静止した亡骸は、堆積した餌・薬と同様に静かに横たわったままにする
                // (揺れ続けると死骸というより漂っているように見えてしまうため)。
                // 浮力は時間とともに単調に減衰するだけなので、一度水底に着地したら
                // 再び浮くことはなく、この判定だけで「着地済み」を表せる。
                let settled_at_bottom = f.y >= bottom_y - 0.01;
                if !settled_at_bottom {
                    // f.ageは死亡時点で増加が止まるため、個体ごとに位相をずらす
                    // 安定したオフセットとして使える。
                    let sway_phase = f.dead_timer * DEAD_SWAY_ANGULAR_SPEED + f.age * 1.7;
                    f.x += DEAD_SWAY_AMPLITUDE * DEAD_SWAY_ANGULAR_SPEED * sway_phase.cos() * dt;
                    f.x = f.x.clamp(1.0, safe_upper(w - 1.0));
                }
                f.y = f.y.clamp(1.0, bottom_y);
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
                attract_timer,
                attract_dx,
                attract_dy,
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
                meals_since_full,
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
                    f.attract_timer,
                    f.attract_dx,
                    f.attract_dy,
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
                    f.piranha_meals_since_full,
                    // 魚が水底に張り付いて見えるという指摘への対応: 成長段階で
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
            let nearest_food = if hunger < HUNGRY_THRESHOLD
                && !self.food.is_empty()
                && !(food_appetite_lost && !sp.is_predator())
            {
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

            // スター(無敵アイテム)への誘引: 取得できるのは捕食者でない・まだ無敵でない
            // 個体だけなので、それ以外は反応しない。バグ修正(新規): 以前は誘引ベクトルが
            // 存在せず、偶然の遊泳で触れない限り誰もスターに近づいて行かなかった。
            let nearest_star = if !sp.is_predator() && !is_invincible_self && !self.stars.is_empty() {
                let mut best = f64::INFINITY;
                let mut best_pos = None;
                for s in &self.stars {
                    let d = (s.x - fx).powi(2) + (s.y - fy).powi(2);
                    if d < best {
                        best = d;
                        best_pos = Some((s.x, s.y));
                    }
                }
                best_pos.map(|pos| (pos, best.sqrt().max(0.001)))
            } else {
                None
            };

            // 捕食者(ピラニア・タコ)ごとの狩りパラメータ。タコもピラニアと同じ「頻繁に狩る」
            // 方針を共有しつつ、専用の定数で独立に調整できるようにしている。
            // 無敵中の一時的捕食者(本来は捕食されない側の魚)にも、同様に狩りの吸引
            // パラメータを与える。そうしないと、追跡中でない限り出会えず「捕食者を
            // 倒せる」だけで実際には追いかけ回さない(見た目に地味な)ギミックに
            // なってしまうため。
            let (hunt_threshold, hunt_radius, hunt_pull) = match sp {
                Species::Piranha => (PIRANHA_HUNT_HUNGER_THRESHOLD, PIRANHA_HUNT_RADIUS, PIRANHA_HUNT_PULL),
                Species::Octopus => (
                    OCTOPUS_HUNT_HUNGER_THRESHOLD,
                    OCTOPUS_HUNT_RADIUS,
                    OCTOPUS_HUNT_PULL,
                ),
                _ if is_invincible_self => (MAX_HUNGER, STAR_HUNT_RADIUS, STAR_HUNT_PULL),
                _ => (0.0, 0.0, 0.0), // 非捕食者(かつ無敵でもない)では使わない
            };
            // 墨が近くに広がっている間、捕食者は獲物を検知できない(「視界が悪くなる」演出)。
            // 描画側のアニメーション曲線とは独立に、ゲームロジック側はINK_MAX_RADIUS基準で
            // シンプルに判定する。
            let blinded_by_ink = sp.is_predator()
                && self
                    .ink_clouds
                    .iter()
                    .any(|c| ((c.x - fx).powi(2) + (c.y - fy).powi(2)).sqrt() < INK_MAX_RADIUS);
            // ピラニアは、既に1匹以上捕食済み(meals_since_full>0)かつPIRANHA_KILLS_TO_FULL
            // 未満の間は、hungerが閾値以上でも狩りをやめない(食欲を旺盛にする実機
            // フィードバック対応)。まだ一度も捕食していない場合は通常のhunger判定のみ。
            let still_hungry_for_hunt = hunger < hunt_threshold
                || (sp == Species::Piranha
                    && meals_since_full > 0
                    && meals_since_full < PIRANHA_KILLS_TO_FULL);

            // ピラニア専用の肉餌(`M`キー)。満腹の間は肉餌とはいえ食いつかない
            // (still_hungry_for_hunt=falseの間は無視する。通常の狩りhunger判定と
            // 同じ基準を共有する)。
            let nearest_meat = if sp == Species::Piranha && still_hungry_for_hunt && !self.meat.is_empty() {
                let mut best = f64::INFINITY;
                let mut best_pos = None;
                for mt in &self.meat {
                    let d = (mt.x - fx).powi(2) + (mt.y - fy).powi(2);
                    if d < best {
                        best = d;
                        best_pos = Some((mt.x, mt.y));
                    }
                }
                best_pos.map(|pos| (pos, best.sqrt().max(0.001)))
            } else {
                None
            };
            // 捕食者の狩り: 空腹度が閾値未満・クールダウン明けなら、近くの獲物を先に探しておく
            // (自分と同種・ピラニア同士・タコからピラニアは対象外。タコが隠れている間も対象外)。
            // 追いかけている間は通常の遊泳を弱め、吸引ベクトルをはっきり優先させる。
            let chase_target = if (sp.is_predator() || is_invincible_self)
                && !blinded_by_ink
                && still_hungry_for_hunt
                && predation_cooldown <= 0.0
            {
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
                        phunger,
                        _pcooldown,
                        phidden,
                        pgrowth,
                        pkill,
                        pinvincible,
                        pcover,
                        pstage,
                        psick,
                    ),
                ) in snap.iter().enumerate()
                {
                    let p_hungry = phunger < HUNGRY_THRESHOLD;
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
                        pstage,
                        psick,
                        p_hungry,
                    ) {
                        continue;
                    }
                    let d = (px - fx).powi(2) + (py - fy).powi(2);
                    if d < best {
                        best = d;
                        best_pos = Some((px, py));
                    }
                }
                // 無敵中の一時的捕食者は、スターへの誘引と同程度に徹底的に追いかけ
                // 回すため、検知範囲を距離無制限にする(hunt_radiusによる打ち切りをしない)。
                best_pos.map(|pos| (pos, best.sqrt().max(0.001))).filter(|&(_, dist)| {
                    is_invincible_self || dist < hunt_radius
                })
            } else {
                None
            };
            // 追跡中かどうか(捕食モードで獲物を追っている間だけ、後段で最高速度を
            // 通常3種よりはっきり速くブーストする)
            let is_chasing = chase_target.is_some();

            // 被食者側の警戒: 近くにピラニアがいたら常に検知する(方針変更: 「みんなピラニアが
            // 嫌い」という設定にするため、ピラニアが捕食モードかどうかは問わない)。
            // タコもピラニアに襲われる対象なので、ピラニア自身以外は全種がこの警戒に参加する。
            // 青い魚がタコに向かって突進して捕食されるというバグ報告への対応:
            // 通常の魚(タコ自身を除く)は、タコからも同様に警戒・回避する(ピラニアと同じ
            // 回り込み・機敏な逃走ロジックをそのまま再利用する)。タコが隠れている間は
            // 見えていないので警戒対象にしない。タコ自身は同種(他のタコ)を警戒しない。
            // スター(無敵アイテム)取得中は、逆に両方を捕食できる側になるため怖がらない。
            let fear_target = if sp != Species::Piranha && !is_invincible_self {
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
                        _pgrowth,
                        _pkill,
                        _pinvincible,
                        _pcover,
                        _pstage,
                        _psick,
                    ),
                ) in snap.iter().enumerate()
                {
                    if j == i || pdead {
                        continue;
                    }
                    let is_threat = psp == Species::Piranha
                        || (psp == Species::Octopus && psp != sp && !phidden);
                    if !is_threat {
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
            // 名前は旧仕様(ピラニアのみ)からの継続だが、現在はピラニア・タコいずれかから
            // 逃走中であることを表す(is_fleeing_piranha → is_fleeing_predator)。
            let is_fleeing_predator = fear_target.is_some();

            // 血の匂いの追跡(新規): ピラニアは、既存の狩り(chase_target。満腹・クールダウン
            // 中はゲートされる)とは独立に、検知範囲内に血の匂い(出血元)があれば満腹中・
            // クールダウン中でも優先的にそちらへ向かう。「近くの獲物が今まさに空腹判定で
            // 狩れるか」に関わらず出血元に群がる、という生態を表現する追加の吸引ベクトル。
            let blood_scent_target = if sp == Species::Piranha {
                let mut best = f64::INFINITY;
                let mut best_pos = None;
                for sc in &self.blood_scents {
                    let d = (sc.x - fx).powi(2) + (sc.y - fy).powi(2);
                    if d < best {
                        best = d;
                        best_pos = Some((sc.x, sc.y));
                    }
                }
                best_pos
                    .map(|pos| (pos, best.sqrt()))
                    .filter(|&(_, dist)| dist < PIRANHA_BLOOD_SCENT_RADIUS)
            } else {
                None
            };

            // 餌を追っている・獲物を追っている・ピラニアから逃げている、のいずれかの間は
            // 通常の遊泳(ランダムウォーク・群れ)を大きく弱め、該当のベクトルを
            // はっきり優先させる(「一直線に向かう/逃げる」のが見た目でわかるように)。
            let normal_move_mix = if is_fleeing_predator {
                // ピラニア・タコから逃走中は、通常の遊泳(ランダムウォーク・群れ)を
                // 他の状態より一段強く抑え、逃走ベクトルが確実に勝つようにする
                // (フラフラ近づいてしまうバグの再発防止。下のPIRANHA_FEAR_MIN_STRENGTH_FRACと
                // セットで、検知範囲内なら常に逃走が優先されることを保証する)。
                PIRANHA_FEAR_MOVE_DAMP
            } else if nearest_food.is_some()
                || nearest_meat.is_some()
                || nearest_star.is_some()
                || chase_target.is_some()
                || blood_scent_target.is_some()
            {
                HUNGRY_NORMAL_MOVE_DAMP
            } else {
                1.0
            };

            // ランダムウォーク(縦は控えめ)。空腹度・病気に応じて活発さが変わるほか、
            // サイズが小さいほど(稚魚・成長段階が低いほど)agilityが1.0を超えて強くなり、
            // 大きいほど1.0未満になって弱まる(通常の遊泳だけに効かせる)
            ax += self.rng.signed() * sp.wander() * spd_mult * agility * normal_move_mix;
            ay += self.rng.signed() * sp.wander() * 0.55 * spd_mult * agility * normal_move_mix;

            // 群れ: 同種近傍の平均速度に少し寄せる(死亡個体は対象外)。
            // ついでに同じ走査で、近くにある死骸(dead=true、種は問わない)からの
            // 忌避ベクトルも積算しておく(専用ループを新設せず走査コストを避ける)。
            let (mut svx, mut svy, mut cnt) = (0.0, 0.0, 0);
            let (mut corpse_ax, mut corpse_ay) = (0.0, 0.0);
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
                    _ostage,
                    _osick,
                ),
            ) in snap.iter().enumerate()
            {
                if j == i {
                    continue;
                }
                if odead {
                    let d = ((ox - fx).powi(2) + (oy - fy).powi(2)).sqrt();
                    if d < CORPSE_AVOID_RADIUS && d > 0.001 {
                        let strength = CORPSE_AVOID_STRENGTH * (1.0 - d / CORPSE_AVOID_RADIUS);
                        corpse_ax += (fx - ox) / d * strength;
                        corpse_ay += (fy - oy) / d * strength;
                    }
                    continue;
                }
                if osp != sp {
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
            // 死骸忌避は空腹・逃走・追跡中でも常に効かせる(normal_move_mixの
            // 減衰対象には含めない)。あくまで背景的な弱い反発なので、他の強い
            // 意思決定(逃走・捕食)を上書きすることはない。
            ax += corpse_ax;
            ay += corpse_ay;

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

            // スターへの誘引: 空腹度に関わらず、見つけたら一直線に向かう
            // (ゲームとして面白い一時的パワーアップのため、餌のような空腹条件は設けない)。
            if let Some(((bx, by), dist)) = nearest_star {
                ax += (bx - fx) / dist * STAR_ATTRACT_PULL;
                ay += (by - fy) / dist * STAR_ATTRACT_PULL;
            }

            // 捕食者の狩り: 探しておいた獲物へ向かって強く近づく(実際の捕食判定は
            // update_predation 側で行う)。通常の遊泳は上で damp 済みなので、
            // この吸引ベクトルが「追いかけている」動きとしてはっきり見えるようにする。
            if let Some(((bx, by), dist)) = chase_target {
                ax += (bx - fx) / dist * hunt_pull;
                ay += (by - fy) / dist * hunt_pull;
            }

            // ピラニア専用の肉餌への吸引(空腹の間だけ。nearest_meat自体が
            // still_hungry_for_huntでゲートされている)。狩り吸引(hunt_pull)と
            // 同じ強さを使い、通常の獲物と同様に一直線に向かう動きにする。
            if let Some(((bx, by), dist)) = nearest_meat {
                ax += (bx - fx) / dist * PIRANHA_HUNT_PULL;
                ay += (by - fy) / dist * PIRANHA_HUNT_PULL;
            }

            // 血の匂いへの追跡(新規): chase_targetの狩りゲート(空腹度・クールダウン)を
            // 無視して、満腹中・クールダウン中のピラニアも含めて出血元へ向かう。
            // chase_targetと同じ獲物が出血元でもある場合は両方のベクトルが乗り、より
            // はっきり「優先的に追跡している」動きになる。
            if let Some(((bx, by), dist)) = blood_scent_target {
                let d = dist.max(0.001);
                ax += (bx - fx) / d * PIRANHA_BLOOD_SCENT_PULL;
                ay += (by - fy) / d * PIRANHA_BLOOD_SCENT_PULL;
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
                // 検知範囲(PIRANHA_FEAR_RADIUS)の縁ではdist≈radiusとなり、距離だけの
                // 比例では強さがほぼ0になってしまう(=検知していてもフラフラ近づける)。
                // PIRANHA_FEAR_MIN_STRENGTH_FRACを下限として確保し、検知した瞬間から
                // 常にはっきり距離を取る動きになるようにする。
                let falloff = (1.0 - raw_dist / PIRANHA_FEAR_RADIUS).max(0.0);
                let ramp = PIRANHA_FEAR_MIN_STRENGTH_FRAC + (1.0 - PIRANHA_FEAR_MIN_STRENGTH_FRAC) * falloff;
                let strength = PIRANHA_FEAR_STRENGTH * ramp * escape_boost;
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
            // 壁際に追い詰めた魚を永遠に捕食できないという指摘への対応:
            // 追跡中(捕食者側)・逃走中(被食者側、ピラニアからの逃走・ガラスの驚き逃げ)は
            // 反発力を無効化するだけでなく、マージン自体も基本値(サイズ非依存)に戻す。
            // サイズ基準マージンのままだと、捕食者と被食者でスプライトの大きさが違う場合に
            // 壁際・角で「自分の取れる位置」の余白が個体ごとに変わってしまい、大きい捕食者
            // だけが十分壁・角に近づけず、獲物との間に絶対に詰め切れない隙間が残ってしまう
            // (実際に発生した根本原因: 追跡・逃走中もサイズ基準マージンのままだったため)。
            // 追跡・逃走中だけ基本マージンに揃えることで、サイズに関わらず同じだけ
            // 壁・角へ詰められるようにする。壁の外に出ないこと自体は後段の位置クランプ
            // (ハード上限。ここで決めたマージンをそのまま使う)が保証する。
            // 大きく成長したピラニアが底面に堆積した肉餌・餌を食べられないという指摘への
            // 対応: 肉餌(nearest_meat)・通常の餌(nearest_food)
            // への接近中も、生きた獲物の追跡中(is_chasing)と同様にマージンを基本値へ戻す。
            // そうしないと、大きく育った個体ほどサイズ基準のマージンで壁際・水底に十分
            // 近づけず、水底に着地した餌・肉餌との間に詰め切れない隙間が残ってしまう。
            let wall_push_suppressed = is_chasing
                || nearest_food.is_some()
                || nearest_meat.is_some()
                || is_fleeing_predator
                || flee_timer > 0.0
                || ink_escape_timer > 0.0;
            let x_margin = if wall_push_suppressed { margin } else { size_x_margin };
            let top_edge_margin = if wall_push_suppressed { top_margin } else { size_top_margin };
            let bottom_edge_margin = if wall_push_suppressed { 1.0 } else { size_bottom_margin };
            let effective_wall_push = if wall_push_suppressed { 0.0 } else { wall_push };
            // 壁際に群れ(同種の速度平均・上のschooling)が張り付いて滞留し続けるという
            // 実機フィードバックへの対応: 従来はmarginの内側に入った瞬間だけ反発
            // (wall_push)が立つ「硬い壁」だったため、それより手前(marginの外)では
            // 何の力もかからなかった。群れは一度壁際で減速する個体を含むと、他個体も
            // その平均速度に引き寄せられて壁に張り付いたまま集団で居座りやすい
            // (長時間シミュレートすると顕著になる。水流だけを強めても、群れの引力が
            // 勝ってしまい滞留率はほとんど下がらないことを実測で確認した)。margin の
            // 手前(wall_soft_band_mult倍の距離)から緩やかに反発を立ち上げ、壁に
            // 張り付く前に早めに向きを変えさせることで、群れが壁際で固まりにくくする。
            // margin以内(effective_wall_pushがそのまま乗る領域)は従来と完全に同じ
            // 強さ(t=1.0)になるので、壁際の捕食・角の詰め等の既存挙動は変えない。
            let wall_soft_band_mult = 3.5;
            let x_band = (x_margin * wall_soft_band_mult).max(x_margin);
            if fx < x_band {
                let t = ((x_band - fx) / (x_band - x_margin).max(0.001)).clamp(0.0, 1.0);
                ax += effective_wall_push * t;
            } else if fx > w - x_band {
                let t = ((fx - (w - x_band)) / (x_band - x_margin).max(0.001)).clamp(0.0, 1.0);
                ax -= effective_wall_push * t;
            }
            let top_band = (top_edge_margin * wall_soft_band_mult).max(top_edge_margin);
            let bottom_band = (bottom_edge_margin * wall_soft_band_mult).max(bottom_edge_margin);
            if fy < top_band {
                let t = ((top_band - fy) / (top_band - top_edge_margin).max(0.001)).clamp(0.0, 1.0);
                ay += effective_wall_push * t;
            } else if fy > sand_top - bottom_band {
                let t =
                    ((fy - (sand_top - bottom_band)) / (bottom_band - bottom_edge_margin).max(0.001))
                        .clamp(0.0, 1.0);
                ay -= effective_wall_push * t;
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
            let is_fleeing = is_knock_fleeing || is_fleeing_predator;

            // トントン(`T`キー)で興味を持っている間は、カーソル位置へ穏やかに加速する。
            // knockの逃走(flee)とは逆方向・かつ回り込み(ジグザグ)は付けず、素直に
            // 寄っていくだけの優しい動きにする。
            let is_attracted = attract_timer > 0.0;
            if is_attracted {
                ax += attract_dx * TAP_ATTRACT_STRENGTH;
                ay += attract_dy * TAP_ATTRACT_STRENGTH;
            }

            // ランダムな瞬発ダッシュ: ピラニア・餌などのトリガーが無い「通常時」だけ、
            // 低頻度・ランダムなタイミングで一瞬だけ速く動く演出を入れる(躍動感)。
            // 既に他の強い意図(餌を追う・追跡する・逃げる・寄ってくる)がある間は割り込まない。
            let normal_state = nearest_food.is_none()
                && chase_target.is_none()
                && !is_fleeing_predator
                && !is_knock_fleeing
                && !is_attracted;
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

            // 水流: 魚の現在位置における渦の力場を求める。self.fish[i]を可変借用する前に
            // 計算しておく(current_at()は共有借用が要るため、可変借用と重ねられない)。
            let (current_vx, current_vy) = self.current_at(fx, fy);

            let f = &mut self.fish[i];
            f.dash_timer = new_dash_timer;
            f.dash_dx = new_dash_dx;
            f.dash_dy = new_dash_dy;
            f.vx += ax * dt;
            f.vy += ay * dt;
            // 水流: 位置ごとの渦の力場を、他の遊泳力と同じく小さな加速度として毎tick加算する
            // (生きていて隠れていない魚だけ。死骸は独自の浮力/重力計算、隠れ中のタコは
            // 巣に固定なので、いずれもこの手前でcontinue済みでここには来ない)。魚だけは
            // 壁際に滞留しないようCURRENT_FISH_MULTで半分に弱め、遊泳意思で逆らえるようにする。
            f.vx += current_vx * CURRENT_FISH_MULT * dt;
            f.vy += current_vy * CURRENT_FISH_MULT * dt;
            // 慣性(ドラッグ)。逃走中・ダッシュ中・追跡中はドラッグ(ブレーキ)も弱めて
            // 反応を鈍らせない。追跡中も速さが体感できないという指摘への対応:
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
            if is_fleeing_predator {
                if f.flee_timer <= 0.0 {
                    f.hunger = (f.hunger - FLEE_HUNGER_COST).max(0.0);
                }
                f.flee_timer = f.flee_timer.max(PIRANHA_FEAR_FLEE_MARK);
            } else {
                f.flee_timer = (f.flee_timer - dt).max(0.0);
            }
            // トントンの引き寄せ状態を減らす
            f.attract_timer = (f.attract_timer - dt).max(0.0);
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
    fn update_food(&mut self, dt: f64, sand_top: f64, pix_w: usize) {
        // 積もる見た目のため、既に着地済みの位置をスナップショットしておく
        // (このtick中に新たに着地したものも互いに積み上がるよう別途記録する)
        let landed_snapshot: Vec<f64> = self.food.iter().filter(|fd| fd.landed).map(|fd| fd.x).collect();
        let mut new_landings: Vec<f64> = Vec::new();

        for i in 0..self.food.len() {
            if self.food[i].landed {
                continue; // 着地済みは停留(寿命は減らさない)
            }
            // 沈下中は蛇行に加えて渦の力場の水平成分でも横に流される(着地後は砂の上に
            // 停留するので効かせない)。self.food[i]を可変借用する前に力場を求める。
            let (cvx, _) = self.current_at(self.food[i].x, self.food[i].y);
            let fd = &mut self.food[i];
            fd.y += fd.vy * dt;
            // 螺旋階段のように左右へサラサラと蛇行しながら沈む(単純な直下降にしない)
            fd.sway_phase += SPRINKLE_SWAY_ANGULAR_SPEED * dt;
            fd.x += SPRINKLE_SWAY_AMPLITUDE * SPRINKLE_SWAY_ANGULAR_SPEED * fd.sway_phase.cos() * dt;
            fd.x += cvx * dt;
            fd.x = fd.x.clamp(1.0, safe_upper(pix_w as f64 - 1.0));
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
                f.hunger = (f.hunger + FEED_AMOUNT * f.feed_efficiency_mult).min(MAX_HUNGER);
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
    fn update_medicine(&mut self, dt: f64, sand_top: f64, pix_w: usize) {
        // 餌と同様、積もる見た目のため着地済みの位置をスナップショットしておく
        let landed_snapshot: Vec<f64> = self.medicine.iter().filter(|md| md.landed).map(|md| md.x).collect();
        let mut new_landings: Vec<f64> = Vec::new();

        for i in 0..self.medicine.len() {
            if self.medicine[i].landed {
                continue;
            }
            // 餌と同様、沈下中だけ渦の力場の水平成分で横に流される。
            let (cvx, _) = self.current_at(self.medicine[i].x, self.medicine[i].y);
            let md = &mut self.medicine[i];
            md.y += md.vy * dt;
            // 餌と同様、螺旋階段のように左右へ蛇行しながら沈む
            md.sway_phase += SPRINKLE_SWAY_ANGULAR_SPEED * dt;
            md.x += SPRINKLE_SWAY_AMPLITUDE * SPRINKLE_SWAY_ANGULAR_SPEED * md.sway_phase.cos() * dt;
            md.x += cvx * dt;
            md.x = md.x.clamp(1.0, safe_upper(pix_w as f64 - 1.0));
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

    // 浄化剤: 薬と同じ沈降+蛇行の動きで沈むが、水底に着いた瞬間に効果を発動して
    // 即座に消える(Food/Medicine/Meatのように水底へ停留しない)。効果=浄化剤の濃度を
    // 最大(1.0)に立て直す+着水演出(浄化ブルーム)を出す+効果音を鳴らす。
    fn update_purifiers(&mut self, dt: f64, sand_top: f64, pix_w: usize) {
        // 着水した浄化剤のx座標。借用の都合で、沈下処理のあとにまとめて効果を発動する。
        let mut landings: Vec<f64> = Vec::new();
        for i in 0..self.purifiers.len() {
            // 薬と同様、沈下中だけ渦の力場の水平成分で横に流される。
            let (cvx, _) = self.current_at(self.purifiers[i].x, self.purifiers[i].y);
            let p = &mut self.purifiers[i];
            p.y += p.vy * dt;
            // 薬と同様、螺旋階段のように左右へ蛇行しながら沈む
            p.sway_phase += SPRINKLE_SWAY_ANGULAR_SPEED * dt;
            p.x += SPRINKLE_SWAY_AMPLITUDE * SPRINKLE_SWAY_ANGULAR_SPEED * p.sway_phase.cos() * dt;
            p.x += cvx * dt;
            p.x = p.x.clamp(1.0, safe_upper(pix_w as f64 - 1.0));
            if p.y >= sand_top {
                landings.push(p.x);
            }
        }
        if !landings.is_empty() {
            // 着水したものは水底に停留させず即座に取り除く。
            self.purifiers.retain(|p| p.y < sand_top);
            for lx in landings {
                // 着水した瞬間はブルーム(拡散演出)を出すだけで、効果(濃度加算)は
                // まだ発動しない。拡散が完了したタイミングでupdate_effects側が
                // activatedを立てて初めて濃度に反映する(着水直後に効果が始まると
                // 見た目と体感がズレるとの指摘への対応)。
                self.purify_blooms.push(PurifyBloom {
                    x: lx,
                    y: sand_top, // 水底(着水位置)を中心に広がる
                    life: PURIFY_BLOOM_LIFETIME,
                    max_life: PURIFY_BLOOM_LIFETIME,
                    activated: false,
                });
                self.sound_events.push(SfxEvent::Purify);
            }
        }
    }

    // 浄化剤の濃度を毎tick薄める(PURIFIER_DILUTION_TIMEをかけて線形に0まで)。濃度が
    // 残っている間は、その濃度に比例した量だけ水質を直接押し下げる(急速な浄化)。
    fn update_purifier_concentration(&mut self, dt: f64) {
        if self.purifier_concentration > 0.0 {
            // そのtickの浄化量は薄める前の濃度で計算する(二重適用しない)。
            self.pollution =
                (self.pollution - PURIFIER_MAX_CLEAN_RATE * self.purifier_concentration * dt).max(0.0);
            self.purifier_concentration =
                (self.purifier_concentration - dt / PURIFIER_DILUTION_TIME).max(0.0);
        }
    }

    // ピラニア専用の肉餌: 沈降・着地(水底で停留)・捕食。餌・薬と同じ沈降+蛇行の
    // 動きだが、消費できるのはピラニアだけ(他の魚は一切近づかず消費しない)。
    // 通常の狩りhunger判定(piranha_still_hungry)と同じ基準で、満腹の間は
    // 肉餌とはいえ食いつかない。1tickで1匹の捕食者が消費できる肉餌は1つまでに
    // 制限する(魚・薬と同じ考え方)。
    fn update_meat(&mut self, dt: f64, sand_top: f64, pix_w: usize) {
        let landed_snapshot: Vec<f64> = self.meat.iter().filter(|mt| mt.landed).map(|mt| mt.x).collect();
        let mut new_landings: Vec<f64> = Vec::new();

        for i in 0..self.meat.len() {
            if self.meat[i].landed {
                continue;
            }
            // 餌・薬と同様、沈下中だけ渦の力場の水平成分で横に流される。
            let (cvx, _) = self.current_at(self.meat[i].x, self.meat[i].y);
            let mt = &mut self.meat[i];
            mt.y += mt.vy * dt;
            mt.sway_phase += SPRINKLE_SWAY_ANGULAR_SPEED * dt;
            mt.x += SPRINKLE_SWAY_AMPLITUDE * SPRINKLE_SWAY_ANGULAR_SPEED * mt.sway_phase.cos() * dt;
            mt.x += cvx * dt;
            mt.x = mt.x.clamp(1.0, safe_upper(pix_w as f64 - 1.0));
            mt.life -= dt;
            if mt.y >= sand_top {
                let nearby = landed_snapshot.iter().filter(|&&x| (x - mt.x).abs() < PILE_RADIUS).count()
                    + new_landings.iter().filter(|&&x| (x - mt.x).abs() < PILE_RADIUS).count();
                let rise = (nearby as f64 * PILE_STACK_STEP).min(PILE_MAX_HEIGHT);
                mt.y = sand_top - rise;
                mt.vy = 0.0;
                mt.landed = true;
                new_landings.push(mt.x);
            }
        }

        let mut eaten = vec![false; self.meat.len()];
        let mut bitten = false;
        for f in &mut self.fish {
            if f.species != Species::Piranha || f.dead {
                continue; // ピラニア以外は一切近づかず消費しない
            }
            if !piranha_still_hungry(f) {
                continue; // 満腹の間は肉餌とはいえ食いつかない
            }
            let mut best_dist = f64::INFINITY;
            let mut best_mi = None;
            // 大きく成長したピラニアが底面に堆積した肉餌を食べられないという指摘への対応:
            // 判定距離を生きた獲物と同じPIRANHA_STRIKE_RADIUS
            // (EAT_RADIUSより広い)にする。判定基準は中心と口(mouth_position)の
            // 近い方を使う: 遠くから接近する間は口が先に届くようにしたいが、壁際で
            // 体格(半径)より狭い隙間に獲物が挟まっている場合、正面を向いた口の位置は
            // 獲物を飛び越えてしまい逆に遠くなることがある(体は既に触れているのに、
            // 口だけが先へ突き抜けてしまう)。中心・口どちらか近い方を使うことで
            // どちらのケースでも確実に判定できるようにする。
            let (mouth_x, mouth_y) = f.mouth_position();
            for (mi, mt) in self.meat.iter().enumerate() {
                if eaten[mi] {
                    continue;
                }
                let center_d = ((mt.x - f.x).powi(2) + (mt.y - f.y).powi(2)).sqrt();
                let mouth_d = ((mt.x - mouth_x).powi(2) + (mt.y - mouth_y).powi(2)).sqrt();
                let d = center_d.min(mouth_d);
                if d < PIRANHA_STRIKE_RADIUS && d < best_dist {
                    best_dist = d;
                    best_mi = Some(mi);
                }
            }
            if let Some(mi) = best_mi {
                // 空腹の間だけ食いつき、満腹まで回復する(個体差で満たされ方は変わる)。
                // 旺盛な食欲ロジックのカウンタもリセットする(肉餌を与えて狩りを
                // 止められる救済ツールにする)。
                f.hunger = (f.hunger + MEAT_SATIATION_AMOUNT * f.feed_efficiency_mult).min(MAX_HUNGER);
                f.piranha_meals_since_full = 0;
                eaten[mi] = true;
                bitten = true;
            }
        }
        if bitten {
            self.set_message("ピラニアが肉餌を食べて満腹になった");
            self.sound_events.push(SfxEvent::Predation);
        }

        let mut idx = 0;
        self.meat.retain(|mt| {
            let keep = if eaten[idx] {
                false
            } else if mt.landed {
                true
            } else {
                mt.life > 0.0
            };
            idx += 1;
            keep
        });

        trim_landed(&mut self.meat, |mt| mt.landed, SEABED_ITEM_CAP);
    }

    // スター(無敵アイテム): 低頻度でランダムな位置に出現し、寿命が尽きると誰にも
    // 取られず消える。触れた魚(通常種・ピラニア・タコいずれでも)は一定時間無敵化する。
    // スターはネタ機能のため自然発生させず、`Z`キー(debug_spawn_star)経由でのみ
    // 出現する。ここでは寿命の消化・無敵タイマーの減衰・取得判定だけを行う。
    fn update_stars(&mut self, dt: f64) {
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
            // 取得できるのは捕食者でない通常種のみ(捕食者を倒す側の魚がスターを
            // 取るギミックのため、ピラニア・タコ自身は対象外)。
            if f.dead || f.species.is_predator() || (f.species == Species::Octopus && f.hidden) {
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
            self.sound_events.push(SfxEvent::StarPickup);
        }
    }

    // カメオ生物(ウミガメ・クラゲ・小魚の群れ): 低頻度で画面の端に出現し、
    // 反対側の端までゆっくり横切って消える。完全観賞用で、育成ロジック・捕食判定の
    // いずれにも参加しない(Fishとは完全に独立したエンティティ)。
    fn update_cameos(&mut self, dt: f64, w: f64, sand_top: f64) {
        // 既に1匹出ている間は追加抽選しない(同時に複数出す演出ではないため。Starと同じ考え方)
        if self.cameos.is_empty() && self.rng.next_f64() < CAMEO_SPAWN_CHANCE_PER_SEC * dt {
            let kind = match self.rng.range_usize(0, 2) {
                0 => CameoKind::Turtle,
                1 => CameoKind::Jellyfish,
                _ => CameoKind::FishSchool,
            };
            let from_left = self.rng.next_f64() < 0.5;
            let speed = self.rng.range(CAMEO_SPEED_MIN, CAMEO_SPEED_MAX);
            let (x, vx) = if from_left {
                (-CAMEO_DESPAWN_MARGIN + 1.0, speed)
            } else {
                (w + CAMEO_DESPAWN_MARGIN - 1.0, -speed)
            };
            let y = self.rng.range(4.0, (sand_top - 6.0).max(4.0));
            self.cameos.push(Cameo {
                kind,
                x,
                y,
                vx,
                vy: 0.0,
                phase: self.rng.range(0.0, std::f64::consts::TAU),
            });
        }

        for c in &mut self.cameos {
            c.x += c.vx * dt;
            // ゆったり縦にふらつくだけの、育成ロジックとは無関係な見た目の動き
            c.phase += CAMEO_BOB_FREQ * dt * std::f64::consts::TAU;
            c.vy = c.phase.sin() * CAMEO_BOB_AMPLITUDE * 0.2;
            c.y = (c.y + c.vy * dt).clamp(2.0, (sand_top - 4.0).max(2.0));
        }
        // 画面外(反対側)へ十分出たら消える
        self.cameos
            .retain(|c| c.x > -CAMEO_DESPAWN_MARGIN && c.x < w + CAMEO_DESPAWN_MARGIN);
    }

    // 育成: 空腹度減少・病気の発症/進行・成長・産卵・孵化・死亡(演出付き)
    fn update_biology(&mut self, dt: f64, cap: usize, w: f64, sand_top: f64) {
        // 過密判定・孵化の上限ゲートは、死んで浮いている個体(dead)を除いた
        // 「生きている」個体数を基準にする。居座る死骸が繁殖を止めないようにするため。
        let living = self.living_count();
        let overcrowded = living as f64 >= cap as f64 * OVERCROWD_RATIO;
        // 水質が悪いほど病気の発症確率を上げる(水質最悪でPOLLUTION_SICK_CHANCE_MAX_MULT倍)。
        let pollution_sick_mult =
            1.0 + (self.pollution / POLLUTION_MAX) * (POLLUTION_SICK_CHANCE_MAX_MULT - 1.0);
        // 水質がPOLLUTION_SICK_ELIGIBLE_FRAC以上悪化している間は、腹ぺこ・過密でない
        // 健康な個体も発症判定の対象に含める。倍率だけでは、そもそも発症判定に入る
        // 条件(腹ぺこ継続・過密)を満たさない限り水質が何であっても発症しないため、
        // 満腹を保っていれば水質が最悪でも病気にならないという抜け穴があった。
        let heavily_polluted = self.pollution >= POLLUTION_MAX * POLLUTION_SICK_ELIGIBLE_FRAC;
        // 水質が悪いほど、捕食者でない通常種の空腹度の減りを速める(食欲不振の表現。
        // 水質最悪でPOLLUTION_HUNGER_DECAY_MAX_MULT倍)。加えてupdate_movement側で
        // 餌を探して寄っていく誘引ベクトル自体も止めるため、餌を放置していると
        // 結果的に餓死しやすくなる。
        let pollution_hunger_decay_mult =
            1.0 + (self.pollution / POLLUTION_MAX) * (POLLUTION_HUNGER_DECAY_MAX_MULT - 1.0);
        // 浄化剤の濃度でも、水質とは別の発生源として通常種(捕食者以外)の空腹減衰を
        // 速める(食欲不振)。水質由来の倍率と掛け合わせる(置き換えない)。
        let purifier_hunger_decay_mult =
            1.0 + self.purifier_concentration * (PURIFIER_HUNGER_DECAY_MAX_MULT - 1.0);
        // 浄化剤の濃度は老化速度を速める(全種対象=捕食者も含む)。強力な浄化の代償。
        let purifier_age_mult =
            1.0 + self.purifier_concentration * (PURIFIER_LIFESPAN_MAX_MULT - 1.0);
        let mut messages: Vec<String> = Vec::new();
        let mut deaths: Vec<String> = Vec::new();
        // 産卵イベント: (親x, 親y, 種)。借用の都合で後からまとめて卵を生成する。
        let mut spawn_eggs: Vec<(f64, f64, Species, bool)> = Vec::new();

        for f in &mut self.fish {
            if f.dead {
                // 死亡演出中は育成ロジックの対象外。浮上している時間だけ進める。
                f.dead_timer += dt;
                continue;
            }

            // 年齢を進める(寿命・老齢判定に使う。死亡演出中でない間だけ加算する)。
            // 浄化剤の濃度に比例して老化を速める(全種対象=捕食者も含む)。
            f.age += dt * purifier_age_mult;

            // 空腹度の減少。捕食者以外は水質由来・浄化剤由来の食欲不振倍率を掛け合わせる
            // (捕食者はどちらの影響も受けず常に1.0)。
            let hunger_pollution_mult = if f.species.is_predator() {
                1.0
            } else {
                pollution_hunger_decay_mult * purifier_hunger_decay_mult
            };
            f.hunger =
                (f.hunger - HUNGER_DECAY * f.hunger_decay_mult * hunger_pollution_mult * dt).max(0.0);

            // 「食欲がなくても無限に追いかけまわす」バグの修正: 旺盛な食欲の
            // クォータ(meals_since_full)が進行中(1以上KILLS_TO_FULL未満)の間だけ
            // 時間を計測し、グレースピリオドを超えても次を捕食できなければ諦めて
            // クォータを放棄する(通常のhunger基準の狩りだけに戻る)。
            if f.species == Species::Piranha
                && f.piranha_meals_since_full > 0
                && f.piranha_meals_since_full < PIRANHA_KILLS_TO_FULL
            {
                f.piranha_quota_timer += dt;
                if f.piranha_quota_timer >= PIRANHA_QUOTA_GRACE_PERIOD {
                    f.piranha_meals_since_full = 0;
                    f.piranha_quota_timer = 0.0;
                }
            } else {
                f.piranha_quota_timer = 0.0;
            }

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
            // ただし一度でもつがいの交尾を経験した個体(has_mated)に限る。一度もつがいに
            // なれなかった個体はこの最後の産卵を行わない。
            if !f.elderly_spawned
                && f.age >= ELDERLY_AGE * f.lifespan_mult
                && f.species.breeds()
                && f.has_mated
            {
                f.elderly_spawned = true;
                spawn_eggs.push((f.x, f.y, f.species, false));
                messages.push(format!(
                    "{}が老齢に差し掛かり、最後の卵を産んだ",
                    species_name(f.species)
                ));
            }

            // ガラスの叩きすぎ(ストレス)の残り時間を進める
            f.stress_timer = (f.stress_timer - dt).max(0.0);

            // なつき度: クールダウンを進め、時間経過でゆっくり減衰させる
            f.affinity_cooldown = (f.affinity_cooldown - dt).max(0.0);
            f.affinity = (f.affinity - AFFINITY_DECAY_PER_SEC * dt).max(0.0);

            // ピラニアに噛まれた負傷は、しばらく追加で噛まれなければ時間経過で1段階ずつ癒える
            // (死亡演出中の個体はループ冒頭で除外済みなので、生きている個体だけが対象)。
            if f.piranha_bite_count > 0 {
                f.piranha_bite_recover_timer += dt;
                if f.piranha_bite_recover_timer >= PIRANHA_BITE_RECOVER_INTERVAL {
                    f.piranha_bite_count -= 1;
                    f.piranha_bite_recover_timer = 0.0;
                }

                // 負傷している間は、噛まれた瞬間の大きな血飛沫とは別に、治るまでずっと
                // 少量の血を滲ませ続ける(負傷が見た目にも継続して分かるようにする)。
                f.bleed_timer -= dt;
                if f.bleed_timer <= 0.0 {
                    f.bleed_timer = self.rng.range(BLEED_TRICKLE_INTERVAL_MIN, BLEED_TRICKLE_INTERVAL_MAX);
                    for _ in 0..BLEED_TRICKLE_PARTICLE_COUNT {
                        let px = f.x + self.rng.range(-BLOOD_SPREAD_RADIUS, BLOOD_SPREAD_RADIUS);
                        let py = f.y + self.rng.range(-BLOOD_SPREAD_RADIUS * 0.6, BLOOD_SPREAD_RADIUS * 0.6);
                        let particle_life = BLOOD_EFFECT_LIFETIME * self.rng.range(0.6, 1.0);
                        self.drop_effects.push(DropEffect {
                            x: px,
                            y: py,
                            life: particle_life,
                            max_life: particle_life,
                            kind: EffectKind::Blood,
                        });
                    }
                }
            }

            // タコがかじられた弱りは、しばらく追加でかじられなければ時間経過で1段階ずつ癒える
            // (ピラニアの被噛みつき回復と同じ仕組み。死亡演出中の個体はループ冒頭で除外済み)。
            if f.species == Species::Octopus && f.octopus_bite_count > 0 {
                f.octopus_bite_recover_timer += dt;
                if f.octopus_bite_recover_timer >= OCTOPUS_BITE_RECOVER_INTERVAL {
                    f.octopus_bite_count -= 1;
                    f.octopus_bite_recover_timer = 0.0;
                }
            }

            // 病気の発症: 腹ぺこ長期 or 過密で確率的に発症。ガラスを叩きすぎた直後は
            // ストレスにより発症確率が一時的に上がる。
            if !f.sick {
                let eligible = f.hungry_timer >= HUNGRY_SICK_TIME || overcrowded || heavily_polluted;
                let stress_mult = if f.stress_timer > 0.0 {
                    KNOCK_STRESS_DISEASE_MULT
                } else {
                    1.0
                };
                if eligible
                    && self.rng.next_f64() < DISEASE_CHANCE_PER_SEC * stress_mult * pollution_sick_mult * dt
                {
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
                // (全種共通。上限は個体差(growth_cap_variance)で±1段階ずれる)。
                let effective_growth_cap = (GENERAL_MAX_GROWTH_STAGE as i8 + f.growth_cap_variance)
                    .clamp(1, GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE as i8)
                    as u8;
                if f.stage == Stage::Adult
                    && f.growth_stage < effective_growth_cap
                    && f.size_timer >= SIZE_GROW_TIME
                {
                    f.growth_stage += 1;
                    f.size_timer = 0.0;
                    messages.push(format!("{}がさらに大きく育った", species_name(f.species)));
                }

                // 産卵(新規・つがい制): 個体ごとの独立したランダム産卵ではなく、
                // 同種で産卵可能な成魚2匹が近づいて「つがい」になり、十分接近したら
                // 交尾→産卵する(update_breeding_pairsでまとめて処理する。ピラニアは
                // 産卵しない=ピラニアを増やす唯一の方法はSキーにする方針のため)。
            }

            // 死亡判定: 猶予(STARVE_DEATH_TIME / SICK_DEATH_TIME)を超えたら死亡演出へ移行する。
            // 死亡演出(仰向けで浮上→静止→消滅)は update_movement / retain 側で処理する。
            // 老衰(LIFESPAN_DEATH_AGE)も同じ死亡演出に乗せる(全種共通・ピラニアも対象)。
            if f.starve_timer >= STARVE_DEATH_TIME || (f.sick && f.sick_timer >= SICK_DEATH_TIME) {
                f.dead = true;
                f.dead_timer = 0.0;
                deaths.push(format!("{}が力尽きた…", species_name(f.species)));
            } else if f.age >= LIFESPAN_DEATH_AGE * f.lifespan_mult {
                f.dead = true;
                f.dead_timer = 0.0;
                deaths.push(format!("{}が老衰で力尽きた…", species_name(f.species)));
            } else if f.species == Species::Octopus && self.purifier_concentration > 1.0 {
                // 浄化剤を連投して濃度が100%を超えると、劇薬が効きすぎてタコは
                // 血を吐いて死亡する(過剰投入への強いペナルティ)。
                f.dead = true;
                f.dead_timer = 0.0;
                for _ in 0..BLOOD_PARTICLE_COUNT {
                    let px = f.x + self.rng.range(-BLOOD_SPREAD_RADIUS, BLOOD_SPREAD_RADIUS);
                    let py = f.y + self.rng.range(-BLOOD_SPREAD_RADIUS * 0.6, BLOOD_SPREAD_RADIUS * 0.6);
                    let particle_life = BLOOD_EFFECT_LIFETIME * self.rng.range(0.6, 1.0);
                    self.drop_effects.push(DropEffect {
                        x: px,
                        y: py,
                        life: particle_life,
                        max_life: particle_life,
                        kind: EffectKind::Blood,
                    });
                }
                self.blood_stains.push(BloodStain {
                    x: f.x,
                    y: f.y,
                    life: BLOOD_STAIN_LIFETIME,
                    max_life: BLOOD_STAIN_LIFETIME,
                });
                self.sound_events.push(SfxEvent::Predation);
                deaths.push("タコが浄化剤の効きすぎで血を吐いて力尽きた…".to_string());
            } else if f.stage == Stage::Fry && self.purifier_concentration >= 0.5 {
                // 浄化剤の濃度が50%以上になると、稚魚はピラニアに噛まれた時と
                // 同規模の大量出血とともに直接死亡する。
                f.dead = true;
                f.dead_timer = 0.0;
                for _ in 0..BLOOD_PARTICLE_COUNT {
                    let px = f.x + self.rng.range(-BLOOD_SPREAD_RADIUS, BLOOD_SPREAD_RADIUS);
                    let py = f.y + self.rng.range(-BLOOD_SPREAD_RADIUS * 0.6, BLOOD_SPREAD_RADIUS * 0.6);
                    let particle_life = BLOOD_EFFECT_LIFETIME * self.rng.range(0.6, 1.0);
                    self.drop_effects.push(DropEffect {
                        x: px,
                        y: py,
                        life: particle_life,
                        max_life: particle_life,
                        kind: EffectKind::Blood,
                    });
                }
                self.blood_stains.push(BloodStain {
                    x: f.x,
                    y: f.y,
                    life: BLOOD_STAIN_LIFETIME,
                    max_life: BLOOD_STAIN_LIFETIME,
                });
                self.sound_events.push(SfxEvent::Predation);
                deaths.push(format!(
                    "{}の稚魚が浄化剤で大量出血して力尽きた…",
                    species_name(f.species)
                ));
            }
        }

        // つがいの交尾→産卵(新規)。個体ごとのループが終わった後にまとめて処理する
        // (2匹同時に更新するため、単体ループの中では扱えない)。
        self.update_breeding_pairs(dt, &mut spawn_eggs);

        // 産卵イベントを卵に変換(2〜4個、水底付近に配置)
        for (px, _py, sp, mated) in spawn_eggs {
            let msg = self.lay_egg_cluster(px, sp, sand_top, w, mated);
            messages.push(msg);
        }

        // 孵化: 時間経過した卵を稚魚にする。上限(生きている個体数基準)超過分は孵化しない(卵は消える)。
        let mut alive = self.living_count();
        let mut newborns: Vec<Fish> = Vec::new();
        let mut hatched_msg = false;
        let mut hatch_failed_msg = false;
        for e in &mut self.eggs {
            e.hatch -= dt;
        }
        self.eggs.retain(|e| {
            if e.hatch > 0.0 {
                return true; // まだ孵化しない
            }
            // 孵化タイミング。水質悪化・浄化剤の副作用として、確率で孵化に失敗して
            // そのまま消えることがある(上限超過による消滅とは別の判定)。
            let hatch_fail_chance = (self.pollution / POLLUTION_MAX
                * EGG_HATCH_FAIL_POLLUTION_MAX_CHANCE
                + self.purifier_concentration * EGG_HATCH_FAIL_PURIFIER_MULT)
                .min(1.0);
            if self.rng.next_f64() < hatch_fail_chance {
                hatch_failed_msg = true;
                return false;
            }
            if alive + newborns.len() < cap {
                newborns.push(roll_individuality_with_rng(
                    &mut self.rng,
                    Fish::new(e.species, Stage::Fry, e.x, e.y),
                ));
                // 羽化(孵化)の演出(新規)。産卵時のSpawnフラッシュとは別の、
                // 孵化した瞬間だけの短い演出にする。
                self.drop_effects.push(DropEffect {
                    x: e.x,
                    y: e.y,
                    life: HATCH_EFFECT_LIFETIME,
                    max_life: HATCH_EFFECT_LIFETIME,
                    kind: EffectKind::Hatch,
                });
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
        if hatch_failed_msg {
            messages.push("水質が悪く、卵が孵化できなかった…".to_string());
        }

        // タコが死んだら、その死んだタコの壺が自動的に消えるようにしてほしいという要望への
        // 対応: タコが死亡演出を終えて水槽から完全に消える瞬間、そのタコが使っていた
        // タコつぼも一緒に消す(空のタコつぼだけが取り残されて不自然にならないように)。
        let vanished_octopus_dens: Vec<(f64, f64)> = self
            .fish
            .iter()
            .filter(|f| f.species == Species::Octopus && f.dead && f.dead_timer >= CORPSE_REMOVE_TIME)
            .map(|f| (f.den_x, f.den_y))
            .collect();

        // 死亡演出(浮上→沈降)をCORPSE_REMOVE_TIMEだけ維持したら水槽から消す
        // (カニに片付けられた場合はupdate_crabs側で先に個別に消える)
        self.fish.retain(|f| !(f.dead && f.dead_timer >= CORPSE_REMOVE_TIME));

        if !vanished_octopus_dens.is_empty() {
            self.dens
                .retain(|d| !vanished_octopus_dens.iter().any(|&(dx, dy)| dx == d.x && dy == d.y));
        }

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
    // 藻・水草・岩に十分近いか(隠れているとみなす距離内か)を判定する。隠れたら実際に
    // 捕食されなくなるよう機能化してほしいという要望への対応: 従来は見た目だけの演出だったが、
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

    // 水質: 水底に堆積した食べ残し(餌・薬・肉餌)・病気の個体・死亡演出中の個体の
    // 放置で悪化し、常時かかる自然浄化(POLLUTION_NATURAL_DECAY)で改善する。
    // 悪化要因が無くなれば自然浄化により相対的にどんどん綺麗になっていく。
    fn update_pollution(&mut self, dt: f64) {
        let landed_food = self.food.iter().filter(|f| f.landed).count();
        let landed_medicine = self.medicine.iter().filter(|m| m.landed).count();
        let landed_meat = self.meat.iter().filter(|m| m.landed).count();
        // 病気・死亡個体は堆積物より一段強い悪化レートにする(急激に悪化させる)。
        let sick = self.fish.iter().filter(|f| f.sick && !f.dead).count();
        let dead = self.fish.iter().filter(|f| f.dead).count();

        let increase = landed_food as f64 * POLLUTION_PER_LANDED_FOOD
            + landed_medicine as f64 * POLLUTION_PER_LANDED_MEDICINE
            + landed_meat as f64 * POLLUTION_PER_LANDED_MEAT
            + sick as f64 * POLLUTION_PER_SICK_FISH
            + dead as f64 * POLLUTION_PER_DEAD_FISH;

        self.pollution =
            (self.pollution + (increase - POLLUTION_NATURAL_DECAY) * dt).clamp(0.0, POLLUTION_MAX);
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
        let snapshot: Vec<(
            Species,
            f64,
            f64,
            bool,
            bool,
            f64,
            u8,
            u8,
            bool,
            bool,
            f64,
            Stage,
            bool,
        )> = self
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
                    f.hunger, // 空腹はタコの捕食対象判定には使わなくなった(引数の並び維持のため渡すのみ)
                    f.stage,  // タコの捕食対象制限(稚魚のみ対象)に使う
                    f.sick,   // 病気もタコの捕食対象判定には使わなくなった(引数の並び維持のため渡すのみ)
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
                    if !piranha_still_hungry(f) && !is_temp_predator {
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
            // 当たり判定を胴体でなく口にすべきという指摘への対応: 捕食判定は
            // 中心(胴体)ではなく、進行方向側のスプライト前端=口の位置を基準にする。
            let (mouth_x, mouth_y) = f.mouth_position();
            let mut best_dist = f64::INFINITY;
            let mut best_j = None;
            for (
                j,
                &(
                    psp,
                    px,
                    py,
                    pdead,
                    phidden,
                    p_ink_escape,
                    p_growth,
                    p_kill,
                    p_invincible,
                    p_cover,
                    phunger,
                    pstage,
                    psick,
                ),
            ) in snapshot.iter().enumerate()
            {
                let p_hungry = phunger < HUNGRY_THRESHOLD;
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
                    pstage,
                    psick,
                    p_hungry,
                ) {
                    continue;
                }
                if p_ink_escape > 0.0 {
                    continue; // 墨を吐いた直後は捕食判定(strike radius)から一時的に除外
                }
                // 中心・口どちらか近い方を使う(大きく成長したピラニアが肉餌を食べられない
                // という指摘で判明した同種の問題への予防的対応:
                // 体格が大きいほど口が正面へ大きく突き出るため、壁際等で獲物が体の
                // 幅より狭い隙間に既に接触している場合、口基準の距離だけでは
                // 獲物を飛び越えて逆に遠いと判定してしまうことがある)。
                let mouth_d = ((px - mouth_x).powi(2) + (py - mouth_y).powi(2)).sqrt();
                let center_d = ((px - f.x).powi(2) + (py - f.y).powi(2)).sqrt();
                let d = mouth_d.min(center_d);
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
            // ピラニアの何発目の噛みつきかを控えておく(満腹回復量の出し分け・メッセージの
            // 出し分けの両方に使う)。1発目=負傷・2発目=瀕死・PIRANHA_BITES_TO_KILL発目=死亡。
            // 増分前の値を基準にするので、まだ噛まれていない獲物なら1になる。
            let piranha_bite_number = if predator_species == Species::Piranha {
                self.fish[pi].piranha_bite_count + 1
            } else {
                0
            };
            let piranha_kill_bite = piranha_bite_number >= PIRANHA_BITES_TO_KILL;
            let (gain, cooldown) = match predator_species {
                Species::Piranha => {
                    // 殺すまで至らなかった噛みつき(1発目・2発目)は、殺した(3発目)ときの
                    // 全回復量ではなく1/3だけ回復する(部分満腹化)。生かしたまま弱らせる
                    // だけの一噛みが、殺すのと同じ満腹効果を持つのは不自然という指摘への対応。
                    let g = if piranha_kill_bite {
                        PIRANHA_PREDATION_HUNGER_GAIN
                    } else {
                        PIRANHA_PREDATION_HUNGER_GAIN * PIRANHA_PARTIAL_BITE_HUNGER_FRAC
                    };
                    (g, PIRANHA_HUNT_COOLDOWN)
                }
                Species::Octopus => (OCTOPUS_PREDATION_HUNGER_GAIN, OCTOPUS_HUNT_COOLDOWN),
                // 無敵中の通常種による一時的な捕食(スターギミック)
                _ => (STAR_PREDATION_HUNGER_GAIN, STAR_PREDATION_COOLDOWN),
            };
            // 捕食者の空腹度を回復し、クールダウンを設定(先に捕食者側を更新してから
            // 獲物を除去する。除去でインデックスがずれても si には影響しないようにするため)
            self.fish[si].hunger =
                (self.fish[si].hunger + gain * self.fish[si].feed_efficiency_mult).min(MAX_HUNGER);
            self.fish[si].predation_cooldown = cooldown;
            // ピラニアは捕食するたびに段階的に大きくなる(上限 PIRANHA_MAX_KILL_STAGE で打ち止め。
            // タコはこの成長ボーナスの対象外)
            if predator_species == Species::Piranha && self.fish[si].kill_stage < PIRANHA_MAX_KILL_STAGE {
                self.fish[si].kill_stage += 1;
            }
            // 食欲を旺盛にする対応: 満腹になってからの捕食数を数え、hungerが満腹相当に
            // 達していてもPIRANHA_KILLS_TO_FULL匹に達するまでは狩りを継続させる。
            // ちょうどその匹数に達し、かつ実際にhungerも満腹相当になったタイミングで
            // 初めて「本当に満腹」と確定させ、次のサイクルのためにカウンタを0へ戻す。
            if predator_species == Species::Piranha {
                self.fish[si].piranha_meals_since_full += 1;
                if self.fish[si].piranha_meals_since_full >= PIRANHA_KILLS_TO_FULL
                    && self.fish[si].hunger >= PIRANHA_HUNT_HUNGER_THRESHOLD
                {
                    self.fish[si].piranha_meals_since_full = 0;
                }
            }
            if predator_species == Species::Piranha {
                // ピラニアの捕食は即消滅させず、まず段階的に弱らせる(1発では死なず、
                // PIRANHA_BITES_TO_KILL発目でようやく死亡演出に入る)。噛まれ続けている
                // 間は回復タイマーをリセットして、居座られると弱っていくようにする。
                // 死亡時は出血して力尽きた死骸として残し、浮上→沈降→カニの片付け、または
                // 24時間で自動消滅する通常の死骸パイプラインに乗せる(`t`キーでつついて
                // 沈降を早める既存の仕組みもそのまま効く)。獲物がタコだった場合のタコつぼの
                // 後始末は、死因を問わず死んだタコを処理する既存の汎用経路(update_biologyの
                // CORPSE_REMOVE_TIME経過時・update_crabsのカニ片付け時)に任せる。
                self.fish[pi].piranha_bite_recover_timer = 0.0;
                if piranha_kill_bite {
                    // 最後の一噛み: Xキー(debug_kill_random_fish)と同じ死亡状態にする。
                    self.fish[pi].dead = true;
                    self.fish[pi].dead_timer = 0.0;
                } else {
                    self.fish[pi].piranha_bite_count += 1;
                }
                // 血の匂い: 殺した瞬間・殺すまで至らなかった負傷時のいずれも、噛みついて
                // 出血させた位置に匂いのソースを残す。検知範囲内のピラニア(自分自身も
                // 含め、満腹中・クールダウン中を問わず)が、次回以降のupdate_movementで
                // これを優先的に追跡できるようにする(BLOOD_SCENT_LIFETIMEで自然に消える)。
                self.blood_scents.push(BloodScent {
                    x: prey_x,
                    y: prey_y,
                    life: BLOOD_SCENT_LIFETIME,
                });
            } else {
                // タコ・無敵の一時的捕食者による捕食は従来どおり即消滅させる。
                // タコが捕食されて消える場合、CORPSE_REMOVE_TIME経過やカニによる片付けを
                // 待たずにこの場で個体が消えるため、対応するタコつぼもここで一緒に
                // 片付ける(update_biology・update_crabs側の同種の後始末とは別経路なので、
                // ここでも必要)。
                let vanished_octopus_den = if prey_species == Species::Octopus {
                    Some((self.fish[pi].den_x, self.fish[pi].den_y))
                } else {
                    None
                };
                self.fish.remove(pi);
                if let Some((dx, dy)) = vanished_octopus_den {
                    self.dens.retain(|d| !(d.x == dx && d.y == dy));
                }
            }

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
            // 範囲エフェクト(内臓の損傷が周囲に広がっていくイメージ)。
            self.blood_stains.push(BloodStain {
                x: prey_x,
                y: prey_y,
                life: BLOOD_STAIN_LIFETIME,
                max_life: BLOOD_STAIN_LIFETIME,
            });
            self.sound_events.push(SfxEvent::Predation);
            // 捕食は血肉が水中に一気に飛び散るイベントのため、堆積物や病気個体の
            // じわじわした悪化とは別に、その場で水質を大きく悪化させる。タコ・無敵の
            // 一時的捕食者による捕食は獲物が即座に消えるためこのスパイクのみだが、
            // ピラニアの捕食は獲物が死骸として残るため、このスパイクに加えて
            // 死骸放置ぶんの悪化(POLLUTION_PER_DEAD_FISH)も別途乗る。
            self.pollution = (self.pollution + POLLUTION_PREDATION_SPIKE).min(POLLUTION_MAX);
            if predator_species == Species::Piranha {
                // 噛みつき段階に応じてメッセージを変える。最後の一噛み(死亡)は他の死因と
                // 同じ「力尽きた」の言い回しにそろえる(消えたのではなく出血して死ぬため)。
                let msg = if piranha_bite_number >= PIRANHA_BITES_TO_KILL {
                    format!("{}がピラニアに襲われ力尽きた…", species_name(prey_species))
                } else if piranha_bite_number == 1 {
                    format!("{}がピラニアにがぶりとやられた…", species_name(prey_species))
                } else {
                    format!("{}がピラニアに噛まれ瀕死になった…", species_name(prey_species))
                };
                self.set_message(msg);
            } else {
                self.set_message(format!("{}が食べられた…", species_name(prey_species)));
            }
        }
    }

    // 観賞用のカニ: 水底を左右に歩き、時々立ち止まる。育成ロジックには参加しない。
    fn update_crabs(&mut self, dt: f64, w: f64, sand_top: f64) {
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

        // カニの掃除役: 水底に着地した餌・薬・肉餌に近づくと食べて片付ける
        // (カニ自身の空腹度等のロジックは追加しない。単に消費するだけ)。肉餌は
        // ピラニアが食べ残すと放置されがちなので、餌・薬と同じ掃除対象に含めた。
        // 山になって盛り上がっている分の高さは無視し、X距離だけで判定する
        // (積もった山の頂上まで判定距離が届かなくなるのを避けるため)。
        // 1匹のカニが1tickで消費できる餌・薬・肉餌はそれぞれ1つまで(範囲内に複数
        // あっても最も近い1つだけ食べ、残りは次のtick以降に持ち越す)。魚側の
        // 「1tickで1粒まで」(fish_eats_only_one_food_per_tick_even_when_surrounded 等)
        // と同じ考え方で、山が一括で消えてしまう過剰消費バグを防ぐ。
        let mut food_eaten = vec![false; self.food.len()];
        let mut med_eaten = vec![false; self.medicine.len()];
        let mut meat_eaten = vec![false; self.meat.len()];
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

            let mut best_dist_mt = f64::INFINITY;
            let mut best_mti = None;
            for (mti, mt) in self.meat.iter().enumerate() {
                if meat_eaten[mti] || !mt.landed {
                    continue;
                }
                let d = (mt.x - c.x).abs();
                if d < CRAB_EAT_RADIUS && d < best_dist_mt {
                    best_dist_mt = d;
                    best_mti = Some(mti);
                }
            }
            if let Some(mti) = best_mti {
                meat_eaten[mti] = true;
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
        let mut mti = 0;
        self.meat.retain(|_| {
            let keep = !meat_eaten[mti];
            mti += 1;
            keep
        });

        // カニの掃除役(拡張): 沈んで水底に落ち着いた亡骸(浮力が失われて沈降を終え、
        // 水底で静止しているもの)にも接触すると片付ける。片付けた瞬間、分解の
        // 演出(EffectKind::Decompose)をその位置に出してから個体を消す
        // (CORPSE_REMOVE_TIME経過を待たずに片付けられる)。餌・薬と同様、X距離
        // だけで判定し、1匹のカニが1tickで片付けられる亡骸は1体まで。
        let bottom_y = safe_upper(sand_top - 1.0);
        let mut corpse_eaten = vec![false; self.fish.len()];
        for c in &self.crabs {
            let mut best_dist = f64::INFINITY;
            let mut best_fi = None;
            for (fi, f) in self.fish.iter().enumerate() {
                if corpse_eaten[fi] || !f.dead || (bottom_y - f.y).abs() > 0.5 {
                    continue; // まだ浮いている/沈降中の亡骸は対象外(水底に落ち着くまで待つ)
                }
                let d = (f.x - c.x).abs();
                if d < CRAB_EAT_RADIUS && d < best_dist {
                    best_dist = d;
                    best_fi = Some(fi);
                }
            }
            if let Some(fi) = best_fi {
                corpse_eaten[fi] = true;
            }
        }
        let mut vanished_octopus_dens: Vec<(f64, f64)> = Vec::new();
        for (fi, f) in self.fish.iter().enumerate() {
            if corpse_eaten[fi] {
                self.drop_effects.push(DropEffect {
                    x: f.x,
                    y: f.y,
                    life: CORPSE_DECOMPOSE_EFFECT_LIFETIME,
                    max_life: CORPSE_DECOMPOSE_EFFECT_LIFETIME,
                    kind: EffectKind::Decompose,
                });
                if f.species == Species::Octopus {
                    // タコの死骸が片付けられた場合、CORPSE_REMOVE_TIMEを待たずに
                    // 個体が消えるため、対応するタコつぼもここで一緒に片付ける
                    // (update_biology側の同種の後始末とは別経路なので、ここでも必要)。
                    vanished_octopus_dens.push((f.den_x, f.den_y));
                }
            }
        }
        let mut ci = 0;
        self.fish.retain(|_| {
            let keep = !corpse_eaten[ci];
            ci += 1;
            keep
        });
        if !vanished_octopus_dens.is_empty() {
            self.dens
                .retain(|d| !vanished_octopus_dens.iter().any(|&(dx, dy)| dx == d.x && dy == d.y));
        }
    }

    // 観賞用のエビ: カニと同じ挙動(水底を左右に歩き、時々立ち止まる)。育成ロジック・
    // 捕食判定のいずれにも参加しない。カニの「掃除役」は再現しない(要望に無いため)。
    fn update_shrimp(&mut self, dt: f64, w: f64) {
        let margin = 3.0;
        for s in &mut self.shrimp {
            if s.pause_timer > 0.0 {
                s.pause_timer -= dt;
                continue;
            }
            s.x += s.dir * SHRIMP_SPEED * dt;
            if s.x < margin {
                s.x = margin;
                s.dir = 1.0;
            } else if s.x > w - margin {
                s.x = (w - margin).max(margin);
                s.dir = -1.0;
            }
            s.facing_right = s.dir > 0.0;
            if self.rng.next_f64() < SHRIMP_PAUSE_CHANCE_PER_SEC * dt {
                s.pause_timer = self.rng.range(1.0, 3.0);
            }
        }
    }

    // 観賞用のタツノオトシゴ: 藻に絡みつくようにゆっくり動き、あまり大きく移動しない
    // (基準位置=藻の近くから、ゆらゆらとした小さな振れ幅でしか離れない)。
    // 育成ロジック・捕食判定のいずれにも参加しない。
    fn update_seahorses(&mut self, dt: f64) {
        for s in &mut self.seahorses {
            s.phase += SEAHORSE_DRIFT_FREQ * dt * std::f64::consts::TAU;
            s.x = s.anchor_x + s.phase.sin() * SEAHORSE_DRIFT_AMPLITUDE;
            s.y = s.anchor_y + (s.phase * 0.7).cos() * SEAHORSE_DRIFT_AMPLITUDE * 0.5;
        }
    }

    // 渦の中心座標を経過時間と水槽サイズから決める。X/Yで別周期のサインカーブに
    // 沿って動かすことでリサージュ曲線状にゆっくり水槽内を周遊し、単純な往復には
    // ならない。中心が縁に寄りすぎないようCURRENT_CENTER_MARGIN_FRAC分の余白を残す。
    // dtは不要(elapsedと水槽サイズだけで決まる純粋な派生値)。
    fn update_current(&mut self, w: f64, h: f64) {
        let amp_x = (w / 2.0) * (1.0 - CURRENT_CENTER_MARGIN_FRAC);
        let amp_y = (h / 2.0) * (1.0 - CURRENT_CENTER_MARGIN_FRAC);
        let phase_x = self.elapsed * (std::f64::consts::TAU / CURRENT_CENTER_DRIFT_PERIOD_X);
        let phase_y = self.elapsed * (std::f64::consts::TAU / CURRENT_CENTER_DRIFT_PERIOD_Y)
            + std::f64::consts::FRAC_PI_2;
        self.current_center_x = w / 2.0 + amp_x * phase_x.sin();
        self.current_center_y = h / 2.0 + amp_y * phase_y.sin();
    }

    // 指定位置における渦の力場ベクトルを返す。中心からの相対位置に垂直な(接線方向の)
    // 一定の大きさCURRENT_STRENGTHの押しを与えるため、水槽の場所ごとに向きが変わる回転流に
    // なる。中心とちょうど同じ位置ではdx=dy=0となり、返り値は正確に(0.0, 0.0)になる
    // (=水流ゼロ。テストの無風基準にこの性質を使う)。
    pub fn current_at(&self, x: f64, y: f64) -> (f64, f64) {
        let dx = x - self.current_center_x;
        let dy = y - self.current_center_y;
        let dist = (dx * dx + dy * dy).sqrt().max(1.0);
        let tx = -dy / dist;
        let ty = dx / dist;
        // トルネードの目のように、中心からの距離に応じて指数関数的に減衰させる。
        // 中心から離れた場所(水槽の大部分)ではほぼ0になり、魚が水流に妨げられず
        // 自由に泳ぎ回れるようにする。
        let falloff = (-dist / CURRENT_FALLOFF_RADIUS).exp();
        let strength = CURRENT_STRENGTH * falloff;
        (tx * strength, ty * strength)
    }

    // 水流を可視化する筋(CurrentStreak)を生成・移動・消去する。水槽全体のランダムな
    // 位置に一定間隔で湧き、その場所の渦の力場に流されて曲線を描きながらフェードして
    // 消える。渦の力場は場所によらず一定の大きさを持つため、筋が止まって見えることはない。
    fn update_current_streaks(&mut self, dt: f64, w: f64, h: f64) {
        self.current_streak_timer -= dt;
        if self.current_streak_timer <= 0.0 {
            self.current_streak_timer = self
                .current_streak_rng
                .range(CURRENT_STREAK_SPAWN_INTERVAL_MIN, CURRENT_STREAK_SPAWN_INTERVAL_MAX);
            // 気泡と同様、水槽全体のランダムな位置から生成する。
            self.current_streaks.push(CurrentStreak {
                x: self.current_streak_rng.range(2.0, (w - 2.0).max(2.0)),
                y: self.current_streak_rng.range(4.0, (h - 4.0).max(4.0)),
                life: CURRENT_STREAK_LIFETIME,
                max_life: CURRENT_STREAK_LIFETIME,
            });
        }
        // 各筋を、その場所の渦の力場ベクトルで流す(複数フレームにわたって渦の中心の
        // まわりを回る曲線軌道になる)。self.current_at()は共有借用が要るため、可変
        // 借用と重ならないようインデックスでアクセスする。
        for i in 0..self.current_streaks.len() {
            let (cvx, cvy) = self.current_at(self.current_streaks[i].x, self.current_streaks[i].y);
            let s = &mut self.current_streaks[i];
            s.x += cvx * dt;
            s.y += cvy * dt;
            s.life -= dt;
        }
        // 寿命が尽きたもの・水槽のいずれかの縁から流れ去ったものを取り除く(溜め込まない)。
        // 筋は縦にも流れるため、上下の境界も判定する。
        self.current_streaks
            .retain(|s| s.life > 0.0 && s.x >= -4.0 && s.x <= w + 4.0 && s.y >= -4.0 && s.y <= h + 4.0);
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
        for i in 0..self.bubbles.len() {
            // ランダムな横揺れに、その場所の渦の力場の水平成分を重ねる。current_at()と
            // self.rngはいずれもselfの借用が要るため、可変借用と重ならない順で取り出す。
            let (cvx, _) = self.current_at(self.bubbles[i].x, self.bubbles[i].y);
            let jitter = self.rng.signed() * 4.0;
            let b = &mut self.bubbles[i];
            b.y += b.vy * dt;
            b.x += (jitter + cvx) * dt;
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
    candidate_stage: Stage,
    // タコの捕食対象がFry限定になったため、成魚の病気・空腹はもう判定に使わない。
    // 呼び出し側の引数の並びは変えずに、未使用であることを明示する。
    _candidate_sick: bool,
    _candidate_hungry: bool,
) -> bool {
    if self_index == candidate_index || candidate_dead {
        return true;
    }
    if candidate_species == Species::Whale {
        return true; // クジラは(ネタ巨大魚のため)誰からも捕食対象にならない
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
        // 無敵中の一時的捕食者(本来は捕食されない側の魚が逆転して捕食者になる
        // ギミック)は、捕食者(ピラニア・タコ)だけを対象にする。通常の魚同士を
        // 襲うことはない(倒せるのは捕食者のみという方針のため)。
        return !candidate_species.is_predator();
    }
    if predator_species == Species::Octopus && candidate_species == Species::Piranha {
        return true; // タコはピラニアを襲わない
    }
    if predator_species == Species::Octopus && candidate_stage == Stage::Adult {
        return true; // タコは稚魚のみを捕食対象にする(成魚は健康・病気・空腹を問わず対象外)
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
        Species::Whale => "クジラ",
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
            sway_phase: 0.0,
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
        // 当たり判定を胴体でなく口にすべきという指摘への対応: 口(進行方向側の
        // スプライト前端)が獲物側へ張り出す分、中心間の距離が近いと初動でほぼ即座に
        // 届いてしまう。実際に「追いかけて近づく」動きを検証できるよう、口の張り出し分
        // (スプライト半幅+strike radius)を超える距離に獲物を置く。
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 38.0, 20.0)); // 獲物(距離28)
        let start_x = sim.fish[0].x;
        for _ in 0..24 {
            sim.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食条件を維持
            sim.update(0.05, 80, 40);
            if sim.fish[1].dead {
                break; // 追いついて捕食し獲物が死骸になったら十分な証拠なので終了
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
        // 壁際に追い詰めた魚を永遠に捕食できないという問題の再発防止テスト。
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
            sim.fish[1].predation_cooldown = 0.0; // 1発では死なないため連続で噛ませる
            sim.update(0.1, w, h);
            // ピラニアの噛みつきは即消滅させず段階的に弱らせるため、「捕食が成立した」=
            // 追い詰めた魚(index 0)が死亡状態になった、で判定する。
            if sim.fish[0].dead {
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
            // 捕食は即消滅させず死骸を残すため、捕まったかどうかは獲物の死亡フラグで見る。
            if sim2.fish[1].dead {
                break;
            }
        }
        // 逃げ切れた(まだ生きている)場合のみ、距離が広がったことを検証する。
        if !sim2.fish[1].dead {
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
    fn regular_fish_flees_from_a_nearby_emerged_octopus() {
        // バグ報告(「青い魚(ネオン)がタコに向かって突進して捕食された」)対応:
        // ピラニアと同様、通常の魚は近くに出ているタコがいれば警戒して逃げる方向へ
        // 加速するはず(タコへ向かって突進しない)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(740));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false; // 出ている(見えている)タコ
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        octo.hunger = MAX_HUNGER; // 満腹(空腹時の狩り行動と混同しないようにする)
        sim.fish.push(octo);
        // タコの右側、PIRANHA_FEAR_RADIUS(26.0)より近い位置に置く
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0));

        sim.update(0.1, w, h);

        assert!(
            sim.fish[1].vx > 0.0,
            "タコが左側にいるので、通常の魚はタコと反対方向(右)へ逃げる速度になるはず: vx={}",
            sim.fish[1].vx
        );
    }

    #[test]
    fn regular_fish_does_not_flee_from_a_hidden_octopus() {
        // タコつぼに隠れている間は見えていないので、警戒対象にならないはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(741));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = true; // 隠れているタコ
        octo.hidden_timer = 999.0; // このtick中に出てこない(=捕食もしない)ようにする
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        sim.fish.push(octo);
        let mut neon = Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0);
        neon.vx = 0.0;
        sim.fish.push(neon);

        sim.update(0.1, w, h);

        // 隠れたタコを警戒して逃げているなら vx が明確に正(右方向)になるはずだが、
        // 隠れている間は対象外なので、ランダムウォークの範囲内(小さい値)に留まるはず。
        assert!(
            sim.fish[1].vx.abs() < 20.0,
            "隠れているタコは警戒対象にならないはず(逃走由来の強い加速が乗らない): vx={}",
            sim.fish[1].vx
        );
    }

    #[test]
    fn octopus_does_not_flee_from_another_octopus() {
        // タコ自身は同種(他のタコ)を警戒しない。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(742));
        let mut octo1 = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo1.hidden = false;
        octo1.hidden_timer = 999.0;
        octo1.den_x = 40.0;
        octo1.den_y = 20.0;
        sim.fish.push(octo1);
        let mut octo2 = Fish::new(Species::Octopus, Stage::Adult, 45.0, 20.0);
        octo2.hidden = false;
        octo2.hidden_timer = 999.0;
        octo2.den_x = 45.0;
        octo2.den_y = 20.0;
        octo2.vx = 0.0;
        sim.fish.push(octo2);

        sim.update(0.1, w, h);

        assert!(
            sim.fish[1].vx.abs() < 20.0,
            "タコは他のタコを警戒して逃げないはず(強い加速が乗らない): vx={}",
            sim.fish[1].vx
        );
    }

    #[test]
    fn living_fish_moves_away_from_a_nearby_corpse() {
        // 死骸(dead=true)に近づきすぎた生きている魚は、本能的に距離を置く方向へ
        // 弱く加速するはず。ただし通常のランダムウォーク(wander)の方が忌避力より
        // 大きく、1tickだけでは符号すら安定しないため、同一seedで「死骸あり」
        // 「死骸なし」を比較する(このファイルの他の吸引・忌避テストと同じ手法)。
        // 死骸(dead=true)はupdate_movement内で乱数を消費しないため、同一seedなら
        // 生きている魚が消費する乱数列は両者で完全に一致し、差分は忌避力のみになる。
        let (w, h) = (80, 40);

        let mut with_corpse = Simulation::new(Rng::new(950));
        let mut corpse = Fish::new(Species::Goldfish, Stage::Adult, 40.0, 20.0);
        corpse.dead = true;
        corpse.dead_timer = 9999.0; // 沈み切って静止済みにしておく(浮遊中の揺れと混同しないため)
        corpse.sink_forced = true;
        with_corpse.fish.push(corpse);
        let mut neon = Fish::new(Species::Neon, Stage::Adult, 46.0, 20.0);
        neon.vx = 0.0;
        with_corpse.fish.push(neon);

        let mut without_corpse = Simulation::new(Rng::new(950));
        let mut neon2 = Fish::new(Species::Neon, Stage::Adult, 46.0, 20.0);
        neon2.vx = 0.0;
        without_corpse.fish.push(neon2);

        for _ in 0..30 {
            with_corpse.update(0.1, w, h);
            without_corpse.update(0.1, w, h);
        }

        assert!(
            with_corpse.fish[1].x > without_corpse.fish[0].x,
            "同一乱数列でも、死骸(左側)がある方がより右(反対方向)へ進んでいるはず: with={} without={}",
            with_corpse.fish[1].x,
            without_corpse.fish[0].x
        );
    }

    #[test]
    fn living_fish_is_not_pushed_by_a_distant_corpse() {
        // CORPSE_AVOID_RADIUSより離れた死骸には反応しないはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(901));
        let mut corpse = Fish::new(Species::Goldfish, Stage::Adult, 5.0, 20.0);
        corpse.dead = true;
        corpse.dead_timer = 9999.0;
        corpse.sink_forced = true;
        sim.fish.push(corpse);
        let mut neon = Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0); // CORPSE_AVOID_RADIUS(12.0)よりずっと遠い
        neon.vx = 0.0;
        sim.fish.push(neon);

        sim.update(0.1, w, h);

        assert!(
            sim.fish[1].vx.abs() < 20.0,
            "離れた死骸には反応せず、ランダムウォーク程度の小さな動きに留まるはず: vx={}",
            sim.fish[1].vx
        );
    }

    #[test]
    fn piranha_does_not_flee_from_an_emerged_octopus() {
        // ピラニアは何も怖がらない既存方針を維持する(タコからも逃げない)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(743));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        octo.hunger = MAX_HUNGER;
        sim.fish.push(octo);
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 45.0, 20.0);
        piranha.hunger = MAX_HUNGER; // 狩りモードにはならない状態にしておく
        piranha.vx = 0.0;
        sim.fish.push(piranha);

        sim.update(0.1, w, h);

        assert!(
            sim.fish[1].vx.abs() < 20.0,
            "ピラニアはタコからも逃げないはず(強い加速が乗らない): vx={}",
            sim.fish[1].vx
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
    fn prey_detected_at_the_edge_of_fear_radius_still_accelerates_away_strongly() {
        // 再発防止: 検知範囲(PIRANHA_FEAR_RADIUS)の縁ではdist≈radiusとなり、以前は
        // 逃走の強さが距離に比例するだけの実装だったため、そこではほぼ0になって
        // しまい、通常の遊泳(ランダムウォーク・群れ)の方が実質的に勝って危険域へ
        // フラフラ近づけてしまうことがあった(「ピラニアの近くをフラフラ泳いで危険域に
        // 入ってしまう」という報告)。検知範囲の縁ぎりぎりでも、はっきり離れる方向へ
        // 加速することを直接確認する。
        let mut sim = Simulation::new(Rng::new(2001));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = MAX_HUNGER; // 追跡はさせず、常時逃走(fear)の対象になることだけを見る
        sim.fish.push(piranha);
        // 検知範囲のすぐ内側(縁ぎりぎり)に獲物を置く。
        let edge_dist = PIRANHA_FEAR_RADIUS - 1.0;
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.0 + edge_dist, 20.0));
        sim.update(0.05, 80, 40);
        assert!(
            sim.fish[1].vx > 5.0,
            "検知範囲の縁でも1tickではっきり逃走方向(+x)へ加速するはず: vx={}",
            sim.fish[1].vx
        );
    }

    #[test]
    fn fleeing_prey_never_gets_meaningfully_closer_to_a_detected_piranha() {
        // 結果ベースの再発防止テスト: ピラニアを固定し、検知範囲の縁ぎりぎりに置いた
        // 獲物が、複数シードのいずれでも危険域(捕食される距離)へ近づくことなく、
        // 時間経過で確実に検知範囲の外まで離れられることを確認する(「検知したら
        // 確実に距離を取る」の保証)。ピラニア側は満腹にして追跡させず、
        // fear(常時逃走)だけを見る。
        // 安全圏の基準はPIRANHA_STRIKE_RADIUSに十分な余裕を掛けたものにする
        // (水流など他の弱い背景の力による数px程度の揺らぎまで不合格にすると、
        // 本題(フラフラ近づいて危険域に入るかどうか)とは無関係な過検出になるため)。
        let danger_zone = PIRANHA_STRIKE_RADIUS * 2.0;
        for seed in [10u64, 20, 30, 40, 50] {
            let mut sim = Simulation::new(Rng::new(seed));
            let piranha_x = 40.0;
            let piranha_y = 20.0;
            let mut piranha = Fish::new(Species::Piranha, Stage::Adult, piranha_x, piranha_y);
            piranha.hunger = MAX_HUNGER;
            sim.fish.push(piranha);
            let edge_dist = PIRANHA_FEAR_RADIUS - 1.0;
            sim.fish
                .push(Fish::new(Species::Neon, Stage::Adult, piranha_x + edge_dist, piranha_y));

            let mut min_dist = edge_dist;
            for _ in 0..100 {
                // ピラニア自身は動かず固定する(このテストは被食者の逃走判定だけを見る)。
                sim.fish[0].x = piranha_x;
                sim.fish[0].y = piranha_y;
                sim.fish[0].vx = 0.0;
                sim.fish[0].vy = 0.0;
                sim.update(0.05, 80, 40);
                let d = ((sim.fish[1].x - piranha_x).powi(2) + (sim.fish[1].y - piranha_y).powi(2)).sqrt();
                min_dist = min_dist.min(d);
            }
            assert!(
                min_dist > danger_zone,
                "seed={seed}: 検知範囲の縁からは危険域(PIRANHA_STRIKE_RADIUSの余裕を持った圏内)へ近づかないはず: min_dist={min_dist} danger_zone={danger_zone}"
            );
            let final_dist =
                ((sim.fish[1].x - piranha_x).powi(2) + (sim.fish[1].y - piranha_y).powi(2)).sqrt();
            assert!(
                final_dist > PIRANHA_FEAR_RADIUS,
                "seed={seed}: 十分な時間が経てば検知範囲の外まで逃げ切れるはず: final_dist={final_dist}"
            );
        }
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
                sway_phase: 0.0,
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
                sway_phase: 0.0,
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
            sway_phase: 0.0,
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
    fn falling_food_sways_left_and_right_instead_of_dropping_straight_down() {
        // 「螺旋階段のように横にサラサラと移動しながらふりかけてほしい」対応の回帰テスト。
        // 沈降中の餌は単純な直下降ではなく、左右に蛇行しながら落ちることを確認する。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(7));
        sim.food.push(Food {
            x: 40.0,
            y: 5.0,
            vy: FOOD_SINK_SPEED,
            life: FOOD_LIFETIME,
            landed: false,
            sway_phase: 0.0, // 位相0スタート = cos(0)=1 で確実に初手から揺れが乗る
        });
        let start_x = sim.food[0].x;
        sim.update(0.2, w, h);
        assert!(
            !sim.food.is_empty() && !sim.food[0].landed,
            "この時間では着地しないはず(テスト前提の確認)"
        );
        assert_ne!(
            sim.food[0].x,
            start_x,
            "沈降中の餌は左右に揺れてxが変化するはず(直下降ではない)"
        );
    }

    #[test]
    fn falling_medicine_sways_left_and_right_instead_of_dropping_straight_down() {
        // 薬も餌と同様に蛇行しながら沈むことを確認する。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(7));
        sim.medicine.push(Medicine {
            x: 40.0,
            y: 5.0,
            vy: MED_SINK_SPEED,
            life: MED_LIFETIME,
            landed: false,
            sway_phase: 0.0,
        });
        let start_x = sim.medicine[0].x;
        sim.update(0.2, w, h);
        assert!(
            !sim.medicine.is_empty() && !sim.medicine[0].landed,
            "この時間では着地しないはず(テスト前提の確認)"
        );
        assert_ne!(
            sim.medicine[0].x, start_x,
            "沈降中の薬は左右に揺れてxが変化するはず(直下降ではない)"
        );
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
            sway_phase: 0.0,
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
            sway_phase: 0.0,
        });
        sim.update(0.1, w, h);
        let second = &sim.food[1]; // 揺れでxがわずかに動くため、着地順(push順)で参照する
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
            sway_phase: 0.0,
        });
        sim.update(0.1, w, h);
        let far = &sim.food[2]; // 揺れでxがわずかに動くため、着地順(push順)で参照する
        assert!(
            (far.y - sand_top).abs() < 0.01,
            "離れた場所の1個目は盛り上がらないはず: {} vs {}",
            far.y,
            sand_top
        );
    }

    #[test]
    fn heavily_fed_pile_reaches_visually_significant_height() {
        // 山として認識できるレベルまで最大高さを上げてほしいという要望への対応を確認するテスト。
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
                sway_phase: 0.0,
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
            sway_phase: 0.0,
        });
        sim.medicine.push(Medicine {
            x: 20.0,
            y: sand_top,
            vy: 0.0,
            life: 999.0,
            landed: true,
            sway_phase: 0.0,
        });
        sim.meat.push(Meat {
            x: 20.0,
            y: sand_top,
            vy: 0.0,
            life: 999.0,
            landed: true,
            sway_phase: 0.0,
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
        assert_eq!(sim.meat.len(), 0, "カニが水底の肉餌も片付けるはず");
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
                sway_phase: 0.0,
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
                sway_phase: 0.0,
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
    fn hunger_decay_mult_gives_individual_differences_in_appetite_speed() {
        // 個体差の回帰テスト: 「各魚、タコ、ピラニアみんな腹の減り方は個体差がある」。
        // hunger_decay_multが大きい個体ほど、同じ時間でより早く空腹になるはず。
        let mut sim = Simulation::new(Rng::new(1));
        let mut slow = Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0);
        slow.hunger_decay_mult = 0.5;
        let mut fast = Fish::new(Species::Neon, Stage::Adult, 30.0, 10.0);
        fast.hunger_decay_mult = 2.0;
        sim.fish.push(slow);
        sim.fish.push(fast);

        run(&mut sim, 5.0, 0.1, 80, 40, false);

        assert!(
            sim.fish[0].hunger > sim.fish[1].hunger,
            "hunger_decay_multが小さい個体の方が空腹度が高く残るはず: {} vs {}",
            sim.fish[0].hunger,
            sim.fish[1].hunger
        );
    }

    #[test]
    fn feed_efficiency_mult_gives_individual_differences_in_satiation() {
        // 個体差の回帰テスト: 「食ったときの腹の満たされ方は個体差がある」。
        // feed_efficiency_multが大きい個体ほど、同じ1粒の餌でより大きく回復するはず。
        let mut sim = Simulation::new(Rng::new(2));
        let mut modest = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        modest.hunger = 10.0;
        modest.feed_efficiency_mult = 0.6;
        let mut gourmand = Fish::new(Species::Goldfish, Stage::Adult, 60.0, 20.0);
        gourmand.hunger = 10.0;
        gourmand.feed_efficiency_mult = 1.5;
        sim.fish.push(modest);
        sim.fish.push(gourmand);
        sim.food.push(Food {
            x: 40.0,
            y: 20.0,
            vy: 0.0,
            life: 30.0,
            landed: false,
            sway_phase: 0.0,
        });
        sim.food.push(Food {
            x: 60.0,
            y: 20.0,
            vy: 0.0,
            life: 30.0,
            landed: false,
            sway_phase: 0.0,
        });

        sim.update(0.1, 80, 40);

        assert!(
            sim.fish[1].hunger > sim.fish[0].hunger,
            "feed_efficiency_multが大きい個体の方がよく満たされるはず: {} vs {}",
            sim.fish[1].hunger,
            sim.fish[0].hunger
        );
    }

    #[test]
    fn lifespan_mult_gives_individual_differences_in_longevity() {
        // 個体差の回帰テスト: 「寿命も個体差がある」。lifespan_multが大きい個体は、
        // 標準のLIFESPAN_DEATH_AGEに達してもまだ老衰死しないはず。
        let mut sim = Simulation::new(Rng::new(3));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0);
        f.lifespan_mult = 2.0;
        f.elderly_spawned = true; // 老齢確定産卵は済んだ前提にして、死亡判定だけを見る
        f.age = LIFESPAN_DEATH_AGE - 0.05;
        sim.fish.push(f);

        sim.update(0.1, 80, 40); // 標準のLIFESPAN_DEATH_AGEを跨ぐ

        assert!(!sim.fish[0].dead, "lifespan_mult=2.0の個体は標準の寿命ではまだ死なないはず");
    }

    #[test]
    fn growth_cap_variance_gives_individual_differences_in_max_size() {
        // 個体差の回帰テスト: 「大きくなるサイズも同様に個体差がある」。
        // growth_cap_variance=+1の個体は標準の上限(GENERAL_MAX_GROWTH_STAGE)を
        // 超えて1段階分大きくなれるはず。-1の個体は逆に1段階分手前で打ち止めになるはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(4));
        let mut big = Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0);
        big.growth_stage = GENERAL_MAX_GROWTH_STAGE;
        big.growth_cap_variance = 1;
        big.hunger = MAX_HUNGER;
        big.well_fed_timer = BREED_READY_TIME;
        sim.fish.push(big);

        run(
            &mut sim,
            SIZE_GROW_TIME * 1.5,
            0.5,
            w,
            h,
            false,
        );

        assert_eq!(
            sim.fish[0].growth_stage,
            GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE,
            "growth_cap_variance=+1の個体は標準上限を超えて成長できるはず"
        );

        let mut small = Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0);
        small.growth_stage = GENERAL_MAX_GROWTH_STAGE - 1;
        small.growth_cap_variance = -1;
        small.hunger = MAX_HUNGER;
        let mut sim2 = Simulation::new(Rng::new(5));
        sim2.fish.push(small);

        run(&mut sim2, SIZE_GROW_TIME * 1.5, 0.5, w, h, false);

        assert_eq!(
            sim2.fish[0].growth_stage,
            GENERAL_MAX_GROWTH_STAGE - 1,
            "growth_cap_variance=-1の個体は標準より1段階手前で打ち止めになるはず"
        );
    }

    #[test]
    fn newly_spawned_fish_get_randomized_individuality() {
        // add_fish経由で生成された魚は、Fish::new()のニュートラル値(1.0/1.0/1.0/0)から
        // 実際にばらついているはず(roll_individualityが呼ばれていることの確認)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(6));
        for _ in 0..20 {
            sim.add_fish(w, h);
        }

        let all_neutral = sim
            .fish
            .iter()
            .all(|f| f.hunger_decay_mult == 1.0 && f.feed_efficiency_mult == 1.0 && f.lifespan_mult == 1.0);
        assert!(!all_neutral, "20匹もいればニュートラル値のまま(未ロール)ではないはず");

        for f in &sim.fish {
            assert!(f.hunger_decay_mult >= INDIVIDUALITY_HUNGER_MULT_MIN);
            assert!(f.feed_efficiency_mult >= INDIVIDUALITY_HUNGER_MULT_MIN);
            assert!(f.lifespan_mult >= INDIVIDUALITY_LIFESPAN_MULT_MIN && f.lifespan_mult <= INDIVIDUALITY_LIFESPAN_MULT_MAX);
            assert!((-1..=1).contains(&f.growth_cap_variance));
        }
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
            sway_phase: 0.0,
        });
        sim.update(0.1, 80, 40);
        assert!(sim.fish[0].hunger > 10.0, "餌で空腹度が回復するはず");
        assert_eq!(sim.food_count(), 0, "食べられた餌は消えるはず");
    }

    #[test]
    fn hungry_fish_can_eventually_eat_food_that_has_landed_on_the_sand() {
        // 堆積した餌が食べられないという問題の回帰テスト。
        // スプライトサイズ基準の壁際マージンにより、水底に着地した餌に十分近づけず
        // 食べられないバグがあった(肉餌では既に同種の問題が修正済みだったが、通常の
        // 餌には同じ修正が漏れていた)。腹ぺこの魚を、着地済みの餌の真上に置いて
        // 十分な時間シミュレーションし、いつかは食べられて空腹度が回復することを確認する。
        let (w, h) = (80, 100);
        let sand_top = sand_height(h) as f64;
        let sand_top = h as f64 - sand_top;
        let mut sim = Simulation::new(Rng::new(21));
        let mut fish = Fish::new(Species::Betta, Stage::Adult, 40.0, sand_top - 40.0);
        fish.hunger = 5.0;
        sim.fish.push(fish);
        sim.food.push(Food {
            x: 40.0,
            y: sand_top,
            vy: 0.0,
            life: FOOD_LIFETIME,
            landed: true,
            sway_phase: 0.0,
        });
        let mut eaten = false;
        for _ in 0..2000 {
            sim.update(0.1, w, h);
            if sim.food_count() == 0 {
                eaten = true;
                break;
            }
        }
        assert!(eaten, "水底に着地した餌も、十分な時間があればいつかは食べられるはず");
        assert!(sim.fish[0].hunger > 5.0, "着地した餌を食べたら空腹度が回復するはず");
    }

    #[test]
    fn pollution_increases_when_landed_food_is_left_unattended() {
        // 水質パラメータの回帰テスト。水底に堆積した餌を放置すると水質(pollution)が
        // 悪化するはず(食べる魚がいない=堆積したまま)。
        let (w, h) = (80, 100);
        let mut sim = Simulation::new(Rng::new(1));
        let sand_top = h as f64 - sand_height(h) as f64;
        // 自然浄化(POLLUTION_NATURAL_DECAY)より明確に上回る悪化量になるよう、
        // 十分な数の餌を堆積させる。
        for _ in 0..20 {
            sim.food.push(Food {
                x: 40.0,
                y: sand_top,
                vy: 0.0,
                life: FOOD_LIFETIME,
                landed: true,
                sway_phase: 0.0,
            });
        }
        assert_eq!(sim.pollution, 0.0);
        sim.update(1.0, w, h);
        assert!(
            sim.pollution > 0.0,
            "堆積した餌を放置すると水質が悪化するはず(実際: {})",
            sim.pollution
        );
    }

    #[test]
    fn pollution_decays_naturally_when_nothing_is_left_unattended() {
        // 堆積物・病気・死亡個体が無ければ、自然浄化により水質は改善(減少)するはず。
        let (w, h) = (80, 100);
        let mut sim = Simulation::new(Rng::new(1));
        sim.pollution = 50.0;
        sim.update(1.0, w, h);
        assert!(
            sim.pollution < 50.0,
            "悪化要因が無ければ自然浄化で水質は改善するはず(実際: {})",
            sim.pollution
        );
    }

    #[test]
    fn pollution_is_clamped_within_its_valid_range() {
        let (w, h) = (80, 100);
        let mut sim = Simulation::new(Rng::new(1));
        sim.pollution = POLLUTION_MAX + 50.0;
        sim.update(1.0, w, h);
        assert!(
            sim.pollution <= POLLUTION_MAX,
            "水質はPOLLUTION_MAXを超えないはず(実際: {})",
            sim.pollution
        );

        sim.pollution = -20.0;
        sim.update(1.0, w, h);
        assert!(
            sim.pollution >= 0.0,
            "水質は0未満にならないはず(実際: {})",
            sim.pollution
        );
    }

    #[test]
    fn high_pollution_raises_sickness_onset_chance() {
        // 水質が悪いほど病気の発症確率が上がる(死にやすくなる)ことの回帰テスト。
        // 同じ発症条件(長期の腹ぺこ)を維持したまま、水質が綺麗な場合と最悪の場合で
        // 発症までの試行回数を比較し、最悪の方が早く(または同時に)発症することを
        // 確認する。starve_timerは毎tickリセットして、発症より先に餓死しないようにする
        // (餓死すると個体が消滅し、比較対象が失われてしまうため)。
        let (w, h) = (80, 40);
        let make_sim = |pollution: f64| {
            let mut sim = Simulation::new(Rng::new(55));
            sim.pollution = pollution;
            let f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
            sim.fish.push(f);
            sim
        };

        let ticks_to_sick = |mut sim: Simulation| -> Option<usize> {
            for i in 0..50_000 {
                sim.fish[0].hunger = 0.0;
                sim.fish[0].hungry_timer = HUNGRY_SICK_TIME + 1.0;
                sim.fish[0].starve_timer = 0.0;
                sim.update(1.0, w, h);
                if sim.fish[0].sick {
                    return Some(i);
                }
            }
            None
        };

        let clean_ticks = ticks_to_sick(make_sim(0.0));
        let dirty_ticks = ticks_to_sick(make_sim(POLLUTION_MAX));
        let (Some(clean), Some(dirty)) = (clean_ticks, dirty_ticks) else {
            panic!("綺麗な場合・最悪な場合のどちらも発症するはず(clean={clean_ticks:?} dirty={dirty_ticks:?})");
        };
        assert!(
            dirty <= clean,
            "水質最悪の方が発症確率が高く、同じ乱数シードならより早く(または同時に)発症するはず(clean={clean} dirty={dirty})"
        );
    }

    #[test]
    fn heavily_polluted_water_can_sicken_a_well_fed_uncrowded_fish() {
        // 回帰テスト: 水質の悪化は発症確率の倍率としてしか効いておらず、腹ぺこ継続・
        // 過密のどちらも満たさない(=満腹を保っている)健康な魚は、水質が最悪でも
        // 発症判定自体に入らず病気にならないという抜け穴があった。
        // POLLUTION_SICK_ELIGIBLE_FRAC以上悪化した水では、満腹を保っていても
        // 発症判定の対象に含まれるようになったことを確認する。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(9001));
        sim.pollution = POLLUTION_MAX; // 最悪
        let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        f.hunger = MAX_HUNGER; // 満腹を維持し続ける(腹ぺこ条件を満たさない)
        sim.fish.push(f);

        let mut became_sick = false;
        for _ in 0..50_000 {
            sim.fish[0].hunger = MAX_HUNGER;
            sim.pollution = POLLUTION_MAX; // 自然浄化で下がっても最悪の水質を維持し続ける
            sim.update(1.0, w, h);
            if sim.fish[0].sick {
                became_sick = true;
                break;
            }
        }
        assert!(
            became_sick,
            "水質が最悪なら、満腹で過密でない魚もいずれ病気になるはず"
        );
    }

    #[test]
    fn mild_pollution_does_not_make_a_well_fed_uncrowded_fish_eligible_for_sickness() {
        // POLLUTION_SICK_ELIGIBLE_FRAC未満の軽い汚れでは、満腹・非過密の魚は
        // これまでどおり発症判定の対象に入らない(常時なんでも病気になるわけではない)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(9002));
        sim.pollution = POLLUTION_MAX * (POLLUTION_SICK_ELIGIBLE_FRAC - 0.1);
        let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        f.hunger = MAX_HUNGER;
        sim.fish.push(f);

        for _ in 0..5_000 {
            sim.fish[0].hunger = MAX_HUNGER;
            sim.update(1.0, w, h);
        }
        assert!(
            !sim.fish[0].sick,
            "軽い汚れ(閾値未満)では満腹・非過密の魚は発症判定の対象外のはず"
        );
    }

    #[test]
    fn heavily_polluted_water_eventually_kills_fry_through_the_sickness_pipeline() {
        // 稚魚専用の直接死亡ロジックは廃止し、成魚と同じ病気の弱り→死亡経路
        // (発症→SICK_WEAK_TIME→SICK_DEATH_TIME)に一本化した。水質最悪だと
        // 発症確率がPOLLUTION_SICK_CHANCE_MAX_MULT倍になるため、稚魚もいずれ
        // 病気になり、病気経由で力尽きるはず。確率的なイベントを待つテストのため、
        // 他の箇所の変更でtickごとの乱数消費数が変わってもタイミングがずれにくいよう、
        // 十分大きめの試行回数にしてある。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(9003));
        sim.pollution = POLLUTION_MAX; // 最悪
        let mut fry = Fish::new(Species::Neon, Stage::Fry, 40.0, 20.0);
        fry.hunger = MAX_HUNGER; // 満腹を維持し、空腹側の死因ではないことを明確にする
        sim.fish.push(fry);

        let mut died = false;
        let mut was_sick = false;
        for _ in 0..20000 {
            if sim.fish.is_empty() {
                died = true;
                break;
            }
            sim.fish[0].hunger = MAX_HUNGER;
            // 満腹を維持すると稚魚はGROW_TIME経過で成魚に育ってしまうため、
            // 毎tick稚魚のままに固定して(成長タイマーもリセット)、稚魚のまま
            // 病気→死亡までを確実に検証する。
            sim.fish[0].stage = Stage::Fry;
            sim.fish[0].well_fed_timer = 0.0;
            sim.pollution = POLLUTION_MAX;
            if sim.fish[0].sick {
                was_sick = true;
            }
            sim.update(1.0, w, h);
            if sim.fish[0].dead {
                died = true;
                break;
            }
        }
        assert!(died, "水質最悪の稚魚もいずれ力尽きるはず");
        assert!(was_sick, "死ぬ前に病気を経由しているはず(直接死亡ロジックは廃止済み)");
    }

    #[test]
    fn heavily_polluted_water_does_not_directly_kill_adult_fish() {
        // 直接死亡する経路は稚魚(Fry)限定。成魚は満腹・非過密であれば、水質最悪でも
        // この経路では死なない(病気を経由する通常の経路のみ)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(9004));
        sim.pollution = POLLUTION_MAX;
        let mut adult = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        adult.hunger = MAX_HUNGER;
        sim.fish.push(adult);

        for _ in 0..300 {
            sim.fish[0].hunger = MAX_HUNGER;
            sim.pollution = POLLUTION_MAX;
            sim.update(1.0, w, h);
            if sim.fish[0].sick {
                break; // 病気経由の死亡は別の仕組みなので、発症したら打ち切る
            }
        }
        assert!(!sim.fish[0].dead, "成魚は水質による直接死亡の対象外のはず");
    }

    #[test]
    fn heavily_polluted_water_speeds_up_hunger_decay_for_common_species_but_not_piranha() {
        // 水質が悪化していると、捕食者でない通常種は空腹度の減りが速まる(食欲不振)。
        // ピラニアはこの影響を受けないはず。
        let (w, h) = (80, 40);
        let make_sim = |pollution: f64, species: Species| {
            let mut sim = Simulation::new(Rng::new(9010));
            sim.pollution = pollution;
            sim.fish.push(Fish::new(species, Stage::Adult, 40.0, 20.0));
            sim
        };

        let mut clean_common = make_sim(0.0, Species::Neon);
        let mut dirty_common = make_sim(POLLUTION_MAX, Species::Neon);
        clean_common.update(1.0, w, h);
        dirty_common.update(1.0, w, h);
        assert!(
            dirty_common.fish[0].hunger < clean_common.fish[0].hunger,
            "水質最悪の通常種は空腹度がより早く減るはず(clean={} dirty={})",
            clean_common.fish[0].hunger,
            dirty_common.fish[0].hunger
        );

        let mut clean_piranha = make_sim(0.0, Species::Piranha);
        let mut dirty_piranha = make_sim(POLLUTION_MAX, Species::Piranha);
        clean_piranha.update(1.0, w, h);
        dirty_piranha.update(1.0, w, h);
        assert_eq!(
            clean_piranha.fish[0].hunger, dirty_piranha.fish[0].hunger,
            "ピラニアは水質による空腹度減少の影響を受けないはず"
        );
    }

    #[test]
    fn heavily_polluted_water_stops_common_species_from_seeking_food() {
        // 水質が悪化していると、捕食者でない通常種は腹ぺこでも餌への誘引ベクトルが
        // つかない(食欲そのものを失う)はず。通常の遊泳(ランダムウォーク)自体は
        // 常に多少の速度を生むため、同じ乱数シード・同じ初期状態で「餌がある場合」
        // と「無い場合」の結果が一致することを比較して検証する。
        let (w, h) = (200, 100);
        let make_sim = |with_food: bool| {
            let mut sim = Simulation::new(Rng::new(9011));
            sim.pollution = POLLUTION_MAX;
            let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
            f.hunger = 0.0; // 腹ぺこ
            sim.fish.push(f);
            if with_food {
                sim.food.push(Food {
                    x: 100.0,
                    y: 20.0,
                    vy: 0.0,
                    life: FOOD_LIFETIME,
                    landed: false,
                    sway_phase: 0.0,
                });
            }
            sim
        };
        let mut with_food = make_sim(true);
        let mut without_food = make_sim(false);
        with_food.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);
        without_food.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);

        assert_eq!(
            with_food.fish[0].vx, without_food.fish[0].vx,
            "水質最悪では餌の有無で速度が変わらないはず(反応していない)"
        );
    }

    #[test]
    fn drop_purifier_adds_one_purifier_and_a_purify_drop_effect() {
        // 浄化剤(`C`キー)は1個だけ投下され、専用の投下エフェクト・効果音が出るはず。
        let mut sim = Simulation::new(Rng::new(71));
        sim.drop_purifier(40.0, 80);
        assert_eq!(sim.purifiers.len(), 1, "浄化剤は1個だけ投下されるはず");
        assert!(
            sim.drop_effects.iter().any(|e| e.kind == EffectKind::Purify),
            "浄化剤の投下エフェクト(EffectKind::Purify)が出るはず"
        );
        assert!(
            sim.sound_events.contains(&SfxEvent::Purify),
            "浄化剤投下で Purify イベントが発生するはず"
        );
    }

    #[test]
    fn purifier_triggers_effect_and_vanishes_on_landing() {
        // 浄化剤は水底に着いた瞬間に着水演出(ブルーム)を出して自身は取り除かれるが
        // (Food/Medicine/Meatのように水底へ停留しない)、効果(濃度加算)はまだ発動しない。
        // 拡散(PURIFY_BLOOM_GROWTH_TIME)が完了して初めて濃度が立つはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(72));
        sim.drop_purifier(40.0, w);
        assert_eq!(sim.purifiers.len(), 1);
        let mut landed = false;
        for _ in 0..500 {
            sim.update(0.1, w, h);
            if sim.purifiers.is_empty() {
                landed = true;
                break;
            }
        }
        assert!(landed, "浄化剤は水底に着けば取り除かれるはず(堆積しない)");
        assert!(!sim.purify_blooms.is_empty(), "着水で浄化ブルームが出るはず");
        assert_eq!(
            sim.purifier_concentration, 0.0,
            "着水した直後はまだ拡散中で、効果はまだ発動していないはず"
        );

        // 拡散が完了するまで(PURIFY_BLOOM_GROWTH_TIME分+余裕)進めると、濃度が立つ。
        for _ in 0..((PURIFY_BLOOM_GROWTH_TIME * 2.0 / 0.1) as usize) {
            sim.update(0.1, w, h);
        }
        assert!(
            sim.purifier_concentration > 0.9,
            "拡散が完了すれば濃度が最大近く(≈1.0)に立つはず(実際: {})",
            sim.purifier_concentration
        );
    }

    #[test]
    fn repeated_purifier_drops_stack_concentration_above_one() {
        // 効果が薄まりきる前に連投すると、濃度は1.0で頭打ちにならず加算されて
        // 積み上がるはず(それに伴い浄化・食欲不振・老化の効果も強まる設計)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(74));
        let settle_ticks = ((PURIFY_BLOOM_GROWTH_TIME * 2.0 / 0.1) as usize).max(500);

        sim.drop_purifier(40.0, w);
        for _ in 0..settle_ticks {
            sim.update(0.1, w, h);
        }
        assert!(sim.purifier_concentration > 0.9);

        sim.drop_purifier(40.0, w);
        for _ in 0..settle_ticks {
            sim.update(0.1, w, h);
        }
        assert!(
            sim.purifier_concentration > 1.5,
            "連投すると濃度は1.0を超えて積み上がるはず(実際: {})",
            sim.purifier_concentration
        );
    }

    #[test]
    fn purifier_concentration_dilutes_linearly_to_zero() {
        // 濃度は着水後PURIFIER_DILUTION_TIMEをかけて線形に0まで薄まるはず。
        let mut sim = Simulation::new(Rng::new(73));
        sim.purifier_concentration = 1.0;
        // 希釈時間の半分だけ進めれば、濃度はおよそ半分(0.5)になるはず。
        let half_steps = (PURIFIER_DILUTION_TIME / 2.0) as usize;
        for _ in 0..half_steps {
            sim.update_purifier_concentration(1.0);
        }
        assert!(
            (sim.purifier_concentration - 0.5).abs() < 1e-6,
            "希釈時間の半分で濃度は約0.5になるはず(実際: {})",
            sim.purifier_concentration
        );
        // 残りの時間を超えて進めれば0でクランプされるはず。
        for _ in 0..(half_steps + 10) {
            sim.update_purifier_concentration(1.0);
        }
        assert_eq!(
            sim.purifier_concentration, 0.0,
            "希釈時間を超えれば濃度は0になるはず"
        );
    }

    #[test]
    fn purifier_concentration_cleans_pollution_faster() {
        // 浄化剤の濃度がある方が、自然浄化だけの場合より水質(pollution)がより下がるはず。
        let (w, h) = (80, 40);
        let make_sim = |conc: f64| {
            let mut sim = Simulation::new(Rng::new(74));
            sim.pollution = 100.0;
            sim.purifier_concentration = conc;
            sim
        };
        let mut with_purifier = make_sim(1.0);
        let mut without_purifier = make_sim(0.0);
        for _ in 0..20 {
            with_purifier.update(0.5, w, h);
            without_purifier.update(0.5, w, h);
        }
        assert!(
            with_purifier.pollution < without_purifier.pollution,
            "浄化剤の濃度がある方が水質はより下がるはず(with={} without={})",
            with_purifier.pollution,
            without_purifier.pollution
        );
    }

    #[test]
    fn purifier_speeds_up_hunger_decay_for_common_species_but_not_piranha() {
        // 浄化剤の効果中は、捕食者でない通常種は空腹度の減りが速まる(食欲不振)。
        // ピラニアはこの影響を受けないはず(水質由来の食欲不振と同じ扱い)。
        let (w, h) = (80, 40);
        let make_sim = |conc: f64, species: Species| {
            let mut sim = Simulation::new(Rng::new(9010));
            sim.purifier_concentration = conc;
            sim.fish.push(Fish::new(species, Stage::Adult, 40.0, 20.0));
            sim
        };

        let mut clean_common = make_sim(0.0, Species::Neon);
        let mut dosed_common = make_sim(1.0, Species::Neon);
        clean_common.update(1.0, w, h);
        dosed_common.update(1.0, w, h);
        assert!(
            dosed_common.fish[0].hunger < clean_common.fish[0].hunger,
            "浄化剤の効果中は通常種の空腹度がより速く減るはず(clean={} dosed={})",
            clean_common.fish[0].hunger,
            dosed_common.fish[0].hunger
        );

        let mut clean_piranha = make_sim(0.0, Species::Piranha);
        let mut dosed_piranha = make_sim(1.0, Species::Piranha);
        clean_piranha.update(1.0, w, h);
        dosed_piranha.update(1.0, w, h);
        assert_eq!(
            clean_piranha.fish[0].hunger, dosed_piranha.fish[0].hunger,
            "ピラニアは浄化剤による空腹度減少(食欲不振)の影響を受けないはず"
        );
    }

    #[test]
    fn purifier_stops_common_species_from_seeking_food() {
        // 浄化剤の効果中は、捕食者でない通常種は腹ぺこでも餌への誘引ベクトルがつかない。
        // 水質由来の食欲不振テストと同じく、「餌がある場合」と「無い場合」で結果が
        // 一致することを比較して検証する。
        let (w, h) = (200, 100);
        let make_sim = |with_food: bool| {
            let mut sim = Simulation::new(Rng::new(9011));
            sim.purifier_concentration = 1.0;
            let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
            f.hunger = 0.0; // 腹ぺこ
            sim.fish.push(f);
            if with_food {
                sim.food.push(Food {
                    x: 100.0,
                    y: 20.0,
                    vy: 0.0,
                    life: FOOD_LIFETIME,
                    landed: false,
                    sway_phase: 0.0,
                });
            }
            sim
        };
        let mut with_food = make_sim(true);
        let mut without_food = make_sim(false);
        with_food.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);
        without_food.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);

        assert_eq!(
            with_food.fish[0].vx, without_food.fish[0].vx,
            "浄化剤の効果中は餌の有無で速度が変わらないはず(反応していない)"
        );
    }

    #[test]
    fn purifier_speeds_up_aging_for_all_species_including_piranha() {
        // 老化速度の加速は食欲不振と違い全種対象(捕食者=ピラニアも含む)。
        // 濃度ありの方が濃度なしより age がより進むことを、通常種とピラニアの両方で確認する。
        let (w, h) = (80, 40);
        let make_sim = |conc: f64, species: Species| {
            let mut sim = Simulation::new(Rng::new(9012));
            sim.purifier_concentration = conc;
            sim.fish.push(Fish::new(species, Stage::Adult, 40.0, 20.0));
            sim
        };
        for species in [Species::Neon, Species::Piranha] {
            let mut clean = make_sim(0.0, species);
            let mut dosed = make_sim(1.0, species);
            for _ in 0..10 {
                clean.update(0.5, w, h);
                dosed.update(0.5, w, h);
            }
            assert!(
                dosed.fish[0].age > clean.fish[0].age,
                "浄化剤の効果中は全種({:?}含む)の老化がより速く進むはず(clean={} dosed={})",
                species,
                clean.fish[0].age,
                dosed.fish[0].age
            );
        }
    }

    #[test]
    fn octopus_dies_bleeding_when_purifier_concentration_exceeds_one() {
        // 浄化剤の連投で濃度が100%を超えると、タコは血を吐いて死亡するはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(9013));
        sim.purifier_concentration = 1.01;
        let mut octopus = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octopus.hidden = false;
        sim.fish.push(octopus);
        sim.update(0.1, w, h);
        assert!(sim.fish[0].dead, "濃度100%超のタコは死亡するはず");
        assert!(
            sim.drop_effects.iter().any(|e| e.kind == EffectKind::Blood),
            "血飛沫が出るはず"
        );
        assert!(!sim.blood_stains.is_empty(), "血の滲みが出るはず");
    }

    #[test]
    fn octopus_does_not_die_from_purifier_at_exactly_one() {
        // 100%ちょうど(1.0)ではまだ死なない(「超えたら」なので厳密に1.0より大きい時のみ)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(9014));
        sim.purifier_concentration = 1.0;
        let mut octopus = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octopus.hidden = false;
        sim.fish.push(octopus);
        sim.update(0.1, w, h);
        assert!(!sim.fish[0].dead, "濃度ちょうど100%ではまだ死なないはず");
    }

    #[test]
    fn fry_bleeds_and_dies_when_purifier_concentration_reaches_half() {
        // 浄化剤の濃度が50%以上になると、稚魚はピラニアの捕食と同規模の
        // 大量出血とともに直接死亡するはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(9015));
        sim.purifier_concentration = 0.5;
        sim.fish.push(Fish::new(Species::Neon, Stage::Fry, 40.0, 20.0));
        sim.update(0.1, w, h);
        assert!(sim.fish[0].dead, "濃度50%以上の稚魚は死亡するはず");
        assert!(
            sim.drop_effects.iter().filter(|e| e.kind == EffectKind::Blood).count() >= BLOOD_PARTICLE_COUNT,
            "ピラニアの捕食と同規模の血飛沫が出るはず"
        );
    }

    #[test]
    fn fry_survives_purifier_below_half_concentration() {
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(9016));
        sim.purifier_concentration = 0.49;
        sim.fish.push(Fish::new(Species::Neon, Stage::Fry, 40.0, 20.0));
        sim.update(0.1, w, h);
        assert!(!sim.fish[0].dead, "濃度50%未満の稚魚はこの経路では死なないはず");
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
    fn default_feed_amount_is_within_the_configured_normal_range() {
        // 餌の量を設定できるようにする機能の回帰テスト。
        // 既定値(feed_amount未設定=レベル1=ふつう)では、レベル1の範囲(15〜25粒)に収まるはず。
        let mut sim = Simulation::new(Rng::new(1));
        assert_eq!(sim.feed_amount, FEED_AMOUNT_DEFAULT);
        sim.feed(40.0, 200);
        assert!(
            (15..=25).contains(&sim.food.len()),
            "既定(ふつう)の餌の量は15〜25粒のはず(実際: {})",
            sim.food.len()
        );
    }

    #[test]
    fn max_feed_amount_dumps_far_more_food_spread_across_a_third_of_the_tank() {
        // 最大レベル(どっぱー)は粒数が大幅に増え、散らばり幅も水槽横幅の約1/3に
        // 広がるはず(最大投下時は水槽の1/3程度が埋まる量、という目安に基づく)。
        let pix_w = 300usize;
        let mut sim = Simulation::new(Rng::new(1));
        sim.feed_amount = FEED_AMOUNT_LEVELS - 1;
        let cursor_x = pix_w as f64 / 2.0;
        sim.feed(cursor_x, pix_w);
        assert!(
            sim.food.len() >= 250,
            "MAXレベルは大量の粒が投下されるはず(実際: {})",
            sim.food.len()
        );
        let expected_spread = pix_w as f64 / 6.0;
        assert!(
            sim.food.iter().any(|f| (f.x - cursor_x).abs() > expected_spread * 0.5),
            "MAXレベルは散らばり幅が水槽横幅の約1/3相当まで広がるはず"
        );
        assert_eq!(
            sim.message.as_deref(),
            Some("どっぱー！餌を大量投入した"),
            "MAXレベルでの餌やりは専用メッセージが出るはず"
        );
    }

    #[test]
    fn cycle_feed_amount_wraps_around_from_max_to_min() {
        let mut sim = Simulation::new(Rng::new(1));
        assert_eq!(sim.feed_amount, FEED_AMOUNT_DEFAULT);
        for _ in 0..(FEED_AMOUNT_LEVELS - 1 - FEED_AMOUNT_DEFAULT) {
            sim.cycle_feed_amount();
        }
        assert_eq!(sim.feed_amount, FEED_AMOUNT_LEVELS - 1, "MAXまで進むはず");
        sim.cycle_feed_amount();
        assert_eq!(sim.feed_amount, 0, "MAXの次は最小に戻るはず");
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
    fn auto_feed_count_matches_hungry_count_and_ignores_feed_amount_setting() {
        // 自動餌やりを`feed_amount`設定と分離した回帰テスト。手動投下量をMAXにしても、
        // 自動餌やりの量はfeed_amount(MAXなら250〜350粒相当)ではなく、実際に空腹な
        // 個体数(この場合4匹)に一致するはず(水質が悪化しすぎないようにするため)。
        let mut sim = Simulation::new(Rng::new(403));
        sim.feed_amount = FEED_AMOUNT_LEVELS - 1;
        for i in 0..4 {
            let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0 + i as f64, 20.0);
            f.hunger = 5.0;
            sim.fish.push(f);
        }

        sim.update_auto_care(0.1, 80, 40);

        assert_eq!(
            sim.food.len(),
            4,
            "自動餌やりの量は空腹な個体数(4匹)に一致するはず(実際: {})",
            sim.food.len()
        );
    }

    #[test]
    fn auto_feed_and_medicate_counts_are_capped_for_a_large_neglected_tank() {
        // 空腹・病気の個体数が非常に多くても、水質悪化・処理落ちを防ぐため
        // AUTO_FEED_COUNT_CAP/AUTO_MEDICATE_COUNT_CAPで頭打ちになるはず。
        let mut sim = Simulation::new(Rng::new(404));
        for i in 0..(AUTO_FEED_COUNT_CAP + 10) {
            let mut f = Fish::new(Species::Neon, Stage::Adult, 4.0 + i as f64 * 0.5, 20.0);
            f.hunger = 5.0;
            f.sick = true;
            sim.fish.push(f);
        }

        sim.update_auto_care(0.1, 200, 40);

        assert_eq!(
            sim.food.len(),
            AUTO_FEED_COUNT_CAP,
            "空腹な個体が多くても餌はAUTO_FEED_COUNT_CAPで頭打ちのはず(実際: {})",
            sim.food.len()
        );
        assert_eq!(
            sim.medicine.len(),
            AUTO_MEDICATE_COUNT_CAP,
            "病気の個体が多くても薬はAUTO_MEDICATE_COUNT_CAPで頭打ちのはず(実際: {})",
            sim.medicine.len()
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
            "自動餌やりはfeed_amount設定に関わらず常に3〜5粒のはず: {count_after_burst}"
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
    fn tap_attract_emits_sound_and_drop_effect_at_cursor() {
        let mut sim = Simulation::new(Rng::new(720));
        sim.tap_attract(40.0, 20.0, 80, 40);
        assert!(
            sim.sound_events.contains(&SfxEvent::Tap),
            "トントンすると Tap イベントが発生するはず"
        );
        assert_eq!(sim.drop_effects.len(), 1, "トントンした位置に波紋エフェクトが1つ出るはず");
        assert_eq!(sim.drop_effects[0].kind, EffectKind::Tap);
    }

    #[test]
    fn tap_attract_makes_nearby_fish_approach_but_not_distant_fish() {
        // `t`(コンコン)の逆: 近くの魚はカーソル位置に興味を持って寄ってくる
        // (逃走ではなく引き寄せ状態になる)。十分離れた魚は対象外。
        let mut sim = Simulation::new(Rng::new(721));
        let near = Fish::new(Species::Neon, Stage::Adult, 42.0, 21.0);
        let far = Fish::new(Species::Goldfish, Stage::Adult, 79.0, 39.0); // TAP_RADIUS=18より遠い
        sim.fish.push(near);
        sim.fish.push(far);

        sim.tap_attract(40.0, 20.0, 80, 40);

        assert!(sim.fish[0].attract_timer > 0.0, "近くの魚は引き寄せ状態になるはず");
        assert_eq!(sim.fish[0].flee_timer, 0.0, "驚いて逃げる状態にはならないはず");
        assert_eq!(sim.fish[1].attract_timer, 0.0, "離れた魚は引き寄せ状態にならないはず");
    }

    #[test]
    fn tap_attract_ignores_dead_fish() {
        let mut sim = Simulation::new(Rng::new(722));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 41.0, 21.0);
        f.dead = true;
        sim.fish.push(f);
        sim.tap_attract(40.0, 20.0, 80, 40);
        assert_eq!(sim.fish[0].attract_timer, 0.0, "死亡演出中の魚は反応しないはず");
    }

    #[test]
    fn tap_attract_moves_fish_toward_cursor_and_timer_expires() {
        let mut sim = Simulation::new(Rng::new(723));
        let fish = Fish::new(Species::Neon, Stage::Adult, 42.0, 20.0);
        sim.fish.push(fish);
        let start_x = sim.fish[0].x;
        // トントン位置(40.0)は魚(42.0)より-x側 → 引き寄せ方向は-x
        sim.tap_attract(40.0, 20.0, 80, 40);
        assert!(sim.fish[0].attract_timer > 0.0);

        sim.update(0.1, 80, 40);
        assert!(
            sim.fish[0].x <= start_x,
            "興味を持った魚はトントン位置へ近づく方向へ動くはず: start={start_x} now={}",
            sim.fish[0].x
        );

        // 十分な時間が経てば引き寄せ状態は終わる
        run(&mut sim, TAP_ATTRACT_DURATION + 1.0, 0.1, 80, 40, false);
        assert_eq!(sim.fish[0].attract_timer, 0.0, "十分時間が経てば引き寄せ状態は終わるはず");
    }

    #[test]
    fn tap_attract_raises_affinity_for_responding_fish_only() {
        // なつき度の回帰テスト。トントンに反応した魚(TAP_RADIUS以内)はなつき度が
        // 上がり、反応しなかった遠い魚は変化しないはず。
        let mut sim = Simulation::new(Rng::new(724));
        let near = Fish::new(Species::Neon, Stage::Adult, 42.0, 20.0);
        let far = Fish::new(Species::Neon, Stage::Adult, 79.0, 39.0); // TAP_RADIUS外
        sim.fish.push(near);
        sim.fish.push(far);

        sim.tap_attract(40.0, 20.0, 80, 40);

        assert!(sim.fish[0].affinity > 0.0, "反応した魚のなつき度は上がるはず");
        assert_eq!(sim.fish[1].affinity, 0.0, "反応しなかった魚のなつき度は変化しないはず");
    }

    #[test]
    fn tap_attract_affinity_gain_is_rate_limited_by_cooldown() {
        // 連打してもクールダウン中はなつき度が上がらない(瞬時カンスト防止)。
        let mut sim = Simulation::new(Rng::new(725));
        sim.fish
            .push(Fish::new(Species::Neon, Stage::Adult, 42.0, 20.0));

        sim.tap_attract(40.0, 20.0, 80, 40);
        let after_first = sim.fish[0].affinity;
        assert_eq!(after_first, AFFINITY_GAIN_PER_TAP);

        // クールダウン中に連打しても増えない
        sim.tap_attract(40.0, 20.0, 80, 40);
        sim.tap_attract(40.0, 20.0, 80, 40);
        assert_eq!(
            sim.fish[0].affinity, after_first,
            "クールダウン中の連打ではなつき度は増えないはず"
        );

        // クールダウンが明けた後は再び増える(シミュレーション時間経過による
        // クールダウン消化を直接再現する。sim.update()を挟むと通常の遊泳で魚が
        // カーソルから離れてしまい、範囲外で再現性が崩れるため)。
        sim.fish[0].affinity_cooldown = 0.0;
        sim.tap_attract(40.0, 20.0, 80, 40);
        assert!(
            sim.fish[0].affinity > after_first,
            "クールダウンが明ければ再びなつき度が増えるはず"
        );
    }

    #[test]
    fn affinity_decays_slowly_over_time_and_clamps_at_zero() {
        let mut sim = Simulation::new(Rng::new(726));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        f.affinity = 10.0;
        sim.fish.push(f);

        sim.update(1.0, 80, 40);
        assert!(
            sim.fish[0].affinity < 10.0 && sim.fish[0].affinity > 0.0,
            "時間経過でなつき度はゆっくり下がるはず(実際: {})",
            sim.fish[0].affinity
        );

        // 十分な時間放置すれば0未満にはならない(keep_fed=trueで餓死を防ぎつつ経過させる)
        run(&mut sim, 1000.0, 10.0, 80, 40, true);
        assert_eq!(sim.fish[0].affinity, 0.0, "なつき度は0未満にならないはず");
    }

    #[test]
    fn affinity_is_capped_at_affinity_max() {
        let mut sim = Simulation::new(Rng::new(727));
        sim.fish
            .push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));
        for _ in 0..1000 {
            sim.fish[0].affinity_cooldown = 0.0; // 連打相当だが毎回クールダウンを解除して上限のみ検証する
            sim.tap_attract(40.0, 20.0, 80, 40);
        }
        assert_eq!(sim.fish[0].affinity, AFFINITY_MAX, "なつき度はAFFINITY_MAXを超えないはず");
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

        // 当たり判定を胴体でなく口にすべきという指摘への対応: 捕食判定は口(進行
        // 方向側のスプライト前端)基準になったため、1tickの中心距離だけでは判定できず、
        // 追跡して口が届くまで数tick必要になった。
        for _ in 0..30 {
            sim.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.update(0.1, 80, 40);
            // ピラニアの噛みつきは1発では殺さず段階的に弱らせるため、「捕食(1発目)が
            // 成立した」= 獲物の被噛みつき回数が増えた、で打ち切る。
            if sim.fish[1].piranha_bite_count > 0 {
                break;
            }
        }

        // 1発目では獲物は死なず、負傷した状態で水槽に残るはず。
        assert_eq!(sim.fish.len(), 2, "ピラニアの噛みつきでは獲物は即消滅しないはず");
        assert!(!sim.fish[1].dead, "1発目では死なず負傷にとどまるはず");
        assert_eq!(sim.fish[1].piranha_bite_count, 1, "1発目で被噛みつき回数が1になるはず");
        assert_eq!(sim.fish[0].species, Species::Piranha, "ピラニアは生きて残るはず");
        assert!(!sim.fish[0].dead, "捕食したピラニア自身は死なないはず");
        assert!(sim.fish[0].hunger > PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0, "捕食で空腹度が回復するはず");
        assert_eq!(sim.fish[0].predation_cooldown, PIRANHA_HUNT_COOLDOWN, "捕食後はクールダウンに入るはず");
        assert!(
            sim.sound_events.contains(&SfxEvent::Predation),
            "捕食で Predation イベントが発生するはず"
        );
        // 血飛沫は複数粒子を散らす強化演出になったため、1個ではなく
        // BLOOD_PARTICLE_COUNT個出るはず(派手・グロテスクに強化する実機要望対応)。
        // 噛みつきごとに毎回出るので、1発目でもこの数だけ出る。
        assert_eq!(
            sim.drop_effects.len(),
            BLOOD_PARTICLE_COUNT,
            "捕食の瞬間に血飛沫エフェクトが複数粒子出るはず"
        );
        assert!(sim.drop_effects.iter().all(|e| e.kind == EffectKind::Blood));
        assert!(
            sim.message.as_deref().unwrap_or("").contains("がぶり"),
            "1発目はがぶりとやられた旨のメッセージが表示されるはず"
        );
        assert_eq!(sim.fish[0].kill_stage, 1, "噛みつくたびにkill_stageが増えるはず");
        // 血の滲み(範囲エフェクト)も捕食位置に1つ出るはず
        assert_eq!(sim.blood_stains.len(), 1, "捕食で血の滲みが1つ出るはず");
        // このtick内で生成後すぐにdt(0.1)分減衰するため、ほぼ満タンのはず
        assert!(sim.blood_stains[0].life > BLOOD_STAIN_LIFETIME - 0.2);
    }

    #[test]
    fn piranha_kill_leaves_a_floating_corpse_instead_of_vanishing() {
        // ピラニアの捕食は即消滅ではなく、出血して力尽きた死骸として水槽に残り、
        // 通常の死骸パイプライン(浮上→沈降→片付け)に乗るはずの回帰テスト。
        let mut sim = Simulation::new(Rng::new(100));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0));

        // 1発では死なずPIRANHA_BITES_TO_KILL発で死ぬので、クールダウンを毎tick解除して
        // 連続で噛ませ、死亡まで到達させる。
        for _ in 0..30 {
            sim.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.fish[0].predation_cooldown = 0.0;
            sim.update(0.1, 80, 40);
            if sim.fish[1].dead {
                break;
            }
        }

        // 獲物は消えず、死骸として残っている。
        assert_eq!(sim.fish.len(), 2, "ピラニアの捕食では獲物は消えず死骸として残るはず");
        assert!(sim.fish[1].dead, "襲われた獲物は死亡状態になるはず");
        // 血の演出は消滅する場合と同様に出ているはず。
        assert!(!sim.blood_stains.is_empty(), "捕食で血の滲みが出るはず");
        assert!(
            sim.drop_effects.iter().any(|e| e.kind == EffectKind::Blood),
            "捕食で血飛沫パーティクルが出るはず"
        );
        assert!(
            sim.sound_events.contains(&SfxEvent::Predation),
            "捕食で Predation イベントが発生するはず"
        );

        // 死亡してからの経過時間が計測され始め、さらにtickを進めると増えていくはず。
        let before = sim.fish[1].dead_timer;
        sim.update(0.1, 80, 40);
        assert!(
            sim.fish[1].dead_timer > before,
            "死骸の経過時間(dead_timer)が進んでいくはず: before={before}, after={}",
            sim.fish[1].dead_timer
        );
    }

    #[test]
    fn octopus_kill_still_removes_prey_instantly() {
        // 対になる回帰テスト: タコの捕食はこれまでどおり獲物を即消滅させ、死骸を
        // 残さないこと(ピラニアの出血死骸化は分岐しており、タコには波及しない)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(605));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        octo.hunger = OCTOPUS_HUNT_HUNGER_THRESHOLD - 10.0;
        let (mx, my) = octo.mouth_position();
        sim.fish.push(octo);
        // 常に捕食対象になる稚魚を口の位置に置く。
        sim.fish.push(Fish::new(Species::Neon, Stage::Fry, mx, my));

        sim.update(0.1, w, h);

        assert_eq!(sim.fish.len(), 1, "タコの捕食では獲物は即消滅し死骸を残さないはず");
        assert_eq!(sim.fish[0].species, Species::Octopus);
        assert!(!sim.fish[0].dead, "タコ自身は生きて残るはず");
    }

    #[test]
    fn piranha_needs_three_bites_before_truly_full() {
        // 「食欲もっと旺盛に」対応の回帰テスト。hungerは1発の噛みつきで満腹相当まで
        // 回復してしまうが、PIRANHA_KILLS_TO_FULL発ぶん噛みつくまでは狩りをやめない
        // はず。ちょうどそのぶんの噛みつきを終えた時点で満腹が確定し、カウンタが0に戻り、
        // それ以上は狩らないはず(1発では死なない仕様のため「匹数」ではなく「噛みつき回数」で
        // 満腹判定していることを確認する)。
        let mut sim = Simulation::new(Rng::new(101));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);
        for i in 0..4 {
            sim.fish.push(Fish::new(
                Species::Neon,
                Stage::Adult,
                43.0 + i as f64 * 0.01,
                20.0,
            ));
        }

        // 実際に何発噛んだかは「死骸1体=PIRANHA_BITES_TO_KILL発ぶん + 生存個体の被噛みつき回数」で
        // 数える。噛みつき回数が食欲クォータ(PIRANHA_KILLS_TO_FULL)に達したら打ち切る。
        let total_bites = |sim: &Simulation| -> u32 {
            sim.fish
                .iter()
                .skip(1)
                .map(|f| {
                    if f.dead {
                        PIRANHA_BITES_TO_KILL as u32
                    } else {
                        f.piranha_bite_count as u32
                    }
                })
                .sum()
        };

        for _ in 0..300 {
            sim.fish[0].predation_cooldown = 0.0; // クールダウン明けを即座に再現する
            sim.update(0.1, 80, 40);
            if total_bites(&sim) >= PIRANHA_KILLS_TO_FULL {
                break;
            }
        }

        assert_eq!(
            total_bites(&sim),
            PIRANHA_KILLS_TO_FULL,
            "食欲クォータぶん(PIRANHA_KILLS_TO_FULL発)噛んだら満腹になり、それ以上は噛まないはず"
        );
        assert_eq!(sim.fish[0].species, Species::Piranha);
        assert!(!sim.fish[0].dead, "ピラニア自身は生きているはず");
        assert_eq!(
            sim.fish[0].piranha_meals_since_full, 0,
            "クォータぶん噛んだタイミングでカウンタはリセットされるはず"
        );
        assert!(
            sim.fish[0].hunger >= PIRANHA_HUNT_HUNGER_THRESHOLD,
            "クォータぶん噛んだ後は満腹相当になっているはず"
        );
    }

    // テスト用: ピラニア(index 0)に獲物(index 1)をちょうど1発だけ噛ませる。獲物を口の
    // 位置へ置き直し、空腹・クールダウン明けにしてから1tickずつ進め、被噛みつき回数か
    // 死亡フラグが1段階進んだ時点で返す(1tickの捕食は高々1回のため、確実に1発だけ増える)。
    fn land_one_piranha_bite(sim: &mut Simulation) {
        let before_count = sim.fish[1].piranha_bite_count;
        let before_dead = sim.fish[1].dead;
        for _ in 0..20 {
            sim.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.fish[0].predation_cooldown = 0.0;
            let (mx, my) = sim.fish[0].mouth_position();
            sim.fish[1].x = mx;
            sim.fish[1].y = my;
            sim.update(0.05, 80, 40);
            if sim.fish[1].piranha_bite_count != before_count || sim.fish[1].dead != before_dead {
                return;
            }
        }
        panic!("1発の噛みつきが成立しなかった(テストの前提が壊れている)");
    }

    // land_one_piranha_biteと同じだが、hungerを毎tick強制的に書き戻さない版。
    // 部分満腹化(PIRANHA_PARTIAL_BITE_HUNGER_FRAC)の回復量を素の値で観測したい
    // テスト用に使う(呼び出し側で事前に低いhungerをセットしておくこと)。
    fn land_one_piranha_bite_without_hunger_reset(sim: &mut Simulation) {
        let before_count = sim.fish[1].piranha_bite_count;
        let before_dead = sim.fish[1].dead;
        for _ in 0..20 {
            sim.fish[0].predation_cooldown = 0.0;
            let (mx, my) = sim.fish[0].mouth_position();
            sim.fish[1].x = mx;
            sim.fish[1].y = my;
            sim.update(0.05, 80, 40);
            if sim.fish[1].piranha_bite_count != before_count || sim.fish[1].dead != before_dead {
                return;
            }
        }
        panic!("1発の噛みつきが成立しなかった(テストの前提が壊れている)");
    }

    #[test]
    fn piranha_non_killing_bite_recovers_only_a_third_of_the_full_kill_gain() {
        // 部分満腹化の回帰テスト: 殺すまで至らない噛みつき(1発目)は、殺した時と同じ
        // 全回復量(PIRANHA_PREDATION_HUNGER_GAIN)ではなく、その1/3
        // (PIRANHA_PARTIAL_BITE_HUNGER_FRAC)だけ回復するはず。
        let mut sim = Simulation::new(Rng::new(221));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = 0.0; // 十分低くして、この1発の回復量がクランプされないようにする
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0)); // 無傷(bite_count=0)

        land_one_piranha_bite_without_hunger_reset(&mut sim);

        assert_eq!(sim.fish[1].piranha_bite_count, 1, "1発目で被噛みつき回数が1になるはず");
        assert!(!sim.fish[1].dead, "1発目では死なないはず");
        let expected_partial = PIRANHA_PREDATION_HUNGER_GAIN * PIRANHA_PARTIAL_BITE_HUNGER_FRAC;
        assert!(
            (sim.fish[0].hunger - expected_partial).abs() < 1.0,
            "殺すまで至らない噛みつきの回復量は全回復量の1/3のはず: hunger={} expected={}",
            sim.fish[0].hunger,
            expected_partial
        );
        assert!(
            sim.fish[0].hunger < PIRANHA_PREDATION_HUNGER_GAIN - 10.0,
            "殺すまで至らない噛みつきの回復量は、殺した時の全回復量よりはっきり小さいはず: hunger={}",
            sim.fish[0].hunger
        );
    }

    #[test]
    fn piranha_killing_bite_still_recovers_the_full_gain() {
        // 殺した(PIRANHA_BITES_TO_KILL発目)ときは、従来どおり全回復量のままのはず。
        let mut sim = Simulation::new(Rng::new(222));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = 0.0; // 十分低くして、回復量がクランプされないようにする
        sim.fish.push(piranha);
        let mut neon = Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0);
        neon.piranha_bite_count = PIRANHA_BITES_TO_KILL - 1; // 次の一噛みで死亡するところまで弱らせておく
        sim.fish.push(neon);

        land_one_piranha_bite_without_hunger_reset(&mut sim);

        assert!(sim.fish[1].dead, "最後の一噛みで死亡するはず");
        assert!(
            (sim.fish[0].hunger - PIRANHA_PREDATION_HUNGER_GAIN).abs() < 1.0,
            "殺した瞬間は全回復量のはず: hunger={} expected={}",
            sim.fish[0].hunger,
            PIRANHA_PREDATION_HUNGER_GAIN
        );
    }

    #[test]
    fn piranha_bite_leaves_a_blood_scent_that_fades_over_time() {
        // 血の匂いの回帰テスト: 噛みつき(殺すまで至らない負傷時)の瞬間、その位置に
        // 血の匂い(BloodScent)が発生し、時間経過で自然に消えるはず。
        let mut sim = Simulation::new(Rng::new(223));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0));

        assert!(sim.blood_scents.is_empty(), "噛みつき前は血の匂いは無いはず");
        land_one_piranha_bite(&mut sim);
        assert_eq!(sim.fish[1].piranha_bite_count, 1);
        assert!(
            !sim.blood_scents.is_empty(),
            "1発目の噛みつきでも血の匂いが発生するはず"
        );

        // この後の待機中に(BLOOD_SCENT_LIFETIME(20秒)がPIRANHA_HUNT_COOLDOWN(15秒)より
        // 長いため)ピラニアがもう一度噛んで血の匂いが上書き延命されてしまわないよう、
        // 満腹・クールダウンを十分長く確定させて狩りを完全に止めておく
        // (このテストで見たいのは「時間経過で薄れて消える」ことそのものだけ)。
        sim.fish[0].hunger = MAX_HUNGER;
        sim.fish[0].piranha_meals_since_full = 0;
        sim.fish[0].predation_cooldown = BLOOD_SCENT_LIFETIME + 10.0;

        // 十分な時間が経てば、血の匂いは自然に薄れて消える。
        run(&mut sim, BLOOD_SCENT_LIFETIME + 1.0, 0.1, 80, 40, false);
        assert!(
            sim.blood_scents.is_empty(),
            "十分な時間が経てば血の匂いは消えるはず"
        );
    }

    #[test]
    fn satiated_piranha_is_pulled_toward_a_blood_scent_it_would_otherwise_ignore() {
        // 血の匂いの追跡の回帰テスト: 満腹中・クールダウン中で通常なら誰も追跡しない
        // (chase_targetが常にNoneになる)状態のピラニアでも、検知範囲内に血の匂いが
        // あれば、その方向へはっきり加速するはず。
        let mut sim = Simulation::new(Rng::new(224));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = MAX_HUNGER; // 満腹(通常の狩りゲートなら誰も追跡しないはず)
        piranha.predation_cooldown = PIRANHA_HUNT_COOLDOWN; // クールダウン中でもある
        sim.fish.push(piranha);
        // 血の匂いをピラニアの右側に置く(獲物本体は無し=chase_targetは常にNoneのまま)。
        sim.blood_scents.push(BloodScent {
            x: 40.0 + PIRANHA_BLOOD_SCENT_RADIUS - 5.0,
            y: 20.0,
            life: BLOOD_SCENT_LIFETIME,
        });

        sim.update(0.05, 80, 40);

        assert!(
            sim.fish[0].vx > 5.0,
            "満腹中・クールダウン中でも血の匂いの方向(+x)へはっきり加速するはず: vx={}",
            sim.fish[0].vx
        );
    }

    #[test]
    fn piranha_first_bite_wounds_prey_without_killing() {
        // 1発目の噛みつきは獲物を殺さず、被噛みつき回数を1にするだけのはず。
        let mut sim = Simulation::new(Rng::new(140));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0));

        land_one_piranha_bite(&mut sim);

        assert_eq!(sim.fish[1].piranha_bite_count, 1, "1発目で被噛みつき回数が1になるはず");
        assert!(!sim.fish[1].dead, "1発目では死なないはず");
    }

    #[test]
    fn piranha_second_bite_brings_prey_to_two_still_alive() {
        // 2発目で被噛みつき回数が2になり、まだ死なないはず。
        let mut sim = Simulation::new(Rng::new(141));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0));

        land_one_piranha_bite(&mut sim);
        land_one_piranha_bite(&mut sim);

        assert_eq!(sim.fish[1].piranha_bite_count, 2, "2発目で被噛みつき回数が2になるはず");
        assert!(!sim.fish[1].dead, "2発目でもまだ死なないはず");
    }

    #[test]
    fn piranha_third_bite_kills_prey_matching_debug_kill_state() {
        // PIRANHA_BITES_TO_KILL(3)発目でようやく死亡し、Xキー(debug_kill_random_fish)が
        // 作る死亡状態(dead=true・dead_timer=0.0)と一致するはず。
        let mut sim = Simulation::new(Rng::new(142));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0));

        for _ in 0..(PIRANHA_BITES_TO_KILL as usize) {
            land_one_piranha_bite(&mut sim);
        }

        assert!(sim.fish[1].dead, "3発目で死亡状態になるはず");
        assert_eq!(
            sim.fish[1].dead_timer, 0.0,
            "死亡直後のdead_timerはdebug_kill_random_fishと同じく0.0のはず"
        );

        // 参考: debug_kill_random_fishが作る死亡状態と同じフィールドになっていることを確認する。
        let mut sim2 = Simulation::new(Rng::new(142));
        sim2.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0));
        sim2.debug_kill_random_fish();
        assert_eq!(sim.fish[1].dead, sim2.fish[0].dead);
        assert_eq!(sim.fish[1].dead_timer, sim2.fish[0].dead_timer);
    }

    #[test]
    fn piranha_bleeding_effects_fire_on_the_first_and_second_bites_too() {
        // 血飛沫・血の滲み・水質スパイクは殺した瞬間だけでなく、1発目・2発目の
        // 噛みつきでも毎回出るはず(死亡を伴わない噛みつきでも演出は同じ)。
        let mut sim = Simulation::new(Rng::new(143));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 45.0, 20.0));

        // 1発目
        land_one_piranha_bite(&mut sim);
        assert_eq!(sim.fish[1].piranha_bite_count, 1);
        assert!(!sim.fish[1].dead);
        assert!(sim.pollution > 0.0, "1発目でも水質スパイクが出るはず");
        assert!(!sim.blood_stains.is_empty(), "1発目でも血の滲みが出るはず");
        assert!(
            sim.drop_effects.iter().any(|e| e.kind == EffectKind::Blood),
            "1発目でも血飛沫パーティクルが出るはず"
        );

        let pollution_after_first = sim.pollution;
        let stains_after_first = sim.blood_stains.len();

        // 2発目
        land_one_piranha_bite(&mut sim);
        assert_eq!(sim.fish[1].piranha_bite_count, 2);
        assert!(!sim.fish[1].dead);
        assert!(
            sim.pollution > pollution_after_first,
            "2発目でもさらに水質スパイクが乗るはず"
        );
        assert!(
            sim.blood_stains.len() > stains_after_first,
            "2発目でも血の滲みが追加されるはず"
        );
    }

    #[test]
    fn piranha_bite_wounds_heal_over_time_without_further_bites() {
        // これ以上噛まれなければ、被噛みつきはPIRANHA_BITE_RECOVER_INTERVALごとに
        // 1段階ずつ時間経過で癒えて、最終的に無傷(0)に戻るはず。
        let mut sim = Simulation::new(Rng::new(150));
        let mut neon = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        neon.hunger = MAX_HUNGER;
        neon.piranha_bite_count = 2;
        sim.fish.push(neon);

        // 1インターバル未満ではまだ回復しない(keep_fed=trueで満腹を保ち、餓死・病気を避ける)
        run(&mut sim, PIRANHA_BITE_RECOVER_INTERVAL - 5.0, 0.5, 80, 40, true);
        assert_eq!(sim.fish[0].piranha_bite_count, 2, "インターバル未満では回復しないはず");

        // 合計で1インターバルを超えると1段階回復する
        run(&mut sim, 6.0, 0.5, 80, 40, true);
        assert_eq!(sim.fish[0].piranha_bite_count, 1, "1インターバルで1段階回復するはず");

        // さらに1インターバルでもう1段階回復して無傷に戻る
        run(&mut sim, PIRANHA_BITE_RECOVER_INTERVAL + 1.0, 0.5, 80, 40, true);
        assert_eq!(sim.fish[0].piranha_bite_count, 0, "さらに1インターバルで無傷に戻るはず");
    }

    #[test]
    fn wounded_fish_swims_slower_than_an_unwounded_one() {
        // 被噛みつきが増えるほど遊泳速度倍率(speed_mult)が下がるはず(空腹段階・病気を
        // そろえて、負傷ぶんだけの差を直接確認する)。
        let make = |bites: u8| {
            let mut f = Fish::new(Species::Neon, Stage::Adult, 0.0, 0.0);
            f.hunger = 50.0; // 全個体で同じ空腹段階にそろえる
            f.sick = false;
            f.piranha_bite_count = bites;
            f
        };
        let s0 = make(0).speed_mult();
        let s1 = make(1).speed_mult();
        let s2 = make(2).speed_mult();
        assert!(s1 < s0, "1回噛まれた個体は無傷より遅いはず: s1={s1} s0={s0}");
        assert!(s2 < s1, "2回噛まれた個体はさらに遅いはず: s2={s2} s1={s1}");
    }

    #[test]
    fn wounded_fish_keeps_bleeding_a_little_until_healed() {
        // 負傷中(piranha_bite_count>0)の間は、噛まれた瞬間の血飛沫とは別に、
        // 少量の血(EffectKind::Blood)を継続的に滲ませ続けるはず。無傷の個体だけの
        // 水槽では同じ時間経過でも一切発生しないことと対比して確認する。
        let (w, h) = (80, 40);

        let mut wounded_sim = Simulation::new(Rng::new(151));
        let mut wounded = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        wounded.hunger = MAX_HUNGER;
        wounded.piranha_bite_count = 1;
        wounded_sim.fish.push(wounded);

        let mut healthy_sim = Simulation::new(Rng::new(151));
        let mut healthy = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        healthy.hunger = MAX_HUNGER;
        healthy_sim.fish.push(healthy);

        let mut saw_bleed = false;
        for _ in 0..100 {
            wounded_sim.fish[0].hunger = MAX_HUNGER;
            healthy_sim.fish[0].hunger = MAX_HUNGER;
            wounded_sim.update(0.5, w, h);
            healthy_sim.update(0.5, w, h);
            if wounded_sim.drop_effects.iter().any(|e| e.kind == EffectKind::Blood) {
                saw_bleed = true;
            }
            assert!(
                !healthy_sim.drop_effects.iter().any(|e| e.kind == EffectKind::Blood),
                "無傷の個体だけの水槽では血は出ないはず"
            );
        }
        assert!(saw_bleed, "負傷中は時間経過でいつか少量の血が出るはず");
    }

    #[test]
    fn piranha_gives_up_the_appetite_quota_after_grace_period_without_a_kill() {
        // ピラニアが食欲がなくても魚を追いかけまわし体力を減らしてしまうという指摘の
        // 回帰テスト。1匹食べた後、獲物が全くいない(捕食できない)
        // 状態が続くと、hungerは満腹相当のまま止まっているが、meals_since_fullが
        // 1のまま無限にstill_hungryが続いてしまっていた。グレースピリオドを超えたら
        // 諦めてmeals_since_fullが0に戻り、狩り続けなくなるはず。
        let mut sim = Simulation::new(Rng::new(103));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = MAX_HUNGER;
        piranha.piranha_meals_since_full = 1; // 1匹食べた直後、まだクォータ進行中
        sim.fish.push(piranha);
        // 獲物は置かない(捕食できない状況を再現する)

        run(&mut sim, PIRANHA_QUOTA_GRACE_PERIOD + 1.0, 1.0, 80, 40, false);

        assert_eq!(
            sim.fish[0].piranha_meals_since_full, 0,
            "グレースピリオドを超えたらクォータは放棄され0に戻るはず"
        );
    }

    #[test]
    fn piranha_still_within_grace_period_keeps_the_appetite_quota() {
        // グレースピリオド内であれば、まだクォータ(meals_since_full)は保持され続け、
        // 途中で不用意にリセットされないことを確認する(タイマーの取り違え防止)。
        let mut sim = Simulation::new(Rng::new(104));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = MAX_HUNGER;
        piranha.piranha_meals_since_full = 2;
        sim.fish.push(piranha);

        run(&mut sim, PIRANHA_QUOTA_GRACE_PERIOD - 5.0, 1.0, 80, 40, false);

        assert_eq!(
            sim.fish[0].piranha_meals_since_full, 2,
            "グレースピリオド内はクォータが保持されるはず"
        );
    }

    #[test]
    fn piranha_that_never_hunted_stays_full_without_eating() {
        // 既に満腹(hunger=MAX_HUNGER)で、まだ一度も捕食していないピラニアは、
        // 新しいpiranha_meals_since_fullロジックの影響を受けず、これまで通り
        // 狩りに入らないはず(meals_since_full=0のデフォルト値が誤って
        // 「まだ狩りをやめない」判定に使われないことの確認)。
        let mut sim = Simulation::new(Rng::new(102));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = MAX_HUNGER;
        sim.fish.push(piranha);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 41.0, 20.0));

        sim.update(3.0, 80, 40);

        assert_eq!(sim.fish.len(), 2, "満腹で未捕食のピラニアは狩らないはず");
    }

    #[test]
    fn full_piranha_does_not_bite_meat() {
        // 方針転換の回帰テスト: 「満腹のときは肉餌とはいえ食いつかないものとする」。
        // 満腹(hunger=MAX_HUNGER・未捕食でmeals_since_full=0)のピラニアは肉餌を
        // 無視し、肉餌はそのまま残るはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(200));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = MAX_HUNGER;
        sim.fish.push(piranha);
        sim.meat.push(Meat {
            x: 42.0,
            y: 20.0,
            vy: 0.0,
            life: MEAT_LIFETIME,
            landed: false,
            sway_phase: 0.0,
        });

        sim.update(3.0, w, h);

        assert_eq!(sim.meat.len(), 1, "満腹のピラニアは肉餌に食いつかないはず");
        // 肉餌を無視して通常の空腹度減衰だけが進むはず(食べていれば即MAX_HUNGERに
        // 戻るが、無視していれば3秒分のHUNGER_DECAYがそのまま乗るだけのはず)。
        assert!(sim.fish[0].hunger >= PIRANHA_HUNT_HUNGER_THRESHOLD);
        assert!(sim.fish[0].hunger < MAX_HUNGER);
    }

    #[test]
    fn hungry_piranha_bites_meat_and_becomes_full() {
        // 空腹なピラニアは肉餌に確実に食いつき、満腹になるはず
        // (生きた獲物のような逃走・隠れ身がないため確実に捕食成立する)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(200));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);
        sim.meat.push(Meat {
            x: 42.0,
            y: 20.0,
            vy: 0.0,
            life: MEAT_LIFETIME,
            landed: false,
            sway_phase: 0.0,
        });

        for _ in 0..30 {
            sim.update(0.1, w, h);
            if sim.meat.is_empty() {
                break;
            }
        }

        assert!(sim.meat.is_empty(), "空腹なピラニアは肉餌に確実に食いつくはず");
        assert!(sim.fish[0].hunger >= PIRANHA_HUNT_HUNGER_THRESHOLD, "食べたら満腹になるはず");
        assert!(
            sim.message.as_deref().unwrap_or("").contains("肉餌"),
            "肉餌を食べたことを示すメッセージが表示されるはず"
        );
    }

    #[test]
    fn large_piranha_can_still_eat_meat_landed_in_a_bottom_corner() {
        // 大きく成長したピラニアが底面や壁際に堆積した肉餌を食べられないという指摘の
        // 回帰テスト。最大まで育った(拡大サイズの)
        // ピラニアでも、水底の角(壁際かつ底)に着地した肉餌を確実に食べられるはず。
        let (w, h) = (80, 40);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let mut sim = Simulation::new(Rng::new(300));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 6.0, sand_top - 1.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        piranha.growth_stage = GENERAL_MAX_GROWTH_STAGE; // 全種共通の成長も最大まで育った
        piranha.kill_stage = PIRANHA_MAX_KILL_STAGE; // 捕食成長も最大(render_scaleが最も大きい状態)
        sim.fish.push(piranha);
        // 水底の角(壁際)に着地済みの肉餌を置く
        sim.meat.push(Meat {
            x: 3.0,
            y: sand_top,
            vy: 0.0,
            life: MEAT_LIFETIME,
            landed: true,
            sway_phase: 0.0,
        });

        for _ in 0..50 {
            sim.update(0.1, w, h);
            if sim.meat.is_empty() {
                break;
            }
        }

        assert!(
            sim.meat.is_empty(),
            "拡大した大きいピラニアでも壁際・底の肉餌を確実に食べられるはず"
        );
    }

    #[test]
    fn large_piranha_can_still_eat_live_prey_cornered_at_the_wall() {
        // 肉餌と同種の防御的テスト: 最大まで育ったピラニアは、口基準の距離だと
        // 大きな体格ゆえに獲物を飛び越えて逆に遠いと誤判定してしまうことがある
        // (中心はほぼ密着していても口だけ突き抜けてしまうケース)。中心・口の
        // 近い方を使うことで、生きた獲物でも壁際で確実に捕食できることを確認する。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(301));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 5.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        piranha.growth_stage = GENERAL_MAX_GROWTH_STAGE;
        piranha.kill_stage = PIRANHA_MAX_KILL_STAGE;
        sim.fish.push(piranha);
        // 壁際(x=4付近)に獲物を置く。ピラニアの体格が大きいため中心同士は
        // ほぼ密着しているが、口基準の距離だけでは飛び越えてしまう位置関係。
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 4.0, 20.0));

        // 1発では死なずPIRANHA_BITES_TO_KILL発で死ぬので、空腹・クールダウン明けを毎tick維持して
        // 連続で噛ませ、死亡まで到達させる。
        for _ in 0..50 {
            sim.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.fish[0].predation_cooldown = 0.0;
            sim.update(0.1, w, h);
            if sim.fish[1].dead {
                break;
            }
        }

        // ピラニアの捕食は即消滅させず死骸を残すため、獲物(index 1)が死亡状態に
        // なったことで「捕食が成立した」ことを確認する。
        assert_eq!(
            sim.fish.len(),
            2,
            "捕食後も獲物は死骸として残るはず"
        );
        assert!(
            sim.fish[1].dead,
            "拡大した大きいピラニアでも壁際の獲物を確実に捕食できるはず"
        );
    }

    #[test]
    fn eating_meat_resets_piranha_meals_since_full_counter() {
        // 肉餌を与えると、旺盛な食欲ロジックの捕食カウンタもリセットされ、
        // 生きた魚を横取りハンティングし続ける状態を止められるはず(救済ツール)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(201));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = MAX_HUNGER;
        piranha.piranha_meals_since_full = 2; // 3匹目に達する前、まだ狩り続けている状態
        sim.fish.push(piranha);
        sim.meat.push(Meat {
            x: 42.0,
            y: 20.0,
            vy: 0.0,
            life: MEAT_LIFETIME,
            landed: false,
            sway_phase: 0.0,
        });

        for _ in 0..30 {
            sim.update(0.1, w, h);
            if sim.meat.is_empty() {
                break;
            }
        }

        assert_eq!(
            sim.fish[0].piranha_meals_since_full, 0,
            "肉餌を食べたらカウンタは0にリセットされるはず"
        );
        // 食べた直後は満腹相当だが、同tick内の通常の空腹度減衰(HUNGER_DECAY)が
        // 後段で乗るため、厳密にMAX_HUNGERと一致するとは限らない。
        assert!(sim.fish[0].hunger >= PIRANHA_HUNT_HUNGER_THRESHOLD);
    }

    #[test]
    fn non_piranha_species_never_eat_meat() {
        // 「ピラニア以外の魚は一切食べない」の回帰テスト。通常種・タコとも
        // 肉餌のすぐ近くに置いても、消費されず・空腹度も回復しないはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(202));
        let mut neon = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        neon.hunger = 10.0; // 腹ぺこにしておき、もし食べたら回復が分かるようにする
        sim.fish.push(neon);
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 21.0);
        octo.hunger = 10.0;
        sim.fish.push(octo);
        sim.meat.push(Meat {
            x: 40.0,
            y: 20.5,
            vy: 0.0,
            life: MEAT_LIFETIME,
            landed: false,
            sway_phase: 0.0,
        });

        sim.update(1.0, w, h);

        assert_eq!(sim.meat.len(), 1, "ピラニア以外は肉餌を消費しないはず");
        assert!(sim.fish[0].hunger < 15.0, "ネオンは肉餌で空腹度が回復しないはず");
        assert!(sim.fish[1].hunger < 15.0, "タコは肉餌で空腹度が回復しないはず");
    }

    #[test]
    fn falling_meat_sways_left_and_right_instead_of_dropping_straight_down() {
        // 肉餌もFood/Medicineと同様に蛇行しながら沈むことを確認する。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(7));
        sim.meat.push(Meat {
            x: 40.0,
            y: 5.0,
            vy: MEAT_SINK_SPEED,
            life: MEAT_LIFETIME,
            landed: false,
            sway_phase: 0.0,
        });
        let start_x = sim.meat[0].x;
        sim.update(0.2, w, h);
        assert!(
            !sim.meat.is_empty() && !sim.meat[0].landed,
            "この時間では着地しないはず(テスト前提の確認)"
        );
        assert_ne!(
            sim.meat[0].x, start_x,
            "沈降中の肉餌は左右に揺れてxが変化するはず(直下降ではない)"
        );
    }

    #[test]
    fn blood_stain_fades_out_and_disappears_after_its_lifetime() {
        // 血の滲みはパーティクルより長く残り(3〜5秒程度)、その後は消える。
        let mut sim = Simulation::new(Rng::new(420));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        let (mx, my) = piranha.mouth_position();
        sim.fish.push(piranha);
        // 当たり判定を胴体でなく口にすべきという指摘への対応: 口の位置に配置する
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
            // 当たり判定を胴体でなく口にすべきという指摘への対応: 捕食判定は口
            // (進行方向側のスプライト前端)基準になったため、口の現在位置に配置する
            // (捕食成長でピラニアが大きくなるたびに口の位置も変わるため、都度計算する)。
            let (mx, my) = sim.fish[0].mouth_position();
            sim.fish.push(Fish::new(Species::Neon, Stage::Adult, mx, my));
            // 基本移動速度を全体的に4倍にする方針への対応で移動時の加速度も
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
        // 当たり判定を胴体でなく口にすべきという指摘への対応: 口の位置に配置する
        let (mx, my) = big_piranha.mouth_position();
        sim.fish.push(big_piranha);
        // サイズ指標0(成長段階・捕食成長段階ともに0)の小さいピラニア
        sim.fish.push(Fish::new(Species::Piranha, Stage::Adult, mx, my));

        // 1発では死なずPIRANHA_BITES_TO_KILL発で死ぬので、空腹・クールダウン明けを毎tick維持して
        // 連続で噛ませ、死亡まで到達させる。
        for _ in 0..50 {
            sim.fish[0].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.fish[0].predation_cooldown = 0.0;
            sim.update(0.1, 80, 40);
            if sim.fish[1].dead {
                break;
            }
        }

        // ピラニアの捕食(共食い含む)は即消滅させず死骸を残すため、小さい方(index 1)が
        // 死亡状態になり、大きい方(index 0)が生きて残ることを確認する。
        assert_eq!(
            sim.fish.len(),
            2,
            "共食い後も獲物は死骸として残るはず"
        );
        assert!(
            sim.fish[1].dead,
            "十分サイズ差のある大きいピラニアは、小さいピラニアを捕食してよいはず"
        );
        assert_eq!(sim.fish[0].species, Species::Piranha);
        assert!(!sim.fish[0].dead, "大きい方のピラニアは生きて残るはず");
        assert_eq!(
            sim.fish[0].growth_stage, GENERAL_MAX_GROWTH_STAGE,
            "生き残るのは大きい方のピラニアのはず"
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
            sway_phase: 0.0,
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
            // 100 * 0.1 = 10秒。GROW_TIMEより十分短く、稚魚が成魚化しない範囲で
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

        // さらにCORPSE_REMOVE_TIME(約24時間)を超えて放置すると水槽から消える
        // (粗いdtでも死亡演出中は物理計算が安定しているため問題ない)
        run(&mut sim, CORPSE_REMOVE_TIME + 5.0, 200.0, 80, 40, false);
        assert_eq!(sim.fish_count(), 0, "CORPSE_REMOVE_TIMEを超えたら水槽から消えるはず");
    }

    #[test]
    fn dead_fish_floats_upward_and_stops_near_surface() {
        // 死亡演出中は浮力(時間とともに減衰)と重力・抵抗の力学計算で、水面近くまで
        // ゆっくり浮上して静止することを確認する。左右にはゆらゆらと揺れる
        // (揺れ幅DEAD_SWAY_AMPLITUDE以内)が、大きく横移動はしない。
        let mut sim = Simulation::new(Rng::new(46));
        let mut fish = Fish::new(Species::Goldfish, Stage::Adult, 20.0, 30.0);
        fish.dead = true;
        fish.dead_timer = 0.0;
        let start_y = fish.y;
        let start_x = fish.x;
        sim.fish.push(fish);
        // 浮上して水面近くで静止するのに十分な時間だけ進める(沈降が始まる目安=
        // DEAD_FLOAT_TIMEよりかなり手前)
        for _ in 0..200 {
            sim.update(0.1, 80, 40);
        }
        assert!(sim.fish[0].y < start_y, "死亡後は水面へ向けて浮上するはず");
        assert!(
            sim.fish[0].y <= DEAD_SURFACE_MARGIN + 0.5,
            "水面近くまで浮上したら静止するはず: y={}",
            sim.fish[0].y
        );
        assert!(
            (sim.fish[0].x - start_x).abs() <= DEAD_SWAY_AMPLITUDE + 0.5,
            "左右にはゆらゆらと揺れる程度で、大きくは横移動しないはず"
        );
    }

    #[test]
    fn knocking_near_a_floating_corpse_forces_it_to_sink() {
        // 浮いている死骸をカーソル近くで叩く(つつく)と、以降は浮力を無視して
        // 重力だけで沈み始める要望への回帰テスト。叩かなかった場合と比べて、
        // 同じ時間経過後により深く(yが大きく)沈んでいるはず。
        let (w, h) = (80, 40);
        let make_sim = |knocked: bool| {
            let mut sim = Simulation::new(Rng::new(4290));
            let mut fish = Fish::new(Species::Goldfish, Stage::Adult, 20.0, 30.0);
            fish.dead = true;
            fish.dead_timer = 0.0; // 浮力がまだ強い直後
            sim.fish.push(fish);
            if knocked {
                sim.knock(20.0, 30.0, w, h);
            }
            sim
        };
        let mut knocked = make_sim(true);
        let mut not_knocked = make_sim(false);
        assert!(knocked.fish[0].sink_forced, "叩いた死骸にはsink_forcedが立つはず");
        assert!(!not_knocked.fish[0].sink_forced);

        for _ in 0..30 {
            knocked.update(0.1, w, h);
            not_knocked.update(0.1, w, h);
        }
        assert!(
            knocked.fish[0].y > not_knocked.fish[0].y,
            "叩いた死骸は叩いていない死骸より早く沈んでいるはず(knocked.y={} not_knocked.y={})",
            knocked.fish[0].y,
            not_knocked.fish[0].y
        );
    }

    #[test]
    fn knocking_a_settled_corpse_is_a_harmless_no_op() {
        // 既に水底に沈み切った死骸を叩いてもsink_forcedは立つが、位置には影響しない
        // (既に沈んでいるため無害な無操作になる)ことの回帰テスト。
        let (w, h) = (80, 40);
        let sand_top = h as f64 - sand_height(h) as f64;
        let mut sim = Simulation::new(Rng::new(4291));
        let mut fish = Fish::new(Species::Goldfish, Stage::Adult, 20.0, sand_top - 1.0);
        fish.dead = true;
        fish.dead_timer = CORPSE_REMOVE_TIME / 2.0; // 十分に沈み切っている想定
        sim.fish.push(fish);
        let y_before = sim.fish[0].y;

        sim.knock(20.0, sand_top - 1.0, w, h);
        assert!(sim.fish[0].sink_forced);

        for _ in 0..30 {
            sim.update(0.1, w, h);
        }
        assert!(
            (sim.fish[0].y - y_before).abs() < 0.5,
            "沈み切った死骸を叩いても位置はほぼ変わらないはず"
        );
    }

    #[test]
    fn settled_corpse_at_the_bottom_stops_swaying() {
        // 水底に沈み切って静止した亡骸は、堆積物と同様にゆらゆら揺れず静かに
        // 横たわったままになることの回帰テスト。
        let (w, h) = (80, 40);
        let sand_top = h as f64 - sand_height(h) as f64;
        let mut sim = Simulation::new(Rng::new(48));
        let mut fish = Fish::new(Species::Goldfish, Stage::Adult, 20.0, sand_top - 1.0);
        fish.dead = true;
        fish.dead_timer = CORPSE_REMOVE_TIME / 2.0; // 十分に沈み切っている想定
        sim.fish.push(fish);

        sim.update(0.1, w as usize, h as usize); // 沈降がまだ続くよう1tickだけ進めて着地させる
        let settled_x = sim.fish[0].x;
        for _ in 0..50 {
            sim.update(0.1, w as usize, h as usize);
        }
        assert_eq!(
            sim.fish[0].x, settled_x,
            "水底に着地した亡骸は揺れずx座標が変化しないはず"
        );
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
            sway_phase: 0.0,
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
        // 産卵時にキラキラ光るフラッシュ演出を追加してほしいという要望への対応を確認する:
        // 産卵と同時にSpawn種のDropEffectが出るはず。
        assert!(
            sim.drop_effects.iter().any(|e| e.kind == EffectKind::Spawn),
            "産卵時にキラキラ演出(Spawn)のDropEffectが出るはず"
        );
    }

    #[test]
    fn spawn_flash_has_the_expected_lifetime_and_position() {
        // 産卵にはつがい(同種2匹がMATE_RADIUS以内)が必要なので、隣接させて配置する。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(717));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 40.0, 30.0));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 41.0, 30.0));
        let mut saw_flash = false;
        // 確率的なイベントを待つテストのため、他の箇所の変更でtickごとの乱数消費数が
        // 変わってもタイミングがずれにくいよう、十分大きめの試行回数にしてある
        // (40000では無関係なコード変更(スプライトの静的データ追加等)でも境界を
        // 越えてしまう実例があったため、さらに余裕を持たせた)。
        for _ in 0..100000 {
            for f in &mut sim.fish {
                f.hunger = MAX_HUNGER;
                f.well_fed_timer = BREED_READY_TIME + 5.0;
            }
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
    fn courtship_pulls_ready_same_species_fish_closer_together() {
        // COURTSHIP_RADIUS以内・MATE_RADIUSより遠い産卵可能な同種2匹は、
        // 交尾に向けて互いに引き寄せられる(接近する)はず。
        let mut sim = Simulation::new(Rng::new(3));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 10.0, 10.0));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 20.0, 10.0));
        for f in &mut sim.fish {
            f.well_fed_timer = BREED_READY_TIME + 5.0;
        }
        let dist_before = sim.fish[1].x - sim.fish[0].x;
        assert!(dist_before < COURTSHIP_RADIUS && dist_before > MATE_RADIUS);
        sim.update_courtship(0.5);
        assert!(
            sim.fish[0].vx > 0.0,
            "左側の魚は右側の相手に向かって速度がつくはず"
        );
        assert!(
            sim.fish[1].vx < 0.0,
            "右側の魚は左側の相手に向かって速度がつくはず"
        );
    }

    #[test]
    fn courtship_ignores_different_species_and_out_of_range_fish() {
        // 種が違う、またはCOURTSHIP_RADIUSより遠い相手には引き寄せられない。
        let mut sim = Simulation::new(Rng::new(3));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 10.0, 10.0));
        sim.fish
            .push(Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 10.0 + COURTSHIP_RADIUS * 2.0, 10.0));
        for f in &mut sim.fish {
            f.well_fed_timer = BREED_READY_TIME + 5.0;
        }
        sim.update_courtship(0.5);
        for f in &sim.fish {
            assert_eq!(f.vx, 0.0, "対象になる同種の近い相手がいなければ速度は変化しないはず");
        }
    }

    #[test]
    fn breeding_pair_within_mate_radius_eventually_mates_and_resets_both_timers() {
        // MATE_RADIUS以内に同種のつがいがいると、確率判定を経てやがて交尾成立する。
        // 成立時: Mate演出が出る・両方のwell_fed_timerが0になる・中間地点付近に卵ができる。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(9));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 40.0, 30.0));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 42.0, 30.0));
        let mut mated = false;
        // 確率的なイベントを待つテストのため、他の箇所の変更でtickごとの乱数消費数が
        // 変わってもタイミングがずれにくいよう、十分大きめの試行回数にしてある。
        for _ in 0..40000 {
            for f in &mut sim.fish {
                f.hunger = MAX_HUNGER;
                f.well_fed_timer = BREED_READY_TIME + 5.0;
            }
            sim.update(0.1, w, h);
            if sim.drop_effects.iter().any(|e| e.kind == EffectKind::Mate) {
                mated = true;
                break;
            }
        }
        assert!(mated, "十分な時間が経てば交尾演出(Mate)が出るはず");
        assert!(
            !sim.eggs.is_empty(),
            "交尾成立と同時に卵が産まれているはず"
        );
        // 交尾が成立した両個体には交尾経験フラグ(has_mated)が立つはず。
        assert!(
            sim.fish[0].has_mated && sim.fish[1].has_mated,
            "交尾したつがいの両方にhas_matedが立つはず"
        );
        // 交尾したのは水面近く(y=30)だが、卵は常に水底付近に産まれる仕様のため、
        // ハート演出も交尾した実際の位置ではなく卵と同じ水底付近に出るはず
        // (そうしないと演出と卵の位置がズレて不自然に見える)。
        let mate_effect = sim
            .drop_effects
            .iter()
            .find(|e| e.kind == EffectKind::Mate)
            .expect("Mate演出が残っているはず");
        let sand_top = h as f64 - sand_height(h) as f64;
        assert!(
            (mate_effect.y - (sand_top - 1.5)).abs() < 0.01,
            "ハート演出は水底付近(卵と同じ座標)に出るはず(実際のy: {})",
            mate_effect.y
        );
    }

    #[test]
    fn a_fish_cannot_be_paired_with_two_partners_in_the_same_tick() {
        // MATE_RADIUS以内に同種3匹が密集していても、1tickで成立するつがいは
        // 最大1組(1匹が同時に2組へ重複参加しない)。
        let mut sim = Simulation::new(Rng::new(4));
        for i in 0..3 {
            let mut f = Fish::new(Species::Guppy, Stage::Adult, 40.0 + i as f64, 30.0);
            f.hunger = MAX_HUNGER;
            f.well_fed_timer = BREED_READY_TIME + 5.0;
            sim.fish.push(f);
        }
        let mut spawn_eggs = Vec::new();
        sim.update_breeding_pairs(0.1, &mut spawn_eggs);
        assert!(
            spawn_eggs.len() <= 1,
            "3匹しかいないので同時に成立するつがいは最大1組のはず"
        );
    }

    #[test]
    fn hatching_egg_emits_hatch_effect_with_expected_lifetime() {
        // 孵化の瞬間にHatch種のDropEffectが出るはず(産卵時のSpawnとは別演出)。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(11));
        sim.eggs.push(Egg {
            x: 40.0,
            y: 90.0,
            species: Species::Neon,
            hatch: 0.05,
        });
        sim.update(0.1, w, h);
        let hatch_effect = sim
            .drop_effects
            .iter()
            .find(|e| e.kind == EffectKind::Hatch);
        assert!(hatch_effect.is_some(), "孵化した瞬間にHatch演出が出るはず");
        assert_eq!(
            hatch_effect.unwrap().max_life,
            HATCH_EFFECT_LIFETIME,
            "Hatch演出の持続時間は仕様どおりのはず"
        );
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
    fn egg_fails_to_hatch_when_pollution_and_purifier_are_both_severe() {
        // 水質最悪(pollution=POLLUTION_MAX)かつ浄化剤濃度100%(purifier_concentration=1.0)だと
        // EGG_HATCH_FAIL_POLLUTION_MAX_CHANCE(0.6)+EGG_HATCH_FAIL_PURIFIER_MULT(0.5)=1.1が
        // 1.0で頭打ちになり、必ず孵化に失敗する。乱数に関わらず決定的に確認できる。
        let (w, h) = (200, 100); // 上限による孵化阻害とは切り分けるため十分広くしておく
        let mut sim = Simulation::new(Rng::new(960));
        sim.pollution = POLLUTION_MAX;
        sim.purifier_concentration = 1.0;
        let before = sim.fish_count();
        sim.eggs.push(Egg {
            x: 40.0,
            y: 90.0,
            species: Species::Neon,
            hatch: 0.05,
        });
        sim.update(0.1, w, h);
        assert_eq!(sim.fish_count(), before, "水質最悪+浄化剤濃度100%では孵化に失敗して増えないはず");
        assert!(sim.eggs.is_empty(), "孵化に失敗した卵も消える");
        assert!(
            sim.message.as_deref().unwrap_or("").contains("孵化できなかった"),
            "孵化失敗のメッセージが出るはず: {:?}",
            sim.message
        );
    }

    #[test]
    fn egg_hatches_normally_when_water_is_clean_even_with_no_purifier() {
        // 水質が綺麗(pollution=0)・浄化剤未使用(purifier_concentration=0)なら
        // 孵化失敗確率は0のはずで、これまでどおり確実に孵化する。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(961));
        sim.pollution = 0.0;
        sim.purifier_concentration = 0.0;
        let before = sim.fish_count();
        sim.eggs.push(Egg {
            x: 40.0,
            y: 90.0,
            species: Species::Neon,
            hatch: 0.05,
        });
        sim.update(0.1, w, h);
        assert_eq!(sim.fish_count(), before + 1, "水質が綺麗なら確実に孵化するはず");
        assert!(sim.eggs.is_empty());
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
            sway_phase: 0.0,
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
            sway_phase: 0.0,
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
            sway_phase: 0.0,
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

        run(&mut sim, CORPSE_REMOVE_TIME + 5.0, 200.0, 80, 40, false);
        assert_eq!(sim.fish_count(), 0, "CORPSE_REMOVE_TIMEを超えたら水槽から消えるはず");
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
    fn disabling_a_species_excludes_it_from_add_fish_and_seed_initial() {
        // 生成対象の種を切り替えられる機能の回帰テスト。
        // ネオン以外を全てOFFにすれば、add_fish/seed_initialはネオンしか選ばないはず。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(500));
        // Species::COMMON = [Neon, Goldfish, Guppy, Angelfish, Betta]
        sim.species_toggle = [true, false, false, false, false];

        sim.seed_initial(w, h);
        assert!(!sim.fish.is_empty(), "テスト前提: 初期個体が存在すること");
        assert!(
            sim.fish.iter().all(|f| f.species == Species::Neon),
            "OFFにした種はseed_initialで選ばれないはず"
        );

        for _ in 0..10 {
            sim.add_fish(w, h);
        }
        assert!(
            sim.fish.iter().all(|f| f.species == Species::Neon),
            "OFFにした種はadd_fishでも選ばれないはず"
        );
    }

    #[test]
    fn disabling_all_species_falls_back_to_full_pool() {
        // 全種OFFにした場合、空の抽選プールで固まらず通常5種全部にフォールバックするはず。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(501));
        sim.species_toggle = [false; 5];

        sim.seed_initial(w, h);
        assert!(
            !sim.fish.is_empty(),
            "全種OFFでもseed_initialはフォールバックして個体を配置するはず"
        );
    }

    #[test]
    fn toggle_common_species_flips_the_flag_at_the_given_index() {
        let mut sim = Simulation::new(Rng::new(502));
        assert!(sim.species_toggle[1]);
        sim.toggle_common_species(1);
        assert!(!sim.species_toggle[1]);
        sim.toggle_common_species(1);
        assert!(sim.species_toggle[1]);
        // 範囲外のindexは何もしない(パニックしない)
        sim.toggle_common_species(99);
    }

    #[test]
    fn seed_initial_places_same_species_pairs_close_together() {
        // 初期配置は種ごとに同種の成魚2匹を「つがい」として撒く。プールにいる各種について
        // 成魚が2匹以上おり、かつ同種の2匹は互いに近く(求愛範囲COURTSHIP_RADIUSより十分
        // 内側)に配置されるはず。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(4242));
        sim.seed_initial(w, h);
        assert!(!sim.fish.is_empty(), "テスト前提: 初期個体が存在すること");

        let pool = sim.spawn_pool();
        for &sp in &pool {
            let members: Vec<(f64, f64)> = sim
                .fish
                .iter()
                .filter(|f| f.species == sp && f.stage == Stage::Adult)
                .map(|f| (f.x, f.y))
                .collect();
            assert!(
                members.len() >= 2,
                "{:?} は成魚が2匹以上いるはず(実際: {})",
                sp,
                members.len()
            );
            // 同種の全ペア間距離が求愛範囲より十分小さいこと(つがいが最初から近接している)。
            for a in 0..members.len() {
                for b in (a + 1)..members.len() {
                    let d = ((members[a].0 - members[b].0).powi(2)
                        + (members[a].1 - members[b].1).powi(2))
                    .sqrt();
                    assert!(
                        d < COURTSHIP_RADIUS,
                        "同種のつがいは求愛範囲より近くに配置されるはず(実際の距離: {d})"
                    );
                }
            }
        }
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
    fn auto_replenish_adds_a_fish_when_common_species_are_scarce() {
        // 捕食する種(ピラニア・タコ)ばかりだと魚がいなくなるという指摘への
        // 対応: 通常魚(捕食対象になる種)がAUTO_REPLENISH_THRESHOLD以下なら、
        // 自動でadd_fish相当が1匹補充されるはず。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(730));
        // 水槽にはピラニア・タコだけがいる(通常魚は0匹)状態を作る
        sim.fish.push(Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0));
        sim.fish.push(Fish::new(Species::Octopus, Stage::Adult, 60.0, 20.0));
        let count_before = sim.fish.len();

        sim.update_auto_replenish(0.1, w, h);

        assert_eq!(
            sim.fish.len(),
            count_before + 1,
            "通常魚が0匹(閾値以下)なら自動で1匹補充されるはず"
        );
        let added = sim
            .fish
            .iter()
            .find(|f| f.species != Species::Piranha && f.species != Species::Octopus);
        assert!(added.is_some(), "補充されるのは通常種(COMMON)のはず");
    }

    #[test]
    fn auto_replenish_does_nothing_when_common_species_are_plentiful() {
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(731));
        for i in 0..10 {
            sim.fish
                .push(Fish::new(Species::Neon, Stage::Adult, 40.0 + i as f64, 20.0));
        }
        let count_before = sim.fish.len();

        sim.update_auto_replenish(0.1, w, h);

        assert_eq!(
            sim.fish.len(),
            count_before,
            "通常魚が十分いるなら自動補充は起きないはず"
        );
    }

    #[test]
    fn auto_replenish_respects_a_cooldown_between_additions() {
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(732));
        // 通常魚0匹の状態を維持する(補充された分は毎tick除去して、閾値以下を保つ)
        sim.update_auto_replenish(0.1, w, h);
        let count_after_first = sim.fish.len();
        assert!(count_after_first > 0, "1回目で1匹補充されるはず");

        // クールダウン中はすぐには追加補充されないはず
        sim.update_auto_replenish(0.1, w, h);
        assert_eq!(
            sim.fish.len(),
            count_after_first,
            "クールダウン中は連続で補充されないはず"
        );

        // 十分な時間(クールダウン+余裕)が経てば、まだ閾値以下なら再度補充されるはず
        // (update_auto_replenishはsim.update()の外側から呼ぶ想定の関数のため、
        // run()ヘルパーではなく直接ループで時間を進める)
        let mut t = 0.0;
        while t < AUTO_REPLENISH_COOLDOWN + 1.0 {
            sim.update_auto_replenish(0.5, w, h);
            t += 0.5;
        }
        assert!(
            sim.fish.len() > count_after_first,
            "クールダウンが明ければ再度補充されるはず"
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
        // (確率アップではなく一度きりの確定イベント)。ただし一度でもつがいの交尾を
        // 経験した個体(has_mated)に限るので、ここではhas_matedを立てておく。
        let mut sim = Simulation::new(Rng::new(306));
        let mut f = Fish::new(Species::Neon, Stage::Fry, 20.0, 10.0); // 稚魚・満腹でもない
        f.hunger = 5.0; // 満腹条件を問わないことを確認するため、あえて腹ぺこにしておく
        f.age = ELDERLY_AGE - 0.05;
        f.has_mated = true; // 交尾経験あり: 老齢確定産卵の対象
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
    fn elderly_fish_that_never_mated_does_not_get_the_bonus_spawn() {
        // 一度もつがいの交尾を経験していない個体(has_mated=false)は、老齢に達しても
        // 最後の確定産卵を行わない(卵が現れず、確定産卵フラグも立たないまま)。
        let mut sim = Simulation::new(Rng::new(3061));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0);
        f.hunger = MAX_HUNGER;
        f.age = ELDERLY_AGE - 0.05;
        // has_mated は Fish::new のデフォルト(false)のまま = 交尾経験なし
        sim.fish.push(f);
        assert!(sim.eggs.is_empty());

        for _ in 0..5 {
            sim.update(0.1, 80, 40); // ELDERLY_AGE を跨ぐ
        }

        assert!(
            !sim.fish[0].elderly_spawned,
            "交尾経験のない個体は老齢確定産卵の対象外なのでフラグは立たないはず"
        );
        assert!(
            sim.eggs.is_empty(),
            "交尾経験のない個体は老齢でも卵を産まないはず: eggs={}",
            sim.eggs.len()
        );
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
    fn debug_starve_all_sets_hunger_to_zero_for_living_fish_only() {
        // 全個体を一括で空腹にするデバッグショートカットの回帰テスト。
        // 生きている個体は空腹度0になり、死亡演出中の個体は対象外(既に育成
        // ロジックから外れているため、hungerを触っても意味がない)。
        let mut sim = Simulation::new(Rng::new(910));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 10.0, 10.0));
        sim.fish.push(Fish::new(Species::Goldfish, Stage::Adult, 20.0, 10.0));
        sim.fish[1].dead = true;
        sim.fish[1].hunger = MAX_HUNGER; // 死亡演出中はそのままのはず

        sim.debug_starve_all();

        assert_eq!(sim.fish[0].hunger, 0.0, "生きている個体は空腹度0になるはず");
        assert_eq!(
            sim.fish[1].hunger, MAX_HUNGER,
            "死亡演出中の個体は対象外のはず"
        );
    }

    #[test]
    fn debug_force_courtship_proximity_mates_ready_same_species_pairs_instantly() {
        // 交尾成立までの接近待ち・確率判定を飛ばして動作確認できるデバッグ用
        // ショートカットの回帰テスト。産卵可能な同種2匹を遠く離した状態から呼ぶと、
        // 確率判定を待たずに即座に交尾成立(ハート演出+卵)まで完了するはず。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(31));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 10.0, 10.0));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 90.0, 10.0));
        for f in &mut sim.fish {
            f.well_fed_timer = BREED_READY_TIME + 5.0;
        }

        sim.debug_force_courtship_proximity(w, h);

        let d = ((sim.fish[1].x - sim.fish[0].x).powi(2) + (sim.fish[1].y - sim.fish[0].y).powi(2)).sqrt();
        assert!(d < MATE_RADIUS, "呼び出し後はMATE_RADIUS以内まで近づくはず(実際: {d})");
        let mate_effect = sim
            .drop_effects
            .iter()
            .find(|e| e.kind == EffectKind::Mate)
            .expect("確率判定を待たずにハート演出(Mate)が出るはず");
        assert!(!sim.eggs.is_empty(), "確率判定を待たずに卵が産まれるはず");
        // 交尾したのは水面近く(y=10)だが、ハート演出は卵と同じ水底付近に出るはず
        // (演出と卵の位置がズレないようにするため)。
        let sand_top = h as f64 - sand_height(h) as f64;
        assert!(
            (mate_effect.y - (sand_top - 1.5)).abs() < 0.01,
            "ハート演出は水底付近(卵と同じ座標)に出るはず(実際のy: {})",
            mate_effect.y
        );
        assert_eq!(sim.fish[0].well_fed_timer, 0.0, "交尾した個体のwell_fed_timerはリセットされるはず");
        assert_eq!(sim.fish[1].well_fed_timer, 0.0, "交尾した個体のwell_fed_timerはリセットされるはず");
    }

    #[test]
    fn debug_force_courtship_proximity_ignores_fish_not_ready_to_breed() {
        // 産卵可能条件(成魚・非病気・well_fed_timer十分)を満たさない魚は対象外で、
        // 位置が変化せず卵も産まれないはず。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(32));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 10.0, 10.0));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 90.0, 10.0));
        // well_fed_timerを満たさないまま呼ぶ(既定は0.0)

        sim.debug_force_courtship_proximity(w, h);

        assert_eq!(sim.fish[1].x, 90.0, "産卵可能条件を満たさない魚は動かないはず");
        assert!(sim.eggs.is_empty(), "産卵可能条件を満たさない魚は卵を産まないはず");
    }

    #[test]
    fn debug_toggle_pollution_flips_between_zero_and_max() {
        let mut sim = Simulation::new(Rng::new(950));
        assert_eq!(sim.pollution, 0.0);
        sim.debug_toggle_pollution();
        assert_eq!(sim.pollution, POLLUTION_MAX);
        sim.debug_toggle_pollution();
        assert_eq!(sim.pollution, 0.0);
    }

    #[test]
    fn debug_kill_random_fish_marks_exactly_one_living_fish_as_dead() {
        let mut sim = Simulation::new(Rng::new(951));
        for i in 0..5 {
            sim.fish
                .push(Fish::new(Species::Neon, Stage::Adult, 10.0 + i as f64, 10.0));
        }
        sim.debug_kill_random_fish();
        let dead_count = sim.fish.iter().filter(|f| f.dead).count();
        assert_eq!(dead_count, 1, "生きている個体からちょうど1匹だけ死亡するはず");
    }

    #[test]
    fn debug_kill_random_fish_does_nothing_when_no_fish_are_alive() {
        let mut sim = Simulation::new(Rng::new(952));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 10.0, 10.0);
        f.dead = true;
        sim.fish.push(f);
        sim.debug_kill_random_fish(); // パニックしないことを確認する
        assert!(sim.fish[0].dead);
    }

    #[test]
    fn debug_grow_all_to_adult_matures_every_living_fry_but_leaves_dead_and_adults_alone() {
        let mut sim = Simulation::new(Rng::new(956));
        let mut fry1 = Fish::new(Species::Neon, Stage::Fry, 10.0, 10.0);
        fry1.well_fed_timer = 12.0;
        sim.fish.push(fry1);
        let mut fry2 = Fish::new(Species::Goldfish, Stage::Fry, 12.0, 10.0);
        fry2.well_fed_timer = 3.0;
        sim.fish.push(fry2);
        let mut dead_fry = Fish::new(Species::Guppy, Stage::Fry, 14.0, 10.0);
        dead_fry.dead = true;
        sim.fish.push(dead_fry);
        let adult = Fish::new(Species::Betta, Stage::Adult, 16.0, 10.0);
        sim.fish.push(adult);

        sim.debug_grow_all_to_adult();

        assert_eq!(sim.fish[0].stage, Stage::Adult, "生きている稚魚1匹目は成魚になるはず");
        assert_eq!(sim.fish[0].well_fed_timer, 0.0, "通常の成長遷移と同じくタイマーはリセットされるはず");
        assert_eq!(sim.fish[1].stage, Stage::Adult, "生きている稚魚2匹目も成魚になるはず");
        assert_eq!(sim.fish[2].stage, Stage::Fry, "死亡演出中の個体は対象外のはず");
        assert_eq!(sim.fish[3].stage, Stage::Adult, "既に成魚の個体はそのままのはず");
    }

    #[test]
    fn debug_age_random_fish_near_death_leaves_exactly_ten_seconds_of_lifespan() {
        let mut sim = Simulation::new(Rng::new(954));
        for i in 0..5 {
            sim.fish
                .push(Fish::new(Species::Neon, Stage::Adult, 10.0 + i as f64, 10.0));
        }
        sim.debug_age_random_fish_near_death();
        let aged: Vec<&Fish> = sim
            .fish
            .iter()
            .filter(|f| (f.age - (LIFESPAN_DEATH_AGE * f.lifespan_mult - 10.0)).abs() < 1e-9)
            .collect();
        assert_eq!(aged.len(), 1, "ちょうど1匹だけ寿命残り10秒になるはず");
        assert!(!aged[0].dead, "即死ではなく寿命の残りを詰めるだけのはず");
    }

    #[test]
    fn debug_age_random_fish_near_death_does_nothing_when_no_fish_are_alive() {
        let mut sim = Simulation::new(Rng::new(955));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 10.0, 10.0);
        f.dead = true;
        sim.fish.push(f);
        sim.debug_age_random_fish_near_death(); // パニックしないことを確認する
        assert!(sim.fish[0].dead);
    }

    #[test]
    fn debug_spawn_star_adds_one_star_per_call_and_can_stack() {
        // 何度も押してカーソル周辺に複数投入できることの回帰テスト
        // (以前は既に1個あると追加できない制限があった)。
        let mut sim = Simulation::new(Rng::new(953));
        sim.debug_spawn_star(40.0, 20.0, 80, 40);
        assert_eq!(sim.stars.len(), 1, "スターが1つ投入されるはず");
        sim.debug_spawn_star(40.0, 20.0, 80, 40);
        assert_eq!(sim.stars.len(), 2, "既にスターがあっても追加で投入できるはず");
        sim.debug_spawn_star(40.0, 20.0, 80, 40);
        assert_eq!(sim.stars.len(), 3, "何個でも積み増せるはず");
    }

    #[test]
    fn debug_spawn_star_is_offset_from_the_cursor_so_it_is_not_hidden_under_it() {
        // 回帰テスト: カーソルとスターは同じ十字形を描くため、カーソル位置に
        // そのまま重ねると後から描かれるカーソルに完全に隠れて見えなくなる
        // バグがあった。カーソル位置からずらして投入することを確認する。
        let mut sim = Simulation::new(Rng::new(958));
        let (cursor_x, cursor_y) = (40.0, 20.0);
        sim.debug_spawn_star(cursor_x, cursor_y, 80, 40);
        let star = &sim.stars[0];
        let dist = ((star.x - cursor_x).powi(2) + (star.y - cursor_y).powi(2)).sqrt();
        assert!(
            dist >= STAR_CURSOR_OFFSET - 0.01,
            "スターはカーソル位置から離して投入されるはず(実際の距離: {})",
            dist
        );
    }

    #[test]
    fn toggle_crabs_clears_and_repopulates() {
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(954));
        sim.seed_initial(w, h); // ensure_decorative_entitiesでカニが初期配置される
        assert!(!sim.crabs.is_empty(), "テスト前提: 初期状態でカニがいること");

        sim.toggle_crabs(w);
        assert!(sim.crab_toggle == false && sim.crabs.is_empty(), "OFFにすると即座にカニが消えるはず");

        sim.toggle_crabs(w);
        assert!(
            sim.crab_toggle && !sim.crabs.is_empty(),
            "ONに戻すとカニが再配置されるはず"
        );
    }

    #[test]
    fn predation_kill_spikes_pollution_instantly() {
        // 捕食が成立した瞬間、水質(pollution)がPOLLUTION_PREDATION_SPIKE分だけ
        // 一気に悪化するはずの回帰テスト。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(955));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0; // 捕食モード
        sim.fish.push(piranha);
        sim.fish
            .push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0)); // 密着させて即捕食させる

        assert_eq!(sim.pollution, 0.0);
        let mut spiked = false;
        for _ in 0..50 {
            sim.update(0.1, w, h);
            if sim.pollution > 0.0 {
                spiked = true;
                break;
            }
        }
        assert!(spiked, "捕食が成立したら水質が悪化するはず");
        assert!(
            sim.pollution >= POLLUTION_PREDATION_SPIKE - 1.0,
            "捕食1件でPOLLUTION_PREDATION_SPIKE相当の急上昇があるはず(実際: {})",
            sim.pollution
        );
    }

    #[test]
    fn crab_cleans_up_a_settled_corpse_with_decompose_effect() {
        // 沈んで水底に着地した亡骸にカニが接触すると、分解演出(Decompose)を
        // 出しつつ個体が消えることの回帰テスト。
        let (w, h) = (80, 40);
        let sand_top = h as f64 - sand_height(h) as f64;
        let mut sim = Simulation::new(Rng::new(956));
        let mut corpse = Fish::new(Species::Neon, Stage::Adult, 20.0, sand_top - 1.0);
        corpse.dead = true;
        corpse.dead_timer = CORPSE_REMOVE_TIME / 2.0; // 十分に沈んでいる想定
        sim.fish.push(corpse);
        sim.crabs.push(Crab {
            x: 20.0,
            dir: 1.0,
            pause_timer: 0.0,
            facing_right: true,
        });

        sim.update_crabs(0.1, w as f64, sand_top);

        assert!(sim.fish.is_empty(), "水底の亡骸はカニに片付けられて消えるはず");
        assert!(
            sim.drop_effects.iter().any(|e| e.kind == EffectKind::Decompose),
            "片付けられた瞬間に分解演出(Decompose)が出るはず"
        );
    }

    #[test]
    fn crab_ignores_a_corpse_that_is_still_floating_or_sinking() {
        // まだ浮いている/沈んでいる途中の亡骸はカニの対象外(水底に着地するまで待つ)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(957));
        let mut corpse = Fish::new(Species::Neon, Stage::Adult, 20.0, 5.0); // 水面付近=浮いている
        corpse.dead = true;
        corpse.dead_timer = 1.0;
        sim.fish.push(corpse);
        sim.crabs.push(Crab {
            x: 20.0,
            dir: 1.0,
            pause_timer: 0.0,
            facing_right: true,
        });

        let sand_top = h as f64 - sand_height(h) as f64;
        sim.update_crabs(0.1, w as f64, sand_top);

        assert_eq!(sim.fish.len(), 1, "浮いている/沈降中の亡骸はカニの対象外のはず");
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
    fn reposition_dens_trims_excess_dens_down_to_the_current_octopus_count() {
        // タコつぼの数を現在のタコの数に整理し、タコより多いタコつぼは削除してほしいという
        // 要望への対応: 自然にタコが減って(老衰等)空きタコつぼが
        // 取り残された状態を想定し、Dキーでタコの数まで削減されることを確認する。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(733));
        sim.ensure_decorative_entities(w, h);
        sim.add_octopus(w, h); // タコつぼ1個+タコ1匹
        sim.add_octopus(w, h); // タコつぼ2個+タコ2匹(空きが無いので新設される)
        sim.add_octopus(w, h); // タコつぼ3個+タコ3匹
        assert_eq!(sim.dens.len(), 3);

        // 1匹だけタコが自然に居なくなった(老衰等でremove_fishを経由しない)状況を
        // シミュレートする: タコつぼは削除されず3個のまま残っている状態から始める。
        let octo_idx = sim.fish.iter().position(|f| f.species == Species::Octopus).unwrap();
        sim.fish.remove(octo_idx);
        assert_eq!(sim.dens.len(), 3, "この時点ではタコつぼはまだ削除されないはず");
        let remaining_octo_count = sim
            .fish
            .iter()
            .filter(|f| f.species == Species::Octopus)
            .count();
        assert_eq!(remaining_octo_count, 2);

        sim.reposition_dens(w, h);

        assert_eq!(
            sim.dens.len(),
            2,
            "Dキーでタコつぼの数が現在のタコの数(2匹)まで整理されるはず"
        );
    }

    #[test]
    fn reposition_dens_adds_missing_dens_up_to_the_current_octopus_count() {
        // タコの数よりタコつぼが少ない場合、不足分が追加されるはず。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(734));
        sim.fish.push(Fish::new(Species::Octopus, Stage::Adult, 10.0, 10.0));
        sim.fish.push(Fish::new(Species::Octopus, Stage::Adult, 20.0, 10.0));
        sim.fish.push(Fish::new(Species::Octopus, Stage::Adult, 30.0, 10.0));
        assert!(sim.dens.is_empty(), "テスト前提: タコつぼが1つも無い状態");

        sim.reposition_dens(w, h);

        assert_eq!(sim.dens.len(), 3, "タコの数(3匹)分のタコつぼが追加されるはず");
        for f in sim.fish.iter().filter(|f| f.species == Species::Octopus) {
            assert!(
                sim.dens.iter().any(|d| d.x == f.den_x && d.y == f.den_y),
                "各タコが新しいタコつぼのいずれかに紐づいているはず"
            );
        }
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
        // 魚が水底に張り付いて見え、UFOのように動かないという指摘の
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
    fn vanishing_octopus_removes_its_own_den() {
        // タコが死んだら、その死んだタコの壺が自動的に消えるようにしてほしいという要望
        // の回帰テスト。タコが死亡演出を終えて水槽から完全に消えたら、そのタコが
        // 使っていたタコつぼも一緒に消えるはず(空のタコつぼだけ残って不自然に
        // ならないように)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(900));
        sim.add_octopus(w, h);
        assert_eq!(sim.dens.len(), 1, "テスト前提: タコつぼが1つあること");
        let (den_x, den_y) = (sim.fish[0].den_x, sim.fish[0].den_y);
        sim.fish[0].dead = true;
        sim.fish[0].dead_timer = CORPSE_REMOVE_TIME; // 待機時間を経過済みにする

        sim.update(0.1, w, h);

        assert!(
            sim.fish.iter().all(|f| f.species != Species::Octopus),
            "テスト前提: タコは消えているはず"
        );
        assert!(
            !sim.dens.iter().any(|d| d.x == den_x && d.y == den_y),
            "消えたタコが使っていたタコつぼも一緒に消えるはず"
        );
    }

    #[test]
    fn reset_clears_dens_and_other_decorative_entities() {
        // Rキーでのグレートリセット時に蛸壺もリセットしてほしいという要望の
        // 回帰テスト。リセット前の古いタコつぼ・藻・岩・カニ等がそのまま残らず、
        // 新しく再配置されるはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(901));
        sim.seed_initial(w, h);
        sim.add_octopus(w, h);
        let old_den_positions: Vec<(f64, f64)> = sim.dens.iter().map(|d| (d.x, d.y)).collect();
        assert!(!old_den_positions.is_empty(), "テスト前提: タコつぼが存在すること");

        sim.reset(w, h);

        assert!(
            sim.fish.iter().all(|f| f.species != Species::Octopus),
            "リセット後はタコが入っていないはず(seed_initialはタコを含まない)"
        );
        // リセット後にタコつぼが無いか、あっても古い位置とは無関係に再配置されているはず。
        let still_has_old_den = sim
            .dens
            .iter()
            .any(|d| old_den_positions.iter().any(|&(ox, oy)| ox == d.x && oy == d.y));
        assert!(!still_has_old_den, "古いタコつぼの位置がそのまま残っていないはず");
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
        // タコつぼが小さく目立たず、壺らしい形がはっきり分かるサイズにしてほしいという
        // 指摘への対応: 旧サイズ(6幅x5高)よりはっきり大きく描き直した。
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
        // 文字サイズを変更するとタコつぼや水草が床に沈む、または逆に浮いてしまう現象への
        // 対応: タコつぼ・水草は生成時の
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
            // 1000 * 0.1 = 100秒。OCTOPUS_HIDDEN_TIME_MAX(25秒)より十分長い。
            sim.update(0.1, w, h);
            if !sim.fish[0].hidden {
                saw_emerged = true;
                break;
            }
        }
        assert!(saw_emerged, "十分な時間が経てば一度はつぼから出てくるはず");

        // さらに時間を進めれば、出ている時間(最大55秒)を超えて必ず巣へ戻るはず
        for _ in 0..800 {
            // 800 * 0.1 = 80秒。OCTOPUS_EMERGE_TIME_MAX(55秒)より十分長い。
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
        // 当たり判定を胴体でなく口にすべきという指摘への対応: 口の位置に配置する
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

        // 1発では死なずPIRANHA_BITES_TO_KILL発で死ぬので、空腹・クールダウン明けを毎tick維持して
        // 連続で噛ませ、死亡まで到達させる。
        for _ in 0..50 {
            sim.fish[1].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.fish[1].predation_cooldown = 0.0;
            sim.update(0.1, w, h);
            if sim.fish[0].dead {
                break;
            }
        }

        // ピラニアの捕食は即消滅させず死骸を残すため、出ているタコ(index 0)が
        // 死亡状態になったことで「ピラニアの捕食対象になれる」ことを確認する。
        assert_eq!(sim.fish.len(), 2, "捕食後もタコは死骸として残るはず");
        assert_eq!(sim.fish[0].species, Species::Octopus);
        assert!(sim.fish[0].dead, "出ているタコはピラニアに捕食されてよいはず");
        assert_eq!(sim.fish[1].species, Species::Piranha);
        assert!(!sim.fish[1].dead, "ピラニアは生きて残るはず");
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
    fn fish_swims_toward_a_distant_star() {
        // 回帰テスト: スターへの誘引ベクトルが無く、偶然近くを泳いだ時しか取得
        // できないバグがあった。捕食者でない・まだ無敵でない魚は、離れた位置の
        // スターに向かって速度がつくはず。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(959));
        sim.fish
            .push(Fish::new(Species::Neon, Stage::Adult, 20.0, 20.0));
        sim.stars.push(Star {
            x: 100.0,
            y: 20.0,
            life: STAR_LIFETIME,
            phase: 0.0,
        });

        sim.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);

        assert!(
            sim.fish[0].vx > 0.0,
            "右側にあるスターへ向かって速度がつくはず(実際: vx={})",
            sim.fish[0].vx
        );
    }

    #[test]
    fn predator_species_ignores_stars() {
        // ピラニア・タコ自身はスターへの誘引ベクトルがつかないはず(取得できない
        // ものに反応しない)。通常の遊泳(ランダムウォーク)自体は常に多少の速度を
        // 生むため、単純に0であることではなく、同じ乱数シード・同じ初期状態で
        // 「スターがある場合」と「無い場合」の結果が一致することを比較して検証する
        // (一致すればスターは移動に一切影響していないと言える)。
        let (w, h) = (200, 100);
        let make_sim = |with_star: bool| {
            let mut sim = Simulation::new(Rng::new(960));
            let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 20.0, 20.0);
            piranha.hunger = MAX_HUNGER;
            sim.fish.push(piranha);
            if with_star {
                sim.stars.push(Star {
                    x: 100.0,
                    y: 20.0,
                    life: STAR_LIFETIME,
                    phase: 0.0,
                });
            }
            sim
        };
        let mut with_star = make_sim(true);
        let mut without_star = make_sim(false);
        with_star.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);
        without_star.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);

        assert_eq!(
            with_star.fish[0].vx, without_star.fish[0].vx,
            "ピラニアの速度はスターの有無で変わらないはず(反応していない)"
        );
    }

    #[test]
    fn already_invincible_fish_ignores_stars() {
        // 既に無敵中の魚は、スターへの誘引ベクトルがつかないはず(もう1個
        // 取得する必要が無いため)。通常の遊泳(ランダムウォーク)自体は常に
        // 多少の速度を生むため、同じ乱数シード・同じ初期状態で「スターがある
        // 場合」と「無い場合」の結果が一致することを比較して検証する。
        let (w, h) = (200, 100);
        let make_sim = |with_star: bool| {
            let mut sim = Simulation::new(Rng::new(961));
            let mut hero = Fish::new(Species::Neon, Stage::Adult, 20.0, 30.0);
            hero.invincible_timer = 10.0;
            sim.fish.push(hero);
            if with_star {
                sim.stars.push(Star {
                    x: 100.0,
                    y: 20.0,
                    life: STAR_LIFETIME,
                    phase: 0.0,
                });
            }
            sim
        };
        let mut with_star = make_sim(true);
        let mut without_star = make_sim(false);
        with_star.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);
        without_star.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);

        assert_eq!(
            with_star.fish[0].vx, without_star.fish[0].vx,
            "無敵の魚の速度はスターの有無で変わらないはず(反応していない)"
        );
    }

    #[test]
    fn star_out_of_pickup_range_is_not_collected() {
        // 誘引ベクトルの追加(バグ修正: スターへ泳いで近づく挙動)により、update()を
        // そのまま使うと移動込みで距離が縮まってしまうため、取得判定(update_stars)
        // だけを直接呼んで、STAR_PICKUP_RADIUSの範囲判定そのものを検証する。
        let mut sim = Simulation::new(Rng::new(701));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));
        sim.stars.push(Star {
            x: 40.0 + STAR_PICKUP_RADIUS + 5.0,
            y: 20.0,
            life: STAR_LIFETIME,
            phase: 0.0,
        });

        sim.update_stars(0.1);

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
    fn cameo_eventually_spawns_on_its_own_given_enough_time() {
        // 実際のCAMEO_SPAWN_CHANCE_PER_SEC(乱数抽選)経路そのものを検証する
        // (他のテストは手動でCameoをpushして動作を確認しているだけなので、
        // 抽選ロジック自体が機能しているかはここで別に確認する)。
        // 期待発生回数が十分大きくなる時間だけ回し、ほぼ確実に1回は発生するはず。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(739));
        let mut saw_cameo = false;
        for _ in 0..50000 {
            // 50000 * 0.1 = 5000秒(平均発生間隔400秒の12.5倍)。
            sim.update(0.1, w, h);
            if !sim.cameos.is_empty() {
                saw_cameo = true;
                break;
            }
        }
        assert!(saw_cameo, "十分な時間が経てばカメオ生物がいつかは出現するはず");
    }

    #[test]
    fn cameo_crosses_the_screen_and_despawns_on_the_far_side() {
        // カメオ生物(ウミガメ・クラゲ・小魚の群れ)は画面の端から反対側の端まで
        // 通過するとその後消える。育成ロジック・捕食判定には参加しない完全観賞用。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(724));
        sim.cameos.push(Cameo {
            kind: CameoKind::Turtle,
            x: -CAMEO_DESPAWN_MARGIN + 1.0,
            y: 10.0,
            vx: CAMEO_SPEED_MAX * 4.0, // テストを短時間で終わらせるため速めに設定
            vy: 0.0,
            phase: 0.0,
        });

        let mut crossed = false;
        for _ in 0..2000 {
            sim.update(0.1, w, h);
            if sim.cameos.is_empty() {
                crossed = true;
                break;
            }
        }
        assert!(crossed, "十分な時間が経てば画面を通過して消えるはず");
    }

    #[test]
    fn cameo_coexists_with_fish_and_predation_without_interfering() {
        // カメオ生物は魚(Fish)とは完全に独立した別リストであり、捕食対象にならず
        // 自身も捕食しない。カメオが同じ位置に居ても、通常の魚の捕食ロジックは
        // 普段どおり成立し(=カメオが割り込んで邪魔しない)、かつカメオがfishリストに
        // 紛れ込まないことを確認する。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(725));
        let piranha_probe = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        let (mx, my) = piranha_probe.mouth_position();
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, mx, my));
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);
        // ちょうど同じ位置にカメオを置いても、捕食判定に一切関与しないはず
        sim.cameos.push(Cameo {
            kind: CameoKind::Jellyfish,
            x: mx,
            y: my,
            vx: 0.0,
            vy: 0.0,
            phase: 0.0,
        });

        // 1発では死なずPIRANHA_BITES_TO_KILL発で死ぬので、空腹・クールダウン明けを毎tick維持して
        // 連続で噛ませ、死亡まで到達させる。
        for _ in 0..50 {
            sim.fish[1].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.fish[1].predation_cooldown = 0.0;
            sim.update(0.1, w, h);
            if sim.fish[0].dead {
                break;
            }
        }

        // ピラニアの捕食は即消滅させず死骸を残すため、獲物(index 0)が死亡状態に
        // なったことで「カメオが居ても通常の捕食は普段どおり成立する」ことを確認する。
        assert_eq!(sim.fish.len(), 2, "捕食後も獲物は死骸として残るはず");
        assert!(sim.fish[0].dead, "カメオが居ても通常の捕食は普段どおり成立するはず");
        assert!(
            sim.fish.iter().all(|f| f.species == Species::Neon || f.species == Species::Piranha),
            "カメオ自体はfishリストに紛れ込まないはず"
        );
        assert_eq!(sim.cameos.len(), 1, "カメオ自身は捕食されて消えたりしないはず");
    }

    #[test]
    fn cameo_never_persists_in_saved_state_fields() {
        // カメオは完全観賞用の一時的な存在で、育成ロジック(fish)や装飾(plants/dens等)
        // とは別枠であることを確認する簡単な回帰チェック。
        let mut sim = Simulation::new(Rng::new(726));
        sim.cameos.push(Cameo {
            kind: CameoKind::FishSchool,
            x: 10.0,
            y: 10.0,
            vx: 5.0,
            vy: 0.0,
            phase: 0.0,
        });
        assert_eq!(sim.cameos.len(), 1);
        assert_eq!(sim.fish.len(), 0, "カメオはfishリストとは別枠のはず");
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
                Stage::Adult, // 健康・満腹の成魚(タコの成魚制限には引っかからない条件)
                false,
                false,
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
        // (基本移動速度を全体的に4倍にする方針への対応でwander()の加速度も
        // 4倍になったため、旧dtよりさらに小さくして同程度のずれに抑える)。
        sim.update(0.005, w, h);

        assert_eq!(sim.fish_count(), 1, "無敵中のネオンがピラニアを捕食できるはず");
        assert_eq!(sim.fish[0].species, Species::Neon);
        assert!(sim.fish[0].is_invincible());
    }

    #[test]
    fn invincible_common_fish_actively_chases_a_distant_piranha() {
        // 回帰テスト: 無敵中は捕食者(ピラニア・タコ)を捕食対象にできるだけでなく、
        // STAR_HUNT_RADIUS以内にいれば実際に追いかける(吸引ベクトルが働く)はず。
        // 以前はchase_targetの計算が捕食者種(is_predator())限定になっており、
        // 無敵中の通常種には吸引ベクトルが一切つかず、偶然近づいた時しか
        // 捕食できない(自分からは追いかけ回さない)バグがあった。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(706));
        let mut hero = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        hero.invincible_timer = 10.0;
        sim.fish.push(hero);
        // STAR_HUNT_RADIUS(=PIRANHA_HUNT_RADIUS)以内・STAR_STRIKE_RADIUSより十分遠くに
        // ピラニアを置く(まだ接触していない距離で、追跡ベクトルの有無だけを見る)。
        let piranha_x = 40.0 + STAR_HUNT_RADIUS * 0.6;
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, piranha_x, 20.0);
        piranha.hunger = MAX_HUNGER; // ピラニア自身は捕食モードではない
        sim.fish.push(piranha);

        sim.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);

        assert!(
            sim.fish[0].vx > 0.0,
            "無敵の魚は右側にいるピラニアへ向かって速度がつくはず(実際: vx={})",
            sim.fish[0].vx
        );
    }

    #[test]
    fn invincible_fish_chases_a_piranha_beyond_the_normal_hunt_radius() {
        // 回帰テスト: 無敵中はスターへの誘引と同程度に徹底的に追いかけ回す仕様のため、
        // 通常の狩り(STAR_HUNT_RADIUS)を大きく超える距離のピラニアにも吸引ベクトルが
        // つくはず(距離無制限)。
        let (w, h) = (400, 200);
        let mut sim = Simulation::new(Rng::new(707));
        let mut hero = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        hero.invincible_timer = 10.0;
        sim.fish.push(hero);
        let piranha_x = 40.0 + STAR_HUNT_RADIUS * 3.0; // 通常のhunt_radiusを大きく超える距離
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, piranha_x, 20.0);
        piranha.hunger = MAX_HUNGER;
        sim.fish.push(piranha);

        sim.update_movement(0.05, w as f64, h as f64 - sand_height(h) as f64);

        assert!(
            sim.fish[0].vx > 0.0,
            "無敵の魚はSTAR_HUNT_RADIUSを超えた距離のピラニアも追いかけるはず(実際: vx={})",
            sim.fish[0].vx
        );
    }

    #[test]
    fn fish_that_picks_up_a_star_via_full_update_chases_a_nearby_piranha_closer() {
        // 単発のupdate_movement呼び出しではなく、実際のゲームループ(sim.update()を
        // 繰り返し呼ぶ通常のプレイ相当)でも、スター取得後にピラニアとの距離が
        // 縮まっていくことを確認する回帰テスト。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(707));
        sim.fish
            .push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));
        sim.stars.push(Star {
            x: 40.0,
            y: 20.0,
            life: STAR_LIFETIME,
            phase: 0.0,
        });
        let piranha_x = 40.0 + STAR_HUNT_RADIUS * 0.6;
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, piranha_x, 20.0);
        piranha.hunger = MAX_HUNGER;
        sim.fish.push(piranha);

        sim.update(0.1, w, h);
        assert!(sim.fish[0].is_invincible(), "1tick目でスターを取得して無敵になるはず");

        let dist_after_pickup = (sim.fish[1].x - sim.fish[0].x).abs();
        for _ in 0..50 {
            sim.update(0.1, w, h);
            if sim.fish.len() < 2 {
                break; // ピラニアを倒せた場合は成功とみなす
            }
        }
        let final_dist = if sim.fish.len() < 2 {
            0.0
        } else {
            (sim.fish[1].x - sim.fish[0].x).abs()
        };
        assert!(
            final_dist < dist_after_pickup,
            "無敵の魚はピラニアを追いかけて距離が縮まる(または倒す)はず(取得直後: {dist_after_pickup}, 最終: {final_dist})"
        );
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
    fn invincible_fish_preying_on_an_octopus_also_clears_its_den() {
        // 回帰テスト: 無敵中の魚に捕食されて死んだタコのタコつぼが後始末されず
        // 残ってしまうバグがあった(update_biology・update_crabs側の後始末とは
        // 別経路である捕食(update_predation)での削除漏れ)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(962));
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
        sim.dens.push(Den { x: mx, y: my });

        sim.update(0.01, w, h);

        assert_eq!(sim.fish_count(), 1, "無敵中の金魚がタコを捕食できるはず");
        assert!(
            sim.dens.is_empty(),
            "捕食されて消えたタコのタコつぼも一緒に片付くはず"
        );
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
        // 隠れたら実際に捕食されなくなるよう機能化してほしいという要望への対応: 藻に
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

        // 1発では死なずPIRANHA_BITES_TO_KILL発で死ぬので、空腹・クールダウン明けを毎tick維持して
        // 連続で噛ませ、死亡まで到達させる。
        for _ in 0..50 {
            sim.fish[1].hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
            sim.fish[1].predation_cooldown = 0.0;
            sim.update(0.1, w, h);
            if sim.fish[0].dead {
                break;
            }
        }

        // ピラニアの捕食は即消滅させず死骸を残すため、獲物(index 0)が死亡状態に
        // なったことで「隠れ場所が無ければ通常どおり捕食される」ことを確認する。
        assert_eq!(sim.fish.len(), 2, "捕食後も獲物は死骸として残るはず");
        assert!(sim.fish[0].dead, "隠れ場所が無ければ通常どおり捕食されるはず");
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
        // 当たり判定を胴体でなく口にすべきという指摘への対応: 口の位置に配置する
        let (mx, my) = octo.mouth_position();
        sim.fish.push(octo);
        // タコは健康・満腹な成魚を襲わない仕様のため、常に捕食対象になる稚魚を獲物にする。
        sim.fish.push(Fish::new(Species::Neon, Stage::Fry, mx, my));

        sim.update(0.1, w, h);

        assert_eq!(sim.fish_count(), 1, "空腹で出ているタコは近くの稚魚を捕食できるはず");
        assert_eq!(sim.fish[0].species, Species::Octopus);
        assert!(
            sim.fish[0].hunger > OCTOPUS_HUNT_HUNGER_THRESHOLD - 10.0,
            "捕食で空腹度が回復するはず"
        );
    }

    // タコの捕食対象制限(稚魚のみ対象・成魚は状態を問わず対象外)の判定ロジックを
    // 直接呼んで検証する。ピラニア捕食側は無敵バイパスより後・同種判定より前に
    // 挿入したゲートの影響を受けないことも合わせて確認する。
    fn octopus_excludes(
        candidate_stage: Stage,
        candidate_sick: bool,
        candidate_hungry: bool,
    ) -> bool {
        is_excluded_as_prey(
            Species::Octopus,
            0,
            0,
            false, // タコは無敵ではない(通常の捕食者としての判定)
            0,
            1,
            Species::Neon,
            false,
            false,
            false,
            false,
            0,
            0,
            candidate_stage,
            candidate_sick,
            candidate_hungry,
        )
    }

    #[test]
    fn octopus_always_targets_fry_regardless_of_condition() {
        // 稚魚は健康・満腹でも、病気・空腹でも常にタコの捕食対象になる。
        assert!(
            !octopus_excludes(Stage::Fry, false, false),
            "健康・満腹の稚魚もタコの捕食対象になるはず"
        );
        assert!(
            !octopus_excludes(Stage::Fry, true, true),
            "病気・空腹の稚魚もタコの捕食対象になるはず"
        );
    }

    #[test]
    fn octopus_excludes_every_adult_regardless_of_condition() {
        // 成魚はタコの捕食対象から常に外れる(稚魚のみ対象になったため、病気・空腹を
        // 問わず成魚は対象外。#37の「病気/空腹の成魚は対象」テストを反転した)。
        assert!(
            octopus_excludes(Stage::Adult, false, false),
            "健康・満腹の成魚はタコの捕食対象から除外されるはず"
        );
        assert!(
            octopus_excludes(Stage::Adult, true, false),
            "病気の成魚も稚魚限定化により対象外になるはず"
        );
        assert!(
            octopus_excludes(Stage::Adult, false, true),
            "空腹の成魚も稚魚限定化により対象外になるはず"
        );
        assert!(
            octopus_excludes(Stage::Adult, true, true),
            "病気かつ空腹の成魚も対象外になるはず"
        );
    }

    #[test]
    fn piranha_still_targets_a_healthy_well_fed_adult() {
        // 新しい制限はタコ(Species::Octopus)限定のため、ピラニアは健康・満腹な成魚を
        // これまでどおり捕食対象にできる(影響を受けない)。
        assert!(
            !is_excluded_as_prey(
                Species::Piranha,
                0,
                0,
                false,
                0,
                1,
                Species::Neon,
                false,
                false,
                false,
                false,
                0,
                0,
                Stage::Adult,
                false,
                false,
            ),
            "ピラニアは健康・満腹の成魚をこれまでどおり襲えるはず"
        );
    }

    #[test]
    fn hungry_octopus_leaves_a_healthy_well_fed_adult_alone() {
        // 統合テスト: 空腹で出ているタコの目の前に健康・満腹な成魚がいても、
        // 一定時間ずっと捕食しないはず(新しい捕食対象制限)。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(620));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        let (mx, my) = octo.mouth_position();
        sim.fish.push(octo);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, mx, my));

        for _ in 0..100 {
            sim.fish[0].hunger = OCTOPUS_HUNT_HUNGER_THRESHOLD - 10.0; // タコは空腹を維持
            sim.fish[0].predation_cooldown = 0.0;
            sim.fish[1].hunger = MAX_HUNGER; // 獲物は満腹(健康)を維持
            sim.fish[1].sick = false;
            sim.update(0.1, w, h);
        }

        assert_eq!(sim.fish.len(), 2, "健康・満腹の成魚は襲われないはず");
        assert!(!sim.fish[1].dead, "健康・満腹の成魚は生きたままのはず");
    }

    #[test]
    fn hungry_octopus_leaves_a_hungry_adult_alone_now_that_only_fry_are_prey() {
        // #37の対テストを反転: 以前は空腹な成魚もタコに捕食されたが、捕食対象が
        // 稚魚のみに絞られたため、空腹な成魚も襲われず生き残るはず。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(621));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        let (mx, my) = octo.mouth_position();
        sim.fish.push(octo);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, mx, my));

        for _ in 0..200 {
            sim.fish[0].hunger = OCTOPUS_HUNT_HUNGER_THRESHOLD - 10.0; // タコは空腹を維持
            sim.fish[0].predation_cooldown = 0.0;
            sim.fish[1].hunger = HUNGRY_THRESHOLD - 1.0; // 獲物(成魚)は空腹を維持
            sim.update(0.1, w, h);
        }

        // 成魚(index 1)は空腹でも捕食されず生き残るはず(タコが逆に成魚にかじられて
        // 弱る/死ぬことはあり得るが、それは成魚の生存とは別事象)。
        assert!(
            sim.fish.iter().any(|f| f.species == Species::Neon && !f.dead),
            "空腹な成魚も稚魚限定化により襲われず生き残るはず"
        );
    }

    // テスト用: 出ているタコ(index 0)の隣に生きた成魚(index 1)を置き、確率的な
    // かじり判定が1回成立するまで1tickずつ進める。連打防止の免疫時間は毎tick0に戻して
    // 確実に判定に到達させ、成魚は満腹・位置を維持して圏内・生存にとどめる。
    // octopus_bite_countの増加か死亡フラグの変化を検知した時点で返す。
    fn land_one_octopus_bite(sim: &mut Simulation) {
        let before_count = sim.fish[0].octopus_bite_count;
        let before_dead = sim.fish[0].dead;
        // 確率イベントを待つため試行回数は十分大きめにしてある(spawn_flashのテストと
        // 同様に、無関係なコード変更でtickごとの乱数消費数が変わってもタイミングが
        // ずれにくいよう余裕を持たせている)。
        for _ in 0..100000 {
            sim.fish[0].hidden = false;
            sim.fish[0].hidden_timer = 999.0;
            sim.fish[0].octopus_bite_immunity_timer = 0.0; // 連打防止をバイパスして判定に到達させる
            sim.fish[0].hunger = MAX_HUNGER;
            sim.fish[1].hunger = MAX_HUNGER; // かじる側を満腹に保つ(捕食モードに入らせない・餓死させない)
            sim.fish[1].x = sim.fish[0].x;
            sim.fish[1].y = sim.fish[0].y;
            sim.update(0.1, 80, 40);
            if sim.fish[0].octopus_bite_count != before_count || sim.fish[0].dead != before_dead {
                return;
            }
        }
    }

    #[test]
    fn an_adult_fish_near_an_emerged_octopus_can_bite_it() {
        // 出ている(隠れていない)タコの近くに生きた成魚がいると、確率的にかじられて
        // octopus_bite_countが上がる。まず1回かじられて1になることを確認する。
        let mut sim = Simulation::new(Rng::new(660));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        sim.fish.push(octo);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));

        land_one_octopus_bite(&mut sim);
        assert_eq!(
            sim.fish[0].octopus_bite_count, 1,
            "近くの成魚に1回かじられてoctopus_bite_countが1になるはず"
        );
        assert!(!sim.fish[0].dead, "1回かじられただけでは死なないはず");
    }

    #[test]
    fn a_piranha_adult_can_also_bite_an_emerged_octopus() {
        // ピラニアも「かじる側」として参加できる(この仕組みだけはピラニアが捕食者
        // ではなくかじる側になる)。満腹に保って捕食モードに入らせず、かじりのみで
        // タコが弱ることを確認する。
        let mut sim = Simulation::new(Rng::new(661));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        // すぐ近くにピラニアがいると墨→緊急脱出が絡むので、墨はクールダウン中にしておく。
        octo.ink_cooldown = 999.0;
        sim.fish.push(octo);
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = MAX_HUNGER; // 満腹で狩り出さない(かじる側としてのみ参加)
        sim.fish.push(piranha);

        land_one_octopus_bite(&mut sim);
        assert!(
            sim.fish[0].octopus_bite_count >= 1,
            "満腹のピラニア成魚もかじる側として参加してタコを弱らせられるはず"
        );
        assert_eq!(sim.fish[0].species, Species::Octopus, "タコは(かじられただけで)まだ存在するはず");
    }

    #[test]
    fn five_octopus_bites_kill_it_matching_debug_kill_state() {
        // OCTOPUS_BITES_TO_DIE回かじられると死亡する。あと1回で死ぬ段階まで直接進めておき、
        // 実際のかじり処理(update_octopus_bites)で最後の一かじりを成立させて、死亡した瞬間の
        // 状態がXキー(debug_kill_random_fish)と同じ(dead=true・dead_timer=0.0)になることを確認する。
        // update_octopus_bitesはupdate()内でupdate_biology(死骸浮上でdead_timerを進める)より
        // 先に走るため、死亡直後の状態を見たい本テストではupdate_octopus_bitesだけを直接呼ぶ。
        let mut sim = Simulation::new(Rng::new(662));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        octo.octopus_bite_count = OCTOPUS_BITES_TO_DIE - 1;
        sim.fish.push(octo);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));

        // 確率イベントを待つため試行回数は十分大きめにしてある(spawn_flashのテストと同様、
        // 無関係なコード変更で乱数消費数が変わってもタイミングがずれにくいようにする)。
        for _ in 0..100000 {
            sim.fish[0].octopus_bite_immunity_timer = 0.0; // 連打防止をバイパスして判定に到達させる
            sim.update_octopus_bites(0.1);
            if sim.fish[0].dead {
                break;
            }
        }

        assert!(sim.fish[0].dead, "OCTOPUS_BITES_TO_DIE回目のかじりで死亡するはず");
        assert_eq!(
            sim.fish[0].dead_timer, 0.0,
            "死亡直後のdead_timerはdebug_kill_random_fishと同じく0.0のはず"
        );
        assert_eq!(sim.fish[0].species, Species::Octopus);

        // 参考: debug_kill_random_fishが作る死亡状態と同じフィールドになっていることを確認する。
        let mut sim2 = Simulation::new(Rng::new(662));
        sim2.fish.push(Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0));
        sim2.debug_kill_random_fish();
        assert_eq!(sim.fish[0].dead, sim2.fish[0].dead);
        assert_eq!(sim.fish[0].dead_timer, sim2.fish[0].dead_timer);
    }

    #[test]
    fn octopus_bite_weakening_heals_over_time_without_further_bites() {
        // これ以上かじられなければ、かじられ弱りはOCTOPUS_BITE_RECOVER_INTERVALごとに
        // 1段階ずつ時間経過で癒えて、最終的に無傷(0)に戻るはず(ピラニアの被噛みつき
        // 回復テストと同じ構造)。
        let mut sim = Simulation::new(Rng::new(663));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        octo.octopus_bite_count = 2;
        sim.fish.push(octo);

        // 1インターバル未満ではまだ回復しない(keep_fed=trueで満腹を保ち餓死を避ける。
        // タコは1匹だけなのでかじる成魚はおらず、追加のかじりは起きない)。
        run(&mut sim, OCTOPUS_BITE_RECOVER_INTERVAL - 5.0, 0.5, 80, 40, true);
        assert_eq!(sim.fish[0].octopus_bite_count, 2, "インターバル未満では回復しないはず");

        run(&mut sim, 6.0, 0.5, 80, 40, true);
        assert_eq!(sim.fish[0].octopus_bite_count, 1, "1インターバルで1段階回復するはず");

        run(&mut sim, OCTOPUS_BITE_RECOVER_INTERVAL + 1.0, 0.5, 80, 40, true);
        assert_eq!(sim.fish[0].octopus_bite_count, 0, "さらに1インターバルで無傷に戻るはず");
    }

    #[test]
    fn octopus_weakened_by_bites_swims_slower_than_an_unbitten_one() {
        // かじられ回数が増えるほど遊泳速度倍率(speed_mult)が下がるはず(空腹段階・病気を
        // そろえて、かじられぶんだけの差を直接確認する)。
        let make = |bites: u8| {
            let mut f = Fish::new(Species::Octopus, Stage::Adult, 0.0, 0.0);
            f.hunger = 50.0; // 全個体で同じ空腹段階にそろえる
            f.sick = false;
            f.octopus_bite_count = bites;
            f
        };
        let s0 = make(0).speed_mult();
        let s1 = make(1).speed_mult();
        let s2 = make(2).speed_mult();
        let s3 = make(3).speed_mult();
        assert!(s1 < s0, "1回かじられた個体は無傷より遅いはず: s1={s1} s0={s0}");
        assert!(s2 < s1, "2回かじられた個体はさらに遅いはず: s2={s2} s1={s1}");
        assert!(s3 < s2, "3回かじられた個体はさらに遅いはず: s3={s3} s2={s2}");
    }

    #[test]
    fn the_bite_immunity_window_prevents_back_to_back_bites() {
        // 免疫時間中は、隣に成魚がいて何tick進めても新たなかじり判定を受けないこと、
        // 免疫が切れれば再びかじられることを確認する。
        let mut sim = Simulation::new(Rng::new(664));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        octo.octopus_bite_immunity_timer = 999.0;
        sim.fish.push(octo);
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0));

        // 免疫が切れないよう毎tick高い値に戻し続け、圏内に成魚を置き続けても噛まれないこと。
        for _ in 0..2000 {
            sim.fish[0].hidden = false;
            sim.fish[0].hidden_timer = 999.0;
            sim.fish[0].octopus_bite_immunity_timer = 999.0;
            sim.fish[0].hunger = MAX_HUNGER;
            sim.fish[1].hunger = MAX_HUNGER;
            sim.fish[1].x = sim.fish[0].x;
            sim.fish[1].y = sim.fish[0].y;
            sim.update(0.1, 80, 40);
        }
        assert_eq!(sim.fish[0].octopus_bite_count, 0, "免疫時間中はかじられないはず");
        assert!(!sim.fish[0].dead, "免疫時間中は死なないはず");

        // 免疫を解除すれば、同じ状況で今度はかじられるはず。
        sim.fish[0].octopus_bite_immunity_timer = 0.0;
        land_one_octopus_bite(&mut sim);
        assert!(
            sim.fish[0].octopus_bite_count >= 1,
            "免疫が切れれば再びかじられるはず"
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
    fn octopus_inks_when_any_fish_swims_right_up_to_it() {
        // ピラニアが1匹も居なくても、種類を問わず魚がすぐ目の前
        // (OCTOPUS_INK_NEARBY_FISH_RADIUS以内)まで寄ってきたら墨を吐くはず。
        let mut sim = Simulation::new(Rng::new(630));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        sim.fish.push(octo);
        // 捕食者ではない普通の魚を、新しい近接半径の内側に置く。
        sim.fish.push(Fish::new(
            Species::Goldfish,
            Stage::Adult,
            40.0 + OCTOPUS_INK_NEARBY_FISH_RADIUS * 0.5,
            20.0,
        ));

        sim.update_octopus(0.1);

        assert!(
            !sim.ink_clouds.is_empty(),
            "目の前まで寄ってきた魚に対しても墨を吐くはず"
        );
        assert!(sim.sound_events.contains(&SfxEvent::Ink));
        let octo_idx = sim
            .fish
            .iter()
            .position(|f| f.species == Species::Octopus)
            .expect("タコが居るはず");
        assert!(
            sim.fish[octo_idx].ink_escape_timer > 0.0,
            "この新トリガーでも緊急脱出タイマー(逃走ダッシュ)が立つはず"
        );
    }

    #[test]
    fn octopus_does_not_ink_for_a_fish_outside_the_nearby_radius() {
        // 新しい近接半径より遠い(ただし旧ピラニア用トリガー半径よりは近い)位置に
        // 普通の魚が居るだけでは墨を吐かないこと。新半径が実際に使われており、
        // 旧半径に素通りしていないことを確認する。
        let mid = (OCTOPUS_INK_NEARBY_FISH_RADIUS + OCTOPUS_INK_TRIGGER_RADIUS) / 2.0;
        assert!(
            mid > OCTOPUS_INK_NEARBY_FISH_RADIUS && mid < OCTOPUS_INK_TRIGGER_RADIUS,
            "テスト前提: 新半径より遠く旧半径より近い距離であること"
        );
        let mut sim = Simulation::new(Rng::new(631));
        let mut octo = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        octo.hidden = false;
        octo.hidden_timer = 999.0;
        octo.den_x = 40.0;
        octo.den_y = 20.0;
        sim.fish.push(octo);
        sim.fish.push(Fish::new(
            Species::Goldfish,
            Stage::Adult,
            40.0 + mid,
            20.0,
        ));

        sim.update_octopus(0.1);

        assert!(
            sim.ink_clouds.is_empty(),
            "新半径より遠い普通の魚に対しては墨を吐かないはず"
        );
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
        // 墨を吐いたら高確率で逃げ切れる結果まで保証してほしいという要望への対応:
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

    #[test]
    fn ensure_decorative_entities_seeds_shrimp_and_seahorses() {
        // エビ・タツノオトシゴ(カニと同じ位置づけの観賞用背景生物)も、カニと同様に
        // 初期数だけ補充され、再度呼んでも増殖しないはず。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(735));
        assert!(sim.shrimp.is_empty());
        assert!(sim.seahorses.is_empty());
        sim.ensure_decorative_entities(w, h);
        assert_eq!(sim.shrimp.len(), SHRIMP_COUNT, "エビは既定数だけ補充される");
        assert_eq!(sim.seahorses.len(), SEAHORSE_COUNT, "タツノオトシゴは既定数だけ補充される");

        sim.ensure_decorative_entities(w, h);
        assert_eq!(sim.shrimp.len(), SHRIMP_COUNT, "既に居る場合はエビも補充されない");
        assert_eq!(
            sim.seahorses.len(),
            SEAHORSE_COUNT,
            "既に居る場合はタツノオトシゴも補充されない"
        );
    }

    #[test]
    fn shrimp_and_seahorses_stay_in_bounds_and_do_not_panic() {
        // カニと同様、育成ロジックの対象外だが、長時間の経過で座標が水槽の範囲を
        // 飛び出したり panic したりしないことを確認する。
        let (w, h) = (60, 30);
        let mut sim = Simulation::new(Rng::new(736));
        sim.seed_initial(w, h);
        for _ in 0..500 {
            sim.update(0.1, w, h);
        }
        for s in &sim.shrimp {
            assert!(s.x >= 0.0 && s.x <= w as f64, "エビのxは範囲内: {}", s.x);
        }
        for s in &sim.seahorses {
            assert!(s.x.is_finite() && s.y.is_finite(), "タツノオトシゴの座標が有限のはず");
        }
    }

    #[test]
    fn seahorse_stays_near_its_anchor_and_does_not_wander_far() {
        // タツノオトシゴは「藻に絡みつくようにゆっくり動き、あまり大きく移動しない」
        // 仕様のため、長時間経過しても基準位置(anchor)から一定距離以上離れないはず。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(737));
        sim.ensure_decorative_entities(w, h);
        assert!(!sim.seahorses.is_empty());
        let anchor_x = sim.seahorses[0].anchor_x;
        let anchor_y = sim.seahorses[0].anchor_y;

        for _ in 0..300 {
            sim.update(0.1, w, h);
        }

        let s = &sim.seahorses[0];
        assert_eq!(s.anchor_x, anchor_x, "基準位置自体は動かないはず");
        assert_eq!(s.anchor_y, anchor_y, "基準位置自体は動かないはず");
        let dist = ((s.x - anchor_x).powi(2) + (s.y - anchor_y).powi(2)).sqrt();
        assert!(
            dist <= SEAHORSE_DRIFT_AMPLITUDE + 0.5,
            "タツノオトシゴは基準位置からあまり大きく離れないはず: dist={dist}"
        );
    }

    #[test]
    fn shrimp_and_seahorses_are_never_eaten_by_a_hunting_piranha() {
        // エビ・タツノオトシゴはカニと同様、育成ロジック(fish)とは別枠のため、
        // 捕食判定(update_predation)には一切関与しない(捕食対象にならず、
        // fishリストにも紛れ込まない)ことを確認する。
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(738));
        let piranha_probe = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        let (mx, my) = piranha_probe.mouth_position();
        sim.shrimp.push(Shrimp {
            x: mx,
            dir: 1.0,
            pause_timer: 0.0,
            facing_right: true,
        });
        sim.seahorses.push(Seahorse {
            anchor_x: mx,
            anchor_y: my,
            x: mx,
            y: my,
            phase: 0.0,
        });
        let mut piranha = Fish::new(Species::Piranha, Stage::Adult, 40.0, 20.0);
        piranha.hunger = PIRANHA_HUNT_HUNGER_THRESHOLD - 10.0;
        sim.fish.push(piranha);

        sim.update(0.1, w, h);

        assert_eq!(sim.fish_count(), 1, "ピラニア自身は残るはず(食べる対象が居ない)");
        assert_eq!(sim.shrimp.len(), 1, "エビは捕食されないはず");
        assert_eq!(sim.seahorses.len(), 1, "タツノオトシゴは捕食されないはず");
    }

    #[test]
    fn octopus_has_a_larger_default_render_scale_than_other_species() {
        // タコはデフォルトで他種より大きく見せる要望への対応。成長段階0(初期状態)
        // でも、同じ条件の他種よりrender_scale()が大きいはず。
        let octopus = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        let neon = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        assert_eq!(octopus.growth_stage, neon.growth_stage, "テスト前提: 成長段階は揃っていること");
        assert!(
            octopus.render_scale() > neon.render_scale(),
            "タコのデフォルトサイズは他種より大きいはず(octopus={} neon={})",
            octopus.render_scale(),
            neon.render_scale()
        );
        assert_eq!(
            octopus.render_scale(),
            1.0 + OCTOPUS_BASE_SCALE_BONUS,
            "タコのベース倍率はOCTOPUS_BASE_SCALE_BONUS分だけ上乗せされるはず"
        );
    }

    #[test]
    fn add_whale_always_adds_a_whale() {
        // Wキー: ランダムではなく確実にクジラを追加できる。
        let (w, h) = (800, 200);
        let mut sim = Simulation::new(Rng::new(196));
        for _ in 0..5 {
            sim.add_whale(w, h);
        }
        assert_eq!(sim.fish_count(), 5, "5回呼べば5匹追加されるはず");
        assert!(
            sim.fish.iter().all(|f| f.species == Species::Whale),
            "add_whale で追加されるのは常にクジラのはず"
        );
    }

    #[test]
    fn add_whale_is_capped_at_manual_cap() {
        let (w, h) = (800, 200);
        assert!(capacity(w, h) > ADD_FISH_MANUAL_CAP, "テスト前提: 水槽容量はADD_FISH_MANUAL_CAPより大きい");
        let mut sim = Simulation::new(Rng::new(197));
        for _ in 0..(ADD_FISH_MANUAL_CAP + 10) {
            sim.add_whale(w, h);
        }
        assert_eq!(
            sim.fish_count(),
            ADD_FISH_MANUAL_CAP,
            "Wキーでの追加も+キーと同じくADD_FISH_MANUAL_CAPで頭打ちになるはず"
        );
    }

    #[test]
    fn add_whale_respects_tank_capacity_too() {
        // ADD_FISH_MANUAL_CAPより小さい水槽容量でも、そちらの上限が優先されて超えない。
        let (w, h) = (40, 20); // capacity は最小の5になる
        let cap = capacity(w, h);
        assert!(cap < ADD_FISH_MANUAL_CAP, "テスト前提: 水槽容量がADD_FISH_MANUAL_CAPより小さいこと");
        let mut sim = Simulation::new(Rng::new(198));
        for _ in 0..20 {
            sim.add_whale(w, h);
        }
        assert_eq!(sim.fish_count(), cap, "水槽容量の上限で頭打ちになるはず");
    }

    #[test]
    fn whale_is_never_valid_prey() {
        // クジラはネタ枠の巨大魚のため、どんな状況でも(通常の捕食者からも、無敵の
        // 一時的捕食者からも)捕食対象にならない。判定ロジックを直接呼んで検証する。
        // 通常の捕食者(捕食モードのピラニア)からは対象外。
        assert!(
            is_excluded_as_prey(
                Species::Piranha,
                0,
                0,
                false, // 捕食側は無敵ではない
                0,
                1,
                Species::Whale,
                false,
                false,
                false,
                false,
                0,
                0,
                Stage::Adult,
                false,
                false,
            ),
            "通常の捕食者からもクジラは捕食対象にならないはず"
        );
        // 無敵の一時的捕食者(通常なら捕食者すら襲える)からも対象外。
        assert!(
            is_excluded_as_prey(
                Species::Neon,
                0,
                0,
                true, // 捕食側が無敵(一時的捕食者反転)
                0,
                1,
                Species::Whale,
                false,
                false,
                false,
                false,
                0,
                0,
                Stage::Adult,
                false,
                false,
            ),
            "無敵の一時的捕食者からもクジラは捕食対象にならないはず"
        );
    }

    #[test]
    fn species_name_of_whale_is_kujira() {
        assert_eq!(species_name(Species::Whale), "クジラ");
    }

    #[test]
    fn whale_has_the_largest_default_render_scale() {
        // クジラはネタ枠の巨大魚として、タコよりも、どの通常種よりもデフォルトの
        // render_scale()が大きいはず。
        let whale = Fish::new(Species::Whale, Stage::Adult, 40.0, 20.0);
        let octopus = Fish::new(Species::Octopus, Stage::Adult, 40.0, 20.0);
        assert!(
            whale.render_scale() > octopus.render_scale(),
            "クジラのデフォルトサイズはタコより大きいはず(whale={} octopus={})",
            whale.render_scale(),
            octopus.render_scale()
        );
        for &sp in &Species::COMMON {
            let other = Fish::new(sp, Stage::Adult, 40.0, 20.0);
            assert!(
                whale.render_scale() > other.render_scale(),
                "クジラのデフォルトサイズは通常種({sp:?})より大きいはず(whale={} other={})",
                whale.render_scale(),
                other.render_scale()
            );
        }
        assert_eq!(
            whale.render_scale(),
            1.0 + WHALE_BASE_SCALE_BONUS,
            "クジラのベース倍率はWHALE_BASE_SCALE_BONUS分だけ上乗せされるはず"
        );
    }

    #[test]
    fn current_field_is_rotational_uniform_magnitude_and_center_drifts() {
        // current_at() は渦の中心からの相対位置に垂直な接線ベクトルを返す。場所によって
        // 向きが変わり(回転流)、大きさは中心に近いほど強く、離れるほど指数関数的に
        // 減衰する(トルネードの目)。中心とちょうど同じ位置では正確に(0.0, 0.0)になる。
        let mut sim = Simulation::new(Rng::new(900));
        sim.current_center_x = 50.0;
        sim.current_center_y = 20.0;

        // 中心のすぐ近くではCURRENT_STRENGTHに近い大きさになる。
        let (nvx, nvy) = sim.current_at(51.0, 20.0);
        let near_mag = (nvx * nvx + nvy * nvy).sqrt();
        assert!(
            near_mag > CURRENT_STRENGTH * 0.9,
            "中心のすぐ近くではほぼ最大の強さになるはず: near_mag={near_mag}"
        );

        // 中心から離れるほど、大きさは単調に小さくなる(指数減衰)。
        let dists = [1.0, 5.0, 15.0, 40.0, 100.0];
        let mut prev_mag = f64::INFINITY;
        for &d in &dists {
            let (vx, vy) = sim.current_at(sim.current_center_x + d, sim.current_center_y);
            let mag = (vx * vx + vy * vy).sqrt();
            assert!(
                mag < prev_mag,
                "中心から離れるほど大きさは小さくなるはず: dist={d} mag={mag} prev={prev_mag}"
            );
            prev_mag = mag;
        }
        // 十分離れれば、ほぼ0(自由に泳げる)まで減衰する。
        let (fx, fy) = sim.current_at(sim.current_center_x + 100.0, sim.current_center_y);
        let far_mag = (fx * fx + fy * fy).sqrt();
        assert!(far_mag < 0.1, "渦の目から十分離れればほぼ無風になるはず: far_mag={far_mag}");

        // 中心から見て角度の異なる2点は、違う向きのベクトルになる(=単純な一様押しではなく回転流)。
        let (ax, ay) = sim.current_at(80.0, 20.0); // 中心の右
        let (bx, by) = sim.current_at(50.0, 60.0); // 中心の下
        assert!(
            (ax - bx).abs() > 1e-6 || (ay - by).abs() > 1e-6,
            "中心からの角度が違えば力場の向きも違うはず: a=({ax},{ay}) b=({bx},{by})"
        );

        // 中心とちょうど同じ位置では、水流はぴったりゼロになる(無風基準として使える性質)。
        let (cx, cy) = sim.current_at(sim.current_center_x, sim.current_center_y);
        assert_eq!((cx, cy), (0.0, 0.0), "中心では水流はゼロのはず");

        // update_current() は経過時間で中心座標を動かし、余白の範囲内に収まる。
        let (w, h) = (200.0, 80.0);
        let amp_x = (w / 2.0) * (1.0 - CURRENT_CENTER_MARGIN_FRAC);
        let amp_y = (h / 2.0) * (1.0 - CURRENT_CENTER_MARGIN_FRAC);
        let elapseds = [0.0, 30.0, 90.0, 150.0];
        let mut centers = Vec::new();
        for &e in &elapseds {
            sim.elapsed = e;
            sim.update_current(w, h);
            assert!(
                sim.current_center_x >= w / 2.0 - amp_x - 1e-9
                    && sim.current_center_x <= w / 2.0 + amp_x + 1e-9,
                "中心xは余白の範囲内に収まるはず: {}",
                sim.current_center_x
            );
            assert!(
                sim.current_center_y >= h / 2.0 - amp_y - 1e-9
                    && sim.current_center_y <= h / 2.0 + amp_y + 1e-9,
                "中心yは余白の範囲内に収まるはず: {}",
                sim.current_center_y
            );
            centers.push((sim.current_center_x, sim.current_center_y));
        }
        let first = centers[0];
        assert!(
            centers.iter().any(|&(x, y)| (x - first.0).abs() > 1e-6 || (y - first.1).abs() > 1e-6),
            "elapsedを変えれば中心座標も変化するはず: {centers:?}"
        );
    }

    #[test]
    fn current_pushes_a_fish_off_course_compared_to_no_current() {
        // 同一シード・同一手順で、渦の中心を魚から離して置いた(強い水流あり)方と、
        // 中心を魚のいる位置ぴったりに置いた(水流ゼロ)方を比較し、水流ありの方が魚の
        // 位置が測れるほどずれることを確認する。update()は毎tick先頭で中心を再計算して
        // しまうため、水流の効果だけを切り出せるようupdate_movement()を直接呼び、毎tick
        // 中心を明示的に置き直す。
        let (w, h) = (200usize, 60usize);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let dt = 0.05;
        let steps = 200; // 10秒

        let mut with_current = Simulation::new(Rng::new(950));
        with_current.fish.push(Fish::new(Species::Neon, Stage::Adult, 100.0, 30.0));
        let mut without_current = Simulation::new(Rng::new(950));
        without_current.fish.push(Fish::new(Species::Neon, Stage::Adult, 100.0, 30.0));

        let mut sum_with = 0.0;
        let mut sum_without = 0.0;
        for _ in 0..steps {
            // 満腹に保ち、餌探索(誘引ベクトル)が動きを乱さないようにする。
            with_current.fish[0].hunger = MAX_HUNGER;
            without_current.fish[0].hunger = MAX_HUNGER;
            // 水流あり: 中心を魚の真上・渦の減衰半径内の近さに固定 → 左向きの押しになる
            // (CURRENT_FALLOFF_RADIUSより遠いとほぼ無風になるため、十分近くに置く)。
            with_current.current_center_x = 100.0;
            with_current.current_center_y = 30.0 - CURRENT_FALLOFF_RADIUS * 0.5;
            // 水流ゼロ: 中心を魚の現在位置ぴったりに置く → その位置ではcurrent_atが(0,0)。
            without_current.current_center_x = without_current.fish[0].x;
            without_current.current_center_y = without_current.fish[0].y;
            with_current.update_movement(dt, w as f64, sand_top);
            without_current.update_movement(dt, w as f64, sand_top);
            sum_with += with_current.fish[0].x;
            sum_without += without_current.fish[0].x;
        }
        let avg_with = sum_with / steps as f64;
        let avg_without = sum_without / steps as f64;
        assert!(
            avg_with < avg_without - 1.0,
            "上方の中心による左向き水流がある方が魚の平均x位置は左へ寄るはず: with={avg_with} without={avg_without}"
        );
    }

    #[test]
    fn current_drifts_falling_food_but_not_landed_food() {
        // 沈下中(未着地)の餌は水流で横に流されるが、着地済みの餌は影響を受けないことを確認する。
        let (w, h) = (120usize, 60usize);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let dt = 0.05;
        let steps = 20; // 1秒(この間はまだ沈下中)

        let falling = || Food {
            x: 60.0,
            y: 5.0,
            vy: 3.0,
            life: 30.0,
            landed: false,
            sway_phase: 0.0,
        };
        let mut with_current = Simulation::new(Rng::new(960));
        let mut without_current = Simulation::new(Rng::new(960));
        with_current.food.push(falling());
        without_current.food.push(falling());
        for _ in 0..steps {
            // 水流あり: 中心を餌の真上に固定 → 力場の水平成分が左向きになり横に流される。
            with_current.current_center_x = 60.0;
            with_current.current_center_y = 0.0;
            // 水流ゼロ: 中心を餌の現在位置ぴったりに置く → その位置ではcurrent_atが(0,0)。
            without_current.current_center_x = without_current.food[0].x;
            without_current.current_center_y = without_current.food[0].y;
            with_current.update_food(dt, sand_top, w);
            without_current.update_food(dt, sand_top, w);
        }
        assert!(
            !with_current.food[0].landed && !without_current.food[0].landed,
            "この短時間ではまだ沈下中のはず"
        );
        assert!(
            with_current.food[0].x < without_current.food[0].x - 0.5,
            "沈下中の餌は左向き水流で左に流されるはず: with={} without={}",
            with_current.food[0].x,
            without_current.food[0].x
        );

        // 着地済みの餌は水流があっても動かない。
        let mut landed = Simulation::new(Rng::new(961));
        landed.food.push(Food {
            x: 60.0,
            y: sand_top,
            vy: 0.0,
            life: 30.0,
            landed: true,
            sway_phase: 0.0,
        });
        let before = landed.food[0].x;
        for _ in 0..steps {
            landed.current_center_x = 60.0;
            landed.current_center_y = 0.0;
            landed.update_food(dt, sand_top, w);
        }
        assert_eq!(
            landed.food[0].x, before,
            "着地済みの餌は水流の影響を受けないはず"
        );
    }

    #[test]
    fn current_is_weaker_on_a_fish_than_the_undamped_full_strength() {
        // 魚への水流だけCURRENT_FISH_MULTで弱められていることを確認する。強い水流の中で
        // 魚を動かし、実際のx方向の移動量が「力場をそのまま位置に全力で足し込んだ場合の
        // 予測移動量(current_at(...).0 * dt * steps)」より小さいことを見る。半分の強さ+
        // 速度への加算+慣性(ドラッグ)のいずれもが移動量を予測より小さくする。
        let (w, h) = (200usize, 60usize);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let dt = 0.05;
        let steps = 200; // 10秒

        let mut sim = Simulation::new(Rng::new(955));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 100.0, 30.0));
        let start_x = sim.fish[0].x;
        // 中心を魚の真上・渦の減衰半径内の近さに固定 → 左向きの押しになる
        // (CURRENT_FALLOFF_RADIUSより遠いとほぼ無風になるため、十分近くに置く)。
        let near_y = 30.0 - CURRENT_FALLOFF_RADIUS * 0.5;
        sim.current_center_x = 100.0;
        sim.current_center_y = near_y;
        let (cvx, _) = sim.current_at(sim.fish[0].x, sim.fish[0].y);
        for _ in 0..steps {
            sim.fish[0].hunger = MAX_HUNGER;
            sim.current_center_x = 100.0;
            sim.current_center_y = near_y;
            sim.update_movement(dt, w as f64, sand_top);
        }
        let actual_disp = (sim.fish[0].x - start_x).abs();
        let undamped_pred = cvx.abs() * dt * steps as f64;
        assert!(
            actual_disp < undamped_pred,
            "魚の実移動量は全力・無減衰の予測より小さいはず(CURRENT_FISH_MULT<1の効果): actual={actual_disp} pred={undamped_pred}"
        );
    }

    #[test]
    fn current_streaks_spawn_and_are_eventually_removed() {
        // 水流の筋は数秒回せば生成され、長時間回しても寿命・画面外で除去されて
        // 無制限には増えないことを確認する(溜め込みバグの再発防止)。
        let mut sim = Simulation::new(Rng::new(970));
        run(&mut sim, 10.0, 0.1, 100, 50, false);
        assert!(
            !sim.current_streaks.is_empty(),
            "数秒回せば水流の筋が生成されているはず"
        );

        run(&mut sim, 600.0, 0.1, 100, 50, false);
        // 生成間隔の下限(CURRENT_STREAK_SPAWN_INTERVAL_MIN)と寿命(CURRENT_STREAK_LIFETIME)から、
        // 同時存在数はごく少数に収まる。溜め込みの検出には十分余裕のある上限で確認する。
        assert!(
            sim.current_streaks.len() <= 10,
            "水流の筋は寿命・画面外で除去され、無制限には増えないはず: {}",
            sim.current_streaks.len()
        );
    }

    #[test]
    fn fish_do_not_disproportionately_pile_up_along_the_tank_edges_over_a_long_run() {
        // 水流を導入してもなお画面端に魚が滞留するとの再指摘の再発防止テスト。実機相当の
        // 広い水槽(端末幅が大きいほどpix_widthも大きくなり、旧CURRENT_CENTER_MARGIN_FRACの
        // 不具合が顕在化するサイズ)に、壁際から離した中央寄りの位置で魚を格子状に配置し、
        // 長時間(5分ぶん)シミュレートする。その後、壁際の帯(margin_band)に留まっている
        // 生存個体の割合が高すぎないことを確認する(「そのうち端に寄って居座り続けるか」を見る)。
        let (w, h) = (220usize, 90usize);
        let sand_top = (h as f64 - sand_height(h) as f64).max(2.0);
        let mut sim = Simulation::new(Rng::new(3001));

        let species = [Species::Neon, Species::Guppy, Species::Betta];
        let cols = 6usize;
        let rows = 4usize;
        let mut n = 0usize;
        for row in 0..rows {
            for col in 0..cols {
                let x = 30.0 + col as f64 * ((w as f64 - 60.0) / (cols - 1) as f64);
                let y = 15.0 + row as f64 * ((sand_top - 30.0) / (rows - 1) as f64);
                let sp = species[n % species.len()];
                sim.fish.push(Fish::new(sp, Stage::Adult, x, y));
                n += 1;
            }
        }

        // 満腹を維持し、餌探索・産卵行動に気を取られず純粋に遊泳(ランダムウォーク・
        // 群れ・水流・壁反射)だけの挙動を見る。5分(300秒)ぶんをdt=0.2で進める。
        run(&mut sim, 300.0, 0.2, w, h, true);

        let margin_band = 10.0;
        let alive: Vec<&Fish> = sim.fish.iter().filter(|f| !f.dead).collect();
        let near_edge = alive
            .iter()
            .filter(|f| {
                f.x < margin_band
                    || f.x > w as f64 - margin_band
                    || f.y < margin_band
                    || f.y > sand_top - margin_band
            })
            .count();
        let total = alive.len().max(1);
        let ratio = near_edge as f64 / total as f64;
        assert!(
            ratio <= 0.5,
            "長時間シミュレート後、壁際の帯に留まっている生存個体の割合が高すぎる: {near_edge}/{total} (ratio={ratio:.2})"
        );
    }
}
