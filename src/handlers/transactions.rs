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

use crate::utils::{doc_to_json, get_current_time, get_today_date};
use crate::AppState;

#[derive(Deserialize)]
pub struct TransactionsQuery {
    pub r#type: Option<String>,
    pub category: Option<String>,
    pub month: Option<String>,
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub page: Option<u64>,
}

#[derive(Deserialize)]
pub struct CreateTransactionBody {
    pub r#type: Option<String>,
    pub title: Option<String>,
    pub category: Option<String>,
    pub amount: Option<f64>,
    pub date: Option<String>,
    pub time: Option<String>,
    pub recorder: Option<String>,
    #[serde(rename = "iconBrand")]
    pub icon_brand: Option<String>,
    pub note: Option<String>,
    #[serde(rename = "memberId")]
    pub member_id: Option<String>,
    pub period: Option<String>,
    #[serde(rename = "allocatedWeeks")]
    pub allocated_weeks: Option<Vec<String>>,
    #[serde(rename = "weeksCount")]
    pub weeks_count: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateKasPaymentBody {
    #[serde(rename = "memberId")]
    pub member_id: Option<String>,
    pub period: Option<String>,
    pub amount: Option<f64>,
    pub note: Option<String>,
}

#[derive(Deserialize)]
pub struct BulkKasPaymentBody {
    pub period: Option<String>,
    pub amount: Option<f64>,
    pub note: Option<String>,
}

/// GET /api/transactions
pub async fn get_transactions(
    Query(query): Query<TransactionsQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let tx_coll = state.db.collection::<Document>("transactions");
    let members_coll = state.db.collection::<Document>("members");

    let mut filter = doc! {};

    if let Some(t) = query.r#type {
        if t == "in" || t == "out" {
            filter.insert("type", t);
        }
    }

    if let Some(cat) = query.category {
        if cat != "all" && !cat.trim().is_empty() {
            filter.insert("category", cat.trim());
        }
    }

    if let Some(month) = query.month {
        if month.len() == 7 {
            filter.insert("date", doc! { "$regex": format!("^{}", month) });
        }
    }

    if let Some(search) = query.search {
        let s = search.trim();
        if !s.is_empty() {
            let regex_doc = doc! { "$regex": s, "$options": "i" };
            filter.insert(
                "$or",
                vec![
                    doc! { "title": regex_doc.clone() },
                    doc! { "category": regex_doc.clone() },
                    doc! { "recorder": regex_doc.clone() },
                    doc! { "note": regex_doc },
                ],
            );
        }
    }

    let limit = query.limit.unwrap_or(100).clamp(1, 1000);
    let page = query.page.unwrap_or(1).max(1);
    let skip = (page - 1) * (limit as u64);

    let total = match tx_coll.count_documents(filter.clone()).await {
        Ok(c) => c,
        Err(_) => 0,
    };

    let find_options = mongodb::options::FindOptions::builder()
        .sort(doc! { "date": -1, "createdAt": -1 })
        .skip(skip)
        .limit(limit)
        .build();

