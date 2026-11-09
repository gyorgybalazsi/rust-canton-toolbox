use daml_type_rep::built_in_types::DamlParty;
use daml_type_rep::lapi_access::LapiAccess;
use derive_lapi_access::LapiAccess;
use ledger_api::v2::Record;

#[derive(Debug, PartialEq, serde::Serialize, LapiAccess)]
pub struct Give {
    new_owner: DamlParty,
}

impl Give {
    pub fn new(new_owner: String) -> Self {
        Give {
            new_owner: DamlParty::new(&new_owner),
        }
    }
}
