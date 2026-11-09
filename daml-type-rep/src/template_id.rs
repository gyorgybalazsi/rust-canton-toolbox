use ledger_api::v2::Identifier;

#[derive(Clone)]
pub struct TemplateId(String, String, String);

impl TemplateId {
    pub fn new(package_id: &str, module_name: &str, entity_name: &str) -> Self {
        TemplateId(
            package_id.to_string(),
            module_name.to_string(),
            entity_name.to_string(),
        )
    }

    pub fn to_template_id(&self) -> Identifier {
        Identifier {
            package_id: self.0.clone(),
            module_name: self.1.clone(),
            entity_name: self.2.clone(),
        }
    }
}
