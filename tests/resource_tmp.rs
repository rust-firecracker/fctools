use std::path::PathBuf;

use fctools::{
    process_spawner::DirectProcessSpawner,
    runtime::tokio::TokioRuntime,
    vmm::{
        ownership::VmmOwnershipModel,
        resource_v3::{bus::tokio::TokioBus, system::ResourceSystem, MovedResourceType},
    },
};

#[tokio::test]
async fn resource_system_v3() {
    let mut system =
        ResourceSystem::<_, _, TokioBus>::new(DirectProcessSpawner, TokioRuntime, VmmOwnershipModel::Shared);
    let mut resource = system
        .new_moved_resource(PathBuf::from("/home/kanpov/test.txt"), MovedResourceType::Copied)
        .unwrap();

    resource
        .clone()
        .start_initialization(PathBuf::from("/a"), Some(PathBuf::from("/b")))
        .unwrap();
    dbg!(resource.get_state());

    system.shutdown().await.unwrap();
}
