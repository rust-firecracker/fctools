use std::ops::Deref;

use super::{CreatedResource, MovedResource, ProducedResource, Resource};

pub trait ResourceSet {
    type Iterator: Iterator<Item = Resource> + Send;

    fn get_resources(&self) -> Self::Iterator;
}

pub struct VecResourceSet {
    pub created_resources: Vec<CreatedResource>,
    pub moved_resources: Vec<MovedResource>,
    pub produced_resources: Vec<ProducedResource>,
}

impl VecResourceSet {
    pub fn new() -> Self {
        Self {
            created_resources: Vec::new(),
            moved_resources: Vec::new(),
            produced_resources: Vec::new(),
        }
    }
}

impl ResourceSet for VecResourceSet {
    type Iterator = std::vec::IntoIter<Resource>;

    fn get_resources(&self) -> Self::Iterator {
        let mut resources = Vec::with_capacity(
            self.created_resources.len() + self.moved_resources.len() + self.produced_resources.len(),
        );

        resources.extend(self.created_resources.iter().map(|r| r.deref().clone()));
        resources.extend(self.moved_resources.iter().map(|r| r.deref().clone()));
        resources.extend(self.produced_resources.iter().map(|r| r.deref().clone()));

        resources.into_iter()
    }
}
