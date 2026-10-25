use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::utils::{doc_to_json, get_current_time, get_today_date};
use crate::AppState;

#[derive(Deserialize)]
pub struct CreateOrderBody {
    #[serde(rename = "memberId")]
    pub member_id: Option<String>,
    #[serde(rename = "allocatedWeeks")]
    pub allocated_weeks: Option<Vec<String>>,
    pub period: Option<String>,
    pub amount: Option<f64>,
    #[serde(rename = "adminFee")]
    pub admin_fee: Option<f64>,
    #[serde(rename = "paymentType")]
    pub payment_type: Option<String>,
}

/// POST /api/payment/create-order
pub async fn create_order(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateOrderBody>,
) -> impl IntoResponse {
    let member_id_str = match body.member_id {
        Some(ref id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Data memberId dan allocatedWeeks (daftar tanggal Kamis) wajib disertakan."
                })),
            )
        }
    };

    let allocated_weeks = match body.allocated_weeks {
        Some(ref weeks) if !weeks.is_empty() => weeks.clone(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Data memberId dan allocatedWeeks (daftar tanggal Kamis) wajib disertakan."
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
                    "message": "Format memberId tidak valid."
                })),
            )
        }
    };

    let members_coll = state.db.collection::<Document>("members");
    let tx_coll = state.db.collection::<Document>("transactions");
    let orders_coll = state.db.collection::<Document>("payment_orders");

    // Cari data siswa
    let member = match members_coll.find_one(doc! { "_id": member_oid }).await {
        Ok(Some(m)) => m,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "success": false,
                    "message": "Data anggota tidak ditemukan."
                })),
            )
        }
    };

    let member_name = member.get_str("name").unwrap_or("Anggota");
    let member_role = member.get_str("role").unwrap_or("Anggota OSIS");

    // Proteksi Server: Cek apakah minggu yang dipilih sudah pernah dibayar
    let check_filter = doc! {
        "memberId": member_oid,
        "isVoid": { "$ne": true },
        "allocatedWeeks": { "$in": &allocated_weeks }
    };

    if let Ok(Some(existing)) = tx_coll.find_one(check_filter).await {
        let existing_weeks = existing
            .get_array("allocatedWeeks")
            .ok()
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| b.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let conflict_dates: Vec<String> = allocated_weeks
            .iter()
            .filter(|w| existing_weeks.contains(w))
            .cloned()
            .collect();

        return (
            StatusCode::CONFLICT,
            Json(json!({
                "success": false,
                "message": format!(
                    "Pembayaran ditolak: Siswa \"{}\" sudah membayar kas untuk tanggal ({}).",
                    member_name,
                    if !conflict_dates.is_empty() { conflict_dates.join(", ") } else { allocated_weeks.join(", ") }
                )
            })),
        );
    }

    // Generate Order ID Unik (contoh: KAS-172983-ABCD)
    let timestamp = chrono::Utc::now().timestamp_millis();
    let rand_num: u16 = rand::rng().random_range(1000..9999);
    let order_id = format!("KAS-{}-{:X}", timestamp, rand_num);

    let amount = body.amount.unwrap_or(0.0);
    let admin_fee = body.admin_fee.unwrap_or(0.0);
    let total_amount = amount + admin_fee;
    let payment_type = body.payment_type.unwrap_or_else(|| "qris".to_string());

    let now_dt = chrono::Utc::now();
    let expired_dt = now_dt + chrono::Duration::minutes(15);

    let period = body
        .period
        .or_else(|| allocated_weeks.first().map(|w| w[..7.min(w.len())].to_string()))
        .unwrap_or_else(|| "2026-10".to_string());

    // Generate QR String & QR Code URL
    let qr_string = format!(
        "00020101021226600016ID.CO.QRIS.WWW01189360099900000000000215{}51450015ID.OR.OSIS.SKAR0215{}520458125303360540{}5802ID5913OSIS SKARLAKES6009BALIKPAPAN6304ABCD",
        order_id, member_id_str, total_amount as i64
    );
    let qr_code_url = format!(
        "https://api.qrserver.com/v1/create-qr-code/?size=300x300&margin=10&data={}",
        url_encode(&qr_string)
    );
    let deeplink_url = format!("https://link.dana.id/m/qris?data={}", url_encode(&qr_string));

    let new_order_doc = doc! {
        "orderId": &order_id,
        "memberId": member_oid,
        "memberName": member_name,
        "memberRole": member_role,
        "allocatedWeeks": &allocated_weeks,
        "period": &period,
        "amount": amount,
        "adminFee": admin_fee,
        "totalAmount": total_amount,
        "paymentType": &payment_type,
        "qrCodeUrl": &qr_code_url,
        "qrString": &qr_string,
        "deeplinkUrl": &deeplink_url,
        "status": "pending",
        "expiredAt": mongodb::bson::DateTime::from_millis(expired_dt.timestamp_millis()),
        "createdAt": mongodb::bson::DateTime::from_millis(now_dt.timestamp_millis()),
        "updatedAt": mongodb::bson::DateTime::from_millis(now_dt.timestamp_millis())
    };

    match orders_coll.insert_one(new_order_doc.clone()).await {
        Ok(_) => (
            StatusCode::CREATED,
            Json(json!({
                "success": true,
                "message": "Order pembayaran QRIS berhasil dibuat.",
                "data": doc_to_json(new_order_doc)
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": format!("Gagal membuat order pembayaran QRIS: {}", e)
            })),
        ),
    }
}

