//! Converts Ledger API (Daml-LF) values into SurrealDB values, keeping their
//! natural nested shape.
//!
//! Records become objects, lists become arrays, text maps become objects,
//! timestamps become datetimes and numerics become decimals, so a field like
//! `create_arguments.person.address.city` can be queried directly.

use ledger_api::v2::{Record, Value as DamlValue, value::Sum};
use surrealdb::types::{Array, Datetime, Number, Object, Value};

/// Converts a Daml record into a SurrealDB object (`{ field: value, ... }`).
pub fn record_to_value(record: &Record) -> Value {
    let mut obj = Object::new();
    for field in &record.fields {
        let v = field
            .value
            .as_ref()
            .map(daml_to_value)
            .unwrap_or(Value::Null);
        obj.insert(field.label.clone(), v);
    }
    Value::Object(obj)
}

/// Converts any Daml value into a SurrealDB value.
///
/// | Daml                         | SurrealDB                          |
/// |------------------------------|------------------------------------|
/// | Record                       | object                             |
/// | List                         | array                              |
/// | TextMap                      | object                             |
/// | GenMap                       | array of `{ key, value }`          |
/// | Variant                      | `{ constructor, value }`           |
/// | Enum                         | string (the constructor)           |
/// | Optional                     | the inner value, or `null`         |
/// | Timestamp                    | datetime                           |
/// | Date                         | datetime at 00:00 UTC              |
/// | Numeric                      | decimal (string if it can't parse) |
/// | Int64 / Bool                 | int / bool                         |
/// | Text / Party / ContractId    | string                             |
/// | Unit                         | `{}`                               |
pub fn daml_to_value(value: &DamlValue) -> Value {
    match &value.sum {
        None => Value::Null,
        Some(Sum::Unit(_)) => Value::Object(Object::new()),
        Some(Sum::Bool(b)) => Value::Bool(*b),
        Some(Sum::Int64(i)) => Value::Number(Number::Int(*i)),
        Some(Sum::Numeric(n)) => numeric_to_value(n),
        Some(Sum::Text(s)) | Some(Sum::Party(s)) | Some(Sum::ContractId(s)) => {
            Value::String(s.clone())
        }
        Some(Sum::Timestamp(micros)) => micros_to_datetime(*micros)
            .map(Value::Datetime)
            .unwrap_or(Value::Null),
        Some(Sum::Date(days)) => Datetime::from_timestamp(i64::from(*days) * 86_400, 0)
            .map(Value::Datetime)
            .unwrap_or(Value::Null),
        Some(Sum::Record(rec)) => record_to_value(rec),
        Some(Sum::Optional(opt)) => opt
            .value
            .as_deref()
            .map(daml_to_value)
            .unwrap_or(Value::Null),
        Some(Sum::List(list)) => Value::Array(list.elements.iter().map(daml_to_value).collect()),
        Some(Sum::TextMap(map)) => {
            let mut obj = Object::new();
            for entry in &map.entries {
                let v = entry
                    .value
                    .as_ref()
                    .map(daml_to_value)
                    .unwrap_or(Value::Null);
                obj.insert(entry.key.clone(), v);
            }
            Value::Object(obj)
        }
        Some(Sum::GenMap(map)) => {
            let entries: Array = map
                .entries
                .iter()
                .map(|entry| {
                    let mut obj = Object::new();
                    obj.insert(
                        "key",
                        entry.key.as_ref().map(daml_to_value).unwrap_or(Value::Null),
                    );
                    obj.insert(
                        "value",
                        entry
                            .value
                            .as_ref()
                            .map(daml_to_value)
                            .unwrap_or(Value::Null),
                    );
                    Value::Object(obj)
                })
                .collect();
            Value::Array(entries)
        }
        Some(Sum::Variant(variant)) => {
            let mut obj = Object::new();
            obj.insert("constructor", variant.constructor.clone());
            obj.insert(
                "value",
                variant
                    .value
                    .as_deref()
                    .map(daml_to_value)
                    .unwrap_or(Value::Null),
            );
            Value::Object(obj)
        }
        Some(Sum::Enum(e)) => Value::String(e.constructor.clone()),
    }
}

/// Daml `Numeric` arrives as a decimal string. Store it as a decimal so it can
/// be compared and summed; fall back to the string if it doesn't fit.
fn numeric_to_value(n: &str) -> Value {
    match n.parse::<rust_decimal::Decimal>() {
        Ok(d) => Value::Number(Number::Decimal(d)),
        Err(_) => Value::String(n.to_string()),
    }
}

/// Daml timestamps are microseconds since the Unix epoch.
pub fn micros_to_datetime(micros: i64) -> Option<Datetime> {
    let seconds = micros.div_euclid(1_000_000);
    let nanos = (micros.rem_euclid(1_000_000) * 1_000) as u32;
    Datetime::from_timestamp(seconds, nanos)
}

