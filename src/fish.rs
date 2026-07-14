// 魚の定義: 種類・成長段階・ドットマトリクスのスプライト・個体状態。
// 育成ロジック本体(更新・繁殖・死亡判定)は sim.rs 側にある。

use crate::color::Color;
use crate::sim::{
    AGILITY_FRY_SIZE_STEPS, AGILITY_MULT_MAX, AGILITY_MULT_MIN, AGILITY_STEP, FULL_THRESHOLD,
    GENERAL_GROWTH_SCALE_STEP, GENERAL_MAX_GROWTH_STAGE, HUNGRY_THRESHOLD, MAX_HUNGER,
    PIRANHA_KILL_GROWTH_SCALE_STEP, PIRANHA_MAX_KILL_STAGE, SIZE_SPEED_PENALTY_STEP,
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

    // 最高遊泳速度(論理ピクセル/秒)。実機フィードバック(「生き物の基本移動速度を
    // 4倍にしてほしい。シミュレーション再生速度(SPEED_STEPS)とは別物」)を受けて、
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

    // ランダムウォークの強さ。max_speed()と同じ実機フィードバックを受けて全種一律4倍。
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

    // 餌への吸引の強さ(反応速度)。max_speed()と同じ実機フィードバックを受けて
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
            self.growth_stage.min(GENERAL_MAX_GROWTH_STAGE) as f64 * GENERAL_GROWTH_SCALE_STEP;
        let kill = if matches!(self.species, Species::Piranha) {
            self.kill_stage.min(PIRANHA_MAX_KILL_STAGE) as f64 * PIRANHA_KILL_GROWTH_SCALE_STEP
        } else {
            0.0
        };
        1.0 + general + kill
    }

    // 口(頭部前端)のワールド座標。実機フィードバック(「捕食判定を胴体でなく口に」)
    // 対応: 捕食判定(strike radius)は魚の中心(胴体)ではなく、進行方向
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
        let stages = self.growth_stage.min(GENERAL_MAX_GROWTH_STAGE) as f64
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
        let growth_component = self.growth_stage.min(GENERAL_MAX_GROWTH_STAGE) as f64;
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
        // 実機フィードバック(「病気のまだら模様など、魚の構造がはっきり分かるくらい
        // 大きくしてほしい。1.5〜2倍程度では不十分」)を受けて、既存4種は大幅に拡大・
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
            (Species::Goldfish, Stage::Adult) => &[
                ".....FFFFF.....",
                "...BBBBBBBBB...",
                "..BBBBBBBBBBB..",
                "<<BBBAAAABBBBB.",
                "<BBBBBAAAABBBBE",
                "<<BBBAAAABBBBB.",
                "..BBBBBBBBBBB..",
                "...BBBBBBBBB...",
                ".....FFFFF.....",
            ],
            (Species::Guppy, Stage::Fry) => &["..FF..", ".BBBB.", "<BABBE", ".BBBB.", "..FF.."],
            (Species::Guppy, Stage::Adult) => &[
                "....FFF.....",
                "...BBBBB....",
                "<<BBBBBBB...",
                "<<BAAABBBBBE",
                "<<BBBBBBB...",
                "...BBBBB....",
                "....FFF.....",
                "....FFF.....",
            ],
            // 実機フィードバック(「ピラニアっぽくない。卵型のUFOに見える」)を受けて、
            // 背びれが体から連続的に伸びる紡錘形のシルエットに描き直した(背びれが
            // 体から浮いて見えたり、尾びれが下に伸びる脚のように見えていた問題を修正)。
            // 方針転換(「サメではなくピラニアにしよう」)を受けて描き直した。既存の
            // 大型・紡錘形のサメのシルエットではなく、小型でずんぐりした体高のある
            // 楕円形+下顎の鋭い歯(A)+銀色の体という伝統的なピラニアの見た目にする。
            // (受け取ったパターン例は頭部が左向きだったため、既存の「頭部は右向き
            // (facing_right時)」規約に合わせて左右反転して使っている)
            // 実機フィードバック(「もっとピラニアらしくして」)を受けて再調整。
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
            (Species::Angelfish, Stage::Adult) => &[
                "...FF......",
                "..FBBF.....",
                ".F.BB.F....",
                "..ABBA.....",
                "<.ABBBA...E",
                "..ABBA.....",
                ".F.BB.F....",
                "..FBBF.....",
                "...FF......",
                "....F......",
                "....F......",
            ],
            (Species::Betta, Stage::Fry) => &[
                "..FF...",
                ".BBBBF.",
                "<BABBFE",
                ".BBBBF.",
                "..FF...",
            ],
            (Species::Betta, Stage::Adult) => &[
                "......FFFF......",
                "....FBBBBBFF....",
                "..FFBBAAABBBFF..",
                "<FFBBBAAABBBBBFE",
                "..FFBBAAABBBFF..",
                "....FBBBBBFF....",
                "......FFFF......",
                "......FF.FF.....",
            ],
            // 実機フィードバック(具体的なドット絵パターンを受領)を元に描き直した。
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
            fin: Color::new(255, 170, 60),
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
            body: Color::new(220, 60, 30),    // 鮮やかな赤
            accent: Color::new(170, 40, 190), // 派手な紫
            eye: Color::new(15, 5, 10),
            fin: Color::new(230, 110, 60),    // 長く広がるヒレ
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

// タコつぼ(装飾+タコの巣)のスプライト。水底に置く壺型の静的オブジェクト。
// 実機フィードバック(「小さく目立たなかった。壺らしい形がはっきり分かるサイズに」)を
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
// 実機フィードバック(「藻・岩を魚が隠れられるくらい大きく」)対応: 魚のスプライトが
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
