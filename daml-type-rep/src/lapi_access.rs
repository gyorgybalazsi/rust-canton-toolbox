use crate::built_in_types::*;
use chrono::Datelike;
use ledger_api::v2::{Record, RecordField, Value, value::Sum};

// Traits
pub trait LapiAccess {
    fn to_lapi_value(&self) -> Value;
    // Default implementation
    fn to_lapi_record_field(&self, field_name: &str) -> RecordField {
        RecordField {
            label: field_name.to_string(),
            value: Some(self.to_lapi_value()),
        }
    }
    fn from_lapi_value(_value: &Value) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
}

pub trait ToCreateArguments {
    fn to_create_arguments(&self) -> Record;
}

// Implementations for built-in types

// DamlInt
impl LapiAccess for DamlInt {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Int64(self.value())),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Int64(i)) => Some(DamlInt::new(*i)),
            _ => None,
        }
    }
}

// DamlText
impl LapiAccess for DamlText {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Text(self.value().to_string())),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Text(s)) => Some(DamlText::new(s.clone())),
            _ => None,
        }
    }
}

// DamlBool
impl LapiAccess for DamlBool {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Bool(self.value())),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Bool(b)) => Some(DamlBool::new(*b)),
            _ => None,
        }
    }
}

// DamlDate
impl LapiAccess for DamlDate {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Date(self.value().num_days_from_ce())),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Date(days)) => {
                chrono::NaiveDate::from_num_days_from_ce_opt(*days).map(DamlDate::new)
            }
            _ => None,
        }
    }
}

// DamlTime
impl LapiAccess for DamlTime {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Text(self.value().format("%H:%M:%S").to_string())),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Text(s)) => chrono::NaiveTime::parse_from_str(s, "%H:%M:%S")
                .ok()
                .map(DamlTime::new),
            _ => None,
        }
    }
}

// DamlTimestamp
impl LapiAccess for DamlTimestamp {
    fn to_lapi_value(&self) -> Value {
        let ts = self.value();
        let micros = ts.timestamp() * 1_000_000 + (ts.timestamp_subsec_micros() as i64);
        Value {
            sum: Some(Sum::Timestamp(micros)),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Timestamp(micros)) => {
                chrono::DateTime::<chrono::Utc>::from_timestamp_micros(*micros)
                    .map(DamlTimestamp::new)
            }
            _ => None,
        }
    }
}

// DamlUnit
impl LapiAccess for DamlUnit {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Unit(())),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Unit(_)) => Some(DamlUnit::new()),
            _ => None,
        }
    }
}

// DamlList
impl<T: LapiAccess + DamlValue> LapiAccess for DamlList<T> {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::List(ledger_api::v2::List {
                elements: self.value().iter().map(|x| x.to_lapi_value()).collect(),
            })),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::List(list)) => {
                let elements: Option<Vec<T>> = list
                    .elements
                    .iter()
                    .map(|v| T::from_lapi_value(v))
                    .collect();
                elements.map(DamlList::new)
            }
            _ => None,
        }
    }
}

// DamlOptional
impl<T: LapiAccess + DamlValue> LapiAccess for DamlOptional<T> {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Optional(Box::new(ledger_api::v2::Optional {
                value: self.value().as_ref().map(|x| Box::new(x.to_lapi_value())),
            }))),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Optional(opt)) => {
                let inner = opt.value.as_ref().and_then(|v| T::from_lapi_value(v));
                Some(DamlOptional::new(inner))
            }
            _ => None,
        }
    }
}

// DamlTextMap
impl<V: LapiAccess + DamlValue> LapiAccess for DamlTextMap<V> {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::TextMap(ledger_api::v2::TextMap {
                entries: self
                    .value()
                    .iter()
                    .map(|(k, v)| ledger_api::v2::text_map::Entry {
                        key: k.clone(),
                        value: Some(v.to_lapi_value()),
                    })
                    .collect(),
            })),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::TextMap(map)) => {
                let mut result = std::collections::BTreeMap::new();
                for entry in &map.entries {
                    if let (Some(v), k) = (entry.value.as_ref(), &entry.key) {
                        if let Some(val) = V::from_lapi_value(v) {
                            result.insert(k.clone(), val);
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                Some(DamlTextMap::new(result))
            }
            _ => None,
        }
    }
}

// DamlMap
impl<K, V> LapiAccess for DamlMap<K, V>
where
    K: LapiAccess + DamlValue + Ord,
    V: LapiAccess + DamlValue,
{
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::GenMap(ledger_api::v2::GenMap {
                entries: self
                    .value()
                    .iter()
                    .map(|(k, v)| ledger_api::v2::gen_map::Entry {
                        key: Some(k.to_lapi_value()),
                        value: Some(v.to_lapi_value()),
                    })
                    .collect(),
            })),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::GenMap(gen_map)) => {
                let mut result = std::collections::BTreeMap::new();
                for entry in &gen_map.entries {
                    if let (Some(kv), Some(vv)) = (entry.key.as_ref(), entry.value.as_ref()) {
                        let k = K::from_lapi_value(kv)?;
                        let v = V::from_lapi_value(vv)?;
                        result.insert(k, v);
                    } else {
                        return None;
                    }
                }
                Some(DamlMap::new(result))
            }
            _ => None,
        }
    }
}

// DamlParty
impl LapiAccess for DamlParty {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Party(self.party_id.to_string())),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Party(s)) => Some(DamlParty::new(s.clone())),
            _ => None,
        }
    }
}

// DamlContractId
impl LapiAccess for DamlContractId {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::ContractId(self.contract_id.to_string())),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::ContractId(s)) => Some(DamlContractId::new(s.clone())),
            _ => None,
        }
    }
}

// DamlDecimal
impl LapiAccess for DamlDecimal {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Numeric(self.value.to_string())),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Numeric(s)) => s.parse().ok().map(DamlDecimal::new),
            _ => None,
        }
    }
}

// DamlNumeric
impl LapiAccess for DamlNumeric {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Numeric(self.value.to_string())),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Numeric(s)) => s
                .parse()
                .ok()
                .map(|v| DamlNumeric::new(v, NumericScale(10))),
            _ => None,
        }
    }
}

// Implement LapiAccess for i64 so it can be used in enums and records
impl LapiAccess for i64 {
    fn to_lapi_value(&self) -> Value {
        Value {
            sum: Some(Sum::Int64(*self)),
        }
    }
    fn from_lapi_value(value: &Value) -> Option<Self> {
        match &value.sum {
            Some(Sum::Int64(i)) => Some(*i),
            _ => None,
        }
    }
}
