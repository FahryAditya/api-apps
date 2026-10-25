use chrono::{Local, Utc};
use mongodb::bson::{self, Bson, Document};
use serde_json::Value;

/// Format tanggal hari ini dalam format YYYY-MM-DD
pub fn get_today_date() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Format jam saat ini dalam format hh:mm AM/PM
pub fn get_current_time() -> String {
    Local::now().format("%I:%M %p").to_string()
}

/// Timestamp ISO 8601 UTC
pub fn get_iso_timestamp() -> String {
    Utc::now().to_rfc3339()
}

/// Helper Title Case untuk sanitasi nama (cth: "bambang pamungkas" -> "Bambang Pamungkas")
pub fn to_title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str().to_lowercase().as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Konversi BSON Document ke serde_json::Value
pub fn doc_to_json(doc: Document) -> Value {
    match bson::from_bson::<Value>(Bson::Document(doc)) {
        Ok(v) => clean_bson_json(v),
        Err(_) => serde_json::json!({}),
    }
}

/// Merapikan objek format BSON ($oid, $date) menjadi JSON standar yang ramah frontend
pub fn clean_bson_json(val: Value) -> Value {
    match val {
        Value::Object(map) => {
            if map.len() == 1 {
                if let Some(Value::String(oid)) = map.get("$oid") {
                    return Value::String(oid.clone());
                }
                if let Some(Value::String(date_str)) = map.get("$date") {
                    return Value::String(date_str.clone());
                }
                if let Some(Value::Object(date_map)) = map.get("$date") {
                    if let Some(Value::String(ms)) = date_map.get("$numberLong") {
                        return Value::String(ms.clone());
                    }
                }
            }

            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                new_map.insert(k, clean_bson_json(v));
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(clean_bson_json).collect()),
        other => other,
    }
}

/// Konversi serde_json::Value ke BSON Document
#[allow(dead_code)]
pub fn json_to_doc(val: Value) -> Option<Document> {
    match bson::to_bson(&val) {
        Ok(Bson::Document(doc)) => Some(doc),
        _ => None,
    }
}
