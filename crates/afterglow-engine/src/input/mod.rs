mod actions;
mod bindings;
mod plugin;

pub use actions::AfterglowAction;
pub use bindings::default_gameplay_input_map;
pub use plugin::{AfterglowInputPlugin, AfterglowLeafwingPlugin};

#[cfg(test)]
mod tests;
