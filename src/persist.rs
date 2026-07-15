// 状態の永続化: ~/.config/aquaterm/state.json に保存/復元する。
// 魚(種類・成長段階・空腹度・座標・速度)、餌、経過時間を保存する。

use crate::fish::Fish;
use crate::sim::{Crab, Den, Egg, Food, Meat, Medicine, Plant, Purifier, Rock, Seahorse, Shrimp, Simulation, Star};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    // 気泡音に続く個別トグルの拡張(捕食系・投下系・状態通知系)。旧セーブには
    // キーが無いため、Ctl::new()と同じ既定のtrue(従来の常時鳴る挙動)にfallbackする。
    #[serde(default = "default_true")]
    pub predation_sfx_on: bool,
    #[serde(default = "default_true")]
    pub drop_sfx_on: bool,
    #[serde(default = "default_true")]
    pub health_sfx_on: bool,
    // 生み出す魚の種類のトグル(Species::COMMONと同じ並び順)。
    #[serde(default = "default_species_toggle")]
    pub species_toggle: [bool; 5],
    // 餌やりの投下量レベル(餌の量を設定できるようにしてほしいという要望への対応)。
    // 旧セーブにはキーが無いため、従来どおりの量(レベル1)にfallbackする。
    #[serde(default = "default_feed_amount")]
    pub feed_amount: usize,
    // 個体数上限のユーザー設定値(水槽内の個体数上限を設定で引き下げたいという要望への
    // 対応)。旧セーブにはキーが無いため、従来どおり動的MAX(画面サイズ由来のcapacity)を
    // そのまま使う「無制限」にfallbackする。
    #[serde(default = "default_max_fish_cap")]
    pub max_fish_cap: usize,
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

fn default_max_fish_cap() -> usize {
    crate::sim::MAX_FISH_CAP_UNLIMITED
}

fn default_speed_idx() -> usize {
    crate::SPEED_DEFAULT
}

// --- 複数水槽(名前付き保存、新規) ---------------------------------------
// 従来は ~/.config/aquaterm/state.json への固定パス単一セーブのみだった。
// 複数の水槽を名前付きで保存・呼び出せるようにするため、水槽ごとの保存先を
// ~/.config/aquaterm/tanks/<name>.json に分け、「いまどの水槽を開いているか」は
// ~/.config/aquaterm/current_tank.txt という小さな別ファイルに記録する
// (state.json自体に持たせる案もあったが、保存先そのものが水槽ごとに複数へ
// 分かれる構成にしたため、「今どれを開いているか」は水槽本体とは別に持つ方が
// シンプルになる)。

// 水槽名として使える文字数の上限(ファイル名が極端に長くならないようにするため)。
pub const TANK_NAME_MAX_CHARS: usize = 40;
// 初回移行・新規インストール時に使うフォールバック名。
const DEFAULT_TANK_NAME: &str = "default";

// 水槽名をファイルシステムで安全な文字だけに絞る。英数字(Unicodeの文字種を
// 含む。`char::is_alphanumeric()`はCJK等も対象になるため、日本語の水槽名も
// そのまま使える)と `-` ・ `_` 以外の文字(パス区切り `/` ・ `.` ・引用符・
// 空白・制御文字等)はすべて除去する。結果が空になる(全部除去された)場合は
// 空文字列を返す(呼び出し側で「無効な名前」として扱う)。
pub fn sanitize_tank_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(TANK_NAME_MAX_CHARS)
        .collect()
}

// ~/.config/aquaterm 本体(水槽ディレクトリ・現在水槽マーカー・旧固定パスの親)。
// 注意: `dirs::config_dir()` は macOS では `~/Library/Application Support` を返してしまい、
// 仕様書が明示する `~/.config/aquaterm` と一致しない(実機テストで発覚)。
// 姉妹プロジェクト termmap と同じく $HOME を直接使い、OSに依らず固定する。
fn config_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config").join("aquaterm"))
}

fn tanks_dir_in(base: &Path) -> PathBuf {
    base.join("tanks")
}

