//! Process-wide Tokio runtime. The plugin's RPC loop is synchronous, so every
//! handler drives async MongoDB work through `block_on` on this single runtime.

use std::sync::OnceLock;

use tokio::runtime::Runtime;

pub fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime")
    })
}
