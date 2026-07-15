// 状態の永続化: ~/.config/aquaterm/state.json に保存/復元する。
// 魚(種類・成長段階・空腹度・座標・速度)、餌、経過時間を保存する。

use crate::fish::Fish;
use crate::sim::{Crab, Den, Egg, Food, Meat, Medicine, Plant, Purifier, Rock, Seahorse, Shrimp, Simulation, Star};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
pub struct SavedState {
    pub fish: Vec<Fish>,
    pub food: Vec<Food>,
    #[serde(default)]
    pub medicine: Vec<Medicine>,
    // ピラニア専用の肉餌。旧セーブには存在しないため #[serde(default)] で空扱いにする。
    #[serde(default)]
    pub meat: Vec<Meat>,
    // 浄化剤(沈下中の個体)。旧セーブには存在しないため #[serde(default)] で空扱いにする。
    #[serde(default)]
    pub purifiers: Vec<Purifier>,
    #[serde(default)]
    pub eggs: Vec<Egg>,
    // スター(無敵アイテム)。旧セーブには存在しないため #[serde(default)] で空扱いにする。
    #[serde(default)]
    pub stars: Vec<Star>,
    // 観賞用エンティティ(カニ)。旧セーブには存在しないため #[serde(default)] で空扱いにし、
    // main.rs 側で ensure_decorative_entities() により補充する。
    // (旧仕様の大型魚=BigFishは方針転換で廃止。旧セーブに残る "big_fish" キーは
    // serde が未知フィールドとして無視するだけで安全)
    #[serde(default)]
    pub crabs: Vec<Crab>,
    // エビ・タツノオトシゴ(カニと同じ位置づけの観賞用背景生物)。旧セーブには存在
    // しないため #[serde(default)] で空扱いにし、main.rs 側で ensure_decorative_entities()
    // により補充する。
    #[serde(default)]
    pub shrimp: Vec<Shrimp>,
    #[serde(default)]
    pub seahorses: Vec<Seahorse>,
    // 藻・水草・タコつぼ。旧セーブには存在しないため #[serde(default)] で空扱いにし、
    // main.rs 側で ensure_decorative_entities() により補充する。タコの巣(den_x/den_y)は
    // Fish側で保存されるため、タコつぼの位置もここで保存してズレないようにする
    // (保存しないと再起動時に新しい位置へ再抽選され、隠れているタコの位置と食い違う)。
    #[serde(default)]
    pub plants: Vec<Plant>,
    // 岩(隠れ場所)。旧セーブには存在しないため #[serde(default)] で空扱いにし、
    // main.rs 側で ensure_decorative_entities() により補充する。
    #[serde(default)]
    pub rocks: Vec<Rock>,
    #[serde(default)]
    pub dens: Vec<Den>,
    pub elapsed: f64,
    // 設定値を次回起動時に覚えておいてほしいという要望への対応で追加。
    // 旧セーブにはキーが無いため、Ctl::new()と同じ既定値にfallbackする
    // (真偽値の既定trueはserdeの標準defaultでは作れないため専用関数を使う)。
    #[serde(default = "default_true")]
    pub sfx_on: bool,
    #[serde(default = "default_true")]
    pub overlay_on: bool,
    #[serde(default)]
    pub auto_on: bool,
    #[serde(default = "default_true")]
    pub day_night_on: bool,
    #[serde(default)]
    pub auto_replenish_on: bool,
    #[serde(default = "default_true")]
    pub bubble_sfx_on: bool,
    // 生み出す魚の種類のトグル(Species::COMMONと同じ並び順)。
    #[serde(default = "default_species_toggle")]
    pub species_toggle: [bool; 5],
    // 餌やりの投下量レベル(餌の量を設定できるようにしてほしいという要望への対応)。
    // 旧セーブにはキーが無いため、従来どおりの量(レベル1)にfallbackする。
    #[serde(default = "default_feed_amount")]
    pub feed_amount: usize,
    // 水質(0=綺麗〜POLLUTION_MAX=最悪)。旧セーブにはキーが無いため、綺麗な状態(0.0)
    // にfallbackする(#[serde(default)]でf64の標準デフォルト0.0がそのまま使える)。
    #[serde(default)]
    pub pollution: f64,
    // 浄化剤の濃度。旧セーブにはキーが無いため、効果なし(0.0)にfallbackする。
    #[serde(default)]
    pub purifier_concentration: f64,
    // カニの表示ON/OFF。旧セーブにはキーが無いため、既定のtrue(表示)にfallbackする。
    #[serde(default = "default_true")]
    pub crab_toggle: bool,
    // シミュレーション速度([/]キーで変更するSPEED_STEPSのインデックス)。旧セーブには
    // キーが無いため、Ctl::new()と同じ既定速度(等倍)にfallbackする。
    #[serde(default = "default_speed_idx")]
    pub speed_idx: usize,
}

