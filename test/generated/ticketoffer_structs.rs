#[allow(non_snake_case, unused_imports, dead_code)]
pub mod daml_ticketoffer {
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
        #[derive(Debug, Clone, serde::Serialize, LapiAccess, ToCreateArguments)]
        pub struct TicketAgreement {
            pub organizer: DamlParty,
            pub owner: DamlParty,
        }
        #[derive(Debug, Clone, serde::Serialize, LapiAccess)]
        pub struct Transfer {
            #[serde(rename = "newOwner")]
            pub new_owner: DamlParty,
        }
        #[derive(Debug, Clone, serde::Serialize, LapiAccess, ToCreateArguments)]
        pub struct Cash {
            pub issuer: DamlParty,
            pub owner: DamlParty,
            pub amount: DamlDecimal,
        }
        #[derive(Debug, Clone, serde::Serialize, LapiAccess)]
        pub struct Accept {
            #[serde(rename = "cashId")]
            pub cash_id: DamlContractId,
        }
        #[derive(Debug, Clone, serde::Serialize, LapiAccess, ToCreateArguments)]
        pub struct TicketOffer {
            pub organizer: DamlParty,
            pub buyer: DamlParty,
            pub price: DamlDecimal,
        }
    }
}
