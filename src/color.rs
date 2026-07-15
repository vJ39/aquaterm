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

// 水質が悪化した時の濁った緑〜茶系の色。水質パラメータの可視化に使う。
// 悪化の最大値を引き上げたのに合わせて、より暗く濁った色にしてある。
pub const MURKY_TINT: Color = Color::new(70, 60, 25);
// 水槽の色にpollution_frac(0.0=綺麗..1.0=最悪)に応じて濁った色を混ぜる。
// 最悪(1.0)ではほぼMURKY_TINT一色になるくらいまで濁らせる(MURKY_MAX_MIX)。
pub const MURKY_MAX_MIX: f64 = 0.85;
pub fn apply_murkiness(c: Color, pollution_frac: f64) -> Color {
    let t = pollution_frac.clamp(0.0, 1.0) * MURKY_MAX_MIX;
    lerp(c, MURKY_TINT, t)
}

// 浄化剤が効いている間、水を紫色に染める演出用。
pub const PURIFIER_TINT: Color = Color::new(150, 60, 210);
// 濃度(purifier_concentration)は1.0(通常最大)を超えても連投で積み上がる仕様のため、
// 入力側はクランプせず混合率だけ上限で頭打ちにする。100%を超えて積み上がるほど
// 紫がどんどん濃くなり、ほぼ透明度を失う(PURIFIER_TINT一色に近づく)くらいまで
// 染まっていく。
pub const PURIFIER_TINT_MAX_MIX: f64 = 0.97;
pub fn apply_purifier_tint(c: Color, purifier_concentration: f64) -> Color {
    let t = (purifier_concentration.max(0.0) * 0.5).min(PURIFIER_TINT_MAX_MIX);
    lerp(c, PURIFIER_TINT, t)
}

