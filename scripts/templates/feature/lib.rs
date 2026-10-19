// __NAME__ — a METEOR V1 feature. The contract is docs/ARCHITECTURE.md §2: nothing
// runs before `start`, everything is gone after `stop`, and every command of
// the feature arrives through `call`.

use notch_core::{Ctx, Feature, Group, Manifest};
use serde_json::Value;

static MANIFEST: Manifest = Manifest {
    id: "__ID__",
    name: "__NAME__",
    group: Group::Tools,
    // Off until the package works; flip it then if the feature belongs to the core product.
    default_on: false,
    uses: &[],
    shortcuts: &[],
    tray: &[],
    defaults: "{}",
};

#[derive(Default)]
pub struct __TYPE__;

impl Feature for __TYPE__ {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, _cx: &Ctx) -> Result<(), String> {
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {}

    fn call(&self, _cx: &Ctx, _method: &str, _args: Value) -> Result<Value, String> {
        Err("unknown method".into())
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/__ID__/strings.json")
    }
}

pub fn feature() -> std::sync::Arc<dyn Feature> {
    std::sync::Arc::new(__TYPE__)
}
