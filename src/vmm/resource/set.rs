use super::{created::CreatedVmmResource, moved::MovedVmmResource, produced::ProducedVmmResource};

pub trait VmmResourceSet: Send {
    type MovedResources<'r>: IntoIterator<Item = &'r mut MovedVmmResource> + Send
    where
        Self: 'r;

    type CreatedResources<'r>: IntoIterator<Item = &'r mut CreatedVmmResource> + Send
    where
        Self: 'r;

    type ProducedResources<'r>: IntoIterator<Item = &'r mut ProducedVmmResource> + Send
    where
        Self: 'r;

    fn moved_resources(&mut self) -> Self::MovedResources<'_>;

    fn created_resources(&mut self) -> Self::CreatedResources<'_>;

    fn produced_resources(&mut self) -> Self::ProducedResources<'_>;
}
