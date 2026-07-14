use basketballman::repo::LeagueRepository;
use basketballman::routes::{AppState, app};
use std::env;
use std::fs;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() {
    let data_path = env::var("DATA_PATH").unwrap_or_else(|_| "data".to_string());
    let _ = fs::create_dir_all(&data_path);
    let repo = LeagueRepository::new(format!("{data_path}/league.json"));
    let league = repo
        .load_or_generate(7)
        .expect("league state should load or generate");
    let state = AppState {
        repo,
        league: Arc::new(Mutex::new(league)),
    };
    let app = app(state);

    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{port}")
        .parse()
        .expect("socket address should parse");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("server should bind");

    println!("basketballman listening on http://{addr}");
    axum::serve(listener, app).await.expect("server should run");
}