    let mut cursor = match tx_coll.find(filter).with_options(find_options).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "success": false, "message": e.to_string() })),
            )
        }
    };

    let mut raw_txs = Vec::new();
    let mut member_ids = Vec::new();

    while let Ok(Some(doc)) = cursor.try_next().await {
        if let Ok(oid) = doc.get_object_id("memberId") {
            member_ids.push(Bson::ObjectId(oid));
        } else if let Ok(s) = doc.get_str("memberId") {
            if let Ok(oid) = ObjectId::parse_str(s) {
                member_ids.push(Bson::ObjectId(oid));
            }
        }
        raw_txs.push(doc);
    }

    // Lookup data member untuk populate memberId: { name, role }
    let mut member_map: HashMap<String, Document> = HashMap::new();
    if !member_ids.is_empty() {
        if let Ok(mut m_cursor) = members_coll
            .find(doc! { "_id": { "$in": member_ids } })
            .await
        {
            while let Ok(Some(m_doc)) = m_cursor.try_next().await {
                if let Ok(oid) = m_doc.get_object_id("_id") {
                    member_map.insert(oid.to_hex(), m_doc);
                }
            }
        }
    }

    let mut populated_txs = Vec::new();
    for tx in raw_txs {
        let mut val = doc_to_json(tx.clone());
        let m_id_opt = tx
            .get_object_id("memberId")
            .map(|o| o.to_hex())
            .ok()
            .or_else(|| tx.get_str("memberId").map(|s| s.to_string()).ok());

        if let Some(m_id) = m_id_opt {
            if let Some(m_doc) = member_map.get(&m_id) {
                if let Value::Object(ref mut map) = val {
                    map.insert(
                        "memberId".to_string(),
                        json!({
                            "_id": m_id,
                            "name": m_doc.get_str("name").unwrap_or(""),
                            "role": m_doc.get_str("role").unwrap_or("")
                        }),
                    );
                }
            }
        }

        populated_txs.push(val);
    }

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "Berhasil mengambil daftar transaksi",
            "total": total,
            "page": page,
            "limit": limit,
            "data": populated_txs
        })),
    )
}

