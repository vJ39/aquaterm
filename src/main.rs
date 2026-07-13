// aquaterm — aquazone 風の端末熱帯魚育成ツール
//   crossterm による自前ハーフブロック描画。ratatui は使わない。
//   起動: 前回状態を ~/.config/aquaterm/state.json から復元(無ければ初期状態)。
//   キー: f=餌 / m=薬 / p=一時停止 / [ ]=速度 / R=リセット / +,-=増減 / ,=設定 / ?=ヘルプ / q=終了

mod color;
mod framebuffer;
mod fish;
mod persist;
mod rng;
mod sim;

use color::{lerp, scale, water_gradient, Color, SAND, SAND_DEEP, SICK_TINT};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyModifiers},
    queue,
    style::Print,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use fish::{Fish, HungerLevel};
use framebuffer::FrameBuffer;
use rng::Rng;
use sim::{capacity, sand_height, Simulation};
use std::io::{Stdout, Write};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

const TARGET_FPS: u64 = 30;
// シミュレーション速度の段階(停止相当の低速〜通常〜高速)
const SPEED_STEPS: [f64; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];
const SPEED_DEFAULT: usize = 2; // = 1.0倍

// 端末サイズの下限。極端に小さい端末(script経由の疑似端末等)でも水槽ロジックが
// 破綻しないよう、cols/cell_rows にこの最低値を保証する。
const MIN_COLS: usize = 10;
const MIN_CELL_ROWS: usize = 4;

// 端末の生サイズから、下限を適用した (cols, cell_rows) を計算する
fn sane_size(cols: u16, rows: u16) -> (usize, usize) {
    let cols_u = (cols as usize).max(MIN_COLS);
    let cell_rows = (rows.saturating_sub(1) as usize).max(MIN_CELL_ROWS);
    (cols_u, cell_rows)
}

// 時間制御・確認プロンプトなどの UI 状態
struct Ctl {
    paused: bool,
    speed_idx: usize,
    awaiting_reset: bool, // グレートリセットの確認待ち
}

fn main() {
    if let Err(e) = run() {
        let mut out = std::io::stdout();
        let _ = execute_teardown(&mut out);
        eprintln!("aquaterm error: {e}");
        std::process::exit(1);
    }
}

fn execute_setup(out: &mut Stdout) -> std::io::Result<()> {
    enable_raw_mode()?;
    queue!(out, EnterAlternateScreen, Hide, Clear(ClearType::All))?;
    out.flush()
}

fn execute_teardown(out: &mut Stdout) -> std::io::Result<()> {
    let _ = queue!(out, Show, LeaveAlternateScreen);
    let _ = out.flush();
    let _ = disable_raw_mode();
    Ok(())
}

