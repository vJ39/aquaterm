// 魚の定義: 種類・成長段階・ドットマトリクスのスプライト・個体状態。
// 育成ロジック本体(更新・繁殖・死亡判定)は sim.rs 側にある。

use crate::color::Color;
use crate::sim::{FULL_THRESHOLD, HUNGRY_THRESHOLD};
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
}

impl Species {
    pub const ALL: [Species; 3] = [Species::Neon, Species::Goldfish, Species::Guppy];

    // 最高遊泳速度(論理ピクセル/秒)
    pub fn max_speed(self) -> f64 {
        match self {
            Species::Neon => 22.0,
            Species::Goldfish => 13.0,
            Species::Guppy => 18.0,
        }
    }

    // ランダムウォークの強さ
    pub fn wander(self) -> f64 {
        match self {
            Species::Neon => 26.0,
            Species::Goldfish => 14.0,
            Species::Guppy => 22.0,
        }
    }

    // 餌への吸引の強さ(反応速度)
    pub fn food_pull(self) -> f64 {
        match self {
            Species::Neon => 40.0,
            Species::Goldfish => 30.0,
            Species::Guppy => 55.0,
        }
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
    // 空腹度0が続いている時間(死亡判定に使う)
    pub starve_timer: f64,
    // 病気状態
    pub sick: bool,
    // 病気が続いている時間(弱り・死亡判定に使う)
    pub sick_timer: f64,
    // 腹ぺこ状態が続いている時間(発症判定に使う)
    #[serde(default)]
    pub hungry_timer: f64,
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
    }
}
