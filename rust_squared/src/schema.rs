use serde_json::Value;

pub trait RsqSchema {
    fn schema_name() -> &'static str;
    fn schema() -> Value;
    fn description() -> Option<&'static str> { None }
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

impl RsqSchema for u32 {
    fn schema_name() -> &'static str { "u32" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "uint32" })
    }
}

impl RsqSchema for i32 {
    fn schema_name() -> &'static str { "i32" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "int32" })
    }
}

impl RsqSchema for f32 {
    fn schema_name() -> &'static str { "f32" }
    fn schema() -> Value {
        serde_json::json!({ "type": "number", "format": "float" })
    }
}

impl RsqSchema for u16 {
    fn schema_name() -> &'static str { "u16" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "uint16" })
    }
}

impl RsqSchema for i16 {
    fn schema_name() -> &'static str { "i16" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "int16" })
    }
}

impl RsqSchema for u8 {
    fn schema_name() -> &'static str { "u8" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "uint8" })
    }
}

impl RsqSchema for i8 {
    fn schema_name() -> &'static str { "i8" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "int8" })
    }
}

impl<T: RsqSchema> RsqSchema for Vec<T> {
    fn schema_name() -> &'static str { "Vec" }
    fn schema() -> Value {
        serde_json::json!({ "type": "array", "items": T::schema() })
    }
}

impl<T: RsqSchema> RsqSchema for Option<T> {
    fn schema_name() -> &'static str { "Option" }
    fn schema() -> Value {
        let mut s = T::schema();
        if let Some(obj) = s.as_object_mut() {
            obj.insert("nullable".to_string(), serde_json::json!(true));
        }
        s
    }
}

impl<T: RsqSchema> RsqSchema for std::collections::HashMap<String, T> {
    fn schema_name() -> &'static str { "HashMap" }
    fn schema() -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": T::schema()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_schema() {
        assert_eq!(String::schema(), serde_json::json!({"type": "string"}));
    }

    #[test]
    fn u32_schema() {
        assert_eq!(u32::schema(), serde_json::json!({"type": "integer", "format": "uint32"}));
    }

    #[test]
    fn i32_schema() {
        assert_eq!(i32::schema(), serde_json::json!({"type": "integer", "format": "int32"}));
    }

    #[test]
    fn f32_schema() {
        assert_eq!(f32::schema(), serde_json::json!({"type": "number", "format": "float"}));
    }

    #[test]
    fn vec_schema() {
        assert_eq!(<Vec<String>>::schema(), serde_json::json!({"type": "array", "items": {"type": "string"}}));
    }

    #[test]
    fn option_schema() {
        let s = <Option<i64>>::schema();
        assert_eq!(s["type"], "integer");
        assert_eq!(s["nullable"], true);
    }

    #[test]
    fn hashmap_schema() {
        use std::collections::HashMap;
        let s = <HashMap<String, bool>>::schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["additionalProperties"]["type"], "boolean");
    }
}