fn run() -> std::io::Result<()> {
    let mut out = std::io::stdout();
    execute_setup(&mut out)?;

    let (mut cols, mut rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let (mut cols_u, mut cell_rows) = sane_size(cols, rows);
    let mut fb = FrameBuffer::new(cols_u, cell_rows);

    // シミュレーション初期化: セーブがあれば復元、無ければ初回起動として初期個体+ヘルプ
    let mut sim = Simulation::new(Rng::from_time());
    let saved = persist::load();
    let mut help = saved.is_none(); // 初回のみヘルプ自動表示
    match saved {
        Some(state) => persist::restore_into(&mut sim, state),
        None => sim.seed_initial(fb.pix_width(), fb.pix_height()),
    }

    let mut ctl = Ctl {
        paused: false,
        speed_idx: SPEED_DEFAULT,
        awaiting_reset: false,
    };

    let frame_budget = Duration::from_millis(1000 / TARGET_FPS);
    let mut last = Instant::now();
    let mut running = true;

    while running {
        // --- 入力処理(フレーム予算ぶん待つ=アイドル時fps制御・キーには即応) ---
        if event::poll(frame_budget)? {
            loop {
                if let Event::Key(k) = event::read()? {
                    handle_key(
                        k.code,
                        k.modifiers,
                        &mut sim,
                        &mut fb,
                        &mut ctl,
                        &mut help,
                        &mut running,
                        &mut out,
                    )?;
                }
                if !event::poll(Duration::from_millis(0))? {
                    break;
                }
            }
        }

        // --- リサイズ検知 ---
        let (nc, nr) = crossterm::terminal::size().unwrap_or((cols, rows));
        if nc != cols || nr != rows {
            cols = nc;
            rows = nr;
            let (new_cols_u, new_cell_rows) = sane_size(cols, rows);
            cols_u = new_cols_u;
            cell_rows = new_cell_rows;
            fb.resize(cols_u, cell_rows);
            queue!(out, Clear(ClearType::All))?;
        }

        // --- 更新(一時停止=dt0 / 速度倍率を時間経過ロジックに適用) ---
        let now = Instant::now();
        let real_dt = (now - last).as_secs_f64().min(0.1);
        last = now;
        if !help {
            let sim_dt = if ctl.paused {
                0.0
            } else {
                real_dt * SPEED_STEPS[ctl.speed_idx]
            };
            sim.update(sim_dt, fb.pix_width(), fb.pix_height());
        }

        // --- 描画 ---
        // 表示行数は実端末の生値ではなく、下限保証済みの cell_rows から導出する
        // (rows が極端に小さい/0 の場合でも draw_status_bar 側で減算アンダーフローしないため)
        let display_rows = cell_rows + 1;
        if help {
            draw_help(&mut out, cols_u, display_rows)?;
        } else {
            render_tank(&mut fb, &sim);
            fb.flush(&mut out)?;
            draw_status_bar(&mut out, &sim, &ctl, fb.pix_width(), fb.pix_height(), cols_u, display_rows)?;
            out.flush()?;
        }
    }

    let _ = persist::save(&sim);
    execute_teardown(&mut out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_key(
    code: KeyCode,
    mods: KeyModifiers,
    sim: &mut Simulation,
    fb: &mut FrameBuffer,
    ctl: &mut Ctl,
    help: &mut bool,
    running: &mut bool,
    out: &mut Stdout,
) -> std::io::Result<()> {
    // ヘルプ表示中はどのキーでも閉じる(以後は自動表示しない)
    if *help {
        *help = false;
        fb.force_full_redraw();
        queue!(out, Clear(ClearType::All))?;
        return Ok(());
    }

    // リセット確認待ち: y で確定、それ以外は取消
    if ctl.awaiting_reset {
        ctl.awaiting_reset = false;
        if let KeyCode::Char('y') | KeyCode::Char('Y') = code {
            sim.reset(fb.pix_width(), fb.pix_height());
        } else {
            sim.set_message("リセットを取り消しました");
        }
        return Ok(());
    }

    // Ctrl-C は終了
    if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        *running = false;
        return Ok(());
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => *running = false,
        KeyCode::Char('f') => sim.feed(fb.pix_width()),
        KeyCode::Char('m') => sim.medicate(fb.pix_width()),
        KeyCode::Char('p') => {
            ctl.paused = !ctl.paused;
            sim.set_message(if ctl.paused { "一時停止" } else { "再開" });
        }
        KeyCode::Char('[') => {
            if ctl.speed_idx > 0 {
                ctl.speed_idx -= 1;
            }
            sim.set_message(format!("速度 x{}", SPEED_STEPS[ctl.speed_idx]));
        }
        KeyCode::Char(']') => {
            if ctl.speed_idx < SPEED_STEPS.len() - 1 {
                ctl.speed_idx += 1;
            }
            sim.set_message(format!("速度 x{}", SPEED_STEPS[ctl.speed_idx]));
        }
        KeyCode::Char('R') => {
            ctl.awaiting_reset = true; // 確認プロンプトはステータスバーに出す
        }
        KeyCode::Char('+') | KeyCode::Char('=') => sim.add_fish(fb.pix_width(), fb.pix_height()),
        KeyCode::Char('-') => sim.remove_fish(),
        KeyCode::Char(',') => sim.set_message("設定画面は今後対応予定です"),
        KeyCode::Char('?') => {
            *help = true;
        }
        _ => {}
    }
    Ok(())
}

// 魚のスプライト1ピクセルの色に、空腹度(腹ぺこは薄暗く)と病気(まだら・くすみ)を反映
fn fish_pixel_color(f: &Fish, dx: usize, dy: usize, base: Color) -> Color {
    let mut c = base;
    if let HungerLevel::Hungry = f.hunger_level() {
        c = scale(c, 0.72); // 腹ぺこ = やや薄暗い
    }
    if f.sick {
        c = lerp(c, SICK_TINT, 0.45); // くすんだ色へ寄せる
        if (dx + dy) % 2 == 0 {
            c = scale(c, 0.68); // まだら(市松状に減光)
        }
    }
    c
}

// 水槽の1フレームをフレームバッファへ描く
fn render_tank(fb: &mut FrameBuffer, sim: &Simulation) {
    let w = fb.pix_width();
    let h = fb.pix_height();
    let sand_h = sand_height(h);
    let sand_top = h.saturating_sub(sand_h);

    // 背景: 水のグラデーション + 水底の砂
    for y in 0..h {
        if y >= sand_top {
            for x in 0..w {
                let speckle = (x * 7 + y * 13) % 11 == 0;
                fb.set_pixel(x, y, if speckle { SAND } else { SAND_DEEP });
            }
        } else {
            let frac = if sand_top > 0 {
                y as f64 / sand_top as f64
            } else {
                0.0
            };
            let c = water_gradient(frac);
            for x in 0..w {
                fb.set_pixel(x, y, c);
            }
        }
    }

    // 気泡(魚の後ろ)
    let bubble = Color::new(200, 235, 245);
    for b in &sim.bubbles {
        put(fb, b.x, b.y, bubble, w, h);
    }

    // 卵(水底付近、淡いクリーム色)
    let egg_color = Color::new(228, 218, 196);
    for e in &sim.eggs {
        put(fb, e.x, e.y, egg_color, w, h);
    }

    // 餌(暖色) / 薬(緑系・餌と別色)
    let food_color = Color::new(236, 214, 150);
    for fd in &sim.food {
        put(fb, fd.x, fd.y, food_color, w, h);
    }
    let med_color = Color::new(138, 236, 162);
    for md in &sim.medicine {
        put(fb, md.x, md.y, med_color, w, h);
    }

    // 魚(最前面)
    for f in &sim.fish {
        let sprite = f.sprite();
        let left = f.x.round() as isize - (sprite.width as isize) / 2;
        let top = f.y.round() as isize - (sprite.height as isize) / 2;
        for &(dx, dy, base) in &sprite.pixels {
            // 進行方向で左右反転
            let sx = if f.facing_right {
                dx as isize
            } else {
                sprite.width as isize - 1 - dx as isize
            };
            let px = left + sx;
            let py = top + dy as isize;
            if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                let c = fish_pixel_color(f, dx, dy, base);
                fb.set_pixel(px as usize, py as usize, c);
            }
        }
    }
}

// f64 座標を丸めて範囲内なら1ピクセル置く
fn put(fb: &mut FrameBuffer, x: f64, y: f64, c: Color, w: usize, h: usize) {
    let ix = x.round() as isize;
    let iy = y.round() as isize;
    if ix >= 0 && iy >= 0 && (ix as usize) < w && (iy as usize) < h {
        fb.set_pixel(ix as usize, iy as usize, c);
    }
}

// 下部1行の反転ステータスバー
#[allow(clippy::too_many_arguments)]
fn draw_status_bar(
    out: &mut Stdout,
    sim: &Simulation,
    ctl: &Ctl,
    pix_w: usize,
    pix_h: usize,
    cols: usize,
    rows: usize,
) -> std::io::Result<()> {
    let content = if ctl.awaiting_reset {
        // 確認プロンプトを最優先で表示
        " 水槽をリセットしますか?  [y] 実行  /  他のキー 取消 ".to_string()
    } else {
        let cap = capacity(pix_w, pix_h);
        let t = sim.elapsed as u64;
        let (mm, ss) = (t / 60, t % 60);
        let speed = if ctl.paused {
            "停止".to_string()
        } else {
            format!("x{}", SPEED_STEPS[ctl.speed_idx])
        };
        let base = format!(
            " 魚 {}/{}  病気 {}  餌 {}  速度 {}  経過 {:02}:{:02}   [f]餌 [m]薬 [p]停止 [[/]]速度 [R]初期化 [?]ヘルプ [q]終了 ",
            sim.fish_count(),
            cap,
            sim.sick_count(),
            sim.food_count(),
            speed,
            mm,
            ss
        );
        match &sim.message {
            Some(msg) => format!(" {msg}  |{base}"),
            None => base,
        }
    };
    let padded = fit_width(&content, cols);
    let bar = format!("\x1b[38;2;18;18;18m\x1b[48;2;222;230;240m{}\x1b[0m", padded);
    // rows が 0 のような異常値でも減算アンダーフローしないよう saturating_sub を使う
    queue!(out, MoveTo(0, rows.saturating_sub(1) as u16), Print(bar))?;
    Ok(())
}

// 表示幅を cols にそろえる(不足はスペース埋め、超過は切り詰め)
fn fit_width(s: &str, cols: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w == cols {
        return s.to_string();
    }
    if w < cols {
        let mut out = s.to_string();
        out.push_str(&" ".repeat(cols - w));
        return out;
    }
    let mut acc = String::new();
    let mut acc_w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthStr::width(ch.to_string().as_str());
        if acc_w + cw > cols {
            break;
        }
        acc.push(ch);
        acc_w += cw;
    }
    while acc_w < cols {
        acc.push(' ');
        acc_w += 1;
    }
    acc
}

