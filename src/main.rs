// aquaterm — aquazone 風の端末熱帯魚育成ツール
//   crossterm による自前ハーフブロック描画。ratatui は使わない。
//   起動: 前回状態を ~/.config/aquaterm/state.json から復元(無ければ初期状態)。
//   キー: 矢印=カーソル移動 / f=餌 / m=薬 / t=ガラスを叩く / T=トントン(引き寄せ) /
//        p=一時停止 / [ ]=速度 / R=リセット / v=ステータスオーバーレイ表示切替 /
//        s=効果音ON/OFF / a=自動モード / A=自動魚補充 / +,-=増減 /
//        S=ピラニア確定追加 / O=タコ確定追加 / D=タコつぼ再配置 / P=藻・水草再配置 / ,=設定 / ?=ヘルプ / q=終了

mod color;
mod framebuffer;
mod fish;
mod persist;
mod rng;
mod sim;
mod sound;

use color::{
    apply_day_night, apply_murkiness, day_brightness, lerp, scale, vitality_color, water_gradient, Color, CURSOR,
    AFFINITY_FLAG, CRITICAL_FLAG, DEAD_FLAG, ELDERLY_FLAG, GAUGE_EMPTY, HUNGRY_FLAG, INVINCIBLE_GLOW_COLOR, SAND,
    SAND_DEEP, SICK_FLAG, SICK_TINT, WOUNDED_FLAG,
};
use chrono::{Local, Timelike};
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
use fish::{crab_sprite, Fish, HungerLevel, Species, Stage};
use framebuffer::FrameBuffer;
use rng::Rng;
use sim::{capacity, clamp_point, sand_height, Simulation, Star};
use std::io::{Stdout, Write};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

const TARGET_FPS: u64 = 30;
// シミュレーション速度の段階(停止相当の低速〜通常〜高速)
// もっと速く進めたいという要望を受けて最大4倍→16倍まで拡張した。
const SPEED_STEPS: [f64; 7] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];
const SPEED_DEFAULT: usize = 2; // = 1.0倍
// カーソル(照準)を矢印キー1回でどれだけ動かすか(論理ピクセル)
const CURSOR_STEP: f64 = 2.0;
const CURSOR_STEP_SHIFT_MULT: f64 = 4.0; // Shiftを押しながらの高速移動時の倍率
// 生命残りゲージのセグメント数(スプライト直上の1行に収まる小さいバー)
const GAUGE_SEGMENTS: usize = 3;
// 起動スプラッシュ(タイトルロゴ)を自動で消すまでの時間
const SPLASH_DURATION: Duration = Duration::from_millis(2200);
// ステータスオーバーレイの点滅間隔(秒)。常時点灯だと水槽の背景色に埋もれて
// 見えづらいという指摘を受けて点滅表示にした。実時間基準(一時停止・
// 速度倍率の影響を受けない)で切り替える。
const OVERLAY_BLINK_INTERVAL_SECS: f64 = 0.5;

// 端末サイズの下限。極端に小さい端末(script経由の疑似端末等)でも水槽ロジックが
// 破綻しないよう、cols/cell_rows にこの最低値を保証する。
const MIN_COLS: usize = 10;
const MIN_CELL_ROWS: usize = 4;

// エビの浮遊表現(見た目のみ・カニとの違いを出す演出)
const SHRIMP_HOVER_HEIGHT: f64 = 2.5; // 水底からこの高さだけ浮いた位置を基準にする
const SHRIMP_BOB_AMPLITUDE: f64 = 1.2; // 上下にゆらゆら揺れる振れ幅
const SHRIMP_BOB_SPEED: f64 = 1.1; // 揺れの角速度(大きいほど速く揺れる)

// 端末の生サイズから、下限を適用した (cols, cell_rows) を計算する
fn sane_size(cols: u16, rows: u16) -> (usize, usize) {
    let cols_u = (cols as usize).max(MIN_COLS);
    let cell_rows = (rows.saturating_sub(1) as usize).max(MIN_CELL_ROWS);
    (cols_u, cell_rows)
}

// 時間制御・確認プロンプト・カーソル位置などの UI 状態。
// pub(crate)にしているのは persist.rs から設定値の保存/復元(save/restore_into)で
// 直接読み書きするため(設定値を次回起動時に覚えておいてほしいという要望への対応)。
pub(crate) struct Ctl {
    paused: bool,
    speed_idx: usize,
    awaiting_reset: bool, // グレートリセットの確認待ち
    cursor_x: f64,        // 照準カーソルの位置(論理ピクセル)。餌/薬の投下X座標に使う
    cursor_y: f64,
    pub(crate) overlay_on: bool, // ステータスオーバーレイ(腹ペコ/病気フラグ・生命ゲージ)表示ON/OFF
    pub(crate) sfx_on: bool, // 効果音(SE)のON/OFF。気泡音(Bubble)は対象外(bubble_sfx_onで別管理)
    pub(crate) auto_on: bool, // 自動モード(自動餌やり・自動投薬・自動ガラス叩き)のON/OFF。既定OFF
    settings_on: bool,     // 設定画面(,キー)を開いているか
    settings_selected: usize, // 設定画面での選択項目インデックス(0..SETTINGS_ITEM_COUNT)
    pub(crate) day_night_on: bool, // 実際の時刻に応じた昼夜の照明変化のON/OFF。既定ON
    pub(crate) auto_replenish_on: bool, // 自動魚補充(Aキー)のON/OFF。既定OFF。自動モード(aキー)とは別トグル
    // バブル音と他のSEのトグルを分離してほしいという要望への対応。気泡が上る音だけ
    // 他の効果音(sfx_on)とは独立にON/OFFできる。既定ON(従来の常時鳴る挙動を維持)。
    pub(crate) bubble_sfx_on: bool,
}