// 名前を受け取り、サニタイズ後のtanks/<name>.jsonパスを返す。サニタイズ後に
// 空文字列になる(=ファイル名として使える文字が無かった)場合はNone。
fn tank_path_in(base: &Path, name: &str) -> Option<PathBuf> {
    let safe = sanitize_tank_name(name);
    if safe.is_empty() {
        return None;
    }
    Some(tanks_dir_in(base).join(format!("{safe}.json")))
}

fn current_tank_marker_path_in(base: &Path) -> PathBuf {
    base.join("current_tank.txt")
}

// 旧バージョンの固定保存先(移行専用。複数水槽化より前の唯一のセーブ)。
fn legacy_state_path_in(base: &Path) -> PathBuf {
    base.join("state.json")
}

fn load_legacy_in(base: &Path) -> Option<SavedState> {
    let data = std::fs::read_to_string(legacy_state_path_in(base)).ok()?;
    serde_json::from_str(&data).ok()
}

fn write_saved_state_to(path: &Path, state: &SavedState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}

// Simulation/Ctlの現在値からSavedStateを組み立てる(保存先が複数(水槽ごと)に
// 分かれても、組み立てロジック自体は1箇所にまとめておく)。
fn build_saved_state(sim: &Simulation, ctl: &crate::Ctl) -> SavedState {
    SavedState {
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
        predation_sfx_on: ctl.predation_sfx_on,
        drop_sfx_on: ctl.drop_sfx_on,
        health_sfx_on: ctl.health_sfx_on,
        species_toggle: sim.species_toggle,
        feed_amount: sim.feed_amount,
        max_fish_cap: sim.max_fish_cap,
        pollution: sim.pollution,
        purifier_concentration: sim.purifier_concentration,
        crab_toggle: sim.crab_toggle,
        speed_idx: ctl.speed_idx,
    }
}

fn save_named_in(base: &Path, sim: &Simulation, ctl: &crate::Ctl, name: &str) -> std::io::Result<()> {
    let path = match tank_path_in(base, name) {
        Some(p) => p,
        None => return Ok(()), // サニタイズ後に空になる名前では書き込まない
    };
    write_saved_state_to(&path, &build_saved_state(sim, ctl))
}

fn load_named_in(base: &Path, name: &str) -> Option<SavedState> {
    let path = tank_path_in(base, name)?;
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

// 保存済みの水槽名の一覧(拡張子抜き・ソート済み)。ディレクトリが無ければ空。
fn list_tanks_in(base: &Path) -> Vec<String> {
    let entries = match std::fs::read_dir(tanks_dir_in(base)) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                path.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    names.sort();
    names
}

fn save_current_tank_name_in(base: &Path, name: &str) -> std::io::Result<()> {
    let path = current_tank_marker_path_in(base);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, name)
}

