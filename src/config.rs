// 起動時設定ファイル: ~/.config/aquaterm/config.toml
//
// 例:
//   [http]
//   enabled = true
//   port = 7887
//
//   [general]
//   language = "en"
//
// スコープはHTTP API・表示言語関連のみ(配色等の他設定は今回は対象外)。設定画面
// (`,`キー)のstate.jsonベースのトグルとは別物: HTTPサーバーの有効/無効・ポート、
// 表示言語はいずれも実行中に切り替えられる性質のものではなく起動時にしか決められない
// ため、実行時保存のstate.json方式ではなく起動時読み込みのconfig.tomlを使う。
//
// ファイルが無い場合・壊れている(TOMLとして読めない)場合は、既定値
// (HTTP無効・言語は環境変数LANGから自動判定、判定できなければ日本語)に
// フォールバックしてTUI自体は通常どおり起動する(load/resolve参照。表示言語の
// 決定自体はi18n::detect()が担い、ここではconfig.toml側の値をSome/Noneで
// 受け渡すだけにする)。

use serde::Deserialize;
use std::path::PathBuf;

pub const DEFAULT_HTTP_PORT: u16 = 7887;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    pub enabled: bool,
    pub port: u16,
}

impl Default for HttpConfig {
    fn default() -> Self {
        HttpConfig {
            enabled: false,
            port: DEFAULT_HTTP_PORT,
        }
    }
}

// 表示言語(日英切り替え)設定。languageは"ja"/"en"を想定する文字列だが、
// 実際の解釈(未指定・不明値のフォールバック含む)はi18n::detect()に任せ、
// ここではTOMLの値をそのまま(あるいは無指定ならNoneとして)保持するだけにする。
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub http: HttpConfig,
    pub general: GeneralConfig,
}

// config.tomlの読み込み結果。persist::LoadOutcomeと同じ考え方で、
// 「無い」と「壊れている」を呼び出し側が区別できるようにしておく
// (壊れている場合だけユーザーに一言伝えたい、といった使い方ができる)。
pub enum ConfigOutcome {
    Loaded(Config),
    Missing,
    Corrupted,
}

// 注意: persist.rsのconfig_dir()と同じ理由で`dirs::config_dir()`は使わない
// (macOSで`~/Library/Application Support`を返してしまい、仕様書が明示する
// `~/.config/aquaterm`と食い違うため)。$HOMEを直接使う。
fn config_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config").join("aquaterm"))
}

fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

// 文字列からのパース本体。ファイルI/Oを含まないため単体テストしやすいよう
// 分離してある。
pub fn parse(src: &str) -> Result<Config, toml::de::Error> {
    toml::from_str(src)
}

// 実際の~/.config/aquaterm/config.tomlを読み込む。
pub fn load() -> ConfigOutcome {
    let Some(path) = config_path() else {
        return ConfigOutcome::Missing;
    };
    match std::fs::read_to_string(&path) {
        Ok(src) => match parse(&src) {
            Ok(cfg) => ConfigOutcome::Loaded(cfg),
            Err(_) => ConfigOutcome::Corrupted,
        },
        // 読めない理由(そもそも存在しない/権限が無い等)を区別する必要はない。
        // どちらも「無いものとして既定値で起動する」扱いで十分。
        Err(_) => ConfigOutcome::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_http_config_is_disabled_with_standard_port() {
        let cfg = Config::default();
        assert!(!cfg.http.enabled);
        assert_eq!(cfg.http.port, DEFAULT_HTTP_PORT);
    }

    #[test]
    fn parse_empty_string_falls_back_to_defaults() {
        let cfg = parse("").expect("空文字列は既定値で解釈できるはず");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn parse_reads_http_section() {
        let cfg = parse(
            r#"
            [http]
            enabled = true
            port = 12345
            "#,
        )
        .expect("正しいTOMLはパースできるはず");
        assert!(cfg.http.enabled);
        assert_eq!(cfg.http.port, 12345);
    }

    #[test]
    fn parse_partial_http_section_keeps_missing_fields_default() {
        // enabledだけ書いてportは省略 -> 既定ポートのままになる
        let cfg = parse("[http]\nenabled = true\n").expect("部分指定もパースできるはず");
        assert!(cfg.http.enabled);
        assert_eq!(cfg.http.port, DEFAULT_HTTP_PORT);
    }

    #[test]
    fn parse_rejects_malformed_toml() {
        assert!(parse("this is not [ valid toml").is_err());
    }

    #[test]
    fn default_general_config_has_no_language_override() {
        let cfg = Config::default();
        assert_eq!(cfg.general.language, None);
    }

    #[test]
    fn parse_reads_general_language_section() {
        let cfg = parse("[general]\nlanguage = \"en\"\n").expect("正しいTOMLはパースできるはず");
        assert_eq!(cfg.general.language.as_deref(), Some("en"));
    }

    #[test]
    fn parse_without_general_section_leaves_language_none() {
        let cfg = parse("[http]\nenabled = true\n").expect("部分指定もパースできるはず");
        assert_eq!(cfg.general.language, None);
    }

    #[test]
    fn parse_ignores_unrelated_sections_and_keys() {
        // 将来他の設定セクションが増えても、httpセクションの解釈に影響しないことを確認する。
        let cfg = parse(
            r#"
            [other]
            some_key = "value"

            [http]
            port = 9999
            "#,
        )
        .expect("無関係なセクションがあってもパースできるはず");
        assert!(!cfg.http.enabled);
        assert_eq!(cfg.http.port, 9999);
    }
}
