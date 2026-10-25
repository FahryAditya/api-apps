use rust_project_lib::{create_app, get_app_state};
use tower_layer::Layer;
use vercel_runtime::{axum::VercelLayer, run, Error};

#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenvy::dotenv().ok();

    let state = get_app_state()
        .await
        .map_err(|e| Error::from(e.to_string()))?;
    let app = create_app(state);

    let vercel_service = VercelLayer::new().layer(app);
    run(vercel_service).await
}
