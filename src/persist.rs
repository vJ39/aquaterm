// 状態の永続化: ~/.config/aquaterm/state.json に保存/復元する。
// 魚(種類・成長段階・空腹度・座標・速度)、餌、経過時間を保存する。

use crate::fish::Fish;
use crate::sim::{Crab, Den, Egg, Food, Meat, Medicine, Plant, Rock, Seahorse, Shrimp, Simulation, Star};
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
pub fn save(sim: &Simulation) -> std::io::Result<()> {
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
        eggs: sim.eggs.clone(),
        stars: sim.stars.clone(),
        crabs: sim.crabs.clone(),
        shrimp: sim.shrimp.clone(),
        seahorses: sim.seahorses.clone(),
        plants: sim.plants.clone(),
        rocks: sim.rocks.clone(),
        dens: sim.dens.clone(),
        elapsed: sim.elapsed,
    };
    let json = serde_json::to_string_pretty(&state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&path, json)
}

// セーブ内容を Simulation に流し込む。
pub fn restore_into(sim: &mut Simulation, state: SavedState) {
    sim.fish = state.fish;
    sim.food = state.food;
    sim.medicine = state.medicine;
    sim.meat = state.meat;
    sim.eggs = state.eggs;
    sim.stars = state.stars;
    sim.crabs = state.crabs;
    sim.shrimp = state.shrimp;
    sim.seahorses = state.seahorses;
    sim.plants = state.plants;
    sim.rocks = state.rocks;
    sim.dens = state.dens;
    sim.elapsed = state.elapsed;
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
            eggs: Vec::new(),
            stars: Vec::new(),
            crabs: Vec::new(),
            shrimp: Vec::new(),
            seahorses: Vec::new(),
            plants: Vec::new(),
            rocks: vec![Rock { x: 22.0, y: 33.0 }],
            dens: Vec::new(),
            elapsed: 9.0,
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
        };
        let json = serde_json::to_string(&state).expect("シリアライズできるはず");
        let restored: SavedState = serde_json::from_str(&json).expect("デシリアライズできるはず");
        assert_eq!(restored.stars.len(), 1);
        assert_eq!(restored.stars[0].x, 15.0);
        assert_eq!(restored.stars[0].y, 8.0);
    }
}
