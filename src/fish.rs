// 魚の定義: 種類・成長段階・ドットマトリクスのスプライト・個体状態。
// 育成ロジック本体(更新・繁殖・死亡判定)は sim.rs 側にある。

use crate::color::Color;
use crate::sim::{
    AGILITY_FRY_SIZE_STEPS, AGILITY_MULT_MAX, AGILITY_MULT_MIN, AGILITY_STEP, FULL_THRESHOLD,
    GENERAL_GROWTH_SCALE_STEP, GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE, HUNGRY_THRESHOLD, MAX_HUNGER,
    OCTOPUS_BASE_SCALE_BONUS, OCTOPUS_BITE_SPEED_MULT, PIRANHA_BITE_SPEED_MULT,
    PIRANHA_KILL_GROWTH_SCALE_STEP,
    PIRANHA_MAX_KILL_STAGE, SIZE_SPEED_PENALTY_STEP, WHALE_BASE_SCALE_BONUS,
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
    Whale,     // クジラ。現実の巨大魚をモチーフにしたネタ枠の特殊入手種(Wキー)。他種よりずば抜けて大きい以外は無害な通常魚として振る舞う
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
            Species::Whale => 28.0,    // 巨体ゆえ全種で最もゆっくり泳ぐ
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
            Species::Whale => 18.0,    // 巨体でゆったり泳ぎ、せわしなく動かない
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
            Species::Whale => 30.0,   // ネタ枠の観賞用巨大魚で、餌にはほとんど反応しない
        }
    }

    // 捕食者かどうか(ピラニア・タコ)。sim.rs の捕食ロジックが参照する。
    pub fn is_predator(self) -> bool {
        matches!(self, Species::Piranha | Species::Octopus)
    }

    // 産卵(繁殖)するかどうか。ピラニア・タコ・クジラは特殊入手種として、通常の産卵→孵化からは除外する。
    pub fn breeds(self) -> bool {
        !matches!(self, Species::Piranha | Species::Octopus | Species::Whale)
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
    // 一度でもつがいの交尾が成立したことがあるかどうか(老齢確定産卵の対象を絞るのに使う)
    #[serde(default)]
    pub has_mated: bool,
    // カーソル近くで叩かれた(つつかれた)死骸かどうか。trueになると浮力を無視して
    // 沈降するだけになる(浮遊時間を待たず、つついてすぐ沈められるようにする要望への対応)。
    #[serde(default)]
    pub sink_forced: bool,
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
    // タコがかじられた回数(0〜4)。5回目のかじられで死亡演出に入る(update_octopus_bites側で判定)。
    #[serde(default)]
    pub octopus_bite_count: u8,
    // 直近のかじられからの経過時間。OCTOPUS_BITE_RECOVER_INTERVALごとに1段階回復する。
    #[serde(default)]
    pub octopus_bite_recover_timer: f64,
    // かじってくる魚が続けて連打しないよう、直近にかじられてからの猶予(秒)。
    // この間は同じ/別のどの魚からも新たなかじり判定を受けない。
    #[serde(default)]
    pub octopus_bite_immunity_timer: f64,
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
    // ピラニアに噛まれた回数(0〜2)。3回目の噛みつきで死亡演出に入る(update_predation側で判定)。
    #[serde(default)]
    pub piranha_bite_count: u8,
    // 直近の被噛みつきからの経過時間。PIRANHA_BITE_RECOVER_INTERVALごとに1段階回復する。
    #[serde(default)]
    pub piranha_bite_recover_timer: f64,
    // 負傷中(piranha_bite_count>0)の間、次に少量の血を滲ませるまでの残り時間。
    #[serde(default)]
    pub bleed_timer: f64,
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
    // --- クジラ専用(他種は使わない。デフォルトのままで無害) ---
    // クジラの死骸が沈み切って水底に着地した瞬間からの経過時間。浮遊中・沈降中は
    // 0のまま計測しない(update_biology側で着地を検知してから加算する)。
    // WHALE_EXPLOSION_DELAYに達すると大爆発する。
    #[serde(default)]
    pub whale_landed_timer: f64,
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
            has_mated: false,
            sink_forced: false,
            dash_timer: 0.0,
            dash_dx: 0.0,
            dash_dy: 0.0,
            hidden: false,
            hidden_timer: 0.0,
            den_x: 0.0,
            den_y: 0.0,
            ink_cooldown: 0.0,
            ink_escape_timer: 0.0,
            octopus_bite_count: 0,
            octopus_bite_recover_timer: 0.0,
            octopus_bite_immunity_timer: 0.0,
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
            piranha_bite_count: 0,
            piranha_bite_recover_timer: 0.0,
            bleed_timer: 0.0,
            hunger_decay_mult: 1.0,
            feed_efficiency_mult: 1.0,
            lifespan_mult: 1.0,
            growth_cap_variance: 0,
            whale_landed_timer: 0.0,
        }
    }

    // 描画用スプライト(種類×成長段階)
    pub fn sprite(&self) -> Sprite {
        Sprite::for_fish(self.species, self.stage, self.growth_stage)
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
        let sick_mult = if self.sick { 0.5 } else { 1.0 };
        // ピラニアに噛まれて負傷しているほど遊泳速度が落ちる(弱るほど逃げ足が遅くなり、
        // 追加で噛まれやすくなる)。噛まれていない個体はインデックス0で倍率1.0=無影響。
        let wound_mult = PIRANHA_BITE_SPEED_MULT[self.piranha_bite_count.min(2) as usize];
        // タコがかじられて弱っているほど遊泳速度が落ちる(弱るほど逃げ足が遅くなる)。
        // かじられていない個体はインデックス0で倍率1.0=無影響。
        let octopus_wound_mult = OCTOPUS_BITE_SPEED_MULT[self.octopus_bite_count.min(4) as usize];
        base * sick_mult * wound_mult * octopus_wound_mult
    }

    // 成長・種固有サイズから決まる、本来欲しい見た目上の総倍率。全種共通の
    // 成長段階(growth_stage)に、ピラニアだけは捕食由来の成長段階(kill_stage)が
    // さらに積み重なる。両方に上限があるので無限に大きくならない。
    fn desired_visual_scale(&self) -> f64 {
        let general =
            self.growth_stage.min(GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE) as f64 * GENERAL_GROWTH_SCALE_STEP;
        let kill = if matches!(self.species, Species::Piranha) {
            self.kill_stage.min(PIRANHA_MAX_KILL_STAGE) as f64 * PIRANHA_KILL_GROWTH_SCALE_STEP
        } else {
            0.0
        };
        // タコ・クジラはデフォルトで他種より大きく見せたいという要望への対応。成長段階に
        // よるスケールとは別枠の、種固有のベース倍率として加算する。クジラはネタ枠の巨大魚
        // として、他のどの種よりもずば抜けて大きい倍率を持たせる。
        let species_bonus = match self.species {
            Species::Octopus => OCTOPUS_BASE_SCALE_BONUS,
            Species::Whale => WHALE_BASE_SCALE_BONUS,
            _ => 0.0,
        };
        1.0 + species_bonus + general + kill
    }

    // growth_stageがBIG_ADULT_GROWTH_STAGEに達すると、通常種はスプライト自体が
    // 基準(growth_stage=0)より一回り大きい専用パターンに切り替わる。この
    // キャンバス自体の拡大分を差し引かないと、「もともと大きい絵をさらに
    // desired_visual_scale倍する」二重拡大になり、非整数倍率の拡大描画と相まって
    // 輪郭が肥大・ガタつく(画面が汚く見える不具合の主因)。
    fn intrinsic_sprite_scale(&self) -> f64 {
        if self.stage == Stage::Fry {
            return 1.0; // Fryは成長段階でスプライトが切り替わらない
        }
        let base = Sprite::for_fish(self.species, self.stage, 0);
        let selected = Sprite::for_fish(self.species, self.stage, self.growth_stage);
        if base.width == 0 || base.height == 0 {
            return 1.0;
        }
        let width_ratio = selected.width as f64 / base.width as f64;
        let height_ratio = selected.height as f64 / base.height as f64;
        width_ratio.max(height_ratio).max(1.0)
    }

    // 実際にラスタライズへ渡す拡大率。高解像度スプライトが最初から持っている
    // 大きさ分を補償し、そこからの不足分だけを追加の拡大でまかなう
    // (1.0未満にはしない=描いたドットを間引いて潰さない)。
    pub fn render_scale(&self) -> f64 {
        (self.desired_visual_scale() / self.intrinsic_sprite_scale()).max(1.0)
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

// この成長段階(growth_stage)以降の通常種(COMMON)成魚は、低解像度の
// 拡大ではなく専用の高解像度パターンに切り替える。全種で同じ段階で
// 切り替えて見た目の一貫性を保つ(fish.rs内でのみ使う定数)。
const BIG_ADULT_GROWTH_STAGE: u8 = 2;

// 「完全に成長しきった」個体(成長段階が最大値に達した個体)だけを対象に、
// BIG_ADULT_GROWTH_STAGE以降の高解像度パターンよりさらに一段精細な専用
// パターンに切り替える閾値。growth_stageが取り得る最大値(sim.rs側の
// GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE)そのものを使う(この段階を
// 超えるgrowth_stageは存在しないため、独自の値を別途持つ必要はない)。
// 対象は既にBIG_ADULT高解像度パターンを持つ通常種のうちネオン・金魚・
// グッピー・ベタの4種のみ。エンゼルフィッシュは縦縞の連続性が崩れて
// 見える懸念があり、実際にレンダリングして確認した上で今回は対象外とした。
const MAX_ADULT_GROWTH_STAGE: u8 = GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE;

// ドットマトリクスのスプライト。原点は左上、facing で左右反転する。
pub struct Sprite {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<(usize, usize, Color)>, // (dx, dy, color)
}

impl Sprite {
    fn for_fish(species: Species, stage: Stage, growth_stage: u8) -> Sprite {
        // 病気のまだら模様など、魚の構造がはっきり分かるくらい大きくしてほしい
        // (1.5〜2倍程度では不十分)という要望を受けて、既存4種は大幅に拡大・
        // 精細化して描き直した(ヒレ('F')・眼('E')・体の帯('A')が見て取れる解像度)。
        // 新種(エンゼルフィッシュ・ベタ・タコ)も同じ解像度感で追加する。
        let lines: &[&str] = match (species, stage) {
            // ネオン・グッピーのシルエットを「もっとシュッと(streamlined)させたい」
            // という要望を受けて描き直した(2026/07/16)。過去に金魚・ベタで
            // 「尾ビレ(菱形)と胴体(丸)を別々の塊として点でつなぐ」構成にして
            // 不評だった反省を踏まえ、今回は必ず一続きの輪郭(体から尾へ滑らかに
            // テーパーする紡錘形)で描く。尾は胴体の幅が列ごとに連続的に狭まって
            // いった先に生える切れ込み(フォーク)として表現し、独立した塊を後から
            // 接着しない。ネオンテトラは体高がありつつも尾に向けて素早く絞り込む
            // 魚雷型で、尾の切れ込みは浅いフォーク、体側には太いアクセント帯(A)を
            // 面で塗って通す。
            (Species::Neon, Stage::Fry) => &[
                "FF.BB....",
                ".FBBBBB..",
                "..BAABBBE",
                ".FBBBBB..",
                "FF.BB....",
            ],
            // 完全に成長しきった(MAX_ADULT_GROWTH_STAGE)個体だけは、BIG_ADULTの
            // 高解像度パターンよりもさらに一段精細な専用パターンに切り替える。
            // 同じ紡錘形+浅いフォーク尾の構図をそのまま、行・列を約1.22倍に
            // 増やして描き直したもの(2倍だと基のスプライトごと大きくなりすぎるため
            // 控えめに留めている)。この判定はBIG_ADULT判定より先に書く必要がある
            // (growth_stageがMAX_ADULT_GROWTH_STAGEの個体はBIG_ADULTの条件も
            // 満たすため、matchの順序がそのまま優先順位になる)。
            (Species::Neon, Stage::Adult) if growth_stage >= MAX_ADULT_GROWTH_STAGE => &[
                "FF.....FFFFF..........",
                "FFFF.BBBBBBBBBB.......",
                ".FFFBBBBBBBBBBBBB.....",
                ".FFFBBBBBBBBBBBBB.....",
                "..FFBBBAAAAAAAABBBBB..",
                "....BBBAAAAAAAABBBBBBE",
                "..FFBBBAAAAAAAABBBBB..",
                ".FFFBBBBBBBBBBBBB.....",
                ".FFFBBBBBBBBBBBBB.....",
                "FFFF.BBBBBBBBBB.......",
                "FF.....FFFFF..........",
            ],
            // 成長段階が上がって大きく表示されるほど、低解像度パターンの拡大では
            // 模様が潰れて間延びする。BIG_ADULT_GROWTH_STAGE以降は、同じ紡錘形の
            // 構図を一回り高い解像度で描き直した専用パターンに切り替える。
            (Species::Neon, Stage::Adult) if growth_stage >= BIG_ADULT_GROWTH_STAGE => &[
                "FF....FFFF........",
                "FFF.BBBBBBBB......",
                ".FFBBBBBBBBBBB....",
                "..FBBBAAAAAABBBB..",
                "...BBBAAAAAABBBBBE",
                "..FBBBAAAAAABBBB..",
                ".FFBBBBBBBBBBB....",
                "FFF.BBBBBBBB......",
                "FF....FFFF........",
            ],
            (Species::Neon, Stage::Adult) => &[
                "FF...BBB......",
                ".FF.BBBBBB....",
                "..FBBAAAAABB..",
                "...BBAAAAABBBE",
                "..FBBAAAAABB..",
                ".FF.BBBBBB....",
                "FF...BBB......",
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
            // 完全に成長しきった(MAX_ADULT_GROWTH_STAGE)個体専用の、BIG_ADULTより
            // さらに一段精細なパターン(Neonと同じ約1.22倍・行列を増やしただけで
            // シルエット自体は変えていない)。
            (Species::Goldfish, Stage::Adult) if growth_stage >= MAX_ADULT_GROWTH_STAGE => &[
                "......FFFFF...............",
                "....FFFFFBBBBBBBB.........",
                "..FFFFBBBBBBBBBBBBBB......",
                "..FFFFBBBBBBBBBBBBBB......",
                ".FFFBBBBBBBBBBBBBBBBBB....",
                ".FBBBBBBBBBBBBBBBBBBBBBB..",
                "FBBBBBBBBBBAAAAAABBBBBBBE.",
                "FBBBBBBBBBBAAAAAABBBBBBBE.",
                "FBBBBBBBBBBAAAAAABBBBBBBE.",
                ".FBBBBBBBBBBBBBBBBBBBBBB..",
                ".FFFBBBBBBBBBBBBBBBBBB....",
                "..FFFFBBBBBBBBBBBBBB......",
                "..FFFFBBBBBBBBBBBBBB......",
                "....FFFFFBBBBBBBB.........",
                "......FFFFF...............",
            ],
            // BIG_ADULT_GROWTH_STAGE以降は高解像度の専用パターンに切り替える。
            // 左に扇状の尾びれ(F)、丸みのある卵形の体(B)、右の目(E)という
            // 金魚らしいシルエットはそのままに、腹のアクセント(A)を一回り
            // 大きくして拡大表示でも見えるようにした。
            (Species::Goldfish, Stage::Adult) if growth_stage >= BIG_ADULT_GROWTH_STAGE => &[
                ".....FFFF............",
                "...FFFFBBBBBBB.......",
                "..FFFBBBBBBBBBBB.....",
                ".FFBBBBBBBBBBBBBBB...",
                ".FBBBBBBBBBBBBBBBBB..",
                "FBBBBBBBBAAAAABBBBBE.",
                "FBBBBBBBBAAAAABBBBBE.",
                ".FBBBBBBBBBBBBBBBBB..",
                ".FFBBBBBBBBBBBBBBB...",
                "..FFFBBBBBBBBBBB.....",
                "...FFFFBBBBBBB.......",
                ".....FFFF............",
            ],
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
            // ネオンと同じ理由(2026/07/16)でグッピーも描き直した。グッピー最大の
            // 特徴である扇状の尾ビレは、胴体側から列ごとに幅がなめらかに増えていく
            // 一枚のウェッジ(くさび形)として描き、独立した扇の塊を体へ点で接着する
            // 構成には絶対にしない。尾から頭まで幅が単調に変化する一続きの輪郭を
            // 保つことで、体高の高い扇尾と細い胴体が自然に繋がって見える。
            (Species::Guppy, Stage::Fry) => &[
                "FF.B....",
                "FFFBBB..",
                "FFFBABBE",
                "FFFBBB..",
                "FF.B....",
            ],
            // 完全に成長しきった(MAX_ADULT_GROWTH_STAGE)個体専用の、BIG_ADULTより
            // さらに一段精細なパターン(Neonと同じ約1.22倍・行列を増やしただけで
            // シルエット自体は変えていない)。
            (Species::Guppy, Stage::Adult) if growth_stage >= MAX_ADULT_GROWTH_STAGE => &[
                "FF........FF............",
                "FFFFF...BBBBBB..........",
                "FFFFFFBBBBBBBBBBB.......",
                "FFFFFFBBBBBBBBBBB.......",
                "FFFFFFBBBBBBBBBBBBB.....",
                "FFFFFFBBAAAAABBBBBBBBB..",
                "FFFFFFBBAAAAABBBBBBBBBBE",
                "FFFFFFBBAAAAABBBBBBBBB..",
                "FFFFFFBBBBBBBBBBBBB.....",
                "FFFFFFBBBBBBBBBBB.......",
                "FFFFFFBBBBBBBBBBB.......",
                "FFFFF...BBBBBB..........",
                "FF........FF............",
            ],
            // BIG_ADULT_GROWTH_STAGE以降は高解像度の専用パターンに切り替える。
            (Species::Guppy, Stage::Adult) if growth_stage >= BIG_ADULT_GROWTH_STAGE => &[
                "FF......FF..........",
                "FFFF...BBBBB........",
                "FFFFFBBBBBBBBB......",
                "FFFFFBBBBBBBBBBB....",
                "FFFFFBBAAAABBBBBBB..",
                "FFFFFBBAAAABBBBBBBBE",
                "FFFFFBBAAAABBBBBBB..",
                "FFFFFBBBBBBBBBBB....",
                "FFFFFBBBBBBBBB......",
                "FFFF...BBBBB........",
                "FF......FF..........",
            ],
            (Species::Guppy, Stage::Adult) => &[
                "FF.....FF.......",
                "FFFF..BBBBB.....",
                "FFFFFBBBBBBBB...",
                "FFFFFBAAAAABBB..",
                "FFFFFBAAAAABBBBE",
                "FFFFFBAAAAABBB..",
                "FFFFFBBBBBBBB...",
                "FFFF..BBBBB.....",
                "FF.....FF.......",
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
                ".BB..BBBB.....",
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
            // 縦縞模様(A)がほぼ2ドットしかなく見えづらいとの指摘を受けて、
            // 体の縦方向に3本の縦縞が通るよう(A)を各行へ配置し直した(体高が
            // 狭い頭側・尾側の行では縞の本数が自然に減り、中心付近の広い行で
            // 3本とも見える)。
            // 成長段階が上がって大きく表示されるほど、この低解像度の模様を
            // 拡大するだけでは間延びしてエンゼルフィッシュらしく見えなくなる
            // との指摘への対応。BIG_ADULT_GROWTH_STAGE以降は、同じ縦縞の構図を
            // 一回り高い解像度で描き直した専用パターンに切り替える(縞・ヒレの
            // 要素自体を描き込むことで拡大後も模様が潰れないようにする)。
            // 解像度は通常パターンの縦横2倍まで上げると基のスプライトごと
            // 大きくなりすぎるため、通常の1.3倍前後に抑えている。
            (Species::Angelfish, Stage::Adult) if growth_stage >= BIG_ADULT_GROWTH_STAGE => &[
                "........FF........",
                ".......FFFF.......",
                "......FFFFFF......",
                ".....FFBBABFF.....",
                "....F.ABBABB.F....",
                ".....BABBABBA.....",
                "....BBABBABBAB....",
                "....BBABBABBAB....",
                "<<.BBBABBABBABB..E",
                "....BBABBABBAB....",
                "....BBABBABBAB....",
                ".....BABBABBA.....",
                "....F.ABBABB.F....",
                ".....FFBBABFF.....",
                "......FFFFFF......",
                ".......FFFF.......",
                "........FF........",
            ],
            (Species::Angelfish, Stage::Adult) => &[
                "......FF......",
                ".....FFFF.....",
                "....FFBAFF....",
                "...F.ABAB.F...",
                "....BABABA....",
                "...BBABABAB...",
                "<..BBABABAB..E",
                "...BBABABAB...",
                "....BABABA....",
                "...F.ABAB.F...",
                "....FFBAFF....",
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
            // 完全に成長しきった(MAX_ADULT_GROWTH_STAGE)個体専用の、BIG_ADULTより
            // さらに一段精細なパターン(Neonと同じ約1.22倍・行列を増やしただけで
            // シルエット自体は変えていない)。
            (Species::Betta, Stage::Adult) if growth_stage >= MAX_ADULT_GROWTH_STAGE => &[
                "........FFFFFF..........",
                "......FFFFFFFFFFF.......",
                "....FFFBBBBBBBBBBFF.....",
                "....FFFBBBBBBBBBBFF.....",
                "..FFFBBBBBBBBBBBBBBFFF..",
                ".FFFBBBBBBBBBBBBBBBBFFF.",
                "<FFFBBBBBBBAABBBBBBBFFFE",
                ".FFFBBBBBBBBBBBBBBBBFFF.",
                "..FFFBBBBBBBBBBBBBBFFF..",
                "....FFFBBBBBBBBBBFF.....",
                "....FFFBBBBBBBBBBFF.....",
                "......FFFFFFFFFFF.......",
                ".......FFFF..FFFF.......",
            ],
            // BIG_ADULT_GROWTH_STAGE以降は高解像度の専用パターンに切り替える。
            // ベタの見どころは大きく優雅に広がるヒレ(F)なので、上下・周囲に
            // 流れるヒレを一回り大きく描き、体(B)は中庸に、中央のアクセント(A)は
            // 控えめな一点のまま保つ。左に尾(<)、右に目(E)。
            (Species::Betta, Stage::Adult) if growth_stage >= BIG_ADULT_GROWTH_STAGE => &[
                ".......FFFFF........",
                ".....FFFFFFFFF......",
                "...FFFBBBBBBBBFF....",
                "..FFBBBBBBBBBBBBFF..",
                ".FFBBBBBBBBBBBBBBFF.",
                "<FFBBBBBBAABBBBBBFFE",
                ".FFBBBBBBBBBBBBBBFF.",
                "..FFBBBBBBBBBBBBFF..",
                "...FFFBBBBBBBBFF....",
                ".....FFFFFFFFF......",
                "......FFF..FFF......",
            ],
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
            // クジラ(ネタ枠の巨大魚)。全種で最も横に長く大きいシルエットにする。
            // 左端に上下へ大きく開く尾びれ(<)、中央上部に小さな背びれの隆起(F)、
            // 右側(頭)寄りに目(E)、腹側に淡い色の帯(A)を配した紡錘形の胴体。
            // Fry(稚魚)側は同じ形の考え方で小さく描く。
            (Species::Whale, Stage::Fry) => &[
                "......FFF......",
                "<....BBBBBBB...",
                "<<.BBBBBBBBBB..",
                ".<<BBBBBBBBBEB.",
                "<<.BBBBBBBBBB..",
                "<....BAAAABB...",
            ],
            (Species::Whale, Stage::Adult) => &[
                "<.............FFF..........",
                "<<...........FFFFF.........",
                ".<<......BBBBBBBBBBBBBB....",
                ".<<....BBBBBBBBBBBBBBBBB...",
                "..<<.BBBBBBBBBBBBBBBBBBBB..",
                "..<<BBBBBBBBBBBBBBBBBBBEBB.",
                "..<<BBBBBBBBBBBBBBBBBBBBBB.",
                "..<<BBBBBBBBBBBBBBBBBBBBBB.",
                "..<<.BBBBAAAAAAAAAAAABBBB..",
                ".<<....BBAAAAAAAAAABBBBB...",
                ".<<......BBBBBBBBBBBBBB....",
                "<<.........................",
                "<..........................",
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
        Species::Whale => Palette {
            body: Color::new(55, 70, 90),     // 濃い青灰色(クジラらしい体色)
            accent: Color::new(200, 205, 210), // 淡い灰白色の腹側
            eye: Color::new(10, 12, 16),
            fin: Color::new(42, 54, 72),      // 体よりわずかに暗いヒレ
        },
    }
}

// 種ごとのヒレ('F')色を返す。Sprite::pixelsは解決済みの色しか持たないため、
// 描画側(main.rs)でヒレピクセルかどうかを判別したいとき、このパレット色と
// 一致するかどうかで判定する(Spriteにフラグを追加する大掛かりな変更を避けるため)。
pub fn fin_color(species: Species) -> Color {
    palette(species).fin
}

// --- 観賞用の追加生物(育成ロジックには参加しない。見た目の賑やかしのみ) ---

// カニのスプライト。水底を歩くだけの観賞用(育成ロジック対象外)。
// 旧パターンは3x2の無地の四角(AEA/BBB)で、脚も爪も無く「カニ」と分かる
// 手がかりが目にしか無かったため、両脇に伸びる爪(A)と甲羅下の脚(F)を
// 追加してカニらしいシルエットにした。
pub fn crab_sprite() -> Sprite {
    let lines: &[&str] = &[".A...A.", "AEBBBEA", ".F.F.F."];
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
        ".A.A.A.A.",
        ".A.A.A.A.",
        ".A.A...A.",
        ".A.....A.",
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

#[cfg(test)]
mod tests {
    use super::*;

    // 通常種(COMMON)の成魚は、BIG_ADULT_GROWTH_STAGE以降で専用の高解像度
    // パターンに切り替わり、基準(growth_stage=0)より一回り大きい描画キャンバスに
    // なるはず。ただし基のスプライトごと大きくなりすぎないよう、縦横ともに
    // 2倍未満(通常の1.3〜1.5倍程度)に収めていることも確認する。
    #[test]
    fn common_species_switch_to_a_bigger_but_not_oversized_adult_sprite() {
        for &sp in &Species::COMMON {
            let mut base = Fish::new(sp, Stage::Adult, 0.0, 0.0);
            base.growth_stage = 0;
            let base_sprite = base.sprite();

            let mut big = Fish::new(sp, Stage::Adult, 0.0, 0.0);
            big.growth_stage = BIG_ADULT_GROWTH_STAGE;
            let big_sprite = big.sprite();

            assert!(
                big_sprite.width > base_sprite.width && big_sprite.height > base_sprite.height,
                "{sp:?}: 大サイズ成魚は基準より幅・高さともに大きいはず (base={}x{}, big={}x{})",
                base_sprite.width, base_sprite.height, big_sprite.width, big_sprite.height
            );
            assert!(
                big_sprite.width < base_sprite.width * 2 && big_sprite.height < base_sprite.height * 2,
                "{sp:?}: 大サイズ成魚は縦横2倍未満に収めるはず (base={}x{}, big={}x{})",
                base_sprite.width, base_sprite.height, big_sprite.width, big_sprite.height
            );
        }
    }

    // 高解像度スプライトへの切り替え直後(growth_stage=BIG_ADULT_GROWTH_STAGE)は、
    // スプライト自体がすでに大きいので、追加の拡大描画はほぼ不要になるはず
    // (intrinsic_sprite_scaleでの相殺が効いていることの確認)。
    #[test]
    fn high_resolution_sprite_is_not_scaled_twice() {
        for &sp in &Species::COMMON {
            let mut fish = Fish::new(sp, Stage::Adult, 0.0, 0.0);
            fish.growth_stage = BIG_ADULT_GROWTH_STAGE;
            assert!(
                fish.render_scale() <= 1.1,
                "{sp:?}: 高解像度スプライトへ切り替えた直後は追加拡大をほぼ行わないはず: {}",
                fish.render_scale()
            );
        }
    }

    // 完全に成長しきった(MAX_ADULT_GROWTH_STAGE)個体を持つ4種(ネオン・金魚・
    // グッピー・ベタ)は、BIG_ADULT_GROWTH_STAGEの高解像度パターンよりさらに
    // 一回り大きい専用パターンに切り替わるはず。ただし基準(growth_stage=0)から
    // 大きくなりすぎないよう、縦横ともに2倍未満に収めていることも確認する
    // (2倍だと図鑑のグリッド計算が壊れるという過去の教訓を踏まえた上限)。
    #[test]
    fn max_adult_species_switch_to_an_even_bigger_but_not_oversized_sprite() {
        for &sp in &[Species::Neon, Species::Goldfish, Species::Guppy, Species::Betta] {
            let mut base = Fish::new(sp, Stage::Adult, 0.0, 0.0);
            base.growth_stage = 0;
            let base_sprite = base.sprite();

            let mut big = Fish::new(sp, Stage::Adult, 0.0, 0.0);
            big.growth_stage = BIG_ADULT_GROWTH_STAGE;
            let big_sprite = big.sprite();

            let mut max_adult = Fish::new(sp, Stage::Adult, 0.0, 0.0);
            max_adult.growth_stage = MAX_ADULT_GROWTH_STAGE;
            let max_sprite = max_adult.sprite();

            assert!(
                max_sprite.width > big_sprite.width && max_sprite.height > big_sprite.height,
                "{sp:?}: 完全成長個体はBIG_ADULTより幅・高さともに大きいはず (big={}x{}, max={}x{})",
                big_sprite.width, big_sprite.height, max_sprite.width, max_sprite.height
            );
            assert!(
                max_sprite.width < base_sprite.width * 2 && max_sprite.height < base_sprite.height * 2,
                "{sp:?}: 完全成長個体も基準の縦横2倍未満に収めるはず (base={}x{}, max={}x{})",
                base_sprite.width, base_sprite.height, max_sprite.width, max_sprite.height
            );
        }
    }

    // 完全成長個体の高解像度スプライトへの切り替え直後(growth_stage=
    // MAX_ADULT_GROWTH_STAGE)も、BIG_ADULTの場合と同じく、スプライト自体が
    // すでに大きいので追加の拡大描画はほぼ不要になるはず(intrinsic_sprite_scale
    // がこの新しい段階でも正しく相殺できていることの確認)。
    #[test]
    fn max_adult_high_resolution_sprite_is_not_scaled_twice() {
        for &sp in &[Species::Neon, Species::Goldfish, Species::Guppy, Species::Betta] {
            let mut fish = Fish::new(sp, Stage::Adult, 0.0, 0.0);
            fish.growth_stage = MAX_ADULT_GROWTH_STAGE;
            assert!(
                fish.render_scale() <= 1.1,
                "{sp:?}: 完全成長個体の高解像度スプライトへ切り替えた直後は追加拡大をほぼ行わないはず: {}",
                fish.render_scale()
            );
        }
    }

    // エンゼルフィッシュは、縦縞の連続性が崩れて見える懸念があるため今回の
    // MAX_ADULT_GROWTH_STAGE専用パターンの対象外にした。growth_stageが
    // MAX_ADULT_GROWTH_STAGEに達しても、BIG_ADULT_GROWTH_STAGEと全く同じ
    // スプライト(次元・ピクセル内容とも)が返り続けることを確認する
    // (対象外であることの回帰テスト)。
    #[test]
    fn angelfish_is_excluded_from_the_max_adult_tier() {
        let mut big = Fish::new(Species::Angelfish, Stage::Adult, 0.0, 0.0);
        big.growth_stage = BIG_ADULT_GROWTH_STAGE;
        let big_sprite = big.sprite();

        let mut max_adult = Fish::new(Species::Angelfish, Stage::Adult, 0.0, 0.0);
        max_adult.growth_stage = MAX_ADULT_GROWTH_STAGE;
        let max_sprite = max_adult.sprite();

        assert_eq!(
            (big_sprite.width, big_sprite.height),
            (max_sprite.width, max_sprite.height),
            "エンゼルフィッシュはMAX_ADULT_GROWTH_STAGEでもBIG_ADULTと同じ寸法のはず(対象外)"
        );
        assert_eq!(
            big_sprite.pixels, max_sprite.pixels,
            "エンゼルフィッシュはMAX_ADULT_GROWTH_STAGEでもBIG_ADULTと同じピクセル内容のはず(対象外)"
        );
    }

    // render_scaleは常に1.0以上でなければならない(1.0未満だとドットが間引かれて
    // スプライトが潰れる)。全種・全成長段階で下回らないことを確認する。
    #[test]
    fn render_scale_never_downsamples_any_species() {
        for sp in [
            Species::Neon,
            Species::Goldfish,
            Species::Guppy,
            Species::Piranha,
            Species::Angelfish,
            Species::Betta,
            Species::Octopus,
            Species::Whale,
        ] {
            for growth_stage in 0..=GENERAL_MAX_GROWTH_STAGE_WITH_VARIANCE {
                let mut fish = Fish::new(sp, Stage::Adult, 0.0, 0.0);
                fish.growth_stage = growth_stage;
                assert!(
                    fish.render_scale() >= 1.0,
                    "{sp:?} growth_stage={growth_stage}: render_scaleは1.0を下回ってはいけない: {}",
                    fish.render_scale()
                );
            }
        }
    }
}
