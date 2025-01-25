use std::path::PathBuf;

use fctools::{
    process_spawner::DirectProcessSpawner,
    runtime::tokio::TokioRuntime,
    vmm::{
        ownership::VmmOwnershipModel,
        resource_v3::{system::ResourceSystem, CreatedResourceType},
    },
};

#[tokio::test]
async fn resource_system_v3() {
    let mut system = ResourceSystem::new(DirectProcessSpawner, TokioRuntime, VmmOwnershipModel::Shared);
    let path = PathBuf::from("/home/kanpov/ok");

    let resource = system
        .new_created_resource(path.clone(), CreatedResourceType::File)
        .unwrap();

    dbg!(resource.get_state());

    resource.start_initialization(path.clone(), None).unwrap();
    tokio::time::sleep(std::time::Duration::from_micros(1)).await;
    system.wait_for_pending_tasks().await.unwrap();
    dbg!(resource.get_state());

    system.shutdown().await.unwrap();
}