/// Converts a protobuf timestamp into a SurrealDB datetime.
pub fn timestamp_to_datetime(ts: &prost_types::Timestamp) -> Option<Datetime> {
    Datetime::from_timestamp(ts.seconds, u32::try_from(ts.nanos).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_api::v2::{Enum, GenMap, List, Optional, RecordField, TextMap, Variant};

    fn v(sum: Sum) -> DamlValue {
        DamlValue { sum: Some(sum) }
    }

    fn field(label: &str, value: DamlValue) -> RecordField {
        RecordField {
            label: label.to_string(),
            value: Some(value),
        }
    }

    fn rec(fields: Vec<RecordField>) -> Record {
        Record {
            record_id: None,
            fields,
        }
    }

    fn obj(value: &Value) -> &Object {
        match value {
            Value::Object(o) => o,
            other => panic!("expected object, got {other:?}"),
        }
    }

    #[test]
    fn nested_record_stays_nested() {
        let address = rec(vec![
            field("street", v(Sum::Text("123 Main St".into()))),
            field("city", v(Sum::Text("Zurich".into()))),
        ]);
        let person = rec(vec![
            field("name", v(Sum::Text("Alice".into()))),
            field("age", v(Sum::Int64(30))),
            field("address", v(Sum::Record(address))),
        ]);
        let value = record_to_value(&person);
        let o = obj(&value);
        assert_eq!(o.get("name"), Some(&Value::String("Alice".into())));
        assert_eq!(o.get("age"), Some(&Value::Number(Number::Int(30))));
        let addr = obj(o.get("address").unwrap());
        assert_eq!(addr.get("city"), Some(&Value::String("Zurich".into())));
    }

    #[test]
    fn lists_and_maps_are_real_arrays_and_objects() {
        let list = v(Sum::List(List {
            elements: vec![v(Sum::Int64(1)), v(Sum::Int64(2))],
        }));
        assert!(matches!(daml_to_value(&list), Value::Array(a) if a.len() == 2));

        let text_map = v(Sum::TextMap(TextMap {
            entries: vec![ledger_api::v2::text_map::Entry {
                key: "k".into(),
                value: Some(v(Sum::Bool(true))),
            }],
        }));
        let tm = daml_to_value(&text_map);
        assert_eq!(obj(&tm).get("k"), Some(&Value::Bool(true)));

        let gen_map = v(Sum::GenMap(GenMap {
            entries: vec![ledger_api::v2::gen_map::Entry {
                key: Some(v(Sum::Int64(7))),
                value: Some(v(Sum::Text("seven".into()))),
            }],
        }));
        match daml_to_value(&gen_map) {
            Value::Array(a) => {
                let e = obj(&a[0]);
                assert_eq!(e.get("key"), Some(&Value::Number(Number::Int(7))));
                assert_eq!(e.get("value"), Some(&Value::String("seven".into())));
            }
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn variant_enum_optional() {
        let variant = v(Sum::Variant(Box::new(Variant {
            variant_id: None,
            constructor: "Circle".into(),
            value: Some(Box::new(v(Sum::Numeric("3.14".into())))),
        })));
        let vv = daml_to_value(&variant);
        let o = obj(&vv);
        assert_eq!(o.get("constructor"), Some(&Value::String("Circle".into())));
        assert!(matches!(
            o.get("value"),
            Some(Value::Number(Number::Decimal(_)))
        ));

        let e = v(Sum::Enum(Enum {
            enum_id: None,
            constructor: "Red".into(),
        }));
        assert_eq!(daml_to_value(&e), Value::String("Red".into()));

        let none = v(Sum::Optional(Box::new(Optional { value: None })));
        assert_eq!(daml_to_value(&none), Value::Null);
        let some = v(Sum::Optional(Box::new(Optional {
            value: Some(Box::new(v(Sum::Text("x".into())))),
        })));
        assert_eq!(daml_to_value(&some), Value::String("x".into()));
    }

    #[test]
    fn timestamps_and_dates_become_datetimes() {
        // 2024-01-01T00:00:00.5Z in micros
        let ts = v(Sum::Timestamp(1_704_067_200_500_000));
        match daml_to_value(&ts) {
            Value::Datetime(d) => assert_eq!(d.to_rfc3339(), "2024-01-01T00:00:00.500+00:00"),
            other => panic!("expected datetime, got {other:?}"),
        }
        // 19723 days after epoch = 2024-01-01
        let date = v(Sum::Date(19_723));
        match daml_to_value(&date) {
            Value::Datetime(d) => assert_eq!(d.timestamp(), 1_704_067_200),
            other => panic!("expected datetime, got {other:?}"),
        }
    }

    #[test]
    fn oversized_numeric_falls_back_to_string() {
        let big = "12345678901234567890123456789012345.1234567890";
        assert_eq!(
            daml_to_value(&v(Sum::Numeric(big.into()))),
            Value::String(big.into())
        );
    }
}