fn default_true() -> bool {
    true
}

fn default_species_toggle() -> [bool; 5] {
    [true; 5]
}

fn default_feed_amount() -> usize {
    crate::sim::FEED_AMOUNT_DEFAULT
}

fn default_speed_idx() -> usize {
    crate::SPEED_DEFAULT
}

// 保存先パス ~/.config/aquaterm/state.json
// 注意: `dirs::config_dir()` は macOS では `~/Library/Application Support` を返してしまい、
// 仕様書が明示する `~/.config/aquaterm/state.json` と一致しない(実機テストで発覚)。
// 姉妹プロジェクト termmap と同じく $HOME を直接使い、OSに依らず `~/.config/aquaterm` に固定する。
pub fn state_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config").join("aquaterm").join("state.json"))
}

// セーブファイルを読み込む。無ければ None。
pub fn load() -> Option<SavedState> {
    let path = state_path()?;
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

// 現在の状態を保存する。
pub fn save(sim: &Simulation, ctl: &crate::Ctl) -> std::io::Result<()> {
    let path = match state_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let state = SavedState {
        fish: sim.fish.clone(),
        food: sim.food.clone(),
        medicine: sim.medicine.clone(),
        meat: sim.meat.clone(),
        purifiers: sim.purifiers.clone(),
        eggs: sim.eggs.clone(),
        stars: sim.stars.clone(),
        crabs: sim.crabs.clone(),
        shrimp: sim.shrimp.clone(),
        seahorses: sim.seahorses.clone(),
        plants: sim.plants.clone(),
        rocks: sim.rocks.clone(),
        dens: sim.dens.clone(),
        elapsed: sim.elapsed,
        sfx_on: ctl.sfx_on,
        overlay_on: ctl.overlay_on,
        auto_on: ctl.auto_on,
        day_night_on: ctl.day_night_on,
        auto_replenish_on: ctl.auto_replenish_on,
        bubble_sfx_on: ctl.bubble_sfx_on,
        species_toggle: sim.species_toggle,
        feed_amount: sim.feed_amount,
        pollution: sim.pollution,
        purifier_concentration: sim.purifier_concentration,
        crab_toggle: sim.crab_toggle,
        speed_idx: ctl.speed_idx,
    };
    let json = serde_json::to_string_pretty(&state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&path, json)
}

// セーブ内容を Simulation / Ctl に流し込む。
pub fn restore_into(sim: &mut Simulation, ctl: &mut crate::Ctl, state: SavedState) {
    sim.fish = state.fish;
    sim.food = state.food;
    sim.medicine = state.medicine;
    sim.meat = state.meat;
    sim.purifiers = state.purifiers;
    sim.eggs = state.eggs;
    sim.stars = state.stars;
    sim.crabs = state.crabs;
    sim.shrimp = state.shrimp;
    sim.seahorses = state.seahorses;
    sim.plants = state.plants;
    sim.rocks = state.rocks;
    sim.dens = state.dens;
    sim.elapsed = state.elapsed;
    sim.species_toggle = state.species_toggle;
    sim.feed_amount = state.feed_amount;
    sim.pollution = state.pollution;
    sim.purifier_concentration = state.purifier_concentration;
    sim.crab_toggle = state.crab_toggle;
    ctl.sfx_on = state.sfx_on;
    ctl.overlay_on = state.overlay_on;
    ctl.auto_on = state.auto_on;
    ctl.day_night_on = state.day_night_on;
    ctl.auto_replenish_on = state.auto_replenish_on;
    ctl.bubble_sfx_on = state.bubble_sfx_on;
    // SPEED_STEPSの要素数が将来変わっても範囲外インデックスでpanicしないよう頭打ちにする。
    ctl.speed_idx = state.speed_idx.min(crate::SPEED_STEPS.len() - 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Den, Plant};

    // 実ファイルへの読み書きはユーザーの実セーブを壊すリスクがあるため、ここでは
    // シリアライズ/デシリアライズの往復のみを検証する(state_path()経由のI/Oはしない)。
    #[test]
    fn plants_and_dens_survive_a_serialize_round_trip() {
        // 藻・水草・タコつぼも保存対象であることの回帰テスト。保存しないと再起動時に
        // 位置が再抽選され、隠れているタコのden_x/den_y(Fish側)と食い違ってしまう。
        let state = SavedState {
            fish: Vec::new(),
            food: Vec::new(),
            medicine: Vec::new(),
            meat: Vec::new(),
            purifiers: Vec::new(),
            eggs: Vec::new(),
            stars: Vec::new(),
            crabs: Vec::new(),
            shrimp: Vec::new(),
            seahorses: Vec::new(),
            plants: vec![Plant {
                x: 12.5,
                y: 30.0,
                height: 5.0,
                phase: 1.2,
            }],
            rocks: Vec::new(),
            dens: vec![Den { x: 40.0, y: 31.0 }],
            elapsed: 123.0,
            sfx_on: true,
            overlay_on: true,
            auto_on: false,
            day_night_on: true,
            auto_replenish_on: false,
            bubble_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            pollution: 0.0,
            purifier_concentration: 0.0,
            crab_toggle: true,
            speed_idx: 2,
        };
        let json = serde_json::to_string(&state).expect("シリアライズできるはず");
        let restored: SavedState = serde_json::from_str(&json).expect("デシリアライズできるはず");

        assert_eq!(restored.plants.len(), 1);
        assert_eq!(restored.plants[0].x, 12.5);
        assert_eq!(restored.plants[0].y, 30.0);
        assert_eq!(restored.dens.len(), 1);
        assert_eq!(restored.dens[0].x, 40.0);
        assert_eq!(restored.dens[0].y, 31.0);
    }

    #[test]
    fn old_save_without_plants_or_dens_still_deserializes() {
        // 旧セーブ(plants/densキーが無い)でも #[serde(default)] で空扱いになり、
        // 読み込み自体が失敗しないことを確認する(main.rs側でensure_decorative_entities()
        // により補充される前提)。
        let old_json = r#"{"fish":[],"food":[],"elapsed":0.0}"#;
        let restored: SavedState = serde_json::from_str(old_json).expect("旧セーブも読めるはず");
        assert!(restored.plants.is_empty());
        assert!(restored.rocks.is_empty(), "旧セーブにrocksが無くても空扱いで読めるはず");
        assert!(restored.dens.is_empty());
        assert!(restored.stars.is_empty(), "旧セーブにstarsが無くても空扱いで読めるはず");
        assert!(restored.shrimp.is_empty(), "旧セーブにshrimpが無くても空扱いで読めるはず");
        assert!(restored.seahorses.is_empty(), "旧セーブにseahorsesが無くても空扱いで読めるはず");
        assert!(restored.purifiers.is_empty(), "旧セーブにpurifiersが無くても空扱いで読めるはず");
        assert_eq!(
            restored.purifier_concentration, 0.0,
            "旧セーブではpurifier_concentrationは効果なし(0.0)にfallbackするはず"
        );
        // 旧セーブに設定トグルのキーが無い場合、Ctl::new()と同じ既定値にfallbackするはず
        // (設定値を次回起動時に覚えておいてほしいという要望への対応で追加した
        // フィールド。旧セーブとの互換性を保つための回帰テスト)。
        assert!(restored.sfx_on, "旧セーブではsfx_onは既定のtrueにfallbackするはず");
        assert!(restored.overlay_on, "旧セーブではoverlay_onは既定のtrueにfallbackするはず");
        assert!(!restored.auto_on, "旧セーブではauto_onは既定のfalseにfallbackするはず");
        assert!(restored.day_night_on, "旧セーブではday_night_onは既定のtrueにfallbackするはず");
        assert!(!restored.auto_replenish_on, "旧セーブではauto_replenish_onは既定のfalseにfallbackするはず");
        assert!(restored.bubble_sfx_on, "旧セーブではbubble_sfx_onは既定のtrueにfallbackするはず");
        assert_eq!(
            restored.species_toggle,
            [true; 5],
            "旧セーブではspecies_toggleは全種ONにfallbackするはず"
        );
        assert_eq!(
            restored.feed_amount,
            crate::sim::FEED_AMOUNT_DEFAULT,
            "旧セーブではfeed_amountは従来どおりの量(デフォルト)にfallbackするはず"
        );
        assert_eq!(
            restored.pollution, 0.0,
            "旧セーブではpollutionは綺麗な状態(0.0)にfallbackするはず"
        );
        assert!(
            restored.crab_toggle,
            "旧セーブではcrab_toggleは既定のtrue(表示)にfallbackするはず"
        );
        assert_eq!(
            restored.speed_idx,
            crate::SPEED_DEFAULT,
            "旧セーブではspeed_idxは既定速度(等倍)にfallbackするはず"
        );
    }

    #[test]
    fn settings_toggles_survive_a_serialize_round_trip() {
        // 設定トグル(設定値を次回起動時に覚えておいてほしいという要望)が
        // 保存/復元できることの回帰テスト。既定値とは異なる値を使い、単純な
        // 既定値表示ではなく実際に値が往復していることを確認する。
        let state = SavedState {
            fish: Vec::new(),
            food: Vec::new(),
            medicine: Vec::new(),
            meat: Vec::new(),
            purifiers: Vec::new(),
            eggs: Vec::new(),
            stars: Vec::new(),
            crabs: Vec::new(),
            shrimp: Vec::new(),
            seahorses: Vec::new(),
            plants: Vec::new(),
            rocks: Vec::new(),
            dens: Vec::new(),
            elapsed: 0.0,
            sfx_on: false,
            overlay_on: false,
            auto_on: true,
            day_night_on: false,
            auto_replenish_on: true,
            bubble_sfx_on: false,
            species_toggle: [true, false, true, false, true],
            feed_amount: 3,
            pollution: 42.5,
            purifier_concentration: 0.0,
            crab_toggle: false,
            speed_idx: 4,
        };
        let json = serde_json::to_string(&state).expect("シリアライズできるはず");
        let restored: SavedState = serde_json::from_str(&json).expect("デシリアライズできるはず");

        assert!(!restored.sfx_on);
        assert!(!restored.overlay_on);
        assert!(restored.auto_on);
        assert!(!restored.day_night_on);
        assert!(restored.auto_replenish_on);
        assert!(!restored.bubble_sfx_on);
        assert_eq!(restored.species_toggle, [true, false, true, false, true]);
        assert_eq!(restored.feed_amount, 3);
        assert_eq!(restored.pollution, 42.5);
        assert!(!restored.crab_toggle);
        assert_eq!(restored.speed_idx, 4);
    }

    #[test]
    fn shrimp_and_seahorses_survive_a_serialize_round_trip() {
        // エビ・タツノオトシゴ(観賞用背景生物)も保存対象であることの回帰テスト。
        use crate::sim::{Seahorse, Shrimp};
        let state = SavedState {
            fish: Vec::new(),
            food: Vec::new(),
            medicine: Vec::new(),
            meat: Vec::new(),
            purifiers: Vec::new(),
            eggs: Vec::new(),
            stars: Vec::new(),
            crabs: Vec::new(),
            shrimp: vec![Shrimp {
                x: 11.0,
                dir: 1.0,
                pause_timer: 0.0,
                facing_right: true,
            }],
            seahorses: vec![Seahorse {
                anchor_x: 5.0,
                anchor_y: 6.0,
                x: 5.5,
                y: 6.2,
                phase: 0.4,
            }],
            plants: Vec::new(),
            rocks: Vec::new(),
            dens: Vec::new(),
            elapsed: 3.0,
            sfx_on: true,
            overlay_on: true,
            auto_on: false,
            day_night_on: true,
            auto_replenish_on: false,
            bubble_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            pollution: 0.0,
            purifier_concentration: 0.0,
            crab_toggle: true,
            speed_idx: 2,
        };
        let json = serde_json::to_string(&state).expect("シリアライズできるはず");
        let restored: SavedState = serde_json::from_str(&json).expect("デシリアライズできるはず");
        assert_eq!(restored.shrimp.len(), 1);
        assert_eq!(restored.shrimp[0].x, 11.0);
        assert_eq!(restored.seahorses.len(), 1);
        assert_eq!(restored.seahorses[0].anchor_x, 5.0);
    }

    #[test]
    fn rocks_survive_a_serialize_round_trip() {
        // 岩(隠れ場所)も保存対象であることの回帰テスト。保存しないと再起動のたびに
        // 位置が再抽選されてしまう。
        use crate::sim::Rock;
        let state = SavedState {
            fish: Vec::new(),
            food: Vec::new(),
            medicine: Vec::new(),
            meat: Vec::new(),
            purifiers: Vec::new(),
            eggs: Vec::new(),
            stars: Vec::new(),
            crabs: Vec::new(),
            shrimp: Vec::new(),
            seahorses: Vec::new(),
            plants: Vec::new(),
            rocks: vec![Rock { x: 22.0, y: 33.0 }],
            dens: Vec::new(),
            elapsed: 9.0,
            sfx_on: true,
            overlay_on: true,
            auto_on: false,
            day_night_on: true,
            auto_replenish_on: false,
            bubble_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            pollution: 0.0,
            purifier_concentration: 0.0,
            crab_toggle: true,
            speed_idx: 2,
        };
        let json = serde_json::to_string(&state).expect("シリアライズできるはず");
        let restored: SavedState = serde_json::from_str(&json).expect("デシリアライズできるはず");
        assert_eq!(restored.rocks.len(), 1);
        assert_eq!(restored.rocks[0].x, 22.0);
        assert_eq!(restored.rocks[0].y, 33.0);
    }

    #[test]
    fn stars_survive_a_serialize_round_trip() {
        // スター(無敵アイテム)も保存対象であることの回帰テスト。保存しないと
        // 再起動のたびに出現中のスターが消えてしまう。
        use crate::sim::Star;
        let state = SavedState {
            fish: Vec::new(),
            food: Vec::new(),
            medicine: Vec::new(),
            meat: Vec::new(),
            purifiers: Vec::new(),
            eggs: Vec::new(),
            stars: vec![Star {
                x: 15.0,
                y: 8.0,
                life: 20.0,
                phase: 0.7,
            }],
            crabs: Vec::new(),
            shrimp: Vec::new(),
            seahorses: Vec::new(),
            plants: Vec::new(),
            rocks: Vec::new(),
            dens: Vec::new(),
            elapsed: 5.0,
            sfx_on: true,
            overlay_on: true,
            auto_on: false,
            day_night_on: true,
            auto_replenish_on: false,
            bubble_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            pollution: 0.0,
            purifier_concentration: 0.0,
            crab_toggle: true,
            speed_idx: 2,
        };
        let json = serde_json::to_string(&state).expect("シリアライズできるはず");
        let restored: SavedState = serde_json::from_str(&json).expect("デシリアライズできるはず");
        assert_eq!(restored.stars.len(), 1);
        assert_eq!(restored.stars[0].x, 15.0);
        assert_eq!(restored.stars[0].y, 8.0);
    }

    #[test]
    fn purifiers_and_concentration_survive_a_serialize_round_trip() {
        // 浄化剤(沈下中の個体)と浄化剤の濃度も保存対象であることの回帰テスト。
        // 保存しないと、効果継続中や沈下途中の状態が再起動でリセットされてしまう。
        let state = SavedState {
            fish: Vec::new(),
            food: Vec::new(),
            medicine: Vec::new(),
            meat: Vec::new(),
            purifiers: vec![Purifier {
                x: 18.0,
                y: 3.0,
                vy: 5.0,
                sway_phase: 0.5,
            }],
            eggs: Vec::new(),
            stars: Vec::new(),
            crabs: Vec::new(),
            shrimp: Vec::new(),
            seahorses: Vec::new(),
            plants: Vec::new(),
            rocks: Vec::new(),
            dens: Vec::new(),
            elapsed: 7.0,
            sfx_on: true,
            overlay_on: true,
            auto_on: false,
            day_night_on: true,
            auto_replenish_on: false,
            bubble_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            pollution: 0.0,
            purifier_concentration: 0.6,
            crab_toggle: true,
            speed_idx: 2,
        };
        let json = serde_json::to_string(&state).expect("シリアライズできるはず");
        let restored: SavedState = serde_json::from_str(&json).expect("デシリアライズできるはず");
        assert_eq!(restored.purifiers.len(), 1);
        assert_eq!(restored.purifiers[0].x, 18.0);
        assert_eq!(restored.purifier_concentration, 0.6);
    }
}
