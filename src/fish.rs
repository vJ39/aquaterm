// 魚の定義: 種類・成長段階・ドットマトリクスのスプライト・個体状態。
// 育成ロジック本体(更新・繁殖・死亡判定)は sim.rs 側にある。

use crate::color::Color;
use crate::sim::{
    AGILITY_FRY_SIZE_STEPS, AGILITY_MULT_MAX, AGILITY_MULT_MIN, AGILITY_STEP, FULL_THRESHOLD,
    GENERAL_GROWTH_SCALE_STEP, GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE, HUNGRY_THRESHOLD, MAX_HUNGER,
    OCTOPUS_BASE_SCALE_BONUS, PIRANHA_KILL_GROWTH_SCALE_STEP, PIRANHA_MAX_KILL_STAGE,
    SIZE_SPEED_PENALTY_STEP,
};
use serde::{Deserialize, Serialize};

// 空腹度の3段階(見た目・挙動に反映)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HungerLevel {
    Full,   // 満腹: ゆったり泳ぐ
    Normal, // 普通
    Hungry, // 腹ぺこ: 速く泳ぎ餌に強く寄る、色が薄暗い
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Species {
    Neon,      // 小型青系(ネオンテトラ風)。速い・群れやすい
    Goldfish,  // オレンジ金魚風。大きめ・ゆったり
    Guppy,     // 白+差し色(グッピー風)。餌への反応が速い
    Piranha,   // 小型でずんぐりしたピラニア型の捕食種。既存3種と同じ育成ロジックにフル参加し、他の魚を捕食する
    Angelfish, // 縦長で優雅な新種。銀白+黒の縞模様、ゆったり泳ぐ
    Betta,     // 派手な長いヒレを持つ新種(ベタ風)。単独行動気味・反応は速い
    Octopus,   // タコ。ピラニアとは別の捕食者。タコつぼに隠れ、時々出てきて泳ぐ(Sキー等の特殊入手扱い)
}

impl Species {
    // 特殊入手種(ピラニア・タコ)を除いた通常種。初期配置(seed_initial)・グレートリセット・
    // `+`キーのランダム追加はこちらから選ぶ。
    pub const COMMON: [Species; 5] = [
        Species::Neon,
        Species::Goldfish,
        Species::Guppy,
        Species::Angelfish,
        Species::Betta,
    ];

    // 最高遊泳速度(論理ピクセル/秒)。生き物の基本移動速度を(シミュレーション再生速度
    // (SPEED_STEPS)とは別に)全体的に4倍にすべきという要望を受けて、
    // 旧基準値(Neon=22.0等)から全種一律4倍にした。既存の倍率(speed_mult()の
    // 空腹度・病気による増減、PIRANHA_CHASE_SPEED_MULT等)はそのまま上に乗る。
    pub fn max_speed(self) -> f64 {
        match self {
            Species::Neon => 88.0,
            Species::Goldfish => 52.0,
            Species::Guppy => 72.0,
            Species::Piranha => 64.0,
            Species::Angelfish => 48.0, // 優雅にゆったり
            Species::Betta => 76.0,     // 単独行動・反応は速い
            Species::Octopus => 56.0,  // 慎重に動く待ち伏せ型
        }
    }

    // ランダムウォークの強さ。max_speed()と同じ基本移動速度4倍化の方針を受けて全種一律4倍。
    pub fn wander(self) -> f64 {
        match self {
            Species::Neon => 104.0,
            Species::Goldfish => 56.0,
            Species::Guppy => 88.0,
            Species::Piranha => 44.0, // 動きは比較的直線的(数値は旧仕様を維持)
            Species::Angelfish => 40.0, // 優雅にゆったり、あまりせわしなく動かない
            Species::Betta => 96.0,     // 気が強く動きが多い
            Species::Octopus => 36.0,  // 普段は物陰でじっとしている慎重な生き物
        }
    }

    // 餌への吸引の強さ(反応速度)。max_speed()と同じ基本移動速度4倍化の方針を受けて
    // 全種一律4倍(HUNGRY_FOOD_PULL_BOOST等の既存の倍率はそのまま上に乗る)。
    pub fn food_pull(self) -> f64 {
        match self {
            Species::Neon => 160.0,
            Species::Goldfish => 120.0,
            Species::Guppy => 220.0,
            Species::Piranha => 80.0, // 通常の餌にはあまり反応しない(捕食の方が効率よい)
            Species::Angelfish => 112.0,
            Species::Betta => 180.0,
            Species::Octopus => 48.0, // 通常の餌にはほぼ反応しない(捕食の方が効率よい)
        }
    }

