use crate::vmm::resource::ProducedResource;

use super::configuration::VmConfigurationData;

/// The data associated with a snapshot created for a [Vm](crate::vm::Vm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmSnapshot {
    pub snapshot: ProducedResource,
    pub mem_file: ProducedResource,
    pub configuration_data: VmConfigurationData,
}

impl VmSnapshot {
    // TODO: reimplement

    // pub async fn copy<R: Runtime>(
    //     &mut self,
    //     new_snapshot_path: impl Into<PathBuf>,
    //     new_mem_file_path: impl Into<PathBuf>,
    //     runtime: &R,
    // ) -> Result<(), std::io::Error> {
    //     futures_util::try_join!(
    //         self.snapshot.copy(new_snapshot_path, runtime),
    //         self.mem_file.copy(new_mem_file_path, runtime)
    //     )
    //     .map(|_| ())
    // }

    // pub async fn rename<R: Runtime>(
    //     &mut self,
    //     new_snapshot_path: impl Into<PathBuf>,
    //     new_mem_file_path: impl Into<PathBuf>,
    //     runtime: &R,
    // ) -> Result<(), std::io::Error> {
    //     futures_util::try_join!(
    //         self.snapshot.rename(new_snapshot_path, runtime),
    //         self.mem_file.rename(new_mem_file_path, runtime)
    //     )
    //     .map(|_| ())
    // }

    // pub async fn remove<R: Runtime>(self, runtime: &R) {
    //     let _ = futures_util::try_join!(self.snapshot.delete(runtime), self.mem_file.delete(runtime));
    // }

    // pub fn into_configuration(
    //     self,
    //     move_method: VmmResourceMoveMethod,
    //     enable_diff_snapshots: Option<bool>,
    //     resume_vm: Option<bool>,
    // ) -> VmConfiguration {
    //     let mem_file = MovedVmmResource::new(self.mem_file.effective_path(), move_method);
    //     let snapshot = MovedVmmResource::new(self.snapshot.effective_path(), move_method);

    //     let load_snapshot = LoadSnapshot {
    //         enable_diff_snapshots,
    //         mem_backend: MemoryBackend {
    //             backend_type: MemoryBackendType::File,
    //             backend: mem_file,
    //         },
    //         snapshot,
    //         resume_vm,
    //     };

    //     VmConfiguration::RestoredFromSnapshot {
    //         load_snapshot,
    //         data: self.configuration_data,
    //     }
    // }
}