/// GET /api/payment/status/:orderId
pub async fn get_order_status(
    Path(order_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let orders_coll = state.db.collection::<Document>("payment_orders");

    let order = match orders_coll.find_one(doc! { "orderId": &order_id }).await {
        Ok(Some(o)) => o,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "success": false,
                    "message": "Order pembayaran tidak ditemukan."
                })),
            )
        }
    };

    let status = order.get_str("status").unwrap_or("pending");
    let mut updated_order = order.clone();

    // Cek apakah expired
    if status == "pending" {
        if let Ok(exp_dt) = order.get_datetime("expiredAt") {
            let exp_millis = exp_dt.timestamp_millis();
            let now_millis = chrono::Utc::now().timestamp_millis();
            if now_millis > exp_millis {
                let _ = orders_coll
                    .update_one(
                        doc! { "orderId": &order_id },
                        doc! { "$set": { "status": "expired", "updatedAt": mongodb::bson::DateTime::now() } },
                    )
                    .await;
                updated_order.insert("status", "expired");
            }
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "data": doc_to_json(updated_order)
        })),
    )
}

/// POST /api/payment/simulate/:orderId
pub async fn simulate_payment(
    Path(order_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let orders_coll = state.db.collection::<Document>("payment_orders");
    let tx_coll = state.db.collection::<Document>("transactions");

    let order = match orders_coll.find_one(doc! { "orderId": &order_id }).await {
        Ok(Some(o)) => o,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "success": false,
                    "message": "Order tidak ditemukan."
                })),
            )
        }
    };

    let current_status = order.get_str("status").unwrap_or("pending");
    if current_status == "paid" {
        return (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "Order sudah lunas sebelumnya.",
                "data": doc_to_json(order)
            })),
        );
    }

    // Buat transaksi kas resmi jika belum ada
    let member_oid = order.get_object_id("memberId").unwrap_or_else(|_| ObjectId::new());
    let member_name = order.get_str("memberName").unwrap_or("Anggota");
    let allocated_weeks = order.get_array("allocatedWeeks").cloned().unwrap_or_default();
    let weeks_count = allocated_weeks.len();
    let period = order.get_str("period").unwrap_or("2026-10");
    let amount = order
        .get_f64("amount")
        .ok()
        .or_else(|| order.get_i64("amount").ok().map(|i| i as f64))
        .unwrap_or(0.0);

    let weeks_str_list: Vec<String> = allocated_weeks
        .iter()
        .filter_map(|b| b.as_str().map(|s| s.to_string()))
        .collect();

    let weeks_desc = if weeks_str_list.len() <= 1 {
        weeks_str_list.first().cloned().unwrap_or_default()
    } else {
        format!(
            "{} s/d {}",
            weeks_str_list.first().unwrap(),
            weeks_str_list.last().unwrap()
        )
    };

    let now_date = mongodb::bson::DateTime::now();
    let new_tx = doc! {
        "type": "in",
        "title": format!("Kas OSIS (QRIS) - {} ({} Minggu)", member_name, weeks_count),
        "category": "Kas Anggota",
        "amount": amount,
        "date": get_today_date(),
        "time": get_current_time(),
        "recorder": "System Gateway (QRIS)",
        "iconBrand": "kas",
        "memberId": member_oid,
        "period": period,
        "allocatedWeeks": &allocated_weeks,
        "weeksCount": weeks_count as i64,
        "note": format!("Pembayaran otomatis via QRIS / DANA (Order: {}, Periode: {})", order_id, weeks_desc),
        "isVoid": false,
        "createdAt": now_date,
        "updatedAt": now_date
    };

    let insert_res = tx_coll.insert_one(new_tx.clone()).await;
    let mut created_tx_doc = new_tx;
    let tx_id_str = match insert_res {
        Ok(r) => {
            created_tx_doc.insert("_id", r.inserted_id);
            if let Ok(oid) = created_tx_doc.get_object_id("_id") {
                oid.to_hex()
            } else {
                created_tx_doc.get_str("_id").unwrap_or("").to_string()
            }
        }
        Err(_) => "".to_string(),
    };

    let mut updated_order = order;
    updated_order.insert("status", "paid");
    updated_order.insert("paidAt", now_date);
    if !tx_id_str.is_empty() {
        updated_order.insert("transactionId", &tx_id_str);
    }

    let _ = orders_coll
        .update_one(
            doc! { "orderId": &order_id },
            doc! {
                "$set": {
                    "status": "paid",
                    "paidAt": now_date,
                    "transactionId": &tx_id_str,
                    "updatedAt": now_date
                }
            },
        )
        .await;

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "Simulasi pembayaran berhasil! Kas otomatis tercatat.",
            "data": {
                "order": doc_to_json(updated_order),
                "transaction": doc_to_json(created_tx_doc)
            }
        })),
    )
}

