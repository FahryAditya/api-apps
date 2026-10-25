use dotenvy::dotenv;
use rust_project_lib::{create_app, get_app_state};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    let port = env::var("PORT").unwrap_or_else(|_| "5000".to_string());
    let db_name = env::var("DB_NAME").unwrap_or_else(|_| "kas_osis".to_string());

    println!("====================================================");
    println!("⚡ Menghubungkan ke MongoDB Atlas (Database: {})...", db_name);

    let state = get_app_state().await?;
    println!("✅ Berhasil terhubung ke MongoDB Atlas!");

    let collections = state.db.list_collection_names().await.unwrap_or_default();
    println!("📑 Koleksi yang tersedia: {:?}", collections);

    let app = create_app(state);

    let addr = format!("0.0.0.0:{}", port);
    println!("🚀 Server Kas OSIS (Rust Axum) berjalan pada port: {}", port);
    println!("📡 URL API: http://localhost:{}/api", port);
    println!("🩺 Health check: http://localhost:{}/api/health", port);
    println!("====================================================");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
