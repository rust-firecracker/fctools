use std::path::PathBuf;

use fctools::{
    process_spawner::DirectProcessSpawner,
    runtime::tokio::TokioRuntime,
    vmm::{
        ownership::VmmOwnershipModel,
        resource_v3::{system::ResourceSystem, MovedResourceType},
    },
};

#[tokio::test]
async fn resource_system_v3() {
    let system = ResourceSystem::new(DirectProcessSpawner, TokioRuntime, VmmOwnershipModel::Shared);
    let handle = system
        .new_moved_resource(PathBuf::from("/home/kanpov/test.txt"), MovedResourceType::Copied)
        .unwrap();

    handle.ping().await.unwrap();

    system.shutdown().await.unwrap();
}
