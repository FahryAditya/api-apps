pub mod handlers;
pub mod utils;

use axum::{
    http::{header, Method, StatusCode, Uri},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use mongodb::{options::ClientOptions, Client, Database};
use serde_json::json;
use std::env;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub struct AppState {
    pub client: Client,
    pub db: Database,
    pub default_db_name: String,
}

pub async fn get_app_state() -> Result<Arc<AppState>, Box<dyn std::error::Error>> {
    let mongodb_uri = env::var("MONGODB_URI")
        .or_else(|_| env::var("MONGO_URI"))
        .expect("Variabel MONGODB_URI atau MONGO_URI belum diatur di environment variable");

    let db_name = env::var("DB_NAME").unwrap_or_else(|_| "kas_osis".to_string());

    let mut client_options = ClientOptions::parse(&mongodb_uri).await?;
    client_options.app_name = Some("Kas-OSIS-Rust-Axum".to_string());

    let client = Client::with_options(client_options)?;
    let active_db = client.database(&db_name);

    Ok(Arc::new(AppState {
        client,
        db: active_db,
        default_db_name: db_name,
    }))
}

pub fn create_app(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    let api_routes = Router::new()
        // Health Check
        .route("/health", get(health_check_handler))
        // Settings Routes
        .route(
            "/settings",
            get(handlers::settings::get_settings).put(handlers::settings::update_settings),
        )
        .route(
            "/settings/categories",
            post(handlers::settings::add_category),
        )
        .route(
            "/settings/categories/{name}",
            delete(handlers::settings::delete_category),
        )
        // Members Routes
        .route(
            "/members",
            get(handlers::members::get_members).post(handlers::members::create_member),
        )
        .route("/members/bulk", post(handlers::members::bulk_create_members))
        .route(
            "/members/{id}",
            get(handlers::members::get_member_by_id)
                .put(handlers::members::update_member)
                .delete(handlers::members::delete_member),
        )
        // Transactions Routes
        .route(
            "/transactions",
            get(handlers::transactions::get_transactions)
                .post(handlers::transactions::create_transaction)
                .delete(handlers::transactions::delete_all_transactions),
        )
        .route(
            "/transactions/kas-payment",
            post(handlers::transactions::create_kas_payment),
        )
        .route(
            "/transactions/kas-payment/bulk",
            post(handlers::transactions::create_bulk_kas_payment),
        )
        .route(
            "/transactions/{id}",
            put(handlers::transactions::update_transaction)
                .delete(handlers::transactions::delete_transaction),
        )
        // Reports Routes
        .route("/reports/summary", get(handlers::reports::get_report_summary))
        // Payment Gateway (QRIS / Midtrans / DANA) Routes
        .route("/payment/create-order", post(handlers::payment::create_order))
        .route(
            "/payment/status/{orderId}",
            get(handlers::payment::get_order_status),
        )
        .route(
            "/payment/simulate/{orderId}",
            post(handlers::payment::simulate_payment),
        )
        .route(
            "/payment/webhook",
            post(handlers::payment::handle_webhook).get(handlers::payment::webhook_ping),
        );

    Router::new()
        .route("/", get(root_handler))
        .nest("/api", api_routes)
        .fallback(fallback_404)
        .layer(cors)
        .with_state(state)
}

async fn root_handler() -> impl IntoResponse {
    Json(json!({
        "app": "Kas OSIS REST API",
        "status": "Active",
        "platform": "Rust Axum Server (Vercel Serverless)",
        "version": "1.0.0",
        "documentation": "/api/health"
    }))
}

async fn health_check_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "Backend Kas OSIS API berjalan dengan baik di Vercel",
            "timestamp": utils::get_iso_timestamp()
        })),
    )
}

async fn fallback_404(uri: Uri, method: Method) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "success": false,
            "message": format!("Endpoint tidak ditemukan: {} {}", method, uri)
        })),
    )
}
