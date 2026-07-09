use basketballman::repo::LeagueRepository;
use basketballman::routes::{AppState, app};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() {
    let repo = LeagueRepository::new("data/league.json");
    let league = repo
        .load_or_generate(7)
        .expect("league state should load or generate");
    let state = AppState {
        repo,
        league: Arc::new(Mutex::new(league)),
    };
    let app = app(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("server should bind");

    println!("basketballman listening on http://{addr}");
    axum::serve(listener, app).await.expect("server should run");
}
