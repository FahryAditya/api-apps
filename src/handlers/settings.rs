use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use mongodb::bson::{doc, Document};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::utils::doc_to_json;
use crate::AppState;

#[derive(Deserialize)]
pub struct CategoryTypeQuery {
    pub r#type: Option<String>,
}

#[derive(Deserialize)]
pub struct AddCategoryBody {
    pub name: Option<String>,
    pub r#type: Option<String>,
}

/// GET /api/settings
pub async fn get_settings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let collection = state.db.collection::<Document>("settings");

    let setting = match collection.find_one(doc! { "_id": "main_config" }).await {
        Ok(Some(d)) => doc_to_json(d),
        _ => {
            // Auto-seed default setting jika belum ada
            let default_doc = doc! {
                "_id": "main_config",
                "orgName": "OSIS SKARLAKES",
                "schoolName": "SMK Airlangga & SMK Kesehatan Airlangga",
                "treasurerName": "Bendahara OSIS",
                "initialBalance": 0,
                "kasFee": 2000,
                "kasFeeEffectiveDate": "2026-10-01",
                "feeHistory": [],
                "holidays": [],
                "activePeriod": "2026-10",
                "currencyStyle": "idr",
                "categoriesIn": [
                    "Kas Anggota",
                    "Dana Sekolah",
                    "Sponsorship",
                    "Donasi Alumni",
                    "Bazar OSIS"
                ],
                "categoriesOut": [
                    "Perlengkapan OSIS",
                    "Konsumsi Rapat",
                    "Administrasi & Cetak",
                    "Dokumentasi & Publikasi",
                    "Transportasi"
                ]
            };
            let _ = collection.insert_one(default_doc.clone()).await;
            doc_to_json(default_doc)
        }
    };

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "Berhasil mengambil konfigurasi kas OSIS",
            "data": setting
        })),
    )
}

/// PUT /api/settings
pub async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let collection = state.db.collection::<Document>("settings");

    let mut update_doc = Document::new();
    if let Value::Object(map) = payload {
        for (k, v) in map {
            if k != "_id" {
                if let Ok(b) = mongodb::bson::to_bson(&v) {
                    update_doc.insert(k, b);
                }
            }
        }
    }

    let options = mongodb::options::FindOneAndUpdateOptions::builder()
        .upsert(true)
        .return_document(mongodb::options::ReturnDocument::After)
        .build();

    match collection
        .find_one_and_update(
            doc! { "_id": "main_config" },
            doc! { "$set": update_doc },
        )
        .with_options(options)
        .await
    {
        Ok(Some(updated)) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "Pengaturan kas berhasil diperbarui",
                "data": doc_to_json(updated)
            })),
        ),
        Ok(None) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "Pengaturan kas berhasil diperbarui",
                "data": {}
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": e.to_string()
            })),
        ),
    }
}

/// POST /api/settings/categories
pub async fn add_category(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AddCategoryBody>,
) -> impl IntoResponse {
    let name = match body.name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Nama kategori wajib diisi"
                })),
            )
        }
    };

    let cat_type = body.r#type.unwrap_or_default();
    if cat_type != "in" && cat_type != "out" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Tipe kategori harus \"in\" atau \"out\""
            })),
        );
    }

    let field = if cat_type == "in" {
        "categoriesIn"
    } else {
        "categoriesOut"
    };

    let collection = state.db.collection::<Document>("settings");
    let options = mongodb::options::FindOneAndUpdateOptions::builder()
        .upsert(true)
        .return_document(mongodb::options::ReturnDocument::After)
        .build();

    match collection
        .find_one_and_update(
            doc! { "_id": "main_config" },
            doc! { "$addToSet": { field: &name } },
        )
        .with_options(options)
        .await
    {
        Ok(Some(updated)) => (
            StatusCode::CREATED,
            Json(json!({
                "success": true,
                "message": format!("Kategori \"{}\" berhasil ditambahkan ke {}", name, if cat_type == "in" { "pemasukan" } else { "pengeluaran" }),
                "data": doc_to_json(updated)
            })),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Gagal menambahkan kategori"
            })),
        ),
    }
}

/// DELETE /api/settings/categories/:name
pub async fn delete_category(
    Path(name): Path<String>,
    Query(query): Query<CategoryTypeQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let cat_type = query.r#type.unwrap_or_else(|| "in".to_string());
    let field = if cat_type == "in" {
        "categoriesIn"
    } else {
        "categoriesOut"
    };

    let collection = state.db.collection::<Document>("settings");
    let options = mongodb::options::FindOneAndUpdateOptions::builder()
        .return_document(mongodb::options::ReturnDocument::After)
        .build();

    match collection
        .find_one_and_update(
            doc! { "_id": "main_config" },
            doc! { "$pull": { field: &name } },
        )
        .with_options(options)
        .await
    {
        Ok(Some(updated)) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": format!("Kategori \"{}\" berhasil dihapus", name),
                "data": doc_to_json(updated)
            })),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Gagal menghapus kategori"
            })),
        ),
    }
}