// クジラの死骸大爆発(ネタ枠)の演出用: 一時的に画面全体を真っ赤に染める色。
pub const WHALE_EXPLOSION_FLASH_TINT: Color = Color::new(200, 0, 0);
// 混色の最大値(発生直後)。水質の濁り(MURKY_MAX_MIX=0.85)より強く、ほぼ画面全体が
// 真っ赤に見えるくらいまで染める。
pub const WHALE_EXPLOSION_FLASH_MAX_MIX: f64 = 0.9;
// intensity(0.0=消灯~1.0=発生直後の最大)に応じて、画面全体を真っ赤に染めるための
// オーバーレイ。apply_murkiness/apply_purifier_tintと同じ「色を混ぜるだけ」の考え方だが、
// 対象は水中の背景色だけでなく画面全体(魚・エフェクト等を含めた最終合成)に適用する
// 想定のため、呼び出し側(main.rs)が全ピクセルに対して呼ぶ。減衰(フェード)自体は
// 呼び出し側が計算し、ここには既に減衰済みのintensityを渡す。
pub fn apply_whale_explosion_flash(c: Color, intensity: f64) -> Color {
    let t = intensity.clamp(0.0, 1.0) * WHALE_EXPLOSION_FLASH_MAX_MIX;
    lerp(c, WHALE_EXPLOSION_FLASH_TINT, t)
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
pub const AFFINITY_FLAG: Color = Color::new(255, 130, 170); // なつき度フラグ(ピンク)
pub const DEAD_FLAG: Color = Color::new(140, 140, 150); // 死亡マーク(生気のない灰色)
pub const WOUNDED_FLAG: Color = Color::new(230, 90, 40); // 負傷(1回噛まれた)フラグ(オレンジ寄りの赤)
pub const CRITICAL_FLAG: Color = Color::new(200, 0, 0); // 瀕死(2回噛まれた)フラグ(強い赤)
pub const ELDERLY_FLAG: Color = Color::new(220, 220, 150); // 寿命間近フラグ(枯れたような黄白色)

// スター(無敵アイテム)取得中の発光エフェクト用の色(明るい金色)
pub const INVINCIBLE_GLOW_COLOR: Color = Color::new(255, 235, 120);

// --- 昼夜の照明変化 ---
// 実際の時刻に応じて水槽の照明を自動で変えてほしい(昼は現行のまま・夜は暗め落ち着いた
// トーン・境界はなめらかに)という要望への対応。
// 夜間トーンで寄せる先の暗く落ち着いた紺色
pub const NIGHT_TINT: Color = Color::new(6, 10, 28);
// 昼(6:00)/夜(18:00)の境界の前後、これだけの時間(単位:時)をかけてなめらかに変化する
pub const DAY_NIGHT_TRANSITION_HOURS: f64 = 1.0;

// ローカル時刻(0.0..24.0、分を小数で含む)から、昼=1.0・夜=0.0のなめらかな
// 明るさ係数を返す。昼(6:00-18:00)はそのまま(1.0)、夜(18:00-6:00)は0.0。
// 境界の前後 DAY_NIGHT_TRANSITION_HOURS はスムーズステップで補間し、パキッと
// 切り替わらないようにする。
pub fn day_brightness(hour: f64) -> f64 {
    let smoothstep = |t: f64| {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let tr = DAY_NIGHT_TRANSITION_HOURS;
    if hour >= 6.0 - tr && hour < 6.0 + tr {
        smoothstep((hour - (6.0 - tr)) / (2.0 * tr))
    } else if hour >= 18.0 - tr && hour < 18.0 + tr {
        1.0 - smoothstep((hour - (18.0 - tr)) / (2.0 * tr))
    } else if hour >= 6.0 + tr && hour < 18.0 - tr {
        1.0
    } else {
        0.0
    }
}

// 昼夜の明るさ係数(day: 1.0=昼..0.0=夜)を色に適用する。夜に近いほど暗く
// 落ち着いた紺色(NIGHT_TINT)に寄せつつ、明度自体も落とす。day=1.0のときは
// 元の色から変化しない(「昼間は現行のまま」を保証する)。
pub fn apply_day_night(c: Color, day: f64) -> Color {
    let night_mix = (1.0 - day).clamp(0.0, 1.0);
    let tinted = lerp(c, NIGHT_TINT, night_mix * 0.55);
    scale(tinted, 1.0 - night_mix * 0.45)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_brightness_is_full_during_the_day_plateau() {
        assert_eq!(day_brightness(7.0), 1.0);
        assert_eq!(day_brightness(12.0), 1.0);
        assert_eq!(day_brightness(17.0), 1.0);
    }

    #[test]
    fn day_brightness_is_zero_during_the_night_plateau() {
        assert_eq!(day_brightness(0.0), 0.0);
        assert_eq!(day_brightness(2.0), 0.0);
        assert_eq!(day_brightness(19.0), 0.0);
        assert_eq!(day_brightness(23.9), 0.0);
    }

    #[test]
    fn day_brightness_transitions_smoothly_at_boundaries() {
        // 6:00・18:00の境界そのものはちょうど中間(0.5)付近になるはず(スムーズステップ)。
        let morning_mid = day_brightness(6.0);
        assert!(
            morning_mid > 0.0 && morning_mid < 1.0,
            "境界ちょうどはパキッと切り替わらず中間値のはず: {morning_mid}"
        );
        let evening_mid = day_brightness(18.0);
        assert!(
            evening_mid > 0.0 && evening_mid < 1.0,
            "境界ちょうどはパキッと切り替わらず中間値のはず: {evening_mid}"
        );
        // 境界に近づくほど単調に変化する(パキッと不連続に飛ばない)ことを、
        // 細かい刻みでサンプルして確認する。
        let mut prev = day_brightness(5.0 - 0.001);
        let mut max_jump = 0.0f64;
        let mut t = 5.0;
        while t <= 7.0 {
            let v = day_brightness(t);
            max_jump = max_jump.max((v - prev).abs());
            prev = v;
            t += 0.01;
        }
        assert!(
            max_jump < 0.02,
            "0.01時間刻みでの変化が滑らかであるはず(急激な飛びが無いはず): {max_jump}"
        );
    }

    #[test]
    fn apply_day_night_leaves_color_unchanged_at_full_day() {
        let c = Color::new(64, 190, 210);
        assert_eq!(apply_day_night(c, 1.0), c, "昼間は現行のまま変化しないはず");
    }

    #[test]
    fn apply_day_night_dims_and_tints_toward_navy_at_full_night() {
        let c = Color::new(64, 190, 210);
        let night = apply_day_night(c, 0.0);
        assert_ne!(night, c, "夜は元の色から変化するはず");
        // 全体的に暗くなる(明度が下がる)はず
        let brightness = |col: Color| col.r as u32 + col.g as u32 + col.b as u32;
        assert!(
            brightness(night) < brightness(c),
            "夜は全体的に暗くなるはず: night={night:?} day={c:?}"
        );
    }

    #[test]
    fn apply_murkiness_leaves_color_unchanged_when_clean() {
        let c = WATER_MID;
        assert_eq!(apply_murkiness(c, 0.0), c, "水質が綺麗(0.0)なら色は変化しないはず");
    }

    #[test]
    fn apply_murkiness_tints_toward_murky_color_as_pollution_increases() {
        let c = WATER_MID;
        let dirty = apply_murkiness(c, 1.0);
        assert_ne!(dirty, c, "水質最悪(1.0)なら色が変化するはず");
        let half = apply_murkiness(c, 0.5);
        // 濁り具合が強いほどMURKY_TINTに近づくはず(単調に変化する)
        let dist = |a: Color, b: Color| {
            ((a.r as i32 - b.r as i32).pow(2)
                + (a.g as i32 - b.g as i32).pow(2)
                + (a.b as i32 - b.b as i32).pow(2)) as f64
        };
        assert!(
            dist(dirty, MURKY_TINT) < dist(half, MURKY_TINT),
            "pollution_fracが大きいほど濁った色に近づくはず"
        );
    }

    #[test]
    fn apply_whale_explosion_flash_leaves_color_unchanged_when_intensity_is_zero() {
        let c = WATER_MID;
        assert_eq!(
            apply_whale_explosion_flash(c, 0.0),
            c,
            "intensity=0.0(消灯)なら色は変化しないはず"
        );
    }

    #[test]
    fn apply_whale_explosion_flash_tints_toward_red_as_intensity_increases() {
        let c = WATER_MID;
        let flashed = apply_whale_explosion_flash(c, 1.0);
        assert_ne!(flashed, c, "intensity=1.0(発生直後)なら色が変化するはず");
        let half = apply_whale_explosion_flash(c, 0.5);
        let dist = |a: Color, b: Color| {
            ((a.r as i32 - b.r as i32).pow(2)
                + (a.g as i32 - b.g as i32).pow(2)
                + (a.b as i32 - b.b as i32).pow(2)) as f64
        };
        assert!(
            dist(flashed, WHALE_EXPLOSION_FLASH_TINT) < dist(half, WHALE_EXPLOSION_FLASH_TINT),
            "intensityが大きいほど真っ赤(WHALE_EXPLOSION_FLASH_TINT)に近づくはず"
        );
    }
}
