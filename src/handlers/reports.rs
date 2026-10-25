use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use futures::stream::TryStreamExt;
use mongodb::bson::{doc, Document};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::AppState;

#[derive(Deserialize)]
pub struct ReportsQuery {
    pub period: Option<String>,
}

fn extract_f64(doc: &Document, key: &str) -> f64 {
    if let Ok(v) = doc.get_f64(key) {
        return v;
    }
    if let Ok(v) = doc.get_i64(key) {
        return v as f64;
    }
    if let Ok(v) = doc.get_i32(key) {
        return v as f64;
    }
    0.0
}

fn extract_i64(doc: &Document, key: &str) -> i64 {
    if let Ok(v) = doc.get_i64(key) {
        return v;
    }
    if let Ok(v) = doc.get_i32(key) {
        return v as i64;
    }
    if let Ok(v) = doc.get_f64(key) {
        return v as i64;
    }
    0
}

/// GET /api/reports/summary
pub async fn get_report_summary(
    Query(query): Query<ReportsQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let settings_coll = state.db.collection::<Document>("settings");
    let tx_coll = state.db.collection::<Document>("transactions");

    let setting = settings_coll
        .find_one(doc! { "_id": "main_config" })
        .await
        .unwrap_or(None);

    let initial_balance = setting
        .as_ref()
        .map(|s| extract_f64(s, "initialBalance"))
        .unwrap_or(4752300.0);

    let active_period = query.period.unwrap_or_else(|| {
        setting
            .as_ref()
            .and_then(|s| s.get_str("activePeriod").ok().map(|s| s.to_string()))
            .unwrap_or_else(|| "2026-10".to_string())
    });

    // 1. Agregasi Total Arus Kas Keseluruhan
    let total_pipeline = vec![doc! {
        "$group": {
            "_id": "$type",
            "total": { "$sum": "$amount" },
            "count": { "$sum": 1 }
        }
    }];

    let mut total_income = 0.0;
    let mut total_expense = 0.0;

    if let Ok(mut cursor) = tx_coll.aggregate(total_pipeline).await {
        while let Ok(Some(item)) = cursor.try_next().await {
            let t_type = item.get_str("_id").unwrap_or("");
            let sum_val = extract_f64(&item, "total");

            if t_type == "in" {
                total_income = sum_val;
            } else if t_type == "out" {
                total_expense = sum_val;
            }
        }
    }

    let final_balance = initial_balance + total_income - total_expense;

    // 2. Agregasi Arus Kas Bulan yang Dipilih (Active Period)
    let month_pipeline = vec![
        doc! { "$match": { "date": { "$regex": format!("^{}", active_period) } } },
        doc! {
            "$group": {
                "_id": "$type",
                "total": { "$sum": "$amount" }
            }
        },
    ];

    let mut month_income = 0.0;
    let mut month_expense = 0.0;

    if let Ok(mut cursor) = tx_coll.aggregate(month_pipeline).await {
        while let Ok(Some(item)) = cursor.try_next().await {
            let t_type = item.get_str("_id").unwrap_or("");
            let sum_val = extract_f64(&item, "total");

            if t_type == "in" {
                month_income = sum_val;
            } else if t_type == "out" {
                month_expense = sum_val;
            }
        }
    }

    // 3. Agregasi Kategori untuk Segmen Donut Chart
    let cat_pipeline = vec![
        doc! {
            "$group": {
                "_id": { "type": "$type", "category": "$category" },
                "total": { "$sum": "$amount" },
                "count": { "$sum": 1 }
            }
        },
        doc! {
            "$project": {
                "_id": 0,
                "type": "$_id.type",
                "category": "$_id.category",
                "total": "$total",
                "count": "$count"
            }
        },
        doc! { "$sort": { "total": -1 } },
    ];

    let mut category_breakdown = Vec::new();
    if let Ok(mut cursor) = tx_coll.aggregate(cat_pipeline).await {
        while let Ok(Some(item)) = cursor.try_next().await {
            let t_type = item.get_str("type").unwrap_or("").to_string();
            let cat_name = item.get_str("category").unwrap_or("").to_string();
            let total_val = extract_f64(&item, "total");
            let count_val = extract_i64(&item, "count");

            category_breakdown.push(json!({
                "type": t_type,
                "category": cat_name,
                "total": total_val,
                "count": count_val
            }));
        }
    }

    let surplus_raw = total_income - total_expense;
    let surplus = if surplus_raw > 0.0 { surplus_raw } else { 0.0 };
    let flow_sum = total_income + total_expense;
    let income_pct = if flow_sum > 0.0 {
        ((total_income / flow_sum) * 100.0).round() as i64
    } else {
        0
    };
    let expense_pct = if flow_sum > 0.0 {
        ((total_expense / flow_sum) * 100.0).round() as i64
    } else {
        0
    };

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "Berhasil mengambil ringkasan laporan kas OSIS",
            "period": active_period,
            "data": {
                "initialBalance": initial_balance,
                "totalIncome": total_income,
                "totalExpense": total_expense,
                "finalBalance": final_balance,
                "monthIncome": month_income,
                "monthExpense": month_expense,
                "surplus": surplus,
                "ratios": {
                    "incomePercentage": income_pct,
                    "expensePercentage": expense_pct
                },
                "categoryBreakdown": category_breakdown
            }
        })),
    )
}
