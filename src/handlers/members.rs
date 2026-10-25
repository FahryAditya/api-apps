use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use futures::stream::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::utils::{doc_to_json, to_title_case};
use crate::AppState;

#[derive(Deserialize)]
pub struct MembersQuery {
    pub period: Option<String>,
    pub search: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateMemberBody {
    pub name: Option<String>,
    pub role: Option<String>,
    #[serde(rename = "className")]
    pub class_name: Option<String>,
}

#[derive(Deserialize)]
pub struct BulkMemberItem {
    pub name: Option<String>,
    pub role: Option<String>,
    #[serde(rename = "className")]
    pub class_name: Option<String>,
}

#[derive(Deserialize)]
pub struct BulkMembersBody {
    pub members: Option<Vec<BulkMemberItem>>,
}

#[derive(Deserialize)]
pub struct UpdateMemberBody {
    pub name: Option<String>,
    pub role: Option<String>,
    #[serde(rename = "className")]
    pub class_name: Option<String>,
}

/// GET /api/members
pub async fn get_members(
    Query(query): Query<MembersQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let settings_coll = state.db.collection::<Document>("settings");
    let members_coll = state.db.collection::<Document>("members");
    let tx_coll = state.db.collection::<Document>("transactions");

    // 1. Tentukan periode aktif
    let period = match query.period {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => {
            if let Ok(Some(s)) = settings_coll.find_one(doc! { "_id": "main_config" }).await {
                s.get_str("activePeriod").unwrap_or("2026-10").to_string()
            } else {
                "2026-10".to_string()
            }
        }
    };

    // 2. Query filter anggota aktif
    let mut filter = doc! { "isActive": true };
    if let Some(search) = query.search {
        let s = search.trim();
        if !s.is_empty() {
            let regex_doc = doc! { "$regex": s, "$options": "i" };
            filter.insert(
                "$or",
                vec![
                    doc! { "name": regex_doc.clone() },
                    doc! { "role": regex_doc.clone() },
                    doc! { "className": regex_doc },
                ],
            );
        }
    }

    let find_options = mongodb::options::FindOptions::builder()
        .sort(doc! { "createdAt": 1 })
        .build();

    let mut members_cursor = match members_coll.find(filter).with_options(find_options).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "success": false, "message": e.to_string() })),
            )
        }
    };

    let mut raw_members = Vec::new();
    let mut member_oids = Vec::new();
    let mut member_str_ids = Vec::new();

    while let Ok(Some(doc)) = members_cursor.try_next().await {
        if let Ok(oid) = doc.get_object_id("_id") {
            member_oids.push(Bson::ObjectId(oid));
            member_str_ids.push(oid.to_hex());
        } else if let Ok(id_str) = doc.get_str("_id") {
            member_str_ids.push(id_str.to_string());
        }
        raw_members.push(doc);
    }

    // 3. Ambil transaksi kas masuk periode ini untuk anggota terkait
    let tx_filter = doc! {
        "memberId": { "$in": &member_oids },
        "period": &period,
        "type": "in"
    };

    let mut tx_map: HashMap<String, Document> = HashMap::new();
    if let Ok(mut tx_cursor) = tx_coll.find(tx_filter).await {
        while let Ok(Some(tx_doc)) = tx_cursor.try_next().await {
            if let Ok(m_id) = tx_doc.get_object_id("memberId") {
                tx_map.insert(m_id.to_hex(), tx_doc);
            } else if let Ok(m_str) = tx_doc.get_str("memberId") {
                tx_map.insert(m_str.to_string(), tx_doc);
            }
        }
    }

    // 4. Gabungkan status pelunasan
    let mut members_with_status = Vec::new();
    let mut paid_count = 0;

    for m in raw_members {
        let mut json_val = doc_to_json(m.clone());
        let m_id_str = if let Ok(oid) = m.get_object_id("_id") {
            oid.to_hex()
        } else {
            m.get_str("_id").unwrap_or("").to_string()
        };

        if let Some(tx) = tx_map.get(&m_id_str) {
            paid_count += 1;
            let tx_id = if let Ok(tx_oid) = tx.get_object_id("_id") {
                tx_oid.to_hex()
            } else {
                tx.get_str("_id").unwrap_or("").to_string()
            };
            let paid_amount = tx.get_i64("amount").unwrap_or_else(|_| {
                tx.get_i32("amount").map(|i| i as i64).unwrap_or(0)
            });
            let paid_date = tx.get_str("date").unwrap_or("").to_string();

            if let Value::Object(ref mut map) = json_val {
                map.insert("isPaid".to_string(), Value::Bool(true));
                map.insert("paidAmount".to_string(), json!(paid_amount));
                map.insert("paidTransactionId".to_string(), Value::String(tx_id));
                map.insert("paidDate".to_string(), Value::String(paid_date));
            }
        } else {
            if let Value::Object(ref mut map) = json_val {
                map.insert("isPaid".to_string(), Value::Bool(false));
                map.insert("paidAmount".to_string(), json!(0));
                map.insert("paidTransactionId".to_string(), Value::Null);
                map.insert("paidDate".to_string(), Value::Null);
            }
        }

        members_with_status.push(json_val);
    }

    let total = members_with_status.len();
    let unpaid_count = total.saturating_sub(paid_count);

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": format!("Daftar anggota untuk periode {}", period),
            "period": period,
            "total": total,
            "count": total,
            "paidCount": paid_count,
            "unpaidCount": unpaid_count,
            "data": members_with_status
        })),
    )
}

