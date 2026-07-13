// 色(truecolor RGB)とブルー系配色パレット

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }
}

// 2色を t(0.0..1.0)で線形補間する
pub fn lerp(a: Color, b: Color, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| -> u8 { (x as f64 + (y as f64 - x as f64) * t).round() as u8 };
    Color::new(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b))
}

// 明度を factor 倍して減光する
pub fn scale(c: Color, factor: f64) -> Color {
    let f = |v: u8| (v as f64 * factor).clamp(0.0, 255.0).round() as u8;
    Color::new(f(c.r), f(c.g), f(c.b))
}

// 病気のくすんだ色(緑がかったグレー)
pub const SICK_TINT: Color = Color::new(120, 132, 108);

// 水槽グラデーション用のアンカー色(上→下)
pub const WATER_TOP: Color = Color::new(64, 190, 210); // 明るいシアン
pub const WATER_MID: Color = Color::new(28, 96, 176); // 青
pub const WATER_DEEP: Color = Color::new(8, 24, 72); // 濃紺
pub const SAND: Color = Color::new(120, 104, 68); // 砂色(差し色)
pub const SAND_DEEP: Color = Color::new(60, 52, 40); // 水底の濃い砂

// 論理ピクセル1行の高さ(0..1)から水の背景色を返す。y_frac=0が水面。
pub fn water_gradient(y_frac: f64) -> Color {
    if y_frac < 0.5 {
        lerp(WATER_TOP, WATER_MID, y_frac / 0.5)
    } else {
        lerp(WATER_MID, WATER_DEEP, (y_frac - 0.5) / 0.5)
    }
}

// 元気メーター用のグラデーション色(赤=低 → 黄=中 → 緑=高)
pub const VITALITY_LOW: Color = Color::new(214, 48, 44); // 赤(瀕死)
pub const VITALITY_MID: Color = Color::new(232, 198, 46); // 黄(普通)
pub const VITALITY_HIGH: Color = Color::new(96, 214, 88); // 緑(元気)

// 元気度 t(0.0=瀕死..1.0=満点)から表示色を返す
pub fn vitality_color(t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        lerp(VITALITY_LOW, VITALITY_MID, t / 0.5)
    } else {
        lerp(VITALITY_MID, VITALITY_HIGH, (t - 0.5) / 0.5)
    }
}

// カーソル(照準)の色。魚・背景のどの配色とも被らない明るいマゼンタ。
pub const CURSOR: Color = Color::new(255, 60, 220);

// ステータスオーバーレイ(vキーでON/OFF)用の色
pub const GAUGE_EMPTY: Color = Color::new(40, 42, 50); // 生命残りゲージの未点灯セグメント
pub const HUNGRY_FLAG: Color = Color::new(255, 150, 40); // 腹ペコフラグ(琥珀色)
pub const SICK_FLAG: Color = Color::new(180, 70, 200); // 病気フラグ(紫)
