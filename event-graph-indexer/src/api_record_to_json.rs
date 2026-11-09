use ledger_api::com::daml::ledger::api::v2::{Record, Value};
use serde_json::json;

pub fn api_record_to_json(record: &Record) -> serde_json::Value {
    let fields_json = record
        .fields
        .iter()
        .map(|field| {
            let value_json = match &field.value {
                Some(val) => api_value_to_json(val),
                None => json!("Couldn't convert"),
            };
            (field.label.clone(), value_json)
        })
        .collect::<serde_json::Map<_, _>>();
    serde_json::Value::Object(fields_json)
}

fn api_value_to_json(value: &Value) -> serde_json::Value {
    match &value.sum {
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::Text(s)) => json!(s),
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::Int64(i)) => json!(i),
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::Bool(b)) => json!(b),
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::Numeric(n)) => json!(n),
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::Party(p)) => json!(p),
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::ContractId(cid)) => json!(cid),
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::Record(rec)) => {
            api_record_to_json(rec)
        }
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::Optional(opt)) => {
            match &opt.value {
                Some(inner) => api_value_to_json(inner),
                None => serde_json::Value::Null, // Use null for missing optionals
            }
        }
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::List(list)) => {
            let items: Vec<_> = list.elements.iter().map(api_value_to_json).collect();
            serde_json::Value::Array(items)
        }
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::TextMap(text_map)) => {
            let map: serde_json::Map<String, serde_json::Value> = text_map
                .entries
                .iter()
                .map(|entry| {
                    let value_json = match &entry.value {
                        Some(val) => api_value_to_json(val),
                        None => json!("Couldn't convert"),
                    };
                    (entry.key.clone(), value_json)
                })
                .collect();
            serde_json::Value::Object(map)
        }
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::GenMap(gen_map)) => {
            let arr: Vec<_> = gen_map
                .entries
                .iter()
                .map(|entry| {
                    json!({
                        "key": match &entry.key {
                            Some(key_val) => api_value_to_json(key_val),
                            None => json!("Couldn't convert"),
                        },
                        "value": match &entry.value {
                            Some(val) => api_value_to_json(val),
                            None => json!("Couldn't convert"),
                        }
                    })
                })
                .collect();
            serde_json::Value::Array(arr)
        }
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::Variant(variant)) => {
            json!({
                "constructor": &variant.constructor,
                "value": variant.value.as_ref().map(|v| api_value_to_json(v)).unwrap_or(json!("Couldn't convert"))
            })
        }
        Some(ledger_api::com::daml::ledger::api::v2::value::Sum::Enum(enum_val)) => {
            json!({
                "constructor": &enum_val.constructor
            })
        }
        _ => json!("Couldn't convert"),
    }
}

pub fn choice_argument_json(choice_argument: &Option<ledger_api::v2::Value>) -> serde_json::Value {
    match choice_argument {
        Some(value) => {
            if let Some(ledger_api::v2::value::Sum::Record(record)) = &value.sum {
                api_record_to_json(record)
            } else {
                json!("Couldn't convert")
            }
        }
        None => json!("Couldn't convert"),
    }
}

/// Flattens a DAML Record into dot-separated property pairs for Neo4j node properties.
///
/// Example: `Record { person: Record { name: "Alice", age: 30 } }` with prefix `"create_arg."`
/// produces: `[("create_arg.person.name", json!("Alice")), ("create_arg.person.age", json!(30))]`
pub fn flatten_record_to_properties(
    record: &Record,
    prefix: &str,
    max_depth: usize,
) -> Vec<(String, serde_json::Value)> {
    let mut result = Vec::new();
    for field in &record.fields {
        let key = format!("{}{}", prefix, field.label);
        if let Some(val) = &field.value {
            flatten_value_inner(val, &key, max_depth, 0, &mut result);
        }
    }
    result
}