/// GET /api/members/:id
pub async fn get_member_by_id(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let members_coll = state.db.collection::<Document>("members");
    let tx_coll = state.db.collection::<Document>("transactions");

    let filter = match ObjectId::parse_str(&id) {
        Ok(oid) => doc! { "_id": oid },
        Err(_) => doc! { "_id": &id },
    };

    let member_doc = match members_coll.find_one(filter).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "success": false,
                    "message": "Anggota tidak ditemukan"
                })),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "success": false, "message": e.to_string() })),
            )
        }
    };

    // Ambil riwayat kas transaksi anggota
    let tx_filter = match ObjectId::parse_str(&id) {
        Ok(oid) => doc! {
            "$or": [
                { "memberId": oid },
                { "memberId": &id }
            ],
            "type": "in"
        },
        Err(_) => doc! { "memberId": &id, "type": "in" },
    };

    let tx_options = mongodb::options::FindOptions::builder()
        .sort(doc! { "date": -1, "createdAt": -1 })
        .build();

    let mut kas_history = Vec::new();
    if let Ok(mut cursor) = tx_coll.find(tx_filter).with_options(tx_options).await {
        while let Ok(Some(tx_doc)) = cursor.try_next().await {
            kas_history.push(doc_to_json(tx_doc));
        }
    }

    let mut member_json = doc_to_json(member_doc);
    if let Value::Object(ref mut map) = member_json {
        map.insert("kasHistory".to_string(), Value::Array(kas_history));
    }

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "Detail data anggota berhasil diambil",
            "data": member_json
        })),
    )
}

/// POST /api/members
pub async fn create_member(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateMemberBody>,
) -> impl IntoResponse {
    let name_raw = match body.name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Nama anggota wajib diisi"
                })),
            )
        }
    };

    let name = to_title_case(&name_raw);
    let role = body
        .role
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| "Anggota OSIS".to_string());
    let class_name = body.class_name.unwrap_or_default();

    let members_coll = state.db.collection::<Document>("members");

    // Cek duplikasi nama anggota aktif (case-insensitive)
    let duplicate_filter = doc! {
        "name": { "$regex": format!("^{}$", regex_escape(&name)), "$options": "i" },
        "isActive": true
    };

    if let Ok(Some(_)) = members_coll.find_one(duplicate_filter).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": format!("Anggota dengan nama \"{}\" sudah ada di database.", name)
            })),
        );
    }

    let now_date = mongodb::bson::DateTime::now();
    let new_member = doc! {
        "name": &name,
        "role": &role,
        "className": &class_name,
        "isActive": true,
        "createdAt": now_date,
        "updatedAt": now_date
    };

    match members_coll.insert_one(new_member.clone()).await {
        Ok(result) => {
            let mut res_doc = new_member;
            res_doc.insert("_id", result.inserted_id);
            (
                StatusCode::CREATED,
                Json(json!({
                    "success": true,
                    "message": "Anggota OSIS berhasil ditambahkan",
                    "data": doc_to_json(res_doc)
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": e.to_string()
            })),
        ),
    }
}

