// 水槽シミュレーション本体: 魚の遊泳・餌・薬・卵・気泡の更新、育成ロジック
// (空腹度・成長・産卵→孵化・病気・死亡)。端末描画には依存しない純粋なロジック。

use crate::fish::{Fish, Species, Stage};
use crate::rng::Rng;
use serde::{Deserialize, Serialize};

// --- 空腹度の段階しきい値(fish.rs の hunger_level が参照) ---
pub const MAX_HUNGER: f64 = 100.0;
pub const FULL_THRESHOLD: f64 = 75.0; // これ以上で「満腹」
pub const HUNGRY_THRESHOLD: f64 = 50.0; // これ未満で「腹ぺこ」= 餌を探す

// --- 育成パラメータ(すべて秒単位) ---
pub const HUNGER_DECAY: f64 = 1.6; // 空腹度の毎秒減少量
pub const FEED_AMOUNT: f64 = 34.0; // 餌1粒で回復する空腹度
pub const WELL_FED_THRESHOLD: f64 = 60.0; // 成長・産卵の満腹判定
pub const GROW_TIME: f64 = 30.0; // 満腹維持で稚魚→成魚
pub const BREED_READY_TIME: f64 = 22.0; // 成魚が満腹維持でこの時間経つと産卵可能
pub const BREED_CHANCE_PER_SEC: f64 = 0.06; // 産卵可能時、毎秒の産卵確率
pub const EGG_HATCH_TIME: f64 = 14.0; // 卵が孵化するまでの時間
pub const STARVE_WEAK_TIME: f64 = 8.0; // 空腹度0がこの時間続くと弱る
pub const STARVE_DEATH_TIME: f64 = 22.0; // さらに続くと死亡

// --- 病気パラメータ ---
pub const HUNGRY_SICK_TIME: f64 = 10.0; // 腹ぺこがこの時間続くと発症判定対象
pub const OVERCROWD_RATIO: f64 = 0.9; // 個体数/上限がこれ以上で過密=発症判定対象
pub const DISEASE_CHANCE_PER_SEC: f64 = 0.03; // 発症条件下での毎秒発症確率
pub const SICK_WEAK_TIME: f64 = 12.0; // 病気がこの時間続くと弱る
pub const SICK_DEATH_TIME: f64 = 30.0; // さらに続くと死亡

// --- 餌・薬・気泡パラメータ ---
pub const FOOD_SINK_SPEED: f64 = 7.0; // 餌の沈降速度(px/秒)
pub const FOOD_LIFETIME: f64 = 26.0; // 餌の寿命(秒)
pub const EAT_RADIUS: f64 = 3.2; // 魚が餌を食べられる距離
pub const MED_SINK_SPEED: f64 = 5.0; // 薬の沈降速度
pub const MED_LIFETIME: f64 = 26.0; // 薬の寿命
pub const CURE_RADIUS: f64 = 3.2; // 病気の魚が薬で治る距離
pub const NEIGHBOR_RADIUS: f64 = 16.0; // 群れ判定の近傍距離

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Food {
    pub x: f64,
    pub y: f64,
    pub vy: f64,
    pub life: f64,
}

// 薬(病気治療用の粒。餌とは別色)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Medicine {
    pub x: f64,
    pub y: f64,
    pub vy: f64,
    pub life: f64,
}

// 卵(水底付近に産まれ、一定時間で孵化して稚魚になる)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Egg {
    pub x: f64,
    pub y: f64,
    pub species: Species,
    pub hatch: f64, // 孵化までの残り時間
}

#[derive(Clone, Debug)]
pub struct Bubble {
    pub x: f64,
    pub y: f64,
    pub vy: f64,
}

pub struct Simulation {
    pub fish: Vec<Fish>,
    pub food: Vec<Food>,
    pub medicine: Vec<Medicine>,
    pub eggs: Vec<Egg>,
    pub bubbles: Vec<Bubble>,
    pub rng: Rng,
    pub elapsed: f64,            // 累計経過秒
    pub message: Option<String>, // ステータスバー用の一言
    message_ttl: f64,
    bubble_timer: f64,
}

