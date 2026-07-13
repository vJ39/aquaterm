// 魚の定義: 種類・成長段階・ドットマトリクスのスプライト・個体状態。
// 育成ロジック本体(更新・繁殖・死亡判定)は sim.rs 側にある。

use crate::color::Color;
use crate::sim::{
    FULL_THRESHOLD, GENERAL_GROWTH_SCALE_STEP, GENERAL_MAX_GROWTH_STAGE, HUNGRY_THRESHOLD,
    MAX_HUNGER, SHARK_KILL_GROWTH_SCALE_STEP, SHARK_MAX_KILL_STAGE, SIZE_SPEED_PENALTY_STEP,
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
    Neon,     // 小型青系(ネオンテトラ風)。速い・群れやすい
    Goldfish, // オレンジ金魚風。大きめ・ゆったり
    Guppy,    // 白+差し色(グッピー風)。餌への反応が速い
    Shark,    // サメ型の大型種。既存3種と同じ育成ロジックにフル参加し、他の魚を捕食する
}

impl Species {
    // サメを除いた通常種。初期配置(seed_initial)・グレートリセット・`+`キーのランダム
    // 追加はこちらから選ぶ(サメの入手経路は`S`キーのみに限定する方針のため)。
    pub const COMMON: [Species; 3] = [Species::Neon, Species::Goldfish, Species::Guppy];

    // 最高遊泳速度(論理ピクセル/秒)
    pub fn max_speed(self) -> f64 {
        match self {
            Species::Neon => 22.0,
            Species::Goldfish => 13.0,
            Species::Guppy => 18.0,
            Species::Shark => 16.0,
        }
    }

    // ランダムウォークの強さ
    pub fn wander(self) -> f64 {
        match self {
            Species::Neon => 26.0,
            Species::Goldfish => 14.0,
            Species::Guppy => 22.0,
            Species::Shark => 11.0, // 大きくゆったり、より直線的に泳ぐ
        }
    }

    // 餌への吸引の強さ(反応速度)
    pub fn food_pull(self) -> f64 {
        match self {
            Species::Neon => 40.0,
            Species::Goldfish => 30.0,
            Species::Guppy => 55.0,
            Species::Shark => 20.0, // 通常の餌にはあまり反応しない(捕食の方が効率よい)
        }
    }

    // 捕食者かどうか(サメのみ)。sim.rs の捕食ロジックが参照する。
    pub fn is_predator(self) -> bool {
        matches!(self, Species::Shark)
    }

    // 産卵(繁殖)するかどうか。サメは`S`キー以外で増えないようにするため、
    // 産卵→孵化の繁殖ロジックからは除外する。
    pub fn breeds(self) -> bool {
        !matches!(self, Species::Shark)
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
    // サメの捕食クールダウン(0より大きい間は連続捕食しない)
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
    // サメが捕食するたびに増える、捕食由来のサイズ成長段階(0..=SHARK_MAX_KILL_STAGE)
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
            dead: false,
            dead_timer: 0.0,
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
    // サメだけは捕食由来の成長段階(kill_stage)がさらに積み重なる。両方に上限があるので
    // 無限に大きくならない。
    pub fn render_scale(&self) -> f64 {
        let general =
            self.growth_stage.min(GENERAL_MAX_GROWTH_STAGE) as f64 * GENERAL_GROWTH_SCALE_STEP;
        let kill = if matches!(self.species, Species::Shark) {
            self.kill_stage.min(SHARK_MAX_KILL_STAGE) as f64 * SHARK_KILL_GROWTH_SCALE_STEP
        } else {
            0.0
        };
        1.0 + general + kill
    }

    // サイズ成長に応じた泳ぐ速度の減衰率(1.0=減衰なし)。必須ではない体感の変化として、
    // 大きくなるほどわずかに遅くなる。
    pub fn size_speed_mult(&self) -> f64 {
        let stages = self.growth_stage.min(GENERAL_MAX_GROWTH_STAGE) as f64
            + if matches!(self.species, Species::Shark) {
                self.kill_stage.min(SHARK_MAX_KILL_STAGE) as f64
            } else {
                0.0
            };
        (1.0 - SIZE_SPEED_PENALTY_STEP * stages).max(0.6)
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
        let lines: &[&str] = match (species, stage) {
            (Species::Neon, Stage::Fry) => &[".BE", "<BB"],
            (Species::Neon, Stage::Adult) => &[".BBB.", "<BABE", ".BBB."],
            (Species::Goldfish, Stage::Fry) => &[".BBE", "<BBB"],
            (Species::Goldfish, Stage::Adult) => &[".AAA..", "<BBBBE", "<BBBBE", ".AAA.."],
            (Species::Guppy, Stage::Fry) => &[".BE", "<AB"],
            (Species::Guppy, Stage::Adult) => &[".BBB.", "<BBBE", ".AAA."],
            (Species::Shark, Stage::Fry) => &["..A...", "<BBBBE", "..A..."],
            (Species::Shark, Stage::Adult) => &[
                ".....A....",
                "....AAA...",
                "<BBBBBBBBE",
                ".BBBBBBBB.",
                "...A..A...",
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

// 種ごとの色マップ。B=body, A=accent, E=eye, <=tail(bodyと同色)
struct Palette {
    body: Color,
    accent: Color,
    eye: Color,
}

impl Palette {
    fn color(&self, ch: char) -> Option<Color> {
        match ch {
            'B' | '<' => Some(self.body),
            'A' => Some(self.accent),
            'E' => Some(self.eye),
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
        },
        Species::Goldfish => Palette {
            body: Color::new(240, 140, 20),
            accent: Color::new(250, 210, 60),
            eye: Color::new(30, 12, 0),
        },
        Species::Guppy => Palette {
            body: Color::new(235, 235, 240),
            accent: Color::new(230, 70, 120),
            eye: Color::new(20, 20, 40),
        },
        Species::Shark => Palette {
            body: Color::new(90, 105, 120),   // 鋼のような灰青
            accent: Color::new(175, 190, 200), // 腹側の淡い灰白(ヒレ)
            eye: Color::new(10, 10, 15),
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
        },
    )
}
