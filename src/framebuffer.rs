// ハーフブロック(▀)による高密度カラー描画用フレームバッファ。
// 論理ピクセルグリッドは「セル高さの2倍」の解像度を持ち、上下2ピクセルを
// 1文字(前景色=上半分, 背景色=下半分, 文字=▀)に合成する。
// 前フレームとの diff のみ crossterm に書き込みちらつきを抑える。

use crate::color::Color;
use crossterm::{cursor::MoveTo, queue, style::Print};
use std::io::Write;

pub struct FrameBuffer {
    pub cols: usize,      // セル横幅 = 端末幅
    pub cell_rows: usize, // セル縦幅(=水槽の高さ。ステータスバー分を除く)
    pub pix_rows: usize,  // 論理ピクセル縦解像度 = cell_rows * 2
    pixels: Vec<Color>,   // cols * pix_rows
    // diff 用の前フレームのセル状態(上色, 下色)。None は未描画。
    prev: Vec<Option<(Color, Color)>>,
    force_redraw: bool,
}

impl FrameBuffer {
    pub fn new(cols: usize, cell_rows: usize) -> Self {
        let pix_rows = cell_rows * 2;
        FrameBuffer {
            cols,
            cell_rows,
            pix_rows,
            pixels: vec![Color::new(0, 0, 0); cols * pix_rows],
            prev: vec![None; cols * cell_rows],
            force_redraw: true,
        }
    }

    // 端末リサイズ時に呼ぶ。バッファを作り直し全再描画させる。
    pub fn resize(&mut self, cols: usize, cell_rows: usize) {
        *self = FrameBuffer::new(cols, cell_rows);
    }

    // 次フレームで全セルを描き直させる(ヘルプ画面から復帰した時など)。
    pub fn force_full_redraw(&mut self) {
        self.force_redraw = true;
    }

    pub fn pix_width(&self) -> usize {
        self.cols
    }

    pub fn pix_height(&self) -> usize {
        self.pix_rows
    }

    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, c: Color) {
        if x < self.cols && y < self.pix_rows {
            self.pixels[y * self.cols + x] = c;
        }
    }

    // 既存ピクセル色を読む(血の滲みのように背景色とブレンドする描画で使う)。
    // 範囲外は黒(初期値)を返す(set_pixelと同じ境界チェックの考え方)。
    #[inline]
    pub fn get_pixel(&self, x: usize, y: usize) -> Color {
        if x < self.cols && y < self.pix_rows {
            self.pixels[y * self.cols + x]
        } else {
            Color::new(0, 0, 0)
        }
    }

    // diff を stdout に流す。次フレーム用に前状態を更新する。
    pub fn flush<W: Write>(&mut self, out: &mut W) -> std::io::Result<()> {
        for cy in 0..self.cell_rows {
            let top_row = cy * 2;
            let bot_row = cy * 2 + 1;
            // 行内で連続する変更セルはまとめて出力(MoveTo 回数を減らす)
            let mut cx = 0;
            while cx < self.cols {
                let top = self.pixels[top_row * self.cols + cx];
                let bot = self.pixels[bot_row * self.cols + cx];
                let idx = cy * self.cols + cx;
                let changed = self.force_redraw || self.prev[idx] != Some((top, bot));
                if !changed {
                    cx += 1;
                    continue;
                }
                // 変更開始。連続する変更範囲を一気に書く。
                queue!(out, MoveTo(cx as u16, cy as u16))?;
                let mut run = String::new();
                while cx < self.cols {
                    let top = self.pixels[top_row * self.cols + cx];
                    let bot = self.pixels[bot_row * self.cols + cx];
                    let idx = cy * self.cols + cx;
                    let ch = self.force_redraw || self.prev[idx] != Some((top, bot));
                    if !ch {
                        break;
                    }
                    // 前景=上ピクセル, 背景=下ピクセル, 文字=▀
                    run.push_str(&format!(
                        "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m\u{2580}",
                        top.r, top.g, top.b, bot.r, bot.g, bot.b
                    ));
                    self.prev[idx] = Some((top, bot));
                    cx += 1;
                }
                run.push_str("\x1b[0m");
                queue!(out, Print(run))?;
            }
        }
        self.force_redraw = false;
        out.flush()
    }
}