/// POST /api/transactions
pub async fn create_transaction(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTransactionBody>,
) -> impl IntoResponse {
    let tx_type = match body.r#type {
        Some(t) if t == "in" || t == "out" => t,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Tipe transaksi wajib 'in' atau 'out'"
                })),
            )
        }
    };

    let title = match body.title {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Keterangan transaksi wajib diisi"
                })),
            )
        }
    };

    let category = match body.category {
        Some(c) if !c.trim().is_empty() => c.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Kategori transaksi wajib diisi"
                })),
            )
        }
    };

    let amount = match body.amount {
        Some(a) if a > 0.0 => a,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Nominal transaksi harus lebih dari 0"
                })),
            )
        }
    };

    let tx_coll = state.db.collection::<Document>("transactions");
    let members_coll = state.db.collection::<Document>("members");
    let settings_coll = state.db.collection::<Document>("settings");

    let default_recorder = if let Ok(Some(s)) = settings_coll.find_one(doc! { "_id": "main_config" }).await {
        s.get_str("treasurerName").unwrap_or("Bendahara OSIS").to_string()
    } else {
        "Bendahara OSIS".to_string()
    };

    let recorder = body.recorder.unwrap_or(default_recorder);
    let date = body.date.unwrap_or_else(get_today_date);
    let time = body.time.unwrap_or_else(get_current_time);
    let icon_brand = body.icon_brand.unwrap_or_else(|| "general".to_string());
    let note = body.note.unwrap_or_default();

    // Resolusi memberId
    let mut resolved_member_oid: Option<ObjectId> = None;
    let mut resolved_student_name = String::new();

    if let Some(ref m_id_str) = body.member_id {
        if let Ok(oid) = ObjectId::parse_str(m_id_str) {
            resolved_member_oid = Some(oid);
        }
    }

    if resolved_member_oid.is_none() && title.contains("Kas OSIS -") {
        if let Some(name_part) = extract_student_name_from_title(&title) {
            resolved_student_name = name_part.clone();
            if let Ok(Some(found)) = members_coll
                .find_one(doc! { "name": { "$regex": format!("^{}$", name_part), "$options": "i" } })
                .await
            {
                if let Ok(oid) = found.get_object_id("_id") {
                    resolved_member_oid = Some(oid);
                }
            }
        }
    }

    let allocated_weeks = body.allocated_weeks.unwrap_or_default();
    let weeks_to_check = if !allocated_weeks.is_empty() {
        allocated_weeks.clone()
    } else if let Some(ref p) = body.period {
        vec![p.clone()]
    } else {
        Vec::new()
    };

    // PROTEKSI ANTI-DUPLIKASI: Cegah input ganda untuk siswa dan minggu yang sama
    if tx_type == "in" && resolved_member_oid.is_some() && !weeks_to_check.is_empty() {
        let m_oid = resolved_member_oid.unwrap();
        let dup_filter = doc! {
            "type": "in",
            "memberId": m_oid,
            "$or": [
                { "allocatedWeeks": { "$in": &weeks_to_check } },
                { "period": { "$in": &weeks_to_check } }
            ]
        };

        if let Ok(Some(existing_payment)) = tx_coll.find_one(dup_filter).await {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "success": false,
                    "message": format!(
                        "Pembayaran ditolak: Siswa \"{}\" sudah tercatat membayar kas untuk tanggal ({}). Mencegah duplikasi data.",
                        if !resolved_student_name.is_empty() { &resolved_student_name } else { "terkait" },
                        weeks_to_check.join(", ")
                    ),
                    "data": doc_to_json(existing_payment)
                })),
            );
        }
    }

    let period_val = body.period.or_else(|| allocated_weeks.first().cloned());
    let now_date = mongodb::bson::DateTime::now();

    let mut new_doc = doc! {
        "type": &tx_type,
        "title": &title,
        "category": &category,
        "amount": amount,
        "date": &date,
        "time": &time,
        "recorder": &recorder,
        "iconBrand": &icon_brand,
        "note": &note,
        "weeksCount": body.weeks_count.unwrap_or(1),
        "allocatedWeeks": &allocated_weeks,
        "createdAt": now_date,
        "updatedAt": now_date
    };

    if let Some(m_oid) = resolved_member_oid {
        new_doc.insert("memberId", m_oid);
    }
    if let Some(p) = period_val {
        new_doc.insert("period", p);
    }

    match tx_coll.insert_one(new_doc.clone()).await {
        Ok(result) => {
            let mut res_doc = new_doc;
            res_doc.insert("_id", result.inserted_id);
            (
                StatusCode::CREATED,
                Json(json!({
                    "success": true,
                    "message": "Transaksi berhasil disimpan",
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

/// POST /api/transactions/kas-payment
pub async fn create_kas_payment(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateKasPaymentBody>,
) -> impl IntoResponse {
    let member_id_str = match body.member_id {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "memberId, period (YYYY-MM), dan amount wajib diisi"
                })),
            )
        }
    };

    let period = match body.period {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "memberId, period (YYYY-MM), dan amount wajib diisi"
                })),
            )
        }
    };

    let amount = match body.amount {
        Some(a) if a > 0.0 => a,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Nominal pembayaran kas harus lebih besar dari 0"
                })),
            )
        }
    };

    let member_oid = match ObjectId::parse_str(&member_id_str) {
        Ok(oid) => oid,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Format memberId tidak valid"
                })),
            )
        }
    };

    let members_coll = state.db.collection::<Document>("members");
    let tx_coll = state.db.collection::<Document>("transactions");
    let settings_coll = state.db.collection::<Document>("settings");

    // Pastikan member ada dan aktif
    let member = match members_coll
        .find_one(doc! { "_id": member_oid, "isActive": true })
        .await
    {
        Ok(Some(m)) => m,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "success": false,
                    "message": "Data anggota tidak ditemukan atau sudah tidak aktif"
                })),
            )
        }
    };

    let member_name = member.get_str("name").unwrap_or("Anggota");

    // Compound Check: Cegah dobel pembayaran kas pada periode yang sama
    let existing_filter = doc! {
        "memberId": member_oid,
        "period": &period,
        "type": "in"
    };

    if let Ok(Some(_)) = tx_coll.find_one(existing_filter).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": format!("Anggota {} sudah tercatat lunas pada periode {}.", member_name, period)
            })),
        );
    }

    let default_recorder = if let Ok(Some(s)) = settings_coll.find_one(doc! { "_id": "main_config" }).await {
        s.get_str("treasurerName").unwrap_or("Bendahara OSIS").to_string()
    } else {
        "Bendahara OSIS".to_string()
    };

    let now_date = mongodb::bson::DateTime::now();
    let new_tx = doc! {
        "type": "in",
        "title": format!("Kas OSIS - {} ({})", member_name, period),
        "category": "Kas Anggota",
        "amount": amount,
        "date": get_today_date(),
        "time": get_current_time(),
        "recorder": default_recorder,
        "iconBrand": "kas",
        "memberId": member_oid,
        "period": &period,
        "note": body.note.unwrap_or_default(),
        "createdAt": now_date,
        "updatedAt": now_date
    };

    match tx_coll.insert_one(new_tx.clone()).await {
        Ok(result) => {
            let mut res_doc = new_tx;
            res_doc.insert("_id", result.inserted_id);
            (
                StatusCode::CREATED,
                Json(json!({
                    "success": true,
                    "message": format!("Pembayaran kas {} untuk periode {} berhasil disimpan", member_name, period),
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

/// POST /api/transactions/kas-payment/bulk
pub async fn create_bulk_kas_payment(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BulkKasPaymentBody>,
) -> impl IntoResponse {
    let period = match body.period {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "period (YYYY-MM) dan amount wajib diisi"
                })),
            )
        }
    };

    let amount = match body.amount {
        Some(a) if a > 0.0 => a,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Nominal pembayaran massal harus lebih dari 0"
                })),
            )
        }
    };

    let members_coll = state.db.collection::<Document>("members");
    let tx_coll = state.db.collection::<Document>("transactions");
    let settings_coll = state.db.collection::<Document>("settings");

    // Ambil semua anggota aktif
    let mut cursor = match members_coll.find(doc! { "isActive": true }).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "success": false, "message": e.to_string() })),
            )
        }
    };

    let mut active_members = Vec::new();
    let mut active_oids = Vec::new();
    while let Ok(Some(doc)) = cursor.try_next().await {
        if let Ok(oid) = doc.get_object_id("_id") {
            active_oids.push(Bson::ObjectId(oid));
        }
        active_members.push(doc);
    }

    if active_members.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Belum ada anggota aktif untuk dilakukan pembayaran massal"
            })),
        );
    }

    // Cek yang sudah bayar di periode ini
    let paid_txs_filter = doc! {
        "memberId": { "$in": &active_oids },
        "period": &period,
        "type": "in"
    };

    let mut paid_oids = std::collections::HashSet::new();
    if let Ok(mut paid_cursor) = tx_coll.find(paid_txs_filter).await {
        while let Ok(Some(t)) = paid_cursor.try_next().await {
            if let Ok(oid) = t.get_object_id("memberId") {
                paid_oids.insert(oid);
            }
        }
    }

    let unpaid_members: Vec<Document> = active_members
        .into_iter()
        .filter(|m| {
            if let Ok(oid) = m.get_object_id("_id") {
                !paid_oids.contains(&oid)
            } else {
                false
            }
        })
        .collect();

    if unpaid_members.is_empty() {
        return (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": format!("Seluruh anggota sudah lunas pada periode {}. Tidak ada transaksi baru dibuat.", period),
                "data": { "count": 0, "period": period }
            })),
        );
    }

    let default_recorder = if let Ok(Some(s)) = settings_coll.find_one(doc! { "_id": "main_config" }).await {
        s.get_str("treasurerName").unwrap_or("Bendahara OSIS").to_string()
    } else {
        "Bendahara OSIS".to_string()
    };

    let today = get_today_date();
    let time_now = get_current_time();
    let note = body.note.unwrap_or_else(|| "Pembayaran massal kas OSIS".to_string());
    let now_date = mongodb::bson::DateTime::now();

    let new_docs: Vec<Document> = unpaid_members
        .iter()
        .map(|m| {
            let m_name = m.get_str("name").unwrap_or("Anggota");
            let m_oid = m.get_object_id("_id").unwrap();
            doc! {
                "type": "in",
                "title": format!("Kas OSIS - {} ({})", m_name, period),
                "category": "Kas Anggota",
                "amount": amount,
                "date": &today,
                "time": &time_now,
                "recorder": &default_recorder,
                "iconBrand": "kas",
                "memberId": m_oid,
                "period": &period,
                "note": &note,
                "createdAt": now_date,
                "updatedAt": now_date
            }
        })
        .collect();

    let count = new_docs.len();
    let total_collected = (count as f64) * amount;

    match tx_coll.insert_many(new_docs).await {
        Ok(insert_res) => {
            (
                StatusCode::CREATED,
                Json(json!({
                    "success": true,
                    "message": format!("Berhasil mencatat pembayaran kas massal untuk {} anggota", count),
                    "data": {
                        "count": count,
                        "period": period,
                        "totalCollected": total_collected,
                        "insertedCount": insert_res.inserted_ids.len()
                    }
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "success": false, "message": e.to_string() })),
        ),
    }
}

