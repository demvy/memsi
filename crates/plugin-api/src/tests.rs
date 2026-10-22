use super::*;

fn capability() -> Capability {
    Capability::pure(CapabilityId::new("candles").unwrap(), 1)
}

fn plugin(name: &str, dependencies: &[&str]) -> ConfiguredPlugin {
    ConfiguredPlugin {
        instance: InstanceId::new(name).unwrap(),
        descriptor: PluginDescriptor {
            id: PluginId::new(format!("plugin.{name}")).unwrap(),
            version: "1".into(),
            kind: PluginKind::Analyzer,
            provides: vec![capability()],
        },
        config_digest: ContentHash::of_bytes(name.as_bytes()),
        requires: dependencies
            .iter()
            .map(|provider| Requirement {
                binding: BindingId::new(*provider).unwrap(),
                provider: InstanceId::new(*provider).unwrap(),
                capability: capability(),
            })
            .collect(),
    }
}

#[test]
fn capabilities_require_matching_schema_and_all_requested_dimensions() {
    let mut provided = capability();
    assert!(provided.covers(&capability()));
    let mut request = capability();
    request.schema_major = 2;
    assert!(!provided.covers(&request));
    request = capability();
    request.id = CapabilityId::new("other").unwrap();
    assert!(!provided.covers(&request));
    request = capability();
    request.chains.push(ChainId::Evm { chain_id: 1 });
    assert!(!provided.covers(&request));
    provided.chains = request.chains.clone();
    assert!(provided.covers(&request));
    request.resolutions.push(Resolution::new(60).unwrap());
    assert!(!provided.covers(&request));
    provided.resolutions = request.resolutions.clone();
    request.price_units.push(PriceUnit::UsdPerToken);
    assert!(!provided.covers(&request));
    provided.price_units = request.price_units.clone();
    assert!(provided.covers(&request));
}

#[test]
fn catalog_rejects_duplicates_and_retains_original_descriptor() {
    let mut catalog = PluginCatalog::default();
    let descriptor = plugin("test", &[]).descriptor;
    assert!(matches!(
        catalog.get(&descriptor.id),
        Err(RegistryError::NotCompiled(_))
    ));
    catalog.register(descriptor.clone()).unwrap();
    let mut duplicate = descriptor.clone();
    duplicate.version = "2".into();
    assert!(matches!(
        catalog.register(duplicate),
        Err(RegistryError::DuplicatePlugin(_))
    ));
    assert_eq!(catalog.get(&descriptor.id).unwrap().version, "1");
    assert_eq!(catalog.descriptors().count(), 1);
}

#[test]
fn instance_validation_orders_dependencies_once_and_handles_empty_graphs() {
    assert!(validate_instances(&[]).unwrap().is_empty());
    let instances = [
        plugin("a", &["b", "c"]),
        plugin("b", &["d"]),
        plugin("c", &["d"]),
        plugin("d", &[]),
    ];
    let order = validate_instances(&instances).unwrap();
    let names: Vec<_> = order.iter().map(InstanceId::as_str).collect();
    assert_eq!(names, ["d", "b", "c", "a"]);
}

#[test]
fn instance_validation_reports_duplicate_missing_incompatible_and_cyclic_dependencies() {
    let instance = plugin("a", &[]);
    assert!(matches!(
        validate_instances(&[instance.clone(), instance]),
        Err(RegistryError::DuplicateInstance(_))
    ));
    assert!(matches!(
        validate_instances(&[plugin("a", &["missing"])]),
        Err(RegistryError::MissingCapability { .. })
    ));
    let mut incompatible = plugin("b", &[]);
    incompatible.descriptor.provides[0].schema_major = 2;
    assert!(matches!(
        validate_instances(&[plugin("a", &["b"]), incompatible]),
        Err(RegistryError::MissingCapability { .. })
    ));
    let Err(RegistryError::Cycle(path)) =
        validate_instances(&[plugin("a", &["b"]), plugin("b", &["a"])])
    else {
        panic!("expected cycle")
    };
    assert_eq!(
        path.iter().map(InstanceId::as_str).collect::<Vec<_>>(),
        ["a", "b", "a"]
    );
    assert!(matches!(
        validate_instances(&[plugin("a", &["a"])]),
        Err(RegistryError::Cycle(_))
    ));
}