    // 捕食者かどうか(ピラニア・タコ)。sim.rs の捕食ロジックが参照する。
    pub fn is_predator(self) -> bool {
        matches!(self, Species::Piranha | Species::Octopus)
    }

    // 産卵(繁殖)するかどうか。ピラニア・タコは特殊入手種として、通常の産卵→孵化からは除外する。
    pub fn breeds(self) -> bool {
        !matches!(self, Species::Piranha | Species::Octopus)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Stage {
    Fry,   // 稚魚
    Adult, // 成魚
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fish {
    pub species: Species,
    pub stage: Stage,
    pub hunger: f64, // 0.0(空腹)..100.0(満腹)
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub facing_right: bool,
    // 満腹を維持している時間(成長・繁殖の判定に使う)
    pub well_fed_timer: f64,
    // 空腹度0が続いている時間(弱り・死亡判定に使う)
    pub starve_timer: f64,
    // 病気状態
    pub sick: bool,
    // 病気が続いている時間(弱り・死亡判定に使う)
    pub sick_timer: f64,
    // 腹ぺこ状態が続いている時間(発症判定に使う)
    #[serde(default)]
    pub hungry_timer: f64,
    // 死亡演出中かどうか(true の間は仰向けスプライトで浮上し、育成ロジックの対象外になる)
    #[serde(default)]
    pub dead: bool,
    // 死亡してからの経過時間(一定時間で水槽から消える判定に使う)
    #[serde(default)]
    pub dead_timer: f64,
    // ガラスを叩かれて驚き逃げている残り時間(0より大きい間、逃走方向へ加速する)
    #[serde(default)]
    pub flee_timer: f64,
    // 逃走方向の単位ベクトル(ガラスを叩かれた瞬間に決定)
    #[serde(default)]
    pub flee_dx: f64,
    #[serde(default)]
    pub flee_dy: f64,
    // ピラニアの捕食クールダウン(0より大きい間は連続捕食しない)
    #[serde(default)]
    pub predation_cooldown: f64,
    // ガラスの叩きすぎ(ストレス)による病気発症ボーナスが乗っている残り時間
    #[serde(default)]
    pub stress_timer: f64,
    // 成魚になった後、満腹維持でさらにサイズが大きくなる段階(0..=GENERAL_MAX_GROWTH_STAGE)
    #[serde(default)]
    pub growth_stage: u8,
    // growth_stage の判定専用の満腹維持タイマー(well_fed_timer とは別枠で持つ。
    // 産卵・稚魚成長でのタイマーリセットに影響されないようにするため)
    #[serde(default)]
    pub size_timer: f64,
    // ピラニアが捕食するたびに増える、捕食由来のサイズ成長段階(0..=PIRANHA_MAX_KILL_STAGE)
    #[serde(default)]
    pub kill_stage: u8,
    // 生まれてからの経過時間(秒)。寿命・老齢判定に使う
    #[serde(default)]
    pub age: f64,
    // 老齢に達した瞬間の「最後の産卵」確定イベントを既に消化したかどうか
    #[serde(default)]
    pub elderly_spawned: bool,
    // ランダムな瞬発ダッシュ(特定のトリガーが無い通常時の躍動感演出)の残り時間
    #[serde(default)]
    pub dash_timer: f64,
    #[serde(default)]
    pub dash_dx: f64,
    #[serde(default)]
    pub dash_dy: f64,
    // --- タコ専用(他種は使わない。デフォルトのままで無害) ---
    // タコつぼに隠れているかどうか(隠れている間は非表示・移動しない・捕食対象にならない)
    #[serde(default)]
    pub hidden: bool,
    // 現在の状態(隠れている/出ている)の残り時間。0になると状態が切り替わる
    #[serde(default)]
    pub hidden_timer: f64,
    // タコつぼ(巣)の位置。隠れている間はここに留まり、出ている間も最終的にここへ戻る
    #[serde(default)]
    pub den_x: f64,
    #[serde(default)]
    pub den_y: f64,
    // 墨を吐いた直後のクールダウン(連発防止)
    #[serde(default)]
    pub ink_cooldown: f64,
    // 墨を吐いた直後の緊急脱出時間。この間、緊急ダッシュ(速度ブースト)がかかり、
    // 捕食判定(strike radius)からも一時的に除外される(「墨を吐いたら逃げ切れる」を
    // 結果として保証するための猶予)。
    #[serde(default)]
    pub ink_escape_timer: f64,
    // スター(無敵アイテム)取得後の残り無敵時間。0より大きい間は、誰からも捕食
    // されず、逆に触れた他の魚(ピラニア・タコを含む)を種類に関わらず捕食できる
    // (一時的な捕食者反転ギミック)。
    #[serde(default)]
    pub invincible_timer: f64,
    // `T`キー(トントン)で軽くノックされた直後、興味を持ってその位置へ近づいて
    // いる残り時間。`t`(コンコン)の驚き逃走(flee_timer/flee_dx/flee_dy)と対に
    // なる、引き寄せ側の状態。0より大きい間、attract_dx/dyの方向へ穏やかに加速する。
    #[serde(default)]
    pub attract_timer: f64,
    #[serde(default)]
    pub attract_dx: f64,
    #[serde(default)]
    pub attract_dy: f64,
    // なつき度(0..=AFFINITY_MAX)。`T`(トントン)に反応するたびに少し上昇し、
    // 時間経過でゆっくり減衰する。閾値以上でステータスオーバーレイにマークが出る。
    #[serde(default)]
    pub affinity: f64,
    // なつき度上昇のクールダウン(0より大きい間は`T`に反応しても上昇しない。
    // 連打による瞬時のカンスト防止)。
    #[serde(default)]
    pub affinity_cooldown: f64,
    // --- ピラニア専用(他種は使わない。デフォルトのままで無害) ---
    // 満腹(hunger>=PIRANHA_HUNT_HUNGER_THRESHOLD)になってから捕食した匹数。
    // PIRANHA_KILLS_TO_FULL に達するまでは、満腹相当の空腹度でも狩りをやめない
    // (食欲を旺盛にする)。満腹判定が確定した瞬間に0へ戻す。
    #[serde(default)]
    pub piranha_meals_since_full: u32,
    // piranha_meals_since_fullが1以上PIRANHA_KILLS_TO_FULL未満の間だけ経過時間を計測する
    // タイマー。PIRANHA_QUOTA_GRACE_PERIODを超えても次を捕食できなかった場合、諦めて
    // meals_since_fullを0に戻す(「食欲がなくても無限に追いかけまわす」バグの修正)。
    #[serde(default)]
    pub piranha_quota_timer: f64,
    // --- 個体差(全種共通。同じ種でも個体ごとにばらつく) ---
    // 空腹になる速さの倍率(HUNGER_DECAYに乗算)。1.0が標準、大きいほど早く空腹になる。
    // 旧セーブにフィールドが無い場合も1.0(ニュートラル・挙動不変)にする。
    #[serde(default = "unit_multiplier")]
    pub hunger_decay_mult: f64,
    // 食べた時に満たされる量の倍率(FEED_AMOUNT・捕食hunger_gain・肉餌に乗算)。
    // 1.0が標準、大きいほど1回でしっかり満たされる(いわゆる大食い)。
    #[serde(default = "unit_multiplier")]
    pub feed_efficiency_mult: f64,
    // 寿命(ELDERLY_AGE・LIFESPAN_DEATH_AGE)の倍率。1.0が標準、大きいほど長生きする。
    #[serde(default = "unit_multiplier")]
    pub lifespan_mult: f64,
    // 成長できる上限段階(GENERAL_MAX_GROWTH_STAGE)からのずれ(-1/0/+1)。
    // 旧セーブでは0(ずれ無し・挙動不変)になる。
    #[serde(default)]
    pub growth_cap_variance: i8,
}

// serde(default = ...) 用。0.0ではなく1.0(ニュートラル)を旧セーブの既定値にするための関数。
fn unit_multiplier() -> f64 {
    1.0
}

impl Fish {
    pub fn new(species: Species, stage: Stage, x: f64, y: f64) -> Self {
        Fish {
            species,
            stage,
            hunger: 70.0,
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            facing_right: true,
            well_fed_timer: 0.0,
            starve_timer: 0.0,
            sick: false,
            sick_timer: 0.0,
            hungry_timer: 0.0,
            flee_timer: 0.0,
            flee_dx: 0.0,
            flee_dy: 0.0,
            predation_cooldown: 0.0,
            stress_timer: 0.0,
            growth_stage: 0,
            size_timer: 0.0,
            kill_stage: 0,
            age: 0.0,
            elderly_spawned: false,
            dash_timer: 0.0,
            dash_dx: 0.0,
            dash_dy: 0.0,
            hidden: false,
            hidden_timer: 0.0,
            den_x: 0.0,
            den_y: 0.0,
            ink_cooldown: 0.0,
            ink_escape_timer: 0.0,
            dead: false,
            dead_timer: 0.0,
            invincible_timer: 0.0,
            attract_timer: 0.0,
            attract_dx: 0.0,
            attract_dy: 0.0,
            affinity: 0.0,
            affinity_cooldown: 0.0,
            piranha_meals_since_full: 0,
            piranha_quota_timer: 0.0,
            hunger_decay_mult: 1.0,
            feed_efficiency_mult: 1.0,
            lifespan_mult: 1.0,
            growth_cap_variance: 0,
        }
    }

    // 描画用スプライト(種類×成長段階)
    pub fn sprite(&self) -> Sprite {
        Sprite::for_fish(self.species, self.stage)
    }

    // 空腹度の段階
    pub fn hunger_level(&self) -> HungerLevel {
        if self.hunger >= FULL_THRESHOLD {
            HungerLevel::Full
        } else if self.hunger < HUNGRY_THRESHOLD {
            HungerLevel::Hungry
        } else {
            HungerLevel::Normal
        }
    }

    // 遊泳速度の倍率(満腹はゆったり・腹ぺこは速い・病気は鈍い)
    pub fn speed_mult(&self) -> f64 {
        let base = match self.hunger_level() {
            HungerLevel::Full => 0.72,
            HungerLevel::Normal => 1.0,
            HungerLevel::Hungry => 1.3,
        };
        if self.sick {
            base * 0.5
        } else {
            base
        }
    }

    // 見た目の拡大率(1.0=通常成魚サイズ)。全種共通の成長段階(growth_stage)に、
    // ピラニアだけは捕食由来の成長段階(kill_stage)がさらに積み重なる。両方に上限があるので
    // 無限に大きくならない。
    pub fn render_scale(&self) -> f64 {
        let general =
            self.growth_stage.min(GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE) as f64 * GENERAL_GROWTH_SCALE_STEP;
        let kill = if matches!(self.species, Species::Piranha) {
            self.kill_stage.min(PIRANHA_MAX_KILL_STAGE) as f64 * PIRANHA_KILL_GROWTH_SCALE_STEP
        } else {
            0.0
        };
        // タコはデフォルトで他種より大きく見せたいという要望への対応。成長段階に
        // よるスケールとは別枠の、種固有のベース倍率として加算する。
        let species_bonus = if matches!(self.species, Species::Octopus) {
            OCTOPUS_BASE_SCALE_BONUS
        } else {
            0.0
        };
        1.0 + species_bonus + general + kill
    }

    // 口(頭部前端)のワールド座標。捕食判定を胴体でなく口にすべきという指摘への対応:
    // 捕食判定(strike radius)は魚の中心(胴体)ではなく、進行方向
    // (facing_right)側のスプライト前端=口の位置を基準にする。スプライト全体の
    // 描画幅(render_scale適用後)の半分だけ、向いている方向に中心からずらす
    // (魚は左右方向にしか反転しないため、Y座標は中心のままでよい)。
    pub fn mouth_position(&self) -> (f64, f64) {
        let sprite = self.sprite();
        let half_w = (sprite.width as f64 * self.render_scale()) / 2.0;
        let dx = if self.facing_right { half_w } else { -half_w };
        (self.x + dx, self.y)
    }

    // スター(無敵アイテム)取得中かどうか。無敵中は誰からも捕食されず、逆に
    // 種類に関わらず触れた他の魚を捕食できる(一時的な捕食者反転)。
    pub fn is_invincible(&self) -> bool {
        self.invincible_timer > 0.0
    }

    // サイズ成長に応じた泳ぐ速度の減衰率(1.0=減衰なし)。必須ではない体感の変化として、
    // 大きくなるほどわずかに遅くなる。
    pub fn size_speed_mult(&self) -> f64 {
        let stages = self.growth_stage.min(GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE) as f64
            + if matches!(self.species, Species::Piranha) {
                self.kill_stage.min(PIRANHA_MAX_KILL_STAGE) as f64
            } else {
                0.0
            };
        (1.0 - SIZE_SPEED_PENALTY_STEP * stages).max(0.6)
    }

    // サイズの指標(0.0=通常成魚基準)。稚魚はAGILITY_FRY_SIZE_STEPS段階分小さい扱いにし、
    // 成長段階・(ピラニアのみ)捕食成長段階が上がるほど大きくなる分と同じ軸で表す
    // (render_scale/size_speed_multが使う「段階数」と符号だけ揃えたイメージ)。
    fn size_index(&self) -> f64 {
        let stage_component = if self.stage == Stage::Fry {
            -AGILITY_FRY_SIZE_STEPS
        } else {
            0.0
        };
        let growth_component = self.growth_stage.min(GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE) as f64;
        let kill_component = if matches!(self.species, Species::Piranha) {
            self.kill_stage.min(PIRANHA_MAX_KILL_STAGE) as f64
        } else {
            0.0
        };
        stage_component + growth_component + kill_component
    }

    // 機敏さの倍率(1.0=通常成魚基準)。サイズが小さいほど大きく(キビキビ)、
    // 大きいほど小さくなる(ゆったり)。「大きくなるほど遅くなる」(size_speed_mult)と
    // 対になる形で同じサイズ軸から滑らかに算出する。通常の遊泳(ランダムウォーク+慣性)
    // にだけ使う想定(空腹時の餌吸引・逃走・追跡等の特別なベクトルには使わない)。
    pub fn agility_mult(&self) -> f64 {
        (1.0 - AGILITY_STEP * self.size_index()).clamp(AGILITY_MULT_MIN, AGILITY_MULT_MAX)
    }

    // 元気度(0.0=瀕死 .. 1.0=満点)。空腹度と病気状態を合算した「元気メーター」用の値。
    // 空腹度が高く病気でなければ満点、空腹度が低い/病気だと下がるシンプルな合成。
    pub fn vitality(&self) -> f64 {
        let hunger_ratio = (self.hunger / MAX_HUNGER).clamp(0.0, 1.0);
        if self.sick {
            (hunger_ratio * 0.45).clamp(0.0, 1.0)
        } else {
            hunger_ratio
        }
    }
}

// ドットマトリクスのスプライト。原点は左上、facing で左右反転する。
pub struct Sprite {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<(usize, usize, Color)>, // (dx, dy, color)
}

impl Sprite {
    fn for_fish(species: Species, stage: Stage) -> Sprite {
        // 病気のまだら模様など、魚の構造がはっきり分かるくらい大きくしてほしい
        // (1.5〜2倍程度では不十分)という要望を受けて、既存4種は大幅に拡大・
        // 精細化して描き直した(ヒレ('F')・眼('E')・体の帯('A')が見て取れる解像度)。
        // 新種(エンゼルフィッシュ・ベタ・タコ)も同じ解像度感で追加する。
        let lines: &[&str] = match (species, stage) {
            (Species::Neon, Stage::Fry) => &["..FFF..", ".BBBBB.", "<BBABBE", ".BBBBB.", "..FFF.."],
            (Species::Neon, Stage::Adult) => &[
                ".....FFF.....",
                "...BBBBBBB...",
                "<<BBBBBBBBB..",
                "<BBBAAABBBBBE",
                "<<BBBBBBBBB..",
                "...BBBBBBB...",
                ".....FFF.....",
            ],
            (Species::Goldfish, Stage::Fry) => &[
                "..FFFF..",
                ".BBBBBB.",
                "<BBBBBBE",
                ".BBBBBB.",
                "..FFFF..",
            ],
            // 金魚の見た目が種の特徴を捉えられておらず、もっと金魚らしいシルエットに
            // してほしいという指摘を受けて描き直した。旧パターンは上下端に
            // ヒレの尖りがあるだけの、ほぼ真円のシルエットで「丸いだけ」に見えていた。
            // 尾びれ(F)を左側にまとまった扇状に配置して尾とわかるようにし、
            // 体(B)は丸みのある卵形のまま、頭側(右・目のある側)は尾側より少し
            // すぼめて前後の区別がつくようにした。
            (Species::Goldfish, Stage::Adult) => &[
                "......FF........",
                "....FFBBBB......",
                "...FBBBBBBBB....",
                "..FBBBBBBBBBBB..",
                ".FBBBBBAAAABBBBE",
                "..FBBBBBBBBBBB..",
                "...FBBBBBBBB....",
                "....FFBBBB......",
                "......FF........",
            ],
            (Species::Guppy, Stage::Fry) => &["..FF..", ".BBBB.", "<BABBE", ".BBBB.", "..FF.."],
            // グッピーの見た目をもっと可愛くしてほしいという要望を受けて
            // 描き直した。旧パターンは尾が体と同色('<')で見た目に溶け込んでおり、
            // 蝶ネクタイのような輪郭になっていた。グッピーらしい大きく広がる
            // 扇状の尾びれ(F)を左側にはっきり配置し、体は小さく丸くまとめた。
            (Species::Guppy, Stage::Adult) => &[
                "...FF.......",
                "..FBBBF.....",
                ".FBBBBBF....",
                "FFBBAABBBB.E",
                ".FBBBBBF....",
                "..FBBBF.....",
                "...FF.......",
                "....F.......",
            ],
            // ピラニアらしく見えず卵型のUFOに見えるという指摘を受けて、
            // 背びれが体から連続的に伸びる紡錘形のシルエットに描き直した(背びれが
            // 体から浮いて見えたり、尾びれが下に伸びる脚のように見えていた問題を修正)。
            // 小型でずんぐりした体高のある楕円形+下顎の鋭い歯(A)+銀色の体という
            // 伝統的なピラニアの見た目にする。
            // (受け取ったパターン例は頭部が左向きだったため、既存の「頭部は右向き
            // (facing_right時)」規約に合わせて左右反転して使っている)
            // もっとピラニアらしくしてほしいという要望を受けて再調整。
            // 背びれをA(赤)からF(ヒレ色)に変更し、体の広範囲を覆っていた赤を
            // 頭側の腹(のど元)に絞ることで、「銀色の体+腹に赤みのアクセント」を
            // 誇張しすぎない配色に修正した(赤が多すぎると金魚のように見えてしまう)。
            (Species::Piranha, Stage::Fry) => &[
                ".....FF...",
                ".BBBBBBBB.",
                ".BBBBBBBB<",
                ".BBBBBBBBE",
                ".BBB.AAA<F",
            ],
            (Species::Piranha, Stage::Adult) => &[
                "......FF......",
                ".B...BBBB.....",
                "..BBBBBBBBB...",
                ".BBBBBBBBBBB<.",
                ".BBBBBBBBBBBBE",
                ".BBB..AAAAAB<F",
                "....BBAAABB...",
            ],
            (Species::Angelfish, Stage::Fry) => &[
                "..FF..",
                ".AABA.",
                ".ABBA.",
                "<ABBAE",
                ".ABBA.",
                ".AABA.",
                "..FF..",
            ],
            // エンゼルフィッシュの見た目がタツノオトシゴのように見えてしまっており、
            // もっとエンゼルフィッシュらしい見た目にしてほしいという指摘を受けて描き直した。
            // 旧パターンは中心の体(B)が2列ほどしかなく、縦に伸びるだけの細い線に
            // 見えていた。体幹をしっかり幅を持たせた菱形にし、背びれ・尻びれ(F)を
            // その上下から連続的に長く伸ばすことで、エンゼルフィッシュらしい
            // 「体高があり、上下に長いヒレを引いた」シルエットにした。
            (Species::Angelfish, Stage::Adult) => &[
                "......FF......",
                ".....FFFF.....",
                "....FFBBFF....",
                "...F.BBBB.F...",
                "....BBBBBB....",
                "...BBBBBBBB...",
                "<..BBAABBBB..E",
                "...BBBBBBBB...",
                "....BBBBBB....",
                "...F.BBBB.F...",
                "....FFBBFF....",
                ".....FFFF.....",
                "......FF......",
            ],
            (Species::Betta, Stage::Fry) => &[
                "..FF...",
                ".BBBBF.",
                "<BABBFE",
                ".BBBBF.",
                "..FF...",
            ],
            // ベタの見た目が種の特徴を捉えられていなかったとの指摘を受けて描き直した。
            // 旧パターンは体の中心にaccent(紫)が3行×3列の四角い塊として居座り、
            // 「窓」や「機械のパネル」のように見えていた。accentは腹の小さな
            // 一点だけに絞り、色も紫からベタらしい赤+青の対比に変更した
            // (パレット側のaccentも参照)。周囲のヒレ(F)はそのまま活かし、
            // 「体は小さく、ヒレが大きく優雅に広がる」印象を保つ。
            (Species::Betta, Stage::Adult) => &[
                "......FFFF......",
                "....FBBBBBFF....",
                "..FFBBBBBBBBFF..",
                "<FFBBBBAABBBBBFE",
                "..FFBBBBBBBBFF..",
                "....FBBBBBFF....",
                "......FFFF......",
                "......FF.FF.....",
            ],
            // 提示された具体的なドット絵パターンを元に描き直した。
            // 頭部(丸いドーム型のマント)+大きめの目を左右に、そこから連続して
            // 足の付け根がまとまり、8本の足が波打つように下へ伸びて吸盤(A)が
            // 点在する構成。Fry(稚魚)側は同じ考え方を踏襲しつつ、足を4本に減らして
            // 小さく描いている。
            (Species::Octopus, Stage::Fry) => &[
                "...BBBBB...",
                ".BBBBBBBBB.",
                "BBEBBBBBEBB",
                ".BBBBBBBBB.",
                "BAB.BAB.BAB",
                "AB.BAB.BAB.",
            ],
            (Species::Octopus, Stage::Adult) => &[
                "......BBBBB......",
                "....BBBBBBBBB....",
                "...BEBBBBBBBEB...",
                "..BBEBBBBBBBEBB..",
                "..BBBBBBBBBBBBB..",
                ".BBBBBBBBBBBBBBB.",
                ".BAB.BAB.BAB.BAB.",
                "..B...B...B...B..",
                ".ABA.ABA.ABA.ABA.",
                ".B.B.B.B.B.B.B.B.",
                "AB.BAB.BAB.BAB.BA",
                ".................",
            ],
        };
        Sprite::parse(lines, palette(species))
    }

    // 文字列スプライトを解析する。'.'/' ' は透明。
    fn parse(lines: &[&str], pal: Palette) -> Sprite {
        let height = lines.len();
        let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let mut pixels = Vec::new();
        for (dy, line) in lines.iter().enumerate() {
            for (dx, ch) in line.chars().enumerate() {
                if let Some(c) = pal.color(ch) {
                    pixels.push((dx, dy, c));
                }
            }
        }
        Sprite {
            width,
            height,
            pixels,
        }
    }
}

// 種ごとの色マップ。B=body, A=accent, E=eye, F=fin(ヒレ), <=tail(bodyと同色)
struct Palette {
    body: Color,
    accent: Color,
    eye: Color,
    fin: Color,
}

impl Palette {
    fn color(&self, ch: char) -> Option<Color> {
        match ch {
            'B' | '<' => Some(self.body),
            'A' => Some(self.accent),
            'E' => Some(self.eye),
            'F' => Some(self.fin),
            _ => None, // '.', ' ' 等は透明
        }
    }
}

fn palette(species: Species) -> Palette {
    match species {
        Species::Neon => Palette {
            body: Color::new(40, 120, 230),
            accent: Color::new(90, 230, 240),
            eye: Color::new(12, 12, 30),
            fin: Color::new(140, 210, 245),
        },
        Species::Goldfish => Palette {
            body: Color::new(240, 140, 20),
            accent: Color::new(250, 210, 60),
            eye: Color::new(30, 12, 0),
            // 金魚の見た目が種の特徴を捉えられていなかったとの指摘への対応で、
            // 尾びれの色を体とはっきり区別できる
            // 淡い色に変更(旧255,170,60は体とほぼ同色でヒレの輪郭が見えなかった)。
            fin: Color::new(255, 225, 175),
        },
        Species::Guppy => Palette {
            body: Color::new(235, 235, 240),
            accent: Color::new(230, 70, 120),
            eye: Color::new(20, 20, 40),
            fin: Color::new(240, 170, 200),
        },
        Species::Piranha => Palette {
            body: Color::new(160, 168, 178),  // 銀色系の体
            accent: Color::new(200, 40, 40),  // 腹のあかみ+鋭い歯のアクセント
            eye: Color::new(10, 10, 15),
            fin: Color::new(120, 128, 138),   // 銀色より少し暗いヒレ
        },
        Species::Angelfish => Palette {
            body: Color::new(210, 215, 222),  // 銀白
            accent: Color::new(25, 25, 32),   // 黒の縞模様
            eye: Color::new(10, 10, 12),
            fin: Color::new(180, 190, 200),   // 優雅な長いヒレ
        },
        Species::Betta => Palette {
            body: Color::new(220, 60, 30),   // 鮮やかな赤
            // ベタの見た目が種の特徴を捉えられていなかったとの指摘への対応: 紫の
            // ブロック状のaccentが不自然だったため、赤+青の伝統的なベタ配色に
            // 変更(腹の小さな一点のみに使うので、面積が小さくても目立つ濃い青にする)。
            accent: Color::new(60, 110, 220),
            eye: Color::new(15, 5, 10),
            fin: Color::new(230, 110, 60), // 長く広がるヒレ
        },
        Species::Octopus => Palette {
            body: Color::new(150, 80, 90),    // くすんだ赤茶(タコらしい色)
            accent: Color::new(190, 120, 130), // まだら模様(吸盤・斑点)
            eye: Color::new(15, 8, 10),
            fin: Color::new(130, 65, 75),
        },
    }
}

// --- 観賞用の追加生物(育成ロジックには参加しない。見た目の賑やかしのみ) ---

// カニのスプライト。水底を歩くだけの観賞用(育成ロジック対象外)。
pub fn crab_sprite() -> Sprite {
    let lines: &[&str] = &["AEA", "BBB"];
    Sprite::parse(
        lines,
        Palette {
            body: Color::new(200, 90, 55),
            accent: Color::new(235, 150, 90),
            eye: Color::new(20, 10, 5),
            fin: Color::new(220, 120, 70),
        },
    )
}

// エビのスプライト。カニと同じ位置づけの観賞用背景生物(育成ロジック対象外・
// 捕食対象外・自身も捕食しない)。水底や藻の近くをゆっくり歩く/漂う。
pub fn shrimp_sprite() -> Sprite {
    let lines: &[&str] = &[".AA..", "EBBB<"];
    Sprite::parse(
        lines,
        Palette {
            body: Color::new(235, 170, 165),  // 淡い桜色の体
            accent: Color::new(255, 140, 120), // 背の縞・触角の差し色
            eye: Color::new(20, 10, 5),
            fin: Color::new(235, 170, 165), // 尾(<)は体と同色
        },
    )
}

// タツノオトシゴのスプライト。カニ・エビと同じ位置づけの観賞用背景生物。
// 藻に絡みつくようにゆっくり動き、あまり大きく移動しない。
pub fn seahorse_sprite() -> Sprite {
    let lines: &[&str] = &[".AA.", "EBBA", ".BB.", ".BA.", "..A."];
    Sprite::parse(
        lines,
        Palette {
            body: Color::new(230, 195, 90),   // 黄金色の体
            accent: Color::new(190, 150, 60), // 背の模様・尾の巻き
            eye: Color::new(20, 10, 5),
            fin: Color::new(230, 195, 90), // 使わない(bodyと同色にしておく)
        },
    )
}

// タコつぼ(装飾+タコの巣)のスプライト。水底に置く壺型の静的オブジェクト。
// タコつぼが小さく目立たず、壺らしい形がはっきり分かるサイズにしてほしいという指摘を
// 受けて、開口部(狭い口)・首・肩の張り・丸みのある胴体・すぼまった底までしっかり
// 描き分けた壺(アンフォラ)らしいシルエットに大きく描き直した。
pub fn den_sprite() -> Sprite {
    let lines: &[&str] = &[
        "....AAAAA....",
        ".....BBB.....",
        "...ABBBBBA...",
        ".ABBBBBBBBBA.",
        "ABBBBBBBBBBBA",
        "ABBBBBBBBBBBA",
        ".ABBBBBBBBBA.",
        "...ABBBBBA...",
        "....ABBBA....",
        ".....AAA.....",
    ];
    Sprite::parse(
        lines,
        Palette {
            body: Color::new(110, 70, 55),  // 素焼きの壺らしい茶色
            accent: Color::new(80, 50, 40),  // 縁・開口部の濃い色
            eye: Color::new(0, 0, 0),        // 使わない
            fin: Color::new(0, 0, 0),        // 使わない
        },
    )
}

// 岩(装飾+隠れ場所)のスプライト。水底に置く丸みのある岩塊の静的オブジェクト。
// 藻・岩を魚が隠れられるくらい大きくしてほしいという要望への対応: 魚のスプライトが
// すっぽり収まる大きさの、丸みのある岩塊シルエットにしている。
pub fn rock_sprite() -> Sprite {
    let lines: &[&str] = &[
        "...AAAAAAA...",
        ".ABBBBBBBBBA.",
        "ABBBBBBBBBBBA",
        "BBBBBBBBBBBBB",
        "BBBBBBBBBBBBB",
        "ABBBBBBBBBBBA",
        ".AABBBBBAAA..",
    ];
    Sprite::parse(
        lines,
        Palette {
            body: Color::new(120, 118, 112),  // 灰色の岩肌
            accent: Color::new(80, 78, 74),   // 陰影の濃い灰色
            eye: Color::new(0, 0, 0),         // 使わない
            fin: Color::new(0, 0, 0),         // 使わない
        },
    )
}

// カメオ生物(完全観賞用・低頻度出現・育成ロジック・捕食判定のいずれにも参加しない)。
// ウミガメ: 甲羅+頭部のシルエット。
pub fn turtle_sprite() -> Sprite {
    let lines: &[&str] = &[
        "....AAAA....",
        "..ABBBBBBA..",
        ".BBBBBBBBBB.",
        "EBBBBBBBBBBB",
        ".BBBBBBBBBB.",
        "..A.BBBB.A..",
    ];
    Sprite::parse(
        lines,
        Palette {
            body: Color::new(70, 130, 80),   // 深緑の甲羅
            accent: Color::new(45, 95, 55),  // 甲羅の模様・ヒレの濃い緑
            eye: Color::new(15, 15, 15),
            fin: Color::new(0, 0, 0), // 使わない
        },
    )
}

// クラゲ: 丸いカサ+ゆらめく足(触手)。
pub fn jellyfish_sprite() -> Sprite {
    let lines: &[&str] = &[
        "..AAAAA..",
        ".ABBBBBA.",
        "ABBBBBBBA",
        ".BBBBBBB.",
        "..A.A.A..",
        ".A.A.A.A.",
        "A.A.A.A.A",
        ".A.A.A.A.",
    ];
    Sprite::parse(
        lines,
        Palette {
            body: Color::new(220, 180, 235),  // 淡い紫のカサ
            accent: Color::new(180, 130, 210), // 触手・カサの縁の濃い紫
            eye: Color::new(0, 0, 0),          // 使わない
            fin: Color::new(0, 0, 0),          // 使わない
        },
    )
}
