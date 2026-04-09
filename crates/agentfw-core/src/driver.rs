use std::collections::HashMap;

use crate::error::FrameworkError;
use crate::runtime::AgentDriver;

/// Registry for runtime-usable driver implementations.
///
/// The registry only resolves a driver by key. It does not imply scheduling,
/// orchestration, or role semantics.
pub trait DriverRegistry {
    fn register(&mut self, key: String, driver: Box<dyn AgentDriver>)
        -> Result<(), FrameworkError>;
    fn get(&self, key: &str) -> Option<&dyn AgentDriver>;
}

pub(crate) struct InMemoryDriverRegistry {
    drivers: HashMap<String, Box<dyn AgentDriver>>,
}

impl InMemoryDriverRegistry {
    pub(crate) fn new() -> Self {
        Self {
            drivers: HashMap::new(),
        }
    }
}

impl Default for InMemoryDriverRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DriverRegistry for InMemoryDriverRegistry {
    fn register(
        &mut self,
        key: String,
        driver: Box<dyn AgentDriver>,
    ) -> Result<(), FrameworkError> {
        self.drivers.insert(key, driver);
        Ok(())
    }

    fn get(&self, key: &str) -> Option<&dyn AgentDriver> {
        self.drivers.get(key).map(|driver| driver.as_ref())
    }
}