/// Flattens a DAML Value into dot-separated property pairs.
/// For exercised events where `choice_argument` is a `Value` (not always a `Record`).
pub fn flatten_value_to_properties(
    value: &Value,
    prefix: &str,
    max_depth: usize,
) -> Vec<(String, serde_json::Value)> {
    let mut result = Vec::new();
    if let Some(ledger_api::com::daml::ledger::api::v2::value::Sum::Record(record)) = &value.sum {
        for field in &record.fields {
            let key = format!("{}{}", prefix, field.label);
            if let Some(val) = &field.value {
                flatten_value_inner(val, &key, max_depth, 0, &mut result);
            }
        }
    } else {
        // Non-record value: emit as single property at prefix (strip trailing dot)
        let key = prefix.trim_end_matches('.');
        flatten_value_inner(value, key, max_depth, 0, &mut result);
    }
    result
}

fn flatten_value_inner(
    value: &Value,
    key: &str,
    max_depth: usize,
    depth: usize,
    result: &mut Vec<(String, serde_json::Value)>,
) {
    use ledger_api::com::daml::ledger::api::v2::value::Sum;

    if depth >= max_depth {
        result.push((key.to_string(), api_value_to_json(value)));
        return;
    }

    match &value.sum {
        Some(Sum::Text(s)) => result.push((key.to_string(), json!(s))),
        Some(Sum::Int64(i)) => result.push((key.to_string(), json!(i))),
        Some(Sum::Bool(b)) => result.push((key.to_string(), json!(b))),
        Some(Sum::Numeric(n)) => result.push((key.to_string(), json!(n))),
        Some(Sum::Party(p)) => result.push((key.to_string(), json!(p))),
        Some(Sum::ContractId(c)) => result.push((key.to_string(), json!(c))),
        Some(Sum::Date(d)) => result.push((key.to_string(), json!(d))),
        Some(Sum::Timestamp(t)) => result.push((key.to_string(), json!(t))),
        Some(Sum::Record(rec)) => {
            for field in &rec.fields {
                let nested_key = format!("{}.{}", key, field.label);
                if let Some(val) = &field.value {
                    flatten_value_inner(val, &nested_key, max_depth, depth + 1, result);
                }
            }
        }
        Some(Sum::Optional(opt)) => match &opt.value {
            Some(inner) => flatten_value_inner(inner.as_ref(), key, max_depth, depth, result),
            None => result.push((key.to_string(), serde_json::Value::Null)),
        },
        Some(Sum::Variant(variant)) => {
            result.push((key.to_string(), json!(variant.constructor)));
            if let Some(inner) = &variant.value {
                let value_key = format!("{}.value", key);
                // Check if Unit — skip if so
                if inner.sum.is_some() && !matches!(&inner.sum, Some(Sum::Unit(_))) {
                    flatten_value_inner(inner.as_ref(), &value_key, max_depth, depth + 1, result);
                }
            }
        }
        Some(Sum::Enum(e)) => result.push((key.to_string(), json!(e.constructor))),
        Some(Sum::Unit(_)) => {} // skip
        // List, TextMap, GenMap — emit as JSON strings
        Some(Sum::List(_)) | Some(Sum::TextMap(_)) | Some(Sum::GenMap(_)) => {
            let json_str = serde_json::to_string(&api_value_to_json(value))
                .unwrap_or_else(|_| "null".to_string());
            result.push((key.to_string(), json!(json_str)));
        }
        _ => {} // unknown variants — skip
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_api::com::daml::ledger::api::v2::{
        Optional, Record, RecordField, Value, Variant, value::Sum,
    };

    fn text_value(s: &str) -> Option<Value> {
        Some(Value {
            sum: Some(Sum::Text(s.to_string())),
        })
    }

    fn int_value(i: i64) -> Option<Value> {
        Some(Value {
            sum: Some(Sum::Int64(i)),
        })
    }

    fn record_value(fields: Vec<RecordField>) -> Option<Value> {
        Some(Value {
            sum: Some(Sum::Record(Record {
                record_id: None,
                fields,
            })),
        })
    }

    fn field(label: &str, value: Option<Value>) -> RecordField {
        RecordField {
            label: label.to_string(),
            value,
        }
    }

    #[test]
    fn test_flatten_simple_record() {
        let record = Record {
            record_id: None,
            fields: vec![
                field("name", text_value("Alice")),
                field("age", int_value(30)),
            ],
        };

        let props = flatten_record_to_properties(&record, "create_arg.", 10);
        assert_eq!(props.len(), 2);
        assert_eq!(props[0], ("create_arg.name".to_string(), json!("Alice")));
        assert_eq!(props[1], ("create_arg.age".to_string(), json!(30)));
    }

    #[test]
    fn test_flatten_nested_record() {
        let address = record_value(vec![
            field("street", text_value("123 Main St")),
            field("city", text_value("Zurich")),
        ]);
        let record = Record {
            record_id: None,
            fields: vec![
                field("name", text_value("Alice")),
                field("address", address),
            ],
        };

        let props = flatten_record_to_properties(&record, "create_arg.", 10);
        assert_eq!(props.len(), 3);
        assert_eq!(props[0], ("create_arg.name".to_string(), json!("Alice")));
        assert_eq!(
            props[1],
            (
                "create_arg.address.street".to_string(),
                json!("123 Main St")
            )
        );
        assert_eq!(
            props[2],
            ("create_arg.address.city".to_string(), json!("Zurich"))
        );
    }

    #[test]
    fn test_flatten_optional_some_and_none() {
        let record = Record {
            record_id: None,
            fields: vec![
                field(
                    "present",
                    Some(Value {
                        sum: Some(Sum::Optional(Box::new(Optional {
                            value: Some(Box::new(Value {
                                sum: Some(Sum::Text("hello".to_string())),
                            })),
                        }))),
                    }),
                ),
                field(
                    "absent",
                    Some(Value {
                        sum: Some(Sum::Optional(Box::new(Optional { value: None }))),
                    }),
                ),
            ],
        };

        let props = flatten_record_to_properties(&record, "create_arg.", 10);
        assert_eq!(props.len(), 2);
        assert_eq!(props[0], ("create_arg.present".to_string(), json!("hello")));
        assert_eq!(
            props[1],
            ("create_arg.absent".to_string(), serde_json::Value::Null)
        );
    }

    #[test]
    fn test_flatten_variant() {
        let record = Record {
            record_id: None,
            fields: vec![field(
                "shape",
                Some(Value {
                    sum: Some(Sum::Variant(Box::new(Variant {
                        variant_id: None,
                        constructor: "Circle".to_string(),
                        value: Some(Box::new(Value {
                            sum: Some(Sum::Numeric("3.14".to_string())),
                        })),
                    }))),
                }),
            )],
        };

        let props = flatten_record_to_properties(&record, "create_arg.", 10);
        assert_eq!(props.len(), 2);
        assert_eq!(props[0], ("create_arg.shape".to_string(), json!("Circle")));
        assert_eq!(
            props[1],
            ("create_arg.shape.value".to_string(), json!("3.14"))
        );
    }

    #[test]
    fn test_flatten_depth_limit() {
        let deeply_nested = record_value(vec![field("inner", text_value("deep"))]);
        let record = Record {
            record_id: None,
            fields: vec![field("outer", deeply_nested)],
        };

        // max_depth = 0 means no recursion — emit as JSON at top level
        let props = flatten_record_to_properties(&record, "create_arg.", 0);
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].0, "create_arg.outer");
        // Should be the JSON representation of the nested record
        assert!(props[0].1.is_object());
    }

    #[test]
    fn test_flatten_value_non_record() {
        let value = Value {
            sum: Some(Sum::Text("hello".to_string())),
        };
        let props = flatten_value_to_properties(&value, "choice_arg.", 10);
        assert_eq!(props.len(), 1);
        assert_eq!(props[0], ("choice_arg".to_string(), json!("hello")));
    }

    #[test]
    fn test_flatten_enum() {
        let record = Record {
            record_id: None,
            fields: vec![field(
                "color",
                Some(Value {
                    sum: Some(Sum::Enum(ledger_api::com::daml::ledger::api::v2::Enum {
                        enum_id: None,
                        constructor: "Red".to_string(),
                    })),
                }),
            )],
        };

        let props = flatten_record_to_properties(&record, "create_arg.", 10);
        assert_eq!(props.len(), 1);
        assert_eq!(props[0], ("create_arg.color".to_string(), json!("Red")));
    }
}