/// POST /api/payment/webhook
pub async fn handle_webhook(
    State(state): State<Arc<AppState>>,
    Json(notification): Json<Value>,
) -> impl IntoResponse {
    let order_id = notification
        .get("order_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if order_id.is_empty() || order_id.to_lowercase().contains("test") {
        return (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "Midtrans test notification successfully received and acknowledged"
            })),
        );
    }

    let tx_status = notification
        .get("transaction_status")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let orders_coll = state.db.collection::<Document>("payment_orders");

    if tx_status == "settlement" || tx_status == "capture" {
        // Panggil simulasi / settlement internal
        let _ = orders_coll
            .update_one(
                doc! { "orderId": &order_id },
                doc! {
                    "$set": {
                        "status": "paid",
                        "paidAt": mongodb::bson::DateTime::now(),
                        "paymentGatewayResponse": mongodb::bson::to_bson(&notification).unwrap_or(Bson::Null)
                    }
                },
            )
            .await;
    } else if tx_status == "expire" {
        let _ = orders_coll
            .update_one(
                doc! { "orderId": &order_id },
                doc! { "$set": { "status": "expired" } },
            )
            .await;
    } else if tx_status == "cancel" {
        let _ = orders_coll
            .update_one(
                doc! { "orderId": &order_id },
                doc! { "$set": { "status": "cancelled" } },
            )
            .await;
    }

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "Webhook diproses"
        })),
    )
}

/// GET /api/payment/webhook
pub async fn webhook_ping() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "Midtrans Webhook Endpoint Active"
        })),
    )
}

fn url_encode(input: &str) -> String {
    let mut encoded = String::new();
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            _ => {
                encoded.push_str(&format!("%{:02X}", b));
            }
        }
    }
    encoded
}
