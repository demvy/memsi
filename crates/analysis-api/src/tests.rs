use super::*;
use analysis_model::ArtifactProvenance;

fn slot() -> ArtifactSlot {
    ArtifactSlot {
        schema: SchemaId::new("test.schema").unwrap(),
        major: 1,
        instance: InstanceId::new("analyzer").unwrap(),
        invocation_digest: ContentHash::of_bytes(b"invocation"),
        input_hash: ContentHash::of_bytes(b"input"),
    }
}

fn artifact() -> Artifact<u32> {
    let slot = slot();
    Artifact {
        provenance: ArtifactProvenance {
            schema: slot.schema,
            schema_major: slot.major,
            producer: PluginId::new("test").unwrap(),
            producer_version: "1".into(),
            analyzer_instance: slot.instance,
            config_digest: ContentHash::of_bytes(b"config"),
            input_hash: slot.input_hash,
            as_of: TimestampMs::new(0).unwrap(),
        },
        value: 42,
    }
}

#[test]
fn artifact_store_distinguishes_missing_wrong_type_duplicate_and_success() {
    let mut store = ArtifactStore::default();
    let key = ArtifactKey::<u32>::new(slot());
    assert_eq!(store.get(&key).unwrap_err(), ArtifactError::Missing);
    store.insert(ArtifactKey::new(slot()), artifact()).unwrap();
    assert_eq!(store.get(&key).unwrap().value, 42);
    assert_eq!(
        store.get(&ArtifactKey::<String>::new(slot())).unwrap_err(),
        ArtifactError::WrongType
    );
    assert_eq!(
        store.insert(ArtifactKey::new(slot()), artifact()),
        Err(ArtifactError::Duplicate)
    );
    assert_eq!(store.get(&key).unwrap().value, 42);
    let mut other_slot = slot();
    other_slot.invocation_digest = ContentHash::of_bytes(b"other invocation");
    assert_eq!(
        store.get(&ArtifactKey::<u32>::new(other_slot)).unwrap_err(),
        ArtifactError::Missing
    );
}

#[test]
fn artifact_store_checks_each_provenance_field_before_insertion() {
    for field in 0..4 {
        let mut store = ArtifactStore::default();
        let mut value = artifact();
        match field {
            0 => value.provenance.schema = SchemaId::new("wrong.schema").unwrap(),
            1 => value.provenance.schema_major = 2,
            2 => value.provenance.analyzer_instance = InstanceId::new("wrong").unwrap(),
            _ => value.provenance.input_hash = ContentHash::of_bytes(b"wrong input"),
        }

        assert_eq!(
            store.insert(ArtifactKey::new(slot()), value),
            Err(ArtifactError::ProvenanceMismatch)
        );
        assert_eq!(
            store.get(&ArtifactKey::<u32>::new(slot())).unwrap_err(),
            ArtifactError::Missing
        );
    }
}
