use afterglow_engine::{Replicate, network::replication::Replicate as ReplicateTrait, replicate};

#[derive(Replicate)]
struct DerivedReplicatedComponent;

#[replicate]
struct AttributedReplicatedComponent;

#[derive(Replicate)]
struct GenericDerivedReplicatedComponent<T>(T);

#[replicate]
struct GenericAttributedReplicatedComponent<T>(T)
where
    T: Copy;

#[test]
fn derive_replicate_marks_type_for_registration() {
    assert!(DerivedReplicatedComponent::REPLICATION_NAME.ends_with("::DerivedReplicatedComponent"));
}

#[test]
fn attribute_replicate_marks_type_for_registration() {
    assert!(
        AttributedReplicatedComponent::REPLICATION_NAME
            .ends_with("::AttributedReplicatedComponent")
    );
}

#[test]
fn derive_replicate_supports_generic_types() {
    assert!(
        GenericDerivedReplicatedComponent::<u32>::REPLICATION_NAME
            .ends_with("::GenericDerivedReplicatedComponent")
    );
}

#[test]
fn attribute_replicate_supports_generic_types_with_existing_where_clause() {
    assert!(
        GenericAttributedReplicatedComponent::<u32>::REPLICATION_NAME
            .ends_with("::GenericAttributedReplicatedComponent")
    );
}