// 設定画面で切り替えられる項目数(効果音・オーバーレイ・自動モード・昼夜連動・自動魚補充・
// 気泡音・通常5種それぞれの生成トグル)
const SETTINGS_ITEM_COUNT: usize = 13;

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

    // 設定トグル類はCtl構築時点でまだ既定値のまま。セーブがあれば直後に
    // persist::restore_into()で上書きする(設定値を次回起動時に覚えておいてほしいという
    // 要望への対応)。Ctlの構築自体はrestore_intoより先に必要(restore_into
    // が&mut Ctlを取るため)。
    let mut ctl = Ctl {
        paused: false,
        speed_idx: SPEED_DEFAULT,
        awaiting_reset: false,
        // カーソルの初期位置は画面中央
        cursor_x: fb.pix_width() as f64 / 2.0,
        cursor_y: fb.pix_height() as f64 / 2.0,
        overlay_on: true, // 既定は表示ON
        sfx_on: true,     // 既定は再生ON
        auto_on: false,   // 既定はOFF(手動操作を邪魔しないため)
        settings_on: false,
        settings_selected: 0,
        day_night_on: true, // 既定はON(実際の時刻に応じて自動で明るさが変わる)
        auto_replenish_on: false, // 既定OFF(勝手に増えるのを望まない場合もあるため)
        bubble_sfx_on: true, // 既定ON(従来の常時鳴る挙動を維持)
    };

    let saved = persist::load();
    let mut help = saved.is_none(); // 初回のみヘルプ自動表示
    match saved {
        Some(state) => {
            persist::restore_into(&mut sim, &mut ctl, state);
            // 旧セーブには観賞用エンティティが無いため、空なら補充する
            sim.ensure_decorative_entities(fb.pix_width(), fb.pix_height());
        }
        None => sim.seed_initial(fb.pix_width(), fb.pix_height()),
    }

    // 効果音エンジン: オーディオデバイスが無い/初期化失敗でも SoundEngine::new() 自体は
    // panic せず、以後の play() が静かに無視されるだけになる(内部で握りつぶす)。
    let sound = sound::SoundEngine::new();

    // 起動スプラッシュ(タイトルロゴ)。毎回の起動時に表示し、キー入力か一定時間で消える。
    let mut splash = true;
    let splash_start = Instant::now();
    // オーバーレイの点滅用(実時間の累計。一時停止・速度倍率の影響を受けない)
    let mut blink_elapsed: f64 = 0.0;

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
                        &mut splash,
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

        // --- スプラッシュの自動消灯(一定時間経過で消す) ---
        if splash && splash_start.elapsed() >= SPLASH_DURATION {
            splash = false;
            fb.force_full_redraw();
            queue!(out, Clear(ClearType::All))?;
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
            // 端末サイズ(フォントサイズ)変更で水底の位置がずれるため、タコつぼ・
            // 水草の座標を新しい水底に合わせて再計算する(この要望への対応)
            sim.resync_seabed_decor(fb.pix_width(), fb.pix_height());
            queue!(out, Clear(ClearType::All))?;
        }

        // --- 更新(一時停止=dt0 / 速度倍率を時間経過ロジックに適用) ---
        let now = Instant::now();
        let real_dt = (now - last).as_secs_f64().min(0.1);
        last = now;
        // オーバーレイの点滅は実時間基準で常に進める(一時停止中でも点滅は止めない)
        blink_elapsed += real_dt;
        // 設定画面をサイドメニューにし、画面切り替えにならないようにしてほしいという
        // 要望への対応: 設定画面を開いている間もシミュレーションを
        // 止めない(水槽が裏で動き続けたまま設定を見られるようにする)。
        if !help && !splash {
            let sim_dt = if ctl.paused {
                0.0
            } else {
                real_dt * SPEED_STEPS[ctl.speed_idx]
            };
            sim.update(sim_dt, fb.pix_width(), fb.pix_height());
            // 自動モード(aキー)ON中は、自動餌やり・自動投薬・自動ガラス叩きを
            // sim.update() と同じ時間軸(一時停止・速度倍率も反映)で進める。
            if ctl.auto_on {
                sim.update_auto_care(sim_dt, fb.pix_width(), fb.pix_height());
            }
            // 自動魚補充(Aキー)ON中は、通常魚が少なくなったら自動で+キー相当の
            // 補充を行う(自動モードaキーとは別トグル)。
            if ctl.auto_replenish_on {
                sim.update_auto_replenish(sim_dt, fb.pix_width(), fb.pix_height());
            }

            // このtickで発生した効果音イベントを再生する(OFF中は消費だけして鳴らさない)。
            // 気泡音(Bubble)だけはbubble_sfx_onで独立にON/OFFできる(他のSEはsfx_on)。
            for ev in sim.sound_events.drain(..) {
                let enabled = if ev == sim::SfxEvent::Bubble {
                    ctl.bubble_sfx_on
                } else {
                    ctl.sfx_on
                };
                if enabled {
                    sound.play(ev);
                }
            }
        }

        // --- 描画 ---
        // 表示行数は実端末の生値ではなく、下限保証済みの cell_rows から導出する
        // (rows が極端に小さい/0 の場合でも draw_status_bar 側で減算アンダーフローしないため)
        let display_rows = cell_rows + 1;
        if splash {
            draw_splash(&mut out, cols_u, display_rows)?;
        } else if help {
            draw_help(&mut out, cols_u, display_rows)?;
        } else {
            // オーバーレイは点滅表示(OVERLAY_BLINK_INTERVAL_SECS 毎にON/OFFを切り替える)。
            // v キーで非表示にしている間は点滅にかかわらず常に非表示のまま。
            let blink_phase = (blink_elapsed / OVERLAY_BLINK_INTERVAL_SECS) as u64 % 2 == 0;
            let overlay_visible = ctl.overlay_on && blink_phase;
            let day = if ctl.day_night_on {
                let now = Local::now();
                let hour =
                    now.hour() as f64 + now.minute() as f64 / 60.0 + now.second() as f64 / 3600.0;
                day_brightness(hour)
            } else {
                1.0 // OFF中は常に昼間相当の明るさに固定する
            };
            render_tank(&mut fb, &sim, ctl.cursor_x, ctl.cursor_y, overlay_visible, day);
            fb.flush(&mut out)?;
            draw_status_bar(&mut out, &sim, &ctl, fb.pix_width(), fb.pix_height(), cols_u, display_rows)?;
            // 設定画面(サイドメニュー): 水槽の描画を止めず、右側に固定幅パネルを
            // 重ねて描く(全画面Clearしない・オーバーレイ表示)。
            if ctl.settings_on {
                draw_settings_panel(&mut out, &ctl, &sim, cols_u, cell_rows)?;
            }
            out.flush()?;
        }
    }

    let _ = persist::save(&sim, &ctl);
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
    splash: &mut bool,
    help: &mut bool,
    running: &mut bool,
    out: &mut Stdout,
) -> std::io::Result<()> {
    // スプラッシュ表示中はどのキーでも閉じる
    if *splash {
        *splash = false;
        fb.force_full_redraw();
        queue!(out, Clear(ClearType::All))?;
        return Ok(());
    }

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

    // 設定画面: 上下で項目選択、Enter/スペースでトグル、Escで閉じる。
    // 他のキーは何もしない(誤操作で水槽側の操作に伝わらないようにする)。
    if ctl.settings_on {
        match code {
            KeyCode::Up => {
                ctl.settings_selected = if ctl.settings_selected == 0 {
                    SETTINGS_ITEM_COUNT - 1
                } else {
                    ctl.settings_selected - 1
                };
            }
            KeyCode::Down => {
                ctl.settings_selected = (ctl.settings_selected + 1) % SETTINGS_ITEM_COUNT;
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                let now_on = match ctl.settings_selected {
                    0 => {
                        ctl.sfx_on = !ctl.sfx_on;
                        ctl.sfx_on
                    }
                    1 => {
                        ctl.overlay_on = !ctl.overlay_on;
                        ctl.overlay_on
                    }
                    2 => {
                        ctl.auto_on = !ctl.auto_on;
                        ctl.auto_on
                    }
                    3 => {
                        ctl.day_night_on = !ctl.day_night_on;
                        ctl.day_night_on
                    }
                    4 => {
                        ctl.auto_replenish_on = !ctl.auto_replenish_on;
                        ctl.auto_replenish_on
                    }
                    5 => {
                        ctl.bubble_sfx_on = !ctl.bubble_sfx_on;
                        ctl.bubble_sfx_on
                    }
                    idx @ 6..=10 => {
                        sim.toggle_common_species(idx - 6);
                        sim.species_toggle[idx - 6]
                    }
                    11 => {
                        sim.cycle_feed_amount();
                        true
                    }
                    12 => {
                        sim.toggle_crabs(fb.pix_width());
                        sim.crab_toggle
                    }
                    _ => false,
                };
                if now_on {
                    sim.sound_events.push(sim::SfxEvent::UiClick);
                }
            }
            KeyCode::Esc => {
                ctl.settings_on = false;
                fb.force_full_redraw();
                queue!(out, Clear(ClearType::All))?;
            }
            _ => {}
        }
        return Ok(());
    }

    // Ctrl-C は終了
    if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        *running = false;
        return Ok(());
    }

    // Shiftを押しながらのカーソル移動は高速(CURSOR_STEPの何倍か)にする。
    let cursor_step = if mods.contains(KeyModifiers::SHIFT) {
        CURSOR_STEP * CURSOR_STEP_SHIFT_MULT
    } else {
        CURSOR_STEP
    };

    match code {
        KeyCode::Char('q') | KeyCode::Esc => *running = false,
        KeyCode::Left => {
            let (cx, cy) = clamp_point(
                ctl.cursor_x - cursor_step,
                ctl.cursor_y,
                fb.pix_width(),
                fb.pix_height(),
            );
            ctl.cursor_x = cx;
            ctl.cursor_y = cy;
        }
        KeyCode::Right => {
            let (cx, cy) = clamp_point(
                ctl.cursor_x + cursor_step,
                ctl.cursor_y,
                fb.pix_width(),
                fb.pix_height(),
            );
            ctl.cursor_x = cx;
            ctl.cursor_y = cy;
        }
        KeyCode::Up => {
            let (cx, cy) = clamp_point(
                ctl.cursor_x,
                ctl.cursor_y - cursor_step,
                fb.pix_width(),
                fb.pix_height(),
            );
            ctl.cursor_x = cx;
            ctl.cursor_y = cy;
        }
        KeyCode::Down => {
            let (cx, cy) = clamp_point(
                ctl.cursor_x,
                ctl.cursor_y + cursor_step,
                fb.pix_width(),
                fb.pix_height(),
            );
            ctl.cursor_x = cx;
            ctl.cursor_y = cy;
        }
        KeyCode::Char('f') => sim.feed(ctl.cursor_x, fb.pix_width()),
        KeyCode::Char('m') => sim.medicate(ctl.cursor_x, fb.pix_width()),
        // ピラニア専用の肉餌(自動モードには絶対に組み込まない、キー入力専用の操作)
        KeyCode::Char('M') => sim.drop_meat(ctl.cursor_x, fb.pix_width()),
        KeyCode::Char('t') => sim.knock(ctl.cursor_x, ctl.cursor_y, fb.pix_width(), fb.pix_height()),
        KeyCode::Char('T') => sim.tap_attract(ctl.cursor_x, ctl.cursor_y, fb.pix_width(), fb.pix_height()),
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
        KeyCode::Char('v') => {
            ctl.overlay_on = !ctl.overlay_on;
            if ctl.overlay_on {
                sim.sound_events.push(sim::SfxEvent::UiClick);
            }
            sim.set_message(if ctl.overlay_on {
                "オーバーレイ表示"
            } else {
                "オーバーレイ非表示"
            });
        }
        KeyCode::Char('s') => {
            ctl.sfx_on = !ctl.sfx_on;
            // `s`は全体ミュートのクイックキーとして、バブル音トグルも連動させる
            // (個別に分けたい場合は設定画面(`,`)で気泡音だけを後から切り替えられる)。
            ctl.bubble_sfx_on = ctl.sfx_on;
            if ctl.sfx_on {
                sim.sound_events.push(sim::SfxEvent::UiClick);
            }
            sim.set_message(if ctl.sfx_on { "効果音ON" } else { "効果音OFF" });
        }
        KeyCode::Char('a') => {
            ctl.auto_on = !ctl.auto_on;
            if ctl.auto_on {
                sim.sound_events.push(sim::SfxEvent::UiClick);
            }
            sim.set_message(if ctl.auto_on {
                "自動モードON(自動餌やり/投薬/ガラス叩き)"
            } else {
                "自動モードOFF"
            });
        }
        KeyCode::Char('A') => {
            ctl.auto_replenish_on = !ctl.auto_replenish_on;
            if ctl.auto_replenish_on {
                sim.sound_events.push(sim::SfxEvent::UiClick);
            }
            sim.set_message(if ctl.auto_replenish_on {
                "自動魚補充ON(通常魚が減ったら自動追加)"
            } else {
                "自動魚補充OFF"
            });
        }
        KeyCode::Char('+') | KeyCode::Char('=') => sim.add_fish(fb.pix_width(), fb.pix_height()),
        KeyCode::Char('S') => sim.add_piranha(fb.pix_width(), fb.pix_height()),
        KeyCode::Char('O') => sim.add_octopus(fb.pix_width(), fb.pix_height()),
        KeyCode::Char('W') => sim.add_whale(fb.pix_width(), fb.pix_height()),
        KeyCode::Char('D') => sim.reposition_dens(fb.pix_width(), fb.pix_height()),
        KeyCode::Char('P') => sim.reposition_plants(fb.pix_width(), fb.pix_height()),
        KeyCode::Char('-') => sim.remove_fish(),
        // デバッグ用: 全員を一発で空腹にする(この要望への対応)
        KeyCode::Char('H') => sim.debug_starve_all(),
        // デバッグ用: 産卵可能な同種ペアを即座に交尾成立(ハート+卵)させる
        KeyCode::Char('K') => sim.debug_force_courtship_proximity(fb.pix_width(), fb.pix_height()),
        // デバッグ用: 水質を0とMAXの間でトグルする
        KeyCode::Char('J') => sim.debug_toggle_pollution(),
        // デバッグ用: 生きている個体からランダムに1匹選んで即座に死亡させる
        KeyCode::Char('X') => sim.debug_kill_random_fish(),
        // デバッグ用: スター(無敵アイテム)をカーソル位置に確実に投入する
        KeyCode::Char('Z') => sim.debug_spawn_star(ctl.cursor_x, ctl.cursor_y, fb.pix_width(), fb.pix_height()),
        // デバッグ用: 生きている個体からランダムに1匹選んで寿命(老衰死)の残りを10秒にする
        KeyCode::Char('L') => sim.debug_age_random_fish_near_death(),
        KeyCode::Char(',') => {
            ctl.settings_on = true;
            ctl.settings_selected = 0;
        }
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
    if f.is_invincible() {
        // スター(無敵アイテム)取得中: 光る/点滅するエフェクトで通常状態と見分けられる
        // ようにする。invincible_timer自体は時間経過で減っていく値なので、そのまま
        // 明滅の位相として使えば追加の時刻引き渡しなしに点滅させられる。
        let blink = ((f.invincible_timer * sim::INVINCIBLE_BLINK_FREQ).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
        c = lerp(c, INVINCIBLE_GLOW_COLOR, 0.25 + 0.45 * blink);
    }
    c
}

// 水槽の1フレームをフレームバッファへ描く
fn render_tank(
    fb: &mut FrameBuffer,
    sim: &Simulation,
    cursor_x: f64,
    cursor_y: f64,
    overlay_on: bool,
    day: f64,
) {
    let w = fb.pix_width();
    let h = fb.pix_height();
    let sand_h = sand_height(h);
    let sand_top = h.saturating_sub(sand_h);

    // 背景: 水のグラデーション + 水底の砂。実際の時刻に応じた昼夜の明るさ係数(day:
    // 1.0=昼..0.0=夜)を適用し、夜は暗く落ち着いた紺色に寄せる(水槽を暗くしたり
    // 明るくしたりしたいという要望への対応。境界はcolor::day_brightness側で
    // なめらかに補間済みなので、ここではその係数をそのまま使うだけでよい)。
    for y in 0..h {
        if y >= sand_top {
            for x in 0..w {
                let speckle = (x * 7 + y * 13) % 11 == 0;
                let base = if speckle { SAND } else { SAND_DEEP };
                fb.set_pixel(x, y, apply_day_night(base, day));
            }
        } else {
            let frac = if sand_top > 0 {
                y as f64 / sand_top as f64
            } else {
                0.0
            };
            // 水質パラメータの可視化: 汚れているほど水が濁った緑〜茶系の色に寄る。
            let pollution_frac = sim.pollution / sim::POLLUTION_MAX;
            let c = apply_day_night(apply_murkiness(water_gradient(frac), pollution_frac), day);
            for x in 0..w {
                fb.set_pixel(x, y, c);
            }
        }
    }

    // 藻・水草(装飾。育成ロジックには参加しない静的オブジェクト。ゆらゆら揺れるだけ)。
    // 魚が隠れられるくらい大きくしてほしいという要望に対し、まだ小さいとの再指摘を受けた
    // (本当に魚が隠れるぐらい、魚のドット絵より明らかに大きいサイズにという要望)対応:
    // 単一の細い茎ではなく、複数の太い茎(2px幅)を大きく束ねた「株」として描くことで、
    // 魚を完全に覆い隠せる大きさの茂みにする。先端ほど大きく揺れるようにして、
    // 水中で揺らめく感じを出す。
    let plant_color = Color::new(55, 145, 75);
    let plant_color_dark = Color::new(38, 118, 60); // 奥の茎(重なりの奥行き表現)
    const PLANT_BLADE_OFFSETS: [f64; 9] = [-4.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0];
    // 水流で藻が傾く量の倍率。先端(t=1)ほど大きく傾き、根元(t=0)は動かない。
    const PLANT_CURRENT_LEAN_MULT: f64 = 0.6;
    for p in &sim.plants {
        let segs = p.height.round().max(2.0) as usize;
        // 束になった茎(株)を数本、根元のx位置をずらして描く。茎ごとに位相・高さ・
        // 揺れの速さを少しずつずらすことで、まとまって見えつつも一体感のある茂みになる。
        for (blade_i, &blade_dx) in PLANT_BLADE_OFFSETS.iter().enumerate() {
            let blade_phase = p.phase + blade_i as f64 * 0.9;
            let blade_segs = ((segs as f64) * (0.75 + 0.05 * blade_i as f64)).round().max(2.0) as usize;
            let color = if blade_i % 2 == 0 { plant_color } else { plant_color_dark };
            for seg in 0..blade_segs {
                let t = seg as f64 / blade_segs as f64; // 0(根元)〜1(先端側)
                let sway = (sim.elapsed * sim::PLANT_SWAY_FREQ + blade_phase).sin() * t * 2.2;
                // 揺れに加えて、その場所の渦の力場の水平成分の方向へ傾ける(先端ほど大きく傾く)。
                let (cvx, _) = sim.current_at(p.x, p.y);
                let current_lean = cvx * PLANT_CURRENT_LEAN_MULT * t;
                let bx = p.x + blade_dx + sway + current_lean;
                let by = p.y - seg as f64;
                put(fb, bx, by, color, w, h);
                put(fb, bx + 1.0, by, color, w, h); // 1px幅の細い線に見えないよう太らせる
            }
        }
    }

    // 岩(装飾+隠れ場所)。藻・水草と同様、近くにいる魚が視覚的に隠れられる大きさの
    // 静的オブジェクト。まだ小さいとの指摘を受け、タコつぼ(DEN_SCALE)と同様の
    // 拡大描画で大きく見せる(ALGAE_HIDE_RADIUSの拡大と合わせて隠れやすくする)。
    for r in &sim.rocks {
        draw_sprite_scaled(fb, &fish::rock_sprite(), r.x, r.y, ROCK_SCALE, w, h);
    }

    // タコつぼ(装飾+タコの巣)。タコ自体をデフォルトで大きくした(OCTOPUS_BASE_SCALE_BONUS)
    // のに合わせて、タコつぼの見た目も揃えて大きく描く。
    for d in &sim.dens {
        draw_sprite_scaled(fb, &fish::den_sprite(), d.x, d.y, DEN_SCALE, w, h);
    }

    // カメオ生物(ウミガメ・クラゲ・小魚の群れ)。完全観賞用で、育成ロジック・
    // 捕食判定のいずれにも参加しない。低頻度で画面の端から端まで通過するだけ。
    for c in &sim.cameos {
        let facing_right = c.vx > 0.0;
        match c.kind {
            sim::CameoKind::Turtle => {
                draw_sprite(fb, &fish::turtle_sprite(), c.x, c.y, facing_right, w, h, |_, _, base| base);
            }
            sim::CameoKind::Jellyfish => {
                draw_sprite(fb, &fish::jellyfish_sprite(), c.x, c.y, facing_right, w, h, |_, _, base| base);
            }
            sim::CameoKind::FishSchool => {
                // 専用スプライトを持たず、小さな魚を数匹の群れとして描く
                // (それぞれ位相をずらして、ばらけつつも一緒に泳いでいるように見せる)。
                let school_color = Color::new(210, 225, 235);
                const SCHOOL_SIZE: usize = 4;
                for j in 0..SCHOOL_SIZE {
                    let ox = (j as f64 - (SCHOOL_SIZE as f64 - 1.0) / 2.0) * 3.0;
                    let oy = (c.phase * 1.3 + j as f64 * 1.1).sin() * 1.5;
                    let fx = c.x + ox;
                    let fy = c.y + oy;
                    put(fb, fx, fy, school_color, w, h);
                    // 尾びれ分の1ピクセルを進行方向の後ろに置いて、小魚らしい輪郭にする
                    let tail_dx = if facing_right { -1.0 } else { 1.0 };
                    put(fb, fx + tail_dx, fy, school_color, w, h);
                }
            }
        }
    }

    // 血の滲み(範囲エフェクト): 捕食位置を中心に、同心円状に赤い波紋が広がっていく
    // アニメーション。発生から寿命の半分程度は色の濃さを最大近くで維持して
    // 数秒間はっきり赤く見えるようにし、残りの区間で広がりながらフェードアウトする
    // (捕食による損傷が周囲に染み渡っていく見た目を意図している)。魚より背面に描く。
    let stain_tint = Color::new(150, 10, 12);
    for s in &sim.blood_stains {
        draw_spreading_stain(
            fb,
            s.x,
            s.y,
            s.life,
            s.max_life,
            sim::BLOOD_STAIN_GROWTH_TIME,
            sim::BLOOD_STAIN_MAX_RADIUS,
            sim::BLOOD_STAIN_HOLD_FRACTION,
            sim::BLOOD_STAIN_MIX,
            stain_tint,
            w,
            h,
        );
    }

    // 墨(タコが吐く): 血の滲みと同じ「同心円状に広がってフェードアウトする」構造を
    // 再利用しつつ、血より広め・勢いよく(速く)拡散する黒っぽい色で描く。
    // 色が薄く、完全な黒に近づけてほしいという指摘を受けて、ほぼ純黒にした(旧(18,16,20))。
    let ink_tint = Color::new(2, 2, 3);
    for c in &sim.ink_clouds {
        draw_spreading_stain(
            fb,
            c.x,
            c.y,
            c.life,
            c.max_life,
            sim::INK_GROWTH_TIME,
            sim::INK_MAX_RADIUS,
            sim::INK_HOLD_FRACTION,
            sim::INK_MIX,
            ink_tint,
            w,
            h,
        );
    }

    // 水流の筋(可視化演出): 淡い青白の短い横線を、背景に薄く溶かして描く(はっきりした
    // 実線ではなく、水が流れる揺らめきとして読めるようにする)。生成直後と消滅間際で薄く、
    // 中間で最も濃くなる三角フェードにし、血飛沫・墨と同じく背景色へのlerpで混ぜる。
    let streak_color = Color::new(200, 225, 240);
    const STREAK_WIDTH: i32 = 4; // 横線の長さ(px)
    for s in &sim.current_streaks {
        let frac = if s.max_life > 0.0 {
            (s.life / s.max_life).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // frac: 1.0(生成直後)→0.0(消滅)。中間(0.5)で最大になる三角フェード。
        let fade = (1.0 - (2.0 * frac - 1.0).abs()).clamp(0.0, 1.0);
        let alpha = fade * 0.5; // 全体に控えめ(うっすら揺らめく程度)
        if alpha <= 0.02 {
            continue;
        }
        let iy = s.y.round() as isize;
        let base_x = s.x.round() as isize;
        for dx in 0..STREAK_WIDTH {
            let ix = base_x + dx as isize;
            if ix >= 0 && iy >= 0 && (ix as usize) < w && (iy as usize) < h {
                let bg = fb.get_pixel(ix as usize, iy as usize);
                fb.set_pixel(ix as usize, iy as usize, lerp(bg, streak_color, alpha));
            }
        }
    }

    // 気泡(魚の後ろ)
    let bubble = Color::new(200, 235, 245);
    for b in &sim.bubbles {
        put(fb, b.x, b.y, bubble, w, h);
    }

    // カニ(観賞用・水底を歩くだけ。育成ロジック対象外)
    let crab_y = sand_top as f64 - 1.0;
    for c in &sim.crabs {
        draw_sprite(fb, &crab_sprite(), c.x, crab_y, c.facing_right, w, h, |_, _, base| base);
    }

    // エビ(観賞用・カニと同じ横移動ロジックを共有しつつ、見た目だけ水底より少し
    // 浮いてゆらゆらと上下するようにしている。歩くだけのカニとの見た目の違いを
    // 出すための表示上の演出で、シミュレーションの状態(Shrimp構造体)は変えない。
    for s in &sim.shrimp {
        let bob = (sim.elapsed * SHRIMP_BOB_SPEED + s.x * 0.5).sin() * SHRIMP_BOB_AMPLITUDE;
        let shrimp_y = sand_top as f64 - SHRIMP_HOVER_HEIGHT + bob;
        draw_sprite(fb, &fish::shrimp_sprite(), s.x, shrimp_y, s.facing_right, w, h, |_, _, base| base);
    }

    // タツノオトシゴ(観賞用・カニ・エビと同じ位置づけ。藻に絡みつくようにゆっくり
    // 動き、あまり大きく移動しない。育成ロジック対象外)
    for s in &sim.seahorses {
        draw_sprite(fb, &fish::seahorse_sprite(), s.x, s.y, true, w, h, |_, _, base| base);
    }

    // 卵(水底付近)。卵の位置がわからず急に魚が湧いてくるように見えて不自然という
    // 指摘への対応: 砂色に埋もれて気づかれにくかったため、より明るい
    // 色にし、位置基準の位相でゆっくり明滅させて存在に気づきやすくする
    // (個体ごとに専用のタイマーを持たせず、座標から安定した位相を作ることで
    // フレームごとのチラつきを避けている)。
    let egg_color = Color::new(255, 236, 190);
    for e in &sim.eggs {
        let pulse = ((sim.elapsed * 2.0 + e.x * 0.7 + e.y * 1.3).sin() * 0.5 + 0.5).clamp(0.3, 1.0);
        put(fb, e.x, e.y, scale(egg_color, pulse), w, h);
    }

    // 餌(暖色) / 薬(緑系・餌と別色)。水底に着地したものは、点1つだけだと積もって
    // いる様子が薄く見えるため、着地位置から砂地の表面まで縦に塗って、しっかり
    // 積もった山として見えるようにする(浮いて沈降中のものは点のままでよい)。
    let sand_top_f = sand_top as f64;
    let draw_settled = |fb: &mut FrameBuffer, x: f64, y: f64, color: Color| {
        let mut py = y;
        while py <= sand_top_f {
            put(fb, x, py, color, w, h);
            py += 1.0;
        }
    };
    let food_color = Color::new(236, 214, 150);
    for fd in &sim.food {
        if fd.landed {
            draw_settled(fb, fd.x, fd.y, food_color);
        } else {
            put(fb, fd.x, fd.y, food_color, w, h);
        }
    }
    let med_color = Color::new(138, 236, 162);
    for md in &sim.medicine {
        if md.landed {
            draw_settled(fb, md.x, md.y, med_color);
        } else {
            put(fb, md.x, md.y, med_color, w, h);
        }
    }
    // 肉餌(ピラニア専用。生肉らしい濃い赤で、餌・薬とはっきり見分けられる色にする)
    let meat_color = Color::new(190, 40, 40);
    for mt in &sim.meat {
        if mt.landed {
            draw_settled(fb, mt.x, mt.y, meat_color);
        } else {
            put(fb, mt.x, mt.y, meat_color, w, h);
        }
    }

    // スター(無敵アイテム): キラキラ点滅する十字型。触れた魚が一定時間無敵化する。
    for s in &sim.stars {
        draw_star(fb, s, sim.elapsed, w, h);
    }

    // カーソル(照準): 餌・薬はこのX座標付近から投下される
    draw_cursor(fb, cursor_x, cursor_y, w, h);

    // 投下エフェクト(f/m を押した瞬間に一瞬だけ出る光/波紋)
    for e in &sim.drop_effects {
        draw_drop_effect(fb, e, w, h);
    }

    // 魚(最前面)。成長段階・ピラニアの捕食段階に応じて render_scale() 倍に拡大して描く
    // (拡大は最近傍サンプリングでスプライトの見た目をそのまま拡大するだけで、
    // scale==1.0 のときは従来と全く同じ結果になる)。
    for f in &sim.fish {
        if f.hidden {
            // タコつぼに隠れている間は姿が見えない(タコの節を参照)
            continue;
        }
        // 藻・水草・岩の近くにいるほど「隠れている」表現にする(背景色へ寄せてなじませる)。
        // ピラニアから逃げる魚が隠れ場所に逃げ込む使われ方を想定した見た目の効果。
        // 隠れたら実際に捕食されなくなるよう機能化してほしいという要望への対応で、この
        // 見た目の演出と同じ距離判定(is_hidden_in_cover/ALGAE_HIDE_RADIUS)を
        // 捕食ロジック側でも使っているため、見た目と実際の安全地帯が一致している。
        let nearest_cover_dist = sim
            .plants
            .iter()
            .map(|p| ((p.x - f.x).powi(2) + (p.y - f.y).powi(2)).sqrt())
            .chain(
                sim.rocks
                    .iter()
                    .map(|r| ((r.x - f.x).powi(2) + (r.y - f.y).powi(2)).sqrt()),
            )
            .fold(f64::INFINITY, f64::min);
        let hide_alpha = (1.0 - nearest_cover_dist / sim::ALGAE_HIDE_RADIUS).clamp(0.0, 1.0)
            * sim::ALGAE_HIDE_MIX;
        let sprite = f.sprite();
        let grid = sprite_dense(&sprite);
        let scale = f.render_scale();
        let out_w = ((sprite.width as f64) * scale).round().max(1.0) as isize;
        let out_h = ((sprite.height as f64) * scale).round().max(1.0) as isize;
        let left = f.x.round() as isize - out_w / 2;
        let top = f.y.round() as isize - out_h / 2;
        // タコの足のうねうねアニメーション: 頭部(マント)は静止させたまま、足の部分
        // (スプライト下側)だけを時間経過でサイン波的に左右へオフセットさせて波打つ
        // ように見せる。足の付け根から先端に向かうほど振れを大きくする。
        let octopus_leg_start_row = match (f.species, f.stage) {
            (Species::Octopus, Stage::Adult) => Some(6),
            (Species::Octopus, Stage::Fry) => Some(4),
            _ => None,
        };
        for oy in 0..out_h {
            for ox in 0..out_w {
                let sdx = ((ox as f64) / scale).floor() as usize;
                let sdy = ((oy as f64) / scale).floor() as usize;
                let sdx = sdx.min(sprite.width.saturating_sub(1));
                let sdy = sdy.min(sprite.height.saturating_sub(1));
                // 進行方向で左右反転・死亡演出中は仰向け(上下反転)。出力側のマス目は
                // そのままに、参照する元ピクセルを反転させることで同じ見た目にする。
                let src_dx = if f.facing_right {
                    sdx
                } else {
                    sprite.width - 1 - sdx
                };
                let src_dy = if f.dead {
                    sprite.height - 1 - sdy
                } else {
                    sdy
                };
                if let Some(base) = grid[src_dy * sprite.width + src_dx] {
                    let wiggle_dx = match octopus_leg_start_row {
                        Some(leg_start) if !f.dead && sdy >= leg_start => {
                            let depth = (sdy - leg_start) as f64 + 1.0; // 先端ほど大きく振れる
                            let leg_phase = (sdx as f64 / sprite.width.max(1) as f64)
                                * std::f64::consts::TAU
                                * 2.0; // 足ごとに位相をずらす
                            let phase = sim.elapsed * sim::OCTOPUS_LEG_WIGGLE_FREQ + leg_phase;
                            (phase.sin() * depth * sim::OCTOPUS_LEG_WIGGLE_AMPLITUDE * scale)
                                .round() as isize
                        }
                        _ => 0,
                    };
                    let px = left + ox + wiggle_dx;
                    let py = top + oy;
                    if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                        let mut c = fish_pixel_color(f, src_dx, src_dy, base);
                        if hide_alpha > 0.0 {
                            // 藻・水草に隠れている表現: 既にそこに描かれている背景色へ寄せる
                            let bg = fb.get_pixel(px as usize, py as usize);
                            c = lerp(c, bg, hide_alpha);
                        }
                        fb.set_pixel(px as usize, py as usize, c);
                    }
                }
            }
        }
        // ステータスオーバーレイ: スプライト直上(1論理ピクセル上)に腹ペコ/病気フラグと
        // 生命残りゲージをまとめて表示する(v キーでON/OFF)。拡大表示分の高さも
        // 踏まえ、拡大後のスプライト上端(top)基準で描く。
        if overlay_on {
            draw_status_overlay(fb, f, top, w, h);
        }
    }
}

// スプライトの疎な pixels リストを、(dy*width+dx) で引ける密な配列に変換する
// (拡大描画で最近傍サンプリングする際に使う)。透明部分は None。
fn sprite_dense(sprite: &fish::Sprite) -> Vec<Option<Color>> {
    let mut grid = vec![None; sprite.width * sprite.height];
    for &(dx, dy, c) in &sprite.pixels {
        if dx < sprite.width && dy < sprite.height {
            grid[dy * sprite.width + dx] = Some(c);
        }
    }
    grid
}

// ステータスオーバーレイ: 生命残りゲージ(数セグメントの横棒)+腹ペコ/病気フラグを
// スプライト直上の1行に描く。画面外にはみ出す分は put() が無視する。
fn draw_status_overlay(fb: &mut FrameBuffer, f: &Fish, sprite_top: isize, w: usize, h: usize) {
    let meter_y = sprite_top as f64 - 1.0;
    let half = (GAUGE_SEGMENTS as f64 - 1.0) / 2.0;

    if f.dead {
        // 死亡演出中は満腹・病気等の通常フラグを一切出さず、専用の死亡マークだけを
        // 表示する(生きているかのような表示のまま放置されるのを防ぐため)。
        put(fb, f.x, meter_y, DEAD_FLAG, w, h);
        return;
    }

    // 生命残りゲージ: 元気度に応じて点灯セグメント数を決める(高いほど緑〜黄、低いほど赤)
    let vit = f.vitality();
    let lit = ((vit * GAUGE_SEGMENTS as f64).round() as usize).min(GAUGE_SEGMENTS);
    let gauge_color = vitality_color(vit);
    for i in 0..GAUGE_SEGMENTS {
        let gx = f.x + (i as f64 - half);
        let c = if i < lit { gauge_color } else { GAUGE_EMPTY };
        put(fb, gx, meter_y, c, w, h);
    }

    // 腹ペコフラグ: ゲージのすぐ左に、空腹度が「腹ぺこ」閾値以下のときだけ表示
    if matches!(f.hunger_level(), HungerLevel::Hungry) {
        put(fb, f.x - half - 1.0, meter_y, HUNGRY_FLAG, w, h);
    }
    // 病気フラグ: ゲージのすぐ右に、病気の個体だけ表示(腹ペコフラグとは別の色)
    if f.sick {
        put(fb, f.x + half + 1.0, meter_y, SICK_FLAG, w, h);
    }
    // なつき度フラグ: 病気フラグのさらに右に、閾値以上なついている個体だけ表示
    if f.affinity >= sim::AFFINITY_MARK_THRESHOLD {
        put(fb, f.x + half + 2.0, meter_y, AFFINITY_FLAG, w, h);
    }
    // 負傷フラグ: なつき度フラグのさらに右に、ピラニアに噛まれた個体だけ表示。
    // 1回=負傷・2回以上=瀕死で、色を変えて同時には出さない(排他)。
    if f.piranha_bite_count >= 2 {
        put(fb, f.x + half + 3.0, meter_y, CRITICAL_FLAG, w, h);
    } else if f.piranha_bite_count == 1 {
        put(fb, f.x + half + 3.0, meter_y, WOUNDED_FLAG, w, h);
    }
    // 寿命間近フラグ: 負傷フラグのさらに右に、老衰死までの残り時間が短い個体だけ表示する
    // (Lキーのデバッグショートカット等で寿命を詰めた個体がひと目で分かるようにする)。
    let remaining_lifespan = sim::LIFESPAN_DEATH_AGE * f.lifespan_mult - f.age;
    if remaining_lifespan <= ELDERLY_WARNING_SECS {
        put(fb, f.x + half + 4.0, meter_y, ELDERLY_FLAG, w, h);
    }
}

// 寿命間近フラグを表示する残り時間のしきい値。Lキー(debug_age_random_fish_near_death)が
// 残り10秒に設定するので、少し余裕を持たせてすぐ確認できるようにする。
const ELDERLY_WARNING_SECS: f64 = 15.0;

// 投下エフェクト(餌/薬を投げた瞬間の光/波紋)を描く。中心の一瞬の光→広がるリングの順で、
// 1秒未満で消える。餌と薬で色を変え、何をどこに投げたか一目でわかるようにする。
// 同心円状に半径が広がっていき、保持区間の後にフェードアウトして消える範囲エフェクトを
// 描く汎用ヘルパー。血の滲み・墨の両方で使う(見た目のパラメータだけ変える)。
// growth_time: 半径が0→maxまで広がるのにかける実時間。hold_fraction: 全体の寿命の
// うちこの割合までは色の濃さを最大近くで維持する(その後フェードアウト)。
#[allow(clippy::too_many_arguments)]
fn draw_spreading_stain(
    fb: &mut FrameBuffer,
    cx: f64,
    cy: f64,
    life: f64,
    max_life: f64,
    growth_time: f64,
    max_radius: f64,
    hold_fraction: f64,
    mix: f64,
    tint: Color,
    w: usize,
    h: usize,
) {
    let progress = (1.0 - life / max_life).clamp(0.0, 1.0);
    let elapsed = (max_life - life).max(0.0);
    let growth_progress = (elapsed / growth_time).clamp(0.0, 1.0);
    let radius = (growth_progress * max_radius).max(0.6);
    let intensity = if progress < hold_fraction {
        1.0
    } else {
        (1.0 - (progress - hold_fraction) / (1.0 - hold_fraction)).clamp(0.0, 1.0)
    };
    let x0 = (cx - radius).floor().max(0.0) as usize;
    let x1 = ((cx + radius).ceil() as usize).min(w.saturating_sub(1));
    let y0 = (cy - radius).floor().max(0.0) as usize;
    let y1 = ((cy + radius).ceil() as usize).min(h.saturating_sub(1));
    for py in y0..=y1 {
        for px in x0..=x1 {
            let dx = px as f64 - cx;
            let dy = py as f64 - cy;
            // 縦を少し潰した楕円にする(half-block解像度の見た目調整)
            let nx = dx / radius;
            let ny = dy / (radius * 0.6);
            let norm_dist = (nx * nx + ny * ny).sqrt();
            if norm_dist > 1.0 {
                continue;
            }
            let falloff = 1.0 - norm_dist;
            let alpha = (falloff * intensity * mix).clamp(0.0, 1.0);
            if alpha <= 0.02 {
                continue;
            }
            let base = fb.get_pixel(px, py);
            fb.set_pixel(px, py, lerp(base, tint, alpha));
        }
    }
}

fn draw_drop_effect(fb: &mut FrameBuffer, e: &sim::DropEffect, w: usize, h: usize) {
    // 進行度は生成時のlife(max_life)基準で計算する(血飛沫は個体ごとに寿命がばらつくため、
    // 共通定数ではなく実際の初期値を使わないと進行度がずれる)。
    let max_life = if e.max_life > 0.0 {
        e.max_life
    } else {
        sim::DROP_EFFECT_LIFETIME
    };
    let progress = (1.0 - e.life / max_life).clamp(0.0, 1.0);
    let color = match e.kind {
        sim::EffectKind::Food => Color::new(255, 230, 140), // 餌色に近い明るい暖色
        sim::EffectKind::Medicine => Color::new(170, 255, 190), // 薬色に近い明るい緑
        sim::EffectKind::Knock => Color::new(215, 225, 235), // ガラスの振動らしい淡い銀白色
        // ピラニアの捕食時の血飛沫。内臓の損傷が周囲に飛び散るイメージで、
        // 単色ではなく暗赤・赤黒・ピンク寄りの赤を粒子ごとに固定で割り当てる
        // (座標から安定的に決めるので、フレームごとに色がチラつかない)。
        sim::EffectKind::Blood => {
            let variant = ((e.x * 97.3 + e.y * 53.7).abs() as u64) % 3;
            match variant {
                0 => Color::new(150, 0, 8),   // 濃い暗赤
                1 => Color::new(90, 0, 10),   // 赤黒
                _ => Color::new(205, 55, 90), // ピンク寄りの赤(内臓っぽさ)
            }
        }
        // 産卵時のキラキラ演出。中心色はここでは使わず(下のSpawn専用分岐で描く)、
        // 網羅性のためだけに明るい金〜白を割り当てておく。
        sim::EffectKind::Spawn => Color::new(255, 245, 190),
        // トントン(Tキー): Knock(淡い銀白色)とは対照的な、優しい印象の暖かいピンク。
        sim::EffectKind::Tap => Color::new(255, 195, 205),
        // 肉餌(Mキー): 生肉らしい濃い赤身の色にし、餌(暖色)・薬(緑)と見分けられるようにする。
        sim::EffectKind::Meat => Color::new(200, 60, 60),
        // つがいの交尾: ハートらしい鮮やかなピンク(Spawnと同じキラキラ演出を使い回す)。
        sim::EffectKind::Mate => Color::new(255, 110, 160),
        // 孵化(羽化): 生まれたばかりの柔らかい印象の淡い黄緑。
        sim::EffectKind::Hatch => Color::new(200, 240, 160),
        // カニが亡骸を片付ける瞬間の分解演出。土に還るような、くすんだ灰褐色。
        sim::EffectKind::Decompose => Color::new(90, 78, 60),
    };

    if e.kind == sim::EffectKind::Spawn || e.kind == sim::EffectKind::Mate {
        // 産卵時にキラキラ光るフラッシュ演出を追加してほしいという要望への対応: 生まれた
        // 卵の位置の周囲に光点をちりばめ、光点ごとに位相をずらして明滅させることで
        // 「キラキラ」感を出す。寿命が近づくほど全体をフェードアウトさせる。
        // つがいの交尾演出(Mate)は同じキラキラを流用しつつ、ピンク色のキラキラを
        // もっと見えやすくしてほしいという要望への対応で、消灯時間を
        // 減らし・輝度と粒の大きさをSpawnより一段強めてある。
        let is_mate = e.kind == sim::EffectKind::Mate;
        let fade = (1.0 - progress).clamp(0.0, 1.0);
        let sparkle_points = if is_mate { 8 } else { 6 };
        let twinkle_floor = if is_mate { 0.15 } else { 0.35 };
        let dim_base = if is_mate { 0.8 } else { 0.6 };
        let dim_range = if is_mate { 0.2 } else { 0.4 };
        let radius_base = if is_mate { 2.2 } else { 1.5 };
        let radius_step = if is_mate { 1.0 } else { 0.8 };
        for i in 0..sparkle_points {
            let theta = (i as f64) * std::f64::consts::PI * 2.0 / (sparkle_points as f64);
            let twinkle = ((progress * 14.0 + i as f64 * 1.7).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
            if twinkle < twinkle_floor {
                continue; // 明滅で「消えている」瞬間は描かない
            }
            let r = radius_base + radius_step * (i % 2) as f64;
            let px = e.x + r * theta.cos();
            let py = e.y + r * theta.sin() * 0.6;
            let c = scale(color, fade * (dim_base + dim_range * twinkle));
            put(fb, px, py, c, w, h);
        }
        // 中心も光らせる(Mateはより明るく)
        put(fb, e.x, e.y, scale(color, fade * if is_mate { 0.95 } else { 0.7 }), w, h);
        return;
    }

    if e.kind == sim::EffectKind::Blood || e.kind == sim::EffectKind::Decompose {
        // 内臓の破片・分解した破片らしい重量感: 単発の小さい点ではなく、若いうちは塊
        // (隣接ピクセルも塗って太らせる)として見せ、時間とともにゆっくり沈み
        // (drift)ながら小さく溶けるように消える。カニによる分解演出(Decompose)は
        // 血の滲みと同じ仕組みを流用しつつ、より大きく破片が飛び散り、フェード
        // アウトも加えて「崩れて消えていく」印象を強める。
        let is_decompose = e.kind == sim::EffectKind::Decompose;
        let drift_y = progress * 3.0; // ゆっくり沈んでいく
        let cy = e.y + drift_y;
        let fade = if is_decompose { (1.0 - progress).clamp(0.0, 1.0) } else { 1.0 };
        let c = scale(color, fade);
        put(fb, e.x, cy, c, w, h);
        if progress < 0.5 {
            // 若いうち(寿命の前半)は十字方向にも塗って重たい塊に見せる
            put(fb, e.x - 1.0, cy, c, w, h);
            put(fb, e.x + 1.0, cy, c, w, h);
            put(fb, e.x, cy - 0.6, c, w, h);
            put(fb, e.x, cy + 0.6, c, w, h);
        }
        // ゆったり広がる飛び散り(Decomposeは血の滲みより一回り大きく広がる)
        let spread_mult = if is_decompose { 1.4 } else { 0.7 };
        let radius = progress * sim::BLOOD_SPREAD_RADIUS * spread_mult;
        const SCATTER_RING_POINTS: usize = 6;
        for i in 0..SCATTER_RING_POINTS {
            let theta = (i as f64) * std::f64::consts::PI * 2.0 / (SCATTER_RING_POINTS as f64);
            let px = e.x + radius * theta.cos();
            let py = cy + radius * theta.sin() * 0.6;
            put(fb, px, py, c, w, h);
        }
        return;
    }

    // 発生直後は中心が一瞬強く光る(Food/Medicine/Knock共通)
    if progress < 0.35 {
        put(fb, e.x, e.y, color, w, h);
    }
    // 波紋: 半径が時間とともに広がる8方向のリング
    let radius = progress * sim::DROP_EFFECT_MAX_RADIUS;
    const RING_POINTS: usize = 8;
    for i in 0..RING_POINTS {
        let theta = (i as f64) * std::f64::consts::PI * 2.0 / (RING_POINTS as f64);
        let px = e.x + radius * theta.cos();
        // half-block解像度による縦の見た目調整で少し潰した楕円にする
        let py = e.y + radius * theta.sin() * 0.6;
        put(fb, px, py, color, w, h);
    }
}

// 観賞用エンティティ(大型魚・カニ)のスプライトを、進行方向の左右反転のみ適用して描く。
// color_fn は (dx, dy, base_color) -> 実際に置く色 (現状は素通しだが将来の色演出用に残す)。
fn draw_sprite(
    fb: &mut FrameBuffer,
    sprite: &fish::Sprite,
    cx: f64,
    cy: f64,
    facing_right: bool,
    w: usize,
    h: usize,
    color_fn: impl Fn(usize, usize, Color) -> Color,
) {
    let left = cx.round() as isize - (sprite.width as isize) / 2;
    let top = cy.round() as isize - (sprite.height as isize) / 2;
    for &(dx, dy, base) in &sprite.pixels {
        let sx = if facing_right {
            dx as isize
        } else {
            sprite.width as isize - 1 - dx as isize
        };
        let px = left + sx;
        let py = top + dy as isize;
        if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
            fb.set_pixel(px as usize, py as usize, color_fn(dx, dy, base));
        }
    }
}

// タコつぼをデフォルトで少し大きく描くための拡大表示(タコ自体の拡大に合わせて
// 見た目を揃える)。draw_sprite(等倍・反転のみ対応)とは別に、最近傍サンプリングで
// scale倍に拡大して描く(魚の拡大描画と同じ考え方)。
const DEN_SCALE: f64 = 1.4;
// 岩をデフォルトで大きく描くための拡大率。魚が十分に隠れられる大きさにしてほしい
// という再指摘への対応(藻はheightの拡大で対応済みだが、岩はheightのような
// サイズフィールドを持たない固定スプライトのため、タコつぼと同じ拡大描画で対応する)。
const ROCK_SCALE: f64 = 1.8;
fn draw_sprite_scaled(fb: &mut FrameBuffer, sprite: &fish::Sprite, cx: f64, cy: f64, scale: f64, w: usize, h: usize) {
    let out_w = ((sprite.width as f64) * scale).round().max(1.0) as usize;
    let out_h = ((sprite.height as f64) * scale).round().max(1.0) as usize;
    let grid = sprite_dense(sprite);
    let left = cx.round() as isize - (out_w as isize) / 2;
    let top = cy.round() as isize - (out_h as isize) / 2;
    for oy in 0..out_h {
        for ox in 0..out_w {
            let sx = ((ox as f64) / scale).floor() as usize;
            let sy = ((oy as f64) / scale).floor() as usize;
            let sx = sx.min(sprite.width.saturating_sub(1));
            let sy = sy.min(sprite.height.saturating_sub(1));
            if let Some(base) = grid[sy * sprite.width + sx] {
                let px = left + ox as isize;
                let py = top + oy as isize;
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    fb.set_pixel(px as usize, py as usize, base);
                }
            }
        }
    }
}

