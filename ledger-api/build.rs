use std::error;
use std::fs;
use std::io::Error;
use std::path::Path;
use std::path::PathBuf;

const ALL_PROTO_SRC_PATHS: &[&str] = &[
    "com/daml/ledger/api/v2",
    "com/daml/ledger/api/v2/testing",
    "com/daml/ledger/api/v2/admin",
    "com/daml/ledger/api/v2/interactive",
    "google/protobuf",
    "google/rpc",
];
const PROTO_ROOT_PATH: &str = "resources/protobuf";

fn main() -> Result<(), Box<dyn error::Error>> {
    let all_protos = get_all_protos(ALL_PROTO_SRC_PATHS)?;
    let serialize = "#[derive(serde::Serialize)]";
    let ts_ser =
        r#"#[serde(serialize_with = "crate::serde_helpers::serialize_optional_timestamp")]"#;
    let bytes_ser =
        r#"#[serde(serialize_with = "crate::serde_helpers::serialize_bytes_as_base64")]"#;
    let skip = r#"#[serde(skip)]"#;

    tonic_build::configure()
        // Value types (existing)
        .type_attribute("com.daml.ledger.api.v2.Record", serialize)
        .type_attribute("com.daml.ledger.api.v2.RecordField", serialize)
        .type_attribute("com.daml.ledger.api.v2.Identifier", serialize)
        .type_attribute("com.daml.ledger.api.v2.Value", serialize)
        .type_attribute("com.daml.ledger.api.v2.Value.sum", serialize)
        .type_attribute("com.daml.ledger.api.v2.Optional", serialize)
        .type_attribute("com.daml.ledger.api.v2.List", serialize)
        .type_attribute("com.daml.ledger.api.v2.TextMap", serialize)
        .type_attribute("com.daml.ledger.api.v2.TextMap.Entry", serialize)
        .type_attribute("com.daml.ledger.api.v2.GenMap", serialize)
        .type_attribute("com.daml.ledger.api.v2.GenMap.Entry", serialize)
        .type_attribute("com.daml.ledger.api.v2.Variant", serialize)
        .type_attribute("com.daml.ledger.api.v2.Enum", serialize)
        // Response and update types
        .type_attribute("com.daml.ledger.api.v2.GetUpdateResponse", serialize)
        .type_attribute("com.daml.ledger.api.v2.GetUpdateResponse.update", serialize)
        .type_attribute("com.daml.ledger.api.v2.Transaction", serialize)
        .type_attribute("com.daml.ledger.api.v2.Reassignment", serialize)
        .type_attribute("com.daml.ledger.api.v2.TopologyTransaction", serialize)
        // Event types
        .type_attribute("com.daml.ledger.api.v2.Event", serialize)
        .type_attribute("com.daml.ledger.api.v2.Event.event", serialize)
        .type_attribute("com.daml.ledger.api.v2.CreatedEvent", serialize)
        .type_attribute("com.daml.ledger.api.v2.ArchivedEvent", serialize)
        .type_attribute("com.daml.ledger.api.v2.ExercisedEvent", serialize)
        .type_attribute("com.daml.ledger.api.v2.InterfaceView", serialize)
        .type_attribute("com.daml.ledger.api.v2.TraceContext", serialize)
        // Reassignment event types
        .type_attribute("com.daml.ledger.api.v2.ReassignmentEvent", serialize)
        .type_attribute("com.daml.ledger.api.v2.ReassignmentEvent.event", serialize)
        .type_attribute("com.daml.ledger.api.v2.UnassignedEvent", serialize)
        .type_attribute("com.daml.ledger.api.v2.AssignedEvent", serialize)
        // Topology event types
        .type_attribute("com.daml.ledger.api.v2.TopologyEvent", serialize)
        .type_attribute("com.daml.ledger.api.v2.TopologyEvent.event", serialize)
        .type_attribute(
            "com.daml.ledger.api.v2.ParticipantAuthorizationAdded",
            serialize,
        )
        .type_attribute(
            "com.daml.ledger.api.v2.ParticipantAuthorizationChanged",
            serialize,
        )
        .type_attribute(
            "com.daml.ledger.api.v2.ParticipantAuthorizationRevoked",
            serialize,
        )
        .type_attribute(
            "com.daml.ledger.api.v2.ParticipantAuthorizationOnboarding",
            serialize,
        )
        // google.rpc.Status (used in InterfaceView)
        .type_attribute("google.rpc.Status", serialize)
        // Timestamp fields — serialize as RFC3339 strings
        .field_attribute("com.daml.ledger.api.v2.Transaction.effective_at", ts_ser)
        .field_attribute("com.daml.ledger.api.v2.Transaction.record_time", ts_ser)
        .field_attribute("com.daml.ledger.api.v2.CreatedEvent.created_at", ts_ser)
        .field_attribute(
            "com.daml.ledger.api.v2.UnassignedEvent.assignment_exclusivity",
            ts_ser,
        )
        .field_attribute("com.daml.ledger.api.v2.Reassignment.record_time", ts_ser)
        .field_attribute(
            "com.daml.ledger.api.v2.TopologyTransaction.record_time",
            ts_ser,
        )
        // Bytes fields — serialize as base64
        .field_attribute(
            "com.daml.ledger.api.v2.CreatedEvent.created_event_blob",
            bytes_ser,
        )
        // Skip fields with prost_types::Any (not serializable)
        .field_attribute("google.rpc.Status.details", skip)
        .build_server(false)
        .build_client(true)
        .out_dir("src/pb")
        .compile_protos(&all_protos, &[PROTO_ROOT_PATH])?;
    Ok(())
}

fn get_all_protos(src_paths: &[&str]) -> Result<Vec<PathBuf>, Error> {
    let mut protos = Vec::new();
    for path in src_paths {
        let dir = Path::new(path);
        let files = get_protos_from_dir(dir)?;
        protos.extend(files);
    }
    Ok(protos)
}

fn get_protos_from_dir(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    fs::read_dir(Path::new(PROTO_ROOT_PATH).join(dir))?
        .filter_map(|entry| match entry {
            Ok(d) => match d.path().extension() {
                Some(a) if a == "proto" => Some(Ok(d.path())),
                _ => None,
            },
            Err(e) => Some(Err(e)),
        })
        .collect()
}
