use daml_type_rep::built_in_types::{DamlParty, DamlText};
use daml_type_rep::lapi_access::LapiAccess;
use daml_type_rep::lapi_access::ToCreateArguments;
use derive_lapi_access::ToCreateArguments;
use ledger_api::v2::Record;

#[derive(Debug, serde::Serialize, ToCreateArguments)]
pub struct Asset {
    issuer: DamlParty,
    owner: DamlParty,
    name: DamlText,
}

impl Asset {
    pub fn new(issuer: String, owner: String, name: String) -> Self {
        Asset {
            issuer: DamlParty::new(&issuer),
            owner: DamlParty::new(&owner),
            name: DamlText::new(name),
        }
    }
}