// 水底(砂)の高さ(論理ピクセル)
pub fn sand_height(pix_h: usize) -> usize {
    (pix_h / 12).max(2)
}

// 端末サイズに応じた個体数上限
pub fn capacity(pix_w: usize, pix_h: usize) -> usize {
    ((pix_w * pix_h) / 1400).clamp(5, 40)
}

// `x.clamp(1.0, upper)` の upper が 1.0 未満(NaN含む)だと `min > max` で panic するため、
// upper を必ず 1.0 以上に補正してから渡すための安全弁。
// 端末が極端に小さく pix_w/pix_h が小さい場合の防御(main.rs 側の最小サイズ保証と二重で守る)。
fn safe_upper(v: f64) -> f64 {
    if v.is_finite() {
        v.max(1.0)
    } else {
        1.0
    }
}

impl Simulation {
    pub fn new(rng: Rng) -> Self {
        Simulation {
            fish: Vec::new(),
            food: Vec::new(),
            medicine: Vec::new(),
            eggs: Vec::new(),
            bubbles: Vec::new(),
            rng,
            elapsed: 0.0,
            message: None,
            message_ttl: 0.0,
            bubble_timer: 0.0,
        }
    }

    // 初期個体を撒く(セーブが無い初回起動 / リセット用)
    pub fn seed_initial(&mut self, pix_w: usize, pix_h: usize) {
        let n = 5.min(capacity(pix_w, pix_h));
        for i in 0..n {
            let sp = Species::ALL[i % Species::ALL.len()];
            let stage = if i % 2 == 0 { Stage::Adult } else { Stage::Fry };
            let x = self.rng.range(6.0, (pix_w as f64 - 6.0).max(6.0));
            let y = self
                .rng
                .range(4.0, (pix_h as f64 - sand_height(pix_h) as f64 - 2.0).max(4.0));
            self.fish.push(Fish::new(sp, stage, x, y));
        }
    }

