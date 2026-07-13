// 状態の永続化: ~/.config/aquaterm/state.json に保存/復元する。
// 魚(種類・成長段階・空腹度・座標・速度)、餌、経過時間を保存する。

use crate::fish::Fish;
use crate::sim::{Egg, Food, Medicine, Simulation};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
pub struct SavedState {
    pub fish: Vec<Fish>,
    pub food: Vec<Food>,
    #[serde(default)]
    pub medicine: Vec<Medicine>,
    #[serde(default)]
    pub eggs: Vec<Egg>,
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
        eggs: sim.eggs.clone(),
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
    sim.eggs = state.eggs;
    sim.elapsed = state.elapsed;
}