/// POST /api/members/bulk
pub async fn bulk_create_members(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BulkMembersBody>,
) -> impl IntoResponse {
    let items = match body.members {
        Some(list) if !list.is_empty() => list,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Daftar anggota (array) wajib disediakan"
                })),
            )
        }
    };

    let members_coll = state.db.collection::<Document>("members");
    let mut inserted = Vec::new();
    let mut skipped = Vec::new();

    for item in items {
        let name_str = match item.name {
            Some(n) if !n.trim().is_empty() => to_title_case(n.trim()),
            _ => continue,
        };
        let class_str = item.class_name.unwrap_or_default().trim().to_string();
        let role_str = item
            .role
            .filter(|r| !r.trim().is_empty())
            .unwrap_or_else(|| "Anggota OSIS".to_string());

        let existing_filter = doc! {
            "name": { "$regex": format!("^{}$", regex_escape(&name_str)), "$options": "i" },
            "isActive": true
        };

        if let Ok(Some(existing)) = members_coll.find_one(existing_filter).await {
            if !class_str.is_empty() {
                let curr_class = existing.get_str("className").unwrap_or("");
                if curr_class.is_empty() {
                    let _ = members_coll
                        .update_one(
                            doc! { "_id": existing.get("_id").unwrap() },
                            doc! { "$set": { "className": &class_str } },
                        )
                        .await;
                }
            }
            skipped.push(name_str);
        } else {
            let now_date = mongodb::bson::DateTime::now();
            let new_doc = doc! {
                "name": &name_str,
                "className": &class_str,
                "role": &role_str,
                "isActive": true,
                "createdAt": now_date,
                "updatedAt": now_date
            };
            if let Ok(res) = members_coll.insert_one(new_doc.clone()).await {
                let mut created = new_doc;
                created.insert("_id", res.inserted_id);
                inserted.push(doc_to_json(created));
            }
        }
    }

    (
        StatusCode::CREATED,
        Json(json!({
            "success": true,
            "message": format!("Berhasil mengimpor {} anggota baru{}", inserted.len(), if !skipped.is_empty() { format!(" ({} dilewati)", skipped.len()) } else { "".to_string() }),
            "count": inserted.len(),
            "skippedCount": skipped.len(),
            "data": inserted
        })),
    )
}

/// PUT /api/members/:id
pub async fn update_member(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<UpdateMemberBody>,
) -> impl IntoResponse {
    let members_coll = state.db.collection::<Document>("members");
    let filter = match ObjectId::parse_str(&id) {
        Ok(oid) => doc! { "_id": oid },
        Err(_) => doc! { "_id": &id },
    };

    let mut set_doc = doc! { "updatedAt": mongodb::bson::DateTime::now() };
    if let Some(n) = body.name {
        if !n.trim().is_empty() {
            set_doc.insert("name", to_title_case(n.trim()));
        }
    }
    if let Some(r) = body.role {
        if !r.trim().is_empty() {
            set_doc.insert("role", r.trim());
        }
    }
    if let Some(c) = body.class_name {
        set_doc.insert("className", c.trim());
    }

    let options = mongodb::options::FindOneAndUpdateOptions::builder()
        .return_document(mongodb::options::ReturnDocument::After)
        .build();

    match members_coll
        .find_one_and_update(filter, doc! { "$set": set_doc })
        .with_options(options)
        .await
    {
        Ok(Some(updated)) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "Data anggota berhasil diperbarui",
                "data": doc_to_json(updated)
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "message": "Anggota tidak ditemukan"
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

/// DELETE /api/members/:id
pub async fn delete_member(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let members_coll = state.db.collection::<Document>("members");
    let filter = match ObjectId::parse_str(&id) {
        Ok(oid) => doc! { "_id": oid },
        Err(_) => doc! { "_id": &id },
    };

    match members_coll
        .find_one_and_update(
            filter,
            doc! { "$set": { "isActive": false, "updatedAt": mongodb::bson::DateTime::now() } },
        )
        .await
    {
        Ok(Some(doc)) => {
            let name = doc.get_str("name").unwrap_or("Anggota");
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "message": format!("Anggota {} berhasil dinonaktifkan", name)
                })),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "message": "Anggota tidak ditemukan"
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

fn regex_escape(s: &str) -> String {
    let mut escaped = String::new();
    for c in s.chars() {
        if "^$.*+?()[]{}|\\".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}
