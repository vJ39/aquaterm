// HTTP API(ローカルループバックのみ・別タスクで着手・docs/spec.mdの
// 「## HTTP API」節参照)。
//
// エンドポイント:
//   GET  /status    … 魚数・病気数・空腹度平均・経過時間等をJSONで返す(read-only)
//   POST /feed      … 餌やり(fキー相当)をトリガーする
//   POST /medicate  … 投薬(mキー相当)をトリガーする
// それ以外のパス/メソッドには404を返す。認証は無し(個人用途・外部非公開想定)。
//
// 設計方針: SimulationそのものをArc<Mutex<Simulation>>でスレッドまたぎ共有する
// のではなく、(1)直近状態のスナップショット(StatusSnapshot)と(2)操作要求を
// 貯めるコマンドキュー(HttpCommand)の2つだけを共有する。Simulation本体は
// これまでどおりメインループのスレッドだけが所有し続けるため、
// 既存の単一スレッド構造(sim.update()やhandle_key()がSimulationを直接
// &mutで触る作り)を一切変更せずに済む。メインループは毎フレーム
// (a) drain_commands()で溜まった操作要求を取り出してsim.feed()/sim.medicate()を
//     呼び、
// (b) publish_snapshot()で最新状態を公開する
// だけでよい。

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;
use tiny_http::{Header, Method, Response, Server, StatusCode};

