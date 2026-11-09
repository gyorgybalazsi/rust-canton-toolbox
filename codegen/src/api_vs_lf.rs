use crate::lf_protobuf::com::daml::daml_lf_2::{
    self, BuiltinLit, Expr, FieldWithExpr, FieldWithType, Type, builtin_lit, expr,
};
use ledger_api::v2::{Record as ApiRecord, RecordField as ApiRecordField, Value as ApiValue};
use std::collections::HashMap;

/// Converts an API Record to a lf_protobuf Record (Vec<FieldWithExpr>)
pub fn api_record_to_lf_record(
    api_record: &ApiRecord,
    field_types: &[FieldWithType],
    string_to_interned: &HashMap<String, i32>,
) -> Vec<FieldWithExpr> {
    api_record
        .fields
        .iter()
        .zip(field_types.iter())
        .map(|(api_field, field_type)| FieldWithExpr {
            field_interned_str: field_type.field_interned_str,
            expr: api_value_to_lf_expr(
                api_field.value.as_ref(),
                field_type.r#type.as_ref(),
                string_to_interned,
            ),
        })
        .collect()
}

/// Converts an API Value to a lf_protobuf Expr
fn api_value_to_lf_expr(
    api_value: Option<&ApiValue>,
    field_type: Option<&Type>,
    string_to_interned: &HashMap<String, i32>,
) -> Option<Expr> {
    match api_value {
        Some(val) => {
            match &val.sum {
                Some(ledger_api::v2::value::Sum::Text(s)) => {
                    let idx = string_to_interned.get(s).cloned().unwrap_or(0);
                    Some(Expr {
                        location: None,
                        sum: Some(expr::Sum::BuiltinLit(BuiltinLit {
                            sum: Some(builtin_lit::Sum::TextInternedStr(idx)),
                        })),
                    })
                }
                Some(ledger_api::v2::value::Sum::Int64(i)) => Some(Expr {
                    location: None,
                    sum: Some(expr::Sum::BuiltinLit(BuiltinLit {
                        sum: Some(builtin_lit::Sum::Int64(*i)),
                    })),
                }),
                Some(ledger_api::v2::value::Sum::Bool(b)) => {
                    let con = if *b {
                        daml_lf_2::BuiltinCon::ConTrue as i32
                    } else {
                        daml_lf_2::BuiltinCon::ConFalse as i32
                    };
                    Some(Expr {
                        location: None,
                        sum: Some(expr::Sum::BuiltinCon(con)),
                    })
                }
                Some(ledger_api::v2::value::Sum::Numeric(n)) => {
                    let idx = string_to_interned.get(n).cloned().unwrap_or(0);
                    Some(Expr {
                        location: None,
                        sum: Some(expr::Sum::BuiltinLit(BuiltinLit {
                            sum: Some(builtin_lit::Sum::NumericInternedStr(idx)),
                        })),
                    })
                }
                Some(ledger_api::v2::value::Sum::Party(p)) => {
                    let idx = string_to_interned.get(p).cloned().unwrap_or(0);
                    Some(Expr {
                        location: None,
                        sum: Some(expr::Sum::BuiltinLit(BuiltinLit {
                            sum: Some(builtin_lit::Sum::TextInternedStr(idx)),
                        })),
                    })
                }
                Some(ledger_api::v2::value::Sum::ContractId(cid)) => {
                    let idx = string_to_interned.get(cid).cloned().unwrap_or(0);
                    Some(Expr {
                        location: None,
                        sum: Some(expr::Sum::BuiltinLit(BuiltinLit {
                            sum: Some(builtin_lit::Sum::TextInternedStr(idx)),
                        })),
                    })
                }
                Some(ledger_api::v2::value::Sum::Record(rec)) => {
                    // Recursively convert fields
                    let fields = rec
                        .fields
                        .iter()
                        .map(|f| {
                            let idx = string_to_interned.get(&f.label).cloned().unwrap_or(0);
                            FieldWithExpr {
                                field_interned_str: idx,
                                expr: api_value_to_lf_expr(
                                    f.value.as_ref(),
                                    None,
                                    string_to_interned,
                                ),
                            }
                        })
                        .collect();
                    Some(Expr {
                        location: None,
                        sum: Some(expr::Sum::RecCon(expr::RecCon {
                            tycon: None,
                            fields,
                        })),
                    })
                }
                Some(ledger_api::v2::value::Sum::Optional(opt)) => match &opt.value {
                    Some(inner) => Some(Expr {
                        location: None,
                        sum: Some(expr::Sum::OptionalSome(Box::new(expr::OptionalSome {
                            r#type: field_type.cloned(),
                            value: api_value_to_lf_expr(
                                Some(inner),
                                field_type,
                                string_to_interned,
                            )
                            .map(Box::new),
                        }))),
                    }),
                    None => Some(Expr {
                        location: None,
                        sum: Some(expr::Sum::OptionalNone(expr::OptionalNone {
                            r#type: field_type.cloned(),
                        })),
                    }),
                },
                Some(ledger_api::v2::value::Sum::List(list)) => {
                    let elements: Vec<Expr> = list
                        .elements
                        .iter()
                        .filter_map(|v| {
                            api_value_to_lf_expr(Some(v), field_type, string_to_interned)
                        })
                        .collect();
                    Some(Expr {
                        location: None,
                        sum: Some(expr::Sum::Cons(Box::new(expr::Cons {
                            r#type: field_type.cloned(),
                            front: elements,
                            tail: None,
                        }))),
                    })
                }
                // Add handling for TextMap, GenMap, Variant, Enum, etc. as needed
                _ => None,
            }
        }
        None => None,
    }
}