// スター(無敵アイテム): 十字形の光る点。中心は明るい金色、周囲4点は控えめな輝きで、
// 時間経過(elapsed+phase)に応じてサイン波で明滅させ「キラキラ」感を出す。
fn draw_star(fb: &mut FrameBuffer, s: &Star, elapsed: f64, w: usize, h: usize) {
    let twinkle = ((elapsed * sim::STAR_TWINKLE_FREQ + s.phase).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
    let center = scale(Color::new(255, 235, 120), 0.7 + 0.3 * twinkle);
    let arm = scale(Color::new(255, 235, 120), 0.35 + 0.35 * twinkle);
    put(fb, s.x, s.y, center, w, h);
    put(fb, s.x - 1.0, s.y, arm, w, h);
    put(fb, s.x + 1.0, s.y, arm, w, h);
    put(fb, s.x, s.y - 1.0, arm, w, h);
    put(fb, s.x, s.y + 1.0, arm, w, h);
}

// カーソル(照準)を小さな十字(プラス形)で描く。魚・背景と被らない専用色。
fn draw_cursor(fb: &mut FrameBuffer, cx: f64, cy: f64, w: usize, h: usize) {
    put(fb, cx, cy, CURSOR, w, h);
    put(fb, cx - 1.0, cy, CURSOR, w, h);
    put(fb, cx + 1.0, cy, CURSOR, w, h);
    put(fb, cx, cy - 1.0, CURSOR, w, h);
    put(fb, cx, cy + 1.0, CURSOR, w, h);
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
        let auto = if ctl.auto_on { "ON" } else { "OFF" };
        // 既存の自動モード(自動餌やり/投薬/ガラス叩き)とは別枠の表示にする
        // (捕食する種ばかりだと魚がいなくなるという指摘への対応の新規トグル)。
        let auto_replenish = if ctl.auto_replenish_on { "ON" } else { "OFF" };
        let base = format!(
            " 魚 {}/{}  病気 {}  餌 {}  速度 {}  自動 {}  補充 {}  経過 {:02}:{:02}   [矢印]照準 [f]餌 [m]薬 [M]肉餌 [t]コンコン [T]トントン [p]停止 [[/]]速度 [R]初期化 [v]表示 [s]SE [a]自動 [A]補充 [+/-]増減 [S]ピラニア [O]タコ [D]タコつぼ [P]水草 [,]設定 [?]ヘルプ [q]終了 ",
            sim.fish_count(),
            cap,
            sim.sick_count(),
            sim.food_count(),
            speed,
            auto,
            auto_replenish,
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

// タイトルロゴ(この文字列パターンをそのまま使う)。起動スプラッシュとヘルプ画面の両方で使う。
const LOGO_BLOCK: [&str; 7] = [
    " ███    ███   █   █   ███   █████  █████  ████   █   █",
    "█   █  █   █  █   █  █   █    █    █      █   █  ██ ██",
    "█   █  █   █  █   █  █   █    █    █      █   █  █ █ █",
    "█████  █   █  █   █  █████    █    ████   ████   █   █",
    "█   █  █ █ █  █   █  █   █    █    █      █  █   █   █",
    "█   █  █  █   █   █  █   █    █    █      █   █  █   █",
    "█   █   ██ █   ███   █   █    █    █████  █   █  █   █",
];
const LOGO_SUBTITLE: &str = "        ~~~~  terminal aquarium  ~~~~";
// ロゴ下部の魚アート(ブロックアート版)
const LOGO_FISH_ART: [&str; 3] = [
    "   ▄▄▄▄▖              ▗▄▄▄▄",
    " ◁█●███▊▔▔      ▔▔▊███●█▷",
    "   ▀▀▀▀▘              ▝▀▀▀▀",
];
// ロゴ全体の行構成を組み立てる(描画にも行数計算にも同じものを使い、食い違いを防ぐ)。
fn build_logo_lines() -> Vec<(&'static str, (u8, u8, u8))> {
    let block_color = (200u8, 240u8, 255u8); // 明るい水色〜白
    let subtitle_color = (120u8, 190u8, 220u8); // 控えめな水色
    let fish_color = (240u8, 140u8, 20u8); // 金魚オレンジ

    let mut lines: Vec<(&str, (u8, u8, u8))> = Vec::with_capacity(LOGO_BLOCK.len() + LOGO_FISH_ART.len() + 3);
    for l in LOGO_BLOCK.iter() {
        lines.push((*l, block_color));
    }
    lines.push(("", block_color)); // 空行
    lines.push((LOGO_SUBTITLE, subtitle_color));
    lines.push(("", block_color)); // 空行
    for l in LOGO_FISH_ART.iter() {
        lines.push((*l, fish_color));
    }
    lines
}

// ロゴの表示に必要な最小幅(最も広い行の表示幅)
fn logo_width() -> usize {
    build_logo_lines()
        .iter()
        .map(|(text, _)| UnicodeWidthStr::width(*text))
        .max()
        .unwrap_or(0)
}

// ロゴを cols 幅の中央寄せで start_row から描画する。端末が狭すぎて収まらない場合は
// 何も描かず Ok(0) を返す(小さい端末での panic 防止。実際に描いた行数を返す)。
fn draw_logo(out: &mut Stdout, cols: usize, rows: usize, start_row: usize) -> std::io::Result<usize> {
    if rows == 0 || cols < logo_width() {
        return Ok(0);
    }
    let lines = build_logo_lines();

    let mut drawn = 0usize;
    for (i, (text, (r, g, b))) in lines.iter().enumerate() {
        let row = start_row + i;
        if row >= rows {
            break;
        }
        if !text.is_empty() {
            let w = UnicodeWidthStr::width(*text);
            let col = if cols > w { (cols - w) / 2 } else { 0 };
            queue!(
                out,
                MoveTo(col as u16, row as u16),
                Print(format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[0m"))
            )?;
        }
        drawn += 1;
    }
    Ok(drawn)
}

// 起動スプラッシュ画面: タイトルロゴを画面中央に表示する。何かキーを押すか
// SPLASH_DURATION 経過で消える(main.rs の run() 側で制御)。
fn draw_splash(out: &mut Stdout, cols: usize, rows: usize) -> std::io::Result<()> {
    queue!(out, Clear(ClearType::All))?;
    if cols >= logo_width() {
        let start_row = rows.saturating_sub(build_logo_lines().len()) / 2;
        draw_logo(out, cols, rows, start_row)?;
    }
    out.flush()
}

// 図鑑(ヘルプ画面の一部): 種類ごとの名前・特徴・実際のドット絵スプライトを並べる。
// 通常3種(ネオン/金魚/グッピー)+ピラニアが対象。成魚の見た目を使う。
fn dex_entries() -> Vec<(&'static str, &'static str, Species)> {
    // ヘルプが縦長すぎて図鑑が見切れるという指摘への対応で、図鑑を複数列の
    // グリッド表示に変更した。列数を確保しやすくするため説明文は短めにしてある
    // (詳しい説明は本文の育成要素の説明側にある)。
    vec![
        ("ネオン", "小型・速い", Species::Neon),
        ("金魚", "大きめ・ゆったり", Species::Goldfish),
        ("グッピー", "反応が速い", Species::Guppy),
        ("エンゼルフィッシュ", "縦長で優雅", Species::Angelfish),
        ("ベタ", "派手なヒレ", Species::Betta),
        ("ピラニア", "捕食者(Sキー)", Species::Piranha),
        ("タコ", "捕食者(Oキー)", Species::Octopus),
        ("クジラ", "巨大・ネタ枠(Wキー)", Species::Whale),
    ]
}

// 図鑑が狭い端末で収まらない場合のテキストのみフォールバック行。
// dex_entries() に種類を追加/削除したら、こちらも合わせて更新すること
// (実機フィードバック: 図鑑本体は7種類に対応済みなのに、この文言だけ旧4種のまま
// 取り残されていたのを見落とした事故があったため、dex_entries_covers_all_species_names
// のテストで両者の一致を機械的に検証する)。7種類分を1行に収めると幅が長くなりすぎるため、
// 複数行に分けている。
fn dex_fallback_lines() -> [&'static str; 6] {
    [
        "  種類: ネオン(青) / 金魚(オレンジ) / グッピー(白+差し色)",
        "        エンゼルフィッシュ(縦長で優雅) / ベタ(派手なヒレ)",
        "  捕食者: ピラニア(銀色+腹に赤み) / タコ(つぼに隠れ、時々出てくる)",
        "  特殊: クジラ(ずば抜けて大きいネタ枠・Wキーで追加、捕食も繁殖もしない)",
        "  観賞用: カニ・エビ(水底を歩く) / タツノオトシゴ(藻の近くを漂う)も泳いでいます",
        "",
    ]
}

// 図鑑1件分の表示に必要な幅(スプライト幅とラベル幅の大きい方)
fn dex_entry_width(name: &str, desc: &str, sp: Species) -> usize {
    let sprite = Fish::new(sp, Stage::Adult, 0.0, 0.0).sprite();
    let label_w = UnicodeWidthStr::width(format!("{name} — {desc}").as_str());
    sprite.width.max(label_w)
}

// 図鑑1件分の表示に使う「セル幅」(全種の中で最大のdex_entry_width)。
// グリッドの列を揃えるため、全エントリで同じセル幅を使う。
fn dex_cell_width() -> usize {
    dex_entries()
        .iter()
        .map(|(name, desc, sp)| dex_entry_width(name, desc, *sp))
        .max()
        .unwrap_or(0)
}

const DEX_COL_GAP: usize = 3; // 列の間の余白
const DEX_MAX_COLS: usize = 3; // 詰め込みすぎて窮屈にならないよう列数の上限を設ける

// 図鑑セクション全体の表示に必要な最小幅(端末が狭い場合のフォールバック判定に使う。
// 最低1列分は表示できる幅が必要)
fn dex_min_width() -> usize {
    dex_cell_width()
}

// 端末幅(cols)に応じて、図鑑を何列のグリッドで表示するか決める。
fn dex_grid_columns(cols: usize) -> usize {
    let cell_w = dex_cell_width();
    if cell_w == 0 {
        return 1;
    }
    (cols / (cell_w + DEX_COL_GAP)).clamp(1, DEX_MAX_COLS)
}

// 図鑑セクション全体の表示に必要な行数。ヘルプが縦長すぎて
// 図鑑が見切れるという指摘への対応で、全種を1列に縦積みするのではなく、端末幅に応じた
// 複数列のグリッドに詰めることで必要行数を大きく減らす(列ごとにその行の中で
// 最も背の高いスプライトの高さ+1行の空行、を列数で割った分だけで済む)。
fn dex_total_rows(cols: usize) -> usize {
    let num_cols = dex_grid_columns(cols);
    let heights: Vec<usize> = dex_entries()
        .iter()
        .map(|(_, _, sp)| Fish::new(*sp, Stage::Adult, 0.0, 0.0).sprite().height)
        .collect();
    heights
        .chunks(num_cols)
        .map(|chunk| chunk.iter().max().copied().unwrap_or(0) + 2)
        .sum()
}

// 図鑑を start_row から複数列のグリッドで描く。端末が狭すぎる場合は何も描かず
// Ok(0) を返す(呼び出し側でテキストのみのフォールバック行に切り替える)。
fn draw_species_dex(
    out: &mut Stdout,
    cols: usize,
    rows: usize,
    start_row: usize,
) -> std::io::Result<usize> {
    let cell_w = dex_cell_width();
    if cols < cell_w {
        return Ok(0);
    }
    let num_cols = dex_grid_columns(cols);
    let grid_w = num_cols * cell_w + (num_cols.saturating_sub(1)) * DEX_COL_GAP;
    let left = if cols > grid_w { (cols - grid_w) / 2 } else { 0 };

    let mut row = start_row;
    for group in dex_entries().chunks(num_cols) {
        if row >= rows {
            break;
        }
        let sprites: Vec<_> = group
            .iter()
            .map(|(_, _, sp)| Fish::new(*sp, Stage::Adult, 0.0, 0.0).sprite())
            .collect();
        let group_h = sprites.iter().map(|s| s.height).max().unwrap_or(0);

        for (i, (name, desc, _)) in group.iter().enumerate() {
            let col_x = left + i * (cell_w + DEX_COL_GAP);
            let label = format!("{name} — {desc}");
            queue!(
                out,
                MoveTo(col_x as u16, row as u16),
                Print(format!("\x1b[38;2;220;235;245m{label}\x1b[0m"))
            )?;
        }
        row += 1;

        for (i, sprite) in sprites.iter().enumerate() {
            let col_x = left + i * (cell_w + DEX_COL_GAP);
            let grid = sprite_dense(sprite);
            for dy in 0..sprite.height {
                let r = row + dy;
                if r >= rows {
                    break;
                }
                for dx in 0..sprite.width {
                    if let Some(c) = grid[dy * sprite.width + dx] {
                        queue!(
                            out,
                            MoveTo((col_x + dx) as u16, r as u16),
                            Print(format!("\x1b[38;2;{};{};{}m█\x1b[0m", c.r, c.g, c.b))
                        )?;
                    }
                }
            }
        }
        row += group_h + 1; // スプライト最大高さ + 行間の空行
    }
    Ok(row.saturating_sub(start_row))
}

// 設定パネル(,キー): サイドメニューとして画面右側に固定幅で重ねて描く
// (全画面Clearしない・水槽の描画は止めずに裏で動き続ける)。上下キーで選択・
// Enter/スペースで切替・Escで閉じる。効果音等の既存トグルに加え、通常5種
// それぞれの「生成トグル」(species_toggle)も一覧する。
fn draw_settings_panel(
    out: &mut Stdout,
    ctl: &Ctl,
    sim: &Simulation,
    cols: usize,
    rows: usize,
) -> std::io::Result<()> {
    if cols == 0 || rows == 0 {
        return Ok(());
    }
    let panel_w = 34usize.min(cols);
    let panel_col = cols - panel_w;
    let bg = "\x1b[48;2;18;26;38m";
    let fg = "\x1b[38;2;220;235;245m";
    let dim_fg = "\x1b[38;2;170;190;205m";
    let selected_fg = "\x1b[38;2;255;230;140m";

    let species_names = ["ネオン", "金魚", "グッピー", "エンゼルフィッシュ", "ベタ"];
    let on_off = |on: bool| if on { "ON".to_string() } else { "OFF".to_string() };
    let mut items: Vec<(String, String)> = vec![
        ("効果音(SE)".to_string(), on_off(ctl.sfx_on)),
        ("オーバーレイ表示".to_string(), on_off(ctl.overlay_on)),
        ("自動モード".to_string(), on_off(ctl.auto_on)),
        ("昼夜連動".to_string(), on_off(ctl.day_night_on)),
        ("自動魚補充".to_string(), on_off(ctl.auto_replenish_on)),
        ("気泡音".to_string(), on_off(ctl.bubble_sfx_on)),
    ];
    for (i, name) in species_names.iter().enumerate() {
        items.push((format!("生成:{name}"), on_off(sim.species_toggle[i])));
    }
    items.push((
        "餌の量".to_string(),
        sim::FEED_AMOUNT_LABELS[sim.feed_amount].to_string(),
    ));
    items.push(("カニ".to_string(), on_off(sim.crab_toggle)));

    let mut lines: Vec<(String, &str)> = Vec::new();
    lines.push((" 設定".to_string(), fg));
    lines.push((String::new(), fg));
    for (i, (label, status)) in items.iter().enumerate() {
        let marker = if i == ctl.settings_selected { "▶" } else { " " };
        let color = if i == ctl.settings_selected { selected_fg } else { fg };
        lines.push((format!(" {marker}{label}:{status}"), color));
    }
    lines.push((String::new(), fg));
    lines.push((" ↑↓ 選択".to_string(), dim_fg));
    lines.push((" Enter/Space 切替".to_string(), dim_fg));
    lines.push((" Esc 閉じる".to_string(), dim_fg));

    for row in 0..rows {
        let (text, color) = lines
            .get(row)
            .map(|(t, c)| (t.as_str(), *c))
            .unwrap_or(("", fg));
        let padded = fit_width(text, panel_w);
        queue!(
            out,
            MoveTo(panel_col as u16, row as u16),
            Print(format!("{bg}{color}{padded}\x1b[0m"))
        )?;
    }
    out.flush()
}

fn draw_help(out: &mut Stdout, cols: usize, rows: usize) -> std::io::Result<()> {
    // ヘルプが縦長すぎて図鑑が見切れるという指摘への対応で、キー操作を
    // 詳細な1キー1行から、関連キーをまとめた行に圧縮した(縦の行数を減らし、
    // 図鑑の表示に使える余白を確保するため)。
    let intro_lines = [
        "",
        "  aquaterm — 端末熱帯魚アクアリウム",
        "",
        "  魚に餌をやって育てよう。満腹を保つと成長し、まれに産卵→孵化で増えます。",
        "  空腹が続いたり過密だと病気になります。薬で治療を。長く放置すると力尽きて",
        "  仰向けに浮き、しばらくして水槽から消えます。",
        "",
        "  基本操作:",
        "    矢印キー  カーソル移動(Shift+矢印で高速)    f / m   餌 / 薬    t / T   コンコン / トントン",
        "    p  一時停止/再開    [ / ]  速度変更    ,  設定画面    ?  ヘルプ    q  終了",
        "",
        "  その他: v オーバーレイ / s 効果音 / a 自動モード / A 自動魚補充 / R リセット",
        "          + - 追加/間引き / S ピラニア / O タコ / W クジラ / M 肉餌 / D タコつぼ / P 水草",
        "          H 全員空腹に(デバッグ)  K つがいを即座に交尾させる(デバッグ)",
        "          J 水質トグル / X ランダム死亡 / Z スター投入 / L 寿命残り10秒(いずれもデバッグ)",
        "",
    ];
    // 図鑑が狭い端末で収まらない場合のテキストのみフォールバック行
    let dex_fallback = dex_fallback_lines();
    let outro_lines = ["  何かキーを押すと水槽に戻ります。", ""];

    queue!(out, Clear(ClearType::All))?;
    // ロゴを先頭(上端)に表示し、収まらない端末ではスキップして本文だけ表示する
    let logo_rows = draw_logo(out, cols, rows, 0)?;
    let body_start = if logo_rows > 0 { logo_rows + 1 } else { 0 };

    let remaining = rows.saturating_sub(body_start);
    // 図鑑は幅だけでなく、高さも十分に余っているときだけ使う。片方でも足りない
    // 端末では、中途半端に描画が欠けるのではなくテキストのみのフォールバックにする。
    let dex_fits = cols >= dex_min_width()
        && intro_lines.len() + dex_total_rows(cols) + outro_lines.len() <= remaining;
    let dex_rows_needed = if dex_fits { dex_total_rows(cols) } else { 0 };
    let fallback_rows_needed = if dex_fits { 0 } else { dex_fallback.len() };
    let total_body_len =
        intro_lines.len() + dex_rows_needed + fallback_rows_needed + outro_lines.len();

    let start_row = body_start + remaining.saturating_sub(total_body_len) / 2;

    // ヘルプが縦長すぎて図鑑が見切れるという指摘への対応の副次修正: 端末が
    // 想定より低く、全体が収まりきらない場合に、はみ出した行を最終行へ重ねて
    // 描いてしまい文字が重なって読めなくなるバグがあった(`row.min(rows-1)`で
    // クランプしていたため)。収まらない分は素直に描かず打ち切る(breakする)ように
    // 修正し、重なりを防ぐ。
    let mut row = start_row;
    for line in intro_lines.iter() {
        if row >= rows {
            break;
        }
        let w = UnicodeWidthStr::width(*line);
        let col = if cols > w { (cols - w) / 2 } else { 0 };
        queue!(
            out,
            MoveTo(col as u16, row as u16),
            Print(format!("\x1b[38;2;220;235;245m{line}\x1b[0m"))
        )?;
        row += 1;
    }

    if dex_fits {
        let drawn = draw_species_dex(out, cols, rows, row)?;
        row += drawn;
    } else {
        for line in dex_fallback.iter() {
            if row >= rows {
                break;
            }
            let w = UnicodeWidthStr::width(*line);
            let col = if cols > w { (cols - w) / 2 } else { 0 };
            queue!(
                out,
                MoveTo(col as u16, row as u16),
                Print(format!("\x1b[38;2;220;235;245m{line}\x1b[0m"))
            )?;
            row += 1;
        }
    }

    for line in outro_lines.iter() {
        if row >= rows {
            break;
        }
        let w = UnicodeWidthStr::width(*line);
        let col = if cols > w { (cols - w) / 2 } else { 0 };
        queue!(
            out,
            MoveTo(col as u16, row as u16),
            Print(format!("\x1b[38;2;220;235;245m{line}\x1b[0m"))
        )?;
        row += 1;
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

    // 回帰テスト: ロゴが要求する最小幅を把握しておく(60桁未満ならスキップされる想定の根拠)。
    // 文字列が将来書き換わって極端に幅が変わった場合に気づけるようにする。
    #[test]
    fn logo_width_is_reasonable_and_guards_tiny_terminals() {
        let w = logo_width();
        assert!(w > 40, "ロゴはある程度の幅を持つはず: {w}");
        assert!(w < 200, "ロゴの幅が異常に大きくなっていないか: {w}");
    }

    // ヘルプ画面の図鑑: 通常5種+ピラニア+タコ+クジラの8種類を対象にすること
    #[test]
    fn dex_entries_cover_all_eight_species() {
        let entries = dex_entries();
        assert_eq!(
            entries.len(),
            8,
            "ネオン/金魚/グッピー/エンゼルフィッシュ/ベタ/ピラニア/タコ/クジラの8種類のはず"
        );
        assert!(entries.iter().any(|(_, _, sp)| *sp == Species::Neon));
        assert!(entries.iter().any(|(_, _, sp)| *sp == Species::Goldfish));
        assert!(entries.iter().any(|(_, _, sp)| *sp == Species::Guppy));
        assert!(entries.iter().any(|(_, _, sp)| *sp == Species::Angelfish));
        assert!(entries.iter().any(|(_, _, sp)| *sp == Species::Betta));
        assert!(entries.iter().any(|(_, _, sp)| *sp == Species::Piranha));
        assert!(entries.iter().any(|(_, _, sp)| *sp == Species::Octopus));
        assert!(entries.iter().any(|(_, _, sp)| *sp == Species::Whale));
    }

    // 回帰テスト: 狭い端末向けのテキストのみフォールバック行が、図鑑本体
    // (dex_entries)に載っている全種類の名前を機械的に含んでいることを確認する。
    // (実機フィードバック: 図鑑本体は7種類対応済みなのに、このフォールバック文言だけ
    // 旧4種のまま取り残されていた見落とし事故の再発防止)
    #[test]
    fn dex_fallback_text_mentions_every_species_in_dex_entries() {
        let fallback_text = dex_fallback_lines().join("\n");
        for (name, _desc, _sp) in dex_entries() {
            assert!(
                fallback_text.contains(name),
                "図鑑フォールバック文言に「{name}」が含まれていないはず: dex_entries()に追加されたのにフォールバック文言の更新が漏れている"
            );
        }
    }

    // エビ・タツノオトシゴを追加したのに観賞用の行がカニのまま取り残されていた、
    // という指摘の再発防止。図鑑フォールバック文言と同じ「テキスト一覧に
    // 新要素が反映されていない」パターンなので、同様に回帰テストを置く。
    #[test]
    fn dex_fallback_text_mentions_all_decorative_background_creatures() {
        let fallback_text = dex_fallback_lines().join("\n");
        for name in ["カニ", "エビ", "タツノオトシゴ"] {
            assert!(
                fallback_text.contains(name),
                "観賞用背景生物「{name}」がフォールバック文言に含まれていないはず"
            );
        }
    }

    // 図鑑の最小幅・必要行数は正の値で、極端に大きくならないこと(狭い端末での
    // フォールバック判定・レイアウト計算がおかしくなっていないかの回帰テスト)。
    #[test]
    fn dex_min_width_and_total_rows_are_reasonable() {
        let w = dex_min_width();
        // 十分広い端末(120桁)なら複数列のグリッドで表示できるはず
        let rows = dex_total_rows(120);
        assert!(w > 0 && w < 100, "図鑑の最小幅が異常な値になっていないか: {w}");
        // ヘルプが縦長すぎて図鑑が見切れるという指摘への対応でグリッド化した
        // ため、単純な縦積み(旧仕様は150行未満が許容上限)よりずっと少ない行数で
        // 収まるはず。
        assert!(
            rows > 0 && rows < 60,
            "図鑑の必要行数が異常な値になっていないか: {rows}"
        );
        // ピラニアは既存3種より大きいドット絵なので、図鑑の最小幅はピラニア単体の幅以上のはず
        let piranha_w = dex_entry_width("ピラニア", "捕食者(Sキー)", Species::Piranha);
        assert!(w >= piranha_w);
    }

    // グリッド化の回帰テスト: 十分広い端末では複数列に分かれ、必要行数が
    // 単純な縦積み(全7種の高さ合計)よりはっきり少なくなるはず。
    #[test]
    fn dex_grid_uses_multiple_columns_on_wide_terminals_and_saves_rows() {
        let stacked_rows: usize = dex_entries()
            .iter()
            .map(|(_, _, sp)| Fish::new(*sp, Stage::Adult, 0.0, 0.0).sprite().height + 2)
            .sum();
        let wide_cols = 160;
        assert!(
            dex_grid_columns(wide_cols) >= 2,
            "十分広い端末では2列以上のグリッドになるはず: {}",
            dex_grid_columns(wide_cols)
        );
        let grid_rows = dex_total_rows(wide_cols);
        assert!(
            grid_rows < stacked_rows,
            "グリッド化後の行数({grid_rows})は縦積み({stacked_rows})より少ないはず"
        );
    }

    // 狭い端末では1列にフォールバックし、パニックしないことを確認する。
    #[test]
    fn dex_grid_falls_back_to_a_single_column_on_narrow_terminals() {
        assert_eq!(dex_grid_columns(10), 1);
        let rows = dex_total_rows(10);
        assert!(rows > 0);
    }
}