use crate::config::HttpConfig;
use crate::sim::Simulation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpCommand {
    Feed,
    Medicate,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatusSnapshot {
    pub fish_count: usize,
    pub sick_count: usize,
    pub avg_hunger: f64,
    pub elapsed_secs: f64,
    pub paused: bool,
}

impl Default for StatusSnapshot {
    fn default() -> Self {
        StatusSnapshot {
            fish_count: 0,
            sick_count: 0,
            avg_hunger: 0.0,
            elapsed_secs: 0.0,
            paused: false,
        }
    }
}

impl StatusSnapshot {
    // Simulationの現在状態から/statusのレスポンス形式を作る。
    pub fn capture(sim: &Simulation, paused: bool) -> Self {
        let fish_count = sim.fish_count();
        let sick_count = sim.sick_count();
        let avg_hunger = if fish_count == 0 {
            0.0
        } else {
            sim.fish.iter().map(|f| f.hunger).sum::<f64>() / fish_count as f64
        };
        StatusSnapshot {
            fish_count,
            sick_count,
            avg_hunger,
            elapsed_secs: sim.elapsed,
            paused,
        }
    }
}

// メインループ(1スレッド)とHTTPサーバー(別スレッド)の間で共有する状態。
// フィールドは意図的に小さく保つ(Simulation全体を持たせない)。
pub struct HttpShared {
    snapshot: Mutex<StatusSnapshot>,
    commands: Mutex<VecDeque<HttpCommand>>,
}

impl HttpShared {
    pub fn new() -> Arc<Self> {
        Arc::new(HttpShared {
            snapshot: Mutex::new(StatusSnapshot::default()),
            commands: Mutex::new(VecDeque::new()),
        })
    }

    // メインループから毎フレーム呼ぶ想定: 最新状態を公開する。
    pub fn publish_snapshot(&self, snapshot: StatusSnapshot) {
        if let Ok(mut guard) = self.snapshot.lock() {
            *guard = snapshot;
        }
    }

    fn snapshot(&self) -> StatusSnapshot {
        match self.snapshot.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => StatusSnapshot::default(),
        }
    }

    fn push_command(&self, cmd: HttpCommand) {
        if let Ok(mut guard) = self.commands.lock() {
            guard.push_back(cmd);
        }
    }

    // メインループから毎フレーム呼ぶ想定: 溜まった操作要求を全部取り出す。
    pub fn drain_commands(&self) -> Vec<HttpCommand> {
        match self.commands.lock() {
            Ok(mut guard) => guard.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }
}

// 設定で指定されたポートにHTTPサーバーをbindし、専用スレッドで待ち受けを
// 開始する。ポート使用中等でbindに失敗した場合はErrを返すだけでpanicしない
// (呼び出し側でメッセージ表示のみ行い、TUI自体はそのまま起動を続ける)。
pub fn start(config: &HttpConfig, shared: Arc<HttpShared>) -> Result<thread::JoinHandle<()>, String> {
    let server = bind(config.port)?;
    Ok(thread::spawn(move || serve(server, shared)))
}

fn bind(port: u16) -> Result<Server, String> {
    Server::http(("127.0.0.1", port)).map_err(|e| e.to_string())
}

fn serve(server: Server, shared: Arc<HttpShared>) {
    for request in server.incoming_requests() {
        handle_request(request, &shared);
    }
}

fn handle_request(request: tiny_http::Request, shared: &Arc<HttpShared>) {
    let (status, body) = match (request.method(), request.url()) {
        (Method::Get, "/status") => {
            let snap = shared.snapshot();
            (
                200,
                serde_json::to_string(&snap).unwrap_or_else(|_| "{}".to_string()),
            )
        }
        (Method::Post, "/feed") => {
            shared.push_command(HttpCommand::Feed);
            (200, r#"{"ok":true}"#.to_string())
        }
        (Method::Post, "/medicate") => {
            shared.push_command(HttpCommand::Medicate);
            (200, r#"{"ok":true}"#.to_string())
        }
        _ => (404, r#"{"error":"not found"}"#.to_string()),
    };

    let json_header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("固定のヘッダ名/値なので失敗しない");
    let response = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(json_header);
    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fish::{Fish, Species, Stage};
    use crate::rng::Rng;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    fn new_fish(hunger: f64) -> Fish {
        let mut f = Fish::new(Species::Neon, Stage::Adult, 10.0, 10.0);
        f.hunger = hunger;
        f
    }

    #[test]
    fn capture_with_no_fish_reports_zero_avg_hunger() {
        let sim = Simulation::new(Rng::new(1));
        let snap = StatusSnapshot::capture(&sim, false);
        assert_eq!(snap.fish_count, 0);
        assert_eq!(snap.sick_count, 0);
        assert_eq!(snap.avg_hunger, 0.0);
        assert_eq!(snap.elapsed_secs, 0.0);
        assert!(!snap.paused);
    }

    #[test]
    fn capture_computes_average_hunger_and_sick_count() {
        let mut sim = Simulation::new(Rng::new(1));
        sim.fish.push(new_fish(100.0));
        sim.fish.push(new_fish(50.0));
        sim.fish[1].sick = true;
        sim.elapsed = 12.5;

        let snap = StatusSnapshot::capture(&sim, true);
        assert_eq!(snap.fish_count, 2);
        assert_eq!(snap.sick_count, 1);
        assert_eq!(snap.avg_hunger, 75.0);
        assert_eq!(snap.elapsed_secs, 12.5);
        assert!(snap.paused);
    }

    #[test]
    fn status_snapshot_serializes_with_expected_keys() {
        let snap = StatusSnapshot {
            fish_count: 3,
            sick_count: 1,
            avg_hunger: 42.5,
            elapsed_secs: 100.0,
            paused: false,
        };
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["fish_count"], 3);
        assert_eq!(json["sick_count"], 1);
        assert_eq!(json["avg_hunger"], 42.5);
        assert_eq!(json["elapsed_secs"], 100.0);
        assert_eq!(json["paused"], false);
    }

    #[test]
    fn shared_commands_are_drained_exactly_once() {
        let shared = HttpShared::new();
        shared.push_command(HttpCommand::Feed);
        shared.push_command(HttpCommand::Medicate);

        let drained = shared.drain_commands();
        assert_eq!(drained, vec![HttpCommand::Feed, HttpCommand::Medicate]);
        // 一度drainしたら空になっている(取りこぼし・二重適用防止)
        assert!(shared.drain_commands().is_empty());
    }

    #[test]
    fn shared_snapshot_publishes_latest_value() {
        let shared = HttpShared::new();
        assert_eq!(shared.snapshot(), StatusSnapshot::default());

        let snap = StatusSnapshot {
            fish_count: 5,
            sick_count: 0,
            avg_hunger: 80.0,
            elapsed_secs: 3.0,
            paused: false,
        };
        shared.publish_snapshot(snap.clone());
        assert_eq!(shared.snapshot(), snap);
    }

    fn http_get(port: u16, path: &str) -> (u32, String) {
        http_request(port, "GET", path)
    }

    fn http_post(port: u16, path: &str) -> (u32, String) {
        http_request(port, "POST", path)
    }

    // 生のHTTPクライアントを使わず、実際にTCP接続してリクエスト文字列を
    // 送るだけの最小限のヘルパー(外部HTTPクライアントクレートを追加しないため)。
    fn http_request(port: u16, method: &str, path: &str) -> (u32, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("接続できるはず");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();

        let mut parts = resp.splitn(2, "\r\n");
        let status_line = parts.next().unwrap_or("");
        let status_code: u32 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let body = resp
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or("")
            .to_string();
        (status_code, body)
    }

    // 実際にサーバーをbindしてリクエストを送る統合テスト。ポート0を指定して
    // OSに空きポートを割り当てさせることで、他のテスト/プロセスとの競合を避ける。
    #[test]
    fn server_status_feed_medicate_and_unknown_paths_respond_as_expected() {
        let server = bind(0).expect("127.0.0.1へのbindは失敗しないはず");
        let port = server
            .server_addr()
            .to_ip()
            .expect("IPアドレスでbindしたはず")
            .port();
        let shared = HttpShared::new();
        shared.publish_snapshot(StatusSnapshot {
            fish_count: 7,
            sick_count: 2,
            avg_hunger: 33.0,
            elapsed_secs: 9.0,
            paused: false,
        });
        let shared_for_thread = shared.clone();
        let handle = thread::spawn(move || serve(server, shared_for_thread));

        let (status, body) = http_get(port, "/status");
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_str(&body).expect("JSONで返るはず");
        assert_eq!(json["fish_count"], 7);
        assert_eq!(json["sick_count"], 2);

        let (status, _) = http_post(port, "/feed");
        assert_eq!(status, 200);
        let (status, _) = http_post(port, "/medicate");
        assert_eq!(status, 200);
        assert_eq!(
            shared.drain_commands(),
            vec![HttpCommand::Feed, HttpCommand::Medicate]
        );

        let (status, _) = http_get(port, "/nope");
        assert_eq!(status, 404);

        // サーバースレッドを終了させる(テストプロセス終了まで待ち受け続けないように)。
        drop(TcpStream::connect(("127.0.0.1", port))); // 念のため接続を促す(unblock後は不要になるが無害)
        // tiny_http::Server::unblock()相当の処理が無いためスレッドはjoinせず放置してよい
        // (テストプロセス終了時に片付く)。handleを明示的に使ってdead_code警告を避ける。
        let _ = handle;
    }

    #[test]
    fn start_reports_error_instead_of_panicking_when_port_is_already_in_use() {
        // 先に1つ掴んでおき、同じポートへのstart()がErrで返ることを確認する
        // (「ポート使用中でもTUI自体はクラッシュしない」という完了条件に対応)。
        let first = bind(0).expect("127.0.0.1へのbindは失敗しないはず");
        let port = first
            .server_addr()
            .to_ip()
            .expect("IPアドレスでbindしたはず")
            .port();

        let cfg = HttpConfig {
            enabled: true,
            port,
        };
        let shared = HttpShared::new();
        let result = start(&cfg, shared);
        assert!(result.is_err(), "使用中ポートへのbindはErrになるはず");
    }
}