/// Converts a lf_protobuf Record (Vec<FieldWithExpr>) to an API Record
pub fn lf_record_to_api_record(
    lf_proto_fields: &[FieldWithExpr],
    interned_strings: &[String],
) -> ApiRecord {
    ApiRecord {
        record_id: None,
        fields: lf_proto_fields
            .iter()
            .map(|field| ApiRecordField {
                label: interned_strings
                    .get(field.field_interned_str as usize)
                    .cloned()
                    .unwrap_or_default(),
                value: field
                    .expr
                    .as_ref()
                    .map(|e| lf_expr_to_api_value(e, interned_strings)),
            })
            .collect(),
    }
}

/// Converts a lf_protobuf Expr to an API Value
fn lf_expr_to_api_value(expr: &Expr, interned_strings: &[String]) -> ApiValue {
    match &expr.sum {
        Some(expr::Sum::BuiltinLit(lit)) => match &lit.sum {
            Some(builtin_lit::Sum::Int64(i)) => ApiValue {
                sum: Some(ledger_api::v2::value::Sum::Int64(*i)),
            },
            Some(builtin_lit::Sum::TextInternedStr(idx)) => {
                let s = interned_strings
                    .get(*idx as usize)
                    .cloned()
                    .unwrap_or_default();
                ApiValue {
                    sum: Some(ledger_api::v2::value::Sum::Text(s)),
                }
            }
            Some(builtin_lit::Sum::NumericInternedStr(idx)) => {
                let n = interned_strings
                    .get(*idx as usize)
                    .cloned()
                    .unwrap_or_default();
                ApiValue {
                    sum: Some(ledger_api::v2::value::Sum::Numeric(n)),
                }
            }
            _ => ApiValue { sum: None },
        },
        Some(expr::Sum::BuiltinCon(con)) => match *con {
            x if x == daml_lf_2::BuiltinCon::ConTrue as i32 => ApiValue {
                sum: Some(ledger_api::v2::value::Sum::Bool(true)),
            },
            x if x == daml_lf_2::BuiltinCon::ConFalse as i32 => ApiValue {
                sum: Some(ledger_api::v2::value::Sum::Bool(false)),
            },
            _ => ApiValue { sum: None },
        },
        Some(expr::Sum::RecCon(rec_con)) => {
            let fields = rec_con
                .fields
                .iter()
                .map(|f| ApiRecordField {
                    label: interned_strings
                        .get(f.field_interned_str as usize)
                        .cloned()
                        .unwrap_or_default(),
                    value: f
                        .expr
                        .as_ref()
                        .map(|e| lf_expr_to_api_value(e, interned_strings)),
                })
                .collect();
            ApiValue {
                sum: Some(ledger_api::v2::value::Sum::Record(ApiRecord {
                    record_id: None,
                    fields,
                })),
            }
        }
        Some(expr::Sum::OptionalSome(opt_some)) => {
            let value = opt_some
                .value
                .as_ref()
                .map(|e| Box::new(lf_expr_to_api_value(e, interned_strings)));
            ApiValue {
                sum: Some(ledger_api::v2::value::Sum::Optional(Box::new(
                    ledger_api::v2::Optional { value },
                ))),
            }
        }
        Some(expr::Sum::OptionalNone(_)) => ApiValue {
            sum: Some(ledger_api::v2::value::Sum::Optional(Box::new(
                ledger_api::v2::Optional { value: None },
            ))),
        },
        Some(expr::Sum::Cons(cons)) => {
            let elements = cons
                .front
                .iter()
                .map(|e| lf_expr_to_api_value(e, interned_strings))
                .collect();
            ApiValue {
                sum: Some(ledger_api::v2::value::Sum::List(ledger_api::v2::List {
                    elements,
                })),
            }
        }
        // Add handling for TextMap, GenMap, Variant, Enum, etc. as needed
        _ => ApiValue { sum: None },
    }
}
