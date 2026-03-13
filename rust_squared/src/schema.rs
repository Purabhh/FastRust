use serde_json::Value;

pub trait RsqSchema {
    fn schema_name() -> &'static str;
    fn schema() -> Value;
}

impl RsqSchema for String {
    fn schema_name() -> &'static str {
        "String"
    }

    fn schema() -> Value {
        serde_json::json!({ "type": "string" })
    }
}

impl RsqSchema for bool {
    fn schema_name() -> &'static str {
        "bool"
    }

    fn schema() -> Value {
        serde_json::json!({ "type": "boolean" })
    }
}

impl RsqSchema for u64 {
    fn schema_name() -> &'static str {
        "u64"
    }

    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "uint64" })
    }
}

impl RsqSchema for i64 {
    fn schema_name() -> &'static str {
        "i64"
    }

    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "int64" })
    }
}

impl RsqSchema for f64 {
    fn schema_name() -> &'static str {
        "f64"
    }

    fn schema() -> Value {
        serde_json::json!({ "type": "number", "format": "double" })
    }
}
