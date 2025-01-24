use std::path::PathBuf;

use fctools::{
    process_spawner::DirectProcessSpawner,
    runtime::tokio::TokioRuntime,
    vmm::{
        ownership::VmmOwnershipModel,
        resource_v3::{bus::default::DefaultBus, system::ResourceSystem, MovedResourceType},
    },
};

#[tokio::test]
async fn resource_system_v3() {
    let system = ResourceSystem::<_, _, DefaultBus>::new(DirectProcessSpawner, TokioRuntime, VmmOwnershipModel::Shared);
    let resource = system
        .new_moved_resource(PathBuf::from("/home/kanpov/test.txt"), MovedResourceType::Copied)
        .unwrap();

    dbg!(resource.get_state());

    system.shutdown().await.unwrap();
}