/// PUT /api/transactions/:id
pub async fn update_transaction(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let tx_coll = state.db.collection::<Document>("transactions");
    let filter = match ObjectId::parse_str(&id) {
        Ok(oid) => doc! { "_id": oid },
        Err(_) => doc! { "_id": &id },
    };

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
    update_doc.insert("updatedAt", mongodb::bson::DateTime::now());

    let options = mongodb::options::FindOneAndUpdateOptions::builder()
        .return_document(mongodb::options::ReturnDocument::After)
        .build();

    match tx_coll
        .find_one_and_update(filter, doc! { "$set": update_doc })
        .with_options(options)
        .await
    {
        Ok(Some(updated)) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "Transaksi berhasil diperbarui",
                "data": doc_to_json(updated)
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "message": "Transaksi tidak ditemukan"
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

/// DELETE /api/transactions/:id
pub async fn delete_transaction(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let tx_coll = state.db.collection::<Document>("transactions");
    let filter = match ObjectId::parse_str(&id) {
        Ok(oid) => doc! { "_id": oid },
        Err(_) => doc! { "_id": &id },
    };

    match tx_coll.find_one_and_delete(filter).await {
        Ok(Some(deleted)) => {
            let deleted_id = if let Ok(oid) = deleted.get_object_id("_id") {
                oid.to_hex()
            } else {
                deleted.get_str("_id").unwrap_or("").to_string()
            };
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "message": "Transaksi berhasil dihapus. Saldo dan status kas terkait telah diperbarui.",
                    "data": {
                        "id": deleted_id,
                        "type": deleted.get_str("type").unwrap_or(""),
                        "amount": deleted.get_f64("amount").ok().or_else(|| deleted.get_i64("amount").ok().map(|i| i as f64)).unwrap_or(0.0)
                    }
                })),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "message": "Transaksi tidak ditemukan"
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

/// DELETE /api/transactions (Hapus seluruh riwayat transaksi)
pub async fn delete_all_transactions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let tx_coll = state.db.collection::<Document>("transactions");

    match tx_coll.delete_many(doc! {}).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": format!("Seluruh transaksi kas berhasil dihapus ({} transaksi).", result.deleted_count),
                "data": { "deletedCount": result.deleted_count }
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "success": false, "message": e.to_string() })),
        ),
    }
}

fn extract_student_name_from_title(title: &str) -> Option<String> {
    if let Some(pos) = title.find("Kas OSIS -") {
        let after = &title[pos + 10..].trim();
        // Cek jika ada format "(X Minggu)" di belakang
        if let Some(paren_pos) = after.find('(') {
            let name = after[..paren_pos].trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        } else if !after.is_empty() {
            return Some(after.to_string());
        }
    }
    None
}
