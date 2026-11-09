#[allow(non_snake_case, unused_imports, dead_code)]
pub mod daml_nested_test {
    #[allow(non_snake_case, unused_imports, dead_code)]
    pub mod main {
        use daml_type_rep::built_in_types::*;
        use daml_type_rep::lapi_access::LapiAccess;
        use daml_type_rep::lapi_access::ToCreateArguments;
        use derive_lapi_access::{LapiAccess, ToCreateArguments};
        use ledger_api::v2::Record;
        use ledger_api::v2::RecordField;
        use ledger_api::v2::Value;
        use ledger_api::v2::value::Sum;
        #[derive(Debug, Clone, serde::Serialize, LapiAccess)]
        pub struct UpdatePerson {
            #[serde(rename = "newPerson")]
            pub new_person: types::Person,
        }
        #[derive(Debug, Clone, serde::Serialize, LapiAccess, ToCreateArguments)]
        pub struct Registry {
            pub admin: DamlParty,
            pub person: types::Person,
        }
        pub type RegistryId = DamlContractId;
        #[allow(non_snake_case, unused_imports, dead_code)]
        pub mod types {
            use daml_type_rep::built_in_types::*;
            use daml_type_rep::lapi_access::LapiAccess;
            use daml_type_rep::lapi_access::ToCreateArguments;
            use derive_lapi_access::{LapiAccess, ToCreateArguments};
            use ledger_api::v2::Record;
            use ledger_api::v2::RecordField;
            use ledger_api::v2::Value;
            use ledger_api::v2::value::Sum;
            #[derive(Debug, Clone, serde::Serialize)]
            pub struct TreeNode<A> {
                pub value: A,
                pub left: Box<Tree<A>>,
                pub right: Box<Tree<A>>,
            }
            #[derive(Debug, Clone, serde::Serialize, LapiAccess)]
            pub struct Circle {
                pub radius: DamlDecimal,
            }
            #[derive(Debug, Clone, serde::Serialize, LapiAccess)]
            pub struct Rectangle {
                pub width: DamlDecimal,
                pub height: DamlDecimal,
            }
            #[derive(Debug, Clone, serde::Serialize, LapiAccess)]
            pub struct Person {
                pub name: DamlText,
                pub age: DamlInt,
                #[serde(rename = "homeAddress")]
                pub home_address: address::Address,
            }
            #[derive(Debug, Clone, serde::Serialize)]
            pub enum Tree<A> {
                Leaf,
                Node(Box<TreeNode<A>>),
            }
            #[derive(Debug, Clone, serde::Serialize)]
            pub enum ShapeData {
                Circle(Circle),
                Rectangle(Rectangle),
            }
            #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, LapiAccess)]
            pub enum Color {
                Red,
                Green,
                Blue,
            }
            pub type People = DamlList<Person>;
            #[allow(non_snake_case, unused_imports, dead_code)]
            pub mod address {
                use daml_type_rep::built_in_types::*;
                use daml_type_rep::lapi_access::LapiAccess;
                use daml_type_rep::lapi_access::ToCreateArguments;
                use derive_lapi_access::{LapiAccess, ToCreateArguments};
                use ledger_api::v2::Record;
                use ledger_api::v2::RecordField;
                use ledger_api::v2::Value;
                use ledger_api::v2::value::Sum;
                #[derive(Debug, Clone, serde::Serialize, LapiAccess)]
                pub struct Address {
                    pub street: DamlText,
                    pub city: DamlText,
                    pub zip: DamlText,
                }
            }
        }
    }
}
