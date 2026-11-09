use rust_decimal::prelude::FromPrimitive;
/// Rust equivalents for Daml built-in types as structs
use std::fmt;
use std::fmt::Display;

pub trait DamlValue {} // Marker trait for all Daml value types

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlInt {
    value: i64,
}

impl DamlInt {
    pub fn new(value: i64) -> Self {
        DamlInt { value }
    }
    pub fn value(&self) -> i64 {
        self.value
    }
}

impl DamlValue for DamlInt {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlText {
    value: String,
}

impl DamlText {
    pub fn new(value: impl Into<String>) -> Self {
        DamlText {
            value: value.into(),
        }
    }
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl DamlValue for DamlText {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlBool {
    value: bool,
}

impl DamlBool {
    pub fn new(value: bool) -> Self {
        DamlBool { value }
    }
    pub fn value(&self) -> bool {
        self.value
    }
}

impl DamlValue for DamlBool {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlDate {
    pub value: chrono::NaiveDate,
}

impl DamlDate {
    pub fn new(value: chrono::NaiveDate) -> Self {
        DamlDate { value }
    }
    pub fn value(&self) -> &chrono::NaiveDate {
        &self.value
    }
}

impl DamlValue for DamlDate {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlTime {
    pub value: chrono::NaiveTime,
}

impl DamlTime {
    pub fn new(value: chrono::NaiveTime) -> Self {
        DamlTime { value }
    }
    pub fn value(&self) -> &chrono::NaiveTime {
        &self.value
    }
}

impl DamlValue for DamlTime {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlTimestamp {
    pub value: chrono::DateTime<chrono::Utc>,
}

impl DamlTimestamp {
    pub fn new(value: chrono::DateTime<chrono::Utc>) -> Self {
        DamlTimestamp { value }
    }
    pub fn value(&self) -> &chrono::DateTime<chrono::Utc> {
        &self.value
    }
}

impl DamlValue for DamlTimestamp {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlUnit {
    pub value: (),
}

impl Default for DamlUnit {
    fn default() -> Self {
        Self::new()
    }
}

impl DamlUnit {
    pub fn new() -> Self {
        DamlUnit { value: () }
    }
    pub fn value(&self) -> &() {
        &self.value
    }
}

impl DamlValue for DamlUnit {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlList<T> {
    pub value: Vec<T>,
}

impl<T: DamlValue> DamlList<T> {
    pub fn new(value: Vec<T>) -> Self {
        DamlList { value }
    }
    pub fn value(&self) -> &Vec<T> {
        &self.value
    }
}

impl<T: DamlValue> DamlValue for DamlList<T> {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlOptional<T> {
    pub value: Option<T>,
}

impl<T> DamlOptional<T> {
    pub fn new(value: Option<T>) -> Self {
        DamlOptional { value }
    }
    pub fn value(&self) -> &Option<T> {
        &self.value
    }
}

impl<T: DamlValue> DamlValue for DamlOptional<T> {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlMap<K, V> {
    pub value: std::collections::BTreeMap<K, V>,
}

impl<K: DamlValue, V: DamlValue> DamlMap<K, V> {
    pub fn new(value: std::collections::BTreeMap<K, V>) -> Self {
        DamlMap { value }
    }
    pub fn value(&self) -> &std::collections::BTreeMap<K, V> {
        &self.value
    }
}

impl<K: DamlValue, V: DamlValue> DamlValue for DamlMap<K, V> {}

// TODO String key is ok?
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DamlTextMap<V> {
    pub value: std::collections::BTreeMap<String, V>,
}

impl<V: DamlValue> DamlTextMap<V> {
    pub fn new(value: std::collections::BTreeMap<String, V>) -> Self {
        DamlTextMap { value }
    }
    pub fn value(&self) -> &std::collections::BTreeMap<String, V> {
        &self.value
    }
}

impl<V: DamlValue> DamlValue for DamlTextMap<V> {}

#[derive(Debug, Eq, PartialEq, PartialOrd, Ord, Clone, serde::Serialize)]
pub struct DamlParty {
    pub party_id: String,
}

impl Display for DamlParty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.party_id)
    }
}

impl DamlParty {
    pub fn new(party: impl Into<String>) -> Self {
        Self {
            party_id: party.into(),
        }
    }

    pub fn as_str(&self) -> &str {
        self.party_id.as_str()
    }
}

impl DamlValue for DamlParty {}

#[derive(Debug, Eq, PartialEq, PartialOrd, Ord, Clone, serde::Serialize)]
pub struct DamlContractId {
    pub contract_id: String,
}

impl DamlContractId {
    pub fn new(contract_id: impl Into<String>) -> Self {
        Self {
            contract_id: contract_id.into(),
        }
    }

    pub fn as_str(&self) -> &str {
        self.contract_id.as_str()
    }
}

impl DamlValue for DamlContractId {}

#[derive(Debug, Eq, PartialEq, PartialOrd, Ord, Clone, serde::Serialize)]
pub struct DamlDecimal {
    pub value: rust_decimal::Decimal,
}

impl DamlDecimal {
    pub fn new(value: f64) -> Self {
        DamlDecimal {
            value: rust_decimal::Decimal::from_f64(value).unwrap().round_dp(10),
        }
    }
}

impl fmt::Display for DamlDecimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl DamlValue for DamlDecimal {}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct NumericScale(pub u32);

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct DamlNumeric {
    pub value: rust_decimal::Decimal,
    pub scale: NumericScale,
}

impl DamlNumeric {
    pub fn from_numeric(value: rust_decimal::Decimal, scale: NumericScale) -> Self {
        let scaled_value = value.round_dp(scale.0);
        DamlNumeric {
            value: scaled_value,
            scale,
        }
    }

    pub fn new(value: f64, scale: NumericScale) -> Self {
        DamlNumeric::from_numeric(
            rust_decimal::Decimal::from_f64(value)
                .unwrap()
                .round_dp(scale.0),
            scale,
        )
    }
}

impl fmt::Display for DamlNumeric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (scale: {})", self.value, self.scale.0)
    }
}

impl DamlValue for DamlNumeric {}