fn load_current_tank_name_in(base: &Path) -> Option<String> {
    let data = std::fs::read_to_string(current_tank_marker_path_in(base)).ok()?;
    let trimmed = data.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// 起動時にどの水槽を開くか解決する。
// 1. current_tank.txt に前回開いていた水槽名が記録されていればそれを最優先する。
// 2. 記録が無い(初回起動、または複数水槽化より前のバージョンからのアップグレード
//    直後)場合、旧固定パス(state.json)にセーブがあれば "default" という名前の
//    水槽として取り込む(tanks/default.jsonがまだ無いときだけ行う。既にあるものを
//    黙って上書きしない = ユーザーの既存の進行中データを黙って消さないための
//    一回限りの移行)。
// 3. どちらも無ければ新規インストールとして"default"を返す(SavedStateはNone。
//    呼び出し側でseed_initial等の初期化を行う)。
fn resolve_startup_tank_in(base: &Path) -> (String, Option<SavedState>) {
    if let Some(name) = load_current_tank_name_in(base) {
        let state = load_named_in(base, &name);
        return (name, state);
    }
    if load_named_in(base, DEFAULT_TANK_NAME).is_none() {
        if let Some(legacy) = load_legacy_in(base) {
            if let Some(path) = tank_path_in(base, DEFAULT_TANK_NAME) {
                let _ = write_saved_state_to(&path, &legacy);
            }
        }
    }
    (DEFAULT_TANK_NAME.to_string(), load_named_in(base, DEFAULT_TANK_NAME))
}

// --- ここから公開API(実際の ~/.config/aquaterm を使う) -------------------

// 保存済みの水槽名の一覧(拡張子抜き・ソート済み)。
pub fn list_tanks() -> Vec<String> {
    config_dir().map(|b| list_tanks_in(&b)).unwrap_or_default()
}

// 指定した名前で現在のSimulation/Ctlの状態を保存する。
pub fn save_named(sim: &Simulation, ctl: &crate::Ctl, name: &str) -> std::io::Result<()> {
    match config_dir() {
        Some(base) => save_named_in(&base, sim, ctl, name),
        None => Ok(()),
    }
}

// 指定した名前の水槽を読み込む。無ければ None。
pub fn load_named(name: &str) -> Option<SavedState> {
    load_named_in(&config_dir()?, name)
}

// 現在開いている水槽名を記録する(次回起動時に同じ水槽を開くため)。
pub fn save_current_tank_name(name: &str) -> std::io::Result<()> {
    match config_dir() {
        Some(base) => save_current_tank_name_in(&base, name),
        None => Ok(()),
    }
}

// 起動時にどの水槽を開くかを解決する(旧固定パスからの自動移行込み)。
pub fn resolve_startup_tank() -> (String, Option<SavedState>) {
    match config_dir() {
        Some(base) => resolve_startup_tank_in(&base),
        None => (DEFAULT_TANK_NAME.to_string(), None),
    }
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
    sim.max_fish_cap = state.max_fish_cap;
    sim.pollution = state.pollution;
    sim.purifier_concentration = state.purifier_concentration;
    sim.crab_toggle = state.crab_toggle;
    ctl.sfx_on = state.sfx_on;
    ctl.overlay_on = state.overlay_on;
    ctl.auto_on = state.auto_on;
    ctl.day_night_on = state.day_night_on;
    ctl.auto_replenish_on = state.auto_replenish_on;
    ctl.bubble_sfx_on = state.bubble_sfx_on;
    ctl.predation_sfx_on = state.predation_sfx_on;
    ctl.drop_sfx_on = state.drop_sfx_on;
    ctl.health_sfx_on = state.health_sfx_on;
    // SPEED_STEPSの要素数が将来変わっても範囲外インデックスでpanicしないよう頭打ちにする。
    ctl.speed_idx = state.speed_idx.min(crate::SPEED_STEPS.len() - 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Den, Plant};

    // --- 複数水槽機能のテスト用ヘルパー ------------------------------------
    // 実際の~/.config/aquatermを使う公開API(list_tanks/save_named等)は実ユーザーの
    // セーブを壊すリスクがあるため直接は呼ばない。代わりに、テストごとに独立した
    // 一時ディレクトリをbaseとして渡す `*_in` 版(内部実装)を使う。env var(HOME)の
    // 書き換えはプロセス全体に効いて並行実行中の他テストと競合するため使わない。
    fn temp_base_dir(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("aquaterm_test_{}_{tag}_{n}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    // 各テストで必要な項目だけ変えられるよう、最小限のSavedStateを返す。
    fn minimal_saved_state() -> SavedState {
        SavedState {
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
            sfx_on: true,
            overlay_on: true,
            auto_on: false,
            day_night_on: true,
            auto_replenish_on: false,
            bubble_sfx_on: true,
            predation_sfx_on: true,
            drop_sfx_on: true,
            health_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            max_fish_cap: crate::sim::MAX_FISH_CAP_UNLIMITED,
            pollution: 0.0,
            purifier_concentration: 0.0,
            crab_toggle: true,
            speed_idx: 2,
        }
    }

    // --- 水槽名のサニタイズ ------------------------------------------------
    #[test]
    fn sanitize_tank_name_keeps_alphanumeric_dash_and_underscore() {
        assert_eq!(sanitize_tank_name("reef-tank_01"), "reef-tank_01");
    }

    #[test]
    fn sanitize_tank_name_keeps_japanese_characters() {
        // 日本語の水槽名も使えるようにする(is_alphanumeric()はCJK等も対象)。
        assert_eq!(sanitize_tank_name("リビング水槽"), "リビング水槽");
    }

    #[test]
    fn sanitize_tank_name_strips_path_separators_and_dots() {
        // パス区切り・ドットを許すとtanks/配下から脱出できてしまうため必ず除去する。
        assert_eq!(sanitize_tank_name("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_tank_name("a.json"), "ajson");
    }

    #[test]
    fn sanitize_tank_name_strips_spaces_quotes_and_control_chars() {
        assert_eq!(sanitize_tank_name("my tank\"'\n\t"), "mytank");
    }

    #[test]
    fn sanitize_tank_name_of_only_invalid_chars_is_empty() {
        assert_eq!(sanitize_tank_name("...///   "), "");
        assert_eq!(sanitize_tank_name(""), "");
    }

    #[test]
    fn sanitize_tank_name_truncates_to_max_chars() {
        let long = "a".repeat(TANK_NAME_MAX_CHARS + 20);
        let safe = sanitize_tank_name(&long);
        assert_eq!(safe.chars().count(), TANK_NAME_MAX_CHARS);
    }

    // --- 複数水槽の保存/読み込みが独立していること --------------------------
    #[test]
    fn different_tank_names_store_independent_state() {
        let base = temp_base_dir("independent");

        let mut state_a = minimal_saved_state();
        state_a.elapsed = 111.0;
        state_a.speed_idx = 3;
        let mut state_b = minimal_saved_state();
        state_b.elapsed = 222.0;
        state_b.speed_idx = 5;

        let path_a = tank_path_in(&base, "tank-a").expect("有効な名前のはず");
        let path_b = tank_path_in(&base, "tank_b").expect("有効な名前のはず");
        write_saved_state_to(&path_a, &state_a).expect("書き込めるはず");
        write_saved_state_to(&path_b, &state_b).expect("書き込めるはず");

        let loaded_a = load_named_in(&base, "tank-a").expect("tank-aを読み込めるはず");
        let loaded_b = load_named_in(&base, "tank_b").expect("tank_bを読み込めるはず");
        assert_eq!(loaded_a.elapsed, 111.0, "tank-aの内容がtank_bに影響されていないはず");
        assert_eq!(loaded_a.speed_idx, 3);
        assert_eq!(loaded_b.elapsed, 222.0, "tank_bの内容がtank-aに影響されていないはず");
        assert_eq!(loaded_b.speed_idx, 5);

        let mut names = list_tanks_in(&base);
        names.sort();
        assert_eq!(names, vec!["tank-a".to_string(), "tank_b".to_string()]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn loading_a_tank_that_was_never_saved_returns_none() {
        let base = temp_base_dir("missing");
        assert!(load_named_in(&base, "no-such-tank").is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn tank_path_in_rejects_names_that_sanitize_to_empty() {
        assert!(tank_path_in(Path::new("/tmp/whatever"), "...").is_none());
    }

    // --- 現在の水槽名の永続化 ------------------------------------------------
    #[test]
    fn current_tank_name_round_trips_and_trims_whitespace() {
        let base = temp_base_dir("current-name");
        assert!(
            load_current_tank_name_in(&base).is_none(),
            "まだ記録が無ければNoneのはず"
        );
        save_current_tank_name_in(&base, "myTank\n").expect("書き込めるはず");
        assert_eq!(load_current_tank_name_in(&base).as_deref(), Some("myTank"));
        let _ = std::fs::remove_dir_all(&base);
    }

    // --- 既存の固定パスセーブからの移行 ---------------------------------------
    #[test]
    fn legacy_fixed_path_save_is_migrated_to_default_tank_on_first_resolve() {
        let base = temp_base_dir("migrate");
        let mut legacy = minimal_saved_state();
        legacy.elapsed = 999.0;
        write_saved_state_to(&legacy_state_path_in(&base), &legacy).expect("旧セーブを書き込めるはず");

        // current_tank.txtもtanks/default.jsonも無い状態から解決する。
        let (name, state) = resolve_startup_tank_in(&base);
        assert_eq!(name, "default");
        let state = state.expect("旧セーブがdefault水槽として読めるはず");
        assert_eq!(state.elapsed, 999.0, "旧セーブの内容を失わずに引き継ぐはず");

        // 移行によってtanks/default.jsonが実際に作られているはず(次回以降も残る)。
        assert!(tank_path_in(&base, "default").unwrap().exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_prefers_current_tank_marker_over_legacy_migration() {
        let base = temp_base_dir("prefer-marker");
        let mut legacy = minimal_saved_state();
        legacy.elapsed = 1.0;
        write_saved_state_to(&legacy_state_path_in(&base), &legacy).unwrap();

        let mut other = minimal_saved_state();
        other.elapsed = 42.0;
        write_saved_state_to(&tank_path_in(&base, "reef").unwrap(), &other).unwrap();
        save_current_tank_name_in(&base, "reef").unwrap();

        let (name, state) = resolve_startup_tank_in(&base);
        assert_eq!(name, "reef", "current_tank.txtの記録を最優先するはず");
        assert_eq!(state.unwrap().elapsed, 42.0);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_does_not_overwrite_existing_default_tank_with_legacy_save() {
        // tanks/default.jsonが既にある場合は、旧固定パスの内容で上書きしない
        // (ユーザーの進行中データを黙って消さないため)。
        let base = temp_base_dir("no-overwrite");
        let mut existing_default = minimal_saved_state();
        existing_default.elapsed = 7.0;
        write_saved_state_to(&tank_path_in(&base, "default").unwrap(), &existing_default).unwrap();

        let mut legacy = minimal_saved_state();
        legacy.elapsed = 9999.0;
        write_saved_state_to(&legacy_state_path_in(&base), &legacy).unwrap();

        let (name, state) = resolve_startup_tank_in(&base);
        assert_eq!(name, "default");
        assert_eq!(
            state.unwrap().elapsed,
            7.0,
            "既存のdefault水槽を優先し、旧セーブで上書きしないはず"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_with_no_legacy_save_and_no_marker_falls_back_to_fresh_default() {
        let base = temp_base_dir("fresh-install");
        let (name, state) = resolve_startup_tank_in(&base);
        assert_eq!(name, "default");
        assert!(state.is_none(), "何もセーブが無ければNone(呼び出し側で初期化)のはず");
        let _ = std::fs::remove_dir_all(&base);
    }

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
            predation_sfx_on: true,
            drop_sfx_on: true,
            health_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            max_fish_cap: crate::sim::MAX_FISH_CAP_UNLIMITED,
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
        assert!(restored.predation_sfx_on, "旧セーブではpredation_sfx_onは既定のtrueにfallbackするはず");
        assert!(restored.drop_sfx_on, "旧セーブではdrop_sfx_onは既定のtrueにfallbackするはず");
        assert!(restored.health_sfx_on, "旧セーブではhealth_sfx_onは既定のtrueにfallbackするはず");
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
            restored.max_fish_cap,
            crate::sim::MAX_FISH_CAP_UNLIMITED,
            "旧セーブではmax_fish_capは無制限(動的MAXそのまま)にfallbackするはず"
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
            predation_sfx_on: true,
            drop_sfx_on: false,
            health_sfx_on: true,
            species_toggle: [true, false, true, false, true],
            feed_amount: 3,
            max_fish_cap: 12,
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
        assert!(restored.predation_sfx_on);
        assert!(!restored.drop_sfx_on);
        assert!(restored.health_sfx_on);
        assert_eq!(restored.species_toggle, [true, false, true, false, true]);
        assert_eq!(restored.feed_amount, 3);
        assert_eq!(restored.max_fish_cap, 12, "max_fish_capも往復するはず");
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
            predation_sfx_on: true,
            drop_sfx_on: true,
            health_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            max_fish_cap: crate::sim::MAX_FISH_CAP_UNLIMITED,
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
            predation_sfx_on: true,
            drop_sfx_on: true,
            health_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            max_fish_cap: crate::sim::MAX_FISH_CAP_UNLIMITED,
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
            predation_sfx_on: true,
            drop_sfx_on: true,
            health_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            max_fish_cap: crate::sim::MAX_FISH_CAP_UNLIMITED,
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
            predation_sfx_on: true,
            drop_sfx_on: true,
            health_sfx_on: true,
            species_toggle: [true; 5],
            feed_amount: 1,
            max_fish_cap: crate::sim::MAX_FISH_CAP_UNLIMITED,
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
