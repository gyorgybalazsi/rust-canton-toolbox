// In derive-lapi-access/tests/derive_lapi_access.rs
extern crate derive_lapi_access;
use daml_type_rep::built_in_types::{
    DamlInt, DamlList, DamlMap, DamlOptional, DamlParty, DamlText,
};
use daml_type_rep::lapi_access::LapiAccess;
use derive_lapi_access::LapiAccess;
use ledger_api::v2::Record;

#[derive(Debug, PartialEq, LapiAccess)]
struct MyStruct {
    party: DamlParty,
    text: DamlText,
    optional: DamlOptional<DamlText>,
    list: DamlList<DamlInt>,
    map: DamlMap<DamlText, DamlInt>,
}

#[derive(Debug, PartialEq, LapiAccess)]
pub enum Color {
    Red,
    Green,
    Blue,
}

#[derive(Debug, PartialEq, LapiAccess)]
pub enum Price {
    USD { amount: DamlInt, color: Color },
    EUR { amount: DamlInt, color: Color },
    GBP,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_record_macro_expansion() {
        let s = MyStruct {
            party: DamlParty::new("Alice"),
            text: DamlText::new("Hello"),
            optional: DamlOptional::new(Some(DamlText::new("Optional text"))),
            list: DamlList::new(vec![DamlInt::new(1), DamlInt::new(2), DamlInt::new(3)]),
            map: DamlMap::new(
                vec![
                    (DamlText::new("key1"), DamlInt::new(10)),
                    (DamlText::new("key2"), DamlInt::new(20)),
                ]
                .into_iter()
                .collect(),
            ),
        };
        let value = s.to_lapi_value();
        let deserialized = MyStruct::from_lapi_value(&value).expect("Deserialization failed");
        assert_eq!(s, deserialized);
    }

    #[test]
    fn test_enum_no_fields_macro_expansion() {
        let color = Color::Red;
        let value = color.to_lapi_value();
        let deserialized = Color::from_lapi_value(&value).expect("Deserialization failed");
        assert_eq!(color, deserialized);
    }

    #[test]
    fn test_enum_with_fields_macro_expansion() {
        let price = Price::GBP;
        let value = price.to_lapi_value();
        dbg!(&value);
        let deserialized = Price::from_lapi_value(&value).expect("Deserialization failed");
        dbg!(&deserialized);
        assert_eq!(price, deserialized);
        let price = Price::EUR {
            amount: DamlInt::new(200),
            color: Color::Green,
        };
        let value = price.to_lapi_value();
        dbg!(&value);
        let deserialized = Price::from_lapi_value(&value).expect("Deserialization failed");
        dbg!(&deserialized);
        assert_eq!(price, deserialized);
        let price = Price::USD {
            amount: DamlInt::new(100),
            color: Color::Blue,
        };
        let value = price.to_lapi_value();
        dbg!(&value);
        let deserialized = Price::from_lapi_value(&value).expect("Deserialization failed");
        dbg!(&deserialized);
        assert_eq!(price, deserialized);
    }
}
