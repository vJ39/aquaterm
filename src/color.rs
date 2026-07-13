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