// ヘルプ画面(初回自動表示・? で開閉)
fn draw_help(out: &mut Stdout, cols: usize, rows: usize) -> std::io::Result<()> {
    let lines = [
        "",
        "  aquaterm — 端末熱帯魚アクアリウム",
        "",
        "  魚に餌をやって育てよう。満腹を保つと成長し、まれに産卵→孵化で増えます。",
        "  空腹が続いたり過密だと病気になります。薬で治療を。放置すると力尽きます。",
        "",
        "  操作:",
        "    f          餌やり(中央上部から数粒投下)",
        "    m          薬を投げる(病気の魚が触れると治癒)",
        "    p          一時停止 / 再開",
        "    [ / ]      シミュレーション速度を下げる / 上げる",
        "    R          水槽グレートリセット(確認あり)",
        "    + / -      魚を1匹追加 / 間引く",
        "    ,          設定(将来対応)",
        "    ?          このヘルプを開閉",
        "    q / Esc    終了(状態を保存)",
        "",
        "  種類: ネオン(青) / 金魚(オレンジ) / グッピー(白+差し色)",
        "",
        "  何かキーを押すと水槽に戻ります。",
        "",
    ];
    queue!(out, Clear(ClearType::All))?;
    let start_row = rows.saturating_sub(lines.len()) / 2;
    for (i, line) in lines.iter().enumerate() {
        let w = UnicodeWidthStr::width(*line);
        let col = if cols > w { (cols - w) / 2 } else { 0 };
        let r = (start_row + i).min(rows.saturating_sub(1));
        queue!(
            out,
            MoveTo(col as u16, r as u16),
            Print(format!("\x1b[38;2;220;235;245m{line}\x1b[0m"))
        )?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 回帰テスト: 疑似端末等で極端に小さい/0の端末サイズが返っても、
    // cols/cell_rows には最低値が保証されること(sim.rs 側の clamp panic の間接的な防止線)。
    #[test]
    fn sane_size_enforces_minimums_for_tiny_terminals() {
        assert_eq!(sane_size(0, 0), (MIN_COLS, MIN_CELL_ROWS));
        assert_eq!(sane_size(1, 1), (MIN_COLS, MIN_CELL_ROWS));
        assert_eq!(sane_size(5, 2), (MIN_COLS, MIN_CELL_ROWS));
    }

    #[test]
    fn sane_size_passes_through_normal_terminals() {
        // 十分な大きさの端末では下限に丸められず、実サイズを反映する(高さは status bar 分 -1)
        assert_eq!(sane_size(80, 24), (80, 23));
        assert_eq!(sane_size(120, 40), (120, 39));
    }
}