    // グレートリセット: 魚を初期構成へ、卵・餌・薬・経過時間を消去
    pub fn reset(&mut self, pix_w: usize, pix_h: usize) {
        self.fish.clear();
        self.food.clear();
        self.medicine.clear();
        self.eggs.clear();
        self.bubbles.clear();
        self.elapsed = 0.0;
        self.seed_initial(pix_w, pix_h);
        self.set_message("水槽をリセットしました");
    }

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
        self.message_ttl = 4.0;
    }

    // 餌やり: 中央上部から3〜5粒を投下
    pub fn feed(&mut self, pix_w: usize) {
        let count = self.rng.range_usize(3, 5);
        let cx = pix_w as f64 / 2.0;
        for _ in 0..count {
            self.food.push(Food {
                x: (cx + self.rng.range(-6.0, 6.0)).clamp(1.0, safe_upper(pix_w as f64 - 1.0)),
                y: self.rng.range(1.0, 4.0),
                vy: FOOD_SINK_SPEED * self.rng.range(0.8, 1.2),
                life: FOOD_LIFETIME,
            });
        }
    }

    // 投薬: 中央上部から数粒の薬を投下
    pub fn medicate(&mut self, pix_w: usize) {
        let count = self.rng.range_usize(3, 5);
        let cx = pix_w as f64 / 2.0;
        for _ in 0..count {
            self.medicine.push(Medicine {
                x: (cx + self.rng.range(-6.0, 6.0)).clamp(1.0, safe_upper(pix_w as f64 - 1.0)),
                y: self.rng.range(1.0, 4.0),
                vy: MED_SINK_SPEED * self.rng.range(0.8, 1.2),
                life: MED_LIFETIME,
            });
        }
    }

    // デバッグ: 魚を1匹追加
    pub fn add_fish(&mut self, pix_w: usize, pix_h: usize) {
        if self.fish.len() >= capacity(pix_w, pix_h) {
            self.set_message("水槽が満員です");
            return;
        }
        let sp = Species::ALL[self.rng.range_usize(0, Species::ALL.len() - 1)];
        let x = self.rng.range(6.0, (pix_w as f64 - 6.0).max(6.0));
        let y = self
            .rng
            .range(4.0, (pix_h as f64 - sand_height(pix_h) as f64 - 2.0).max(4.0));
        self.fish.push(Fish::new(sp, Stage::Fry, x, y));
    }

    // デバッグ: 魚を1匹間引く
    pub fn remove_fish(&mut self) {
        self.fish.pop();
    }

    pub fn fish_count(&self) -> usize {
        self.fish.len()
    }

    pub fn food_count(&self) -> usize {
        self.food.len()
    }

    pub fn sick_count(&self) -> usize {
        self.fish.iter().filter(|f| f.sick).count()
    }

    // 1tick分の更新。dt=経過秒(速度倍率適用済み), (pix_w,pix_h)=論理ピクセル寸法。
    // dt=0(一時停止)なら時間経過ロジックは進まない。
    pub fn update(&mut self, dt: f64, pix_w: usize, pix_h: usize) {
        if dt <= 0.0 {
            return;
        }
        self.elapsed += dt;
        if self.message_ttl > 0.0 {
            self.message_ttl -= dt;
            if self.message_ttl <= 0.0 {
                self.message = None;
            }
        }
        let cap = capacity(pix_w, pix_h);
        // sand_height は pix_h に対して最大2までしか保証しないため、pix_h が極端に小さいと
        // sand_top が 0 以下になり得る。水面〜水底の描画領域として最低2px は確保する。
        let sand_top = (pix_h as f64 - sand_height(pix_h) as f64).max(2.0);

        self.update_movement(dt, pix_w as f64, sand_top);
        self.update_food(dt, sand_top);
        self.update_medicine(dt, sand_top);
        self.update_biology(dt, cap, pix_w as f64, sand_top);
        self.update_bubbles(dt, pix_w as f64, pix_h as f64);
    }

    // 遊泳: ランダムウォーク+慣性+壁反射+群れ+餌吸引(空腹度・病気で速度が変化)
    fn update_movement(&mut self, dt: f64, w: f64, sand_top: f64) {
        // 群れ計算のため位置・速度をスナップショット
        let snap: Vec<(Species, f64, f64, f64, f64)> = self
            .fish
            .iter()
            .map(|f| (f.species, f.x, f.y, f.vx, f.vy))
            .collect();

        let margin = 4.0;
        let top_margin = 3.0;
        let wall_push = 70.0;

        for i in 0..self.fish.len() {
            let (sp, hunger, fx, fy, spd_mult, hungry) = {
                let f = &self.fish[i];
                (
                    f.species,
                    f.hunger,
                    f.x,
                    f.y,
                    f.speed_mult(),
                    f.hunger < HUNGRY_THRESHOLD,
                )
            };
            let mut ax = 0.0;
            let mut ay = 0.0;

            // ランダムウォーク(縦は控えめ)。空腹度・病気に応じて活発さが変わる
            ax += self.rng.signed() * sp.wander() * spd_mult;
            ay += self.rng.signed() * sp.wander() * 0.55 * spd_mult;

            // 群れ: 同種近傍の平均速度に少し寄せる
            let (mut svx, mut svy, mut cnt) = (0.0, 0.0, 0);
            for (j, &(osp, ox, oy, ovx, ovy)) in snap.iter().enumerate() {
                if j == i || osp != sp {
                    continue;
                }
                let d = ((ox - fx).powi(2) + (oy - fy).powi(2)).sqrt();
                if d < NEIGHBOR_RADIUS && d > 0.001 {
                    svx += ovx;
                    svy += ovy;
                    cnt += 1;
                }
            }
            if cnt > 0 {
                ax += (svx / cnt as f64) * 0.8;
                ay += (svy / cnt as f64) * 0.8;
            }

            // 餌吸引: 腹ぺこほど強く最寄りの餌へ向かう
            if hunger < HUNGRY_THRESHOLD && !self.food.is_empty() {
                let mut best = f64::INFINITY;
                let (mut bx, mut by) = (0.0, 0.0);
                for fd in &self.food {
                    let d = (fd.x - fx).powi(2) + (fd.y - fy).powi(2);
                    if d < best {
                        best = d;
                        bx = fd.x;
                        by = fd.y;
                    }
                }
                let dist = best.sqrt().max(0.001);
                // 空腹なほど吸引が強い(腹ぺこは spd_mult>1 と相まってより強く寄る)
                let pull = sp.food_pull() * (1.0 - hunger / HUNGRY_THRESHOLD) * spd_mult;
                ax += (bx - fx) / dist * pull;
                ay += (by - fy) / dist * pull;
            }

            // 壁の手前で緩やかに向きを変える(反射)
            if fx < margin {
                ax += wall_push;
            } else if fx > w - margin {
                ax -= wall_push;
            }
            if fy < top_margin {
                ay += wall_push;
            } else if fy > sand_top - 1.0 {
                ay -= wall_push;
            }

            let f = &mut self.fish[i];
            f.vx += ax * dt;
            f.vy += ay * dt;
            // 慣性(ドラッグ)
            let drag = (1.0 - 0.9 * dt).clamp(0.0, 1.0);
            f.vx *= drag;
            f.vy *= drag;
            // 最高速度でクランプ(空腹度・病気で上限が変わる)
            let speed = (f.vx * f.vx + f.vy * f.vy).sqrt();
            let maxs = sp.max_speed() * spd_mult;
            if speed > maxs {
                f.vx = f.vx / speed * maxs;
                f.vy = f.vy / speed * maxs;
            }
            // 積分
            f.x += f.vx * dt;
            f.y += f.vy * dt;
            // 位置クランプ
            f.x = f.x.clamp(1.0, safe_upper(w - 1.0));
            f.y = f.y.clamp(1.0, safe_upper(sand_top - 1.0));
            // 進行方向で左右反転(微小速度では維持)
            if f.vx > 0.6 {
                f.facing_right = true;
            } else if f.vx < -0.6 {
                f.facing_right = false;
            }
            let _ = hungry;
        }
    }

    // 餌: 沈降・寿命・捕食
    fn update_food(&mut self, dt: f64, sand_top: f64) {
        for fd in &mut self.food {
            fd.y += fd.vy * dt;
            fd.life -= dt;
        }
        // 捕食: 餌に十分近い魚がいれば食べる
        let mut eaten = vec![false; self.food.len()];
        for (fi, fd) in self.food.iter().enumerate() {
            for f in &mut self.fish {
                let d = ((fd.x - f.x).powi(2) + (fd.y - f.y).powi(2)).sqrt();
                if d < EAT_RADIUS {
                    f.hunger = (f.hunger + FEED_AMOUNT).min(MAX_HUNGER);
                    eaten[fi] = true;
                    break;
                }
            }
        }
        let mut idx = 0;
        self.food.retain(|fd| {
            let keep = !eaten[idx] && fd.life > 0.0 && fd.y < sand_top;
            idx += 1;
            keep
        });
    }

    // 薬: 沈降・寿命・治癒(病気の魚が触れると治る。健康な魚には無害)
    fn update_medicine(&mut self, dt: f64, sand_top: f64) {
        for md in &mut self.medicine {
            md.y += md.vy * dt;
            md.life -= dt;
        }
        let mut used = vec![false; self.medicine.len()];
        let mut cured = false;
        for (mi, md) in self.medicine.iter().enumerate() {
            for f in &mut self.fish {
                if !f.sick {
                    continue; // 健康な魚は薬に反応しない
                }
                let d = ((md.x - f.x).powi(2) + (md.y - f.y).powi(2)).sqrt();
                if d < CURE_RADIUS {
                    f.sick = false;
                    f.sick_timer = 0.0;
                    used[mi] = true;
                    cured = true;
                    break;
                }
            }
        }
        if cured {
            self.set_message("薬で病気が治った");
        }
        let mut idx = 0;
        self.medicine.retain(|md| {
            let keep = !used[idx] && md.life > 0.0 && md.y < sand_top;
            idx += 1;
            keep
        });
    }

    // 育成: 空腹度減少・病気の発症/進行・成長・産卵・孵化・死亡
    fn update_biology(&mut self, dt: f64, cap: usize, w: f64, sand_top: f64) {
        let count = self.fish.len();
        let overcrowded = count as f64 >= cap as f64 * OVERCROWD_RATIO;
        let mut messages: Vec<String> = Vec::new();
        // 産卵イベント: (親x, 親y, 種)。借用の都合で後からまとめて卵を生成する。
        let mut spawn_eggs: Vec<(f64, f64, Species)> = Vec::new();

        for f in &mut self.fish {
            // 空腹度の減少
            f.hunger = (f.hunger - HUNGER_DECAY * dt).max(0.0);

            // 腹ぺこ継続時間
            if f.hunger < HUNGRY_THRESHOLD {
                f.hungry_timer += dt;
            } else {
                f.hungry_timer = 0.0;
            }

            // 満腹維持タイマー
            if f.hunger >= WELL_FED_THRESHOLD {
                f.well_fed_timer += dt;
            } else {
                f.well_fed_timer = (f.well_fed_timer - dt).max(0.0);
            }

            // 病気の発症: 腹ぺこ長期 or 過密で確率的に発症
            if !f.sick {
                let eligible = f.hungry_timer >= HUNGRY_SICK_TIME || overcrowded;
                if eligible && self.rng.next_f64() < DISEASE_CHANCE_PER_SEC * dt {
                    f.sick = true;
                    f.sick_timer = 0.0;
                    messages.push(format!("{}が病気になった…[m]で薬を", species_name(f.species)));
                }
            }
            // 病気の進行
            if f.sick {
                f.sick_timer += dt;
            }

            // 成長・産卵は病気中は停止
            if !f.sick {
                // 成長: 稚魚→成魚
                if f.stage == Stage::Fry && f.well_fed_timer >= GROW_TIME {
                    f.stage = Stage::Adult;
                    f.well_fed_timer = 0.0;
                    messages.push(format!("{}が成魚に育った", species_name(f.species)));
                }

                // 産卵: 成魚が満腹維持で確率的に卵を産む(空腹度は消費しない)
                if f.stage == Stage::Adult
                    && f.well_fed_timer >= BREED_READY_TIME
                    && self.rng.next_f64() < BREED_CHANCE_PER_SEC * dt
                {
                    spawn_eggs.push((f.x, f.y, f.species));
                    // 親は満腹タイマーを消費(連続産卵しない)
                    f.well_fed_timer = 0.0;
                }
            }
        }

        // 産卵イベントを卵に変換(2〜4個、水底付近に配置)
        for (px, _py, sp) in spawn_eggs {
            let n = self.rng.range_usize(2, 4);
            for _ in 0..n {
                let ex = (px + self.rng.range(-4.0, 4.0)).clamp(1.0, safe_upper(w - 1.0));
                let ey = (sand_top - self.rng.range(0.5, 2.5)).max(1.0);
                self.eggs.push(Egg {
                    x: ex,
                    y: ey,
                    species: sp,
                    hatch: EGG_HATCH_TIME,
                });
            }
            messages.push(format!("{}が卵を産んだ", species_name(sp)));
        }

        // 孵化: 時間経過した卵を稚魚にする。上限超過分は孵化しない(卵は消える)。
        let mut alive = self.fish.len();
        let mut newborns: Vec<Fish> = Vec::new();
        let mut hatched_msg = false;
        for e in &mut self.eggs {
            e.hatch -= dt;
        }
        self.eggs.retain(|e| {
            if e.hatch > 0.0 {
                return true; // まだ孵化しない
            }
            // 孵化タイミング
            if alive + newborns.len() < cap {
                newborns.push(Fish::new(e.species, Stage::Fry, e.x, e.y));
                hatched_msg = true;
            }
            false // 孵化 or 上限超過 → 卵は消える
        });
        alive += newborns.len();
        let _ = alive;
        self.fish.extend(newborns);
        if hatched_msg {
            messages.push("卵が孵化した".to_string());
        }

        // 死亡判定: 空腹度0の衰弱、または病気の長期放置
        let mut deaths: Vec<String> = Vec::new();
        // 死亡・弱りの判定に starve_timer を更新
        for f in &mut self.fish {
            if f.hunger <= 0.0 {
                f.starve_timer += dt;
            } else {
                f.starve_timer = 0.0;
            }
        }
        self.fish.retain(|f| {
            let starved = f.starve_timer >= STARVE_DEATH_TIME;
            let sick_dead = f.sick && f.sick_timer >= SICK_DEATH_TIME;
            if starved {
                deaths.push(format!("{}が空腹で力尽きた…", species_name(f.species)));
                false
            } else if sick_dead {
                deaths.push(format!("{}が病気で力尽きた…", species_name(f.species)));
                false
            } else {
                true
            }
        });

        // メッセージ優先度: 死亡 > 発症/成長/産卵/孵化 > 弱り
        if let Some(m) = deaths.into_iter().last() {
            self.set_message(m);
        } else if let Some(m) = messages.into_iter().last() {
            self.set_message(m);
        } else if let Some(f) = self.fish.iter().find(|f| {
            (f.starve_timer >= STARVE_WEAK_TIME) || (f.sick && f.sick_timer >= SICK_WEAK_TIME)
        }) {
            if self.message.is_none() {
                self.set_message(format!("{}が弱っている…", species_name(f.species)));
            }
        }
    }

    // 気泡: 定期発生して上へ移動
    fn update_bubbles(&mut self, dt: f64, w: f64, h: f64) {
        self.bubble_timer -= dt;
        if self.bubble_timer <= 0.0 {
            self.bubble_timer = self.rng.range(0.3, 0.9);
            self.bubbles.push(Bubble {
                x: self.rng.range(2.0, (w - 2.0).max(2.0)),
                y: h - 2.0,
                vy: -self.rng.range(6.0, 12.0),
            });
        }
        for b in &mut self.bubbles {
            b.y += b.vy * dt;
            b.x += (self.rng.signed() * 4.0) * dt;
        }
        self.bubbles.retain(|b| b.y > 1.0);
        if self.bubbles.len() > 60 {
            let drop = self.bubbles.len() - 60;
            self.bubbles.drain(0..drop);
        }
    }
}

pub fn species_name(sp: Species) -> &'static str {
    match sp {
        Species::Neon => "ネオン",
        Species::Goldfish => "金魚",
        Species::Guppy => "グッピー",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // dt刻みで t秒ぶん更新する。魚を毎ステップ満腹に保つか選べる。
    fn run(sim: &mut Simulation, t: f64, dt: f64, w: usize, h: usize, keep_fed: bool) {
        let steps = (t / dt).round() as usize;
        for _ in 0..steps {
            if keep_fed {
                for f in &mut sim.fish {
                    f.hunger = MAX_HUNGER;
                }
            }
            sim.update(dt, w, h);
        }
    }

    #[test]
    fn hunger_decays_over_time() {
        let mut sim = Simulation::new(Rng::new(1));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0));
        let before = sim.fish[0].hunger;
        run(&mut sim, 5.0, 0.1, 80, 40, false);
        let after = sim.fish[0].hunger;
        assert!(after < before, "空腹度は時間で減るはず: {before} -> {after}");
        assert!((before - after - HUNGER_DECAY * 5.0).abs() < 1.0);
    }

    #[test]
    fn feeding_restores_hunger() {
        let mut sim = Simulation::new(Rng::new(2));
        let mut fish = Fish::new(Species::Guppy, Stage::Adult, 40.0, 20.0);
        fish.hunger = 10.0;
        sim.fish.push(fish);
        sim.food.push(Food {
            x: 40.0,
            y: 20.0,
            vy: 0.0,
            life: 10.0,
        });
        sim.update(0.1, 80, 40);
        assert!(sim.fish[0].hunger > 10.0, "餌で空腹度が回復するはず");
        assert_eq!(sim.food_count(), 0, "食べられた餌は消えるはず");
    }

    #[test]
    fn well_fed_fry_grows_to_adult() {
        let mut sim = Simulation::new(Rng::new(3));
        sim.fish.push(Fish::new(Species::Neon, Stage::Fry, 20.0, 10.0));
        assert_eq!(sim.fish[0].stage, Stage::Fry);
        run(&mut sim, GROW_TIME + 2.0, 0.1, 80, 40, true);
        assert_eq!(sim.fish[0].stage, Stage::Adult, "満腹維持で成魚になるはず");
    }

    #[test]
    fn starving_fish_dies_and_is_removed() {
        let mut sim = Simulation::new(Rng::new(4));
        let mut fish = Fish::new(Species::Goldfish, Stage::Adult, 20.0, 10.0);
        fish.hunger = 0.0;
        sim.fish.push(fish);
        run(&mut sim, STARVE_DEATH_TIME + 3.0, 0.1, 80, 40, false);
        assert_eq!(sim.fish_count(), 0, "餓死した魚は水槽から消えるはず");
    }

    #[test]
    fn breeding_respects_capacity() {
        let (w, h) = (80, 40);
        let cap = capacity(w, h);
        let mut sim = Simulation::new(Rng::new(5));
        for i in 0..cap {
            let mut f = Fish::new(Species::Neon, Stage::Adult, 10.0 + i as f64, 10.0);
            f.well_fed_timer = BREED_READY_TIME + 5.0;
            sim.fish.push(f);
        }
        run(&mut sim, 40.0, 0.1, w, h, true);
        assert!(
            sim.fish_count() <= cap,
            "個体数は上限{}を超えないはず: {}",
            cap,
            sim.fish_count()
        );
    }

    #[test]
    fn well_fed_adult_lays_eggs() {
        // 上限に余裕のある大きな水槽で、満腹の成魚が産卵することを確認。
        // 産卵は確率的なので、満腹・満腹維持タイマーを保ったまま卵が出るまで回す。
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(7));
        sim.fish
            .push(Fish::new(Species::Guppy, Stage::Adult, 40.0, 30.0));
        let mut saw_egg = false;
        for _ in 0..2000 {
            let f = &mut sim.fish[0];
            f.hunger = MAX_HUNGER;
            f.well_fed_timer = BREED_READY_TIME + 5.0;
            sim.update(0.1, w, h);
            if !sim.eggs.is_empty() {
                saw_egg = true;
                break;
            }
        }
        assert!(saw_egg, "満腹の成魚は産卵するはず");
    }

    #[test]
    fn egg_hatches_into_fry_when_below_capacity() {
        // 卵は一定時間で孵化して稚魚になる(2段階繁殖の後半)
        let (w, h) = (200, 100);
        let mut sim = Simulation::new(Rng::new(11));
        sim.eggs.push(Egg {
            x: 40.0,
            y: 90.0,
            species: Species::Neon,
            hatch: 0.05,
        });
        let before = sim.fish_count();
        sim.update(0.1, w, h);
        assert_eq!(sim.fish_count(), before + 1, "卵は孵化して稚魚が1匹増えるはず");
        assert!(sim.eggs.is_empty(), "孵化した卵は消えるはず");
        assert_eq!(sim.fish[before].stage, Stage::Fry, "孵化直後は稚魚");
    }

    #[test]
    fn egg_does_not_hatch_at_capacity() {
        // 上限に達していると卵は孵化せず消える
        let (w, h) = (80, 40);
        let cap = capacity(w, h);
        let mut sim = Simulation::new(Rng::new(12));
        for i in 0..cap {
            sim.fish
                .push(Fish::new(Species::Neon, Stage::Adult, 10.0 + i as f64, 10.0));
        }
        sim.eggs.push(Egg {
            x: 40.0,
            y: 35.0,
            species: Species::Neon,
            hatch: 0.05,
        });
        sim.update(0.1, w, h);
        assert_eq!(sim.fish_count(), cap, "上限では孵化で増えない");
        assert!(sim.eggs.is_empty(), "孵化できない卵も消える");
    }

    #[test]
    fn disease_onset_and_medicine_cures() {
        // 病気の魚が薬に触れると治る
        let mut sim = Simulation::new(Rng::new(13));
        let mut f = Fish::new(Species::Goldfish, Stage::Adult, 40.0, 20.0);
        f.sick = true;
        f.sick_timer = 5.0;
        f.hunger = MAX_HUNGER;
        sim.fish.push(f);
        sim.medicine.push(Medicine {
            x: 40.0,
            y: 20.0,
            vy: 0.0,
            life: 10.0,
        });
        sim.update(0.1, 80, 40);
        assert!(!sim.fish[0].sick, "薬で病気が治るはず");
        assert_eq!(sim.medicine.len(), 0, "使われた薬は消えるはず");
    }

    #[test]
    fn medicine_harmless_to_healthy_fish() {
        let mut sim = Simulation::new(Rng::new(17));
        let mut f = Fish::new(Species::Guppy, Stage::Adult, 40.0, 20.0);
        f.hunger = MAX_HUNGER;
        sim.fish.push(f);
        sim.medicine.push(Medicine {
            x: 40.0,
            y: 20.0,
            vy: 0.0,
            life: 10.0,
        });
        sim.update(0.1, 80, 40);
        assert!(!sim.fish[0].sick, "健康な魚は病気にならない");
        // 健康な魚には反応しないので薬は残る(寿命内)
        assert_eq!(sim.medicine.len(), 1, "健康な魚には薬が消費されない");
    }

    #[test]
    fn hungry_fish_gets_sick_over_time() {
        // 腹ぺこ放置で発症すること(確率的だが十分な時間で発症する)
        let mut sim = Simulation::new(Rng::new(19));
        let mut f = Fish::new(Species::Neon, Stage::Adult, 40.0, 20.0);
        f.hunger = 0.0;
        sim.fish.push(f);
        let mut got_sick = false;
        for _ in 0..600 {
            // 空腹を維持(餓死判定より前に発症を観測したいので hunger を少し戻す)
            sim.fish[0].hunger = 5.0;
            sim.fish[0].starve_timer = 0.0;
            sim.update(0.1, 80, 40);
            if sim.fish[0].sick {
                got_sick = true;
                break;
            }
        }
        assert!(got_sick, "腹ぺこ長期放置で発症するはず");
    }

    #[test]
    fn pause_stops_simulation() {
        let mut sim = Simulation::new(Rng::new(23));
        sim.fish.push(Fish::new(Species::Neon, Stage::Adult, 20.0, 10.0));
        let before = sim.fish[0].hunger;
        // dt=0(一時停止相当)では何も進まない
        sim.update(0.0, 80, 40);
        assert_eq!(sim.fish[0].hunger, before, "一時停止中は空腹度が減らない");
        assert_eq!(sim.elapsed, 0.0, "一時停止中は経過時間が進まない");
    }

    #[test]
    fn reset_restores_initial_state() {
        let (w, h) = (80, 40);
        let mut sim = Simulation::new(Rng::new(29));
        sim.seed_initial(w, h);
        sim.feed(w);
        sim.medicate(w);
        run(&mut sim, 3.0, 0.1, w, h, false);
        sim.reset(w, h);
        assert_eq!(sim.food_count(), 0, "リセットで餌が消える");
        assert_eq!(sim.medicine.len(), 0, "リセットで薬が消える");
        assert_eq!(sim.eggs.len(), 0, "リセットで卵が消える");
        assert_eq!(sim.elapsed, 0.0, "リセットで経過時間が0に戻る");
        assert!(sim.fish_count() > 0, "リセット後は初期個体が存在する");
    }

    #[test]
    fn capacity_scales_with_size_and_is_bounded() {
        assert!(capacity(40, 20) >= 5);
        assert!(capacity(400, 200) <= 40);
        assert!(capacity(200, 100) >= capacity(80, 40));
    }

    // 回帰テスト: 疑似端末等で極端に小さい pix_w/pix_h が渡されても
    // `x.clamp(1.0, upper)` の upper < 1.0(min > max)で panic しないこと。
    // (実機で cell_rows=1 相当の疑似端末を起動して発見された panic の再発防止)
    #[test]
    fn update_does_not_panic_on_tiny_dimensions() {
        for h in 0..=3usize {
            for w in 0..=3usize {
                let mut sim = Simulation::new(Rng::new(42));
                sim.seed_initial(w, h);
                sim.feed(w);
                sim.medicate(w);
                for _ in 0..20 {
                    sim.update(0.1, w, h);
                }
            }
        }
    }

    #[test]
    fn safe_upper_never_returns_below_one() {
        assert_eq!(safe_upper(-5.0), 1.0);
        assert_eq!(safe_upper(0.0), 1.0);
        assert_eq!(safe_upper(0.5), 1.0);
        assert_eq!(safe_upper(f64::NAN), 1.0);
        assert_eq!(safe_upper(5.0), 5.0);
    }
}
